//! Port of `src/responses/huggingface-responses-language-model.test.ts` from
//! `@ai-sdk/huggingface@2.0.62`.
//!
//! Two harnesses share the file: the newer cases use the [`MockFetch`]
//! transport, the older ones (formerly `tests/huggingface_responses_test.rs`)
//! a wiremock `MockServer` from [`super::support`].
//!
//! Harness differences from the TypeScript (expectations are unchanged):
//! - `createTestServer` is the [`MockFetch`] transport or the `MockServer`. `server.calls[0]` is the
//!   request body the transport recorded (`Seen::json_body`).
//! - `toMatchInlineSnapshot` values are JSON values in the assertions, with
//!   `undefined` properties absent (as `JSON.stringify` drops them).
//! - `mockId()` is not needed: the Rust model numbers its sources `id-N` itself.

use std::sync::Arc;

use crate::{HuggingFaceProvider, HuggingFaceProviderSettings, create_huggingface};
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{
    LanguageModelMessage, LanguageModelPrompt, ToolPart, ToolResultOutput, ToolResultPart,
};
use aimux_core::options::{CallOptions, ResponseFormat};
use aimux_core::result::StreamResult;
use futures::StreamExt;
use serde_json::{Value, json};

use crate::test_support::mock_fetch::{Canned, MockFetch};

use super::support::*;

const MODEL: &str = "deepseek-ai/DeepSeek-V3-0324";

// ── helpers ─────────────────────────────────────────────────────────────────

/// The TS `TEST_PROMPT`: one user text message "Hello".
fn test_prompt() -> LanguageModelPrompt {
    vec![LanguageModelMessage::user_text("Hello")]
}

/// `CallOptions` with only `prompt` set.
fn call_options(prompt: LanguageModelPrompt) -> CallOptions {
    CallOptions::new(prompt)
}

