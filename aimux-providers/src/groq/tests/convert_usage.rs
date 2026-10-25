//! Upstream `convert-groq-usage.test.ts`: direct calls, plus the older cases
//! (formerly `tests/groq_test.rs`) checked through `do_generate`'s `usage` on a
//! wiremock `MockServer` ([`super::support`]).
//!
//! `undefined` in the upstream expectations is an absent key in the serialized
//! `Usage`, since serde skips `None`; the expected JSON therefore omits it.

use serde_json::{Value, json};

use crate::groq::api_types::GroqUsage;
use crate::groq::convert_usage::convert_groq_usage;

use super::support::*;

/// `convertGroqUsage(usage)` for a Groq usage object, as its serialized form.
fn convert_json(usage: Value) -> Value {
    let usage: GroqUsage = serde_json::from_value(usage).expect("valid Groq usage");
    serde_json::to_value(convert_groq_usage(Some(&usage))).expect("usage serializes")
}

mod convert_groq_usage {
    use super::*;

    /// upstream: @ai-sdk/groq src/convert-groq-usage.test.ts › convertGroqUsage › should return undefined values when usage is null
    #[test]
    fn should_return_undefined_values_when_usage_is_null() {
        let result = convert_groq_usage(None);

        assert_eq!(
            serde_json::to_value(result).expect("usage serializes"),
            json!({ "inputTokens": {}, "outputTokens": {} })
        );
    }

    /// upstream: @ai-sdk/groq src/convert-groq-usage.test.ts › convertGroqUsage › should return undefined values when usage is undefined
    #[test]
    fn should_return_undefined_values_when_usage_is_undefined() {
        // An absent usage object is `None`, the same as `null` above.
        let result = convert_groq_usage(None);

        assert_eq!(
            serde_json::to_value(result).expect("usage serializes"),
            json!({ "inputTokens": {}, "outputTokens": {} })
        );
    }

    /// upstream: @ai-sdk/groq src/convert-groq-usage.test.ts › convertGroqUsage › should handle null reasoning_tokens in completion_tokens_details
    #[test]
    fn should_handle_null_reasoning_tokens_in_completion_tokens_details() {
        let result = convert_json(json!({
            "prompt_tokens": 20,
            "completion_tokens": 10,
            "completion_tokens_details": {
                "reasoning_tokens": null,
            },
        }));

        assert_eq!(
            result,
            json!({
                "inputTokens": {
                    "total": 20,
                    "noCache": 20,
                },
                "outputTokens": {
                    "total": 10,
                    "text": 10,
                },
                "raw": {
                    "prompt_tokens": 20,
                    "completion_tokens": 10,
                    "completion_tokens_details": {
                        "reasoning_tokens": null,
                    },
                },
            })
        );
    }

    /// upstream: @ai-sdk/groq src/convert-groq-usage.test.ts › convertGroqUsage › should handle null completion_tokens_details
    #[test]
    fn should_handle_null_completion_tokens_details() {
        let result = convert_json(json!({
            "prompt_tokens": 20,
            "completion_tokens": 10,
            "completion_tokens_details": null,
        }));

        assert_eq!(
            result,
            json!({
                "inputTokens": {
                    "total": 20,
                    "noCache": 20,
                },
                "outputTokens": {
                    "total": 10,
                    "text": 10,
                },
                "raw": {
                    "prompt_tokens": 20,
                    "completion_tokens": 10,
                    "completion_tokens_details": null,
                },
            })
        );
    }

    /// upstream: @ai-sdk/groq src/convert-groq-usage.test.ts › convertGroqUsage › clamps text tokens at 0 when reasoning exceeds completion
    #[test]
    fn clamps_text_tokens_at_0_when_reasoning_exceeds_completion() {
        let result = convert_json(json!({
            "prompt_tokens": 951,
            "completion_tokens": 6000,
            "completion_tokens_details": {
                "reasoning_tokens": 6001,
            },
        }));

        assert_eq!(
            result["outputTokens"],
            json!({
                "total": 6000,
                "text": 0,
                "reasoning": 6001,
            })
        );
    }

