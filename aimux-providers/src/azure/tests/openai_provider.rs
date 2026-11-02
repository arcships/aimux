//! Port of `@ai-sdk/azure@4.0.84` `src/azure-openai-provider.test.ts`.
//!
//! The upstream test server, which answers the Azure URLs with the response a
//! test prepares, is the mock transport here: each test builds the
//! [`MockFetch`] with the canned responses it needs. The test server's own
//! response headers are reproduced by the helpers below (`content-length` for a
//! JSON body; `cache-control`, `connection` and `content-type` for a stream).
//!
//! Upstream `mockId()` ids are not injectable: aimux numbers source parts from
//! a process-wide counter, so [`renumber_source_ids`] checks the prefix and
//! renumbers them in order before comparing with the upstream `id-N` values.

use crate::azure::{AzureOpenAIProvider, AzureOpenAIProviderSettings, create_azure};
use crate::openai::internal::{
    CodeInterpreterArgs, ComparisonOperator, FileSearchArgs, FileSearchFilter,
    FileSearchFilterValue, FileSearchRanking, ImageGenerationArgs, ImageOutputFormat, ImageQuality,
    WebSearchPreviewArgs,
};
use crate::shared::aisdk_pins;
use crate::test_support::mock_fetch::{Canned, MockFetch, RouteFetch, Seen};
use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel};
use aimux_core::error::AiMuxError;
use aimux_core::image_model::{ImageCallOptions, ImageModel, ImageOutputs};
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{
    AssistantPart, FilePart, LanguageModelMessage, TextPart, UserPart,
};
use aimux_core::options::CallOptions;
use aimux_core::provider::Provider;
use aimux_core::result::{GenerateContent, GenerateResult, StreamResult};
use aimux_core::shared::{FileBytes, FileData, SharedProviderOptions};
use aimux_core::speech_model::{AudioData, SpeechCallOptions, SpeechModel};
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::{FunctionTool, Tool};
use aimux_core::transcription_model::{AudioInput, TranscriptionCallOptions, TranscriptionModel};
use aimux_provider_utils::Resolvable;
use futures::StreamExt;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use url::Url;

/// The `TEST_PROMPT` of the upstream file: one user message, "Hello".
fn test_prompt() -> CallOptions {
    CallOptions::new(vec![LanguageModelMessage::user_text("Hello")])
}

fn text_part(text: &str) -> TextPart {
    TextPart {
        text: text.into(),
        provider_options: None,
    }
}

/// A user file part whose data is a string (upstream `{ type: 'data', data }`).
fn user_file(data: &str, media_type: &str) -> UserPart {
    UserPart::File(FilePart {
        data: FileData::Data {
            data: FileBytes::Base64(data.into()),
        },
        media_type: media_type.into(),
        filename: None,
        provider_options: None,
    })
}

fn user_prompt(parts: Vec<UserPart>) -> CallOptions {
    CallOptions::new(vec![LanguageModelMessage::User {
        content: parts,
        provider_options: None,
    }])
}

/// `createAzure({ resourceName: 'test-resource', apiKey: 'test-api-key' })`.
fn default_settings() -> AzureOpenAIProviderSettings {
    AzureOpenAIProviderSettings {
        resource_name: Some("test-resource".into()),
        api_key: Some(Resolvable::Value("test-api-key".into())),
        ..Default::default()
    }
}

/// `createAzure(settings)` whose transport is the mock.
fn azure_with(
    fetch: &Arc<MockFetch>,
    settings: AzureOpenAIProviderSettings,
) -> AzureOpenAIProvider {
    create_azure(AzureOpenAIProviderSettings {
        fetch: Some(fetch.transport()),
        ..settings
    })
    .unwrap()
}

/// The upstream module-level `provider`.
fn provider(fetch: &Arc<MockFetch>) -> AzureOpenAIProvider {
    azure_with(fetch, default_settings())
}

/// The upstream `providerApiVersionChanged`.
fn provider_api_version_changed(fetch: &Arc<MockFetch>) -> AzureOpenAIProvider {
    azure_with(
        fetch,
        AzureOpenAIProviderSettings {
            api_version: Some("2025-04-01-preview".into()),
            ..default_settings()
        },
    )
}

/// `tokenProvider: async () => ...`, called on every request.
fn async_token(next: impl Fn() -> String + Send + Sync + 'static) -> Resolvable<String> {
    Resolvable::from_async_fn(move || {
        let token = next();
        async move { Ok(token) }
    })
}

/// A query parameter of a request URL (`requestUrlSearchParams.get`).
fn query_param(seen: &Seen, name: &str) -> Option<String> {
    Url::parse(&seen.url)
        .unwrap()
        .query_pairs()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
}

/// The request headers as the upstream inline snapshots show them: every header
/// but `user-agent`, which upstream checks on its own.
fn sent_headers(seen: &Seen) -> Value {
    let headers: BTreeMap<&String, &String> = seen
        .headers
        .iter()
        .filter(|(name, _)| name.as_str() != "user-agent")
        .collect();
    json!(headers)
}

/// `ai-sdk-azure/<version>` in the user agent. Upstream pins the version to
/// `0.0.0-test` through a module mock; here it is the crate's pinned version.
fn assert_user_agent(seen: &Seen) {
    let expected = format!("ai-sdk-azure/{}", aisdk_pins::AZURE.version);
    let user_agent = &seen.headers["user-agent"];
    assert!(
        user_agent.contains(&expected),
        "user-agent {user_agent:?} does not contain {expected:?}"
    );
}

/// A JSON response with the test server's headers: the body and its `content-length`.
fn json_canned(body: &Value, extra: &[(&str, &str)]) -> Canned {
    let bytes = serde_json::to_vec(body).unwrap();
    let mut headers = vec![
        ("content-type".to_string(), "application/json".to_string()),
        ("content-length".to_string(), bytes.len().to_string()),
    ];
    headers.extend(
        extra
            .iter()
            .map(|(n, v)| ((*n).to_string(), (*v).to_string())),
    );
    Canned {
        status: 200,
        headers,
        body: bytes,
    }
}

/// `prepareJsonFixtureResponse`: a recorded JSON response, unchanged.
fn fixture_json(text: &str, extra: &[(&str, &str)]) -> Canned {
    json_canned(&serde_json::from_str::<Value>(text).unwrap(), extra)
}

/// The SSE body of `prepareChunksFixtureResponse`: each non-empty line of the
/// chunk file as a `data:` event, then `[DONE]`.
fn sse_body(chunks: &str) -> Vec<u8> {
    let mut body = String::new();
    for line in chunks.split('\n').filter(|line| !line.trim().is_empty()) {
        body.push_str("data: ");
        body.push_str(line);
        body.push_str("\n\n");
    }
    body.push_str("data: [DONE]\n\n");
    body.into_bytes()
}

/// A stream response with the test server's headers.
fn sse_canned(body: Vec<u8>, extra: &[(&str, &str)]) -> Canned {
    let mut headers = vec![
        ("cache-control".to_string(), "no-cache".to_string()),
        ("connection".to_string(), "keep-alive".to_string()),
        ("content-type".to_string(), "text/event-stream".to_string()),
    ];
    headers.extend(
        extra
            .iter()
            .map(|(n, v)| ((*n).to_string(), (*v).to_string())),
    );
    Canned {
        status: 200,
        headers,
        body,
    }
}

/// `prepareChunksFixtureResponse` for a recorded chunk file.
fn chunks_canned(chunks: &str, extra: &[(&str, &str)]) -> Canned {
    sse_canned(sse_body(chunks), extra)
}

/// The `prepareJsonResponse` body of the Responses API tests.
fn responses_body(content: &str) -> Value {
    json!({
        "id": "resp_67c97c0203188190a025beb4a75242bc",
        "object": "response",
        "created_at": 1741257730,
        "status": "completed",
        "model": "test-deployment",
        "output": [{
            "id": "msg_67c97c02656c81908e080dfdf4a03cd1",
            "type": "message",
            "status": "completed",
            "role": "assistant",
            "content": [{"type": "output_text", "text": content, "annotations": []}],
        }],
        "usage": {"input_tokens": 4, "output_tokens": 30, "total_tokens": 34},
        "incomplete_details": null,
    })
}

/// The `prepareJsonResponse` body of the Chat Completions tests.
fn chat_body(content: &str) -> Value {
    json!({
        "id": "chatcmpl-95ZTZkhr0mHNKqerQfiwkuox3PHAd",
        "object": "chat.completion",
        "created": 1711115037,
        "model": "gpt-3.5-turbo-0125",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": content},
            "finish_reason": "stop",
        }],
        "usage": {"prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30},
        "system_fingerprint": "fp_3bc1b5746c",
    })
}

/// The response of the recorded `azure-text.1` fixture.
const AZURE_TEXT_1: &str = include_str!("fixtures/azure-text.1.json");

/// The upstream snapshots number source parts `id-0`, `id-1`, ... (the AI SDK's
/// `mockId()`). aimux's come from a process-wide counter (`source-N`), so each
/// source's id is checked for that prefix and renumbered in document order.
fn renumber_source_ids(value: &mut Value) {
    fn walk(value: &mut Value, next: &mut usize) {
        match value {
            Value::Array(items) => {
                for item in items {
                    walk(item, next);
                }
            }
            Value::Object(map) => {
                if map.contains_key("sourceType") {
                    let id = map["id"].as_str().expect("source id").to_string();
                    assert!(id.starts_with("source-"), "unexpected source id {id}");
                    map.insert("id".into(), json!(format!("id-{next}")));
                    *next += 1;
                }
                for item in map.values_mut() {
                    walk(item, next);
                }
            }
            _ => {}
        }
    }
    walk(value, &mut 0);
}

/// A `doGenerate` result as the upstream snapshots show it.
fn generate_json(result: &GenerateResult) -> Value {
    let mut value = serde_json::to_value(result).unwrap();
    renumber_source_ids(&mut value);
    value
}

/// A `doGenerate` result's `content`, the value upstream's `expect(result.content)` snapshots.
fn content_json(result: &GenerateResult) -> Value {
    let mut value = serde_json::to_value(&result.content).unwrap();
    renumber_source_ids(&mut value);
    value
}

/// Collects a `doStream` result into JSON: the array of parts upstream's
/// `convertReadableStreamToArray` gives.
async fn stream_json(result: StreamResult) -> Value {
    let parts = result
        .stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<StreamPart>, _>>()
        .unwrap();
    let mut value = serde_json::to_value(&parts).unwrap();
    renumber_source_ids(&mut value);
    value
}

/// The code interpreter tool of the Azure tool set.
fn code_interpreter_tool(provider: &AzureOpenAIProvider) -> Tool {
    Tool::Provider(
        provider
            .tools
            .code_interpreter(CodeInterpreterArgs::default())
            .unwrap(),
    )
}

/// The `openai.file_search` tool of the upstream describes; `with_ranking`
/// adds the ranking the `beforeEach` blocks use.
fn file_search_tool(provider: &AzureOpenAIProvider, with_ranking: bool) -> Tool {
    Tool::Provider(
        provider
            .tools
            .file_search(FileSearchArgs {
                vector_store_ids: vec!["vs_68caad8bd5d88191ab766cf043d89a18".into()],
                max_num_results: Some(5),
                ranking: with_ranking.then(|| FileSearchRanking {
                    ranker: Some("auto".into()),
                    score_threshold: Some(0.5),
                }),
                filters: Some(FileSearchFilter::Comparison {
                    key: "author".into(),
                    r#type: ComparisonOperator::Eq,
                    value: FileSearchFilterValue::String("Jane Smith".into()),
                }),
            })
            .unwrap(),
    )
}

/// The `openai.web_search_preview` tool with no arguments.
fn web_search_preview_tool(provider: &AzureOpenAIProvider) -> Tool {
    Tool::Provider(
        provider
            .tools
            .web_search_preview(WebSearchPreviewArgs::default())
            .unwrap(),
    )
}