/// `createModel`: the Hugging Face provider with its default base URL, the
/// `Bearer APIKEY` header and the mock transport.
fn provider(fetch: &Arc<MockFetch>) -> HuggingFaceProvider {
    create_huggingface(HuggingFaceProviderSettings {
        api_key: Some("APIKEY".to_string().into()),
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .expect("valid settings")
}

/// One JSON response from `/responses`.
fn json_fetch(body: &Value) -> Arc<MockFetch> {
    MockFetch::new(vec![Canned::json(body)])
}

/// One event-stream response: each frame followed by a blank line.
fn sse_fetch(frames: &[impl AsRef<str>]) -> Arc<MockFetch> {
    let body: String = frames
        .iter()
        .map(|frame| format!("{}\n\n", frame.as_ref()))
        .collect();
    MockFetch::new(vec![Canned {
        status: 200,
        headers: vec![("content-type".into(), "text/event-stream".into())],
        body: body.into_bytes(),
    }])
}

/// Drain a stream into the JSON of each part (the TS `convertReadableStreamToArray`).
///
/// An `Err` item is the stream's error chunk: `enqueue_error` surfaces the
/// AI SDK's `{ type: 'error', error }` part as `Err` and the stream carries on.
async fn stream_parts(result: StreamResult) -> Vec<Value> {
    let mut stream = result.stream;
    let mut parts = Vec::new();
    while let Some(item) = stream.next().await {
        parts.push(match item {
            Ok(part) => serde_json::to_value(part).expect("serializable part"),
            Err(error) => json!({ "type": "error", "error": error.to_string() }),
        });
    }
    parts
}

/// The `beforeEach` response of the `basic text response` describe.
fn basic_text_response() -> Value {
    json!({
        "id": "resp_67c97c0203188190a025beb4a75242bc",
        "model": "deepseek-ai/DeepSeek-V3-0324",
        "object": "response",
        "created_at": 1741257730,
        "status": "completed",
        "error": null,
        "instructions": null,
        "max_output_tokens": null,
        "metadata": null,
        "tool_choice": "auto",
        "tools": [],
        "temperature": 1.0,
        "top_p": 1.0,
        "incomplete_details": null,
        "usage": {
            "input_tokens": 12,
            "output_tokens": 25,
            "total_tokens": 37
        },
        "output": [
            {
                "id": "msg_67c97c02656c81908e080dfdf4a03cd1",
                "type": "message",
                "role": "assistant",
                "status": "completed",
                "content": [
                    {
                        "type": "output_text",
                        "text": "Hello! How can I help you today?"
                    }
                ]
            }
        ],
        "output_text": "Hello! How can I help you today?"
    })
}

/// The `beforeEach` response of the `message conversion` describe.
fn message_conversion_response() -> Value {
    json!({
        "id": "resp_test",
        "model": "moonshotai/Kimi-K2-Instruct",
        "object": "response",
        "created_at": 1741257730,
        "status": "completed",
        "error": null,
        "instructions": null,
        "max_output_tokens": null,
        "metadata": null,
        "tool_choice": "auto",
        "tools": [],
        "temperature": 1.0,
        "top_p": 1.0,
        "incomplete_details": null,
        "usage": null,
        "output": [],
        "output_text": "Test response"
    })
}

// ─────────────────────────────────────────────────────────────────────────────

mod hugging_face_responses_language_model {
    use super::*;

    mod do_generate {
        use super::*;

        mod basic_text_response {
            use super::*;

            /// upstream: @ai-sdk/huggingface src/responses/huggingface-responses-language-model.test.ts › HuggingFaceResponsesLanguageModel › doGenerate › basic text response › should generate text
            #[tokio::test]
            async fn should_generate_text() {
                let fetch = json_fetch(&basic_text_response());
                let result = provider(&fetch)
                    .responses(MODEL)
                    .do_generate(&call_options(test_prompt()))
                    .await
                    .expect("should succeed");

                assert_eq!(
                    serde_json::to_value(&result.content).unwrap(),
                    json!([{
                        "providerMetadata": {
                            "huggingface": { "itemId": "msg_67c97c02656c81908e080dfdf4a03cd1" }
                        },
                        "text": "Hello! How can I help you today?",
                        "type": "text"
                    }])
                );
            }

            /// upstream: @ai-sdk/huggingface src/responses/huggingface-responses-language-model.test.ts › HuggingFaceResponsesLanguageModel › doGenerate › basic text response › should extract usage
            #[tokio::test]
            async fn should_extract_usage() {
                let fetch = json_fetch(&basic_text_response());
                let result = provider(&fetch)
                    .responses(MODEL)
                    .do_generate(&call_options(test_prompt()))
                    .await
                    .expect("should succeed");

                assert_eq!(
                    serde_json::to_value(&result.usage).unwrap(),
                    json!({
                        "inputTokens": { "cacheRead": 0, "noCache": 12, "total": 12 },
                        "outputTokens": { "reasoning": 0, "text": 25, "total": 25 },
                        "raw": { "input_tokens": 12, "output_tokens": 25, "total_tokens": 37 }
                    })
                );
            }

            /// upstream: @ai-sdk/huggingface src/responses/huggingface-responses-language-model.test.ts › HuggingFaceResponsesLanguageModel › doGenerate › basic text response › should extract text from output array when output_text is missing
            #[tokio::test]
            async fn should_extract_text_from_output_array_when_output_text_missing() {
                let fetch = json_fetch(&json!({
                    "id": "resp_test",
                    "model": "deepseek-ai/DeepSeek-V3-0324",
                    "object": "response",
                    "created_at": 1741257730,
                    "status": "completed",
                    "error": null,
                    "instructions": null,
                    "max_output_tokens": null,
                    "metadata": null,
                    "tool_choice": "auto",
                    "tools": [],
                    "temperature": 1.0,
                    "top_p": 1.0,
                    "incomplete_details": null,
                    "usage": null,
                    "output": [
                        {
                            "id": "msg_test",
                            "type": "message",
                            "role": "assistant",
                            "status": "completed",
                            "content": [
                                { "type": "output_text", "text": "Extracted from output array" }
                            ]
                        }
                    ],
                    "output_text": null
                }));
                let result = provider(&fetch)
                    .responses(MODEL)
                    .do_generate(&call_options(test_prompt()))
                    .await
                    .expect("should succeed");

                assert_eq!(
                    serde_json::to_value(&result.content).unwrap(),
                    json!([{
                        "providerMetadata": { "huggingface": { "itemId": "msg_test" } },
                        "text": "Extracted from output array",
                        "type": "text"
                    }])
                );
            }

            /// upstream: @ai-sdk/huggingface src/responses/huggingface-responses-language-model.test.ts › HuggingFaceResponsesLanguageModel › doGenerate › basic text response › should handle missing usage gracefully
            #[tokio::test]
            async fn should_handle_missing_usage_gracefully() {
                let fetch = json_fetch(&json!({
                    "id": "resp_test",
                    "model": "deepseek-ai/DeepSeek-V3-0324",
                    "object": "response",
                    "created_at": 1741257730,
                    "status": "completed",
                    "error": null,
                    "instructions": null,
                    "max_output_tokens": null,
                    "metadata": null,
                    "tool_choice": "auto",
                    "tools": [],
                    "temperature": 1.0,
                    "top_p": 1.0,
                    "incomplete_details": null,
                    "usage": null,
                    "output": [],
                    "output_text": "Test response"
                }));
                let result = provider(&fetch)
                    .responses(MODEL)
                    .do_generate(&call_options(test_prompt()))
                    .await
                    .expect("should succeed");

                // Every usage field is undefined, so none is serialized.
                assert_eq!(
                    serde_json::to_value(&result.usage).unwrap(),
                    json!({ "inputTokens": {}, "outputTokens": {} })
                );
            }

            /// upstream: @ai-sdk/huggingface src/responses/huggingface-responses-language-model.test.ts › HuggingFaceResponsesLanguageModel › doGenerate › basic text response › should send model id, settings, and input
            #[tokio::test]
            async fn should_send_model_id_settings_and_input() {
                let fetch = json_fetch(&basic_text_response());
                let mut call = call_options(vec![
                    LanguageModelMessage::System {
                        content: "You are a helpful assistant.".into(),
                        provider_options: None,
                    },
                    LanguageModelMessage::user_text("Hello"),
                ]);
                call.temperature = Some(0.5);
                call.top_p = Some(0.3);
                call.max_output_tokens = Some(100);

                provider(&fetch)
                    .responses(MODEL)
                    .do_generate(&call)
                    .await
                    .expect("should succeed");

                let seen = fetch.seen();
                assert_eq!(
                    seen[0].json_body(),
                    json!({
                        "model": "deepseek-ai/DeepSeek-V3-0324",
                        "temperature": 0.5,
                        "top_p": 0.3,
                        "max_output_tokens": 100,
                        "stream": false,
                        "input": [
                            { "role": "system", "content": "You are a helpful assistant." },
                            {
                                "role": "user",
                                "content": [{ "type": "input_text", "text": "Hello" }]
                            }
                        ]
                    })
                );
            }

            /// upstream: @ai-sdk/huggingface src/responses/huggingface-responses-language-model.test.ts › HuggingFaceResponsesLanguageModel › doGenerate › basic text response › should handle unsupported settings with warnings
            #[tokio::test]
            async fn should_handle_unsupported_settings_with_warnings() {
                let fetch = json_fetch(&basic_text_response());
                let mut call = call_options(test_prompt());
                call.top_k = Some(10.0);
                call.seed = Some(123);
                call.presence_penalty = Some(0.5);
                call.frequency_penalty = Some(0.3);
                call.stop_sequences = Some(vec!["stop".to_string()]);

                let result = provider(&fetch)
                    .responses(MODEL)
                    .do_generate(&call)
                    .await
                    .expect("should succeed");

                assert_eq!(
                    serde_json::to_value(&result.warnings).unwrap(),
                    json!([
                        { "feature": "topK", "type": "unsupported" },
                        { "feature": "seed", "type": "unsupported" },
                        { "feature": "presencePenalty", "type": "unsupported" },
                        { "feature": "frequencyPenalty", "type": "unsupported" },
                        { "feature": "stopSequences", "type": "unsupported" }
                    ])
                );
            }

            /// upstream: @ai-sdk/huggingface src/responses/huggingface-responses-language-model.test.ts › HuggingFaceResponsesLanguageModel › doGenerate › basic text response › should generate text and sources from annotations
            #[tokio::test]
            async fn should_generate_text_and_sources_from_annotations() {
                let fetch = json_fetch(&json!({
                    "id": "resp_test_annotations",
                    "model": "deepseek-ai/DeepSeek-V3-0324",
                    "object": "response",
                    "created_at": 1741257730,
                    "status": "completed",
                    "error": null,
                    "instructions": null,
                    "max_output_tokens": null,
                    "metadata": null,
                    "tool_choice": "auto",
                    "tools": [],
                    "temperature": 1.0,
                    "top_p": 1.0,
                    "incomplete_details": null,
                    "usage": { "input_tokens": 20, "output_tokens": 50, "total_tokens": 70 },
                    "output": [
                        {
                            "id": "msg_test_annotations",
                            "type": "message",
                            "role": "assistant",
                            "status": "completed",
                            "content": [
                                {
                                    "type": "output_text",
                                    "text": "Here are some recent articles about AI: The first article discusses new developments ([example.com](https://example.com/article1)). Another piece covers industry trends ([test.com](https://test.com/article2)).",
                                    "annotations": [
                                        {
                                            "type": "url_citation",
                                            "url": "https://example.com/article1",
                                            "title": "AI Developments Article"
                                        },
                                        {
                                            "type": "url_citation",
                                            "url": "https://test.com/article2",
                                            "title": "Industry Trends Report"
                                        }
                                    ]
                                }
                            ]
                        }
                    ],
                    "output_text": null
                }));
                let result = provider(&fetch)
                    .responses(MODEL)
                    .do_generate(&call_options(test_prompt()))
                    .await
                    .expect("should succeed");

                assert_eq!(
                    serde_json::to_value(&result.content).unwrap(),
                    json!([
                        {
                            "providerMetadata": {
                                "huggingface": { "itemId": "msg_test_annotations" }
                            },
                            "text": "Here are some recent articles about AI: The first article discusses new developments ([example.com](https://example.com/article1)). Another piece covers industry trends ([test.com](https://test.com/article2)).",
                            "type": "text"
                        },
                        {
                            "id": "id-0",
                            "sourceType": "url",
                            "title": "AI Developments Article",
                            "type": "source",
                            "url": "https://example.com/article1"
                        },
                        {
                            "id": "id-1",
                            "sourceType": "url",
                            "title": "Industry Trends Report",
                            "type": "source",
                            "url": "https://test.com/article2"
                        }
                    ])
                );
            }

            /// upstream: @ai-sdk/huggingface src/responses/huggingface-responses-language-model.test.ts › HuggingFaceResponsesLanguageModel › doGenerate › basic text response › should handle MCP tools with annotations
            #[tokio::test]
            async fn should_handle_mcp_tools_with_annotations() {
                let fetch = json_fetch(&json!({
                    "id": "resp_mcp_test",
                    "model": "deepseek-ai/DeepSeek-V3-0324",
                    "object": "response",
                    "created_at": 1741257730,
                    "status": "completed",
                    "error": null,
                    "instructions": null,
                    "max_output_tokens": null,
                    "metadata": null,
                    "tool_choice": "auto",
                    "tools": [],
                    "temperature": 1.0,
                    "top_p": 1.0,
                    "incomplete_details": null,
                    "usage": { "input_tokens": 50, "output_tokens": 100, "total_tokens": 150 },
                    "output": [
                        {
                            "id": "mcp_search_test",
                            "type": "mcp_call",
                            "server_label": "web_search",
                            "name": "search",
                            "arguments": "{\"query\": \"San Francisco tech events\"}",
                            "output": "Found 25 tech events in San Francisco"
                        },
                        {
                            "id": "msg_mcp_response",
                            "type": "message",
                            "role": "assistant",
                            "status": "completed",
                            "content": [
                                {
                                    "type": "output_text",
                                    "text": "Based on the search results, here are the latest tech events in San Francisco: There are several AI conferences ([techevents.com](https://techevents.com/sf-ai)) and startup meetups ([eventbrite.com](https://eventbrite.com/sf-startups)) happening this week.",
                                    "annotations": [
                                        {
                                            "type": "url_citation",
                                            "url": "https://techevents.com/sf-ai",
                                            "title": "SF AI Conference 2025"
                                        },
                                        {
                                            "type": "url_citation",
                                            "url": "https://eventbrite.com/sf-startups",
                                            "title": "SF Startup Meetups"
                                        }
                                    ]
                                }
                            ]
                        }
                    ],
                    "output_text": null
                }));
                let result = provider(&fetch)
                    .responses(MODEL)
                    .do_generate(&call_options(test_prompt()))
                    .await
                    .expect("should succeed");

                assert_eq!(
                    serde_json::to_value(&result.content).unwrap(),
                    json!([
                        {
                            "input": "{\"query\": \"San Francisco tech events\"}",
                            "providerExecuted": true,
                            "toolCallId": "mcp_search_test",
                            "toolName": "search",
                            "type": "tool-call"
                        },
                        {
                            "result": "Found 25 tech events in San Francisco",
                            "toolCallId": "mcp_search_test",
                            "toolName": "search",
                            "type": "tool-result"
                        },
                        {
                            "providerMetadata": {
                                "huggingface": { "itemId": "msg_mcp_response" }
                            },
                            "text": "Based on the search results, here are the latest tech events in San Francisco: There are several AI conferences ([techevents.com](https://techevents.com/sf-ai)) and startup meetups ([eventbrite.com](https://eventbrite.com/sf-startups)) happening this week.",
                            "type": "text"
                        },
                        {
                            "id": "id-0",
                            "sourceType": "url",
                            "title": "SF AI Conference 2025",
                            "type": "source",
                            "url": "https://techevents.com/sf-ai"
                        },
                        {
                            "id": "id-1",
                            "sourceType": "url",
                            "title": "SF Startup Meetups",
                            "type": "source",
                            "url": "https://eventbrite.com/sf-startups"
                        }
                    ])
                );
            }
        }
    }

    mod do_stream {
        use super::*;

        /// upstream: @ai-sdk/huggingface src/responses/huggingface-responses-language-model.test.ts › HuggingFaceResponsesLanguageModel › doStream › should stream text deltas
        #[tokio::test]
        async fn should_stream_text_deltas() {
            let fetch = sse_fetch(&[
                r#"data:{"type":"response.created","response":{"id":"resp_test","object":"response","created_at":1741269019,"status":"in_progress","model":"deepseek-ai/DeepSeek-V3-0324"}}"#,
                r#"data:{"type":"response.in_progress","response":{"id":"resp_test","object":"response","created_at":1741269019,"status":"in_progress"}}"#,
                r#"data:{"type":"response.output_item.added","output_index":0,"item":{"id":"msg_test","type":"message","role":"assistant","status":"in_progress","content":[]},"sequence_number":1}"#,
                r#"data:{"type":"response.output_text.delta","item_id":"msg_test","output_index":0,"content_index":0,"delta":"Hello,","sequence_number":2}"#,
                r#"data:{"type":"response.output_text.delta","item_id":"msg_test","output_index":0,"content_index":0,"delta":" World!","sequence_number":3}"#,
                r#"data:{"type":"response.output_item.done","output_index":0,"item":{"id":"msg_test","type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":"Hello, World!"}]},"sequence_number":4}"#,
                r#"data:{"type":"response.completed","response":{"id":"resp_test","model":"deepseek-ai/DeepSeek-V3-0324","object":"response","created_at":1741269112,"status":"completed","incomplete_details":null,"usage":{"input_tokens":12,"output_tokens":25,"total_tokens":37},"output":[{"id":"msg_test","type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":"Hello, World!"}]}]},"sequence_number":5}"#,
            ]);
            let result = provider(&fetch)
                .responses(MODEL)
                .do_stream(&call_options(test_prompt()))
                .await
                .expect("should succeed");

            assert_eq!(
                Value::Array(stream_parts(result).await),
                json!([
                    { "type": "stream-start", "warnings": [] },
                    {
                        "id": "resp_test",
                        "modelId": "deepseek-ai/DeepSeek-V3-0324",
                        "timestamp": "2025-03-06T13:50:19.000Z",
                        "type": "response-metadata"
                    },
                    {
                        "id": "msg_test",
                        "providerMetadata": { "huggingface": { "itemId": "msg_test" } },
                        "type": "text-start"
                    },
                    { "delta": "Hello,", "id": "msg_test", "type": "text-delta" },
                    { "delta": " World!", "id": "msg_test", "type": "text-delta" },
                    { "id": "msg_test", "type": "text-end" },
                    {
                        "finishReason": { "unified": "stop" },
                        "providerMetadata": { "huggingface": { "responseId": "resp_test" } },
                        "type": "finish",
                        "usage": {
                            "inputTokens": { "cacheRead": 0, "noCache": 12, "total": 12 },
                            "outputTokens": { "reasoning": 0, "text": 25, "total": 25 },
                            "raw": { "input_tokens": 12, "output_tokens": 25, "total_tokens": 37 }
                        }
                    }
                ])
            );
        }

        /// upstream: @ai-sdk/huggingface src/responses/huggingface-responses-language-model.test.ts › HuggingFaceResponsesLanguageModel › doStream › should handle streaming without usage
        #[tokio::test]
        async fn should_handle_streaming_without_usage() {
            let fetch = sse_fetch(&[
                r#"data:{"type":"response.output_item.added","output_index":0,"item":{"id":"msg_test","type":"message","role":"assistant","status":"in_progress"},"sequence_number":1}"#,
                r#"data:{"type":"response.output_text.delta","item_id":"msg_test","output_index":0,"content_index":0,"delta":"Hi!","sequence_number":2}"#,
                r#"data:{"type":"response.output_item.done","output_index":0,"item":{"id":"msg_test","type":"message","role":"assistant","status":"completed"},"sequence_number":3}"#,
                r#"data:{"type":"response.completed","response":{"id":"resp_test","status":"completed","incomplete_details":null,"usage":null},"sequence_number":4}"#,
            ]);
            let result = provider(&fetch)
                .responses(MODEL)
                .do_stream(&call_options(test_prompt()))
                .await
                .expect("should succeed");

            let parts = stream_parts(result).await;
            let finish = parts
                .iter()
                .find(|part| part["type"] == "finish")
                .expect("finish part");

            // Every usage field is undefined, so none is serialized.
            assert_eq!(
                finish["usage"],
                json!({ "inputTokens": {}, "outputTokens": {} })
            );
        }

        /// upstream: @ai-sdk/huggingface src/responses/huggingface-responses-language-model.test.ts › HuggingFaceResponsesLanguageModel › doStream › should handle streaming errors
        #[tokio::test]
        async fn should_handle_streaming_errors() {
            let fetch = sse_fetch(&[
                r#"data:{"type":"response.output_item.added","output_index":0,"item":{"id":"msg_test","type":"message","role":"assistant"},"sequence_number":1}"#,
                "data:invalid json}",
            ]);
            let result = provider(&fetch)
                .responses(MODEL)
                .do_stream(&call_options(test_prompt()))
                .await
                .expect("should succeed");

            let parts = stream_parts(result).await;
            let error = parts.iter().find(|part| part["type"] == "error");
            let finish = parts.iter().find(|part| part["type"] == "finish");

            assert!(error.is_some(), "expected an error part, got {parts:?}");
            assert_eq!(
                finish.expect("finish part")["finishReason"],
                json!({ "unified": "error" })
            );
        }

        /// upstream: @ai-sdk/huggingface src/responses/huggingface-responses-language-model.test.ts › HuggingFaceResponsesLanguageModel › doStream › should send correct streaming request
        #[tokio::test]
        async fn should_send_correct_streaming_request() {
            let fetch = sse_fetch(&[
                r#"data:{"type":"response.completed","response":{"id":"resp_test","status":"completed"},"sequence_number":1}"#,
            ]);
            let mut call = call_options(test_prompt());
            call.temperature = Some(0.7);

            provider(&fetch)
                .responses(MODEL)
                .do_stream(&call)
                .await
                .expect("should succeed");

            let seen = fetch.seen();
            assert_eq!(
                seen[0].json_body(),
                json!({
                    "model": "deepseek-ai/DeepSeek-V3-0324",
                    "temperature": 0.7,
                    "stream": true,
                    "input": [
                        {
                            "role": "user",
                            "content": [{ "type": "input_text", "text": "Hello" }]
                        }
                    ]
                })
            );
        }

        /// upstream: @ai-sdk/huggingface src/responses/huggingface-responses-language-model.test.ts › HuggingFaceResponsesLanguageModel › doStream › should handle non-message item types
        #[tokio::test]
        async fn should_handle_non_message_item_types() {
            let server = MockServer::start().await;
            let chunks = sse_body(&[
                sse_event(
                    r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"mcp_test","type":"mcp_list_tools","server_label":"test"},"sequence_number":1}"#,
                ),
                sse_event(
                    r#"{"type":"response.output_item.done","output_index":0,"item":{"id":"mcp_test","type":"mcp_list_tools","server_label":"test"},"sequence_number":2}"#,
                ),
                sse_event(
                    r#"{"type":"response.completed","response":{"id":"resp_test","status":"completed","incomplete_details":null},"sequence_number":3}"#,
                ),
            ]);
            mock_sse(&server, chunks).await;

            let provider = make_provider(&server);
            let model = provider.responses("deepseek-ai/DeepSeek-V3-0324");

            let result = model
                .do_stream(&default_options(test_prompt()))
                .await
                .expect("should succeed");

            let parts = collect_stream(result).await;

            // Should only have stream-start and finish events (no text events for
            // non-message items).
            let types: Vec<&str> = parts
                .iter()
                .map(|p| match p {
                    StreamPart::StreamStart { .. } => "stream-start",
                    StreamPart::Finish { .. } => "finish",
                    _ => "other",
                })
                .collect();
            assert_eq!(types, vec!["stream-start", "finish"]);
        }
    }

    mod message_conversion {
        use super::*;

        /// upstream: @ai-sdk/huggingface src/responses/huggingface-responses-language-model.test.ts › HuggingFaceResponsesLanguageModel › message conversion › should warn about tool messages
        #[tokio::test]
        async fn should_warn_about_tool_messages() {
            let fetch = json_fetch(&message_conversion_response());
            let prompt = vec![LanguageModelMessage::Tool {
                content: vec![ToolPart::ToolResult(ToolResultPart {
                    tool_call_id: "test".into(),
                    tool_name: "test".into(),
                    output: ToolResultOutput::Text {
                        value: "test".into(),
                        provider_options: None,
                    },
                    provider_options: None,
                })],
                provider_options: None,
            }];

            let result = provider(&fetch)
                .responses(MODEL)
                .do_generate(&call_options(prompt))
                .await
                .expect("should succeed");

            assert_eq!(
                serde_json::to_value(&result.warnings).unwrap(),
                json!([{ "feature": "tool messages", "type": "unsupported" }])
            );
        }

        /// upstream: @ai-sdk/huggingface src/responses/huggingface-responses-language-model.test.ts › HuggingFaceResponsesLanguageModel › message conversion › should convert user messages with images
        #[tokio::test]
        async fn should_convert_user_messages_with_images() {
            let server = MockServer::start().await;
            stub_empty_response(&server).await;

            let provider = make_provider(&server);
            let model = provider.responses("deepseek-ai/DeepSeek-V3-0324");

            let prompt = vec![LanguageModelMessage::User {
                content: vec![
                    UserPart::Text(TextPart {
                        text: "What do you see?".into(),
                        provider_options: None,
                    }),
                    UserPart::File(FilePart {
                        data: FileData::Data {
                            data: FileBytes::Base64("AQIDBA==".into()),
                        },
                        media_type: "image/jpeg".into(),
                        filename: None,
                        provider_options: None,
                    }),
                ],
                provider_options: None,
            }];

            let result = model
                .do_generate(&default_options(prompt))
                .await
                .expect("ok");
            let body = result
                .request
                .and_then(|request| request.body)
                .expect("body");

            assert_eq!(
                body["input"][0]["content"],
                json!([
                    { "text": "What do you see?", "type": "input_text" },
                    { "image_url": "data:image/jpeg;base64,AQIDBA==", "type": "input_image" }
                ])
            );
        }

        /// upstream: @ai-sdk/huggingface src/responses/huggingface-responses-language-model.test.ts › HuggingFaceResponsesLanguageModel › message conversion › should handle assistant messages
        #[tokio::test]
        async fn should_handle_assistant_messages() {
            let server = MockServer::start().await;
            stub_empty_response(&server).await;

            let provider = make_provider(&server);
            let model = provider.responses("deepseek-ai/DeepSeek-V3-0324");

            let prompt = vec![
                LanguageModelMessage::user_text("Hello"),
                LanguageModelMessage::Assistant {
                    content: vec![AssistantPart::Text(TextPart {
                        text: "Hi there!".into(),
                        provider_options: None,
                    })],
                    provider_options: None,
                },
                LanguageModelMessage::user_text("How are you?"),
            ];

            let result = model
                .do_generate(&default_options(prompt))
                .await
                .expect("ok");
            let body = result
                .request
                .and_then(|request| request.body)
                .expect("body");

            assert_eq!(
                body["input"],
                json!([
                    { "content": [{ "text": "Hello", "type": "input_text" }], "role": "user" },
                    { "content": [{ "text": "Hi there!", "type": "output_text" }], "role": "assistant" },
                    { "content": [{ "text": "How are you?", "type": "input_text" }], "role": "user" }
                ])
            );
        }

        /// Tool-call, tool-result, and reasoning in assistant messages produce NO
        /// warnings (they are silently skipped).
        /// upstream: @ai-sdk/huggingface src/responses/huggingface-responses-language-model.test.ts › HuggingFaceResponsesLanguageModel › message conversion › should warn about unsupported assistant content types
        #[tokio::test]
        async fn should_not_warn_about_assistant_content_types() {
            let server = MockServer::start().await;
            stub_empty_response(&server).await;

            let provider = make_provider(&server);
            let model = provider.responses("deepseek-ai/DeepSeek-V3-0324");

            let prompt = vec![LanguageModelMessage::Assistant {
                content: vec![
                    AssistantPart::ToolCall(ToolCallPart {
                        tool_call_id: "test".into(),
                        tool_name: "test".into(),
                        input: json!({}),
                        provider_executed: None,
                        provider_options: None,
                    }),
                    AssistantPart::ToolResult(ToolResultPart {
                        tool_call_id: "test".into(),
                        tool_name: "test".into(),
                        output: ToolResultOutput::Text {
                            value: "test".into(),
                            provider_options: None,
                        },

                        provider_options: None,
                    }),
                    AssistantPart::Reasoning(ReasoningPart {
                        text: "thinking...".into(),

                        provider_options: None,
                    }),
                ],
                provider_options: None,
            }];

            let result = model
                .do_generate(&default_options(prompt))
                .await
                .expect("ok");

            // TS expects no warnings (tool calls/results/reasoning are silently
            // skipped, reasoning is included as output_text).
            assert!(
                result.warnings.is_empty(),
                "expected no warnings, got: {:?}",
                result.warnings
            );
        }
    }

    mod tool_calls {
        use super::*;

        /// upstream: @ai-sdk/huggingface src/responses/huggingface-responses-language-model.test.ts › HuggingFaceResponsesLanguageModel › tool calls › should handle function_call tool responses
        #[tokio::test]
        async fn should_handle_function_call_tool_responses() {
            let fetch = json_fetch(&json!({
                "id": "resp_tool_test",
                "model": "deepseek-ai/DeepSeek-V3-0324",
                "object": "response",
                "created_at": 1741257730,
                "status": "completed",
                "error": null,
                "instructions": null,
                "max_output_tokens": null,
                "metadata": null,
                "tool_choice": "auto",
                "tools": [],
                "temperature": 1.0,
                "top_p": 1.0,
                "incomplete_details": null,
                "usage": { "input_tokens": 50, "output_tokens": 30, "total_tokens": 80 },
                "output": [
                    {
                        "id": "fc_test",
                        "type": "function_call",
                        "call_id": "call_123",
                        "name": "getWeather",
                        "arguments": "{\"location\": \"New York\"}",
                        "output": "{\"temperature\": \"72°F\", \"condition\": \"sunny\"}"
                    },
                    {
                        "id": "msg_after_tool",
                        "type": "message",
                        "role": "assistant",
                        "status": "completed",
                        "content": [
                            {
                                "type": "output_text",
                                "text": "The weather in New York is 72°F and sunny."
                            }
                        ]
                    }
                ],
                "output_text": null
            }));
            let result = provider(&fetch)
                .responses(MODEL)
                .do_generate(&call_options(test_prompt()))
                .await
                .expect("should succeed");

            assert_eq!(
                serde_json::to_value(&result.content).unwrap(),
                json!([
                    {
                        "input": "{\"location\": \"New York\"}",
                        "toolCallId": "call_123",
                        "toolName": "getWeather",
                        "type": "tool-call"
                    },
                    {
                        "result": "{\"temperature\": \"72°F\", \"condition\": \"sunny\"}",
                        "toolCallId": "call_123",
                        "toolName": "getWeather",
                        "type": "tool-result"
                    },
                    {
                        "providerMetadata": {
                            "huggingface": { "itemId": "msg_after_tool" }
                        },
                        "text": "The weather in New York is 72°F and sunny.",
                        "type": "text"
                    }
                ])
            );
        }

        /// upstream: @ai-sdk/huggingface src/responses/huggingface-responses-language-model.test.ts › HuggingFaceResponsesLanguageModel › tool calls › should stream tool calls
        #[tokio::test]
        async fn should_stream_tool_calls() {
            let fetch = sse_fetch(&[
                r#"data:{"type":"response.created","response":{"id":"resp_tool_stream","object":"response","created_at":1741269019,"status":"in_progress","model":"deepseek-ai/DeepSeek-V3-0324"}}"#,
                r#"data:{"type":"response.output_item.added","output_index":0,"item":{"id":"fc_stream","type":"function_call","call_id":"call_456","name":"calculator","arguments":""},"sequence_number":1}"#,
                r#"data:{"type":"response.function_call_arguments.delta","item_id":"fc_stream","output_index":0,"delta":"{\"operation\"","sequence_number":2}"#,
                r#"data:{"type":"response.function_call_arguments.delta","item_id":"fc_stream","output_index":0,"delta":": \"add\", \"a\": 5, \"b\": 3}","sequence_number":3}"#,
                r#"data:{"type":"response.output_item.done","output_index":0,"item":{"id":"fc_stream","type":"function_call","call_id":"call_456","name":"calculator","arguments":"{\"operation\": \"add\", \"a\": 5, \"b\": 3}","output":"8"},"sequence_number":4}"#,
                r#"data:{"type":"response.completed","response":{"id":"resp_tool_stream","status":"completed","usage":{"input_tokens":20,"output_tokens":15,"total_tokens":35}},"sequence_number":5}"#,
            ]);
            let result = provider(&fetch)
                .responses(MODEL)
                .do_stream(&call_options(test_prompt()))
                .await
                .expect("should succeed");

            assert_eq!(
                Value::Array(stream_parts(result).await),
                json!([
                    { "type": "stream-start", "warnings": [] },
                    {
                        "id": "resp_tool_stream",
                        "modelId": "deepseek-ai/DeepSeek-V3-0324",
                        "timestamp": "2025-03-06T13:50:19.000Z",
                        "type": "response-metadata"
                    },
                    {
                        "id": "call_456",
                        "toolName": "calculator",
                        "type": "tool-input-start"
                    },
                    { "id": "call_456", "type": "tool-input-end" },
                    {
                        "input": "{\"operation\": \"add\", \"a\": 5, \"b\": 3}",
                        "toolCallId": "call_456",
                        "toolName": "calculator",
                        "type": "tool-call"
                    },
                    {
                        "result": "8",
                        "toolCallId": "call_456",
                        "toolName": "calculator",
                        "type": "tool-result"
                    },
                    {
                        "finishReason": { "unified": "stop" },
                        "providerMetadata": {
                            "huggingface": { "responseId": "resp_tool_stream" }
                        },
                        "type": "finish",
                        "usage": {
                            "inputTokens": { "cacheRead": 0, "noCache": 20, "total": 20 },
                            "outputTokens": { "reasoning": 0, "text": 15, "total": 15 },
                            "raw": { "input_tokens": 20, "output_tokens": 15, "total_tokens": 35 }
                        }
                    }
                ])
            );
        }
    }

    mod structured_output {
        use super::*;

        /// upstream: @ai-sdk/huggingface src/responses/huggingface-responses-language-model.test.ts › HuggingFaceResponsesLanguageModel › structured output › should send text.format for structured output
        #[tokio::test]
        async fn should_send_text_format_for_structured_output() {
            let fetch = json_fetch(&json!({
                "id": "resp_structured",
                "model": "moonshotai/Kimi-K2-Instruct",
                "object": "response",
                "created_at": 1741257730,
                "status": "completed",
                "error": null,
                "instructions": null,
                "max_output_tokens": null,
                "metadata": null,
                "tool_choice": "auto",
                "tools": [],
                "temperature": 1.0,
                "top_p": 1.0,
                "incomplete_details": null,
                "usage": null,
                "output": [
                    {
                        "id": "msg_structured",
                        "type": "message",
                        "role": "assistant",
                        "status": "completed",
                        "content": [
                            { "type": "output_text", "text": "{\"name\": \"John Doe\", \"age\": 30}" }
                        ]
                    }
                ],
                "output_text": null
            }));
            let mut call = call_options(test_prompt());
            call.response_format = Some(ResponseFormat::Json {
                schema: Some(json!({
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" },
                        "age": { "type": "number" }
                    },
                    "required": ["name", "age"]
                })),
                name: None,
                description: None,
            });

            provider(&fetch)
                .responses("moonshotai/Kimi-K2-Instruct")
                .do_generate(&call)
                .await
                .expect("should succeed");

            let seen = fetch.seen();
            assert_eq!(
                seen[0].json_body()["text"],
                json!({
                    "format": {
                        "name": "response",
                        "schema": {
                            "properties": {
                                "age": { "type": "number" },
                                "name": { "type": "string" }
                            },
                            "required": ["name", "age"],
                            "type": "object"
                        },
                        "strict": false,
                        "type": "json_schema"
                    }
                })
            );
        }

        /// upstream: @ai-sdk/huggingface src/responses/huggingface-responses-language-model.test.ts › HuggingFaceResponsesLanguageModel › structured output › should handle structured output with custom name and description
        #[tokio::test]
        async fn should_handle_structured_output_with_custom_name_and_description() {
            let server = MockServer::start().await;
            mock_json(
                &server,
                json!({
                    "id": "resp_structured",
                    "model": "moonshotai/Kimi-K2-Instruct",
                    "object": "response",
                    "created_at": 1741257730,
                    "status": "completed",
                    "error": null,
                    "instructions": null,
                    "max_output_tokens": null,
                    "metadata": null,
                    "tool_choice": "auto",
                    "tools": [],
                    "temperature": 1.0,
                    "top_p": 1.0,
                    "incomplete_details": null,
                    "usage": null,
                    "output": [],
                    "output_text": "{}"
                }),
            )
            .await;

            let provider = make_provider(&server);
            let model = provider.responses("moonshotai/Kimi-K2-Instruct");

            let mut options = default_options(test_prompt());
            options.response_format = Some(ResponseFormat::Json {
                schema: Some(
                    json!({ "type": "object", "properties": { "name": { "type": "string" } } }),
                ),
                name: Some("person_profile".to_string()),
                description: Some("A person profile with basic information".to_string()),
            });

            let result = model.do_generate(&options).await.expect("should succeed");
            let body = result
                .request
                .and_then(|request| request.body)
                .expect("body");

            assert_eq!(body["text"]["format"]["name"], json!("person_profile"));
            assert_eq!(
                body["text"]["format"]["description"],
                json!("A person profile with basic information")
            );
        }
    }

    mod reasoning {
        use super::*;

        /// upstream: @ai-sdk/huggingface src/responses/huggingface-responses-language-model.test.ts › HuggingFaceResponsesLanguageModel › reasoning › should handle reasoning content in responses
        #[tokio::test]
        async fn should_handle_reasoning_content_in_responses() {
            let fetch = json_fetch(&json!({
                "id": "resp_reasoning",
                "model": "deepseek-ai/DeepSeek-R1",
                "object": "response",
                "created_at": 1741257730,
                "status": "completed",
                "error": null,
                "instructions": null,
                "max_output_tokens": null,
                "metadata": null,
                "tool_choice": "auto",
                "tools": [],
                "temperature": 1.0,
                "top_p": 1.0,
                "incomplete_details": null,
                "usage": { "input_tokens": 10, "output_tokens": 50, "total_tokens": 60 },
                "output": [
                    {
                        "id": "reasoning_1",
                        "type": "reasoning",
                        "content": [
                            {
                                "type": "reasoning_text",
                                "text": "Let me think about this problem step by step..."
                            }
                        ]
                    },
                    {
                        "id": "msg_after_reasoning",
                        "type": "message",
                        "role": "assistant",
                        "status": "completed",
                        "content": [{ "type": "output_text", "text": "The answer is 42." }]
                    }
                ],
                "output_text": null
            }));
            let result = provider(&fetch)
                .responses("deepseek-ai/DeepSeek-R1")
                .do_generate(&call_options(test_prompt()))
                .await
                .expect("should succeed");

            assert_eq!(
                serde_json::to_value(&result.content).unwrap(),
                json!([
                    {
                        "providerMetadata": { "huggingface": { "itemId": "reasoning_1" } },
                        "text": "Let me think about this problem step by step...",
                        "type": "reasoning"
                    },
                    {
                        "providerMetadata": {
                            "huggingface": { "itemId": "msg_after_reasoning" }
                        },
                        "text": "The answer is 42.",
                        "type": "text"
                    }
                ])
            );
        }

        /// upstream: @ai-sdk/huggingface src/responses/huggingface-responses-language-model.test.ts › HuggingFaceResponsesLanguageModel › reasoning › should stream reasoning content
        #[tokio::test]
        async fn should_stream_reasoning_content() {
            let fetch = sse_fetch(&[
                r#"data:{"type":"response.created","response":{"id":"resp_reasoning_stream","object":"response","created_at":1741269019,"status":"in_progress","model":"deepseek-ai/DeepSeek-R1"}}"#,
                r#"data:{"type":"response.output_item.added","output_index":0,"item":{"id":"reasoning_stream","type":"reasoning"},"sequence_number":1}"#,
                r#"data:{"type":"response.reasoning_text.delta","item_id":"reasoning_stream","output_index":0,"content_index":0,"delta":"Thinking about","sequence_number":2}"#,
                r#"data:{"type":"response.reasoning_text.delta","item_id":"reasoning_stream","output_index":0,"content_index":0,"delta":" the problem...","sequence_number":3}"#,
                r#"data:{"type":"response.reasoning_text.done","item_id":"reasoning_stream","output_index":0,"content_index":0,"text":"Thinking about the problem...","sequence_number":4}"#,
                r#"data:{"type":"response.output_item.done","output_index":0,"item":{"id":"reasoning_stream","type":"reasoning","content":[{"type":"reasoning_text","text":"Thinking about the problem..."}]},"sequence_number":5}"#,
                r#"data:{"type":"response.output_item.added","output_index":1,"item":{"id":"msg_stream","type":"message","role":"assistant"},"sequence_number":6}"#,
                r#"data:{"type":"response.output_text.delta","item_id":"msg_stream","output_index":1,"content_index":0,"delta":"The solution is","sequence_number":7}"#,
                r#"data:{"type":"response.output_text.delta","item_id":"msg_stream","output_index":1,"content_index":0,"delta":" simple.","sequence_number":8}"#,
                r#"data:{"type":"response.output_item.done","output_index":1,"item":{"id":"msg_stream","type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":"The solution is simple."}]},"sequence_number":9}"#,
                r#"data:{"type":"response.completed","response":{"id":"resp_reasoning_stream","status":"completed","usage":{"input_tokens":10,"output_tokens":20,"total_tokens":30}},"sequence_number":10}"#,
            ]);
            let result = provider(&fetch)
                .responses("deepseek-ai/DeepSeek-R1")
                .do_stream(&call_options(test_prompt()))
                .await
                .expect("should succeed");

            assert_eq!(
                Value::Array(stream_parts(result).await),
                json!([
                    { "type": "stream-start", "warnings": [] },
                    {
                        "id": "resp_reasoning_stream",
                        "modelId": "deepseek-ai/DeepSeek-R1",
                        "timestamp": "2025-03-06T13:50:19.000Z",
                        "type": "response-metadata"
                    },
                    {
                        "id": "reasoning_stream",
                        "providerMetadata": {
                            "huggingface": { "itemId": "reasoning_stream" }
                        },
                        "type": "reasoning-start"
                    },
                    {
                        "delta": "Thinking about",
                        "id": "reasoning_stream",
                        "type": "reasoning-delta"
                    },
                    {
                        "delta": " the problem...",
                        "id": "reasoning_stream",
                        "type": "reasoning-delta"
                    },
                    { "id": "reasoning_stream", "type": "reasoning-end" },
                    {
                        "id": "msg_stream",
                        "providerMetadata": { "huggingface": { "itemId": "msg_stream" } },
                        "type": "text-start"
                    },
                    { "delta": "The solution is", "id": "msg_stream", "type": "text-delta" },
                    { "delta": " simple.", "id": "msg_stream", "type": "text-delta" },
                    { "id": "msg_stream", "type": "text-end" },
                    {
                        "finishReason": { "unified": "stop" },
                        "providerMetadata": {
                            "huggingface": { "responseId": "resp_reasoning_stream" }
                        },
                        "type": "finish",
                        "usage": {
                            "inputTokens": { "cacheRead": 0, "noCache": 10, "total": 10 },
                            "outputTokens": { "reasoning": 0, "text": 20, "total": 20 },
                            "raw": { "input_tokens": 10, "output_tokens": 20, "total_tokens": 30 }
                        }
                    }
                ])
            );
        }
    }

    mod provider_options {
        use super::*;

        /// upstream: @ai-sdk/huggingface src/responses/huggingface-responses-language-model.test.ts › HuggingFaceResponsesLanguageModel › provider options › should send provider-specific options
        #[tokio::test]
        async fn should_send_provider_specific_options() {
            let server = MockServer::start().await;
            mock_json(
                &server,
                json!({
                    "id": "resp_provider_options",
                    "model": "deepseek-ai/DeepSeek-V3-0324",
                    "object": "response",
                    "created_at": 1741257730,
                    "status": "completed",
                    "error": null,
                    "instructions": null,
                    "max_output_tokens": null,
                    "metadata": null,
                    "tool_choice": "auto",
                    "tools": [],
                    "temperature": 1.0,
                    "top_p": 1.0,
                    "incomplete_details": null,
                    "usage": null,
                    "output": [],
                    "output_text": "Test"
                }),
            )
            .await;

            let provider = make_provider(&server);
            let model = provider.responses("deepseek-ai/DeepSeek-V3-0324");

            let mut options = default_options(test_prompt());
            let mut po = aimux_core::shared::SharedProviderOptions::new();
            po.insert(
                "huggingface".to_string(),
                serde_json::from_value(json!({
                    "metadata": { "key": "value" },
                    "instructions": "Be concise",
                    "strictJsonSchema": true
                }))
                .unwrap(),
            );
            options.provider_options = Some(po);
            options.response_format = Some(ResponseFormat::Json {
                schema: Some(json!({ "type": "object" })),
                name: None,
                description: None,
            });

            let result = model.do_generate(&options).await.expect("should succeed");
            let body = result
                .request
                .and_then(|request| request.body)
                .expect("body");

            assert_eq!(body["metadata"], json!({ "key": "value" }));
            assert_eq!(body["instructions"], json!("Be concise"));
            assert_eq!(body["text"]["format"]["strict"], json!(true));
        }
    }

    mod tool_preparation {
        use super::*;

        /// upstream: @ai-sdk/huggingface src/responses/huggingface-responses-language-model.test.ts › HuggingFaceResponsesLanguageModel › tool preparation › should prepare tools correctly
        #[tokio::test]
        async fn should_prepare_tools_correctly() {
            let server = MockServer::start().await;
            mock_json(
                &server,
                json!({
                    "id": "resp_tools",
                    "model": "deepseek-ai/DeepSeek-V3-0324",
                    "object": "response",
                    "created_at": 1741257730,
                    "status": "completed",
                    "error": null,
                    "instructions": null,
                    "max_output_tokens": null,
                    "metadata": null,
                    "tool_choice": "auto",
                    "tools": [],
                    "temperature": 1.0,
                    "top_p": 1.0,
                    "incomplete_details": null,
                    "usage": null,
                    "output": [],
                    "output_text": "Test"
                }),
            )
            .await;

            let provider = make_provider(&server);
            let model = provider.responses("deepseek-ai/DeepSeek-V3-0324");

            let mut options = default_options(test_prompt());
            options.tools = Some(vec![Tool::from(
                FunctionTool::new(
                    "getWeather",
                    json!({
                        "type": "object",
                        "properties": { "location": { "type": "string" } },
                        "required": ["location"]
                    }),
                )
                .with_description("Get weather information"),
            )]);
            options.tool_choice = Some(LanguageModelV4ToolChoice::Tool {
                tool_name: "getWeather".to_string(),
            });

            let result = model.do_generate(&options).await.expect("should succeed");
            let body = result
                .request
                .and_then(|request| request.body)
                .expect("body");

            assert_eq!(
                body["tools"],
                json!([{
                    "type": "function",
                    "name": "getWeather",
                    "description": "Get weather information",
                    "parameters": {
                        "type": "object",
                        "properties": { "location": { "type": "string" } },
                        "required": ["location"]
                    }
                }])
            );
            assert_eq!(
                body["tool_choice"],
                json!({ "type": "function", "function": { "name": "getWeather" } })
            );
        }

        /// upstream: @ai-sdk/huggingface src/responses/huggingface-responses-language-model.test.ts › HuggingFaceResponsesLanguageModel › tool preparation › should handle auto and required tool choices
        #[tokio::test]
        async fn should_handle_auto_and_required_tool_choices() {
            let server = MockServer::start().await;
            mock_json(
                &server,
                json!({
                    "id": "resp_tools",
                    "model": "deepseek-ai/DeepSeek-V3-0324",
                    "object": "response",
                    "created_at": 1741257730,
                    "status": "completed",
                    "error": null,
                    "instructions": null,
                    "max_output_tokens": null,
                    "metadata": null,
                    "tool_choice": "auto",
                    "tools": [],
                    "temperature": 1.0,
                    "top_p": 1.0,
                    "incomplete_details": null,
                    "usage": null,
                    "output": [],
                    "output_text": "Test"
                }),
            )
            .await;

            let provider = make_provider(&server);
            let model = provider.responses("deepseek-ai/DeepSeek-V3-0324");

            // Test auto
            let mut options = default_options(test_prompt());
            options.tools = Some(vec![Tool::from(FunctionTool::new(
                "test",
                json!({ "type": "object" }),
            ))]);
            options.tool_choice = Some(LanguageModelV4ToolChoice::Auto);

            let result = model.do_generate(&options).await.expect("should succeed");
            let body = result
                .request
                .and_then(|request| request.body)
                .expect("body");
            assert_eq!(body["tool_choice"], json!("auto"));

            // Test required
            let mut options = default_options(test_prompt());
            options.tools = Some(vec![Tool::from(FunctionTool::new(
                "test",
                json!({ "type": "object" }),
            ))]);
            options.tool_choice = Some(LanguageModelV4ToolChoice::Required);

            let result = model.do_generate(&options).await.expect("should succeed");
            let body = result
                .request
                .and_then(|request| request.body)
                .expect("body");
            assert_eq!(body["tool_choice"], json!("required"));
        }
    }

    mod top_level_only_media_type_resolution {
        use super::*;

        /// upstream: @ai-sdk/huggingface src/responses/huggingface-responses-language-model.test.ts › HuggingFaceResponsesLanguageModel › top-level-only media type resolution › passes full image/png through unchanged for inline data
        #[test]
        fn passes_full_image_png_through_unchanged_for_inline_data() {
            let prompt = vec![LanguageModelMessage::User {
                content: vec![UserPart::File(FilePart {
                    data: FileData::Data {
                        data: FileBytes::Base64(PNG_BASE64.into()),
                    },
                    media_type: "image/png".into(),
                    filename: None,
                    provider_options: None,
                })],
                provider_options: None,
            }];

            let (input, warnings) =
                convert_to_huggingface_responses_messages(&prompt).expect("should succeed");

            assert!(warnings.is_empty());
            let content = &input[0]["content"][0];
            assert_eq!(
                content,
                &json!({
                    "type": "input_image",
                    "image_url": format!("data:image/png;base64,{}", PNG_BASE64)
                })
            );
        }

        /// upstream: @ai-sdk/huggingface src/responses/huggingface-responses-language-model.test.ts › HuggingFaceResponsesLanguageModel › top-level-only media type resolution › detects image subtype from inline bytes for top-level "image"
        #[test]
        fn detects_image_subtype_from_inline_bytes_for_top_level_image() {
            let prompt = vec![LanguageModelMessage::User {
                content: vec![UserPart::File(FilePart {
                    data: FileData::Data {
                        data: FileBytes::Base64(PNG_BASE64.into()),
                    },
                    media_type: "image".into(),
                    filename: None,
                    provider_options: None,
                })],
                provider_options: None,
            }];

            let (input, warnings) =
                convert_to_huggingface_responses_messages(&prompt).expect("should succeed");

            assert!(warnings.is_empty());
            let content = &input[0]["content"][0];
            assert_eq!(
                content,
                &json!({
                    "type": "input_image",
                    "image_url": format!("data:image/png;base64,{}", PNG_BASE64)
                })
            );
        }

        /// upstream: @ai-sdk/huggingface src/responses/huggingface-responses-language-model.test.ts › HuggingFaceResponsesLanguageModel › top-level-only media type resolution › passes through URL source for top-level-only image
        #[test]
        fn passes_through_url_source_for_top_level_only_image() {
            let prompt = vec![LanguageModelMessage::User {
                content: vec![UserPart::File(FilePart {
                    data: FileData::Url {
                        original_url: None,
                        url: "https://example.com/x.png".into(),
                    },
                    media_type: "image".into(),
                    filename: None,
                    provider_options: None,
                })],
                provider_options: None,
            }];

            let (input, warnings) =
                convert_to_huggingface_responses_messages(&prompt).expect("should succeed");

            assert!(warnings.is_empty());
            let content = &input[0]["content"][0];
            assert_eq!(
                content,
                &json!({
                    "type": "input_image",
                    "image_url": "https://example.com/x.png"
                })
            );
        }

        /// upstream: @ai-sdk/huggingface src/responses/huggingface-responses-language-model.test.ts › HuggingFaceResponsesLanguageModel › top-level-only media type resolution › normalizes image/* wildcard via detection
        #[test]
        fn normalizes_image_wildcard_via_detection() {
            let prompt = vec![LanguageModelMessage::User {
                content: vec![UserPart::File(FilePart {
                    data: FileData::Data {
                        data: FileBytes::Base64(PNG_BASE64.into()),
                    },
                    media_type: "image/*".into(),
                    filename: None,
                    provider_options: None,
                })],
                provider_options: None,
            }];

            let (input, warnings) =
                convert_to_huggingface_responses_messages(&prompt).expect("should succeed");

            assert!(warnings.is_empty());
            let content = &input[0]["content"][0];
            assert_eq!(
                content,
                &json!({
                    "type": "input_image",
                    "image_url": format!("data:image/png;base64,{}", PNG_BASE64)
                })
            );
        }
    }
}