    /// upstream: @ai-sdk/groq src/convert-groq-usage.test.ts › convertGroqUsage › should leave cacheRead undefined when cached_tokens is null
    #[test]
    fn should_leave_cache_read_undefined_when_cached_tokens_is_null() {
        let result = convert_json(json!({
            "prompt_tokens": 20,
            "completion_tokens": 10,
            "prompt_tokens_details": {
                "cached_tokens": null,
            },
        }));

        assert_eq!(
            result["inputTokens"],
            json!({
                "total": 20,
                "noCache": 20,
            })
        );
    }

    /// upstream: @ai-sdk/groq src/convert-groq-usage.test.ts › convertGroqUsage › should convert basic usage without token details
    #[tokio::test]
    async fn basic_usage() {
        let server = MockServer::start().await;
        mock_json(
            &server,
            json!({
                "id": "chatcmpl-test",
                "object": "chat.completion",
                "created": 1711115037,
                "model": "gemma2-9b-it",
                "choices": [{
                    "index": 0,
                    "message": { "role": "assistant", "content": "" },
                    "finish_reason": "stop"
                }],
                "usage": { "prompt_tokens": 20, "completion_tokens": 10 }
            }),
        )
        .await;

        let model = make_provider(&server);

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        assert_eq!(
            serde_json::to_value(&result.usage).unwrap(),
            json!({
                "inputTokens": { "total": 20, "noCache": 20 },
                "outputTokens": { "total": 10, "text": 10 },
                "raw": { "prompt_tokens": 20, "completion_tokens": 10 }
            })
        );
    }

    /// upstream: @ai-sdk/groq src/convert-groq-usage.test.ts › convertGroqUsage › should extract reasoning tokens from completion_tokens_details
    #[tokio::test]
    async fn reasoning_tokens() {
        let server = MockServer::start().await;
        mock_json(
            &server,
            json!({
                "id": "chatcmpl-test",
                "object": "chat.completion",
                "created": 1711115037,
                "model": "gemma2-9b-it",
                "choices": [{
                    "index": 0,
                    "message": { "role": "assistant", "content": "" },
                    "finish_reason": "stop"
                }],
                "usage": {
                    "prompt_tokens": 79,
                    "completion_tokens": 40,
                    "completion_tokens_details": { "reasoning_tokens": 21 }
                }
            }),
        )
        .await;

        let model = make_provider(&server);

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        assert_eq!(
            serde_json::to_value(&result.usage).unwrap(),
            json!({
                "inputTokens": { "total": 79, "noCache": 79 },
                "outputTokens": { "total": 40, "text": 19, "reasoning": 21 },
                "raw": {
                    "prompt_tokens": 79,
                    "completion_tokens": 40,
                    "completion_tokens_details": { "reasoning_tokens": 21 }
                }
            })
        );
    }

    /// upstream: @ai-sdk/groq src/convert-groq-usage.test.ts › convertGroqUsage › should handle zero reasoning tokens
    #[tokio::test]
    async fn zero_reasoning_tokens() {
        let server = MockServer::start().await;
        mock_json(
            &server,
            json!({
                "id": "chatcmpl-test",
                "object": "chat.completion",
                "created": 1711115037,
                "model": "gemma2-9b-it",
                "choices": [{
                    "index": 0,
                    "message": { "role": "assistant", "content": "" },
                    "finish_reason": "stop"
                }],
                "usage": {
                    "prompt_tokens": 20,
                    "completion_tokens": 10,
                    "completion_tokens_details": { "reasoning_tokens": 0 }
                }
            }),
        )
        .await;

        let model = make_provider(&server);

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        assert_eq!(
            serde_json::to_value(&result.usage).unwrap(),
            json!({
                "inputTokens": { "total": 20, "noCache": 20 },
                "outputTokens": { "total": 10, "text": 10, "reasoning": 0 },
                "raw": {
                    "prompt_tokens": 20,
                    "completion_tokens": 10,
                    "completion_tokens_details": { "reasoning_tokens": 0 }
                }
            })
        );
    }