/// The `openai.image_generation` tool with the arguments of the upstream `beforeEach`.
fn image_generation_tool(provider: &AzureOpenAIProvider) -> Tool {
    Tool::Provider(
        provider
            .tools
            .image_generation(ImageGenerationArgs {
                output_format: Some(ImageOutputFormat::Webp),
                quality: Some(ImageQuality::Low),
                size: Some("1024x1024".into()),
                partial_images: Some(2),
                ..Default::default()
            })
            .unwrap(),
    )
}

/// The `calculator` function tool of the reasoning tests.
fn calculator_tool() -> Tool {
    Tool::Function(FunctionTool::new(
        "calculator",
        json!({
            "type": "object",
            "properties": {
                "a": {"type": "number"},
                "b": {"type": "number"},
                "op": {"type": "string"},
            },
            "required": ["a", "b"],
            "additionalProperties": false,
        }),
    ))
}

/// The `providerOptions` of one namespace.
fn provider_options(namespace: &str, options: Value) -> SharedProviderOptions {
    aimux_core::shared::provider_namespace(namespace, options).unwrap()
}

/// The `input` the Responses tests expect for the "Hello" prompt.
fn hello_input() -> Value {
    json!([{
        "content": [{"text": "Hello", "type": "input_text"}],
        "role": "user",
    }])
}

// ── The harness of the older end-to-end ports ─────────────────────────────────

fn e2e_provider(fetch: &Arc<MockFetch>) -> AzureOpenAIProvider {
    create_azure(AzureOpenAIProviderSettings {
        resource_name: Some("test-resource".into()),
        api_key: Some(Resolvable::Value("test-api-key".into())),
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .unwrap()
}

fn prompt() -> CallOptions {
    CallOptions::new(vec![LanguageModelMessage::user_text("Hello")])
}

fn assert_request(fetch: &MockFetch, endpoint: &str) -> Value {
    let seen = fetch.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "POST");
    assert_eq!(
        seen[0].url,
        format!("https://test-resource.openai.azure.com/openai/v1/{endpoint}?api-version=v1")
    );
    assert_eq!(seen[0].headers["api-key"], "test-api-key");
    seen[0].json_body()
}
fn chat_response() -> Value {
    json!({"id":"chatcmpl-1","object":"chat.completion","created":1711115037,"model":"test-deployment","choices":[{"index":0,"message":{"role":"assistant","content":"Hello World!"},"finish_reason":"stop"}],"usage":{"prompt_tokens":4,"completion_tokens":30,"total_tokens":34}})
}

fn text_fixture() -> Value {
    json!({"id":"resp_0d6bb044bb6ff37200698c51948054819385e24e2ad931ae6e","object":"response","created_at":1770803604,"status":"completed","model":"test-deployment","output":[{"id":"msg_0d6bb044bb6ff37200698c51952e288193beb9044db4d8c810","type":"message","status":"completed","role":"assistant","content":[{"type":"output_text","text":"Word","annotations":[],"logprobs":[]}]}],"usage":{"input_tokens":11,"input_tokens_details":{"cached_tokens":0},"output_tokens":11,"output_tokens_details":{"reasoning_tokens":0},"total_tokens":22}})
}

fn image_response() -> Value {
    json!({"created":1733837122,"data":[{"revised_prompt":"A charming visual illustration of a baby sea otter swimming joyously.","b64_json":"base64-image-1"},{"revised_prompt":"A charming visual illustration of a baby sea otter swimming joyously.","b64_json":"base64-image-2"}]})
}

fn text_chunks() -> &'static str {
    r###"data: {"type":"response.created","sequence_number":0,"response":{"id":"resp_02ce8deeb6197db200698c5196e9588197a572bbea62d38cd1","object":"response","created_at":1770803606,"status":"in_progress","background":false,"completed_at":null,"content_filters":null,"error":null,"incomplete_details":null,"instructions":null,"max_output_tokens":null,"max_tool_calls":null,"model":"test-deployment","output":[],"parallel_tool_calls":true,"previous_response_id":null,"prompt_cache_key":null,"prompt_cache_retention":null,"reasoning":{"effort":"none","summary":null},"safety_identifier":null,"service_tier":"auto","store":true,"temperature":1,"text":{"format":{"type":"text"},"verbosity":"medium"},"tool_choice":"auto","tools":[],"top_logprobs":0,"top_p":1,"truncation":"disabled","usage":null,"user":null,"metadata":{}}}

data: {"type":"response.in_progress","sequence_number":1,"response":{"id":"resp_02ce8deeb6197db200698c5196e9588197a572bbea62d38cd1","object":"response","created_at":1770803606,"status":"in_progress","background":false,"completed_at":null,"content_filters":null,"error":null,"incomplete_details":null,"instructions":null,"max_output_tokens":null,"max_tool_calls":null,"model":"test-deployment","output":[],"parallel_tool_calls":true,"previous_response_id":null,"prompt_cache_key":null,"prompt_cache_retention":null,"reasoning":{"effort":"none","summary":null},"safety_identifier":null,"service_tier":"auto","store":true,"temperature":1,"text":{"format":{"type":"text"},"verbosity":"medium"},"tool_choice":"auto","tools":[],"top_logprobs":0,"top_p":1,"truncation":"disabled","usage":null,"user":null,"metadata":{}}}

data: {"type":"response.output_item.added","sequence_number":2,"output_index":0,"item":{"id":"msg_02ce8deeb6197db200698c5198ca0c81979bedbe6c98a8ab93","type":"message","status":"in_progress","content":[],"role":"assistant"}}

data: {"type":"response.content_part.added","sequence_number":3,"item_id":"msg_02ce8deeb6197db200698c5198ca0c81979bedbe6c98a8ab93","output_index":0,"content_index":0,"part":{"type":"output_text","annotations":[],"logprobs":[],"text":""}}

data: {"type":"response.output_text.delta","sequence_number":4,"item_id":"msg_02ce8deeb6197db200698c5198ca0c81979bedbe6c98a8ab93","output_index":0,"content_index":0,"delta":"Hello","logprobs":[],"obfuscation":"VLMdSoETMf5"}

data: {"type":"response.output_text.done","sequence_number":5,"item_id":"msg_02ce8deeb6197db200698c5198ca0c81979bedbe6c98a8ab93","output_index":0,"content_index":0,"text":"Hello","logprobs":[]}

data: {"type":"response.content_part.done","sequence_number":6,"item_id":"msg_02ce8deeb6197db200698c5198ca0c81979bedbe6c98a8ab93","output_index":0,"content_index":0,"part":{"type":"output_text","annotations":[],"logprobs":[],"text":"Hello"}}

data: {"type":"response.output_item.done","sequence_number":7,"output_index":0,"item":{"id":"msg_02ce8deeb6197db200698c5198ca0c81979bedbe6c98a8ab93","type":"message","status":"completed","content":[{"type":"output_text","annotations":[],"logprobs":[],"text":"Hello"}],"role":"assistant"}}

data: {"type":"response.completed","sequence_number":8,"response":{"id":"resp_02ce8deeb6197db200698c5196e9588197a572bbea62d38cd1","object":"response","created_at":1770803606,"status":"completed","background":false,"completed_at":1770803608,"content_filters":[{"blocked":false,"source_type":"prompt","content_filter_raw":null,"content_filter_results":{"self_harm":{"filtered":false,"severity":"safe"},"sexual":{"filtered":false,"severity":"safe"},"violence":{"filtered":false,"severity":"safe"},"jailbreak":{"filtered":false,"detected":false},"hate":{"filtered":false,"severity":"safe"}},"content_filter_offsets":{"start_offset":1429,"end_offset":1447,"check_offset":0}},{"blocked":false,"source_type":"completion","content_filter_raw":null,"content_filter_results":{"hate":{"filtered":false,"severity":"safe"},"sexual":{"filtered":false,"severity":"safe"},"self_harm":{"filtered":false,"severity":"safe"},"violence":{"filtered":false,"severity":"safe"},"protected_material_text":{"filtered":false,"detected":false},"protected_material_code":{"filtered":false,"detected":false}},"content_filter_offsets":{"start_offset":0,"end_offset":5,"check_offset":0}}],"error":null,"incomplete_details":null,"instructions":null,"max_output_tokens":null,"max_tool_calls":null,"model":"test-deployment","output":[{"id":"msg_02ce8deeb6197db200698c5198ca0c81979bedbe6c98a8ab93","type":"message","status":"completed","content":[{"type":"output_text","annotations":[],"logprobs":[],"text":"Hello"}],"role":"assistant"}],"parallel_tool_calls":true,"previous_response_id":null,"prompt_cache_key":null,"prompt_cache_retention":null,"reasoning":{"effort":"none","summary":null},"safety_identifier":null,"service_tier":"default","store":true,"temperature":1,"text":{"format":{"type":"text"},"verbosity":"medium"},"tool_choice":"auto","tools":[],"top_logprobs":0,"top_p":1,"truncation":"disabled","usage":{"input_tokens":11,"input_tokens_details":{"cached_tokens":0},"output_tokens":11,"output_tokens_details":{"reasoning_tokens":0},"total_tokens":22},"user":null,"metadata":{}}}

data: [DONE]

"###
}

mod responses_default_language_model {
    use super::*;

    mod do_generate {
        use super::*;

        /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › responses (default language model) › doGenerate › should set the correct default api version
        #[tokio::test]
        async fn should_set_the_correct_default_api_version() {
            let fetch = MockFetch::new(vec![json_canned(&responses_body(""), &[])]);
            provider(&fetch)
                .call("test-deployment")
                .do_generate(&test_prompt())
                .await
                .unwrap();
            assert_eq!(
                query_param(&fetch.seen()[0], "api-version").as_deref(),
                Some("v1")
            );
        }

        /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › responses (default language model) › doGenerate › should set the correct modified api version
        #[tokio::test]
        async fn should_set_the_correct_modified_api_version() {
            let fetch = MockFetch::new(vec![json_canned(&responses_body(""), &[])]);
            provider_api_version_changed(&fetch)
                .call("test-deployment")
                .do_generate(&test_prompt())
                .await
                .unwrap();
            assert_eq!(
                query_param(&fetch.seen()[0], "api-version").as_deref(),
                Some("2025-04-01-preview")
            );
        }

        /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › responses (default language model) › doGenerate › should pass headers
        #[tokio::test]
        async fn should_pass_headers() {
            let fetch = MockFetch::new(vec![json_canned(&responses_body(""), &[])]);
            let provider = azure_with(
                &fetch,
                AzureOpenAIProviderSettings {
                    headers: Some(HashMap::from([(
                        "Custom-Provider-Header".into(),
                        Some("provider-header-value".into()),
                    )])),
                    ..default_settings()
                },
            );
            provider
                .call("test-deployment")
                .do_generate(&CallOptions {
                    headers: Some(HashMap::from([(
                        "Custom-Request-Header".into(),
                        "request-header-value".into(),
                    )])),
                    ..test_prompt()
                })
                .await
                .unwrap();
            let seen = &fetch.seen()[0];
            assert_eq!(
                sent_headers(seen),
                json!({
                    "api-key": "test-api-key",
                    "content-type": "application/json",
                    "custom-provider-header": "provider-header-value",
                    "custom-request-header": "request-header-value",
                })
            );
            assert_user_agent(seen);
        }

        /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › responses (default language model) › doGenerate › should use tokenProvider for Microsoft Entra ID auth
        #[tokio::test]
        async fn should_use_token_provider_for_microsoft_entra_id_auth() {
            let fetch = MockFetch::new(vec![json_canned(&responses_body(""), &[])]);
            let provider = azure_with(
                &fetch,
                AzureOpenAIProviderSettings {
                    resource_name: Some("test-resource".into()),
                    token_provider: Some(async_token(|| "test-azure-ad-token".into())),
                    ..Default::default()
                },
            );
            provider
                .call("test-deployment")
                .do_generate(&test_prompt())
                .await
                .unwrap();
            let seen = &fetch.seen()[0];
            assert_eq!(
                sent_headers(seen),
                json!({
                    "authorization": "Bearer test-azure-ad-token",
                    "content-type": "application/json",
                })
            );
            assert_user_agent(seen);
        }

