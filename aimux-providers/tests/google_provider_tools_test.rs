//! Tests for Google provider-defined tools (`google_search`, `code_execution`,
//! `url_context`, `google_maps`) and their response metadata (`groundingMetadata`,
//! `urlContextMetadata`).
//!
//! Translated from the TypeScript sources in
//! `reference/ai/packages/google/src/`:
//! - `google-prepare-tools.test.ts` — `prepareTools` request-body shape & warnings.
//! - `google-language-model.test.ts` — `doGenerate` / `doStream` provider-tool
//!   request bodies, grounding/url-context metadata parsing, source extraction,
//!   code-execution tool calls, and server-side `toolCall`/`toolResponse` parts.
//!
//! # TDD status (red tests by design)
//!
//! Provider-defined tools are NOT yet implemented in the Rust port:
//! - `aimux_providers::google::convert::prepare_tools` only accepts
//!   `FunctionTool`s; `build_request_body` filters out `Tool::Provider(_)`.
//! - `GoogleModel::do_generate` / `do_stream` do not parse `executableCode` /
//!   `codeExecutionResult` / `toolCall` / `toolResponse` parts, do not extract
//!   sources from `groundingMetadata`, and the stream's `Finish` metadata is a
//!   hard-coded `{ "google": {} }`.
//!
//! Therefore most tests here are expected to FAIL until the feature lands —
//! that is the intended TDD "red" state. They compile against the types that
//! already exist (`Tool::Provider`, `ProviderTool`, `GenerateContent::Source`,
//! `StreamPart::{ToolCall,ToolResult,Source}`). Tests that require a type or
//! function which does not exist at all in the Rust port are marked
//! `#[ignore]` with an explanatory comment.

use futures::StreamExt;
use serde_json::{Value, json};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::content::ContentPart;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelPrompt, LanguageModelPromptMessage};
use aimux_core::message::Role;
use aimux_core::options::{CallOptions, ProviderTool, Tool};
use aimux_core::result::{GenerateContent, StreamResult};
use aimux_core::stream_part::StreamPart;
use aimux_core::types::Warning;

use aimux_providers::google::convert::build_request_body;
use aimux_providers::{GoogleConfig, GoogleProvider};

// ── helpers ─────────────────────────────────────────────────────────────────

/// The TS `TEST_PROMPT`: a single user text message "Hello".
fn test_prompt() -> LanguageModelPrompt {
    vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![ContentPart::text("Hello")],
        ..Default::default()
    }]
}

/// Build `CallOptions` with everything unset except `prompt`.
fn default_options(prompt: LanguageModelPrompt) -> CallOptions {
    CallOptions::new(prompt)
}

/// Build `CallOptions` carrying a pre-built `Vec<Tool>` (function and/or
/// provider-defined tools).
fn options_with_tools(prompt: LanguageModelPrompt, tools: Vec<Tool>) -> CallOptions {
    let mut opts = default_options(prompt);
    opts.tools = Some(tools);
    opts
}

/// Construct a provider-defined `Tool` from its dotted `id`, `name`, and `args`.
fn provider_tool(id: &str, name: &str, args: Value) -> Tool {
    Tool::Provider(ProviderTool {
        id: id.to_string(),
        name: name.to_string(),
        args,
    })
}

fn google_search_tool() -> Tool {
    provider_tool("google.google_search", "google_search", json!({}))
}

fn code_execution_tool() -> Tool {
    provider_tool("google.code_execution", "code_execution", json!({}))
}

fn url_context_tool() -> Tool {
    provider_tool("google.url_context", "url_context", json!({}))
}

fn google_maps_tool() -> Tool {
    provider_tool("google.google_maps", "google_maps", json!({}))
}

/// Build a `GoogleProvider` pointed at a mock `server`.
fn provider_at(uri: &str) -> GoogleProvider {
    GoogleProvider::new(GoogleConfig::new("test-api-key").with_base_url(uri))
}