    /// upstream: @ai-sdk/groq src/convert-groq-usage.test.ts › convertGroqUsage › should handle all tokens being reasoning tokens
    #[tokio::test]
    async fn all_reasoning_tokens() {
        let server = MockServer::start().await;
        mock_json(
            &server,
            json!({
                "id": "chatcmpl-test",
                "object": "chat.completion",
                "created": 1711115037,
                "model": "gemma2-9b-it",
                "choices": [{
                    "index": 0,
                    "message": { "role": "assistant", "content": "" },
                    "finish_reason": "stop"
                }],
                "usage": {
                    "prompt_tokens": 20,
                    "completion_tokens": 50,
                    "completion_tokens_details": { "reasoning_tokens": 50 }
                }
            }),
        )
        .await;

        let model = make_provider(&server);

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        assert_eq!(
            serde_json::to_value(&result.usage).unwrap(),
            json!({
                "inputTokens": { "total": 20, "noCache": 20 },
                "outputTokens": { "total": 50, "text": 0, "reasoning": 50 },
                "raw": {
                    "prompt_tokens": 20,
                    "completion_tokens": 50,
                    "completion_tokens_details": { "reasoning_tokens": 50 }
                }
            })
        );
    }

    /// upstream: @ai-sdk/groq src/convert-groq-usage.test.ts › convertGroqUsage › should map cached_tokens to cacheRead and subtract from noCache
    #[tokio::test]
    async fn cached_tokens() {
        let server = MockServer::start().await;
        mock_json(
            &server,
            json!({
                "id": "chatcmpl-test",
                "object": "chat.completion",
                "created": 1711115037,
                "model": "gemma2-9b-it",
                "choices": [{
                    "index": 0,
                    "message": { "role": "assistant", "content": "" },
                    "finish_reason": "stop"
                }],
                "usage": {
                    "prompt_tokens": 4641,
                    "completion_tokens": 1817,
                    "prompt_tokens_details": { "cached_tokens": 4608 }
                }
            }),
        )
        .await;

        let model = make_provider(&server);

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        assert_eq!(
            serde_json::to_value(&result.usage).unwrap(),
            json!({
                "inputTokens": { "total": 4641, "noCache": 33, "cacheRead": 4608 },
                "outputTokens": { "total": 1817, "text": 1817 },
                "raw": {
                    "prompt_tokens": 4641,
                    "completion_tokens": 1817,
                    "prompt_tokens_details": { "cached_tokens": 4608 }
                }
            })
        );
    }

    /// upstream: @ai-sdk/groq src/convert-groq-usage.test.ts › convertGroqUsage › should treat zero cached_tokens as a cache miss (cacheRead 0)
    #[tokio::test]
    async fn zero_cached_tokens() {
        let server = MockServer::start().await;
        mock_json(
            &server,
            json!({
                "id": "chatcmpl-test",
                "object": "chat.completion",
                "created": 1711115037,
                "model": "gemma2-9b-it",
                "choices": [{
                    "index": 0,
                    "message": { "role": "assistant", "content": "" },
                    "finish_reason": "stop"
                }],
                "usage": {
                    "prompt_tokens": 20,
                    "completion_tokens": 10,
                    "prompt_tokens_details": { "cached_tokens": 0 }
                }
            }),
        )
        .await;

        let model = make_provider(&server);

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        assert_eq!(
            serde_json::to_value(&result.usage.input_tokens).unwrap(),
            json!({ "total": 20, "noCache": 20, "cacheRead": 0 })
        );
    }

    /// upstream: @ai-sdk/groq src/convert-groq-usage.test.ts › convertGroqUsage › should handle missing prompt_tokens and completion_tokens
    #[tokio::test]
    async fn missing_tokens() {
        let server = MockServer::start().await;
        mock_json(
            &server,
            json!({
                "id": "chatcmpl-test",
                "object": "chat.completion",
                "created": 1711115037,
                "model": "gemma2-9b-it",
                "choices": [{
                    "index": 0,
                    "message": { "role": "assistant", "content": "" },
                    "finish_reason": "stop"
                }],
                "usage": {}
            }),
        )
        .await;

        let model = make_provider(&server);

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        assert_eq!(
            serde_json::to_value(&result.usage).unwrap(),
            json!({
                "inputTokens": { "total": 0, "noCache": 0 },
                "outputTokens": { "total": 0, "text": 0 },
                "raw": {}
            })
        );
    }
}