        /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › responses (default language model) › doGenerate › should call tokenProvider for every request
        #[tokio::test]
        async fn should_call_token_provider_for_every_request() {
            let fetch = MockFetch::new(vec![
                json_canned(&responses_body(""), &[]),
                json_canned(&responses_body(""), &[]),
            ]);
            let token_count = Arc::new(AtomicUsize::new(0));
            let calls = token_count.clone();
            let provider = azure_with(
                &fetch,
                AzureOpenAIProviderSettings {
                    resource_name: Some("test-resource".into()),
                    token_provider: Some(async_token(move || {
                        let n = calls.fetch_add(1, Ordering::SeqCst) + 1;
                        format!("token-{n}")
                    })),
                    ..Default::default()
                },
            );
            provider
                .call("test-deployment")
                .do_generate(&test_prompt())
                .await
                .unwrap();
            provider
                .call("test-deployment")
                .do_generate(&test_prompt())
                .await
                .unwrap();

            assert_eq!(token_count.load(Ordering::SeqCst), 2);
            let seen = fetch.seen();
            assert_eq!(seen[0].headers["authorization"], "Bearer token-1");
            assert_eq!(seen[1].headers["authorization"], "Bearer token-2");
        }

        /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › responses (default language model) › doGenerate › should reject explicit apiKey with tokenProvider
        #[test]
        fn should_reject_explicit_api_key_with_token_provider() {
            let error = create_azure(AzureOpenAIProviderSettings {
                resource_name: Some("test-resource".into()),
                api_key: Some(Resolvable::Value("test-api-key".into())),
                token_provider: Some(async_token(|| "test-azure-ad-token".into())),
                ..Default::default()
            })
            .err()
            .expect("creating the provider must fail");
            assert!(matches!(
                error,
                AiMuxError::InvalidArgument { ref message, .. }
                    if message == "Both apiKey and tokenProvider were provided. Please use only one authentication method."
            ));
        }

        /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › responses (default language model) › doGenerate › should use the baseURL correctly
        #[tokio::test]
        async fn should_use_the_base_url_correctly() {
            let fetch = MockFetch::new(vec![json_canned(&responses_body(""), &[])]);
            let provider = azure_with(
                &fetch,
                AzureOpenAIProviderSettings {
                    resource_name: None,
                    base_url: Some("https://test-resource.openai.azure.com/openai".into()),
                    ..default_settings()
                },
            );
            provider
                .call("test-deployment")
                .do_generate(&test_prompt())
                .await
                .unwrap();
            assert_eq!(
                fetch.seen()[0].url,
                "https://test-resource.openai.azure.com/openai/v1/responses?api-version=v1"
            );
        }

        /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › responses (default language model) › doGenerate › should include explicit message item types for Foundry project endpoints
        #[tokio::test]
        async fn should_include_explicit_message_item_types_for_foundry_project_endpoints() {
            let fetch = MockFetch::new(vec![json_canned(&responses_body(""), &[])]);
            let provider = azure_with(
                &fetch,
                AzureOpenAIProviderSettings {
                    resource_name: None,
                    base_url: Some(
                        "https://test-resource.services.ai.azure.com/api/projects/test-project/openai/v1"
                            .into(),
                    ),
                    ..default_settings()
                },
            );
            provider
                .call("test-deployment")
                .do_generate(&CallOptions::new(vec![
                    LanguageModelMessage::System {
                        content: "You are concise.".into(),
                        provider_options: None,
                    },
                    LanguageModelMessage::User {
                        content: vec![UserPart::Text(text_part("Say hi."))],
                        provider_options: None,
                    },
                    LanguageModelMessage::Assistant {
                        content: vec![AssistantPart::Text(text_part("Hi."))],
                        provider_options: None,
                    },
                    LanguageModelMessage::User {
                        content: vec![UserPart::Text(text_part("Say it again."))],
                        provider_options: None,
                    },
                ]))
                .await
                .unwrap();

            let seen = &fetch.seen()[0];
            assert_eq!(
                seen.url,
                "https://test-resource.services.ai.azure.com/api/projects/test-project/openai/v1/responses"
            );
            assert_eq!(
                seen.json_body()["input"],
                json!([
                    {"type": "message", "role": "system", "content": "You are concise."},
                    {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "Say hi."}]},
                    {"type": "message", "role": "assistant", "content": "Hi."},
                    {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "Say it again."}]},
                ])
            );
        }
    }
}

mod chat {
    use super::*;

    /// The `prepareJsonResponse` of the Chat Completions tests, with no content.
    fn chat_canned() -> Canned {
        json_canned(&chat_body(""), &[])
    }

    mod do_generate {
        use super::*;

        /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › chat › doGenerate › should set the correct default api version
        #[tokio::test]
        async fn should_set_the_correct_default_api_version() {
            let fetch = MockFetch::new(vec![chat_canned()]);
            provider(&fetch)
                .chat("test-deployment")
                .do_generate(&test_prompt())
                .await
                .unwrap();
            assert_eq!(
                query_param(&fetch.seen()[0], "api-version").as_deref(),
                Some("v1")
            );
        }

        /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › chat › doGenerate › should set the correct modified api version
        #[tokio::test]
        async fn should_set_the_correct_modified_api_version() {
            let fetch = MockFetch::new(vec![chat_canned()]);
            provider_api_version_changed(&fetch)
                .chat("test-deployment")
                .do_generate(&test_prompt())
                .await
                .unwrap();
            assert_eq!(
                query_param(&fetch.seen()[0], "api-version").as_deref(),
                Some("2025-04-01-preview")
            );
        }

        /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › chat › doGenerate › should pass headers
        #[tokio::test]
        async fn should_pass_headers() {
            let fetch = MockFetch::new(vec![chat_canned()]);
            let provider = azure_with(
                &fetch,
                AzureOpenAIProviderSettings {
                    headers: Some(HashMap::from([(
                        "Custom-Provider-Header".into(),
                        Some("provider-header-value".into()),
                    )])),
                    ..default_settings()
                },
            );
            provider
                .chat("test-deployment")
                .do_generate(&CallOptions {
                    headers: Some(HashMap::from([(
                        "Custom-Request-Header".into(),
                        "request-header-value".into(),
                    )])),
                    ..test_prompt()
                })
                .await
                .unwrap();
            let seen = &fetch.seen()[0];
            assert_eq!(
                sent_headers(seen),
                json!({
                    "api-key": "test-api-key",
                    "content-type": "application/json",
                    "custom-provider-header": "provider-header-value",
                    "custom-request-header": "request-header-value",
                })
            );
            assert_user_agent(seen);
        }

        /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › chat › doGenerate › should use the baseURL correctly
        #[tokio::test]
        async fn should_use_the_base_url_correctly() {
            let fetch = MockFetch::new(vec![chat_canned()]);
            let provider = azure_with(
                &fetch,
                AzureOpenAIProviderSettings {
                    resource_name: None,
                    base_url: Some("https://test-resource.openai.azure.com/openai".into()),
                    ..default_settings()
                },
            );
            provider
                .chat("test-deployment")
                .do_generate(&test_prompt())
                .await
                .unwrap();
            assert_eq!(
                fetch.seen()[0].url,
                "https://test-resource.openai.azure.com/openai/v1/chat/completions?api-version=v1"
            );
        }

        /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › chat › doGenerate › should use $name baseURL correctly
        #[tokio::test]
        async fn should_use_name_base_url_correctly() {
            // (name, baseURL, the URL the request must go to)
            let cases = [
                (
                    "complete Azure OpenAI v1",
                    "https://test-resource.openai.azure.com/openai/v1",
                    "https://test-resource.openai.azure.com/openai/v1/chat/completions",
                ),
                (
                    "unversioned Foundry",
                    "https://test-resource.services.ai.azure.com/openai",
                    "https://test-resource.services.ai.azure.com/openai/v1/chat/completions?api-version=v1",
                ),
                (
                    "complete Foundry v1",
                    "https://test-resource.services.ai.azure.com/openai/v1/",
                    "https://test-resource.services.ai.azure.com/openai/v1/chat/completions",
                ),
                (
                    "unversioned Cognitive Services",
                    "https://test-resource.cognitiveservices.azure.com/openai",
                    "https://test-resource.cognitiveservices.azure.com/openai/v1/chat/completions?api-version=v1",
                ),
                (
                    "complete Cognitive Services v1",
                    "https://test-resource.cognitiveservices.azure.com/openai/v1",
                    "https://test-resource.cognitiveservices.azure.com/openai/v1/chat/completions",
                ),
                (
                    "unversioned Foundry project",
                    "https://test-resource.services.ai.azure.com/api/projects/test-project/openai",
                    "https://test-resource.services.ai.azure.com/api/projects/test-project/openai/v1/chat/completions",
                ),
                (
                    "complete Foundry project v1",
                    "https://test-resource.services.ai.azure.com/api/projects/test-project/openai/v1",
                    "https://test-resource.services.ai.azure.com/api/projects/test-project/openai/v1/chat/completions",
                ),
            ];
            for (name, base_url, expected_url) in cases {
                let fetch = MockFetch::new(vec![chat_canned()]);
                let provider = azure_with(
                    &fetch,
                    AzureOpenAIProviderSettings {
                        resource_name: None,
                        base_url: Some(base_url.into()),
                        ..default_settings()
                    },
                );
                provider
                    .chat("test-deployment")
                    .do_generate(&test_prompt())
                    .await
                    .unwrap();
                assert_eq!(fetch.seen()[0].url, expected_url, "{name}");
            }
        }

        /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › chat › doGenerate › should use custom gateway baseURL as-is
        #[tokio::test]
        async fn should_use_custom_gateway_base_url_as_is() {
            let fetch = MockFetch::new(vec![json_canned(
                &json!({
                    "id": "chatcmpl-repro-13956",
                    "object": "chat.completion",
                    "created": 0,
                    "model": "test-deployment",
                    "choices": [{
                        "index": 0,
                        "message": {"role": "assistant", "content": "ok"},
                        "finish_reason": "stop",
                    }],
                    "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2},
                }),
                &[],
            )]);
            let provider = azure_with(
                &fetch,
                AzureOpenAIProviderSettings {
                    resource_name: None,
                    base_url: Some("https://our-gateway.example.com/azure".into()),
                    ..default_settings()
                },
            );
            provider
                .chat("test-deployment")
                .do_generate(&test_prompt())
                .await
                .unwrap();
            assert_eq!(
                fetch.seen()[0].url,
                "https://our-gateway.example.com/azure/chat/completions"
            );
        }
    }
}

mod transcription {
    use super::*;

    mod do_generate {
        use super::*;

        fn transcription_canned() -> Canned {
            json_canned(
                &json!({"text": "Hello, world!", "segments": [], "language": "en", "duration": 5.0}),
                &[],
            )
        }

        /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › transcription › doGenerate › should use correct URL format
        #[tokio::test]
        async fn should_use_correct_url_format() {
            let fetch = MockFetch::new(vec![transcription_canned()]);
            provider(&fetch)
                .transcription("whisper-1")
                .do_generate(&TranscriptionCallOptions::new(
                    AudioInput::Binary(vec![]),
                    "audio/wav",
                ))
                .await
                .unwrap();
            assert_eq!(
                fetch.seen()[0].url,
                "https://test-resource.openai.azure.com/openai/v1/audio/transcriptions?api-version=v1"
            );
        }

        /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › transcription › doGenerate › should use deployment-based URL format when useDeploymentBasedUrls is true
        #[tokio::test]
        async fn should_use_deployment_based_url_format_when_use_deployment_based_urls_is_true() {
            let fetch = MockFetch::new(vec![transcription_canned()]);
            let provider = azure_with(
                &fetch,
                AzureOpenAIProviderSettings {
                    use_deployment_based_urls: true,
                    ..default_settings()
                },
            );
            provider
                .transcription("whisper-1")
                .do_generate(&TranscriptionCallOptions::new(
                    AudioInput::Binary(vec![]),
                    "audio/wav",
                ))
                .await
                .unwrap();
            assert_eq!(
                fetch.seen()[0].url,
                "https://test-resource.openai.azure.com/openai/deployments/whisper-1/audio/transcriptions?api-version=v1"
            );
        }
    }
}

mod speech {
    use super::*;

    mod do_generate {
        use super::*;