/// Mock a JSON `generateContent` response for `model`.
async fn mock_json_response(server: &MockServer, model: &str, body: Value) {
    Mock::given(method("POST"))
        .and(path(format!("/models/{}:generateContent", model)))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

/// Mock an SSE `streamGenerateContent` response for `model`.
async fn mock_sse_response(server: &MockServer, model: &str, sse_body: String) {
    Mock::given(method("POST"))
        .and(path(format!("/models/{}:streamGenerateContent", model)))
        .and(query_param("alt", "sse"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse_body),
        )
        .mount(server)
        .await;
}

/// Build an SSE body from a sequence of JSON chunk values.
fn sse_body(chunks: &[Value]) -> String {
    let mut body = String::new();
    for chunk in chunks {
        body.push_str(&format!("data: {}\n\n", chunk));
    }
    body
}

/// Collect all `StreamPart`s from a `StreamResult` into a `Vec`.
async fn collect_stream(result: StreamResult) -> Vec<StreamPart> {
    let mut parts = Vec::new();
    let mut stream = result.stream;
    while let Some(part) = stream.next().await {
        match part {
            Ok(p) => parts.push(p),
            Err(e) => panic!("stream error: {:?}", e),
        }
    }
    parts
}

/// Extract `(tool_call_id, tool_name, input)` from streamed `ToolCall` parts.
fn stream_tool_calls(parts: &[StreamPart]) -> Vec<(String, String, Value)> {
    parts
        .iter()
        .filter_map(|p| match p {
            StreamPart::ToolCall {
                tool_call_id,
                tool_name,
                input,
                ..
            } => Some((tool_call_id.clone(), tool_name.clone(), input.clone())),
            _ => None,
        })
        .collect()
}

/// Extract `(tool_call_id, output)` from streamed `ToolResult` parts.
///
/// NOTE: `StreamPart::ToolResult` has no `tool_name` field — the TS asserts
/// `toolName: "code_execution"` on results, which cannot be expressed here yet.
fn stream_tool_results(parts: &[StreamPart]) -> Vec<(String, Value)> {
    parts
        .iter()
        .filter_map(|p| match p {
            StreamPart::ToolResult {
                tool_call_id,
                result,
                ..
            } => Some((tool_call_id.clone(), result.clone())),
            _ => None,
        })
        .collect()
}

/// Extract `(id, source_type, url, title)` from streamed `Source` parts.
fn stream_sources(parts: &[StreamPart]) -> Vec<(String, String, Option<String>, Option<String>)> {
    parts
        .iter()
        .filter_map(|p| match p {
            StreamPart::Source {
                id,
                source_type,
                url,
                title,
                ..
            } => Some((id.clone(), source_type.clone(), url.clone(), title.clone())),
            _ => None,
        })
        .collect()
}

/// Extract `(tool_call_id, tool_name, input)` from `GenerateContent::ToolCall`.
fn gen_tool_calls(content: &[GenerateContent]) -> Vec<(String, String, Value)> {
    content
        .iter()
        .filter_map(|c| match c {
            GenerateContent::ToolCall {
                tool_call_id,
                tool_name,
                input,
                ..
            } => Some((tool_call_id.clone(), tool_name.clone(), input.clone())),
            _ => None,
        })
        .collect()
}

/// Extract `(tool_call_id, tool_name, result)` from `GenerateContent::ToolResult`.
fn gen_tool_results(content: &[GenerateContent]) -> Vec<(String, String, Value)> {
    content
        .iter()
        .filter_map(|c| match c {
            GenerateContent::ToolResult {
                tool_call_id,
                tool_name,
                result,
                ..
            } => Some((tool_call_id.clone(), tool_name.clone(), result.clone())),
            _ => None,
        })
        .collect()
}

/// The `provider_metadata` of the `GenerateContent::ToolCall` / `ToolResult`
/// whose `tool_name` is `name`.
fn gen_server_metadata(content: &[GenerateContent], name: &str) -> Vec<Value> {
    content
        .iter()
        .filter_map(|c| match c {
            GenerateContent::ToolCall {
                tool_name,
                provider_metadata,
                ..
            }
            | GenerateContent::ToolResult {
                tool_name,
                provider_metadata,
                ..
            } if tool_name == name => provider_metadata.clone(),
            _ => None,
        })
        .collect()
}

/// Extract `(id, source_type, url, title)` from `GenerateContent::Source`.
///
/// NOTE: `GenerateContent::Source` has no `filename` / `media_type` fields —
/// the TS `document` sources carry those, which cannot be expressed here yet.
fn gen_sources(
    content: &[GenerateContent],
) -> Vec<(String, String, Option<String>, Option<String>)> {
    content
        .iter()
        .filter_map(|c| match c {
            GenerateContent::Source {
                id,
                source_type,
                url,
                title,
                ..
            } => Some((id.clone(), source_type.clone(), url.clone(), title.clone())),
            _ => None,
        })
        .collect()
}

// ════════════════════════════════════════════════════════════════════════════
// prepareTools / request body
//
// Translated from `google-prepare-tools.test.ts` and the "search tool
// selection" describe block in `google-language-model.test.ts`. The Rust port
// routes everything through `build_request_body` (the public entry point) —
// the standalone `prepare_tools` only accepts `FunctionTool`s.
// ════════════════════════════════════════════════════════════════════════════

mod prepare_tools {
    use super::*;

    // ── provider-defined tools → googleSearch / codeExecution / urlContext ─────

    #[test]
    fn single_google_search_tool_becomes_google_search() {
        // TS: "should correctly prepare single provider-defined tool"
        let body = build_request_body(
            "gemini-2.5-flash",
            &options_with_tools(test_prompt(), vec![google_search_tool()]),
        );
        assert_eq!(body["tools"], json!([{ "googleSearch": {} }]));
        assert!(body.get("toolConfig").is_none());
    }

    #[test]
    fn provider_tools_become_array_of_tool_objects() {
        // TS: "should correctly prepare provider-defined tools as array"
        let body = build_request_body(
            "gemini-2.5-flash",
            &options_with_tools(
                test_prompt(),
                vec![
                    google_search_tool(),
                    url_context_tool(),
                    provider_tool(
                        "google.file_search",
                        "file_search",
                        json!({ "fileSearchStoreNames": ["projects/foo/fileSearchStores/bar"] }),
                    ),
                ],
            ),
        );
        assert_eq!(
            body["tools"],
            json!([
                { "googleSearch": {} },
                { "urlContext": {} },
                { "fileSearch": { "fileSearchStoreNames": ["projects/foo/fileSearchStores/bar"] } }
            ])
        );
        assert!(body.get("toolConfig").is_none());
    }

    #[test]
    fn code_execution_tool_becomes_code_execution() {
        // TS: "should handle code execution tool"
        let body = build_request_body(
            "gemini-2.5-flash",
            &options_with_tools(test_prompt(), vec![code_execution_tool()]),
        );
        assert_eq!(body["tools"], json!([{ "codeExecution": {} }]));
        assert!(body.get("toolConfig").is_none());
    }

    #[test]
    fn url_context_tool_becomes_url_context() {
        // TS: "should handle url context tool alone"
        let body = build_request_body(
            "gemini-2.5-flash",
            &options_with_tools(test_prompt(), vec![url_context_tool()]),
        );
        assert_eq!(body["tools"], json!([{ "urlContext": {} }]));
        assert!(body.get("toolConfig").is_none());
    }

    #[test]
    fn google_maps_tool_becomes_google_maps() {
        // TS: "should handle google maps tool"
        let body = build_request_body(
            "gemini-2.5-flash",
            &options_with_tools(test_prompt(), vec![google_maps_tool()]),
        );
        assert_eq!(body["tools"], json!([{ "googleMaps": {} }]));
        assert!(body.get("toolConfig").is_none());
    }

    #[test]
    fn google_search_passes_search_types_args_through() {
        // TS: "should pass searchTypes args through for google search"
        let body = build_request_body(
            "gemini-3.1-flash-image-preview",
            &options_with_tools(
                test_prompt(),
                vec![provider_tool(
                    "google.google_search",
                    "google_search",
                    json!({ "searchTypes": { "webSearch": {}, "imageSearch": {} } }),
                )],
            ),
        );
        assert_eq!(
            body["tools"],
            json!([{ "googleSearch": { "searchTypes": { "webSearch": {}, "imageSearch": {} } } }])
        );
    }

    #[test]
    fn google_search_passes_time_range_filter_args_through() {
        // TS: "should pass timeRangeFilter args through for google search"
        let body = build_request_body(
            "gemini-2.5-flash",
            &options_with_tools(
                test_prompt(),
                vec![provider_tool(
                    "google.google_search",
                    "google_search",
                    json!({
                        "timeRangeFilter": {
                            "startTime": "2025-01-01T00:00:00Z",
                            "endTime": "2025-12-31T23:59:59Z",
                        }
                    }),
                )],
            ),
        );
        assert_eq!(
            body["tools"][0]["googleSearch"]["timeRangeFilter"],
            json!({
                "startTime": "2025-01-01T00:00:00Z",
                "endTime": "2025-12-31T23:59:59Z",
            })
        );
    }

    // ── combination of function + provider-defined tools (Gemini 3) ───────────

    #[test]
    fn combine_function_and_provider_tools_on_gemini_3() {
        // TS: "should combine function and provider-defined tools on Gemini 3"
        // Gemini 3 keeps both function and provider tools, emits VALIDATED mode
        // with `includeServerSideToolInvocations: true`.
        let function_tool = Tool::Function(
            aimux_core::tool::FunctionTool::new(
                "testFunction".to_string(),
                json!({ "type": "object", "properties": {} }),
            )
            .with_description("A test function".to_string()),
        );
        let body = build_request_body(
            "gemini-3.1-flash-lite-preview",
            &options_with_tools(test_prompt(), vec![google_search_tool(), function_tool]),
        );
        assert_eq!(
            body["tools"],
            json!([
                { "googleSearch": {} },
                { "functionDeclarations": [
                    { "name": "testFunction", "description": "A test function" }
                ] }
            ])
        );
        assert_eq!(
            body["toolConfig"],
            json!({
                "functionCallingConfig": { "mode": "VALIDATED" },
                "includeServerSideToolInvocations": true,
            })
        );
    }

    #[test]
    fn newest_tool_support_for_unknown_future_model() {
        // TS: "should use newest tool support for an unknown future Gemini model"
        let function_tool = Tool::Function(
            aimux_core::tool::FunctionTool::new(
                "getWeather".to_string(),
                json!({
                    "type": "object",
                    "properties": { "location": { "type": "string" } }
                }),
            )
            .with_description("Get the weather".to_string()),
        );
        let body = build_request_body(
            "gemini-99-pro-preview",
            &options_with_tools(
                test_prompt(),
                vec![
                    google_search_tool(),
                    provider_tool(
                        "google.enterprise_web_search",
                        "enterprise_web_search",
                        json!({}),
                    ),
                    url_context_tool(),
                    code_execution_tool(),
                    provider_tool(
                        "google.file_search",
                        "file_search",
                        json!({ "fileSearchStoreNames": ["fileSearchStores/example-store"] }),
                    ),
                    function_tool,
                ],
            ),
        );
        assert_eq!(
            body["tools"],
            json!([
                { "googleSearch": {} },
                { "enterpriseWebSearch": {} },
                { "urlContext": {} },
                { "codeExecution": {} },
                { "fileSearch": { "fileSearchStoreNames": ["fileSearchStores/example-store"] } },
                { "functionDeclarations": [
                    { "name": "getWeather", "description": "Get the weather",
                      "parameters": { "type": "object",
                                      "properties": { "location": { "type": "string" } } } }
                ] }
            ])
        );
        assert_eq!(
            body["toolConfig"],
            json!({
                "functionCallingConfig": { "mode": "VALIDATED" },
                "includeServerSideToolInvocations": true,
            })
        );
    }

    // ── unsupported-tool warnings (surface via do_generate) ───────────────────
    //
    // `build_request_body` does not return warnings, so these go through
    // `do_generate` and assert on `result.warnings`.

    #[tokio::test]
    async fn warn_for_unsupported_provider_tool() {
        // TS: "should add warnings for unsupported tools"
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            "gemini-2.5-flash",
            json!({
                "candidates": [{
                    "content": { "parts": [{ "text": "hi" }], "role": "model" },
                    "finishReason": "STOP",
                    "index": 0
                }]
            }),
        )
        .await;

        let model = provider_at(&server.uri()).model("gemini-2.5-flash");
        let result = model
            .do_generate(&options_with_tools(
                test_prompt(),
                vec![provider_tool(
                    "unsupported.tool",
                    "unsupported_tool",
                    json!({}),
                )],
            ))
            .await
            .expect("do_generate should succeed");

        let has_warning = result.warnings.iter().any(|w| matches!(
            w,
            Warning::Unsupported { feature, .. } if feature == "provider-defined tool unsupported.tool"
        ));
        assert!(
            has_warning,
            "expected an unsupported-tool warning, got {:?}",
            result.warnings
        );
    }

    #[tokio::test]
    async fn warn_for_google_search_on_unsupported_model() {
        // TS: "should add warnings for google search on unsupported models"
        //     + "should warn for google search on non-gemini-2 models"
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            "gemini-1.5-flash",
            json!({
                "candidates": [{
                    "content": { "parts": [{ "text": "hi" }], "role": "model" },
                    "finishReason": "STOP",
                    "index": 0
                }]
            }),
        )
        .await;

        let model = provider_at(&server.uri()).model("gemini-1.5-flash");
        let result = model
            .do_generate(&options_with_tools(
                test_prompt(),
                vec![google_search_tool()],
            ))
            .await
            .expect("do_generate should succeed");

        let has_warning = result.warnings.iter().any(|w| matches!(
            w,
            Warning::Unsupported { feature, details } if feature == "provider-defined tool google.google_search"
                && details.as_deref() == Some("Google Search requires Gemini 2.0 or newer.")
        ));
        assert!(
            has_warning,
            "expected a google_search unsupported-model warning, got {:?}",
            result.warnings
        );
    }

    #[tokio::test]
    async fn warn_when_mixing_function_and_provider_tools_pre_gemini_3() {
        // TS: "should warn when mixing function and provider-defined tools"
        // On pre-Gemini-3 models the function tool is dropped and a warning is
        // emitted for the combination.
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            "gemini-2.5-flash",
            json!({
                "candidates": [{
                    "content": { "parts": [{ "text": "hi" }], "role": "model" },
                    "finishReason": "STOP",
                    "index": 0
                }]
            }),
        )
        .await;

        let function_tool = Tool::Function(
            aimux_core::tool::FunctionTool::new(
                "testFunction".to_string(),
                json!({ "type": "object", "properties": {} }),
            )
            .with_description("A test function".to_string()),
        );
        let model = provider_at(&server.uri()).model("gemini-2.5-flash");
        let result = model
            .do_generate(&options_with_tools(
                test_prompt(),
                vec![function_tool, google_search_tool()],
            ))
            .await
            .expect("do_generate should succeed");

        let has_warning = result.warnings.iter().any(|w| matches!(
            w,
            Warning::Unsupported { feature, .. } if feature == "combination of function and provider-defined tools"
        ));
        assert!(
            has_warning,
            "expected a mixed-tools warning, got {:?}",
            result.warnings
        );
        // The request body should keep only the provider tool (function dropped).
        let body = result.request_body.as_ref().expect("request body");
        assert_eq!(body["tools"], json!([{ "googleSearch": {} }]));
    }

    // ── search tool selection (request body via do_generate) ──────────────────

    #[tokio::test]
    async fn search_tool_selection_google_search_for_gemini_2_0_pro() {
        // TS: "should use googleSearch for gemini-2.0-pro"
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            "gemini-2.0-pro",
            json!({
                "candidates": [{
                    "content": { "parts": [{ "text": "hi" }], "role": "model" },
                    "finishReason": "STOP",
                    "index": 0
                }]
            }),
        )
        .await;

        let model = provider_at(&server.uri()).model("gemini-2.0-pro");
        let result = model
            .do_generate(&options_with_tools(
                test_prompt(),
                vec![google_search_tool()],
            ))
            .await
            .expect("do_generate should succeed");

        let body = result.request_body.as_ref().expect("request body");
        assert_eq!(body["tools"], json!([{ "googleSearch": {} }]));
    }

    #[tokio::test]
    async fn search_tool_selection_url_context_for_gemini_2_0_pro() {
        // TS: "should use urlContextTool for gemini-2.0-pro"
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            "gemini-2.0-pro",
            json!({
                "candidates": [{
                    "content": { "parts": [{ "text": "hi" }], "role": "model" },
                    "finishReason": "STOP",
                    "index": 0
                }]
            }),
        )
        .await;

        let model = provider_at(&server.uri()).model("gemini-2.0-pro");
        let result = model
            .do_generate(&options_with_tools(test_prompt(), vec![url_context_tool()]))
            .await
            .expect("do_generate should succeed");

        let body = result.request_body.as_ref().expect("request body");
        assert_eq!(body["tools"], json!([{ "urlContext": {} }]));
    }
}

