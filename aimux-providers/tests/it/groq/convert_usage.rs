//! Ports of `src/convert-groq-usage.test.ts` from `@ai-sdk/groq`, checked
//! through `do_generate`'s `usage`.

use super::support::*;

mod convert_groq_usage {
    use super::*;

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

        assert_eq!(result.usage.input_tokens.total, Some(20));
        assert_eq!(result.usage.input_tokens.no_cache, Some(20));
        assert_eq!(result.usage.output_tokens.total, Some(10));
        assert_eq!(result.usage.output_tokens.text, Some(10));
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

        assert_eq!(result.usage.input_tokens.total, Some(79));
        assert_eq!(result.usage.output_tokens.total, Some(40));
        assert_eq!(result.usage.output_tokens.reasoning, Some(21));
        assert_eq!(result.usage.output_tokens.text, Some(19)); // 40 - 21
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

        assert_eq!(result.usage.output_tokens.reasoning, Some(0));
        assert_eq!(result.usage.output_tokens.text, Some(10));
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

        assert_eq!(result.usage.output_tokens.total, Some(50));
        assert_eq!(result.usage.output_tokens.text, Some(0)); // 50 - 50
        assert_eq!(result.usage.output_tokens.reasoning, Some(50));
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

        assert_eq!(result.usage.input_tokens.total, Some(4641));
        assert_eq!(result.usage.input_tokens.no_cache, Some(33)); // 4641 - 4608
        assert_eq!(result.usage.input_tokens.cache_read, Some(4608));
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

        assert_eq!(result.usage.input_tokens.cache_read, Some(0));
        assert_eq!(result.usage.input_tokens.no_cache, Some(20));
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

        assert_eq!(result.usage.input_tokens.total, Some(0));
        assert_eq!(result.usage.output_tokens.total, Some(0));
    }
}
