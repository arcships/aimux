//! The DeepSeek provider (`aimux_providers::deepseek`): `create_deepseek`
//! settings and the recorded cassette replay. The chat
//! model's wire behaviour (ported from the `@ai-sdk/deepseek` tests) is in
//! `deepseek_chat_test.rs`.

mod common;

use futures::StreamExt;
use wiremock::MockServer;

use aimux_core::generate::{GenerateTextOptions, generate_text, stream_text};
use aimux_core::stream_part::StreamPart;
use aimux_provider_utils::Resolvable;
use aimux_providers::deepseek::{DeepSeekProvider, DeepSeekProviderSettings, create_deepseek};

fn deepseek_at(server: &MockServer) -> DeepSeekProvider {
    create_deepseek(DeepSeekProviderSettings {
        base_url: Some(server.uri()),
        api_key: Some(Resolvable::Value("test-api-key".to_string())),
        ..Default::default()
    })
    .unwrap()
}

/// The recorded DeepSeek exchanges (`tests/cassettes/deepseek`) replayed
/// through the package.
#[tokio::test]
async fn recorded_deepseek_cassettes_replay_through_the_package() {
    let server = MockServer::start().await;
    let n = common::replay::mount_cassettes(&server, "tests/cassettes/deepseek").await;
    assert!(n > 0, "no deepseek cassettes");
    let model = deepseek_at(&server).chat("deepseek-chat");

    let result = generate_text(&model, "Hello", GenerateTextOptions::default())
        .await
        .expect("generate_text should succeed with cassette replay");
    assert!(!result.text.is_empty() || !result.tool_calls.is_empty());
    assert!(result.usage.input_tokens.total.is_some());

    let result = stream_text(&model, "Hello", GenerateTextOptions::default())
        .await
        .expect("stream_text should succeed");
    let mut stream = result.stream;
    let mut finished = false;
    while let Some(part) = stream.next().await {
        if let StreamPart::Finish { .. } = part.expect("stream part") {
            finished = true;
        }
    }
    assert!(finished, "stream should finish");
}
