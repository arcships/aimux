//! Convert Bedrock usage to the core usage (convert-amazon-bedrock-usage.ts).

use super::api_types::BedrockUsage;

/// Convert Bedrock usage to the core `Usage` type.
///
/// Mirrors the TS `convertAmazonBedrockUsage(usage: AmazonBedrockUsage |
/// undefined | null)`: a `None`/null/undefined usage yields an all-`None`
/// `Usage` (the TS `undefined` fields). The original provider payload is
/// retained in `raw`; `outputTokens.text` mirrors the TS field.
#[must_use]
pub fn convert_usage(usage: Option<&BedrockUsage>) -> aimux_core::types::Usage {
    use aimux_core::types::Usage;

    let Some(usage) = usage else {
        return Usage::default();
    };

    let input = usage.input_tokens.unwrap_or(0);
    let output = usage.output_tokens.unwrap_or(0);
    let cache_read = usage.cache_read_input_tokens.unwrap_or(0);
    let cache_write = usage.cache_write_input_tokens.unwrap_or(0);

    Usage {
        input_tokens: aimux_core::types::InputTokenUsage {
            total: Some(input + cache_read + cache_write),
            no_cache: Some(input),
            cache_read: Some(cache_read),
            cache_write: Some(cache_write),
        },
        output_tokens: aimux_core::types::OutputTokenUsage {
            total: Some(output),
            text: Some(output),
            ..Default::default()
        },
        // RFC-0015 P0-3: keep the raw provider usage payload.
        raw: usage.raw.clone().or_else(|| {
            serde_json::to_value(usage)
                .ok()
                .and_then(|value| value.as_object().cloned())
        }),
    }
}