/// Partial port of upstream `should generate text`: checks the text but not
/// the providerMetadata itemId. Full case: `hugging_face_responses_language_model` above.
#[tokio::test]
async fn should_generate_text() {
    let server = MockServer::start().await;
    mock_json(&server, basic_response_body()).await;

    let provider = make_provider(&server);
    let model = provider.responses("deepseek-ai/DeepSeek-V3-0324");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    assert_eq!(result.content.len(), 1);
    match &result.content[0] {
        GenerateContent::Text { text, .. } => {
            assert_eq!(text, "Hello! How can I help you today?");
        }
        other => panic!("expected Text, got {other:?}"),
    }
}

/// Partial port of upstream `should extract usage`: checks the token counts
/// but not the raw usage object. Full case: `hugging_face_responses_language_model` above.
#[tokio::test]
async fn should_extract_usage() {
    let server = MockServer::start().await;
    mock_json(&server, basic_response_body()).await;

    let provider = make_provider(&server);
    let model = provider.responses("deepseek-ai/DeepSeek-V3-0324");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    // usage: input_tokens=12, output_tokens=25, no cache/reasoning details
    assert_eq!(result.usage.input_tokens.total, Some(12));
    assert_eq!(result.usage.input_tokens.no_cache, Some(12));
    assert_eq!(result.usage.input_tokens.cache_read, Some(0));
    assert_eq!(result.usage.input_tokens.cache_write, None);
    assert_eq!(result.usage.output_tokens.total, Some(25));
    assert_eq!(result.usage.output_tokens.text, Some(25));
    assert_eq!(result.usage.output_tokens.reasoning, Some(0));
}

