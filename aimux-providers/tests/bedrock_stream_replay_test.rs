//! Bedrock streaming replay over a `body_base64` cassette.
//!
//! Bedrock streams `application/vnd.amazon.eventstream` — length-prefixed
//! binary frames that are **not** valid UTF-8, so those cassettes carry
//! `response.body_base64` instead of `response.body`. This test is the one
//! place that proves the whole path works end to end: base64 decode → raw
//! bytes on the wire → the eventstream parser → `StreamPart`s.
//!
//! It exists because the conformance suite cannot fail when that path breaks —
//! and for a sharper reason than error tolerance. Feeding the stream bytes that
//! are not eventstream frames at all does not raise an error: it yields
//! `StreamStart`, `ResponseMetadata` and a clean `Finish { Stop }`. Three parts,
//! one of them a finish, nothing parsed. `conformance_test.rs`'s bedrock case
//! asserts `has_finish(&parts) || !parts.is_empty()`, which that garbage
//! satisfies, so the unparseable input reads as a normal completion.
//!
//! The assertions below are deliberately specific: they name the decoded text
//! the frames actually carry, so the test can only pass if the bytes really
//! travelled the whole path.

mod common;

use common::replay::mount_cassettes;
use futures::StreamExt;
use wiremock::MockServer;

use aimux_core::content::ContentPart;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelPrompt, LanguageModelPromptMessage};
use aimux_core::message::Role;
use aimux_core::options::CallOptions;
use aimux_core::stream_part::StreamPart;
use aimux_providers::{BedrockProvider, BedrockProviderConfig};

/// The one cassette recorded for this model id, so the `(method, path)` group
/// holds exactly one entry and body-scoring cannot pick a different exchange:
/// `cassettes/bedrock/test_bedrock_native_output_stream.json`.
const MODEL_ID: &str = "us.anthropic.claude-sonnet-4-5-20250929-v1:0";

fn prompt() -> LanguageModelPrompt {
    vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![ContentPart::text("What is the capital of France?")],
        ..Default::default()
    }]
}

/// Decoded frames must reach the parser and come out as text plus a finish.
///
/// Red-probe, both run locally against this cassette:
/// - `body_base64` replaced with valid base64 of non-eventstream bytes → fails
///   at the TextDelta assertion. Note it does *not* fail at `do_stream`: the
///   garbage produces a successful stream that finishes with `Stop`.
/// - frames truncated to 400 of 1687 bytes → fails at the `Paris` assertion,
///   with the partial text (`{"`) in the message.
///
/// Neither turns the conformance suite red, which is why this test exists.
#[tokio::test]
async fn body_base64_cassette_decodes_and_parses_into_stream_parts() {
    let server = MockServer::start().await;
    let mounted = mount_cassettes(&server, "tests/cassettes/bedrock").await;
    assert!(
        mounted > 0,
        "no bedrock cassettes mounted — the replay directory moved or is empty"
    );

    let config = BedrockProviderConfig::with_bearer_token("test-token", "us-east-1")
        .with_base_url(server.uri());
    let model = BedrockProvider::new(config).model(MODEL_ID);

    // No error tolerance on purpose. A failed base64 decode, or frames the
    // eventstream parser rejects, surfaces here — and must fail the test rather
    // than be waved through as "some provider error".
    let stream_result = match model.do_stream(&CallOptions::new(prompt())).await {
        Ok(result) => result,
        Err(e) => panic!(
            "do_stream failed on a body_base64 cassette: {e:?}\n\
             the decoded bytes never became a stream — check `response.body_base64` \
             handling in tests/common/replay.rs and the bedrock eventstream parser"
        ),
    };

    let mut parts = Vec::new();
    let mut stream = stream_result.stream;
    while let Some(part) = stream.next().await {
        match part {
            Ok(p) => parts.push(p),
            Err(e) => panic!("stream yielded an error mid-flight: {e:?}"),
        }
    }

    // The frames carry a JSON object spelled out across several text deltas;
    // asserting on its content proves the bytes survived decode *and* parse,
    // which "at least one part arrived" would not.
    let text: String = parts
        .iter()
        .filter_map(|p| match p {
            StreamPart::TextDelta { delta, .. } => Some(delta.as_str()),
            _ => None,
        })
        .collect();

    assert!(
        !text.is_empty(),
        "no TextDelta parts: frames decoded but produced no text — got {} parts: {:?}",
        parts.len(),
        parts
    );
    assert!(
        text.contains("Paris"),
        "decoded text does not carry the recorded content; expected it to contain \"Paris\", got {text:?}"
    );

    assert!(
        parts.iter().any(|p| matches!(p, StreamPart::Finish { .. })),
        "stream never finished: the trailing messageStop/metadata frames did not \
         become a Finish part — got {} parts",
        parts.len()
    );
}