        /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › speech › doGenerate › should use correct URL format
        #[tokio::test]
        async fn should_use_correct_url_format() {
            let fetch = MockFetch::new(vec![Canned {
                status: 200,
                headers: vec![("content-type".into(), "audio/mpeg".into())],
                body: vec![1, 2, 3],
            }]);
            provider(&fetch)
                .speech("tts-1")
                .do_generate(&SpeechCallOptions::new("Hello, world!"))
                .await
                .unwrap();
            assert_eq!(
                fetch.seen()[0].url,
                "https://test-resource.openai.azure.com/openai/v1/audio/speech?api-version=v1"
            );
        }
    }
}

mod embedding {
    use super::*;

    const DUMMY_EMBEDDINGS: [[f64; 5]; 2] = [[0.1, 0.2, 0.3, 0.4, 0.5], [0.6, 0.7, 0.8, 0.9, 1.0]];
    const TEST_VALUES: [&str; 2] = ["sunny day at the beach", "rainy day in the city"];

    fn embedding_options() -> EmbeddingCallOptions {
        let mut options = EmbeddingCallOptions::new(TEST_VALUES[0]);
        options.values.push(TEST_VALUES[1].into());
        options
    }

    fn embedding_canned() -> Canned {
        json_canned(
            &json!({
                "object": "list",
                "data": DUMMY_EMBEDDINGS
                    .iter()
                    .enumerate()
                    .map(|(i, embedding)| json!({"object": "embedding", "index": i, "embedding": embedding}))
                    .collect::<Vec<_>>(),
                "model": "my-embedding",
                "usage": {"prompt_tokens": 8, "total_tokens": 8},
            }),
            &[],
        )
    }

    mod do_embed {
        use super::*;

        /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › embedding › doEmbed › should set the correct api version
        #[tokio::test]
        async fn should_set_the_correct_api_version() {
            let fetch = MockFetch::new(vec![embedding_canned()]);
            provider(&fetch)
                .embedding("my-embedding")
                .do_embed(&embedding_options())
                .await
                .unwrap();
            assert_eq!(
                query_param(&fetch.seen()[0], "api-version").as_deref(),
                Some("v1")
            );
        }

        /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › embedding › doEmbed › should pass headers
        #[tokio::test]
        async fn should_pass_headers() {
            let fetch = MockFetch::new(vec![embedding_canned()]);
            let provider = azure_with(
                &fetch,
                AzureOpenAIProviderSettings {
                    headers: Some(HashMap::from([(
                        "Custom-Provider-Header".into(),
                        Some("provider-header-value".into()),
                    )])),
                    ..default_settings()
                },
            );
            provider
                .embedding("my-embedding")
                .do_embed(&EmbeddingCallOptions {
                    headers: Some(HashMap::from([(
                        "Custom-Request-Header".into(),
                        "request-header-value".into(),
                    )])),
                    ..embedding_options()
                })
                .await
                .unwrap();
            let seen = &fetch.seen()[0];
            assert_eq!(
                sent_headers(seen),
                json!({
                    "api-key": "test-api-key",
                    "content-type": "application/json",
                    "custom-provider-header": "provider-header-value",
                    "custom-request-header": "request-header-value",
                })
            );
            assert_user_agent(seen);
        }
    }
}

mod image {
    use super::*;

    const PROMPT: &str = "A cute baby sea otter";

    /// The two images of the upstream `prepareJsonResponse`.
    fn image_canned() -> Canned {
        json_canned(
            &json!({
                "created": 1733837122,
                "data": [
                    {
                        "revised_prompt": "A charming visual illustration of a baby sea otter swimming joyously.",
                        "b64_json": "base64-image-1",
                    },
                    {"b64_json": "base64-image-2"},
                ],
            }),
            &[],
        )
    }

    /// `doGenerate({ prompt, n, size: '1024x1024', ... })`.
    fn image_options(n: u32) -> ImageCallOptions {
        ImageCallOptions {
            n,
            size: Some("1024x1024".parse().unwrap()),
            ..ImageCallOptions::new(PROMPT)
        }
    }

    mod image_editing_capabilities {
        use super::*;

        /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › image › image editing capabilities › leaves capabilities unknown for arbitrary deployment names created with %s
        #[test]
        fn leaves_capabilities_unknown_for_arbitrary_deployment_names_created_with_s() {
            let fetch = MockFetch::new(vec![]);
            let provider = provider(&fetch);
            for factory in ["image", "imageModel"] {
                for deployment in ["gpt-image-production", "dall-e-3"] {
                    let model: Arc<dyn ImageModel> = match factory {
                        "image" => Arc::new(provider.image(deployment)),
                        _ => provider.image_model(deployment).unwrap(),
                    };
                    assert_eq!(model.supports_file_inputs(), None, "{factory} {deployment}");
                    assert_eq!(model.supports_mask_inputs(), None, "{factory} {deployment}");
                }
            }
        }
    }

    mod do_generate {
        use super::*;

        /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › image › doGenerate › should set the correct default api version
        #[tokio::test]
        async fn should_set_the_correct_default_api_version() {
            let fetch = MockFetch::new(vec![image_canned()]);
            provider(&fetch)
                .image_model("dalle-deployment")
                .unwrap()
                .do_generate(&image_options(1))
                .await
                .unwrap();
            assert_eq!(
                query_param(&fetch.seen()[0], "api-version").as_deref(),
                Some("v1")
            );
        }

        /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › image › doGenerate › should set the correct modified api version
        #[tokio::test]
        async fn should_set_the_correct_modified_api_version() {
            let fetch = MockFetch::new(vec![image_canned()]);
            provider_api_version_changed(&fetch)
                .image_model("dalle-deployment")
                .unwrap()
                .do_generate(&image_options(1))
                .await
                .unwrap();
            assert_eq!(
                query_param(&fetch.seen()[0], "api-version").as_deref(),
                Some("2025-04-01-preview")
            );
        }

        /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › image › doGenerate › should pass headers
        #[tokio::test]
        async fn should_pass_headers() {
            let fetch = MockFetch::new(vec![image_canned()]);
            let provider = azure_with(
                &fetch,
                AzureOpenAIProviderSettings {
                    headers: Some(HashMap::from([(
                        "Custom-Provider-Header".into(),
                        Some("provider-header-value".into()),
                    )])),
                    ..default_settings()
                },
            );
            provider
                .image_model("dalle-deployment")
                .unwrap()
                .do_generate(&ImageCallOptions {
                    headers: Some(HashMap::from([(
                        "Custom-Request-Header".into(),
                        "request-header-value".into(),
                    )])),
                    ..image_options(1)
                })
                .await
                .unwrap();
            let seen = &fetch.seen()[0];
            assert_eq!(
                sent_headers(seen),
                json!({
                    "api-key": "test-api-key",
                    "content-type": "application/json",
                    "custom-provider-header": "provider-header-value",
                    "custom-request-header": "request-header-value",
                })
            );
            assert_user_agent(seen);
        }

        /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › image › doGenerate › should use the baseURL correctly
        #[tokio::test]
        async fn should_use_the_base_url_correctly() {
            let fetch = MockFetch::new(vec![image_canned()]);
            let provider = azure_with(
                &fetch,
                AzureOpenAIProviderSettings {
                    resource_name: None,
                    base_url: Some("https://test-resource.openai.azure.com/openai".into()),
                    ..default_settings()
                },
            );
            provider
                .image_model("dalle-deployment")
                .unwrap()
                .do_generate(&image_options(1))
                .await
                .unwrap();
            assert_eq!(
                fetch.seen()[0].url,
                "https://test-resource.openai.azure.com/openai/v1/images/generations?api-version=v1"
            );
        }

        /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › image › doGenerate › should extract the generated images
        #[tokio::test]
        async fn should_extract_the_generated_images() {
            let fetch = MockFetch::new(vec![image_canned()]);
            let result = provider(&fetch)
                .image_model("dalle-deployment")
                .unwrap()
                .do_generate(&image_options(1))
                .await
                .unwrap();
            assert_eq!(
                serde_json::to_value(&result.images).unwrap(),
                json!(["base64-image-1", "base64-image-2"])
            );
        }

        /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › image › doGenerate › should send the correct request body
        #[tokio::test]
        async fn should_send_the_correct_request_body() {
            let fetch = MockFetch::new(vec![image_canned()]);
            provider(&fetch)
                .image_model("dalle-deployment")
                .unwrap()
                .do_generate(&ImageCallOptions {
                    provider_options: serde_json::from_value(
                        json!({"openai": {"style": "natural"}}),
                    )
                    .unwrap(),
                    ..image_options(2)
                })
                .await
                .unwrap();
            assert_eq!(
                fetch.seen()[0].json_body(),
                json!({
                    "model": "dalle-deployment",
                    "n": 2,
                    "prompt": "A cute baby sea otter",
                    "response_format": "b64_json",
                    "size": "1024x1024",
                    "style": "natural",
                })
            );
        }
    }

    mod image_model_method {
        use super::*;

        /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › image › imageModel method › should create the same model as image method
        #[test]
        fn should_create_the_same_model_as_image_method() {
            let fetch = MockFetch::new(vec![]);
            let provider = provider(&fetch);
            let image_model = provider.image_model("dalle-deployment").unwrap();
            let image_model_alias = provider.image_model("dalle-deployment").unwrap();

            assert_eq!(image_model.provider(), image_model_alias.provider());
            assert_eq!(image_model.model_id(), image_model_alias.model_id());
        }
    }
}

mod responses {
    use super::*;

    mod do_generate {
        use super::*;

        mod text {
            use super::*;

            /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › responses › doGenerate › text › should extract text content
            #[tokio::test]
            async fn should_extract_text_content() {
                let fetch = MockFetch::new(vec![fixture_json(AZURE_TEXT_1, &[])]);
                let result = provider(&fetch)
                    .responses("test-deployment")
                    .do_generate(&test_prompt())
                    .await
                    .unwrap();
                let expected: Value =
                    serde_json::from_str(include_str!("snapshots/text_generate.json")).unwrap();
                assert_eq!(generate_json(&result), expected);
            }
        }

        mod tool_call {
            use super::*;

            /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › responses › doGenerate › tool call › should extract tool call content
            #[tokio::test]
            async fn should_extract_tool_call_content() {
                let fetch = MockFetch::new(vec![fixture_json(
                    include_str!("fixtures/azure-tool-call.1.json"),
                    &[],
                )]);
                let result = provider(&fetch)
                    .responses("test-deployment")
                    .do_generate(&test_prompt())
                    .await
                    .unwrap();
                let expected: Value =
                    serde_json::from_str(include_str!("snapshots/tool_call_generate.json"))
                        .unwrap();
                assert_eq!(generate_json(&result), expected);
            }
        }

        /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › responses › doGenerate › should extract usage
        #[tokio::test]
        async fn should_extract_usage() {
            let fetch = MockFetch::new(vec![fixture_json(AZURE_TEXT_1, &[])]);
            let result = provider(&fetch)
                .responses("test-deployment")
                .do_generate(&test_prompt())
                .await
                .unwrap();
            assert_eq!(
                serde_json::to_value(&result.usage).unwrap(),
                json!({
                    "inputTokens": {"cacheRead": 0, "noCache": 11, "total": 11},
                    "outputTokens": {"reasoning": 0, "text": 11, "total": 11},
                    "raw": {
                        "input_tokens": 11,
                        "input_tokens_details": {"cached_tokens": 0},
                        "output_tokens": 11,
                        "output_tokens_details": {"reasoning_tokens": 0},
                        "total_tokens": 22,
                    },
                })
            );
        }

        /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › responses › doGenerate › should extract response metadata
        #[tokio::test]
        async fn should_extract_response_metadata() {
            let fetch = MockFetch::new(vec![fixture_json(AZURE_TEXT_1, &[])]);
            let result = provider(&fetch)
                .responses("test-deployment")
                .do_generate(&test_prompt())
                .await
                .unwrap();
            let response = result.response.unwrap();
            assert_eq!(
                json!({
                    "id": response.id,
                    "timestamp": response.timestamp,
                    "modelId": response.model_id,
                }),
                json!({
                    "id": "resp_0d6bb044bb6ff37200698c51948054819385e24e2ad931ae6e",
                    "modelId": "gpt-5.1",
                    "timestamp": "2026-02-11T09:53:24.000Z",
                })
            );
        }

