//! Recording coverage for the single-exchange HTTP throat.

use std::sync::Arc;
use std::time::Duration;

use aimux_core::recording::{
    self, JsonlRecorder, OutcomeRecord, OutcomeStatus, Recorder, Recording,
};
use aimux_provider_utils::{
    HttpRequest, ProviderErrorParts, create_json_error_response_handler,
    create_json_response_handler, post_json_to_api,
};
use serde::Deserialize;
use serde_json::json;
use serial_test::serial;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[derive(Deserialize)]
struct Reply {
    #[allow(dead_code)]
    ok: bool,
}

fn failed() -> aimux_provider_utils::ResponseHandler<aimux_core::AiMuxError> {
    create_json_error_response_handler(|_| ProviderErrorParts {
        message: "request failed".into(),
        provider_code: None,
    })
}

fn recorder(tag: &str) -> (Arc<dyn Recorder>, std::path::PathBuf) {
    let directory = std::env::temp_dir().join(format!("aimux-rfc31-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    (Arc::new(JsonlRecorder::new(&directory)), directory)
}

fn request_with_signal(
    url: String,
    call_id: &str,
    recorder: Arc<dyn Recorder>,
    abort_signal: Option<aimux_core::AbortSignal>,
) -> HttpRequest {
    HttpRequest {
        url,
        headers: vec![
            ("authorization".into(), "Bearer secret".into()),
            ("x-custom".into(), "visible".into()),
        ],
        abort_signal,
        call_id: Some(call_id.into()),
        recording_context: Some(aimux_core::recording::RecordingContext::new(
            call_id, recorder,
        )),

        ..Default::default()
    }
}

fn request(url: String, call_id: &str, recorder: Arc<dyn Recorder>) -> HttpRequest {
    request_with_signal(url, call_id, recorder, None)
}

fn finish_call(recorder: &dyn Recorder, call_id: &str) {
    use aimux_core::content::ContentPart;
    use aimux_core::language_model_message::LanguageModelPromptMessage;
    use aimux_core::message::Role;
    use aimux_core::options::CallOptions;

    recorder.record_input(
        call_id,
        &CallOptions::new(vec![LanguageModelPromptMessage {
            role: Role::User,
            content: vec![ContentPart::text("hi")],
            ..Default::default()
        }]),
        "openai",
        "gpt-test",
    );
    recorder.record_outcome(
        call_id,
        &OutcomeRecord {
            status: OutcomeStatus::Success,
            finish_reason: Some("stop".into()),
            error: None,
            error_value: None,
            usage: None,
        },
    );
    recorder.record_transport_closed(call_id);
    recorder.flush();
}

fn read_recording(directory: &std::path::Path) -> Recording {
    let path = directory.join("recordings.jsonl");
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(text) = std::fs::read_to_string(&path)
            && let Some(line) = text.lines().next()
            && let Ok(recording) = serde_json::from_str(line)
        {
            return recording;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "recording was not flushed"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[tokio::test]
#[serial]
async fn one_helper_call_records_one_finalized_exchange() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/call"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("set-cookie", "session=secret")
                .set_body_json(json!({"ok": true})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let (recorder, directory) = recorder("success");
    recording::init_recording(Some(recorder.clone()));

    post_json_to_api(
        request(format!("{}/call", server.uri()), "call-1", recorder.clone()),
        json!({"api_key": "secret", "prompt": "hi"}),
        create_json_response_handler::<Reply>(),
        failed(),
    )
    .await
    .unwrap();
    finish_call(recorder.as_ref(), "call-1");

    let recording = read_recording(&directory);
    assert_eq!(recording.exchanges.len(), 1);
    let exchange = &recording.exchanges[0];
    assert!(exchange.finalized);
    assert_eq!(exchange.attempt, 1);
    assert_eq!(
        exchange
            .request
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
            .unwrap()
            .1,
        "[REDACTED]"
    );
    let response = exchange.response.as_ref().unwrap();
    assert_eq!(response.status, 200);
    assert!(response.body.as_deref().unwrap().contains("true"));
    assert_eq!(
        response
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("set-cookie"))
            .unwrap()
            .1,
        "[REDACTED]"
    );

    recording::init_recording(None);
    let _ = std::fs::remove_dir_all(directory);
}

#[tokio::test]
#[serial]
async fn cancelled_exchange_is_recorded_as_finalized_failure() {
    let signal = aimux_core::AbortSignal::new();
    let (recorder, directory) = recorder("cancelled");
    recording::init_recording(Some(recorder.clone()));

    let request = request_with_signal(
        "http://127.0.0.1:9/call".into(),
        "call-cancelled",
        recorder.clone(),
        Some(signal.clone()),
    );
    signal.abort();
    let result = post_json_to_api(
        request,
        json!({"prompt": "hi"}),
        create_json_response_handler::<Reply>(),
        failed(),
    )
    .await;
    assert!(matches!(result, Err(aimux_core::AiMuxError::Aborted(_))));

    finish_call(recorder.as_ref(), "call-cancelled");
    let recording = read_recording(&directory);
    assert_eq!(recording.exchanges.len(), 1);
    let exchange = &recording.exchanges[0];
    assert!(exchange.finalized);
    assert_eq!(exchange.attempt, 1);
    assert_eq!(exchange.exchange_index, 1);
    assert!(exchange.response.is_none());
    assert!(exchange.error.is_some());

    recording::init_recording(None);
    let _ = std::fs::remove_dir_all(directory);
}
#[tokio::test]
#[serial]
async fn failed_http_response_is_still_one_observed_exchange() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(503).set_body_json(json!({
            "error": {"message": "unavailable"}
        })))
        .expect(1)
        .mount(&server)
        .await;
    let (recorder, directory) = recorder("failure");
    recording::init_recording(Some(recorder.clone()));

    let result = post_json_to_api(
        request(server.uri(), "call-2", recorder.clone()),
        json!({}),
        create_json_response_handler::<Reply>(),
        failed(),
    )
    .await;
    assert!(result.is_err());
    finish_call(recorder.as_ref(), "call-2");

    let recording = read_recording(&directory);
    assert_eq!(recording.exchanges.len(), 1);
    assert_eq!(
        recording.exchanges[0].response.as_ref().unwrap().status,
        503
    );
    assert!(recording.exchanges[0].finalized);

    recording::init_recording(None);
    let _ = std::fs::remove_dir_all(directory);
}

#[tokio::test]
#[serial]
async fn redirect_chain_is_recorded_as_one_logical_exchange() {
    // RFC-0031 §13-9: hops of an automatic redirect chain are transport
    // detail; recording sees a single exchange carrying the final response.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/call"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "/moved"))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/moved"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .expect(1)
        .mount(&server)
        .await;
    let (recorder, directory) = recorder("redirect");
    recording::init_recording(Some(recorder.clone()));

    post_json_to_api(
        request(format!("{}/call", server.uri()), "call-3", recorder.clone()),
        json!({"prompt": "hi"}),
        create_json_response_handler::<Reply>(),
        failed(),
    )
    .await
    .unwrap();
    finish_call(recorder.as_ref(), "call-3");

    let recording = read_recording(&directory);
    assert_eq!(recording.exchanges.len(), 1);
    let exchange = &recording.exchanges[0];
    assert!(exchange.finalized);
    assert_eq!(exchange.attempt, 1);
    assert_eq!(exchange.exchange_index, 1);
    assert_eq!(exchange.response.as_ref().unwrap().status, 200);

    recording::init_recording(None);
    let _ = std::fs::remove_dir_all(directory);
}