/// Partial port of upstream `should extract text from output array when
/// output_text is missing`: checks the text but not the providerMetadata itemId.
#[tokio::test]
async fn should_extract_text_from_output_array_when_output_text_missing() {
    let server = MockServer::start().await;
    mock_json(
        &server,
        json!({
            "id": "resp_test",
            "model": "deepseek-ai/DeepSeek-V3-0324",
            "object": "response",
            "created_at": 1741257730,
            "status": "completed",
            "error": null,
            "instructions": null,
            "max_output_tokens": null,
            "metadata": null,
            "tool_choice": "auto",
            "tools": [],
            "temperature": 1.0,
            "top_p": 1.0,
            "incomplete_details": null,
            "usage": null,
            "output": [
                {
                    "id": "msg_test",
                    "type": "message",
                    "role": "assistant",
                    "status": "completed",
                    "content": [
                        { "type": "output_text", "text": "Extracted from output array" }
                    ]
                }
            ],
            "output_text": null
        }),
    )
    .await;

    let provider = make_provider(&server);
    let model = provider.responses("deepseek-ai/DeepSeek-V3-0324");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    assert_eq!(result.content.len(), 1);
    match &result.content[0] {
        GenerateContent::Text { text, .. } => {
            assert_eq!(text, "Extracted from output array");
        }
        other => panic!("expected Text, got {other:?}"),
    }
}