        /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › responses › doGenerate › should extract response headers
        #[tokio::test]
        async fn should_extract_response_headers() {
            let fetch = MockFetch::new(vec![fixture_json(
                AZURE_TEXT_1,
                &[("test-header", "test-value")],
            )]);
            let result = provider(&fetch)
                .responses("test-deployment")
                .do_generate(&test_prompt())
                .await
                .unwrap();
            assert_eq!(
                serde_json::to_value(result.response.unwrap().headers).unwrap(),
                json!({
                    "content-length": "1978",
                    "content-type": "application/json",
                    "test-header": "test-value",
                })
            );
        }

        /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › responses › doGenerate › should set the correct api version
        #[tokio::test]
        async fn should_set_the_correct_api_version() {
            let fetch = MockFetch::new(vec![fixture_json(AZURE_TEXT_1, &[])]);
            provider(&fetch)
                .responses("test-deployment")
                .do_generate(&test_prompt())
                .await
                .unwrap();
            assert_eq!(
                query_param(&fetch.seen()[0], "api-version").as_deref(),
                Some("v1")
            );
        }

        /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › responses › doGenerate › should pass headers
        #[tokio::test]
        async fn should_pass_headers() {
            let fetch = MockFetch::new(vec![fixture_json(AZURE_TEXT_1, &[])]);
            let provider = azure_with(
                &fetch,
                AzureOpenAIProviderSettings {
                    headers: Some(HashMap::from([(
                        "Custom-Provider-Header".into(),
                        Some("provider-header-value".into()),
                    )])),
                    ..default_settings()
                },
            );
            provider
                .responses("test-deployment")
                .do_generate(&CallOptions {
                    headers: Some(HashMap::from([(
                        "Custom-Request-Header".into(),
                        "request-header-value".into(),
                    )])),
                    ..test_prompt()
                })
                .await
                .unwrap();
            let seen = &fetch.seen()[0];
            assert_eq!(
                sent_headers(seen),
                json!({
                    "api-key": "test-api-key",
                    "content-type": "application/json",
                    "custom-provider-header": "provider-header-value",
                    "custom-request-header": "request-header-value",
                })
            );
            assert_user_agent(seen);
        }

        /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › responses › doGenerate › should use the baseURL correctly
        #[tokio::test]
        async fn should_use_the_base_url_correctly() {
            let fetch = MockFetch::new(vec![fixture_json(AZURE_TEXT_1, &[])]);
            let provider = azure_with(
                &fetch,
                AzureOpenAIProviderSettings {
                    resource_name: None,
                    base_url: Some("https://test-resource.openai.azure.com/openai".into()),
                    ..default_settings()
                },
            );
            provider
                .responses("test-deployment")
                .do_generate(&test_prompt())
                .await
                .unwrap();
            assert_eq!(
                fetch.seen()[0].url,
                "https://test-resource.openai.azure.com/openai/v1/responses?api-version=v1"
            );
        }

        /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › responses › doGenerate › should handle Azure file IDs with assistant- prefix
        #[tokio::test]
        async fn should_handle_azure_file_ids_with_assistant_prefix() {
            let fetch = MockFetch::new(vec![fixture_json(AZURE_TEXT_1, &[])]);
            provider(&fetch)
                .responses("test-deployment")
                .do_generate(&user_prompt(vec![
                    UserPart::Text(text_part("Analyze this image")),
                    user_file("assistant-abc123", "image/jpeg"),
                ]))
                .await
                .unwrap();
            assert_eq!(
                fetch.seen()[0].json_body()["input"],
                json!([{
                    "role": "user",
                    "content": [
                        {"type": "input_text", "text": "Analyze this image"},
                        {"type": "input_image", "file_id": "assistant-abc123"},
                    ],
                }])
            );
        }

        /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › responses › doGenerate › should handle PDF files with assistant- prefix
        #[tokio::test]
        async fn should_handle_pdf_files_with_assistant_prefix() {
            let fetch = MockFetch::new(vec![fixture_json(AZURE_TEXT_1, &[])]);
            provider(&fetch)
                .responses("test-deployment")
                .do_generate(&user_prompt(vec![
                    UserPart::Text(text_part("Analyze this PDF")),
                    user_file("assistant-pdf123", "application/pdf"),
                ]))
                .await
                .unwrap();
            assert_eq!(
                fetch.seen()[0].json_body()["input"],
                json!([{
                    "role": "user",
                    "content": [
                        {"type": "input_text", "text": "Analyze this PDF"},
                        {"type": "input_file", "file_id": "assistant-pdf123"},
                    ],
                }])
            );
        }

        /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › responses › doGenerate › should fall back to base64 for non-assistant file IDs
        #[tokio::test]
        async fn should_fall_back_to_base64_for_non_assistant_file_ids() {
            let fetch = MockFetch::new(vec![fixture_json(AZURE_TEXT_1, &[])]);
            provider(&fetch)
                .responses("test-deployment")
                .do_generate(&user_prompt(vec![
                    UserPart::Text(text_part("Analyze this image")),
                    user_file("file-abc123", "image/jpeg"),
                ]))
                .await
                .unwrap();
            assert_eq!(
                fetch.seen()[0].json_body()["input"],
                json!([{
                    "role": "user",
                    "content": [
                        {"type": "input_text", "text": "Analyze this image"},
                        {"type": "input_image", "image_url": "data:image/jpeg;base64,file-abc123"},
                    ],
                }])
            );
        }

        /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › responses › doGenerate › should send include provider option for file search results
        #[tokio::test]
        async fn should_send_include_provider_option_for_file_search_results() {
            let fetch = MockFetch::new(vec![fixture_json(AZURE_TEXT_1, &[])]);
            let provider = provider(&fetch);
            let result = provider
                .responses("test-deployment")
                .do_generate(&CallOptions {
                    tools: Some(vec![Tool::Provider(
                        provider
                            .tools
                            .file_search(FileSearchArgs {
                                vector_store_ids: vec!["vs_123".into(), "vs_456".into()],
                                max_num_results: Some(10),
                                ranking: Some(FileSearchRanking {
                                    ranker: Some("auto".into()),
                                    score_threshold: None,
                                }),
                                filters: None,
                            })
                            .unwrap(),
                    )]),
                    ..test_prompt()
                })
                .await
                .unwrap();
            assert_eq!(
                fetch.seen()[0].json_body(),
                json!({
                    "input": hello_input(),
                    "model": "test-deployment",
                    "tools": [{
                        "max_num_results": 10,
                        "ranking_options": {"ranker": "auto"},
                        "type": "file_search",
                        "vector_store_ids": ["vs_123", "vs_456"],
                    }],
                })
            );
            assert!(result.warnings.is_empty());
        }

        /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › responses › doGenerate › should forward include provider options to request body
        #[tokio::test]
        async fn should_forward_include_provider_options_to_request_body() {
            let fetch = MockFetch::new(vec![fixture_json(AZURE_TEXT_1, &[])]);
            let result = provider(&fetch)
                .responses("test-deployment")
                .do_generate(&CallOptions {
                    provider_options: Some(provider_options(
                        "azure",
                        json!({"include": ["file_search_call.results"]}),
                    )),
                    ..test_prompt()
                })
                .await
                .unwrap();
            assert_eq!(
                fetch.seen()[0].json_body(),
                json!({
                    "include": ["file_search_call.results"],
                    "input": hello_input(),
                    "model": "test-deployment",
                })
            );
            assert!(result.warnings.is_empty());
        }

        mod code_interpreter_tool {
            use super::*;

            /// The `beforeEach` of the upstream describe: one generate call with the tool.
            async fn generate() -> (Arc<MockFetch>, GenerateResult) {
                let fetch = MockFetch::new(vec![fixture_json(
                    include_str!("fixtures/azure-code-interpreter-tool.1.json"),
                    &[],
                )]);
                let provider = provider(&fetch);
                let result = provider
                    .responses("test-deployment")
                    .do_generate(&CallOptions {
                        tools: Some(vec![code_interpreter_tool(&provider)]),
                        ..test_prompt()
                    })
                    .await
                    .unwrap();
                (fetch, result)
            }

            /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › responses › doGenerate › code interpreter tool › should send request body with include and tool
            #[tokio::test]
            async fn should_send_request_body_with_include_and_tool() {
                let (fetch, _) = generate().await;
                assert_eq!(
                    fetch.seen()[0].json_body(),
                    json!({
                        "include": ["code_interpreter_call.outputs"],
                        "input": hello_input(),
                        "model": "test-deployment",
                        "tools": [{"container": {"type": "auto"}, "type": "code_interpreter"}],
                    })
                );
            }

            /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › responses › doGenerate › code interpreter tool › should include code interpreter tool call and result in content
            #[tokio::test]
            async fn should_include_code_interpreter_tool_call_and_result_in_content() {
                let (_, result) = generate().await;
                let expected: Value =
                    serde_json::from_str(include_str!("snapshots/code_interpreter_generate.json"))
                        .unwrap();
                assert_eq!(content_json(&result), expected);
            }
        }

        mod file_search_tool {
            use super::*;

            mod without_results_include {
                use super::*;

                /// The `beforeEach` of the upstream describe.
                async fn generate() -> (Arc<MockFetch>, GenerateResult) {
                    let fetch = MockFetch::new(vec![fixture_json(
                        include_str!("fixtures/openai-file-search-tool.1.json"),
                        &[],
                    )]);
                    let provider = provider(&fetch);
                    let result = provider
                        .responses("test-deployment")
                        .do_generate(&CallOptions {
                            tools: Some(vec![file_search_tool(&provider, true)]),
                            ..test_prompt()
                        })
                        .await
                        .unwrap();
                    (fetch, result)
                }

                /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › responses › doGenerate › file search tool › without results include › should send request body with tool
                #[tokio::test]
                async fn should_send_request_body_with_tool() {
                    let (fetch, _) = generate().await;
                    assert_eq!(
                        fetch.seen()[0].json_body(),
                        json!({
                            "input": hello_input(),
                            "model": "test-deployment",
                            "tools": [{
                                "filters": {"key": "author", "type": "eq", "value": "Jane Smith"},
                                "max_num_results": 5,
                                "ranking_options": {"ranker": "auto", "score_threshold": 0.5},
                                "type": "file_search",
                                "vector_store_ids": ["vs_68caad8bd5d88191ab766cf043d89a18"],
                            }],
                        })
                    );
                }

                /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › responses › doGenerate › file search tool › without results include › should include file search tool call and result in content
                #[tokio::test]
                async fn should_include_file_search_tool_call_and_result_in_content() {
                    let (_, result) = generate().await;
                    let expected: Value = serde_json::from_str(include_str!(
                        "snapshots/file_search_without_include_generate.json"
                    ))
                    .unwrap();
                    assert_eq!(content_json(&result), expected);
                }
            }

            mod with_results_include {
                use super::*;

                /// The `beforeEach` of the upstream describe: the same tool, with `file_search_call.results` included.
                async fn generate() -> (Arc<MockFetch>, GenerateResult) {
                    let fetch = MockFetch::new(vec![fixture_json(
                        include_str!("fixtures/openai-file-search-tool.2.json"),
                        &[],
                    )]);
                    let provider = provider(&fetch);
                    let result = provider
                        .responses("test-deployment")
                        .do_generate(&CallOptions {
                            tools: Some(vec![file_search_tool(&provider, true)]),
                            provider_options: Some(provider_options(
                                "azure",
                                json!({"include": ["file_search_call.results"]}),
                            )),
                            ..test_prompt()
                        })
                        .await
                        .unwrap();
                    (fetch, result)
                }

                /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › responses › doGenerate › file search tool › with results include › should send request body with tool
                #[tokio::test]
                async fn should_send_request_body_with_tool() {
                    let (fetch, _) = generate().await;
                    assert_eq!(
                        fetch.seen()[0].json_body(),
                        json!({
                            "include": ["file_search_call.results"],
                            "input": hello_input(),
                            "model": "test-deployment",
                            "tools": [{
                                "filters": {"key": "author", "type": "eq", "value": "Jane Smith"},
                                "max_num_results": 5,
                                "ranking_options": {"ranker": "auto", "score_threshold": 0.5},
                                "type": "file_search",
                                "vector_store_ids": ["vs_68caad8bd5d88191ab766cf043d89a18"],
                            }],
                        })
                    );
                }