// ════════════════════════════════════════════════════════════════════════════
// doGenerate — response parsing (grounding metadata, sources, code execution,
// server-side tools)
//
// Translated from `google-language-model.test.ts` `describe('doGenerate')`.
// ════════════════════════════════════════════════════════════════════════════

mod do_generate {
    use super::*;

    /// A `prepareJsonResponse`-shaped body: one candidate with a text part,
    /// `finishReason: STOP`, and optional `groundingMetadata`.
    fn json_text_response(content: &str, grounding: Option<Value>) -> Value {
        let mut candidate = json!({
            "content": { "parts": [{ "text": content }], "role": "model" },
            "finishReason": "STOP",
            "index": 0
        });
        if let Some(g) = grounding {
            candidate["groundingMetadata"] = g;
        }
        json!({
            "candidates": [candidate],
            "usageMetadata": { "promptTokenCount": 1, "candidatesTokenCount": 2, "totalTokenCount": 3 }
        })
    }

    // ── grounding metadata exposed in provider metadata ───────────────────────
    // TS: "should expose grounding metadata in provider metadata"

    #[tokio::test]
    async fn expose_grounding_metadata_in_provider_metadata() {
        let grounding = json!({
            "webSearchQueries": ["What's the weather in Chicago this weekend?"],
            "searchEntryPoint": { "renderedContent": "Sample rendered content for search results" },
            "groundingChunks": [{
                "web": { "uri": "https://example.com/weather", "title": "Chicago Weather Forecast" }
            }],
            "groundingSupports": [{
                "segment": {
                    "startIndex": 0,
                    "endIndex": 65,
                    "text": "Chicago weather changes rapidly, so layers let you adjust easily."
                },
                "groundingChunkIndices": [0],
                "confidenceScores": [0.99]
            }],
            "retrievalMetadata": { "webDynamicRetrievalScore": 0.96879 }
        });
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            "gemini-pro",
            json_text_response("test response", Some(grounding.clone())),
        )
        .await;

        let model = provider_at(&server.uri()).model("gemini-pro");
        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("do_generate should succeed");

