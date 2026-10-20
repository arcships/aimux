use std::sync::Arc;

use aimux_core::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::LanguageModelPrompt;
use aimux_core::options::CallOptions;
use aimux_core::stream_part::StreamPart;
use aimux_core::types::Usage;
use aimux_provider_utils::Resolvable;
use futures::StreamExt;
use serde_json::{Value, json};

use crate::bedrock::event_stream::encode_message;
use crate::bedrock::{AmazonBedrockProviderSettings, BedrockModel, create_amazon_bedrock};
use crate::test_support::mock_fetch::{Canned, RouteFetch};

fn prompt() -> LanguageModelPrompt {
    serde_json::from_value(json!([
        { "role": "user", "content": [{ "type": "text", "text": "Hello" }] },
    ]))
    .unwrap()
}

fn complete_usage() -> Value {
    json!({
        "inputTokens": 13,
        "outputTokens": 5,
        "totalTokens": 18,
        "cacheReadInputTokens": 3,
        "cacheWriteInputTokens": 2,
        "cacheDetails": [
            {
                "inputTokens": 2,
                "ttl": "T5M",
                "providerCacheMetadata": { "source": "provider" },
            },
        ],
        "providerUsageMetadata": { "serverToolUsage": {} },
    })
}

fn create_event_frame(event_type: &str, body: Value) -> Vec<u8> {
    encode_message("event", event_type, &body.to_string())
}

/// `createModel({ generateUsage, streamUsage })`: `/converse` answers a JSON
/// response, every other route an event stream.
fn create_model(generate_usage: Value, stream_usage: Value) -> BedrockModel {
    let fetch = RouteFetch::new(move |seen| {
        if seen.url.ends_with("/converse") {
            Canned::json(&json!({
                "output": { "message": { "role": "assistant", "content": [] } },
                "stopReason": "end_turn",
                "usage": generate_usage,
            }))
        } else {
            let mut body = create_event_frame("messageStop", json!({ "stopReason": "end_turn" }));
            body.extend(create_event_frame(
                "metadata",
                json!({ "metrics": { "latencyMs": 1 }, "usage": stream_usage }),
            ));
            Canned {
                status: 200,
                headers: vec![(
                    "content-type".into(),
                    "application/vnd.amazon.eventstream".into(),
                )],
                body,
            }
        }
    });
    create_amazon_bedrock(AmazonBedrockProviderSettings {
        base_url: Some("https://bedrock-runtime.us-east-1.amazonaws.com".into()),
        api_key: Some(Resolvable::Value("test-api-key".into())),
        fetch: Some(Arc::clone(&fetch).transport()),
        ..Default::default()
    })
    .unwrap()
    .chat("anthropic.claude-3-haiku-20240307-v1:0")
}

fn expect_normalized_usage(usage: &Usage) {
    assert_eq!(
        serde_json::to_value(&usage.input_tokens).unwrap(),
        json!({ "total": 18, "noCache": 13, "cacheRead": 3, "cacheWrite": 2 })
    );
    assert_eq!(
        serde_json::to_value(&usage.output_tokens).unwrap(),
        json!({ "total": 5, "text": 5 })
    );
}

/// `expectInvalidResponseField`: an `APICallError` with message
/// `Invalid JSON response` whose validation cause names the field. aimux
/// carries the cause inside the message (`Invalid JSON response: <cause>`),
/// and its cause is serde's text (`inputTokens: invalid type: …`), not zod's
/// issue list, where the field appears as a quoted JSON string.
fn expect_invalid_response_field(result: Result<impl std::fmt::Debug, AiMuxError>, field: &str) {
    let error = result.expect_err("expected an error");
    let AiMuxError::ApiCall(error) = error else {
        panic!("expected APICallError, got {error:?}");
    };
    let (message, cause) = error
        .message
        .split_once(": ")
        .unwrap_or((error.message.as_str(), ""));
    assert_eq!(message, "Invalid JSON response");
    assert!(
        cause.contains(field),
        "cause does not name \"{field}\": {cause}"
    );
}

async fn stream_parts(model: &BedrockModel) -> Vec<StreamPart> {
    let mut options = CallOptions::new(prompt());
    options.include_raw_chunks = Some(false);
    let result = model.do_stream(&options).await.unwrap();
    result
        .stream
        .map(|part| part.expect("stream item"))
        .collect()
        .await
}

mod amazon_bedrock_chat_language_model_raw_usage {
    use super::*;

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-usage-raw.test.ts › AmazonBedrockChatLanguageModel raw usage › preserves complete raw usage through doGenerate parsing
    #[tokio::test]
    async fn preserves_complete_raw_usage_through_do_generate_parsing() {
        let result = create_model(complete_usage(), complete_usage())
            .do_generate(&CallOptions::new(prompt()))
            .await
            .unwrap();

        assert_eq!(
            result.usage.raw.clone().map(Value::Object),
            Some(complete_usage())
        );
        expect_normalized_usage(&result.usage);
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-usage-raw.test.ts › AmazonBedrockChatLanguageModel raw usage › preserves complete raw usage through the event-stream parser
    #[tokio::test]
    async fn preserves_complete_raw_usage_through_the_event_stream_parser() {
        let parts = stream_parts(&create_model(complete_usage(), complete_usage())).await;
        let usage = parts
            .iter()
            .find_map(|part| match part {
                StreamPart::Finish { usage, .. } => Some(usage),
                _ => None,
            })
            .expect("Expected a finish part");

        assert_eq!(usage.raw.clone().map(Value::Object), Some(complete_usage()));
        expect_normalized_usage(usage);
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-usage-raw.test.ts › AmazonBedrockChatLanguageModel raw usage › validates known cache detail fields during doGenerate parsing
    #[tokio::test]
    async fn validates_known_cache_detail_fields_during_do_generate_parsing() {
        let mut generate_usage = complete_usage();
        generate_usage["cacheDetails"] = json!([{ "inputTokens": "2", "ttl": "T5M" }]);
        let model = create_model(generate_usage, complete_usage());

        expect_invalid_response_field(
            model.do_generate(&CallOptions::new(prompt())).await,
            "inputTokens",
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-usage-raw.test.ts › AmazonBedrockChatLanguageModel raw usage › validates totalTokens in streaming usage
    #[tokio::test]
    async fn validates_total_tokens_in_streaming_usage() {
        let mut stream_usage = complete_usage();
        stream_usage["totalTokens"] = json!("18");
        let parts = stream_parts(&create_model(complete_usage(), stream_usage)).await;

        assert!(
            parts.iter().any(|part| matches!(
                part,
                StreamPart::Error { error } if error.to_string().contains("totalTokens")
            )),
            "{parts:?}"
        );
    }
}