                /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › responses › doGenerate › file search tool › with results include › should include file search tool call and result in content
                #[tokio::test]
                async fn should_include_file_search_tool_call_and_result_in_content() {
                    let (_, result) = generate().await;
                    let expected: Value = serde_json::from_str(include_str!(
                        "snapshots/file_search_with_include_generate.json"
                    ))
                    .unwrap();
                    assert_eq!(content_json(&result), expected);
                }
            }
        }

        mod web_search_preview_tool {
            use super::*;

            /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › responses › doGenerate › web search preview tool › should stream web search preview results include
            #[tokio::test]
            async fn should_stream_web_search_preview_results_include() {
                let fetch = MockFetch::new(vec![fixture_json(
                    include_str!("fixtures/azure-web-search-preview-tool.1.json"),
                    &[],
                )]);
                let provider = provider(&fetch);
                let result = provider
                    .responses("test-deployment")
                    .do_generate(&CallOptions {
                        tools: Some(vec![web_search_preview_tool(&provider)]),
                        ..test_prompt()
                    })
                    .await
                    .unwrap();
                let expected: Value = serde_json::from_str(include_str!(
                    "snapshots/web_search_preview_generate.json"
                ))
                .unwrap();
                assert_eq!(content_json(&result), expected);
            }
        }

        mod reasoning {
            use super::*;

            /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › responses › doGenerate › reasoning › should generate with reasoning encrypted content
            #[tokio::test]
            async fn should_generate_with_reasoning_encrypted_content() {
                let fetch = MockFetch::new(vec![fixture_json(
                    include_str!("fixtures/azure-reasoning-encrypted-content.1.json"),
                    &[],
                )]);
                let result = provider(&fetch)
                    .responses("test-deployment")
                    .do_generate(&CallOptions {
                        tools: Some(vec![calculator_tool()]),
                        provider_options: Some(provider_options(
                            "azure",
                            json!({
                                "reasoningEffort": "high",
                                "maxCompletionTokens": 32_000,
                                "store": false,
                                "include": ["reasoning.encrypted_content"],
                                "reasoningSummary": "auto",
                                "forceReasoning": true,
                            }),
                        )),
                        ..test_prompt()
                    })
                    .await
                    .unwrap();
                let expected: Value =
                    serde_json::from_str(include_str!("snapshots/reasoning_generate.json"))
                        .unwrap();
                assert_eq!(generate_json(&result), expected);
            }
        }

        mod image_generation_tool {
            use super::*;

            /// The `beforeEach` of the upstream describe.
            async fn generate() -> (Arc<MockFetch>, GenerateResult) {
                let fetch = MockFetch::new(vec![fixture_json(
                    include_str!("fixtures/azure-image-generation-tool.1.json"),
                    &[],
                )]);
                let provider = provider(&fetch);
                let result = provider
                    .responses("test-deployment")
                    .do_generate(&CallOptions {
                        tools: Some(vec![image_generation_tool(&provider)]),
                        ..test_prompt()
                    })
                    .await
                    .unwrap();
                (fetch, result)
            }

            /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › responses › doGenerate › image generation tool › should send request body with include and tool
            #[tokio::test]
            async fn should_send_request_body_with_include_and_tool() {
                let (fetch, _) = generate().await;
                assert_eq!(
                    fetch.seen()[0].json_body(),
                    json!({
                        "input": hello_input(),
                        "model": "test-deployment",
                        "tools": [{
                            "output_format": "webp",
                            "partial_images": 2,
                            "quality": "low",
                            "size": "1024x1024",
                            "type": "image_generation",
                        }],
                    })
                );
            }

            /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › responses › doGenerate › image generation tool › should include generate image tool call and result in content
            #[tokio::test]
            async fn should_include_generate_image_tool_call_and_result_in_content() {
                let (_, result) = generate().await;
                let expected: Value =
                    serde_json::from_str(include_str!("snapshots/image_generation_generate.json"))
                        .unwrap();
                assert_eq!(content_json(&result), expected);
            }
        }
    }

    mod do_stream {
        use super::*;

        mod text {
            use super::*;

            /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › responses › doStream › text › should stream text content
            #[tokio::test]
            async fn should_stream_text_content() {
                let fetch = MockFetch::new(vec![chunks_canned(
                    include_str!("fixtures/azure-text.1.chunks.txt"),
                    &[],
                )]);
                let result = provider(&fetch)
                    .responses("test-deployment")
                    .do_stream(&test_prompt())
                    .await
                    .unwrap();
                let expected: Value =
                    serde_json::from_str(include_str!("snapshots/text_stream.json")).unwrap();
                assert_eq!(stream_json(result).await, expected);
            }
        }

        mod tool_call {
            use super::*;

            /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › responses › doStream › tool call › should stream tool call content
            #[tokio::test]
            async fn should_stream_tool_call_content() {
                let fetch = MockFetch::new(vec![chunks_canned(
                    include_str!("fixtures/azure-tool-call.1.chunks.txt"),
                    &[],
                )]);
                let result = provider(&fetch)
                    .responses("test-deployment")
                    .do_stream(&test_prompt())
                    .await
                    .unwrap();
                let expected: Value =
                    serde_json::from_str(include_str!("snapshots/tool_call_stream.json")).unwrap();
                assert_eq!(stream_json(result).await, expected);
            }
        }

        /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › responses › doStream › should extract response headers
        #[tokio::test]
        async fn should_extract_response_headers() {
            let fetch = MockFetch::new(vec![chunks_canned(
                include_str!("fixtures/azure-text.1.chunks.txt"),
                &[("test-header", "test-value")],
            )]);
            let result = provider(&fetch)
                .responses("test-deployment")
                .do_stream(&test_prompt())
                .await
                .unwrap();
            assert_eq!(
                serde_json::to_value(result.response.unwrap().headers).unwrap(),
                json!({
                    "cache-control": "no-cache",
                    "connection": "keep-alive",
                    "content-type": "text/event-stream",
                    "test-header": "test-value",
                })
            );
        }

        /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › responses › doStream › should handle file_citation annotations without optional fields in streaming
        #[tokio::test]
        async fn should_handle_file_citation_annotations_without_optional_fields_in_streaming() {
            let events = [
                r#"{"type":"response.content_part.added","item_id":"msg_456","output_index":0,"content_index":0,"part":{"type":"output_text","text":"","annotations":[]}}"#,
                r#"{"type":"response.output_text.annotation.added","item_id":"msg_456","output_index":0,"content_index":0,"annotation_index":0,"annotation":{"type":"file_citation","file_id":"assistant-YRcoCqn3Fo2K4JgraG","filename":"resource1.json","index":145}}"#,
                r#"{"type":"response.output_text.annotation.added","item_id":"msg_456","output_index":0,"content_index":0,"annotation_index":1,"annotation":{"type":"file_citation","file_id":"assistant-YRcoCqn3Fo2K4JgraG","filename":"resource1.json","index":192}}"#,
                r#"{"type":"response.content_part.done","item_id":"msg_456","output_index":0,"content_index":0,"part":{"type":"output_text","text":"Answer for the specified years....","annotations":[{"type":"file_citation","file_id":"assistant-YRcoCqn3Fo2K4JgraG","filename":"resource1.json","index":145},{"type":"file_citation","file_id":"assistant-YRcoCqn3Fo2K4JgraG","filename":"resource1.json","index":192}]}}"#,
                r#"{"type":"response.output_item.done","output_index":0,"item":{"id":"msg_456","type":"message","status":"completed","role":"assistant","content":[{"type":"output_text","text":"Answer for the specified years....","annotations":[{"type":"file_citation","file_id":"assistant-YRcoCqn3Fo2K4JgraG","filename":"resource1.json","index":145},{"type":"file_citation","file_id":"assistant-YRcoCqn3Fo2K4JgraG","filename":"resource1.json","index":192}]}]}}"#,
                r#"{"type":"response.completed","response":{"id":"resp_456","object":"response","created_at":1234567890,"status":"completed","error":null,"incomplete_details":null,"instructions":null,"max_output_tokens":null,"model":"test-deployment","output":[{"id":"msg_456","type":"message","status":"completed","role":"assistant","content":[{"type":"output_text","text":"Answer for the specified years....","annotations":[{"type":"file_citation","file_id":"assistant-YRcoCqn3Fo2K4JgraG","filename":"resource1.json","index":145},{"type":"file_citation","file_id":"assistant-YRcoCqn3Fo2K4JgraG","filename":"resource1.json","index":192}]}]}],"parallel_tool_calls":true,"previous_response_id":null,"reasoning":{"effort":null,"summary":null},"store":true,"temperature":0,"text":{"format":{"type":"text"}},"tool_choice":"auto","tools":[],"top_p":1,"truncation":"disabled","usage":{"input_tokens":50,"input_tokens_details":{"cached_tokens":0},"output_tokens":25,"output_tokens_details":{"reasoning_tokens":0},"total_tokens":75},"user":null,"metadata":{}}}"#,
            ];
            // Each event is `data:` followed by the JSON, as the upstream chunks are written.
            let mut body = String::new();
            for event in events {
                body.push_str("data:");
                body.push_str(event);
                body.push_str("\n\n");
            }
            body.push_str("data: [DONE]\n\n");
            let fetch = MockFetch::new(vec![sse_canned(body.into_bytes(), &[])]);
            let result = provider(&fetch)
                .responses("test-deployment")
                .do_stream(&test_prompt())
                .await
                .unwrap();

            assert_eq!(
                stream_json(result).await,
                json!([
                    {"type": "stream-start", "warnings": []},
                    {
                        "filename": "resource1.json",
                        "id": "id-0",
                        "mediaType": "text/plain",
                        "providerMetadata": {"azure": {"fileId": "assistant-YRcoCqn3Fo2K4JgraG", "index": 145, "type": "file_citation"}},
                        "sourceType": "document",
                        "title": "resource1.json",
                        "type": "source",
                    },
                    {
                        "filename": "resource1.json",
                        "id": "id-1",
                        "mediaType": "text/plain",
                        "providerMetadata": {"azure": {"fileId": "assistant-YRcoCqn3Fo2K4JgraG", "index": 192, "type": "file_citation"}},
                        "sourceType": "document",
                        "title": "resource1.json",
                        "type": "source",
                    },
                    {
                        "id": "msg_456",
                        "providerMetadata": {"azure": {
                            "annotations": [
                                {"file_id": "assistant-YRcoCqn3Fo2K4JgraG", "filename": "resource1.json", "index": 145, "type": "file_citation"},
                                {"file_id": "assistant-YRcoCqn3Fo2K4JgraG", "filename": "resource1.json", "index": 192, "type": "file_citation"},
                            ],
                            "itemId": "msg_456",
                        }},
                        "type": "text-end",
                    },
                    {
                        "finishReason": {"unified": "stop"},
                        "providerMetadata": {"azure": {"responseId": null}},
                        "type": "finish",
                        "usage": {
                            "inputTokens": {"cacheRead": 0, "noCache": 50, "total": 50},
                            "outputTokens": {"reasoning": 0, "text": 25, "total": 25},
                            "raw": {
                                "input_tokens": 50,
                                "input_tokens_details": {"cached_tokens": 0},
                                "output_tokens": 25,
                                "output_tokens_details": {"reasoning_tokens": 0},
                                "total_tokens": 75,
                            },
                        },
                    },
                ])
            );
        }

        /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › responses › doStream › should send code interpreter calls
        #[tokio::test]
        async fn should_send_code_interpreter_calls() {
            let fetch = MockFetch::new(vec![chunks_canned(
                include_str!("fixtures/azure-code-interpreter-tool.1.chunks.txt"),
                &[],
            )]);
            let provider = provider(&fetch);
            let result = provider
                .responses("test-deployment")
                .do_stream(&CallOptions {
                    tools: Some(vec![code_interpreter_tool(&provider)]),
                    ..test_prompt()
                })
                .await
                .unwrap();
            let expected: Value =
                serde_json::from_str(include_str!("snapshots/code_interpreter_stream.json"))
                    .unwrap();
            assert_eq!(stream_json(result).await, expected);
        }