        let pm = result
            .provider_metadata
            .as_ref()
            .expect("provider metadata");
        assert_eq!(pm["google"]["groundingMetadata"], grounding);
    }

    // ── source extraction from grounding metadata ─────────────────────────────
    //
    // The Rust port does not yet extract `GenerateContent::Source` items from
    // `groundingMetadata.groundingChunks`. These tests are red until that lands.

    #[tokio::test]
    async fn extract_sources_from_grounding_metadata() {
        // TS: "should extract sources from grounding metadata"
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            "gemini-pro",
            json_text_response(
                "test response",
                Some(json!({
                    "groundingChunks": [{
                        "web": { "uri": "https://source.example.com", "title": "Source Title" }
                    }]
                })),
            ),
        )
        .await;

        let model = provider_at(&server.uri()).model("gemini-pro");
        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("do_generate should succeed");

        // Text part should be present.
        assert!(result.content.iter().any(|c| matches!(
            c,
            GenerateContent::Text { text, .. } if text == "test response"
        )));
        // A url source should be extracted (red: not implemented).
        let sources = gen_sources(&result.content);
        let has_url = sources.iter().any(|(_, st, url, title)| {
            st == "url"
                && url.as_deref() == Some("https://source.example.com")
                && title.as_deref() == Some("Source Title")
        });
        assert!(has_url, "expected a url source, got {:?}", sources);
    }

    #[tokio::test]
    async fn extract_sources_from_rag_retrieved_context() {
        // TS: "should extract sources from RAG retrievedContext chunks"
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            "gemini-pro",
            json_text_response(
                "test response with RAG",
                Some(json!({
                    "groundingChunks": [
                        { "web": { "uri": "https://web.example.com", "title": "Web Source" } },
                        { "retrievedContext": {
                            "uri": "gs://rag-corpus/document.pdf",
                            "title": "RAG Document",
                            "text": "Retrieved context..."
                        } },
                        { "retrievedContext": {
                            "uri": "https://external-rag-source.com/page",
                            "title": "External RAG Source",
                            "text": "External retrieved context..."
                        } }
                    ]
                })),
            ),
        )
        .await;

        let model = provider_at(&server.uri()).model("gemini-pro");
        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("do_generate should succeed");

        let sources = gen_sources(&result.content);
        // web → url source; gs:// → document source; external rag → url source.
        assert!(sources.iter().any(|(_, st, url, title)| {
            st == "url"
                && url.as_deref() == Some("https://web.example.com")
                && title.as_deref() == Some("Web Source")
        }));
        // document source from gs:// (filename/mediaType not representable in Rust Source).
        assert!(sources.iter().any(|(_, st, url, title)| {
            st == "document" && url.is_none() && title.as_deref() == Some("RAG Document")
        }));
        assert!(sources.iter().any(|(_, st, url, title)| {
            st == "url"
                && url.as_deref() == Some("https://external-rag-source.com/page")
                && title.as_deref() == Some("External RAG Source")
        }));
    }

    #[tokio::test]
    async fn extract_sources_from_file_search_retrieved_context() {
        // TS: "should extract sources from File Search retrievedContext (new format)"
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            "gemini-pro",
            json_text_response(
                "test response with File Search",
                Some(json!({
                    "groundingChunks": [
                        { "retrievedContext": {
                            "text": "Sample content for testing...",
                            "fileSearchStore": "fileSearchStores/test-store-xyz",
                            "title": "Test Document"
                        } },
                        { "retrievedContext": {
                            "text": "Another document content...",
                            "fileSearchStore": "fileSearchStores/another-store-abc"
                        } }
                    ]
                })),
            ),
        )
        .await;

        let model = provider_at(&server.uri()).model("gemini-pro");
        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("do_generate should succeed");

        let sources = gen_sources(&result.content);
        assert!(sources.iter().any(|(_, st, _, title)| {
            st == "document" && title.as_deref() == Some("Test Document")
        }));
        // Missing title defaults to "Unknown Document".
        assert!(sources.iter().any(|(_, st, _, title)| {
            st == "document" && title.as_deref() == Some("Unknown Document")
        }));
    }

    #[tokio::test]
    async fn handle_url_sources_with_undefined_title() {
        // TS: "should handle URL sources with undefined title correctly"
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            "gemini-pro",
            json_text_response(
                "test response with URLs",
                Some(json!({
                    "groundingChunks": [
                        { "web": { "uri": "https://example.com/page1" } },
                        { "retrievedContext": { "uri": "https://example.com/page2" } }
                    ]
                })),
            ),
        )
        .await;

        let model = provider_at(&server.uri()).model("gemini-pro");
        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("do_generate should succeed");

        let sources = gen_sources(&result.content);
        assert!(sources.iter().any(|(_, st, url, title)| {
            st == "url" && url.as_deref() == Some("https://example.com/page1") && title.is_none()
        }));
        assert!(sources.iter().any(|(_, st, url, title)| {
            st == "url" && url.as_deref() == Some("https://example.com/page2") && title.is_none()
        }));
    }

    #[tokio::test]
    async fn extract_sources_from_maps_grounding_metadata() {
        // TS: "should extract sources from maps grounding metadata"
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            "gemini-pro",
            json_text_response(
                "test response with Maps",
                Some(json!({
                    "groundingChunks": [
                        { "maps": {
                            "uri": "https://maps.google.com/maps?cid=12345",
                            "title": "Best Italian Restaurant",
                            "placeId": "ChIJ12345"
                        } },
                        { "maps": { "uri": "https://maps.google.com/maps?cid=67890" } }
                    ]
                })),
            ),
        )
        .await;

        let model = provider_at(&server.uri()).model("gemini-pro");
        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("do_generate should succeed");

        let sources = gen_sources(&result.content);
        assert!(sources.iter().any(|(_, st, url, title)| {
            st == "url"
                && url.as_deref() == Some("https://maps.google.com/maps?cid=12345")
                && title.as_deref() == Some("Best Italian Restaurant")
        }));
        assert!(sources.iter().any(|(_, st, url, title)| {
            st == "url"
                && url.as_deref() == Some("https://maps.google.com/maps?cid=67890")
                && title.is_none()
        }));
    }

    #[tokio::test]
    async fn extract_sources_from_image_grounding_metadata() {
        // TS: "should extract sources from image grounding metadata"
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            "gemini-pro",
            json_text_response(
                "test response with image search",
                Some(json!({
                    "groundingChunks": [
                        { "image": {
                            "sourceUri": "https://example.com/article",
                            "imageUri": "https://example.com/image.jpg",
                            "title": "Image Result",
                            "domain": "example.com"
                        } },
                        { "image": {
                            "sourceUri": "https://other.example.com/page",
                            "imageUri": "https://other.example.com/photo.png"
                        } }
                    ]
                })),
            ),
        )
        .await;

        let model = provider_at(&server.uri()).model("gemini-pro");
        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("do_generate should succeed");

        let sources = gen_sources(&result.content);
        assert!(sources.iter().any(|(_, st, url, title)| {
            st == "url"
                && url.as_deref() == Some("https://example.com/article")
                && title.as_deref() == Some("Image Result")
        }));
        assert!(sources.iter().any(|(_, st, url, title)| {
            st == "url"
                && url.as_deref() == Some("https://other.example.com/page")
                && title.is_none()
        }));
    }

    #[tokio::test]
    async fn handle_mixed_source_types_with_title_defaults() {
        // TS: "should handle mixed source types with correct title defaults"
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            "gemini-pro",
            json_text_response(
                "test response with mixed sources",
                Some(json!({
                    "groundingChunks": [
                        { "web": { "uri": "https://web.example.com" } },
                        { "retrievedContext": { "uri": "https://external.example.com" } },
                        { "retrievedContext": { "uri": "gs://bucket/document.pdf" } },
                        { "retrievedContext": { "fileSearchStore": "fileSearchStores/store-123" } }
                    ]
                })),
            ),
        )
        .await;

        let model = provider_at(&server.uri()).model("gemini-pro");
        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("do_generate should succeed");

        let sources = gen_sources(&result.content);
        assert!(sources.iter().any(|(_, st, url, title)| {
            st == "url" && url.as_deref() == Some("https://web.example.com") && title.is_none()
        }));
        assert!(sources.iter().any(|(_, st, url, title)| {
            st == "url" && url.as_deref() == Some("https://external.example.com") && title.is_none()
        }));
        // gs:// bucket path → document source with "Unknown Document" title.
        assert!(sources.iter().any(|(_, st, _, title)| {
            st == "document" && title.as_deref() == Some("Unknown Document")
        }));
    }

    // ── code execution tool calls ─────────────────────────────────────────────
    //
    // The request-body assertion is a normal red test. The content assertions
    // for the `tool-call` part compile (`GenerateContent::ToolCall` exists) but
    // are red because `executableCode` parts are not parsed. The TS also expects
    // a `tool-result` content item with `providerExecuted: true`; Rust's
    // `GenerateContent` has no `ToolResult` variant and `ToolCall` has no
    // `provider_executed` flag — see the `#[ignore]`'d test below.

    #[tokio::test]
    async fn code_execution_request_body_contains_code_execution() {
        // TS: "should handle code execution tool calls" (request body portion)
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            "gemini-2.0-pro",
            json!({
                "candidates": [{
                    "content": {
                        "parts": [
                            { "executableCode": { "language": "PYTHON", "code": "print(1+1)" } },
                            { "codeExecutionResult": { "outcome": "OUTCOME_OK", "output": "2" } }
                        ],
                        "role": "model"
                    },
                    "finishReason": "STOP"
                }]
            }),
        )
        .await;

        let model = provider_at(&server.uri()).model("gemini-2.0-pro");
        let result = model
            .do_generate(&options_with_tools(
                test_prompt(),
                vec![code_execution_tool()],
            ))
            .await
            .expect("do_generate should succeed");

        let body = result.request_body.as_ref().expect("request body");
        assert_eq!(body["tools"], json!([{ "codeExecution": {} }]));
    }

    #[tokio::test]
    async fn code_execution_tool_call_content() {
        // TS: "should handle code execution tool calls" (content portion)
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            "gemini-2.0-pro",
            json!({
                "candidates": [{
                    "content": {
                        "parts": [
                            { "executableCode": { "language": "PYTHON", "code": "print(1+1)" } },
                            { "codeExecutionResult": { "outcome": "OUTCOME_OK", "output": "2" } }
                        ],
                        "role": "model"
                    },
                    "finishReason": "STOP"
                }]
            }),
        )
        .await;

        let model = provider_at(&server.uri()).model("gemini-2.0-pro");
        let result = model
            .do_generate(&options_with_tools(
                test_prompt(),
                vec![code_execution_tool()],
            ))
            .await
            .expect("do_generate should succeed");

        let calls = gen_tool_calls(&result.content);
        let has_call = calls.iter().any(|(_, name, input)| {
            name == "code_execution"
                && *input == json!({ "language": "PYTHON", "code": "print(1+1)" })
        });
        assert!(
            has_call,
            "expected a code_execution tool-call, got {:?}",
            calls
        );
        // The tool-result half of the TS assertion lives in
        // `code_execution_tool_result_content` below.
    }

    /// TS: "should handle code execution tool calls" — the `tool-result` portion.
    ///
    /// `GenerateContent::ToolResult` now exists, so the TS assertion is
    /// expressible: the `codeExecutionResult` part becomes a `ToolResult`
    /// named `code_execution` carrying `{ outcome, output }`, paired with the
    /// id of the `executableCode` call it answers.
    #[tokio::test]
    async fn code_execution_tool_result_content() {
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            "gemini-2.0-pro",
            json!({
                "candidates": [{
                    "content": {
                        "parts": [
                            { "executableCode": { "language": "PYTHON", "code": "print(1+1)" } },
                            { "codeExecutionResult": { "outcome": "OUTCOME_OK", "output": "2" } }
                        ],
                        "role": "model"
                    },
                    "finishReason": "STOP"
                }]
            }),
        )
        .await;

        let model = provider_at(&server.uri()).model("gemini-2.0-pro");
        let result = model
            .do_generate(&options_with_tools(
                test_prompt(),
                vec![code_execution_tool()],
            ))
            .await
            .expect("do_generate should succeed");

        let calls = gen_tool_calls(&result.content);
        assert_eq!(calls.len(), 1, "one executableCode part → one tool-call");
        assert_eq!(calls[0].1, "code_execution");
        assert_eq!(
            calls[0].2,
            json!({ "language": "PYTHON", "code": "print(1+1)" })
        );

        let results = gen_tool_results(&result.content);
        assert_eq!(
            results.len(),
            1,
            "one codeExecutionResult part → one tool-result"
        );
        assert_eq!(results[0].1, "code_execution");
        assert_eq!(
            results[0].2,
            json!({ "outcome": "OUTCOME_OK", "output": "2" }),
            "the TS `{{ outcome, output }}` result shape"
        );
        assert_eq!(
            results[0].0, calls[0].0,
            "the result must carry the id of the call it answers"
        );
        assert!(
            !results[0].0.is_empty(),
            "tool_call_id must not be the empty string"
        );
    }

    #[tokio::test]
    async fn code_execution_result_with_missing_output_field() {
        // TS: "should handle code execution result with missing output field"
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            "gemini-2.0-pro",
            json!({
                "candidates": [{
                    "content": {
                        "parts": [
                            { "executableCode": {
                                "language": "PYTHON",
                                "code": "import PIL.Image\nimg = PIL.Image.open('input.png')\nimg.save('output.png')\n"
                            } },
                            { "codeExecutionResult": { "outcome": "OUTCOME_OK" } }
                        ],
                        "role": "model"
                    },
                    "finishReason": "STOP"
                }]
            }),
        )
        .await;

        let model = provider_at(&server.uri()).model("gemini-2.0-pro");
        let result = model
            .do_generate(&options_with_tools(
                test_prompt(),
                vec![code_execution_tool()],
            ))
            .await
            .expect("do_generate should succeed");

        let calls = gen_tool_calls(&result.content);
        let has_call = calls.iter().any(|(_, name, _)| name == "code_execution");
        assert!(
            has_call,
            "expected a code_execution tool-call, got {:?}",
            calls
        );
        // NOTE: TS asserts the result's output defaults to "" when missing —
        // pending GenerateContent::ToolResult.
    }

    // ── code execution finish reason ──────────────────────────────────────────

    #[tokio::test]
    async fn stop_finish_reason_for_code_execution() {
        // TS: "should return stop finish reason for code execution (provider-executed tool)"
        // Provider-executed tools must NOT trigger a 'tool-calls' finish reason.
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            "gemini-2.0-pro",
            json!({
                "candidates": [{
                    "content": {
                        "parts": [
                            { "executableCode": { "language": "PYTHON", "code": "print(1+1)" } },
                            { "codeExecutionResult": { "outcome": "OUTCOME_OK", "output": "2" } }
                        ],
                        "role": "model"
                    },
                    "finishReason": "STOP"
                }]
            }),
        )
        .await;

        let model = provider_at(&server.uri()).model("gemini-2.0-pro");
        let result = model
            .do_generate(&options_with_tools(
                test_prompt(),
                vec![code_execution_tool()],
            ))
            .await
            .expect("do_generate should succeed");

        assert_eq!(
            result.finish_reason.unified,
            aimux_core::types::FinishReasonUnified::Stop
        );
        assert_eq!(result.finish_reason.raw.as_deref(), Some("STOP"));
        // The code execution tool call should be present (red: not parsed).
        assert!(
            gen_tool_calls(&result.content)
                .iter()
                .any(|(_, name, _)| name == "code_execution"),
            "expected a code_execution tool-call in content"
        );
    }

    #[tokio::test]
    async fn stop_finish_reason_for_code_execution_with_text_response() {
        // TS: "should return stop finish reason for code execution with text
        //     response (structured output scenario)"
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            "gemini-2.0-pro",
            json!({
                "candidates": [{
                    "content": {
                        "parts": [
                            { "executableCode": {
                                "language": "PYTHON",
                                "code": "primes = [2, 3, 5, 7, 11]\nprint(sum(primes))"
                            } },
                            { "codeExecutionResult": { "outcome": "OUTCOME_OK", "output": "28" } },
                            { "text": "{\"answer\": 28, \"explanation\": \"Sum of first 5 primes\"}" }
                        ],
                        "role": "model"
                    },
                    "finishReason": "STOP"
                }]
            }),
        )
        .await;

        let model = provider_at(&server.uri()).model("gemini-2.0-pro");
        let result = model
            .do_generate(&options_with_tools(
                test_prompt(),
                vec![code_execution_tool()],
            ))
            .await
            .expect("do_generate should succeed");

        assert_eq!(
            result.finish_reason.unified,
            aimux_core::types::FinishReasonUnified::Stop
        );
        // Text content is included.
        assert!(result.content.iter().any(|c| matches!(
            c,
            GenerateContent::Text { text, .. } if text == "{\"answer\": 28, \"explanation\": \"Sum of first 5 primes\"}"
        )));
        // The code execution tool call should be present (red: not parsed).
        assert!(
            gen_tool_calls(&result.content)
                .iter()
                .any(|(_, name, _)| name == "code_execution"),
            "expected a code_execution tool-call in content"
        );
    }

    #[tokio::test]
    async fn tool_calls_finish_reason_when_code_execution_combined_with_function_tools() {
        // TS: "should return tool-calls finish reason when code execution is
        //     combined with function tools"
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            "gemini-2.0-pro",
            json!({
                "candidates": [{
                    "content": {
                        "parts": [
                            { "executableCode": { "language": "PYTHON", "code": "print(1+1)" } },
                            { "codeExecutionResult": { "outcome": "OUTCOME_OK", "output": "2" } },
                            { "functionCall": { "name": "test-tool", "args": { "value": "test" } } }
                        ],
                        "role": "model"
                    },
                    "finishReason": "STOP"
                }]
            }),
        )
        .await;

        let function_tool = Tool::Function(aimux_core::tool::FunctionTool::new(
            "test-tool".to_string(),
            json!({ "type": "object", "properties": { "value": { "type": "string" } } }),
        ));
        let model = provider_at(&server.uri()).model("gemini-2.0-pro");
        let result = model
            .do_generate(&options_with_tools(
                test_prompt(),
                vec![code_execution_tool(), function_tool],
            ))
            .await
            .expect("do_generate should succeed");

        // STOP + a client-executed function tool → 'tool-calls'.
        assert_eq!(
            result.finish_reason.unified,
            aimux_core::types::FinishReasonUnified::ToolCalls
        );
        assert_eq!(result.finish_reason.raw.as_deref(), Some("STOP"));
        // The code execution tool call should ALSO be present (red: not parsed).
        assert!(
            gen_tool_calls(&result.content)
                .iter()
                .any(|(_, name, _)| name == "code_execution"),
            "expected a code_execution tool-call alongside the function call"
        );
    }

    // ── server-side toolCall / toolResponse parts ─────────────────────────────
    //
    // Gemini 3 can return `toolCall` + `toolResponse` parts for provider tools
    // (e.g. GOOGLE_SEARCH_WEB). Both are parsed now: the call becomes a
    // `ToolCall` named `server:<toolType>` and the response a `ToolResult` with
    // the same name, each carrying
    // `providerMetadata.google.{serverToolCallId,serverToolType,thoughtSignature}`.

    #[tokio::test]
    async fn handle_server_side_tool_call_and_tool_response_parts() {
        // TS: "should handle server-side toolCall and toolResponse parts (tool combination)"
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            "gemini-3-pro-preview",
            json!({
                "candidates": [{
                    "content": {
                        "parts": [
                            { "toolCall": {
                                "toolType": "GOOGLE_SEARCH_WEB",
                                "args": { "query": "San Francisco weather" },
                                "id": "server-call-1"
                              }, "thoughtSignature": "sig-abc" },
                            { "toolResponse": {
                                "toolType": "GOOGLE_SEARCH_WEB",
                                "response": { "results": [{ "title": "Weather in SF" }] },
                                "id": "server-call-1"
                              }, "thoughtSignature": "sig-def" },
                            { "text": "The weather in San Francisco is sunny." }
                        ],
                        "role": "model"
                    },
                    "finishReason": "STOP"
                }],
                "usageMetadata": { "promptTokenCount": 10, "candidatesTokenCount": 20, "totalTokenCount": 30 }
            }),
        )
        .await;

        let function_tool = Tool::Function(aimux_core::tool::FunctionTool::new(
            "weather".to_string(),
            json!({ "type": "object", "properties": { "location": { "type": "string" } } }),
        ));
        let model = provider_at(&server.uri()).model("gemini-3-pro-preview");
        let result = model
            .do_generate(&options_with_tools(
                test_prompt(),
                vec![google_search_tool(), function_tool],
            ))
            .await
            .expect("do_generate should succeed");

        // Text is present.
        assert!(result.content.iter().any(|c| matches!(
            c,
            GenerateContent::Text { text, .. } if text == "The weather in San Francisco is sunny."
        )));

        // Server tool call with toolName "server:GOOGLE_SEARCH_WEB".
        let calls = gen_tool_calls(&result.content);
        assert_eq!(calls.len(), 1, "one toolCall part → one tool-call");
        assert_eq!(calls[0].0, "server-call-1", "the server-assigned id");
        assert_eq!(calls[0].1, "server:GOOGLE_SEARCH_WEB");
        assert_eq!(calls[0].2, json!({ "query": "San Francisco weather" }));

        // The matching toolResponse part.
        let results = gen_tool_results(&result.content);
        assert_eq!(results.len(), 1, "one toolResponse part → one tool-result");
        assert_eq!(
            results[0].0, "server-call-1",
            "the result is paired with its call"
        );
        assert_eq!(results[0].1, "server:GOOGLE_SEARCH_WEB");
        assert_eq!(
            results[0].2,
            json!({ "results": [{ "title": "Weather in SF" }] }),
            "the server tool's response payload must survive verbatim"
        );

        // Per-part providerMetadata: server ids/types plus the thoughtSignature
        // (which must be echoed back on the follow-up turn).
        let meta = gen_server_metadata(&result.content, "server:GOOGLE_SEARCH_WEB");
        assert_eq!(meta.len(), 2, "both the call and the result carry metadata");
        assert_eq!(
            meta[0]["google"],
            json!({
                "serverToolCallId": "server-call-1",
                "serverToolType": "GOOGLE_SEARCH_WEB",
                "thoughtSignature": "sig-abc",
            })
        );
        assert_eq!(
            meta[1]["google"],
            json!({
                "serverToolCallId": "server-call-1",
                "serverToolType": "GOOGLE_SEARCH_WEB",
                "thoughtSignature": "sig-def",
            })
        );

        // Provider-executed tools do NOT flip the finish reason to tool-calls.
        assert_eq!(
            result.finish_reason.unified,
            aimux_core::types::FinishReasonUnified::Stop
        );
    }

    #[tokio::test]
    async fn stop_finish_reason_for_server_tool_calls() {
        // TS: "should return stop finish reason for server tool calls (provider-executed)"
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            "gemini-3-pro-preview",
            json!({
                "candidates": [{
                    "content": {
                        "parts": [
                            { "toolCall": { "toolType": "GOOGLE_SEARCH_WEB", "args": {}, "id": "sc-1" } },
                            { "toolResponse": { "toolType": "GOOGLE_SEARCH_WEB", "response": { "results": [] }, "id": "sc-1" } }
                        ],
                        "role": "model"
                    },
                    "finishReason": "STOP"
                }],
                "usageMetadata": { "promptTokenCount": 5, "candidatesTokenCount": 10, "totalTokenCount": 15 }
            }),
        )
        .await;

        let model = provider_at(&server.uri()).model("gemini-3-pro-preview");
        let result = model
            .do_generate(&options_with_tools(
                test_prompt(),
                vec![google_search_tool()],
            ))
            .await
            .expect("do_generate should succeed");

        // Provider-executed server tools → stop, not tool-calls.
        assert_eq!(
            result.finish_reason.unified,
            aimux_core::types::FinishReasonUnified::Stop
        );
        assert_eq!(result.finish_reason.raw.as_deref(), Some("STOP"));
        // The server tool call should be present in content (red: not parsed).
        assert!(
            gen_tool_calls(&result.content)
                .iter()
                .any(|(_, name, _)| name == "server:GOOGLE_SEARCH_WEB"),
            "expected a server-side tool-call in content"
        );
    }
}

