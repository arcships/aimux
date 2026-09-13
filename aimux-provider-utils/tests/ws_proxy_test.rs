//! RFC-0034 §2: the global `ProxyConfig` governs WebSocket connections.
//!
//! Integration tests run a real local WS server and a fake CONNECT proxy on
//! a dedicated runtime thread (the fixture must outlive every `#[tokio::test]`
//! runtime). Unit tests cover proxy selection and `no_proxy` matching.

#![cfg(feature = "ws")]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use aimux_core::options::TimeoutConfiguration;
use aimux_provider_utils::http::ProxyConfig;
use aimux_provider_utils::ws::{WebSocketRequest, ws_connect};
use futures::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::Message;

// ── Fixture: one fake proxy + two real WS servers on a parked runtime ────────

struct Fixture {
    /// WS server reached THROUGH the proxy (its port is not in no_proxy).
    tunnel_port: u16,
    /// WS server reached DIRECTLY (its port is a port-specific no_proxy entry).
    direct_port: u16,
    connect_authorities: Arc<Mutex<Vec<String>>>,
    /// Full CONNECT request text (request line + headers), newest last.
    connect_requests: Arc<Mutex<Vec<String>>>,
    proxy_connections: Arc<AtomicUsize>,
}

static FIXTURE: OnceLock<Fixture> = OnceLock::new();

fn fixture() -> &'static Fixture {
    FIXTURE.get_or_init(|| {
        let connect_authorities: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let connect_requests: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let proxy_connections = Arc::new(AtomicUsize::new(0));

        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<(u16, u16, u16)>();
        let authorities_for_thread = Arc::clone(&connect_authorities);
        let requests_for_thread = Arc::clone(&connect_requests);
        let conns_for_thread = Arc::clone(&proxy_connections);
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("fixture runtime");
            rt.block_on(async move {
                let tunnel_port = spawn_echo_server().await;
                let direct_port = spawn_echo_server().await;
                let proxy_port = spawn_fake_proxy(
                    Arc::clone(&authorities_for_thread),
                    Arc::clone(&requests_for_thread),
                    Arc::clone(&conns_for_thread),
                )
                .await;
                ready_tx
                    .send((proxy_port, tunnel_port, direct_port))
                    .expect("test process gone");
                // Park this runtime forever: the listener tasks live on it.
                std::future::pending::<()>().await;
            });
        });
        let (proxy_port, tunnel_port, direct_port) = ready_rx.recv().expect("fixture thread died");

        // One process-wide proxy config (init_proxy is a set-once global):
        // everything tunnels through the fake proxy except the direct
        // server's port, which is covered by a port-specific no_proxy entry.
        let configured = aimux_provider_utils::http::init_proxy(ProxyConfig {
            http_url: Some(format!("http://127.0.0.1:{proxy_port}")),
            // wss targets tunnel too — the live smoke relies on it to drive
            // the rustls branch through the CONNECT proxy against the real
            // endpoint.
            https_url: Some(format!("http://127.0.0.1:{proxy_port}")),
            all_url: None,
            // Port-specific entry: also exercises the port-aware matching.
            no_proxy: Some(format!("127.0.0.1:{direct_port}")),
        });
        assert!(configured, "init_proxy must win (no earlier test set it)");

        Fixture {
            tunnel_port,
            direct_port,
            connect_authorities,
            connect_requests,
            proxy_connections,
        }
    })
}

async fn spawn_echo_server() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind echo server");
    let port = listener.local_addr().expect("local addr").port();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                continue;
            };
            tokio::spawn(async move {
                if let Ok(ws) = tokio_tungstenite::accept_async(stream).await {
                    let (mut sink, mut source) = ws.split();
                    while let Some(Ok(Message::Text(text))) = source.next().await {
                        if sink.send(Message::Text(text)).await.is_err() {
                            break;
                        }
                    }
                }
            });
        }
    });
    port
}

