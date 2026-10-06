//! Selected end-to-end cases from @ai-sdk/cohere 4.0.52.

#[path = "common/mock_fetch.rs"]
mod mock_fetch;

use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel};
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::LanguageModelMessage;
use aimux_core::options::{CallOptions, Tool};
use aimux_core::reranking_model::{RerankingCallOptions, RerankingDocuments, RerankingModel};
use aimux_core::result::GenerateContent;
use aimux_core::shared::Warning;
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::FunctionTool;
use aimux_core::types::{FinishReasonUnified, Usage};
use aimux_providers::cohere::{CohereProvider, CohereProviderSettings, create_cohere};
use futures::StreamExt;
use mock_fetch::{Canned, MockFetch};
use serde_json::{Value, json};

fn provider(fetch: &std::sync::Arc<MockFetch>) -> CohereProvider {
    create_cohere(CohereProviderSettings {
        api_key: Some("test-api-key".into()),
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .unwrap()
}

fn options() -> CallOptions {
    CallOptions::new(vec![
        LanguageModelMessage::System {
            content: "you are a friendly bot!".into(),
            provider_options: None,
        },
        LanguageModelMessage::user_text("Hello"),
    ])
}

fn request(streaming: bool) -> Value {
    let mut body = json!({"model":"command-r-plus","messages":[
        {"role":"system","content":"you are a friendly bot!"},
        {"role":"user","content":"Hello"}
    ]});
    if streaming {
        body["stream"] = json!(true);
    }
    body
}

fn assert_request(fetch: &MockFetch, endpoint: &str, body: &Value) {
    let seen = fetch.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "POST");
    assert_eq!(seen[0].url, format!("https://api.cohere.com/v2/{endpoint}"));
    assert_eq!(seen[0].json_body(), *body);
    assert_eq!(seen[0].headers["authorization"], "Bearer test-api-key");
    assert_eq!(seen[0].headers["content-type"], "application/json");
}

fn sse(body: String) -> Canned {
    Canned {
        status: 200,
        headers: vec![("content-type".into(), "text/event-stream".into())],
        body: body.into_bytes(),
    }
}

fn assert_usage(usage: &Usage, raw: Value) {
    assert_eq!(usage.input_tokens.total, Some(507));
    assert_eq!(usage.input_tokens.no_cache, Some(507));
    assert_eq!(usage.input_tokens.cache_read, None);
    assert_eq!(usage.input_tokens.cache_write, None);
    assert_eq!(usage.output_tokens.total, Some(10));
    assert_eq!(usage.output_tokens.text, Some(10));
    assert_eq!(usage.output_tokens.reasoning, None);
    assert_eq!(usage.raw, raw.as_object().cloned());
}

fn cohere_text() -> Value {
    serde_json::from_str(
        r###"{
  "id": "e7592632-1e3d-424f-b129-bd5f9f980f7b",
  "message": {
    "role": "assistant",
    "content": [
      {
        "type": "text",
        "text": "The capital of France is Paris."
      }
    ]
  },
  "finish_reason": "COMPLETE",
  "usage": {
    "billed_units": {
      "input_tokens": 12,
      "output_tokens": 7
    },
    "tokens": {
      "input_tokens": 507,
      "output_tokens": 10
    },
    "cached_tokens": 448
  }
}"###,
    )
    .unwrap()
}
fn cohere_tool_call() -> Value {
    serde_json::from_str(r###"{
  "id": "f201af17-e24a-4396-8f6a-98e8bf9c3432",
  "message": {
    "role": "assistant",
    "tool_plan": "I will use the weather tool to find out the weather in San Francisco. I will also use the cityAttractions tool to find out what attractions are in San Francisco.",
    "tool_calls": [
      {
        "id": "weather_dqgshstja6p9",
        "type": "function",
        "function": {
          "name": "weather",
          "arguments": "{\"location\":\"San Francisco\"}"
        }
      },
      {
        "id": "cityAttractions_dcxfx4myvx68",
        "type": "function",
        "function": {
          "name": "cityAttractions",
          "arguments": "{\"city\":\"San Francisco\"}"
        }
      }
    ]
  },
  "finish_reason": "TOOL_CALL",
  "usage": {
    "billed_units": {
      "input_tokens": 119,
      "output_tokens": 52
    },
    "tokens": {
      "input_tokens": 1549,
      "output_tokens": 103
    },
    "cached_tokens": 992
  }
}"###).unwrap()
}
fn cohere_text_chunks() -> Vec<Value> {
    r###"{"id":"321d178c-2c12-44d3-ae42-2f5510f6b1cc","type":"message-start","delta":{"message":{"role":"assistant","content":[],"tool_plan":"","tool_calls":[],"citations":[]}}}
{"type":"content-start","index":0,"delta":{"message":{"content":{"type":"text","text":""}}}}
{"type":"content-delta","index":0,"delta":{"message":{"content":{"text":"The"}}}}
{"type":"content-delta","index":0,"delta":{"message":{"content":{"text":" capital"}}}}
{"type":"content-delta","index":0,"delta":{"message":{"content":{"text":" of"}}}}
{"type":"content-delta","index":0,"delta":{"message":{"content":{"text":" France"}}}}
{"type":"content-delta","index":0,"delta":{"message":{"content":{"text":" is"}}}}
{"type":"content-delta","index":0,"delta":{"message":{"content":{"text":" Paris"}}}}
{"type":"content-delta","index":0,"delta":{"message":{"content":{"text":"."}}}}
{"type":"content-end","index":0}
{"type":"message-end","delta":{"finish_reason":"COMPLETE","usage":{"billed_units":{"input_tokens":12,"output_tokens":7},"tokens":{"input_tokens":507,"output_tokens":10},"cached_tokens":448}}}"###.lines().filter(|line| !line.trim().is_empty()).map(|line| serde_json::from_str(line).unwrap()).collect()
}