// ════════════════════════════════════════════════════════════════════════════
// doStream — streaming provider tools (grounding/url-context metadata on
// finish, code execution, server-side tools, source events)
//
// Translated from `google-language-model.test.ts` `describe('doStream')`.
// ════════════════════════════════════════════════════════════════════════════

mod do_stream {
    use super::*;

    /// A single SSE chunk carrying `content.parts: [{ text }]`, optional
    /// `finishReason`, and optional `groundingMetadata` / `urlContextMetadata`.
    fn chunk(
        text: &str,
        finish_reason: Option<&str>,
        grounding: Option<Value>,
        url_context: Option<Value>,
    ) -> Value {
        let mut candidate = json!({
            "content": { "parts": [{ "text": text }], "role": "model" }
        });
        if let Some(r) = finish_reason {
            candidate["finishReason"] = json!(r);
        }
        if let Some(g) = grounding {
            candidate["groundingMetadata"] = g;
        }
        if let Some(u) = url_context {
            candidate["urlContextMetadata"] = u;
        }
        json!({ "candidates": [candidate] })
    }

    /// Extract the `Finish` part's provider metadata from a collected stream.
    fn finish_provider_metadata(parts: &[StreamPart]) -> Option<Value> {
        parts.iter().find_map(|p| match p {
            StreamPart::Finish {
                provider_metadata, ..
            } => provider_metadata.clone(),
            _ => None,
        })
    }

