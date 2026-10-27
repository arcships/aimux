//! Model limits and provider options of the OpenAI image model
//! (openai-image-model-options.ts).

use aimux_core::shared::SharedProviderOptions;
use serde_json::Value;

/// Models that default to `b64_json` response format and therefore do NOT need
/// the explicit `response_format` field. Identified by prefix.
const DEFAULT_RESPONSE_FORMAT_PREFIXES: &[&str] = &["chatgpt-image-", "gpt-image-"];

/// Returns `true` if the model ID has a default response format (and thus the
/// `response_format` field should be omitted from the request).
pub(crate) fn has_default_response_format(model_id: &str) -> bool {
    DEFAULT_RESPONSE_FORMAT_PREFIXES
        .iter()
        .any(|prefix| model_id.starts_with(prefix))
}

/// Returns the maximum number of images per call for the given model ID.
///
/// Mirrors TS `getMaxImagesPerCall`:
/// - `dall-e-3` → 1
/// - `dall-e-2`, `gpt-image-*`, `chatgpt-image-*` → 10
/// - unknown models starting with `gpt-image-` → 10
/// - everything else → 1
pub(crate) fn get_max_images_per_call(model_id: &str) -> u32 {
    match model_id {
        "dall-e-3" => 1,
        "dall-e-2"
        | "gpt-image-1"
        | "gpt-image-1-mini"
        | "gpt-image-1.5"
        | "gpt-image-2"
        | "chatgpt-image-latest" => 10,
        _ => {
            if model_id.starts_with("gpt-image-") {
                10
            } else {
                1
            }
        }
    }
}

/// OpenAI generation provider options (camelCase → snake_case mapping).
pub(crate) struct GenerationOptions {
    pub(crate) quality: Option<Value>,
    pub(crate) style: Option<Value>,
    pub(crate) background: Option<Value>,
    pub(crate) moderation: Option<Value>,
    pub(crate) output_format: Option<Value>,
    pub(crate) output_compression: Option<Value>,
    pub(crate) user: Option<Value>,
}

/// Parse generation provider options from the `"openai"` key.
pub(crate) fn parse_generation_provider_options(
    provider_options: &SharedProviderOptions,
) -> GenerationOptions {
    let openai = provider_options.get("openai");
    GenerationOptions {
        quality: openai.and_then(|o| o.get("quality")).cloned(),
        style: openai.and_then(|o| o.get("style")).cloned(),
        background: openai.and_then(|o| o.get("background")).cloned(),
        moderation: openai.and_then(|o| o.get("moderation")).cloned(),
        output_format: openai.and_then(|o| o.get("outputFormat")).cloned(),
        output_compression: openai.and_then(|o| o.get("outputCompression")).cloned(),
        user: openai.and_then(|o| o.get("user")).cloned(),
    }
}

/// OpenAI edit provider options.
pub(crate) struct EditOptions {
    pub(crate) quality: Option<Value>,
    pub(crate) background: Option<Value>,
    pub(crate) output_format: Option<Value>,
    pub(crate) output_compression: Option<Value>,
    pub(crate) input_fidelity: Option<Value>,
    pub(crate) user: Option<Value>,
}

/// Parse edit provider options from the `"openai"` key.
pub(crate) fn parse_edit_provider_options(provider_options: &SharedProviderOptions) -> EditOptions {
    let openai = provider_options.get("openai");
    EditOptions {
        quality: openai.and_then(|o| o.get("quality")).cloned(),
        background: openai.and_then(|o| o.get("background")).cloned(),
        output_format: openai.and_then(|o| o.get("outputFormat")).cloned(),
        output_compression: openai.and_then(|o| o.get("outputCompression")).cloned(),
        input_fidelity: openai.and_then(|o| o.get("inputFidelity")).cloned(),
        user: openai.and_then(|o| o.get("user")).cloned(),
    }
}