/// Partial port of upstream `should handle missing usage gracefully`: checks
/// the token counts but not the raw usage (undefined upstream).
#[tokio::test]
async fn should_handle_missing_usage_gracefully() {
    let server = MockServer::start().await;
    mock_json(
        &server,
        json!({
            "id": "resp_test",
            "model": "deepseek-ai/DeepSeek-V3-0324",
            "object": "response",
            "created_at": 1741257730,
            "status": "completed",
            "error": null,
            "instructions": null,
            "max_output_tokens": null,
            "metadata": null,
            "tool_choice": "auto",
            "tools": [],
            "temperature": 1.0,
            "top_p": 1.0,
            "incomplete_details": null,
            "usage": null,
            "output": [],
            "output_text": "Test response"
        }),
    )
    .await;

    let provider = make_provider(&server);
    let model = provider.responses("deepseek-ai/DeepSeek-V3-0324");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    // All usage fields should be None.
    assert_eq!(result.usage.input_tokens.total, None);
    assert_eq!(result.usage.input_tokens.no_cache, None);
    assert_eq!(result.usage.input_tokens.cache_read, None);
    assert_eq!(result.usage.input_tokens.cache_write, None);
    assert_eq!(result.usage.output_tokens.total, None);
    assert_eq!(result.usage.output_tokens.text, None);
    assert_eq!(result.usage.output_tokens.reasoning, None);
}

/// Partial port of upstream `should send model id, settings, and input`:
/// checks each body field, not the whole body (upstream uses toStrictEqual).
#[tokio::test]
async fn should_send_model_id_settings_and_input() {
    let server = MockServer::start().await;
    mock_json(&server, basic_response_body()).await;

    let provider = make_provider(&server);
    let model = provider.responses("deepseek-ai/DeepSeek-V3-0324");

    let prompt = vec![
        LanguageModelMessage::System {
            content: "You are a helpful assistant.".into(),
            provider_options: None,
        },
        LanguageModelMessage::user_text("Hello"),
    ];
    let mut options = default_options(prompt);
    options.temperature = Some(0.5);
    options.top_p = Some(0.3);
    options.max_output_tokens = Some(100);

    let result = model.do_generate(&options).await.expect("should succeed");
    let body = result
        .request
        .and_then(|request| request.body)
        .expect("body");

    assert_eq!(body["model"], json!("deepseek-ai/DeepSeek-V3-0324"));
    assert_eq!(body["temperature"], json!(0.5));
    assert_eq!(body["top_p"], json!(0.3));
    assert_eq!(body["max_output_tokens"], json!(100));
    assert_eq!(body["stream"], json!(false));
    assert_eq!(
        body["input"],
        json!([
            { "role": "system", "content": "You are a helpful assistant." },
            { "role": "user", "content": [{ "type": "input_text", "text": "Hello" }] }
        ])
    );
}

/// Partial port of upstream `should handle unsupported settings with warnings`:
/// compares only the feature names of Unsupported warnings, not the array.
#[tokio::test]
async fn should_handle_unsupported_settings_with_warnings() {
    let server = MockServer::start().await;
    mock_json(&server, basic_response_body()).await;

    let provider = make_provider(&server);
    let model = provider.responses("deepseek-ai/DeepSeek-V3-0324");

    let mut options = default_options(test_prompt());
    options.top_k = Some(10.0);
    options.seed = Some(123);
    options.presence_penalty = Some(0.5);
    options.frequency_penalty = Some(0.3);
    options.stop_sequences = Some(vec!["stop".to_string()]);

    let result = model.do_generate(&options).await.expect("should succeed");

    let features: Vec<String> = result
        .warnings
        .iter()
        .filter_map(|w| match w {
            aimux_core::types::Warning::Unsupported { feature, .. } => Some(feature.clone()),
            _ => None,
        })
        .collect();

    assert_eq!(
        features,
        vec![
            "topK",
            "seed",
            "presencePenalty",
            "frequencyPenalty",
            "stopSequences"
        ]
    );
}