    // ── grounding / url context metadata on finish ────────────────────────────

    #[tokio::test]
    async fn expose_grounding_metadata_in_provider_metadata_on_finish() {
        // TS: "should expose grounding metadata in provider metadata on finish"
        let grounding = json!({
            "webSearchQueries": ["What's the weather in Chicago this weekend?"],
            "searchEntryPoint": { "renderedContent": "Sample rendered content for search results" },
            "groundingChunks": [{
                "web": { "uri": "https://example.com/weather", "title": "Chicago Weather Forecast" }
            }],
            "groundingSupports": [{
                "segment": {
                    "startIndex": 0,
                    "endIndex": 65,
                    "text": "Chicago weather changes rapidly, so layers let you adjust easily."
                },
                "groundingChunkIndices": [0],
                "confidenceScores": [0.99]
            }],
            "retrievalMetadata": { "webDynamicRetrievalScore": 0.96879 }
        });
        let server = MockServer::start().await;
        mock_sse_response(
            &server,
            "gemini-pro",
            sse_body(&[chunk("test", Some("STOP"), Some(grounding.clone()), None)]),
        )
        .await;

        let model = provider_at(&server.uri()).model("gemini-pro");
        let result = model
            .do_stream(&default_options(test_prompt()))
            .await
            .expect("do_stream should succeed");
        let parts = collect_stream(result).await;

        let pm = finish_provider_metadata(&parts).expect("finish part");
        assert_eq!(pm["google"]["groundingMetadata"], grounding);
    }

    #[tokio::test]
    async fn preserve_grounding_metadata_before_finish_reason_chunk() {
        // TS: "should preserve grounding metadata when it arrives before the
        //     finishReason chunk"
        let server = MockServer::start().await;
        mock_sse_response(
            &server,
            "gemini-pro",
            sse_body(&[
                chunk(
                    "hello",
                    None,
                    Some(json!({
                        "webSearchQueries": ["super bowl 2026 halftime show"],
                        "groundingChunks": [{
                            "web": { "uri": "https://example.com/superbowl", "title": "Super Bowl 2026" }
                        }]
                    })),
                    None,
                ),
                {
                    let mut c = chunk(" world", Some("STOP"), None, None);
                    c["usageMetadata"] = json!({
                        "promptTokenCount": 38,
                        "candidatesTokenCount": 1335,
                        "totalTokenCount": 1890
                    });
                    c
                },
            ]),
        )
        .await;

        let model = provider_at(&server.uri()).model("gemini-pro");
        let result = model
            .do_stream(&default_options(test_prompt()))
            .await
            .expect("do_stream should succeed");
        let parts = collect_stream(result).await;

        let pm = finish_provider_metadata(&parts).expect("finish part");
        assert_eq!(
            pm["google"]["groundingMetadata"],
            json!({
                "webSearchQueries": ["super bowl 2026 halftime show"],
                "groundingChunks": [{
                    "web": { "uri": "https://example.com/superbowl", "title": "Super Bowl 2026" }
                }]
            })
        );
        // A source event should be emitted for the grounding chunk (red).
        assert_eq!(stream_sources(&parts).len(), 1);
    }

    #[tokio::test]
    async fn preserve_url_context_metadata_before_finish_reason_chunk() {
        // TS: "should preserve url context metadata when it arrives before the
        //     finishReason chunk"
        let server = MockServer::start().await;
        mock_sse_response(
            &server,
            "gemini-pro",
            sse_body(&[
                chunk(
                    "hello",
                    None,
                    None,
                    Some(json!({
                        "urlMetadata": [{
                            "retrievedUrl": "https://example.com/page",
                            "urlRetrievalStatus": "URL_RETRIEVAL_STATUS_SUCCESS"
                        }]
                    })),
                ),
                {
                    let mut c = chunk(" world", Some("STOP"), None, None);
                    c["usageMetadata"] = json!({
                        "promptTokenCount": 10,
                        "candidatesTokenCount": 5,
                        "totalTokenCount": 15
                    });
                    c
                },
            ]),
        )
        .await;

        let model = provider_at(&server.uri()).model("gemini-pro");
        let result = model
            .do_stream(&default_options(test_prompt()))
            .await
            .expect("do_stream should succeed");
        let parts = collect_stream(result).await;

        let pm = finish_provider_metadata(&parts).expect("finish part");
        assert_eq!(
            pm["google"]["urlContextMetadata"],
            json!({
                "urlMetadata": [{
                    "retrievedUrl": "https://example.com/page",
                    "urlRetrievalStatus": "URL_RETRIEVAL_STATUS_SUCCESS"
                }]
            })
        );
    }

    #[tokio::test]
    async fn expose_url_context_metadata_in_provider_metadata_on_finish() {
        // TS: "should expose url context metadata in provider metadata on finish"
        let url_context = json!({
            "urlMetadata": [{
                "retrievedUrl": "https://example.com/weather",
                "urlRetrievalStatus": "URL_RETRIEVAL_STATUS_SUCCESS"
            }]
        });
        let server = MockServer::start().await;
        mock_sse_response(
            &server,
            "gemini-pro",
            sse_body(&[chunk("test", Some("STOP"), None, Some(url_context.clone()))]),
        )
        .await;

        let model = provider_at(&server.uri()).model("gemini-pro");
        let result = model
            .do_stream(&default_options(test_prompt()))
            .await
            .expect("do_stream should succeed");
        let parts = collect_stream(result).await;

        let pm = finish_provider_metadata(&parts).expect("finish part");
        assert_eq!(pm["google"]["urlContextMetadata"], url_context);
    }

    // ── streaming code execution ──────────────────────────────────────────────

    #[tokio::test]
    async fn stream_code_execution_tool_calls_and_results() {
        // TS: "should stream code execution tool calls and results"
        let server = MockServer::start().await;
        mock_sse_response(
            &server,
            "gemini-2.0-pro",
            sse_body(&[
                json!({
                    "candidates": [{
                        "content": {
                            "parts": [{ "executableCode": { "language": "PYTHON", "code": "print(\"hello\")" } }]
                        }
                    }]
                }),
                json!({
                    "candidates": [{
                        "content": {
                            "parts": [{ "codeExecutionResult": { "outcome": "OUTCOME_OK", "output": "hello\n" } }]
                        },
                        "finishReason": "STOP"
                    }]
                }),
            ]),
        )
        .await;

        let model = provider_at(&server.uri()).model("gemini-2.0-pro");
        let result = model
            .do_stream(&options_with_tools(
                test_prompt(),
                vec![code_execution_tool()],
            ))
            .await
            .expect("do_stream should succeed");
        let parts = collect_stream(result).await;

        let calls = stream_tool_calls(&parts);
        let has_call = calls.iter().any(|(_, name, input)| {
            name == "code_execution"
                && *input == json!({ "language": "PYTHON", "code": "print(\"hello\")" })
        });
        assert!(
            has_call,
            "expected a code_execution tool-call, got {:?}",
            calls
        );

        let results = stream_tool_results(&parts);
        let has_result = results
            .iter()
            .any(|(_, output)| *output == json!({ "outcome": "OUTCOME_OK", "output": "hello\n" }));
        assert!(
            has_result,
            "expected a code_execution tool-result, got {:?}",
            results
        );
        // NOTE: TS also asserts toolName: "code_execution" on the result and
        // providerExecuted: true on the call — not expressible on current
        // StreamPart variants.
    }

