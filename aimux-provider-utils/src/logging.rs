//! Unified logging (RFC-0014): connect the declared-but-unused `tracing` dep.
//!
//! aimux is a **library**, never a binary — so it never auto-initializes a
//! global subscriber on its own (that would fight consumers who bring their
//! own). Two explicit entry points exist:
//!
//! - **env auto-init** (`AIMUX_LOG` / `AIMUX_LOG_LEVEL`): lazily registers a
//!   `fmt` subscriber on first HTTP call **only if** no global subscriber is
//!   registered yet. Zero-friction for FFI/binding users: set an env var and
//!   logs appear.
//! - **programmatic API** (`init_logging`): idempotent (`Once`-guarded), used
//!   by Rust consumers, the bindings' Rust side, and the `aimux_init_logging`
//!   C ABI export.
//!
//! Privacy rules (§4.3): header *values*, URL query strings, and request/
//! response bodies are **never** logged by default. Bodies only appear when
//! `AIMUX_LOG_BODY=1` **and** the level is `trace`, truncated to 4KB with
//! auth-looking JSON fields redacted.

use std::io::Write;
use std::sync::Once;

use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt;

/// Idempotency guard shared by every entry point (env / API / C ABI).
static INIT: Once = Once::new();

/// Default level: `warn` — only retries and failures are printed, normal
/// traffic stays silent.
pub const DEFAULT_LEVEL: &str = "warn";

/// Body log truncation limit (bytes), RFC-0014 §4.3.
pub const BODY_LOG_LIMIT: usize = 4096;

/// Initialize the global logger (idempotent). Every entry point funnels here.
///
/// Precedence: `AIMUX_LOG` (RUST_LOG-style directives) > `AIMUX_LOG_LEVEL`
/// (simple level name) > `level` argument > [`DEFAULT_LEVEL`].
///
/// No-ops (without registering anything) when a global subscriber is already
/// installed by the consumer — we never override the host's own logger.
fn init_once(level: Option<&str>) {
    INIT.call_once(|| {
        if tracing::dispatcher::has_been_set() {
            return;
        }
        let filter = build_filter(level);
        let _ = fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .try_init();
    });
}

/// Programmatic init: `aimux_providers::init_logging("debug")`.
///
/// Idempotent — later calls are no-ops. If the consumer already installed a
/// global subscriber, this does nothing.
pub fn init_logging(level: &str) {
    init_once(Some(level));
}

/// Lazy env-driven auto-init, called at the HTTP throat (`send`/`send_stream`).
///
/// Cheap when already initialized (`Once::is_completed` is a single atomic
/// load). Registers a subscriber only when an `AIMUX_LOG*` env var is present
/// **and** no global subscriber exists yet.
pub fn auto_init_from_env() {
    if INIT.is_completed() || tracing::dispatcher::has_been_set() {
        return;
    }
    let env_present =
        std::env::var_os("AIMUX_LOG").is_some() || std::env::var_os("AIMUX_LOG_LEVEL").is_some();
    if env_present {
        init_once(None);
    }
}

/// Whether request/response body trace logging is enabled (`AIMUX_LOG_BODY=1`).
#[must_use]
pub fn body_logging_enabled() -> bool {
    std::env::var("AIMUX_LOG_BODY")
        .map(|v| v == "1")
        .unwrap_or(false)
}