/// Different input from upstream `should generate text and sources from
/// annotations` (shorter text, no markdown links). Upstream case: src/huggingface/tests/responses/responses_language_model.rs.
#[tokio::test]
async fn should_generate_text_and_sources_from_annotations() {
    let server = MockServer::start().await;
    mock_json(
        &server,
        json!({
            "id": "resp_test_annotations",
            "model": "deepseek-ai/DeepSeek-V3-0324",
            "object": "response",
            "created_at": 1741257730,
            "status": "completed",
            "error": null,
            "instructions": null,
            "max_output_tokens": null,
            "metadata": null,
            "tool_choice": "auto",
            "tools": [],
            "temperature": 1.0,
            "top_p": 1.0,
            "incomplete_details": null,
            "usage": { "input_tokens": 20, "output_tokens": 50, "total_tokens": 70 },
            "output": [
                {
                    "id": "msg_test_annotations",
                    "type": "message",
                    "role": "assistant",
                    "status": "completed",
                    "content": [
                        {
                            "type": "output_text",
                            "text": "Here are some recent articles about AI.",
                            "annotations": [
                                { "type": "url_citation", "url": "https://example.com/article1", "title": "AI Developments Article" },
                                { "type": "url_citation", "url": "https://test.com/article2", "title": "Industry Trends Report" }
                            ]
                        }
                    ]
                }
            ],
            "output_text": null
        }),
    )
    .await;

    let provider = make_provider(&server);
    let model = provider.responses("deepseek-ai/DeepSeek-V3-0324");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    // content: text + 2 sources
    assert_eq!(result.content.len(), 3);
    match &result.content[0] {
        GenerateContent::Text { text, .. } => {
            assert_eq!(text, "Here are some recent articles about AI.");
        }
        other => panic!("expected Text at [0], got {other:?}"),
    }
    match &result.content[1] {
        GenerateContent::Source(Source::Url { id, url, title, .. }) => {
            assert_eq!(id, "id-0");
            assert_eq!(url, "https://example.com/article1");
            assert_eq!(title.as_deref(), Some("AI Developments Article"));
        }
        other => panic!("expected Source at [1], got {other:?}"),
    }
    match &result.content[2] {
        GenerateContent::Source(Source::Url { id, url, title, .. }) => {
            assert_eq!(id, "id-1");
            assert_eq!(url, "https://test.com/article2");
            assert_eq!(title.as_deref(), Some("Industry Trends Report"));
        }
        other => panic!("expected Source at [2], got {other:?}"),
    }
}

/// Different input from upstream `should handle MCP tools with annotations`
/// (shorter text). Upstream case: src/huggingface/tests/responses/responses_language_model.rs.
#[tokio::test]
async fn should_handle_mcp_tools_with_annotations() {
    let server = MockServer::start().await;
    mock_json(
        &server,
        json!({
            "id": "resp_mcp_test",
            "model": "deepseek-ai/DeepSeek-V3-0324",
            "object": "response",
            "created_at": 1741257730,
            "status": "completed",
            "error": null,
            "instructions": null,
            "max_output_tokens": null,
            "metadata": null,
            "tool_choice": "auto",
            "tools": [],
            "temperature": 1.0,
            "top_p": 1.0,
            "incomplete_details": null,
            "usage": { "input_tokens": 50, "output_tokens": 100, "total_tokens": 150 },
            "output": [
                {
                    "id": "mcp_search_test",
                    "type": "mcp_call",
                    "server_label": "web_search",
                    "name": "search",
                    "arguments": "{\"query\": \"San Francisco tech events\"}",
                    "output": "Found 25 tech events in San Francisco"
                },
                {
                    "id": "msg_mcp_response",
                    "type": "message",
                    "role": "assistant",
                    "status": "completed",
                    "content": [
                        {
                            "type": "output_text",
                            "text": "Based on the search results.",
                            "annotations": [
                                { "type": "url_citation", "url": "https://techevents.com/sf-ai", "title": "SF AI Conference 2025" },
                                { "type": "url_citation", "url": "https://eventbrite.com/sf-startups", "title": "SF Startup Meetups" }
                            ]
                        }
                    ]
                }
            ],
            "output_text": null
        }),
    )
    .await;

    let provider = make_provider(&server);
    let model = provider.responses("deepseek-ai/DeepSeek-V3-0324");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    // content: tool-call, tool-result, text, source, source — the `mcp_call`
    // carries an `output`, which is surfaced as the paired ToolResult (the TS
    // emits it as a separate content item too).
    assert_eq!(result.content.len(), 5);
    match &result.content[0] {
        GenerateContent::ToolCall(RawToolCall {
            tool_call_id,
            tool_name,
            input,
            ..
        }) => {
            assert_eq!(tool_call_id, "mcp_search_test");
            assert_eq!(tool_name, "search");
            assert_eq!(
                input,
                &Value::String(r#"{"query": "San Francisco tech events"}"#.into())
            );
        }
        other => panic!("expected ToolCall at [0], got {other:?}"),
    }
    match &result.content[1] {
        GenerateContent::ToolResult(ToolResult {
            tool_call_id,
            tool_name,
            ..
        }) => {
            assert_eq!(tool_call_id, "mcp_search_test");
            assert_eq!(tool_name, "search");
        }
        other => panic!("expected ToolResult at [1], got {other:?}"),
    }
    match &result.content[2] {
        GenerateContent::Text { text, .. } => assert_eq!(text, "Based on the search results."),
        other => panic!("expected Text at [2], got {other:?}"),
    }
    match &result.content[3] {
        GenerateContent::Source(Source::Url { id, .. }) => assert_eq!(id, "id-0"),
        other => panic!("expected Source at [3], got {other:?}"),
    }
    match &result.content[4] {
        GenerateContent::Source(Source::Url { id, .. }) => assert_eq!(id, "id-1"),
        other => panic!("expected Source at [4], got {other:?}"),
    }

    // Like upstream, the MCP call is not dynamic: without a local `search`
    // tool, Core returns it marked invalid (parseToolCall's NoSuchToolError).
    let result = generate_text(&model, "Hello", GenerateTextOptions::default())
        .await
        .expect("an unknown tool is an invalid call, not an error");
    let call = *result.tool_calls().first().expect("MCP tool call");
    assert_eq!(call.tool_name, "search");
    assert_eq!(call.provider_executed, Some(true));
    assert_eq!(call.dynamic, Some(true));
    assert_eq!(call.invalid, Some(true));
    assert_eq!(call.input, json!({ "query": "San Francisco tech events" }));
}

/// Partial port of upstream `should stream text deltas`: checks part kinds,
/// ids, deltas and token counts, not the timestamp, metadata or raw usage.
#[tokio::test]
async fn should_stream_text_deltas() {
    let server = MockServer::start().await;
    let chunks = sse_body(&[
        sse_event(
            r#"{"type":"response.created","response":{"id":"resp_test","object":"response","created_at":1741269019,"status":"in_progress","model":"deepseek-ai/DeepSeek-V3-0324"}}"#,
        ),
        sse_event(
            r#"{"type":"response.in_progress","response":{"id":"resp_test","object":"response","created_at":1741269019,"status":"in_progress"}}"#,
        ),
        sse_event(
            r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"msg_test","type":"message","role":"assistant","status":"in_progress","content":[]},"sequence_number":1}"#,
        ),
        sse_event(
            r#"{"type":"response.output_text.delta","item_id":"msg_test","output_index":0,"content_index":0,"delta":"Hello,","sequence_number":2}"#,
        ),
        sse_event(
            r#"{"type":"response.output_text.delta","item_id":"msg_test","output_index":0,"content_index":0,"delta":" World!","sequence_number":3}"#,
        ),
        sse_event(
            r#"{"type":"response.output_item.done","output_index":0,"item":{"id":"msg_test","type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":"Hello, World!"}]},"sequence_number":4}"#,
        ),
        sse_event(
            r#"{"type":"response.completed","response":{"id":"resp_test","model":"deepseek-ai/DeepSeek-V3-0324","object":"response","created_at":1741269112,"status":"completed","incomplete_details":null,"usage":{"input_tokens":12,"output_tokens":25,"total_tokens":37},"output":[{"id":"msg_test","type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":"Hello, World!"}]}]},"sequence_number":5}"#,
        ),
    ]);
    mock_sse(&server, chunks).await;

    let provider = make_provider(&server);
    let model = provider.responses("deepseek-ai/DeepSeek-V3-0324");

    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    let parts = collect_stream(result).await;

    // Expected: stream-start, response-metadata, text-start, text-delta×2,
    // text-end, finish
    assert_eq!(parts.len(), 7);

    assert!(matches!(&parts[0], StreamPart::StreamStart { warnings } if warnings.is_empty()));

    match &parts[1] {
        StreamPart::ResponseMetadata(ResponseMetadata { id, model_id, .. }) => {
            assert_eq!(id.as_deref(), Some("resp_test"));
            assert_eq!(model_id.as_deref(), Some("deepseek-ai/DeepSeek-V3-0324"));
        }
        other => panic!("expected ResponseMetadata, got {other:?}"),
    }

    match &parts[2] {
        StreamPart::TextStart { id, .. } => assert_eq!(id, "msg_test"),
        other => panic!("expected TextStart, got {other:?}"),
    }

    match &parts[3] {
        StreamPart::TextDelta { id, delta, .. } => {
            assert_eq!(id, "msg_test");
            assert_eq!(delta, "Hello,");
        }
        other => panic!("expected TextDelta, got {other:?}"),
    }
    match &parts[4] {
        StreamPart::TextDelta { id, delta, .. } => {
            assert_eq!(id, "msg_test");
            assert_eq!(delta, " World!");
        }
        other => panic!("expected TextDelta, got {other:?}"),
    }
    match &parts[5] {
        StreamPart::TextEnd { id, .. } => assert_eq!(id, "msg_test"),
        other => panic!("expected TextEnd, got {other:?}"),
    }
    match &parts[6] {
        StreamPart::Finish {
            finish_reason,
            usage,
            ..
        } => {
            assert_eq!(finish_reason.unified, FinishReasonUnified::Stop);
            assert_eq!(usage.input_tokens.total, Some(12));
            assert_eq!(usage.output_tokens.total, Some(25));
        }
        other => panic!("expected Finish, got {other:?}"),
    }
}

/// Partial port of upstream `should handle streaming without usage`: checks
/// only the total token counts, not every usage field.
#[tokio::test]
async fn should_handle_streaming_without_usage() {
    let server = MockServer::start().await;
    let chunks = sse_body(&[
        sse_event(
            r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"msg_test","type":"message","role":"assistant","status":"in_progress"},"sequence_number":1}"#,
        ),
        sse_event(
            r#"{"type":"response.output_text.delta","item_id":"msg_test","output_index":0,"content_index":0,"delta":"Hi!","sequence_number":2}"#,
        ),
        sse_event(
            r#"{"type":"response.output_item.done","output_index":0,"item":{"id":"msg_test","type":"message","role":"assistant","status":"completed"},"sequence_number":3}"#,
        ),
        sse_event(
            r#"{"type":"response.completed","response":{"id":"resp_test","status":"completed","incomplete_details":null,"usage":null},"sequence_number":4}"#,
        ),
    ]);
    mock_sse(&server, chunks).await;

    let provider = make_provider(&server);
    let model = provider.responses("deepseek-ai/DeepSeek-V3-0324");

    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    let parts = collect_stream(result).await;
    let finish = parts
        .iter()
        .find(|p| matches!(p, StreamPart::Finish { .. }))
        .expect("should have a Finish part");

    match finish {
        StreamPart::Finish { usage, .. } => {
            assert_eq!(usage.input_tokens.total, None);
            assert_eq!(usage.output_tokens.total, None);
        }
        _ => unreachable!(),
    }
}