    #[tokio::test]
    async fn stream_code_execution_result_with_missing_output_field() {
        // TS: "should stream code execution result with missing output field"
        let server = MockServer::start().await;
        mock_sse_response(
            &server,
            "gemini-2.0-pro",
            sse_body(&[
                json!({
                    "candidates": [{
                        "content": {
                            "parts": [{ "executableCode": {
                                "language": "PYTHON",
                                "code": "img = PIL.Image.open('input.png')\nimg.save('output.png')\n"
                            } }]
                        }
                    }]
                }),
                json!({
                    "candidates": [{
                        "content": {
                            "parts": [{ "codeExecutionResult": { "outcome": "OUTCOME_OK" } }]
                        },
                        "finishReason": "STOP"
                    }]
                }),
            ]),
        )
        .await;

        let model = provider_at(&server.uri()).model("gemini-2.0-pro");
        let result = model
            .do_stream(&options_with_tools(
                test_prompt(),
                vec![code_execution_tool()],
            ))
            .await
            .expect("do_stream should succeed");
        let parts = collect_stream(result).await;

        assert!(
            stream_tool_calls(&parts)
                .iter()
                .any(|(_, name, _)| name == "code_execution"),
            "expected a code_execution tool-call"
        );
        // Missing output defaults to "" (red: result not emitted at all yet).
        let results = stream_tool_results(&parts);
        let has_empty = results
            .iter()
            .any(|(_, output)| *output == json!({ "outcome": "OUTCOME_OK", "output": "" }));
        assert!(
            has_empty,
            "expected a tool-result with empty output, got {:?}",
            results
        );
    }

    #[tokio::test]
    async fn stop_finish_reason_for_streamed_code_execution() {
        // TS: "should return stop finish reason for streamed code execution
        //     (provider-executed tool)"
        let server = MockServer::start().await;
        mock_sse_response(
            &server,
            "gemini-2.0-pro",
            sse_body(&[
                json!({
                    "candidates": [{
                        "content": {
                            "parts": [{ "executableCode": { "language": "PYTHON", "code": "print(\"hello\")" } }]
                        }
                    }]
                }),
                json!({
                    "candidates": [{
                        "content": {
                            "parts": [
                                { "codeExecutionResult": { "outcome": "OUTCOME_OK", "output": "hello\n" } },
                                { "text": "{\"result\": \"hello\"}" }
                            ]
                        },
                        "finishReason": "STOP"
                    }]
                }),
            ]),
        )
        .await;

        let model = provider_at(&server.uri()).model("gemini-2.0-pro");
        let result = model
            .do_stream(&options_with_tools(
                test_prompt(),
                vec![code_execution_tool()],
            ))
            .await
            .expect("do_stream should succeed");
        let parts = collect_stream(result).await;

        let finish = parts.iter().find_map(|p| match p {
            StreamPart::Finish { finish_reason, .. } => Some(finish_reason),
            _ => None,
        });
        let finish = finish.expect("finish part");
        assert_eq!(finish.unified, aimux_core::types::FinishReasonUnified::Stop);
        assert_eq!(finish.raw.as_deref(), Some("STOP"));
        // Provider-executed tool call/result should be streamed (red).
        assert!(
            stream_tool_calls(&parts)
                .iter()
                .any(|(_, name, _)| name == "code_execution"),
            "expected a code_execution tool-call in the stream"
        );
    }