/// Build the [`EnvFilter`] honoring the precedence documented on
/// [`init_once`]. Unparseable directives fall back to `warn` rather than
/// panicking.
fn build_filter(level: Option<&str>) -> EnvFilter {
    if let Ok(directive) = std::env::var("AIMUX_LOG")
        && !directive.trim().is_empty()
    {
        return EnvFilter::try_new(directive).unwrap_or_else(|_| EnvFilter::new(DEFAULT_LEVEL));
    }
    let level = std::env::var("AIMUX_LOG_LEVEL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| level.map(str::to_owned))
        .unwrap_or_else(|| DEFAULT_LEVEL.to_owned());
    // "aimux" is the shared target prefix of every aimux crate
    // (aimux_core / aimux_providers / aimux_provider_utils / aimux_stream / aimux_ffi).
    EnvFilter::try_new(format!("aimux={level}")).unwrap_or_else(|_| EnvFilter::new(DEFAULT_LEVEL))
}

/// Truncate `body` to [`BODY_LOG_LIMIT`] bytes at a UTF-8 char boundary.
fn truncate(body: &str) -> &str {
    if body.len() <= BODY_LOG_LIMIT {
        return body;
    }
    let mut end = BODY_LOG_LIMIT;
    while end > 0 && !body.is_char_boundary(end) {
        end -= 1;
    }
    &body[..end]
}

/// Redact a body for trace logging: JSON object keys whose lowercased name
/// contains `authorization` / `api-key` / `apikey` / `key` / `token` have
/// their (string) values replaced with `***`. Non-JSON bodies pass through
/// unchanged. Always truncated to [`BODY_LOG_LIMIT`].
#[must_use]
pub fn redact_body(body: &str) -> String {
    let truncated = truncate(body);
    match serde_json::from_str::<serde_json::Value>(truncated) {
        Ok(value) => {
            serde_json::to_string(&redact_value(value)).unwrap_or_else(|_| truncated.to_owned())
        }
        Err(_) => truncated.to_owned(),
    }
}

fn redact_value(value: serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match value {
        Value::Object(map) => {
            let mut out = serde_json::Map::with_capacity(map.len());
            for (k, v) in map {
                if is_sensitive_key(&k) {
                    out.insert(k, Value::String("***".to_owned()));
                } else {
                    out.insert(k, redact_value(v));
                }
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.into_iter().map(redact_value).collect()),
        other => other,
    }
}

fn is_sensitive_key(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    ["authorization", "api-key", "apikey", "key", "token"]
        .iter()
        .any(|needle| k.contains(needle))
}

/// Small helper used by tests to capture formatted output.
#[doc(hidden)]
pub struct CaptureWriter(pub std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CaptureWriter {
    type Writer = CaptureSink;
    fn make_writer(&'a self) -> Self::Writer {
        CaptureSink(self.0.clone())
    }
}

#[doc(hidden)]
pub struct CaptureSink(pub std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl Write for CaptureSink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    /// RAII guard: cleans up the logging env vars on drop (including panic),
    /// so a failed test cannot poison the next one.
    struct EnvGuard;

    impl EnvGuard {
        fn set(name: &str, value: &str) -> Self {
            // SAFETY: test process, env access is exclusive inside the serial group.
            unsafe { std::env::set_var(name, value) };
            Self
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            // SAFETY: see set().
            unsafe {
                std::env::remove_var("AIMUX_LOG");
                std::env::remove_var("AIMUX_LOG_LEVEL");
                std::env::remove_var("AIMUX_LOG_BODY");
            }
        }
    }

    fn capture(
        filter: EnvFilter,
    ) -> (
        std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
        tracing::subscriber::DefaultGuard,
    ) {
        let captured = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let subscriber = fmt()
            .with_ansi(false)
            .with_env_filter(filter)
            .with_writer(CaptureWriter(captured.clone()))
            .finish();
        let guard = tracing::subscriber::set_default(subscriber);
        (captured, guard)
    }

    fn captured_text(captured: &std::sync::Arc<std::sync::Mutex<Vec<u8>>>) -> String {
        String::from_utf8(captured.lock().unwrap().clone()).unwrap()
    }

    #[test]
    fn redact_json_body() {
        let body = r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}],"api_key":"sk-secret","nested":{"authorization":"Bearer xyz","token":"t"}}"#;
        let out = redact_body(body);
        assert!(!out.contains("sk-secret"), "api_key value leaked: {out}");
        assert!(!out.contains("Bearer xyz"), "authorization leaked: {out}");
        assert!(
            out.contains("\"api_key\":\"***\""),
            "api_key not redacted: {out}"
        );
        assert!(
            out.contains("\"authorization\":\"***\""),
            "authorization not redacted: {out}"
        );
        assert!(
            out.contains("\"model\":\"gpt-4o\""),
            "innocent field changed: {out}"
        );
    }

    #[test]
    fn redact_truncates_long_body() {
        let body = "x".repeat(BODY_LOG_LIMIT + 500);
        let out = redact_body(&body);
        assert!(out.len() <= BODY_LOG_LIMIT);
    }

    #[test]
    fn redact_non_json_passthrough() {
        let body = "not json at all";
        assert_eq!(redact_body(body), body);
    }

    #[test]
    #[serial]
    fn filter_precedence_env_level() {
        let _g = EnvGuard::set("AIMUX_LOG_LEVEL", "debug");
        let (captured, _guard) = capture(build_filter(None));
        tracing::debug!("visible-at-debug");
        let out = captured_text(&captured);
        assert!(
            out.contains("visible-at-debug"),
            "debug event not logged: {out}"
        );
    }

    #[test]
    #[serial]
    fn filter_precedence_aimux_log_wins() {
        let _g1 = EnvGuard::set("AIMUX_LOG", "aimux_provider_utils=error");
        let _g2 = EnvGuard::set("AIMUX_LOG_LEVEL", "debug");
        let (captured, _guard) = capture(build_filter(None));
        tracing::debug!("debug-must-be-filtered");
        tracing::error!("error-must-appear");
        let out = captured_text(&captured);
        assert!(
            !out.contains("debug-must-be-filtered"),
            "AIMUX_LOG=error should suppress debug: {out}"
        );
        assert!(
            out.contains("error-must-appear"),
            "error event missing: {out}"
        );
    }

    #[test]
    #[serial]
    fn filter_defaults_to_warn() {
        let (captured, _guard) = capture(build_filter(None));
        tracing::debug!("debug-must-be-suppressed");
        tracing::warn!("warn-must-appear");
        let out = captured_text(&captured);
        assert!(
            !out.contains("debug-must-be-suppressed"),
            "default level should suppress debug: {out}"
        );
        assert!(
            out.contains("warn-must-appear"),
            "warn event missing: {out}"
        );
    }

    #[test]
    #[serial]
    fn init_is_idempotent() {
        init_logging("debug");
        init_logging("trace"); // second call must be a no-op, not a panic
        assert!(tracing::dispatcher::has_been_set());
    }
}