/// aimux's own: a malformed frame followed by more events. Asserts the stream
/// keeps going and ends with a Stop finish. Upstream's case ends with an error
/// finish; full case: src/huggingface/tests/responses/responses_language_model.rs.
#[tokio::test]
async fn should_handle_streaming_errors() {
    let server = MockServer::start().await;
    let chunks = sse_body(&[
        sse_event(
            r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"msg_test","type":"message","role":"assistant"},"sequence_number":1}"#,
        ),
        "data:invalid json}\n\n".to_string(),
        sse_event(
            r#"{"type":"response.output_text.delta","item_id":"msg_test","output_index":0,"content_index":0,"delta":"after-error","sequence_number":2}"#,
        ),
        sse_event(
            r#"{"type":"response.output_item.done","output_index":0,"item":{"id":"msg_test","type":"message","role":"assistant","status":"completed"},"sequence_number":3}"#,
        ),
        sse_event(
            r#"{"type":"response.completed","response":{"id":"resp_test","status":"completed","incomplete_details":null,"usage":null},"sequence_number":4}"#,
        ),
    ]);
    mock_sse(&server, chunks).await;

    let provider = make_provider(&server);
    let model = provider.responses("deepseek-ai/DeepSeek-V3-0324");

    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    let outcomes: Vec<_> = result.stream.collect().await;
    let error_index = outcomes
        .iter()
        .position(|outcome| {
            matches!(
                outcome,
                Err(AiMuxError::JsonParse { .. } | AiMuxError::InvalidResponseData { .. })
            )
        })
        .expect("malformed frame should surface as a parse error item");
    assert!(outcomes[error_index + 1..].iter().any(|outcome| matches!(
        outcome,
        Ok(StreamPart::TextDelta { delta, .. }) if delta == "after-error"
    )));
    assert!(outcomes.iter().any(|outcome| matches!(
        outcome,
        Ok(StreamPart::Finish { finish_reason, .. })
            if finish_reason.unified == FinishReasonUnified::Stop
    )));
}

/// Partial port of upstream `should send correct streaming request`: checks
/// each body field, not the whole body (upstream uses toStrictEqual).
#[tokio::test]
async fn should_send_correct_streaming_request() {
    let server = MockServer::start().await;
    let chunks = sse_body(&[sse_event(
        r#"{"type":"response.completed","response":{"id":"resp_test","status":"completed"},"sequence_number":1}"#,
    )]);
    mock_sse(&server, chunks).await;

    let provider = make_provider(&server);
    let model = provider.responses("deepseek-ai/DeepSeek-V3-0324");

    let mut options = default_options(test_prompt());
    options.temperature = Some(0.7);

    let result = model.do_stream(&options).await.expect("should succeed");
    let body = result
        .request
        .and_then(|request| request.body)
        .expect("body");

    assert_eq!(body["model"], json!("deepseek-ai/DeepSeek-V3-0324"));
    assert_eq!(body["temperature"], json!(0.7));
    assert_eq!(body["stream"], json!(true));
    assert_eq!(
        body["input"],
        json!([{ "role": "user", "content": [{ "type": "input_text", "text": "Hello" }] }])
    );
}

/// Partial port of upstream `should throw for file parts with provider
/// references`: checks the error and the feature name, not the full message.
#[tokio::test]
async fn should_throw_for_file_parts_with_provider_references() {
    let server = MockServer::start().await;
    stub_empty_response(&server).await;

    let provider = make_provider(&server);
    let model = provider.responses("Qwen/Qwen2.5-VL-32B-Instruct");

    let prompt = vec![LanguageModelMessage::User {
        content: vec![UserPart::File(FilePart {
            data: FileData::Reference {
                reference: [("huggingface".into(), "file-ref-123".into())].into(),
            },
            media_type: "image/jpeg".into(),
            filename: None,
            provider_options: None,
        })],
        provider_options: None,
    }];

    let result = model.do_generate(&default_options(prompt)).await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.to_string()
            .contains("file parts with provider references"),
        "error should mention provider references, got: {err}"
    );
}

/// Partial port of upstream `should warn about tool messages`: compares only
/// the Unsupported feature names, not the warnings array.
#[tokio::test]
async fn should_warn_about_tool_messages() {
    let server = MockServer::start().await;
    stub_empty_response(&server).await;

    let provider = make_provider(&server);
    let model = provider.responses("deepseek-ai/DeepSeek-V3-0324");

    let prompt = vec![LanguageModelMessage::Tool {
        content: vec![ToolPart::ToolResult(ToolResultPart {
            tool_call_id: "test".into(),
            tool_name: "test".into(),
            output: ToolResultOutput::Text {
                value: "test".into(),
                provider_options: None,
            },

            provider_options: None,
        })],
        provider_options: None,
    }];

    let result = model
        .do_generate(&default_options(prompt))
        .await
        .expect("ok");

    let features: Vec<String> = result
        .warnings
        .iter()
        .filter_map(|w| match w {
            aimux_core::types::Warning::Unsupported { feature, .. } => Some(feature.clone()),
            _ => None,
        })
        .collect();

    assert_eq!(features, vec!["tool messages"]);
}

/// Checks the content kinds and values of upstream `should handle function
/// call tool responses`; full case:
/// src/huggingface/tests/responses/responses_language_model.rs.
#[tokio::test]
async fn should_handle_function_call_tool_responses() {
    let server = MockServer::start().await;
    mock_json(
        &server,
        json!({
            "id": "resp_tool_test",
            "model": "deepseek-ai/DeepSeek-V3-0324",
            "object": "response",
            "created_at": 1741257730,
            "status": "completed",
            "error": null,
            "instructions": null,
            "max_output_tokens": null,
            "metadata": null,
            "tool_choice": "auto",
            "tools": [],
            "temperature": 1.0,
            "top_p": 1.0,
            "incomplete_details": null,
            "usage": { "input_tokens": 50, "output_tokens": 30, "total_tokens": 80 },
            "output": [
                {
                    "id": "fc_test",
                    "type": "function_call",
                    "call_id": "call_123",
                    "name": "getWeather",
                    "arguments": "{\"location\": \"New York\"}",
                    "output": "{\"temperature\": \"72°F\", \"condition\": \"sunny\"}"
                },
                {
                    "id": "msg_after_tool",
                    "type": "message",
                    "role": "assistant",
                    "status": "completed",
                    "content": [
                        { "type": "output_text", "text": "The weather in New York is 72°F and sunny." }
                    ]
                }
            ],
            "output_text": null
        }),
    )
    .await;

    let provider = make_provider(&server);
    let model = provider.responses("deepseek-ai/DeepSeek-V3-0324");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    // content: tool-call, tool-result (the item has an output), text.
    assert_eq!(result.content.len(), 3);
    match &result.content[0] {
        GenerateContent::ToolCall(RawToolCall {
            tool_call_id,
            tool_name,
            input,
            ..
        }) => {
            assert_eq!(tool_call_id, "call_123");
            assert_eq!(tool_name, "getWeather");
            assert_eq!(input, &Value::String(r#"{"location": "New York"}"#.into()));
        }
        other => panic!("expected ToolCall at [0], got {other:?}"),
    }
    match &result.content[1] {
        GenerateContent::ToolResult(ToolResult {
            tool_call_id,
            tool_name,
            result,
            ..
        }) => {
            assert_eq!(tool_call_id, "call_123");
            assert_eq!(tool_name, "getWeather");
            assert_eq!(
                result,
                &Value::String(r#"{"temperature": "72°F", "condition": "sunny"}"#.into())
            );
        }
        other => panic!("expected ToolResult at [1], got {other:?}"),
    }
    match &result.content[2] {
        GenerateContent::Text { text, .. } => {
            assert_eq!(text, "The weather in New York is 72°F and sunny.");
        }
        other => panic!("expected Text at [2], got {other:?}"),
    }
}

/// Partial port of upstream `should stream tool calls`: checks ids, names,
/// input and token counts, not the tool-result name, timestamp or metadata.
#[tokio::test]
async fn should_stream_tool_calls() {
    let server = MockServer::start().await;
    let chunks = sse_body(&[
        sse_event(
            r#"{"type":"response.created","response":{"id":"resp_tool_stream","object":"response","created_at":1741269019,"status":"in_progress","model":"deepseek-ai/DeepSeek-V3-0324"}}"#,
        ),
        sse_event(
            r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"fc_stream","type":"function_call","call_id":"call_456","name":"calculator","arguments":""},"sequence_number":1}"#,
        ),
        sse_event(
            r#"{"type":"response.function_call_arguments.delta","item_id":"fc_stream","output_index":0,"delta":"{\"operation\"","sequence_number":2}"#,
        ),
        sse_event(
            r#"{"type":"response.function_call_arguments.delta","item_id":"fc_stream","output_index":0,"delta":": \"add\", \"a\": 5, \"b\": 3}","sequence_number":3}"#,
        ),
        sse_event(
            r#"{"type":"response.output_item.done","output_index":0,"item":{"id":"fc_stream","type":"function_call","call_id":"call_456","name":"calculator","arguments":"{\"operation\": \"add\", \"a\": 5, \"b\": 3}","output":"8"},"sequence_number":4}"#,
        ),
        sse_event(
            r#"{"type":"response.completed","response":{"id":"resp_tool_stream","status":"completed","usage":{"input_tokens":20,"output_tokens":15,"total_tokens":35}},"sequence_number":5}"#,
        ),
    ]);
    mock_sse(&server, chunks).await;

    let provider = make_provider(&server);
    let model = provider.responses("deepseek-ai/DeepSeek-V3-0324");

    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    let parts = collect_stream(result).await;

    // Expected: stream-start, response-metadata, tool-input-start,
    // tool-input-end, tool-call, tool-result, finish
    assert_eq!(parts.len(), 7);

    assert!(matches!(&parts[0], StreamPart::StreamStart { .. }));

    match &parts[1] {
        StreamPart::ResponseMetadata(ResponseMetadata { id, model_id, .. }) => {
            assert_eq!(id.as_deref(), Some("resp_tool_stream"));
            assert_eq!(model_id.as_deref(), Some("deepseek-ai/DeepSeek-V3-0324"));
        }
        other => panic!("expected ResponseMetadata, got {other:?}"),
    }

    match &parts[2] {
        StreamPart::ToolInputStart { id, tool_name, .. } => {
            assert_eq!(id, "call_456");
            assert_eq!(tool_name, "calculator");
        }
        other => panic!("expected ToolInputStart, got {other:?}"),
    }
    match &parts[3] {
        StreamPart::ToolInputEnd { id, .. } => assert_eq!(id, "call_456"),
        other => panic!("expected ToolInputEnd, got {other:?}"),
    }
    match &parts[4] {
        StreamPart::ToolCall(RawToolCall {
            tool_call_id,
            tool_name,
            input,
            ..
        }) => {
            assert_eq!(tool_call_id, "call_456");
            assert_eq!(tool_name, "calculator");
            assert_eq!(
                input,
                &Value::String(r#"{"operation": "add", "a": 5, "b": 3}"#.into())
            );
        }
        other => panic!("expected ToolCall, got {other:?}"),
    }
    match &parts[5] {
        StreamPart::ToolResult(ToolResult {
            tool_call_id,
            result,
            ..
        }) => {
            assert_eq!(tool_call_id, "call_456");
            assert_eq!(result, &json!("8"));
        }
        other => panic!("expected ToolResult, got {other:?}"),
    }
    match &parts[6] {
        StreamPart::Finish {
            finish_reason,
            usage,
            ..
        } => {
            assert_eq!(finish_reason.unified, FinishReasonUnified::Stop);
            assert_eq!(usage.input_tokens.total, Some(20));
            assert_eq!(usage.output_tokens.total, Some(15));
        }
        other => panic!("expected Finish, got {other:?}"),
    }
}

/// Partial port of upstream `should send text.format for structured output`:
/// checks text.format, not the whole text object.
#[tokio::test]
async fn should_send_text_format_for_structured_output() {
    let server = MockServer::start().await;
    mock_json(
        &server,
        json!({
            "id": "resp_structured",
            "model": "moonshotai/Kimi-K2-Instruct",
            "object": "response",
            "created_at": 1741257730,
            "status": "completed",
            "error": null,
            "instructions": null,
            "max_output_tokens": null,
            "metadata": null,
            "tool_choice": "auto",
            "tools": [],
            "temperature": 1.0,
            "top_p": 1.0,
            "incomplete_details": null,
            "usage": null,
            "output": [
                {
                    "id": "msg_structured",
                    "type": "message",
                    "role": "assistant",
                    "status": "completed",
                    "content": [
                        { "type": "output_text", "text": "{\"name\": \"John Doe\", \"age\": 30}" }
                    ]
                }
            ],
            "output_text": null
        }),
    )
    .await;

    let provider = make_provider(&server);
    let model = provider.responses("moonshotai/Kimi-K2-Instruct");

    let mut options = default_options(test_prompt());
    options.response_format = Some(ResponseFormat::Json {
        schema: Some(json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "age": { "type": "number" }
            },
            "required": ["name", "age"]
        })),
        name: None,
        description: None,
    });

    let result = model.do_generate(&options).await.expect("should succeed");
    let body = result
        .request
        .and_then(|request| request.body)
        .expect("body");

    assert_eq!(
        body["text"]["format"],
        json!({
            "type": "json_schema",
            "strict": false,
            "name": "response",
            "schema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "age": { "type": "number" }
                },
                "required": ["name", "age"]
            }
        })
    );
    // description should NOT be present (it was None).
    assert!(
        body["text"]["format"].get("description").is_none(),
        "description should be absent when None"
    );
}