    /// TS: "should stream server-side toolCall and toolResponse parts (tool combination)".
    ///
    /// Both parts are streamed now: `StreamPart::ToolCall` /
    /// `StreamPart::ToolResult` named `server:<toolType>`, each carrying
    /// `providerMetadata.google.{serverToolCallId,serverToolType,thoughtSignature}`.
    #[tokio::test]
    async fn stream_server_side_tool_call_and_tool_response_parts() {
        let server = MockServer::start().await;
        mock_sse_response(
            &server,
            "gemini-3-pro-preview",
            sse_body(&[
                json!({
                    "candidates": [{
                        "content": {
                            "parts": [{ "toolCall": {
                                "toolType": "GOOGLE_SEARCH_WEB",
                                "args": { "query": "San Francisco weather" },
                                "id": "server-call-1"
                            }, "thoughtSignature": "sig-abc" }]
                        }
                    }]
                }),
                json!({
                    "candidates": [{
                        "content": {
                            "parts": [
                                { "toolResponse": {
                                    "toolType": "GOOGLE_SEARCH_WEB",
                                    "response": { "results": [{ "title": "Weather in SF" }] },
                                    "id": "server-call-1"
                                }, "thoughtSignature": "sig-def" },
                                { "text": "The weather in San Francisco is sunny." }
                            ]
                        },
                        "finishReason": "STOP"
                    }]
                }),
            ]),
        )
        .await;

        let function_tool = Tool::Function(aimux_core::tool::FunctionTool::new(
            "weather".to_string(),
            json!({ "type": "object", "properties": { "location": { "type": "string" } } }),
        ));
        let model = provider_at(&server.uri()).model("gemini-3-pro-preview");
        let result = model
            .do_stream(&options_with_tools(
                test_prompt(),
                vec![google_search_tool(), function_tool],
            ))
            .await
            .expect("do_stream should succeed");
        let parts = collect_stream(result).await;

        let calls = stream_tool_calls(&parts);
        assert_eq!(calls.len(), 1, "one toolCall part → one streamed tool-call");
        assert_eq!(calls[0].0, "server-call-1");
        assert_eq!(calls[0].1, "server:GOOGLE_SEARCH_WEB");
        assert_eq!(calls[0].2, json!({ "query": "San Francisco weather" }));

        let results = stream_tool_results(&parts);
        assert_eq!(
            results.len(),
            1,
            "one toolResponse part → one streamed tool-result"
        );
        assert_eq!(
            results[0].0, "server-call-1",
            "the streamed result is paired with its call"
        );
        assert_eq!(
            results[0].1,
            json!({ "results": [{ "title": "Weather in SF" }] }),
            "the server tool's response payload must survive verbatim"
        );

        // Per-part providerMetadata (ids/types + the thoughtSignature that must
        // be echoed back on the follow-up turn).
        let meta: Vec<Value> = parts
            .iter()
            .filter_map(|p| match p {
                StreamPart::ToolCall {
                    tool_name,
                    provider_metadata,
                    ..
                }
                | StreamPart::ToolResult {
                    tool_name,
                    provider_metadata,
                    ..
                } if tool_name == "server:GOOGLE_SEARCH_WEB" => provider_metadata.clone(),
                _ => None,
            })
            .collect();
        assert_eq!(meta.len(), 2, "both the call and the result carry metadata");
        assert_eq!(
            meta[0]["google"],
            json!({
                "serverToolCallId": "server-call-1",
                "serverToolType": "GOOGLE_SEARCH_WEB",
                "thoughtSignature": "sig-abc",
            })
        );
        assert_eq!(
            meta[1]["google"],
            json!({
                "serverToolCallId": "server-call-1",
                "serverToolType": "GOOGLE_SEARCH_WEB",
                "thoughtSignature": "sig-def",
            })
        );

        // The trailing text is still streamed, and the finish reason is Stop
        // (provider-executed tools do not flip it to tool-calls).
        let text: String = parts
            .iter()
            .filter_map(|p| match p {
                StreamPart::TextDelta { delta, .. } => Some(delta.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(text, "The weather in San Francisco is sunny.");
        let finish = parts
            .iter()
            .find_map(|p| match p {
                StreamPart::Finish { finish_reason, .. } => Some(finish_reason),
                _ => None,
            })
            .expect("finish part");
        assert_eq!(finish.unified, aimux_core::types::FinishReasonUnified::Stop);
    }

    // ── streaming source events ───────────────────────────────────────────────
    //
    // The Rust port does not yet emit `StreamPart::Source` from
    // `groundingMetadata`. These tests are red until that lands.

    #[tokio::test]
    async fn stream_source_events() {
        // TS: "should stream source events"
        let server = MockServer::start().await;
        mock_sse_response(
            &server,
            "gemini-pro",
            sse_body(&[chunk(
                "Some initial text",
                Some("STOP"),
                Some(json!({
                    "groundingChunks": [{
                        "web": { "uri": "https://source.example.com", "title": "Source Title" }
                    }]
                })),
                None,
            )]),
        )
        .await;

        let model = provider_at(&server.uri()).model("gemini-pro");
        let result = model
            .do_stream(&default_options(test_prompt()))
            .await
            .expect("do_stream should succeed");
        let parts = collect_stream(result).await;

        let sources = stream_sources(&parts);
        let has = sources.iter().any(|(_, st, url, title)| {
            st == "url"
                && url.as_deref() == Some("https://source.example.com")
                && title.as_deref() == Some("Source Title")
        });
        assert!(has, "expected a url source event, got {:?}", sources);
    }

    #[tokio::test]
    async fn stream_source_events_from_image_grounding_metadata() {
        // TS: "should stream source events from image grounding metadata"
        let server = MockServer::start().await;
        mock_sse_response(
            &server,
            "gemini-pro",
            sse_body(&[chunk(
                "image search response",
                Some("STOP"),
                Some(json!({
                    "groundingChunks": [{
                        "image": {
                            "sourceUri": "https://example.com/article",
                            "imageUri": "https://example.com/image.jpg",
                            "title": "Image Source",
                            "domain": "example.com"
                        }
                    }]
                })),
                None,
            )]),
        )
        .await;

        let model = provider_at(&server.uri()).model("gemini-pro");
        let result = model
            .do_stream(&default_options(test_prompt()))
            .await
            .expect("do_stream should succeed");
        let parts = collect_stream(result).await;

        let sources = stream_sources(&parts);
        let has = sources.iter().any(|(_, st, url, title)| {
            st == "url"
                && url.as_deref() == Some("https://example.com/article")
                && title.as_deref() == Some("Image Source")
        });
        assert!(has, "expected an image url source event, got {:?}", sources);
    }

    #[tokio::test]
    async fn stream_sources_during_intermediate_chunks() {
        // TS: "should stream sources during intermediate chunks"
        let server = MockServer::start().await;
        mock_sse_response(
            &server,
            "gemini-pro",
            sse_body(&[
                chunk(
                    "text",
                    None,
                    Some(json!({
                        "groundingChunks": [
                            { "web": { "uri": "https://a.com", "title": "A" } },
                            { "web": { "uri": "https://b.com", "title": "B" } }
                        ]
                    })),
                    None,
                ),
                chunk("more", Some("STOP"), None, None),
            ]),
        )
        .await;

        let model = provider_at(&server.uri()).model("gemini-pro");
        let result = model
            .do_stream(&default_options(test_prompt()))
            .await
            .expect("do_stream should succeed");
        let parts = collect_stream(result).await;

        let sources = stream_sources(&parts);
        assert!(sources.iter().any(|(_, st, url, title)| {
            st == "url" && url.as_deref() == Some("https://a.com") && title.as_deref() == Some("A")
        }));
        assert!(sources.iter().any(|(_, st, url, title)| {
            st == "url" && url.as_deref() == Some("https://b.com") && title.as_deref() == Some("B")
        }));
    }

    #[tokio::test]
    async fn deduplicate_sources_across_chunks() {
        // TS: "should deduplicate sources across chunks"
        let server = MockServer::start().await;
        mock_sse_response(
            &server,
            "gemini-pro",
            sse_body(&[
                chunk(
                    "first chunk",
                    None,
                    Some(json!({
                        "groundingChunks": [
                            { "web": { "uri": "https://example.com", "title": "Example" } },
                            { "web": { "uri": "https://unique.com", "title": "Unique" } }
                        ]
                    })),
                    None,
                ),
                chunk(
                    "second chunk",
                    None,
                    Some(json!({
                        "groundingChunks": [
                            { "web": { "uri": "https://example.com", "title": "Example Duplicate" } },
                            { "web": { "uri": "https://another.com", "title": "Another" } }
                        ]
                    })),
                    None,
                ),
                chunk("final chunk", Some("STOP"), None, None),
            ]),
        )
        .await;

        let model = provider_at(&server.uri()).model("gemini-pro");
        let result = model
            .do_stream(&default_options(test_prompt()))
            .await
            .expect("do_stream should succeed");
        let parts = collect_stream(result).await;

        let sources = stream_sources(&parts);
        // The duplicate https://example.com appears in two chunks but should be
        // emitted only once (keeping the first title).
        let example: Vec<_> = sources
            .iter()
            .filter(|(_, _, url, _)| url.as_deref() == Some("https://example.com"))
            .collect();
        assert_eq!(
            example.len(),
            1,
            "duplicate source should be emitted once, got {:?}",
            sources
        );
        assert_eq!(example[0].3.as_deref(), Some("Example"));

        // Unique + Another are also present.
        assert!(
            sources
                .iter()
                .any(|(_, _, url, _)| url.as_deref() == Some("https://unique.com"))
        );
        assert!(
            sources
                .iter()
                .any(|(_, _, url, _)| url.as_deref() == Some("https://another.com"))
        );
        assert_eq!(
            sources.len(),
            3,
            "expected 3 deduplicated sources, got {:?}",
            sources
        );
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Schema validation gaps
//
// The TS suite has `describe('groundingMetadataSchema')` (16 tests) and
// `describe('urlContextMetadata')` (3 tests) that validate the zod schemas
// returned by `getGroundingMetadataSchema()` / `getUrlContextMetadataSchema()`.
//
// The Rust port stores `groundingMetadata` / `urlContextMetadata` as untyped
// `serde_json::Value` (see `Candidate` in `types.rs`) and exposes them
// verbatim in provider metadata — there is no schema type to validate against.
// These schema-validation tests therefore have no direct Rust equivalent; the
// structural coverage they provide is instead carried by the response-parsing
// tests above, which assert on the concrete `groundingMetadata` shape.
//
// The ignored tests below are placeholders that document the gap; they build
// the same metadata payloads and would assert schema acceptance once a typed
// schema exists.
// ════════════════════════════════════════════════════════════════════════════

mod schema_validation_gaps {
    use super::*;

    #[test]
    #[ignore = "no get_grounding_metadata_schema() in Rust; metadata is untyped serde_json::Value"]
    fn grounding_metadata_schema_validates_web_search_results() {
        // TS: "validates complete grounding metadata with web search results"
        let _metadata = json!({
            "webSearchQueries": ["What's the weather in Chicago this weekend?"],
            "searchEntryPoint": { "renderedContent": "Sample rendered content for search results" },
            "groundingChunks": [{
                "web": { "uri": "https://example.com/weather", "title": "Chicago Weather Forecast" }
            }],
            "groundingSupports": [{
                "segment": { "startIndex": 0, "endIndex": 65, "text": "..." },
                "groundingChunkIndices": [0],
                "confidenceScores": [0.99]
            }],
            "retrievalMetadata": { "webDynamicRetrievalScore": 0.96879 }
        });
        // TODO: once a typed schema exists, assert it accepts this payload.
    }

    #[test]
    #[ignore = "no get_grounding_metadata_schema() in Rust; metadata is untyped serde_json::Value"]
    fn grounding_metadata_schema_validates_vertex_ai_search_results() {
        // TS: "validates complete grounding metadata with Vertex AI Search results"
        let _metadata = json!({
            "retrievalQueries": ["How to make appointment to renew driving license?"],
            "groundingChunks": [{
                "retrievedContext": {
                    "uri": "https://vertexaisearch.cloud.google.com/grounding-api-redirect/AXiHM.....QTN92V5ePQ==",
                    "title": "dmv"
                }
            }],
            "groundingSupports": [{
                "segment": { "startIndex": 25, "endIndex": 147 },
                "segment_text": "ipsum lorem ...",
                "supportChunkIndices": [1, 2],
                "confidenceScore": [0.9541752, 0.97726375]
            }]
        });
    }

    #[test]
    #[ignore = "no get_grounding_metadata_schema() in Rust; metadata is untyped serde_json::Value"]
    fn grounding_metadata_schema_validates_file_search_store_format() {
        // TS: "validates groundingChunks[].retrievedContext with fileSearchStore (new format)"
        let _metadata = json!({
            "groundingChunks": [{
                "retrievedContext": {
                    "text": "Sample content for testing...",
                    "fileSearchStore": "fileSearchStores/test-store-xyz",
                    "title": "Test Document"
                }
            }]
        });
    }

    #[test]
    #[ignore = "no get_grounding_metadata_schema() in Rust; metadata is untyped serde_json::Value"]
    fn grounding_metadata_schema_validates_image_and_maps_chunks() {
        // TS: "validates groundingChunks[].image" + "validates grounding metadata with maps chunks"
        let _metadata = json!({
            "imageSearchQueries": ["Super Bowl halftime show in space"],
            "groundingChunks": [
                { "image": {
                    "sourceUri": "https://example.com/article",
                    "imageUri": "https://example.com/image.jpg",
                    "title": "Image Title",
                    "domain": "example.com"
                } },
                { "maps": {
                    "uri": "https://maps.google.com/maps?cid=12345",
                    "title": "Best Italian Restaurant",
                    "text": "A great Italian restaurant",
                    "placeId": "ChIJ12345"
                } }
            ]
        });
    }

    #[test]
    #[ignore = "no get_grounding_metadata_schema() in Rust; metadata is untyped serde_json::Value"]
    fn grounding_metadata_schema_validates_partial_and_empty() {
        // TS: "validates partial grounding metadata" + "validates empty grounding metadata"
        let _partial = json!({ "webSearchQueries": ["sample query"] });
        let _empty = json!({});
    }

    #[test]
    #[ignore = "no get_grounding_metadata_schema() in Rust; metadata is untyped serde_json::Value"]
    fn grounding_metadata_schema_rejects_invalid_data_types() {
        // TS: "rejects invalid data types" — a typed schema would reject these.
        let _invalid = json!({
            "webSearchQueries": "not an array",
            "groundingSupports": [{ "confidenceScores": "not an array" }]
        });
    }

    #[test]
    #[ignore = "no get_url_context_metadata_schema() in Rust; metadata is untyped serde_json::Value"]
    fn url_context_metadata_schema_validates_output() {
        // TS: "validates complete url context output" + "validates empty url context output"
        let _complete = json!({
            "urlMetadata": [{
                "retrievedUrl": "https://example.com/weather",
                "urlRetrievalStatus": "URL_RETRIEVAL_STATUS_SUCCESS"
            }]
        });
        let _empty = json!({ "urlMetadata": [] });
    }
}