fn embedding_fixture() -> Value {
    serde_json::from_str(
        r#"{
  "id": "f5aa3e7b-f011-4c5c-a825-f94669f760e5",
  "texts": ["sunny day at the beach", "rainy day in the city"],
  "embeddings": {
    "float": [
      [0.03302002, 0.020904541, -0.019744873, -0.0625, 0.04437256],
      [-0.04660034, 0.00037765503, -0.061157227, -0.08239746, -0.010360718]
    ]
  },
  "meta": {
    "api_version": {
      "version": "2"
    },
    "billed_units": {
      "input_tokens": 10
    }
  },
  "response_type": "embeddings_by_type"
}
"#,
    )
    .unwrap()
}

fn reranking_fixture() -> Value {
    serde_json::from_str(
        r#"{
  "id": "b44fe75b-e3d3-489a-b61e-1a1aede3ef72",
  "results": [
    {
      "index": 1,
      "relevance_score": 0.10183054
    },
    {
      "index": 0,
      "relevance_score": 0.03762639
    }
  ],
  "meta": {
    "api_version": {
      "version": "2"
    },
    "billed_units": {
      "search_units": 1
    }
  }
}
"#,
    )
    .unwrap()
}

/// TS: "should extract text response" (cohere/src/cohere-chat-language-model.test.ts)
#[tokio::test]
async fn generate_text() {
    let fixture = cohere_text();
    let fetch = MockFetch::new(vec![Canned::json(&fixture)]);
    let result = provider(&fetch)
        .chat("command-r-plus")
        .do_generate(&options())
        .await
        .unwrap();
    assert_request(&fetch, "chat", &request(false));
    assert_eq!(result.request.unwrap().body, Some(request(false)));
    assert_eq!(
        result.content,
        vec![GenerateContent::Text {
            text: "The capital of France is Paris.".into(),
            provider_metadata: None,
        }]
    );
    assert_eq!(result.finish_reason.unified, FinishReasonUnified::Stop);
    assert_eq!(result.finish_reason.raw.as_deref(), Some("COMPLETE"));
    assert_usage(&result.usage, fixture["usage"].clone());
    assert!(result.warnings.is_empty());
    assert!(result.provider_metadata.is_none());
    let response = result.response.unwrap();
    assert_eq!(response.body, Some(fixture));
    assert_eq!(response.id, None);
    assert_eq!(response.timestamp, None);
    assert_eq!(response.model_id, None);
}

