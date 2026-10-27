//! Provider options of the OpenAI transcription model (openai-transcription-model-options.ts).

use aimux_core::shared::SharedProviderOptions;
use serde_json::Value;

/// OpenAI-specific transcription options parsed from `providerOptions.openai`.
#[derive(Debug, Clone, Default)]
pub(crate) struct OpenAITranscriptionOptions {
    /// Additional information to include in the transcription response.
    pub(crate) include: Option<Vec<String>>,
    /// The language of the input audio in ISO-639-1 format.
    pub(crate) language: Option<String>,
    /// An optional text to guide the model's style.
    pub(crate) prompt: Option<String>,
    /// The sampling temperature, between 0 and 1.
    pub(crate) temperature: Option<f64>,
    /// The timestamp granularities to populate (`word` / `segment`).
    pub(crate) timestamp_granularities: Option<Vec<String>>,
    /// `json`, `verbose_json` or `diarized_json`.
    pub(crate) response_format: Option<String>,
    /// `chunking_strategy` as sent: `auto` or a server-VAD object as JSON.
    pub(crate) chunking_strategy: Option<String>,
}

/// `chunkingStrategy`: `"auto"`, or `{ type, threshold?, prefixPaddingMs?,
/// silenceDurationMs? }` sent as snake_case JSON
/// (`transcription/openai-transcription-model.ts:241-259`).
pub(crate) fn chunking_strategy(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        return Some(text.to_string());
    }
    let object = value.as_object()?;
    let mut wire = serde_json::Map::new();
    wire.insert("type".into(), object.get("type")?.clone());
    for (key, wire_key) in [
        ("threshold", "threshold"),
        ("prefixPaddingMs", "prefix_padding_ms"),
        ("silenceDurationMs", "silence_duration_ms"),
    ] {
        if let Some(value) = object.get(key).filter(|value| !value.is_null()) {
            wire.insert(wire_key.into(), value.clone());
        }
    }
    Some(Value::Object(wire).to_string())
}

/// Parse provider options from the shared options map.
pub(crate) fn parse_openai_options(
    provider_options: Option<&SharedProviderOptions>,
) -> OpenAITranscriptionOptions {
    let mut opts = OpenAITranscriptionOptions::default();
    if let Some(po) = provider_options
        && let Some(openai) = po.get("openai")
    {
        if let Some(include) = openai.get("include").and_then(|v| v.as_array()) {
            opts.include = Some(
                include
                    .iter()
                    .filter_map(|v| v.as_str().map(std::string::ToString::to_string))
                    .collect(),
            );
        }
        if let Some(language) = openai.get("language").and_then(|v| v.as_str()) {
            opts.language = Some(language.to_string());
        }
        if let Some(prompt) = openai.get("prompt").and_then(|v| v.as_str()) {
            opts.prompt = Some(prompt.to_string());
        }
        if let Some(temp) = openai
            .get("temperature")
            .and_then(serde_json::Value::as_f64)
        {
            opts.temperature = Some(temp);
        }
        if let Some(tg) = openai
            .get("timestampGranularities")
            .and_then(|v| v.as_array())
        {
            opts.timestamp_granularities = Some(
                tg.iter()
                    .filter_map(|v| v.as_str().map(std::string::ToString::to_string))
                    .collect(),
            );
        }
        if let Some(format) = openai.get("responseFormat").and_then(|v| v.as_str()) {
            opts.response_format = Some(format.to_string());
        }
        opts.chunking_strategy = openai.get("chunkingStrategy").and_then(chunking_strategy);
    }
    opts
}