        /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › responses › doStream › should stream with reasoning encrypted content include reasoning-delta part
        #[tokio::test]
        async fn should_stream_with_reasoning_encrypted_content_include_reasoning_delta_part() {
            let fetch = MockFetch::new(vec![chunks_canned(
                include_str!("fixtures/azure-reasoning-encrypted-content.1.chunks.txt"),
                &[],
            )]);
            let result = provider(&fetch)
                .responses("test-deployment")
                .do_stream(&CallOptions {
                    tools: Some(vec![calculator_tool()]),
                    provider_options: Some(provider_options(
                        "openai",
                        json!({
                            "reasoningEffort": "high",
                            "maxCompletionTokens": 32_000,
                            "store": false,
                            "include": ["reasoning.encrypted_content"],
                            "reasoningSummary": "auto",
                            "forceReasoning": true,
                        }),
                    )),
                    ..test_prompt()
                })
                .await
                .unwrap();
            let expected: Value =
                serde_json::from_str(include_str!("snapshots/reasoning_stream.json")).unwrap();
            assert_eq!(stream_json(result).await, expected);
        }

        mod file_search_tool {
            use super::*;

            /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › responses › doStream › file search tool › should stream file search results without results include
            #[tokio::test]
            async fn should_stream_file_search_results_without_results_include() {
                let fetch = MockFetch::new(vec![chunks_canned(
                    include_str!("fixtures/openai-file-search-tool.1.chunks.txt"),
                    &[],
                )]);
                let provider = provider(&fetch);
                let result = provider
                    .responses("test-deployment")
                    .do_stream(&CallOptions {
                        tools: Some(vec![Tool::Provider(
                            provider
                                .tools
                                .file_search(FileSearchArgs {
                                    vector_store_ids: vec![
                                        "vs_68caad8bd5d88191ab766cf043d89a18".into(),
                                    ],
                                    max_num_results: None,
                                    ranking: None,
                                    filters: None,
                                })
                                .unwrap(),
                        )]),
                        ..test_prompt()
                    })
                    .await
                    .unwrap();
                let expected: Value = serde_json::from_str(include_str!(
                    "snapshots/file_search_without_include_stream.json"
                ))
                .unwrap();
                assert_eq!(stream_json(result).await, expected);
            }

            /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › responses › doStream › file search tool › should stream file search results with results include
            #[tokio::test]
            async fn should_stream_file_search_results_with_results_include() {
                let fetch = MockFetch::new(vec![chunks_canned(
                    include_str!("fixtures/openai-file-search-tool.2.chunks.txt"),
                    &[],
                )]);
                let provider = provider(&fetch);
                let result = provider
                    .responses("test-deployment")
                    .do_stream(&CallOptions {
                        tools: Some(vec![Tool::Provider(
                            provider
                                .tools
                                .file_search(FileSearchArgs {
                                    vector_store_ids: vec![
                                        "vs_68caad8bd5d88191ab766cf043d89a18".into(),
                                    ],
                                    max_num_results: None,
                                    ranking: None,
                                    filters: None,
                                })
                                .unwrap(),
                        )]),
                        provider_options: Some(provider_options(
                            "openai",
                            json!({"include": ["file_search_call.results"]}),
                        )),
                        ..test_prompt()
                    })
                    .await
                    .unwrap();
                let expected: Value = serde_json::from_str(include_str!(
                    "snapshots/file_search_with_include_stream.json"
                ))
                .unwrap();
                assert_eq!(stream_json(result).await, expected);
            }
        }

        mod web_search_preview_tool {
            use super::*;

            /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › responses › doStream › web search preview tool › should stream web search preview results include
            #[tokio::test]
            async fn should_stream_web_search_preview_results_include() {
                let fetch = MockFetch::new(vec![chunks_canned(
                    include_str!("fixtures/azure-web-search-preview-tool.1.chunks.txt"),
                    &[],
                )]);
                let provider = provider(&fetch);
                let result = provider
                    .responses("test-deployment")
                    .do_stream(&CallOptions {
                        tools: Some(vec![web_search_preview_tool(&provider)]),
                        ..test_prompt()
                    })
                    .await
                    .unwrap();
                let expected: Value =
                    serde_json::from_str(include_str!("snapshots/web_search_preview_stream.json"))
                        .unwrap();
                assert_eq!(stream_json(result).await, expected);
            }
        }

        mod image_generation_tool {
            use super::*;

            /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › responses › doStream › image generation tool › should stream image generation tool results include
            #[tokio::test]
            async fn should_stream_image_generation_tool_results_include() {
                let fetch = MockFetch::new(vec![chunks_canned(
                    include_str!("fixtures/azure-image-generation-tool.1.chunks.txt"),
                    &[],
                )]);
                let provider = provider(&fetch);
                let result = provider
                    .responses("test-deployment")
                    .do_stream(&CallOptions {
                        tools: Some(vec![image_generation_tool(&provider)]),
                        ..test_prompt()
                    })
                    .await
                    .unwrap();
                let expected: Value =
                    serde_json::from_str(include_str!("snapshots/image_generation_stream.json"))
                        .unwrap();
                assert_eq!(stream_json(result).await, expected);
            }
        }
    }
}

mod resource_name_validation {
    use super::*;

    /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › resourceName validation › rejects %j before sending a request
    #[tokio::test]
    async fn rejects_j_before_sending_a_request() {
        for resource_name in [
            "user@internal:8080/#",
            "169.254.169.254:80/x#",
            "evil.example.com/#",
        ] {
            let fetch = MockFetch::new(vec![]);
            let provider = azure_with(
                &fetch,
                AzureOpenAIProviderSettings {
                    resource_name: Some(resource_name.into()),
                    api_key: Some(Resolvable::Value("test-key".into())),
                    ..Default::default()
                },
            );
            let error = provider
                .call("test-deployment")
                .do_generate(&test_prompt())
                .await
                .expect_err("an invalid resource name must be rejected");
            assert!(
                matches!(
                    error,
                    AiMuxError::InvalidArgument { ref message, .. }
                        if message.contains("Invalid Azure resource name")
                ),
                "{resource_name}: {error}"
            );
            assert!(
                fetch.seen().is_empty(),
                "{resource_name} reached the transport"
            );
        }
    }

    /// upstream: @ai-sdk/azure src/azure-openai-provider.test.ts › resourceName validation › accepts a DNS-label resource name
    #[tokio::test]
    async fn accepts_a_dns_label_resource_name() {
        let fetch = RouteFetch::new(|_| Canned {
            status: 500,
            headers: vec![("content-type".into(), "application/json".into())],
            body: b"{}".to_vec(),
        });
        let provider = create_azure(AzureOpenAIProviderSettings {
            resource_name: Some("my-resource".into()),
            api_key: Some(Resolvable::Value("test-key".into())),
            fetch: Some(fetch.transport()),
            ..Default::default()
        })
        .unwrap();
        assert!(
            provider
                .call("test-deployment")
                .do_generate(&test_prompt())
                .await
                .is_err()
        );
        assert!(
            fetch.seen()[0]
                .url
                .starts_with("https://my-resource.openai.azure.com/")
        );
    }
}

// Not ported (listed in the report): `describe('deepseek')`, 4 cases (Azure-hosted
// DeepSeek chat models are not in aimux); `describe('completion')`, 2 cases (the
// legacy completion model is not on the Azure provider); `describe('embedding')`
// › 'should expose the aggregate token limit' (no aggregate input-bytes limit on
// the embedding model).

/// partial: `should use the baseURL correctly`; full port in
/// src/azure/tests/openai_provider.rs.
#[tokio::test]
async fn chat_generate() {
    let fetch = MockFetch::new(vec![Canned::json(&chat_response())]);
    let result = e2e_provider(&fetch)
        .chat("test-deployment")
        .do_generate(&prompt())
        .await
        .unwrap();
    let body = assert_request(&fetch, "chat/completions");
    assert_eq!(body["model"], "test-deployment");
    assert_eq!(body["messages"], json!([{"role":"user","content":"Hello"}]));
    assert_eq!(result.content.len(), 1);
    assert!(
        matches!(&result.content[0], GenerateContent::Text { text, .. } if text == "Hello World!")
    );
    assert_eq!(result.usage.input_tokens.total, Some(4));
    assert_eq!(result.usage.output_tokens.total, Some(30));
    assert_eq!(
        result.response.as_ref().unwrap().id.as_deref(),
        Some("chatcmpl-1")
    );
}

/// partial: `should extract text content`, `should extract usage`; full port in
/// src/azure/tests/openai_provider.rs.
#[tokio::test]
async fn responses_generate() {
    let fixture = text_fixture();
    let fetch = MockFetch::new(vec![Canned::json(&fixture)]);
    let result = e2e_provider(&fetch)
        .responses("test-deployment")
        .do_generate(&prompt())
        .await
        .unwrap();
    let body = assert_request(&fetch, "responses");
    assert_eq!(body["model"], "test-deployment");
    assert_eq!(
        body["input"],
        json!([{"role":"user","content":[{"type":"input_text","text":"Hello"}]}])
    );
    assert_eq!(result.content.len(), 1);
    match &result.content[0] {
        GenerateContent::Text {
            text,
            provider_metadata,
        } => {
            assert_eq!(text, "Word");
            assert_eq!(
                provider_metadata.as_ref().unwrap()["azure"]["itemId"],
                "msg_0d6bb044bb6ff37200698c51952e288193beb9044db4d8c810"
            );
        }
        _ => panic!("expected text"),
    }
    assert_eq!(
        result.provider_metadata.as_ref().unwrap()["azure"]["responseId"],
        fixture["id"]
    );
    assert_eq!(result.usage.input_tokens.total, Some(11));
    assert_eq!(result.usage.input_tokens.cache_read, Some(0));
    assert_eq!(result.usage.input_tokens.no_cache, Some(11));
    assert_eq!(result.usage.output_tokens.total, Some(11));
    assert_eq!(result.usage.output_tokens.reasoning, Some(0));
    assert_eq!(result.usage.output_tokens.text, Some(11));
    assert_eq!(result.usage.raw, fixture["usage"].as_object().cloned());
    assert_eq!(
        result.response.as_ref().unwrap().model_id.as_deref(),
        Some("test-deployment")
    );
}

/// partial: `should extract tool call content`; full port in
/// src/azure/tests/openai_provider.rs.
#[tokio::test]
async fn responses_tool_call() {
    let mut fixture = text_fixture();
    fixture["output"] = json!([{"type":"function_call","id":"fc_0a2fa1b539ba14ba00698c519ebab0819494302fc0b5c31440","call_id":"call_YunNGbIwdVJ2i0y0Mybva4Pw","name":"weather","arguments":"{\"location\":\"San Francisco\"}","status":"completed"}]);
    let fetch = MockFetch::new(vec![Canned::json(&fixture)]);
    let result = e2e_provider(&fetch)
        .responses("test-deployment")
        .do_generate(&prompt())
        .await
        .unwrap();
    assert_eq!(
        assert_request(&fetch, "responses")["input"],
        json!([{"role":"user","content":[{"type":"input_text","text":"Hello"}]}])
    );
    assert_eq!(result.content.len(), 1);
    match &result.content[0] {
        GenerateContent::ToolCall(call) => {
            assert_eq!(call.tool_call_id, "call_YunNGbIwdVJ2i0y0Mybva4Pw");
            assert_eq!(call.tool_name, "weather");
            assert_eq!(call.input, "{\"location\":\"San Francisco\"}");
        }
        _ => panic!("expected tool call"),
    }
}

