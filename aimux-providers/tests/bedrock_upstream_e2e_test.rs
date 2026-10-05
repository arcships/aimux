#[path = "common/mock_fetch.rs"]
mod mock_fetch;

use aimux_core::AiMuxError;
use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel};
use aimux_core::image_model::{ImageCallOptions, ImageModel, ImageOutputs};
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::LanguageModelMessage;
use aimux_core::options::CallOptions;
use aimux_core::reranking_model::{RerankingCallOptions, RerankingDocuments, RerankingModel};
use aimux_core::result::GenerateContent;
use aimux_core::shared::Size;
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::FunctionTool;
use aimux_core::types::FinishReasonUnified;
use aimux_provider_utils::Resolvable;
use aimux_providers::bedrock::event_stream::encode_message;
use aimux_providers::{AmazonBedrockProviderSettings, create_amazon_bedrock};
use futures::StreamExt;
use mock_fetch::{Canned, MockFetch};
use serde_json::{Value, json};

fn provider(fetch: &std::sync::Arc<MockFetch>) -> aimux_providers::bedrock::AmazonBedrockProvider {
    create_amazon_bedrock(AmazonBedrockProviderSettings {
        api_key: Some(Resolvable::Value("test-token".into())),
        region: Some("us-west-2".into()),
        base_url: Some("https://bedrock.test".into()),
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .unwrap()
}

fn options() -> CallOptions {
    CallOptions::new(vec![LanguageModelMessage::user_text("Hello")])
}

fn chat_body() -> Value {
    json!({"messages":[{"role":"user","content":[{"text":"Hello"}]}],
        "additionalModelResponseFieldPaths":["/delta/stop_sequence"]})
}

fn stream_response(events: Value) -> Canned {
    let mut body = Vec::new();
    for event in events.as_array().unwrap() {
        let (kind, payload) = event.as_object().unwrap().iter().next().unwrap();
        body.extend(encode_message("event", kind, &payload.to_string()));
    }
    Canned {
        status: 200,
        headers: vec![(
            "content-type".into(),
            "application/vnd.amazon.eventstream".into(),
        )],
        body,
    }
}

/// TS: "should extract reasoning text with signature" (amazon-bedrock/src/amazon-bedrock-chat-language-model.test.ts)
#[tokio::test]
async fn generate_reasoning_signature_and_usage() {
    let response = json!({"output":{"message":{"role":"assistant","content":[
        {"reasoningContent":{"reasoningText":{"text":"I need to think about this problem carefully...","signature":"abc123signature"}}},
        {"text":"The answer is 42."}]}},"usage":{"inputTokens":4,"outputTokens":34,"totalTokens":38},"stopReason":"stop_sequence"});
    let fetch = MockFetch::new(vec![Canned::json(&response)]);
    let result = provider(&fetch)
        .chat("anthropic.model")
        .do_generate(&options())
        .await
        .unwrap();
    let seen = fetch.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "POST");
    assert_eq!(
        seen[0].url,
        "https://bedrock.test/model/anthropic.model/converse"
    );
    assert_eq!(seen[0].headers["authorization"], "Bearer test-token");
    assert_eq!(seen[0].json_body(), chat_body());
    assert_eq!(result.request.unwrap().body.unwrap(), chat_body());
    assert_eq!(result.response.unwrap().body.unwrap(), response);
    assert_eq!(result.content.len(), 2);
    let GenerateContent::Reasoning(reasoning) = &result.content[0] else {
        panic!("expected reasoning")
    };
    assert_eq!(
        reasoning.text,
        "I need to think about this problem carefully..."
    );
    assert_eq!(
        serde_json::to_value(&reasoning.provider_metadata).unwrap(),
        json!({"amazonBedrock":{"signature":"abc123signature"},"bedrock":{"signature":"abc123signature"}})
    );
    assert!(
        matches!(&result.content[1], GenerateContent::Text { text, .. } if text == "The answer is 42.")
    );
    assert_eq!(result.finish_reason.unified, FinishReasonUnified::Stop);
    assert_eq!(result.finish_reason.raw.as_deref(), Some("stop_sequence"));
    assert_eq!(result.usage.input_tokens.total, Some(4));
    assert_eq!(result.usage.input_tokens.no_cache, Some(4));
    assert_eq!(result.usage.output_tokens.total, Some(34));
    assert_eq!(result.usage.output_tokens.text, Some(34));
    assert_eq!(result.usage.raw, Some(response["usage"].clone()));
}

/// TS: "should stream text deltas with metadata and usage" (amazon-bedrock/src/amazon-bedrock-chat-language-model.test.ts)
#[tokio::test]
async fn stream_text_and_usage() {
    let fetch = MockFetch::new(vec![stream_response(json!([
        {"contentBlockDelta":{"contentBlockIndex":0,"delta":{"text":"Hello"}}},
        {"contentBlockDelta":{"contentBlockIndex":1,"delta":{"text":", "}}},
        {"contentBlockDelta":{"contentBlockIndex":2,"delta":{"text":"World!"}}},
        {"metadata":{"usage":{"inputTokens":4,"outputTokens":34,"totalTokens":38},"metrics":{"latencyMs":10}}},
        {"messageStop":{"stopReason":"stop_sequence"}}
    ]))]);
    let result = provider(&fetch)
        .chat("anthropic.model")
        .do_stream(&options())
        .await
        .unwrap();
    assert_eq!(
        fetch.seen()[0].url,
        "https://bedrock.test/model/anthropic.model/converse-stream"
    );
    assert_eq!(fetch.seen()[0].json_body(), chat_body());
    assert_eq!(result.request.unwrap().body.unwrap(), chat_body());
    let parts = result
        .stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(parts.len(), 9);
    assert!(matches!(&parts[0], StreamPart::StreamStart { warnings } if warnings.is_empty()));
    assert!(
        matches!(&parts[1], StreamPart::ResponseMetadata(meta) if meta.model_id.as_deref() == Some("anthropic.model"))
    );
    for (offset, id, expected) in [(2, "0", "Hello"), (4, "1", ", "), (6, "2", "World!")] {
        assert!(matches!(&parts[offset], StreamPart::TextStart { id: actual, .. } if actual == id));
        assert!(
            matches!(&parts[offset + 1], StreamPart::TextDelta { id: actual, delta, .. } if actual == id && delta == expected)
        );
    }
    let StreamPart::Finish {
        finish_reason,
        usage,
        ..
    } = &parts[8]
    else {
        panic!("expected finish")
    };
    assert_eq!(finish_reason.unified, FinishReasonUnified::Stop);
    assert_eq!(finish_reason.raw.as_deref(), Some("stop_sequence"));
    assert_eq!(usage.input_tokens.total, Some(4));
    assert_eq!(usage.input_tokens.no_cache, Some(4));
    assert_eq!(usage.input_tokens.cache_read, Some(0));
    assert_eq!(usage.input_tokens.cache_write, Some(0));
    assert_eq!(usage.output_tokens.total, Some(34));
    assert_eq!(usage.output_tokens.text, Some(34));
    assert_eq!(
        usage.raw,
        Some(json!({"inputTokens":4,"outputTokens":34,"totalTokens":38}))
    );
}

/// TS: "should stream tool deltas" (amazon-bedrock/src/amazon-bedrock-chat-language-model.test.ts)
#[tokio::test]
async fn stream_partial_tool_arguments() {
    let fetch = MockFetch::new(vec![stream_response(json!([
        {"contentBlockStart":{"contentBlockIndex":0,"start":{"toolUse":{"toolUseId":"tool-use-id","name":"test-tool"}}}},
        {"contentBlockDelta":{"contentBlockIndex":0,"delta":{"toolUse":{"input":"{\"value\":"}}}},
        {"contentBlockDelta":{"contentBlockIndex":0,"delta":{"toolUse":{"input":"\"Sparkle Day\"}"}}}},
        {"contentBlockStop":{"contentBlockIndex":0}},
        {"messageStop":{"stopReason":"tool_use"}}
    ]))]);
    let schema =
        json!({"type":"object","properties":{"value":{"type":"string"}},"required":["value"]});
    let mut opts = options();
    opts.tools = Some(vec![FunctionTool::new("test-tool", schema.clone()).into()]);
    let result = provider(&fetch)
        .chat("anthropic.model")
        .do_stream(&opts)
        .await
        .unwrap();
    let mut expected = chat_body();
    expected["toolConfig"] = json!({"tools":[{"toolSpec":{"name":"test-tool","inputSchema":{"json":schema}}}],"toolChoice":{"auto":{}}});
    assert_eq!(fetch.seen()[0].json_body(), expected);
    let parts = result
        .stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(parts.len(), 8);
    assert!(
        matches!(&parts[2], StreamPart::ToolInputStart { id, tool_name, .. } if id == "tool-use-id" && tool_name == "test-tool")
    );
    assert!(
        matches!(&parts[3], StreamPart::ToolInputDelta { id, delta, .. } if id == "tool-use-id" && delta == "{\"value\":")
    );
    assert!(
        matches!(&parts[4], StreamPart::ToolInputDelta { id, delta, .. } if id == "tool-use-id" && delta == "\"Sparkle Day\"}")
    );
    assert!(matches!(&parts[5], StreamPart::ToolInputEnd { id, .. } if id == "tool-use-id"));
    let StreamPart::ToolCall(call) = &parts[6] else {
        panic!("expected tool call")
    };
    assert_eq!(call.tool_call_id, "tool-use-id");
    assert_eq!(call.tool_name, "test-tool");
    assert_eq!(call.input, "{\"value\":\"Sparkle Day\"}");
    assert!(
        matches!(&parts[7], StreamPart::Finish { finish_reason, .. } if finish_reason.unified == FinishReasonUnified::ToolCalls && finish_reason.raw.as_deref() == Some("tool_use"))
    );
}

/// TS: "prefixes the provider message when the error type is present" (amazon-bedrock/src/amazon-bedrock-error.test.ts)
#[tokio::test]
async fn generate_error_response() {
    let response = json!({"type":"ValidationException","message":"boom"});
    let mut canned = Canned::json(&response);
    canned.status = 400;
    let fetch = MockFetch::new(vec![canned]);
    let error = provider(&fetch)
        .chat("test")
        .do_generate(&options())
        .await
        .unwrap_err();
    assert_eq!(
        fetch.seen()[0].json_body(),
        json!({"messages":[{"role":"user","content":[{"text":"Hello"}]}]})
    );
    let AiMuxError::ApiCall(error) = error else {
        panic!("expected API error")
    };
    assert_eq!(error.message, "ValidationException: boom");
    assert_eq!(error.status_code, Some(400));
    assert!(!error.is_retryable);
    assert_eq!(error.data, Some(response));
}

/// TS: "should handle single input value and return embeddings" (amazon-bedrock/src/amazon-bedrock-embedding-model.test.ts)
#[tokio::test]
async fn embed_single_value_and_usage() {
    let response = json!({"embedding":[-0.09,0.05,-0.02,0.01,0.04],"inputTextTokenCount":8});
    let fetch = MockFetch::new(vec![Canned::json(&response)]);
    let result = provider(&fetch)
        .embedding("amazon.titan-embed-text-v2:0")
        .do_embed(&EmbeddingCallOptions::new("sunny day at the beach"))
        .await
        .unwrap();
    assert_eq!(
        fetch.seen()[0].url,
        "https://bedrock.test/model/amazon.titan-embed-text-v2%3A0/invoke"
    );
    assert_eq!(
        fetch.seen()[0].json_body(),
        json!({"inputText":"sunny day at the beach"})
    );
    assert_eq!(
        result.embeddings,
        vec![vec![-0.09, 0.05, -0.02, 0.01, 0.04]]
    );
    assert_eq!(result.usage.unwrap().tokens, 8);
    assert!(result.response.is_none());
}

/// TS: "should pass the model and the settings" (amazon-bedrock/src/amazon-bedrock-image-model.test.ts)
#[tokio::test]
async fn generate_images_with_settings() {
    let fetch = MockFetch::new(vec![Canned::json(
        &json!({"images":["base64-image-1","base64-image-2"]}),
    )]);
    let mut opts = ImageCallOptions::new("A cute baby sea otter");
    opts.size = Some(Size::new(1024, 1024));
    opts.seed = Some(1234);
    opts.provider_options = serde_json::from_value(
        json!({"bedrock":{"negativeText":"bad","quality":"premium","cfgScale":1.2}}),
    )
    .unwrap();
    let result = provider(&fetch)
        .image("amazon.nova-canvas-v1:0")
        .do_generate(&opts)
        .await
        .unwrap();
    assert_eq!(
        fetch.seen()[0].url,
        "https://bedrock.test/model/amazon.nova-canvas-v1%3A0/invoke"
    );
    assert_eq!(
        fetch.seen()[0].json_body(),
        json!({"imageGenerationConfig":{"cfgScale":1.2,"height":1024,"numberOfImages":1,"quality":"premium","seed":1234,"width":1024},"taskType":"TEXT_IMAGE","textToImageParams":{"negativeText":"bad","text":"A cute baby sea otter"}})
    );
    let ImageOutputs::Base64(images) = result.images else {
        panic!("expected base64 images")
    };
    assert_eq!(images, vec!["base64-image-1", "base64-image-2"]);
}

/// TS: "should throw error when request is moderated" (amazon-bedrock/src/amazon-bedrock-image-model.test.ts)
#[tokio::test]
async fn image_moderation_error_response() {
    let fetch = MockFetch::new(vec![Canned::json(
        &json!({"id":"fe7256d1-50d9-4663-8592-85eaf002e80c","status":"Request Moderated","result":null,"progress":null,"details":{"Moderation Reasons":["Derivative Works Filter"]},"preview":null}),
    )]);
    let error = provider(&fetch)
        .image("amazon.nova-canvas-v1:0")
        .do_generate(&ImageCallOptions::new(
            "Generate something that triggers moderation",
        ))
        .await
        .unwrap_err();
    assert_eq!(
        fetch.seen()[0].json_body(),
        json!({"imageGenerationConfig":{"numberOfImages":1},"taskType":"TEXT_IMAGE","textToImageParams":{"text":"Generate something that triggers moderation"}})
    );
    assert!(
        error
            .to_string()
            .contains("Amazon Bedrock request was moderated: Derivative Works Filter")
    );
}

/// TS: "should send request with stringified json documents" (amazon-bedrock/src/reranking/amazon-bedrock-reranking-model.test.ts)
#[tokio::test]
async fn rerank_json_documents_and_parse_ranking() {
    let response = json!({"results":[{"index":0,"relevanceScore":0.5110583305358887},{"index":5,"relevanceScore":0.30241215229034424}]});
    let fetch = MockFetch::new(vec![Canned::json(&response)]);
    let mut opts = RerankingCallOptions::new(
        "rainy day",
        RerankingDocuments::Object {
            values: vec![
                json!({"example":"sunny day at the beach"}),
                json!({"example":"rainy day in the city"}),
            ],
        },
    );
    opts.top_n = Some(2);
    opts.provider_options = Some(serde_json::from_value(json!({"bedrock":{"nextToken":"test-token","additionalModelRequestFields":{"test":"test-value"}}})).unwrap());
    let result = provider(&fetch)
        .reranking("cohere.rerank-v3-5:0")
        .do_rerank(&opts)
        .await
        .unwrap();
    assert_eq!(fetch.seen()[0].url, "https://bedrock.test/rerank");
    assert_eq!(
        fetch.seen()[0].json_body(),
        json!({"nextToken":"test-token","queries":[{"textQuery":{"text":"rainy day"},"type":"TEXT"}],"rerankingConfiguration":{"bedrockRerankingConfiguration":{"modelConfiguration":{"modelArn":"arn:aws:bedrock:us-west-2::foundation-model/cohere.rerank-v3-5:0","additionalModelRequestFields":{"test":"test-value"}},"numberOfResults":2},"type":"BEDROCK_RERANKING_MODEL"},"sources":[{"type":"INLINE","inlineDocumentSource":{"type":"JSON","jsonDocument":{"example":"sunny day at the beach"}}},{"type":"INLINE","inlineDocumentSource":{"type":"JSON","jsonDocument":{"example":"rainy day in the city"}}}]})
    );
    assert_eq!(result.ranking.len(), 2);
    assert_eq!(result.ranking[0].index, 0);
    assert_eq!(result.ranking[0].relevance_score, 0.5110583305358887);
    assert_eq!(result.ranking[1].index, 5);
    assert_eq!(result.ranking[1].relevance_score, 0.30241215229034424);
    assert!(result.provider_metadata.is_none());
    assert_eq!(result.response.unwrap().body.unwrap(), response);
}