/// Minimal CONNECT proxy. Behavior by target-authority prefix:
/// `reject.` → 407, `busy.` → 503, `hang.` → accepts and never answers
/// (black hole); everything else → 200 + bridge to the real target.
async fn spawn_fake_proxy(
    connect_authorities: Arc<Mutex<Vec<String>>>,
    connect_requests: Arc<Mutex<Vec<String>>>,
    connections: Arc<AtomicUsize>,
) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fake proxy");
    let port = listener.local_addr().expect("local addr").port();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                continue;
            };
            let connect_authorities = Arc::clone(&connect_authorities);
            let connect_requests = Arc::clone(&connect_requests);
            let connections = Arc::clone(&connections);
            tokio::spawn(async move {
                connections.fetch_add(1, Ordering::SeqCst);
                let mut buffer = Vec::new();
                let mut chunk = [0u8; 512];
                while !buffer.windows(4).any(|w| w == b"\r\n\r\n") {
                    match stream.read(&mut chunk).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buffer.extend_from_slice(&chunk[..n]),
                    }
                }
                let request = String::from_utf8_lossy(&buffer).into_owned();
                let authority = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or_default()
                    .to_string();
                connect_authorities
                    .lock()
                    .expect("connect log mutex")
                    .push(authority.clone());
                connect_requests
                    .lock()
                    .expect("connect request log mutex")
                    .push(request);
                if authority.starts_with("reject.") {
                    let _ = stream
                        .write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n")
                        .await;
                    return;
                }
                if authority.starts_with("busy.") {
                    let _ = stream
                        .write_all(b"HTTP/1.1 503 Service Unavailable\r\n\r\n")
                        .await;
                    return;
                }
                if authority.starts_with("hang.") {
                    // Black hole: keep the connection open, never answer.
                    std::future::pending::<()>().await;
                }
                if stream
                    .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                    .await
                    .is_err()
                {
                    return;
                }
                let Ok(mut upstream) = TcpStream::connect(authority.as_str()).await else {
                    return;
                };
                let _ = tokio::io::copy_bidirectional(&mut stream, &mut upstream).await;
            });
        }
    });
    port
}

fn ws_request(url: String) -> WebSocketRequest {
    WebSocketRequest {
        url,
        headers: Vec::new(),
        subprotocols: Vec::new(),
        abort_signal: None,
        timeout: Some(TimeoutConfiguration {
            first_chunk_ms: Some(5_000),
            chunk_ms: Some(5_000),
            step_ms: None,
            total_ms: Some(10_000),
        }),
    }
}

// ── Integration ──────────────────────────────────────────────────────────────

#[tokio::test]
#[serial_test::serial]
async fn ws_tunnels_through_connect_proxy() {
    let fixture = fixture();
    let mut connection = ws_connect(&ws_request(format!(
        "ws://127.0.0.1:{}",
        fixture.tunnel_port
    )))
    .await
    .expect("tunneled connect");
    connection.send_text("ping").await.expect("send via tunnel");
    match connection.next().await {
        Some(Ok(aimux_provider_utils::ws::WsMessage::Text(text))) => assert_eq!(text, "ping"),
        other => panic!("expected echo through tunnel, got {other:?}"),
    }
    connection.close().await;

    let authorities = fixture
        .connect_authorities
        .lock()
        .expect("connect log mutex")
        .clone();
    assert!(
        authorities.contains(&format!("127.0.0.1:{}", fixture.tunnel_port)),
        "proxy must see the target authority, saw {authorities:?}"
    );
    assert!(fixture.proxy_connections.load(Ordering::SeqCst) >= 1);
}

