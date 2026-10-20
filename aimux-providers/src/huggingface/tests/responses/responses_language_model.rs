//! Port of `src/responses/huggingface-responses-language-model.test.ts` from
//! `@ai-sdk/huggingface@2.0.62`: the cases the Responses test file
//! (`tests/huggingface_responses_test.rs`) does not cover in full.
//!
//! Harness differences from the TypeScript (expectations are unchanged):
//! - `createTestServer` is the [`MockFetch`] transport. `server.calls[0]` is the
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
}
