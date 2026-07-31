# Provider Adapter Layer Improvements

## Current Problems

Thin-wrapper providers differ only in URL and environment variables, with no customization points. DeepSeek's reasoning field is dropped directly, as the code comments themselves admit.

## Suggestions

### Add a Configuration Description Structure

Encode each provider's differences as data — what the reasoning field is named, whether tool calling is supported, whether streaming responses include usage statistics. Request data construction and stream parsing read this configuration to determine behavior. This keeps providers swappable at runtime while still expressing differences.

### Switch Testing to Cassette Mode

Currently, each provider's tests are handwritten once, which is repetitive and error-prone. Change it to record one real response and save it as a file; future test runs replay this file directly, with no network and no API keys. Then write a unified test suite where all providers run the same inputs and assert the same behavior. Only with these two in place can we confidently scale out providers.

## Implementation Progress (2026-07-28)

### Completed

1. **reasoning_content field parsing**: The `reasoning` field in openai/types.rs added `alias = "reasoning_content"`. The `reasoning_content` field returned by providers such as DeepSeek/Alibaba Tongyi can now be automatically parsed by the shared OpenAI parser, and is no longer dropped.

2. **OpenAICompatProfile structure**: Newly added in openai/mod.rs, describes provider differences:
   - `supports_top_k`: whether the top_k parameter is supported
   - `supports_tools`: whether tool calling is supported
   - `supports_response_format`: whether response_format is supported
   - `stream_usage_key`: special key for streaming usage (e.g. Groq's "x_groq")
   - `request_body_override`: request body post-processing (e.g. DeepSeek's thinking field)

3. **OpenAIConfig adds a profile field**: set via `with_profile()`.

4. **convert.rs integrates profile**: `build_request_body_with_warnings` takes a profile parameter; top_k is now controlled by the profile — providers with `supports_top_k=true` send top_k, those with `false` emit a warning.

5. **model.rs integrates profile**: `execute_generate`/`execute_stream` take a profile parameter and pass it to convert.

6. **DeepSeek changed from a standalone implementation to a thin wrapper**: deleted 668 lines of standalone model.rs + convert.rs + types.rs, replaced with a 70-line thin wrapper. The thinking field and reasoning_effort remapping are handled in the shared convert.rs via `RequestBodyOverride::DeepSeek`. reasoning_content is parsed automatically via the serde alias.

7. **All 145 thin wrappers have integrated profile**: Groq uses `groq()`, DeepSeek uses `deepseek()`, and the remaining 143 use `full()`. Azure has also been integrated.

8. **All tests pass**: all test files have 0 failures.

### Pending

1. **model.rs streaming x_groq hardcoded**: The x_groq handling in stream parsing is still hardcoded; it should read `stream_usage_key` to decide. The impact is small (x_groq is only used by Groq, and the current hardcoded result is correct).

2. **supports_tools / supports_response_format integrate into convert**: Currently defined but convert.rs does not read them yet — all providers currently default to supported. If a provider does not support them, integration is needed.