/// TS: "should stream text deltas" (cohere/src/cohere-chat-language-model.test.ts)
#[tokio::test]
async fn stream_text() {
    let chunks = cohere_text_chunks();
    let body = chunks
        .iter()
        .map(|chunk| {
            format!(
                "event: {}\ndata: {chunk}\n\n",
                chunk["type"].as_str().unwrap()
            )
        })
        .collect();
    let fetch = MockFetch::new(vec![sse(body)]);
    let result = provider(&fetch)
        .chat("command-r-plus")
        .do_stream(&options())
        .await
        .unwrap();
    assert_request(&fetch, "chat", &request(true));
    assert_eq!(result.request.unwrap().body, Some(request(true)));
    let parts: Vec<_> = result.stream.map(|part| part.unwrap()).collect().await;
    assert_eq!(parts.len(), 12);
    assert!(matches!(&parts[0], StreamPart::StreamStart { warnings } if warnings.is_empty()));
    assert!(
        matches!(&parts[1], StreamPart::ResponseMetadata(meta) if meta.id.as_deref() == Some("321d178c-2c12-44d3-ae42-2f5510f6b1cc"))
    );
    assert!(matches!(&parts[2], StreamPart::TextStart { id, .. } if id == "0"));
    let deltas: Vec<_> = parts
        .iter()
        .filter_map(|part| match part {
            StreamPart::TextDelta { id, delta, .. } => {
                assert_eq!(id, "0");
                Some(delta.as_str())
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        deltas,
        vec!["The", " capital", " of", " France", " is", " Paris", "."]
    );
    assert!(matches!(&parts[10], StreamPart::TextEnd { id, .. } if id == "0"));
    match &parts[11] {
        StreamPart::Finish {
            finish_reason,
            usage,
            provider_metadata,
        } => {
            assert_eq!(finish_reason.unified, FinishReasonUnified::Stop);
            assert_eq!(finish_reason.raw.as_deref(), Some("COMPLETE"));
            assert_usage(usage, chunks.last().unwrap()["delta"]["usage"].clone());
            assert!(provider_metadata.is_none());
        }
        part => panic!("expected finish, got {part:?}"),
    }
}

/// TS: "should extract tool calls" (cohere/src/cohere-chat-language-model.test.ts)
#[tokio::test]
async fn generate_tool_calls() {
    let fixture = cohere_tool_call();
    let fetch = MockFetch::new(vec![Canned::json(&fixture)]);
    let schema = json!({"type":"object","properties":{"value":{"type":"string"}},"required":["value"],"additionalProperties":false,"$schema":"http://json-schema.org/draft-07/schema#"});
    let mut opts = options();
    opts.tools = Some(vec![Tool::Function(FunctionTool::new(
        "test-tool",
        schema.clone(),
    ))]);
    let result = provider(&fetch)
        .chat("command-r-plus")
        .do_generate(&opts)
        .await
        .unwrap();
    let mut expected = request(false);
    expected["tools"] =
        json!([{"type":"function","function":{"name":"test-tool","parameters":schema}}]);
    assert_request(&fetch, "chat", &expected);
    assert_eq!(result.content.len(), 2);
    for (content, wire) in result
        .content
        .iter()
        .zip(fixture["message"]["tool_calls"].as_array().unwrap())
    {
        match content {
            GenerateContent::ToolCall(call) => {
                assert_eq!(call.tool_call_id, wire["id"]);
                assert_eq!(call.tool_name, wire["function"]["name"]);
                assert_eq!(call.input, wire["function"]["arguments"]);
                assert!(call.provider_metadata.is_none());
            }
            other => panic!("expected tool call, got {other:?}"),
        }
    }
    assert_eq!(result.finish_reason.unified, FinishReasonUnified::ToolCalls);
    assert_eq!(result.finish_reason.raw.as_deref(), Some("TOOL_CALL"));
    assert_eq!(result.usage.input_tokens.total, Some(1549));
    assert_eq!(result.usage.output_tokens.total, Some(103));
    assert_eq!(result.usage.raw, fixture["usage"].as_object().cloned());
}

/// TS: "should handle unparsable stream parts" (cohere/src/cohere-chat-language-model.test.ts)
#[tokio::test]
async fn malformed_stream_response() {
    let fetch = MockFetch::new(vec![sse(
        "event: foo-message\ndata: {unparsable}\n\n".into()
    )]);
    let result = provider(&fetch)
        .chat("command-r-plus")
        .do_stream(&options())
        .await
        .unwrap();
    assert_request(&fetch, "chat", &request(true));
    let parts: Vec<_> = result.stream.map(|part| part.unwrap()).collect().await;
    assert_eq!(parts.len(), 3);
    assert!(matches!(&parts[0], StreamPart::StreamStart { warnings } if warnings.is_empty()));
    assert!(matches!(&parts[1], StreamPart::Error { .. }));
    match &parts[2] {
        StreamPart::Finish {
            finish_reason,
            usage,
            provider_metadata,
        } => {
            assert_eq!(finish_reason.unified, FinishReasonUnified::Error);
            assert_eq!(finish_reason.raw, None);
            assert_eq!(
                serde_json::to_value(usage).unwrap(),
                serde_json::to_value(Usage::default()).unwrap()
            );
            assert!(provider_metadata.is_none());
        }
        part => panic!("expected finish, got {part:?}"),
    }
}

/// TS: "should extract embedding" (cohere/src/cohere-embedding-model.test.ts)
#[tokio::test]
async fn embed_values() {
    let fixture = embedding_fixture();
    let fetch = MockFetch::new(vec![Canned::json(&fixture)]);
    let mut opts = EmbeddingCallOptions::new("sunny day at the beach");
    opts.values.push("rainy day in the city".into());
    let result = provider(&fetch)
        .embedding("embed-english-v3.0")
        .do_embed(&opts)
        .await
        .unwrap();
    assert_request(
        &fetch,
        "embed",
        &json!({"model":"embed-english-v3.0","embedding_types":["float"],"texts":opts.values,"input_type":"search_query"}),
    );
    let expected: Vec<Vec<f32>> =
        serde_json::from_value(fixture["embeddings"]["float"].clone()).unwrap();
    assert_eq!(result.embeddings, expected);
    assert_eq!(result.usage.unwrap().tokens, 10);
    assert_eq!(result.response.unwrap().body, Some(fixture));
    assert!(result.provider_metadata.is_none());
    assert!(result.warnings.is_empty());
}

async fn rerank(object_documents: bool) {
    let fixture = reranking_fixture();
    let fetch = MockFetch::new(vec![Canned::json(&fixture)]);
    let texts = vec![
        "sunny day at the beach".to_string(),
        "rainy day in the city".to_string(),
    ];
    let documents = if object_documents {
        RerankingDocuments::Object {
            values: texts.iter().map(|text| json!({"example":text})).collect(),
        }
    } else {
        RerankingDocuments::Text {
            values: texts.clone(),
        }
    };
    let mut opts = RerankingCallOptions::new("rainy day", documents);
    opts.top_n = Some(2);
    opts.provider_options = Some(
        serde_json::from_value(json!({"cohere":{"maxTokensPerDoc":1000,"priority":1}})).unwrap(),
    );
    let result = provider(&fetch)
        .reranking("rerank-english-v3.0")
        .do_rerank(&opts)
        .await
        .unwrap();
    let wire_documents = if object_documents {
        texts
            .iter()
            .map(|text| json!({"example":text}).to_string())
            .collect::<Vec<_>>()
    } else {
        texts
    };
    assert_request(
        &fetch,
        "rerank",
        &json!({"documents":wire_documents,"model":"rerank-english-v3.0","query":"rainy day","top_n":2,"max_tokens_per_doc":1000,"priority":1}),
    );
    assert_eq!(result.ranking.len(), 2);
    assert_eq!(result.ranking[0].index, 1);
    assert_eq!(result.ranking[0].relevance_score, 0.10183054);
    assert_eq!(result.ranking[1].index, 0);
    assert_eq!(result.ranking[1].relevance_score, 0.03762639);
    assert!(result.provider_metadata.is_none());
    let warnings = result.warnings.unwrap();
    if object_documents {
        assert_eq!(warnings.len(), 1);
        assert!(
            matches!(&warnings[0], Warning::Compatibility { feature, details } if feature == "object documents" && details.as_deref() == Some("Object documents are converted to strings."))
        );
    } else {
        assert!(warnings.is_empty());
    }
    let response = result.response.unwrap();
    assert_eq!(
        response.id.as_deref(),
        Some("b44fe75b-e3d3-489a-b61e-1a1aede3ef72")
    );
    assert_eq!(response.body, Some(fixture));
}

/// TS: "should send request with stringified json documents" (cohere/src/reranking/cohere-reranking-model.test.ts)
#[tokio::test]
async fn rerank_object_documents() {
    rerank(true).await;
}

/// TS: "should send request with text documents" (cohere/src/reranking/cohere-reranking-model.test.ts)
#[tokio::test]
async fn rerank_text_documents() {
    rerank(false).await;
}