/// partial: `should stream text content`; full port in
/// src/azure/tests/openai_provider.rs.
#[tokio::test]
async fn responses_stream() {
    let fetch = MockFetch::new(vec![Canned {
        status: 200,
        headers: vec![("content-type".into(), "text/event-stream".into())],
        body: text_chunks().as_bytes().to_vec(),
    }]);
    let result = e2e_provider(&fetch)
        .responses("test-deployment")
        .do_stream(&prompt())
        .await
        .unwrap();
    let body = assert_request(&fetch, "responses");
    assert_eq!(body["stream"], true);
    assert_eq!(
        body["input"],
        json!([{"role":"user","content":[{"type":"input_text","text":"Hello"}]}])
    );
    let parts = result
        .stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(parts.len(), 6);
    assert!(matches!(parts[0], StreamPart::StreamStart { .. }));
    assert!(matches!(parts[1], StreamPart::ResponseMetadata(_)));
    assert!(matches!(parts[2], StreamPart::TextStart { .. }));
    assert!(matches!(&parts[3], StreamPart::TextDelta { delta, .. } if delta == "Hello"));
    match &parts[4] {
        StreamPart::TextEnd {
            provider_metadata, ..
        } => assert_eq!(
            provider_metadata.as_ref().unwrap()["azure"]["itemId"],
            "msg_02ce8deeb6197db200698c5198ca0c81979bedbe6c98a8ab93"
        ),
        _ => panic!("expected text end"),
    }
    match &parts[5] {
        StreamPart::Finish { usage, .. } => {
            assert_eq!(usage.input_tokens.total, Some(11));
            assert_eq!(usage.output_tokens.total, Some(11));
        }
        _ => panic!("expected finish"),
    }
}

/// partial: `should set the correct api version`; full port in
/// src/azure/tests/openai_provider.rs.
#[tokio::test]
async fn embedding_generate() {
    let fetch = MockFetch::new(vec![Canned::json(
        &json!({"object":"list","data":[{"object":"embedding","index":0,"embedding":[0.1,0.2,0.3,0.4,0.5]},{"object":"embedding","index":1,"embedding":[0.6,0.7,0.8,0.9,1.0]}],"model":"my-embedding","usage":{"prompt_tokens":8,"total_tokens":8}}),
    )]);
    let mut options = EmbeddingCallOptions::new("sunny day at the beach");
    options.values.push("rainy day in the city".into());
    let result = e2e_provider(&fetch)
        .embedding("my-embedding")
        .do_embed(&options)
        .await
        .unwrap();
    assert_eq!(
        assert_request(&fetch, "embeddings"),
        json!({"model":"my-embedding","input":options.values,"encoding_format":"float"})
    );
    assert_eq!(
        result.embeddings,
        vec![vec![0.1, 0.2, 0.3, 0.4, 0.5], vec![0.6, 0.7, 0.8, 0.9, 1.0]]
    );
    assert_eq!(result.usage.unwrap().tokens, 8);
}

/// partial: `should send the correct request body` (file and mask support
/// stay unknown for deployments, `azure-openai-provider.ts:372-376`); full
/// port in src/azure/tests/openai_provider.rs.
#[test]
fn image_input_capabilities_are_unknown_for_deployments() {
    let fetch = MockFetch::new(vec![]);
    let model = e2e_provider(&fetch).image("dall-e-2");
    assert_eq!(model.supports_file_inputs(), None);
    assert_eq!(model.supports_mask_inputs(), None);
}

/// partial: `should send the correct request body`,
/// `should extract the generated images`; full port in
/// src/azure/tests/openai_provider.rs.
#[tokio::test]
async fn image_generate() {
    let fetch = MockFetch::new(vec![Canned::json(&image_response())]);
    let mut options = ImageCallOptions::new("A cute baby sea otter");
    options.n = 2;
    options.size = Some("1024x1024".parse().unwrap());
    options.provider_options =
        serde_json::from_value(json!({"openai":{"style":"natural"}})).unwrap();
    let result = e2e_provider(&fetch)
        .image("test-deployment")
        .do_generate(&options)
        .await
        .unwrap();
    assert_eq!(
        assert_request(&fetch, "images/generations"),
        json!({"model":"test-deployment","prompt":"A cute baby sea otter","n":2,"size":"1024x1024","response_format":"b64_json","style":"natural"})
    );
    match result.images {
        ImageOutputs::Base64(images) => assert_eq!(images, ["base64-image-1", "base64-image-2"]),
        _ => panic!("expected base64 images"),
    }
}

/// partial: `should use correct URL format`; full port in
/// src/azure/tests/openai_provider.rs.
#[tokio::test]
async fn transcription_generate() {
    let fetch = MockFetch::new(vec![Canned::json(
        &json!({"text":"Hello, world!","segments":[],"language":"en","duration":5.0}),
    )]);
    let result = e2e_provider(&fetch)
        .transcription("test-deployment")
        .do_generate(&TranscriptionCallOptions::new(
            AudioInput::Binary(vec![]),
            "audio/wav",
        ))
        .await
        .unwrap();
    let seen = fetch.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "POST");
    assert_eq!(
        seen[0].url,
        "https://test-resource.openai.azure.com/openai/v1/audio/transcriptions?api-version=v1"
    );
    assert_eq!(seen[0].headers["api-key"], "test-api-key");
    assert!(seen[0].headers["content-type"].starts_with("multipart/form-data; boundary="));
    let body = String::from_utf8(seen[0].body.clone()).unwrap();
    assert!(body.contains("name=\"model\"\r\n\r\ntest-deployment"));
    assert!(body.contains("filename=\"audio.wav\""));
    assert!(body.contains("Content-Type: audio/wav"));
    assert_eq!(result.text, "Hello, world!");
    assert!(result.segments.is_empty());
    assert_eq!(result.duration_in_seconds, Some(5.0));
}

/// partial: `should use correct URL format`; full port in
/// src/azure/tests/openai_provider.rs.
#[tokio::test]
async fn speech_generate() {
    let fetch = MockFetch::new(vec![Canned {
        status: 200,
        headers: vec![("content-type".into(), "audio/mpeg".into())],
        body: vec![1, 2, 3],
    }]);
    let result = e2e_provider(&fetch)
        .speech("test-deployment")
        .do_generate(&SpeechCallOptions::new("Hello, world!"))
        .await
        .unwrap();
    assert_eq!(
        assert_request(&fetch, "audio/speech"),
        json!({"model":"test-deployment","input":"Hello, world!","voice":"alloy","response_format":"mp3"})
    );
    assert!(matches!(result.audio, AudioData::Binary(audio) if audio == vec![1,2,3]));
}

/// partial: `sends audio and the required model definition`; full port in
/// src/azure/tests/transcription_model.rs.
#[tokio::test]
async fn speech_transcription_request() {
    for audio in [
        AudioInput::Binary(vec![1, 2, 3]),
        AudioInput::Base64("AQID".into()),
    ] {
        let fetch = MockFetch::new(vec![Canned::json(&json!({
            "combinedPhrases": [{"text": "Hello world."}], "phrases": [],
        }))]);
        e2e_provider(&fetch)
            .transcription("mai-transcribe-2")
            .do_generate(&TranscriptionCallOptions::new(audio, "audio/wav"))
            .await
            .unwrap();
        let seen = fetch.seen();
        assert_eq!(seen.len(), 1);
        assert_eq!(
            seen[0].url,
            "https://test-resource.cognitiveservices.azure.com/speechtotext/transcriptions:transcribe?api-version=2025-10-15"
        );
        assert_eq!(seen[0].headers["ocp-apim-subscription-key"], "test-api-key");
        assert!(!seen[0].headers.contains_key("api-key"));
        assert!(seen[0].headers["user-agent"].contains("ai-sdk-azure/"));
        let body = String::from_utf8(seen[0].body.clone()).unwrap();
        assert!(body.contains("name=\"audio\"; filename=\"audio.wav\""));
        assert!(body.contains("Content-Type: audio/wav"));
        assert!(body.contains("\r\n\r\n\u{1}\u{2}\u{3}\r\n"));
        let definition = body
            .split("name=\"definition\"\r\n\r\n")
            .nth(1)
            .unwrap()
            .split("\r\n")
            .next()
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(definition).unwrap(),
            json!({
                "enhancedMode": {"enabled": true, "model": "MAI-Transcribe-2", "modelOptions": {"timestamps": "segment"}},
            })
        );
    }
}

/// With a `token_provider`, the bearer token is added by the transport
/// (azure-openai-provider.ts:248-262), once, before fetch follows a redirect;
/// fetch drops `authorization` on a cross-origin hop. aimux follows redirects
/// in the helper layer and sends such a hop through the transport again with
/// `RequestCredentials::Omit`, so the token must not be added back there.
#[tokio::test]
async fn token_provider_bearer_is_not_sent_across_a_cross_origin_redirect() {
    use wiremock::matchers::any;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let origin = MockServer::start().await;
    let foreign = MockServer::start().await;
    Mock::given(any())
        .respond_with(
            ResponseTemplate::new(307)
                .insert_header("location", format!("{}/moved", foreign.uri()).as_str()),
        )
        .mount(&origin)
        .await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200).set_body_json(chat_response()))
        .mount(&foreign)
        .await;
    let provider = create_azure(AzureOpenAIProviderSettings {
        base_url: Some(format!("{}/openai", origin.uri())),
        token_provider: Some(Resolvable::Value("test-token".into())),
        ..Default::default()
    })
    .unwrap();
    provider
        .chat("test-deployment")
        .do_generate(&prompt())
        .await
        .unwrap();

    let first = origin.received_requests().await.unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].headers["authorization"], "Bearer test-token");
    let hop = foreign.received_requests().await.unwrap();
    assert_eq!(hop.len(), 1);
    assert!(
        !hop[0].headers.contains_key("authorization"),
        "the bearer token reached another origin"
    );
    assert!(
        hop[0].headers["user-agent"]
            .to_str()
            .unwrap()
            .contains("ai-sdk-azure/")
    );
}

/// Upstream: @ai-sdk/openai 4.0.83 src/responses/openai-responses-language-model.ts:324
/// and :1062-1080 write Azure's responses logprobs under `azure`, the
/// provider's own key, one token list per text part. The OpenAI wire output
/// reads them from there and gives the Chat Completions logprobs object: one
/// `content` list, and `bytes: null` where the token carries none.
#[tokio::test]
async fn responses_logprobs_reach_the_openai_output_under_the_azure_key() {
    let mut fixture = text_fixture();
    let logprobs = json!([{"token":"Word","logprob":-0.25,
        "top_logprobs":[{"token":"Word","logprob":-0.25}]}]);
    fixture["output"][0]["content"][0]["logprobs"] = logprobs.clone();
    let fetch = MockFetch::new(vec![Canned::json(&fixture)]);
    let options = CallOptions {
        provider_options: Some(
            aimux_core::shared::provider_namespace("azure", json!({"logprobs": true})).unwrap(),
        ),
        ..prompt()
    };
    let result = e2e_provider(&fetch)
        .responses("test-deployment")
        .do_generate(&options)
        .await
        .unwrap();
    let metadata = result.provider_metadata.as_ref().unwrap();
    assert!(!metadata.contains_key("openai"));
    assert_eq!(metadata["azure"]["logprobs"], json!([logprobs]));

    let completion = aimux_core::openai_output::to_chat_completion(&result, "test-deployment");
    assert_eq!(
        completion.choices[0].logprobs,
        Some(json!({
            "content": [{"token":"Word","logprob":-0.25,"bytes":null,
                "top_logprobs":[{"token":"Word","logprob":-0.25,"bytes":null}]}],
            "refusal": null
        }))
    );
}

/// Upstream: @ai-sdk/azure 4.0.84 src/azure-openai-provider.ts:434 puts
/// `azureOpenaiTools` (src/azure-openai-tools.ts: the OpenAI code
/// interpreter, file search, image generation and web search tools) on the
/// provider as `azure.tools`.
#[tokio::test]
async fn provider_tools_member_makes_the_azure_tool_subset() {
    let fetch = MockFetch::new(vec![Canned::json(&text_fixture())]);
    let azure = e2e_provider(&fetch);
    let tool = azure.tools.web_search(Default::default()).unwrap();
    assert_eq!(tool.id, "openai.web_search");
    let mut options = prompt();
    options.tools = Some(vec![aimux_core::options::Tool::Provider(tool)]);
    azure
        .responses("test-deployment")
        .do_generate(&options)
        .await
        .unwrap();
    assert_eq!(
        assert_request(&fetch, "responses")["tools"],
        json!([{"type":"web_search"}])
    );
}
