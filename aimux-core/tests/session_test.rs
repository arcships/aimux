//! Integration tests for RFC-0024 session grouping: the `generate_text` /
//! `stream_text` entry points resolve the session id (explicit first, opt-in
//! inference) and record the call in the registered `SessionStore`.
//!
//! NOTE: all tests touching the global session store/inferer live in ONE test
//! function. `cargo test` runs test functions in parallel, and the global
//! registration is a process-wide singleton — separate parallel tests would
//! race on it. The single function keeps the assertions serial and stable.

use std::sync::Arc;

use futures::executor::block_on;

use aimux_core::error::AiMuxError;
use aimux_core::generate::{GenerateTextOptions, generate_text};
use aimux_core::language_model::LanguageModel;
use aimux_core::message::{ModelMessage, ModelPrompt};
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateResult, StreamResult};
use aimux_core::session::{
    SessionSource, SessionStore, init_session_infer, init_session_store, list_sessions,
    session_calls,
};
use aimux_core::types::{FinishReason, FinishReasonUnified, Usage};

struct MockModel;

#[async_trait::async_trait]
impl LanguageModel for MockModel {
    fn provider(&self) -> &str {
        "mock"
    }

    fn model_id(&self) -> &str {
        "mock-1"
    }

    async fn do_generate(&self, _options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        Ok(GenerateResult {
            content: vec![],
            finish_reason: FinishReason {
                unified: FinishReasonUnified::Stop,
                raw: None,
            },
            usage: Usage::default(),
            warnings: vec![],
            provider_metadata: None,
            response: Default::default(),
            request_body: None,
            response_headers: None,
        })
    }

    async fn do_stream(&self, _options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        Ok(StreamResult {
            stream: Box::pin(futures::stream::empty()),
            request_body: None,
            response_headers: None,
        })
    }
}

#[test]
fn generate_text_session_integration() {
    // Fresh store + inference off (explicit ids only).
    init_session_store(Arc::new(SessionStore::new()));
    init_session_infer(false);

    // 1. Without an explicit session_id and inference off → no grouping.
    block_on(generate_text(
        &MockModel,
        "hello",
        GenerateTextOptions::default(),
    ))
    .unwrap();
    assert!(
        list_sessions().is_empty(),
        "no store recording without session id"
    );

    // 2. Explicit session_id groups consecutive calls (steps 0, 1).
    block_on(generate_text(
        &MockModel,
        "first",
        GenerateTextOptions {
            session_id: Some("sess-1".to_string()),
            ..Default::default()
        },
    ))
    .unwrap();
    block_on(generate_text(
        &MockModel,
        "second",
        GenerateTextOptions {
            session_id: Some("sess-1".to_string()),
            ..Default::default()
        },
    ))
    .unwrap();

    let calls = session_calls("sess-1");
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].step, 0);
    assert_eq!(calls[1].step, 1);
    assert_ne!(calls[0].trace_id, calls[1].trace_id);

    let views = list_sessions();
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].session_id, "sess-1");
    assert_eq!(views[0].source, SessionSource::Explicit);
    assert_eq!(views[0].calls.len(), 2);

    // 3. Different explicit ids are separate sessions.
    block_on(generate_text(
        &MockModel,
        "other",
        GenerateTextOptions {
            session_id: Some("sess-2".to_string()),
            ..Default::default()
        },
    ))
    .unwrap();
    assert_eq!(list_sessions().len(), 2);
    assert_eq!(session_calls("sess-2").len(), 1);

    // 4. Inference on: prompt-prefix continuation joins an auto session.
    init_session_infer(true);
    block_on(generate_text(
        &MockModel,
        "u1",
        GenerateTextOptions::default(),
    ))
    .unwrap();
    let auto_ids: Vec<String> = list_sessions()
        .iter()
        .filter(|v| v.session_id.starts_with("auto-"))
        .map(|v| v.session_id.clone())
        .collect();
    assert_eq!(
        auto_ids.len(),
        1,
        "first inferred call starts an auto session"
    );

    // Continuation: ["u1", "a1", "u2"] fully contains ["u1"] as prefix.
    block_on(generate_text(
        &MockModel,
        ModelPrompt::Messages(vec![
            ModelMessage::user("u1"),
            ModelMessage::assistant("a1"),
            ModelMessage::user("u2"),
        ]),
        GenerateTextOptions::default(),
    ))
    .unwrap();
    assert_eq!(
        list_sessions()
            .iter()
            .filter(|v| v.session_id.starts_with("auto-"))
            .count(),
        1,
        "prefix continuation stays in the same auto session"
    );
    let auto = list_sessions()
        .into_iter()
        .find(|v| v.session_id.starts_with("auto-"))
        .unwrap();
    assert_eq!(auto.calls.len(), 2);
    assert_eq!(auto.source, SessionSource::Inferred);

    // 5. Explicit still wins over inference.
    block_on(generate_text(
        &MockModel,
        "u1",
        GenerateTextOptions {
            session_id: Some("sess-1".to_string()),
            ..Default::default()
        },
    ))
    .unwrap();
    assert_eq!(session_calls("sess-1").len(), 3);
}
