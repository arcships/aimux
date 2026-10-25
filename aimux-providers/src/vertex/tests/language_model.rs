use serde_json::json;

use aimux_core::language_model_message::LanguageModelMessage;
use aimux_core::options::{CallOptions, Tool};
use aimux_core::tool::FunctionTool;
use aimux_provider_utils::Resolvable;

use crate::test_support::mock_fetch::{Canned, RouteFetch};
use crate::vertex::{VertexProviderSettings, create_google_vertex};

const TEST_URL: &str = "https://aiplatform.googleapis.com/v1/publishers/google/models/gemini-2.5-flash:generateContent";

/// upstream: @ai-sdk/google-vertex src/google-vertex-language-model.test.ts › should preserve local JSON Schema references in tool requests
#[tokio::test]
async fn should_preserve_local_json_schema_references_in_tool_requests() {
    let response = Canned::json(&json!({
        "candidates": [{
            "content": {"parts": [{"text": "Done"}], "role": "model"},
            "finishReason": "STOP",
            "index": 0,
        }],
        "usageMetadata": {"promptTokenCount": 1, "candidatesTokenCount": 1, "totalTokenCount": 2},
    }));
    // The AI SDK test server: `TEST_URL` answers, anything else 404.
    let server = RouteFetch::new(move |seen| {
        if seen.url == TEST_URL {
            response.clone()
        } else {
            Canned {
                status: 404,
                headers: Vec::new(),
                body: b"Not found".to_vec(),
            }
        }
    });
    let provider = create_google_vertex(VertexProviderSettings {
        api_key: Some(Resolvable::Value("test-api-key".into())),
        fetch: Some(server.transport()),
        ..Default::default()
    })
    .unwrap();
    let mut options = CallOptions::new(vec![LanguageModelMessage::user_text("Hello")]);
    options.tools = Some(vec![Tool::Function(FunctionTool {
        name: "format-date".into(),
        description: Some("Format a date".into()),
        input_schema: json!({
            "type": "object",
            "properties": {
                "locale": {"$ref": "#/$defs/Locale", "description": "Locale for formatting"},
            },
            "required": ["locale"],
            "additionalProperties": false,
            "$defs": {"Locale": {"type": "string", "enum": ["de", "en"]}},
        }),
        strict: None,
        provider_options: None,
        input_examples: None,
    })]);
    provider
        .call("gemini-2.5-flash")
        .do_generate(&options)
        .await
        .unwrap();
    assert_eq!(
        server.seen()[0].json_body()["tools"][0]["functionDeclarations"][0]["parametersJsonSchema"],
        json!({
            "type": "object",
            "properties": {
                "locale": {"$ref": "#/$defs/Locale", "description": "Locale for formatting"},
            },
            "required": ["locale"],
            "additionalProperties": false,
            "$defs": {"Locale": {"type": "string", "enum": ["de", "en"]}},
        })
    );
}