#[tokio::test]
#[serial_test::serial]
async fn no_proxy_entry_connects_directly() {
    let fixture = fixture();
    let before = fixture.proxy_connections.load(Ordering::SeqCst);
    let mut connection = ws_connect(&ws_request(format!(
        "ws://127.0.0.1:{}",
        fixture.direct_port
    )))
    .await
    .expect("direct connect (port-specific no_proxy entry)");
    connection.send_text("direct").await.expect("send direct");
    match connection.next().await {
        Some(Ok(aimux_provider_utils::ws::WsMessage::Text(text))) => assert_eq!(text, "direct"),
        other => panic!("expected direct echo, got {other:?}"),
    }
    connection.close().await;
    assert_eq!(
        fixture.proxy_connections.load(Ordering::SeqCst),
        before,
        "no_proxy-matched target must not touch the proxy"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn proxy_rejection_surfaces_as_non_retryable_api_call() {
    let fixture = fixture();
    let error = match ws_connect(&ws_request(format!(
        "ws://reject.local:{}",
        fixture.tunnel_port
    )))
    .await
    {
        Err(error) => error,
        Ok(_) => panic!("proxy answers 407, connect must fail"),
    };
    match error {
        aimux_core::AiMuxError::ApiCall(api_call) => {
            assert_eq!(api_call.status_code, Some(407));
            assert!(
                !api_call.is_retryable,
                "407 is an auth verdict, not transient"
            );
        }
        other => panic!("expected ApiCall, got {other:?}"),
    }
}

#[tokio::test]
#[serial_test::serial]
async fn transient_proxy_503_is_retryable() {
    let fixture = fixture();
    let error = match ws_connect(&ws_request(format!(
        "ws://busy.local:{}",
        fixture.tunnel_port
    )))
    .await
    {
        Err(error) => error,
        Ok(_) => panic!("proxy answers 503, connect must fail"),
    };
    match error {
        aimux_core::AiMuxError::ApiCall(api_call) => {
            assert_eq!(api_call.status_code, Some(503));
            assert!(
                api_call.is_retryable,
                "proxy 503 is transient — same rule as HTTP"
            );
        }
        other => panic!("expected ApiCall, got {other:?}"),
    }
}

#[tokio::test]
#[serial_test::serial]
async fn unresponsive_proxy_times_out_under_first_chunk_budget() {
    let fixture = fixture();
    let mut request = ws_request(format!("ws://hang.local:{}", fixture.tunnel_port));
    request.timeout = Some(TimeoutConfiguration {
        first_chunk_ms: Some(300),
        chunk_ms: None,
        step_ms: None,
        total_ms: None,
    });
    let error = match ws_connect(&request).await {
        Err(error) => error,
        Ok(_) => panic!("proxy never answers, connect must time out"),
    };
    assert!(
        matches!(error, aimux_core::AiMuxError::Timeout(_)),
        "expected Timeout, got {error:?}"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn abort_during_connect_tunnel_surfaces_as_aborted() {
    let fixture = fixture();
    let abort = aimux_core::AbortSignal::new();
    let fire = abort.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        fire.abort();
    });
    let mut request = ws_request(format!("ws://hang.local:{}", fixture.tunnel_port));
    request.abort_signal = Some(abort);
    // Generous timeout: abort must win the race, not the timer.
    let error = match ws_connect(&request).await {
        Err(error) => error,
        Ok(_) => panic!("aborted connect must not succeed"),
    };
    assert!(
        matches!(error, aimux_core::AiMuxError::Aborted(_)),
        "expected Aborted, got {error:?}"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn connect_request_wire_shape() {
    let fixture = fixture();
    let before = fixture
        .connect_requests
        .lock()
        .expect("connect request log mutex")
        .len();
    let mut connection = ws_connect(&ws_request(format!(
        "ws://127.0.0.1:{}",
        fixture.tunnel_port
    )))
    .await
    .expect("tunneled connect");
    connection.close().await;
    let requests = fixture
        .connect_requests
        .lock()
        .expect("connect request log mutex")
        .clone();
    let request = &requests[before..]
        .last()
        .expect("this test's CONNECT request must be recorded");
    let lines: Vec<&str> = request.lines().collect();
    assert_eq!(
        lines[0],
        format!("CONNECT 127.0.0.1:{} HTTP/1.1", fixture.tunnel_port),
        "CONNECT request line"
    );
    assert!(
        lines.contains(&format!("Host: 127.0.0.1:{}", fixture.tunnel_port).as_str()),
        "Host header must repeat the authority, got {request:?}"
    );
    assert!(
        !request.to_ascii_lowercase().contains("proxy-authorization"),
        "no credentials configured: no Proxy-Authorization expected, got {request:?}"
    );
}

// ── Unit: proxy selection ────────────────────────────────────────────────────

fn config_with(
    https_url: Option<&str>,
    http_url: Option<&str>,
    all_url: Option<&str>,
) -> ProxyConfig {
    ProxyConfig {
        http_url: http_url.map(String::from),
        https_url: https_url.map(String::from),
        all_url: all_url.map(String::from),
        no_proxy: None,
    }
}

#[test]
fn no_proxy_config_means_direct() {
    let target = url::Url::parse("wss://api.example.test").unwrap();
    assert!(
        aimux_provider_utils::ws::ws__proxy_decision_for(&target, &ProxyConfig::default())
            .expect("resolve")
            .is_none()
    );
}

#[test]
fn wss_uses_https_url_ws_uses_http_url() {
    let config = config_with(Some("http://p:8080"), Some("http://h:3128"), None);
    let wss = url::Url::parse("wss://api.example.test").unwrap();
    let tunnel = aimux_provider_utils::ws::ws__proxy_decision_for(&wss, &config)
        .expect("resolve")
        .expect("tunnel");
    assert_eq!(tunnel.host, "p");
    assert_eq!(tunnel.port, 8080);
    assert!(tunnel.target_tls);
    assert_eq!(tunnel.target_authority, "api.example.test:443");

    let ws = url::Url::parse("ws://api.example.test:9000").unwrap();
    let tunnel = aimux_provider_utils::ws::ws__proxy_decision_for(&ws, &config)
        .expect("resolve")
        .expect("tunnel");
    assert_eq!(tunnel.host, "h");
    assert_eq!(tunnel.port, 3128);
    assert!(!tunnel.target_tls);
    assert_eq!(tunnel.target_authority, "api.example.test:9000");
}

#[test]
fn all_url_is_the_fallback_for_both_schemes() {
    let config = config_with(None, None, Some("http://a:1"));
    for scheme in ["wss", "ws"] {
        let target = url::Url::parse(&format!("{scheme}://api.example.test")).unwrap();
        let tunnel = aimux_provider_utils::ws::ws__proxy_decision_for(&target, &config)
            .expect("resolve")
            .expect("tunnel");
        assert_eq!(tunnel.host, "a");
    }
}

#[test]
fn proxy_port_defaults_to_80() {
    let config = config_with(Some("http://p"), None, None);
    let target = url::Url::parse("wss://api.example.test").unwrap();
    let tunnel = aimux_provider_utils::ws::ws__proxy_decision_for(&target, &config)
        .expect("resolve")
        .expect("tunnel");
    assert_eq!(tunnel.port, 80);
}

#[test]
fn socks_and_https_proxies_fail_loudly() {
    for scheme in ["socks5", "socks5h", "https"] {
        let config = config_with(Some(&format!("{scheme}://p:1080")), None, None);
        let target = url::Url::parse("wss://api.example.test").unwrap();
        let error = aimux_provider_utils::ws::ws__proxy_decision_for(&target, &config)
            .expect_err("must refuse");
        assert!(
            matches!(error, aimux_core::AiMuxError::UnsupportedFunctionality(_)),
            "{scheme} must be refused, got {error:?}"
        );
    }
}

#[test]
fn proxy_userinfo_becomes_basic_authorization() {
    let config = config_with(Some("http://user:pass@p:8080"), None, None);
    let target = url::Url::parse("wss://api.example.test").unwrap();
    let tunnel = aimux_provider_utils::ws::ws__proxy_decision_for(&target, &config)
        .expect("resolve")
        .expect("tunnel");
    assert_eq!(tunnel.authorization.as_deref(), Some("Basic dXNlcjpwYXNz"));
}

#[test]
fn ipv6_proxy_host_brackets_are_stripped_for_the_socket() {
    // `url::Url::host_str` returns "[::1]"; the resolver needs "::1".
    let config = config_with(Some("http://[::1]:8080"), None, None);
    let target = url::Url::parse("wss://api.example.test").unwrap();
    let tunnel = aimux_provider_utils::ws::ws__proxy_decision_for(&target, &config)
        .expect("resolve")
        .expect("tunnel");
    assert_eq!(tunnel.host, "::1");
    assert!(tunnel.target_tls);
}

#[test]
fn proxy_errors_do_not_leak_credentials() {
    let config = config_with(Some("socks5://user:secret@p:1080"), None, None);
    let target = url::Url::parse("wss://api.example.test").unwrap();
    let error = aimux_provider_utils::ws::ws__proxy_decision_for(&target, &config)
        .expect_err("must refuse");
    let text = error.to_string();
    assert!(
        !text.contains("secret") && !text.contains("user:secret"),
        "proxy credentials must be masked in errors, got: {text}"
    );
    assert!(
        text.contains("socks5://***@p:1080"),
        "masked URL expected, got: {text}"
    );
}

// ── Unit: no_proxy matching (reqwest NoProxy semantics) ─────────────────────

#[test]
fn no_proxy_matching_rules() {
    use aimux_provider_utils::ws::ws__no_proxy_matches as matches;
    assert!(matches("*", "anything.test", 443));
    assert!(matches("api.example.test", "api.example.test", 443));
    assert!(matches("example.test", "api.example.test", 443));
    assert!(matches(".example.test", "api.example.test", 443));
    assert!(!matches("example.test", "notexample.test", 443));
    assert!(matches("api.example.test:9000", "api.example.test", 9000));
    assert!(!matches("api.example.test:9000", "api.example.test", 443));
    assert!(matches(" a.test , b.test", "b.test", 80));
    assert!(!matches("", "api.example.test", 443));
}

// ── Live handshake smoke (manual: `cargo test -- --ignored`) ─────────────────
//
// No API key needed: the target is the REAL wss endpoint, so TLS through the
// CONNECT tunnel validates against a public CA chain and the server answers
// the WebSocket handshake itself (rejecting unauthenticated callers). Either
// outcome — an HTTP-status rejection from the handshake, or a connected
// session whose first event is an auth error/close — proves the full
// tunnel+rustls path executed against the real internet. A proxy/TLS/cert
// failure would surface as a transport error instead, which this test
// rejects.
#[tokio::test]
#[serial_test::serial]
#[ignore = "network-dependent manual smoke (RFC-0034 §2.3): exercises the real TLS+proxy path"]
async fn live_wss_handshake_through_connect_proxy() {
    let fixture = fixture();
    let mut request = WebSocketRequest {
        url: "wss://api.elevenlabs.io/v1/speech-to-text/realtime\
              ?model_id=scribe_v2_realtime&audio_format=pcm_16000&commit_strategy=manual"
            .to_string(),
        headers: Vec::new(),
        subprotocols: Vec::new(),
        abort_signal: None,
        timeout: Some(TimeoutConfiguration {
            first_chunk_ms: Some(10_000),
            chunk_ms: Some(10_000),
            step_ms: None,
            total_ms: Some(20_000),
        }),
    };
    let _ = &mut request;

    let saw_server_response = match ws_connect(&request).await {
        // Handshake rejected with a real HTTP status: tunnel + TLS + upgrade
        // all executed; the server answered.
        Err(aimux_core::AiMuxError::ApiCall(api_call)) => {
            assert!(
                api_call.status_code.is_some(),
                "expected an HTTP status from the real endpoint, got {api_call:?}"
            );
            true
        }
        // Connected: the server accepted the socket. Without a key the first
        // inbound event must be an auth-flavored error or a close — anything
        // the real server sent. A transport/TLS failure is Err here and fails
        // the match.
        Ok(mut connection) => matches!(connection.next().await, Some(Ok(_)) | Some(Err(_))),
        Err(other) => panic!("tunnel/TLS failure against the real endpoint: {other:?}"),
    };
    assert!(
        saw_server_response,
        "the real endpoint must answer the handshake"
    );
    let authorities = fixture
        .connect_authorities
        .lock()
        .expect("connect log mutex")
        .clone();
    assert!(
        authorities.contains(&"api.elevenlabs.io:443".to_string()),
        "CONNECT must target the real host, saw {authorities:?}"
    );
}