/// Partial port of upstream `should handle reasoning content in responses`:
/// the text part's providerMetadata itemId is not checked.
#[tokio::test]
async fn should_handle_reasoning_content_in_responses() {
    let server = MockServer::start().await;
    mock_json(
        &server,
        json!({
            "id": "resp_reasoning",
            "model": "deepseek-ai/DeepSeek-R1",
            "object": "response",
            "created_at": 1741257730,
            "status": "completed",
            "error": null,
            "instructions": null,
            "max_output_tokens": null,
            "metadata": null,
            "tool_choice": "auto",
            "tools": [],
            "temperature": 1.0,
            "top_p": 1.0,
            "incomplete_details": null,
            "usage": { "input_tokens": 10, "output_tokens": 50, "total_tokens": 60 },
            "output": [
                {
                    "id": "reasoning_1",
                    "type": "reasoning",
                    "content": [
                        { "type": "reasoning_text", "text": "Let me think about this problem step by step..." }
                    ]
                },
                {
                    "id": "msg_after_reasoning",
                    "type": "message",
                    "role": "assistant",
                    "status": "completed",
                    "content": [
                        { "type": "output_text", "text": "The answer is 42." }
                    ]
                }
            ],
            "output_text": null
        }),
    )
    .await;

    let provider = make_provider(&server);
    let model = provider.responses("deepseek-ai/DeepSeek-R1");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    assert_eq!(result.content.len(), 2);
    match &result.content[0] {
        GenerateContent::Reasoning(ReasoningOutput {
            text,
            provider_metadata,
        }) => {
            assert_eq!(text, "Let me think about this problem step by step...");
            assert_eq!(
                provider_metadata.as_ref(),
                Some(
                    &aimux_core::shared::provider_namespace(
                        "huggingface",
                        json!({ "itemId": "reasoning_1" })
                    )
                    .unwrap()
                )
            );
        }
        other => panic!("expected Reasoning at [0], got {other:?}"),
    }
    match &result.content[1] {
        GenerateContent::Text { text, .. } => assert_eq!(text, "The answer is 42."),
        other => panic!("expected Text at [1], got {other:?}"),
    }
}

/// Partial port of upstream `should stream reasoning content`: checks part
/// kinds, ids and deltas, not the timestamp, text-start or finish metadata.
#[tokio::test]
async fn should_stream_reasoning_content() {
    let server = MockServer::start().await;
    let chunks = sse_body(&[
        sse_event(
            r#"{"type":"response.created","response":{"id":"resp_reasoning_stream","object":"response","created_at":1741269019,"status":"in_progress","model":"deepseek-ai/DeepSeek-R1"}}"#,
        ),
        sse_event(
            r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"reasoning_stream","type":"reasoning"},"sequence_number":1}"#,
        ),
        sse_event(
            r#"{"type":"response.reasoning_text.delta","item_id":"reasoning_stream","output_index":0,"content_index":0,"delta":"Thinking about","sequence_number":2}"#,
        ),
        sse_event(
            r#"{"type":"response.reasoning_text.delta","item_id":"reasoning_stream","output_index":0,"content_index":0,"delta":" the problem...","sequence_number":3}"#,
        ),
        sse_event(
            r#"{"type":"response.reasoning_text.done","item_id":"reasoning_stream","output_index":0,"content_index":0,"text":"Thinking about the problem...","sequence_number":4}"#,
        ),
        sse_event(
            r#"{"type":"response.output_item.done","output_index":0,"item":{"id":"reasoning_stream","type":"reasoning","content":[{"type":"reasoning_text","text":"Thinking about the problem..."}]},"sequence_number":5}"#,
        ),
        sse_event(
            r#"{"type":"response.output_item.added","output_index":1,"item":{"id":"msg_stream","type":"message","role":"assistant"},"sequence_number":6}"#,
        ),
        sse_event(
            r#"{"type":"response.output_text.delta","item_id":"msg_stream","output_index":1,"content_index":0,"delta":"The solution is","sequence_number":7}"#,
        ),
        sse_event(
            r#"{"type":"response.output_text.delta","item_id":"msg_stream","output_index":1,"content_index":0,"delta":" simple.","sequence_number":8}"#,
        ),
        sse_event(
            r#"{"type":"response.output_item.done","output_index":1,"item":{"id":"msg_stream","type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":"The solution is simple."}]},"sequence_number":9}"#,
        ),
        sse_event(
            r#"{"type":"response.completed","response":{"id":"resp_reasoning_stream","status":"completed","usage":{"input_tokens":10,"output_tokens":20,"total_tokens":30}},"sequence_number":10}"#,
        ),
    ]);
    mock_sse(&server, chunks).await;

    let provider = make_provider(&server);
    let model = provider.responses("deepseek-ai/DeepSeek-R1");

    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    let parts = collect_stream(result).await;

    // Expected: stream-start, response-metadata, reasoning-start,
    // reasoning-delta×2, reasoning-end, text-start, text-delta×2, text-end,
    // finish
    assert_eq!(parts.len(), 11);

    assert!(matches!(&parts[0], StreamPart::StreamStart { .. }));

    match &parts[1] {
        StreamPart::ResponseMetadata(ResponseMetadata { id, model_id, .. }) => {
            assert_eq!(id.as_deref(), Some("resp_reasoning_stream"));
            assert_eq!(model_id.as_deref(), Some("deepseek-ai/DeepSeek-R1"));
        }
        other => panic!("expected ResponseMetadata, got {other:?}"),
    }

    match &parts[2] {
        StreamPart::ReasoningStart {
            id,
            provider_metadata,
        } => {
            assert_eq!(id, "reasoning_stream");
            assert_eq!(
                provider_metadata.as_ref(),
                Some(
                    &aimux_core::shared::provider_namespace(
                        "huggingface",
                        json!({ "itemId": "reasoning_stream" })
                    )
                    .unwrap()
                )
            );
        }
        other => panic!("expected ReasoningStart, got {other:?}"),
    }
    match &parts[3] {
        StreamPart::ReasoningDelta { id, delta, .. } => {
            assert_eq!(id, "reasoning_stream");
            assert_eq!(delta, "Thinking about");
        }
        other => panic!("expected ReasoningDelta, got {other:?}"),
    }
    match &parts[4] {
        StreamPart::ReasoningDelta { id, delta, .. } => {
            assert_eq!(id, "reasoning_stream");
            assert_eq!(delta, " the problem...");
        }
        other => panic!("expected ReasoningDelta, got {other:?}"),
    }
    match &parts[5] {
        StreamPart::ReasoningEnd { id, .. } => assert_eq!(id, "reasoning_stream"),
        other => panic!("expected ReasoningEnd, got {other:?}"),
    }
    match &parts[6] {
        StreamPart::TextStart { id, .. } => assert_eq!(id, "msg_stream"),
        other => panic!("expected TextStart, got {other:?}"),
    }
    match &parts[7] {
        StreamPart::TextDelta { id, delta, .. } => {
            assert_eq!(id, "msg_stream");
            assert_eq!(delta, "The solution is");
        }
        other => panic!("expected TextDelta, got {other:?}"),
    }
    match &parts[8] {
        StreamPart::TextDelta { id, delta, .. } => {
            assert_eq!(id, "msg_stream");
            assert_eq!(delta, " simple.");
        }
        other => panic!("expected TextDelta, got {other:?}"),
    }
    match &parts[9] {
        StreamPart::TextEnd { id, .. } => assert_eq!(id, "msg_stream"),
        other => panic!("expected TextEnd, got {other:?}"),
    }
    match &parts[10] {
        StreamPart::Finish {
            finish_reason,
            usage,
            ..
        } => {
            assert_eq!(finish_reason.unified, FinishReasonUnified::Stop);
            assert_eq!(usage.input_tokens.total, Some(10));
            assert_eq!(usage.output_tokens.total, Some(20));
        }
        other => panic!("expected Finish, got {other:?}"),
    }
}

/// aimux's own: supported_urls accepts HTTP(S) image URLs and no others. It
/// mirrors supportedUrls (huggingface-responses-language-model.ts:67-69); no
/// upstream test case covers it.
#[tokio::test]
async fn supports_http_image_urls() {
    let server = MockServer::start().await;
    let urls = make_provider(&server)
        .responses("deepseek-ai/DeepSeek-V3-0324")
        .supported_urls()
        .await;
    assert_eq!(urls.0.len(), 1);
    assert!(urls.0["image/*"][0].is_match("https://example.com/a.png"));
    assert!(urls.0["image/*"][0].is_match("http://example.com/a.png"));
    assert!(!urls.0["image/*"][0].is_match("ftp://example.com/a.png"));
}

/// aimux's own: an `mcp_call` whose `output` is an empty string gives only the
/// tool-call; upstream pushes the tool-result only `if (part.output)`
/// (huggingface-responses-language-model.ts:273).
#[tokio::test]
async fn mcp_call_with_empty_output_has_no_tool_result() {
    let server = MockServer::start().await;
    let mut body = basic_response_body();
    body["output"] = json!([{
        "id": "mcp_empty",
        "type": "mcp_call",
        "server_label": "web_search",
        "name": "search",
        "arguments": "{}",
        "output": ""
    }]);
    mock_json(&server, body).await;

    let result = make_provider(&server)
        .responses("deepseek-ai/DeepSeek-V3-0324")
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    assert_eq!(result.content.len(), 1);
    assert!(matches!(result.content[0], GenerateContent::ToolCall(_)));
}

/// aimux's own: an `error` or `response.failed` event after the first one is
/// an error part, and the finish reason becomes error with the code (else the
/// event type) as raw (huggingface-responses-language-model.ts:406-419,
/// :562-590). Upstream's cases send the error as the first event, which aimux
/// returns from `do_stream` instead (registered SSE pre-read).
#[tokio::test]
async fn reads_mid_stream_error_events() {
    let created = r#"{"type":"response.created","response":{"id":"resp_test","object":"response","created_at":1741269019,"status":"in_progress","model":"deepseek-ai/DeepSeek-V3-0324"},"sequence_number":0}"#;
    for (event, message, raw) in [
        (
            r#"{"type":"response.failed","response":{"error":{"code":"429","message":"Rate limit reached"}},"sequence_number":1}"#,
            "Rate limit reached",
            "429",
        ),
        (
            r#"{"type":"error","message":"Service unavailable","param":null,"sequence_number":1}"#,
            "Service unavailable",
            "error",
        ),
    ] {
        let server = MockServer::start().await;
        mock_sse(&server, sse_body(&[sse_event(created), sse_event(event)])).await;

        let result = make_provider(&server)
            .responses("deepseek-ai/DeepSeek-V3-0324")
            .do_stream(&default_options(test_prompt()))
            .await
            .expect("the first event is not an error");
        let parts = collect_stream(result).await;

        let error = parts
            .iter()
            .find_map(|part| match part {
                StreamPart::Error { error } => Some(error),
                _ => None,
            })
            .expect("an error part");
        match error {
            AiMuxError::ApiCall(error) => {
                assert_eq!(error.message, message);
                assert_eq!(error.provider_code.as_deref(), Some(raw));
            }
            other => panic!("expected ApiCall, got {other:?}"),
        }
        match parts.last() {
            Some(StreamPart::Finish { finish_reason, .. }) => {
                assert_eq!(finish_reason.unified, FinishReasonUnified::Error);
                assert_eq!(finish_reason.raw.as_deref(), Some(raw));
            }
            other => panic!("expected Finish last, got {other:?}"),
        }
    }
}

/// aimux's own: a chunk that fails to parse is an error part and, when no
/// later `response.completed` sets it again, the finish reason is error with
/// no raw value (huggingface-responses-language-model.ts:396-404).
#[tokio::test]
async fn malformed_chunk_ends_with_an_error_finish() {
    let server = MockServer::start().await;
    let chunks = sse_body(&[
        sse_event(
            r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"msg_test","type":"message","role":"assistant"},"sequence_number":1}"#,
        ),
        "data:invalid json}\n\n".to_string(),
    ]);
    mock_sse(&server, chunks).await;

    let result = make_provider(&server)
        .responses("deepseek-ai/DeepSeek-V3-0324")
        .do_stream(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    let outcomes: Vec<_> = result.stream.collect().await;
    assert!(outcomes.iter().any(Result::is_err), "a parse error item");
    match outcomes.last() {
        Some(Ok(StreamPart::Finish { finish_reason, .. })) => {
            assert_eq!(finish_reason.unified, FinishReasonUnified::Error);
            assert_eq!(finish_reason.raw, None);
        }
        other => panic!("expected Finish last, got {other:?}"),
    }
}
