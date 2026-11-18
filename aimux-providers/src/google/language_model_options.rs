//! `google-language-model-options.ts`: the `providerOptions.google` schema
//! (`googleLanguageModelOptions`), checked before a request is built.

use aimux_core::error::AiMuxError;
use aimux_core::shared::SharedProviderOptions;
use aimux_provider_utils::optional_field;
use serde::Deserialize;

use super::options::Namespace;

/// `googleLanguageModelOptions`. Every key is `.optional()`: absent is
/// `None`, `null` fails. Only checked: the request is built from the
/// namespace as given.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(
    dead_code,
    reason = "validated only; the request is built from the raw namespace"
)]
struct GoogleLanguageModelOptions {
    #[serde(default, deserialize_with = "optional_field")]
    response_modalities: Option<Vec<ResponseModality>>,
    #[serde(default, deserialize_with = "optional_field")]
    thinking_config: Option<ThinkingConfig>,
    #[serde(default, deserialize_with = "optional_field")]
    cached_content: Option<String>,
    #[serde(default, deserialize_with = "optional_field")]
    structured_outputs: Option<bool>,
    #[serde(default, deserialize_with = "optional_field")]
    safety_settings: Option<Vec<SafetySetting>>,
    #[serde(default, deserialize_with = "optional_field")]
    threshold: Option<HarmBlockThreshold>,
    #[serde(default, deserialize_with = "optional_field")]
    audio_timestamp: Option<bool>,
    #[serde(default, deserialize_with = "optional_field")]
    labels: Option<indexmap::IndexMap<String, String>>,
    #[serde(default, deserialize_with = "optional_field")]
    media_resolution: Option<MediaResolution>,
    #[serde(default, deserialize_with = "optional_field")]
    image_config: Option<ImageConfig>,
    #[serde(default, deserialize_with = "optional_field")]
    retrieval_config: Option<RetrievalConfig>,
    #[serde(default, deserialize_with = "optional_field")]
    stream_function_call_arguments: Option<bool>,
    #[serde(default, deserialize_with = "optional_field")]
    service_tier: Option<ServiceTier>,
    #[serde(default, deserialize_with = "optional_field")]
    shared_request_type: Option<ServiceTier>,
    #[serde(default, deserialize_with = "optional_field")]
    request_type: Option<RequestType>,
}

/// `z.enum(['TEXT', 'IMAGE'])`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
enum ResponseModality {
    Text,
    Image,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(
    dead_code,
    reason = "validated only; the request is built from the raw namespace"
)]
struct ThinkingConfig {
    /// `z.number()`.
    #[serde(default, deserialize_with = "optional_field")]
    thinking_budget: Option<serde_json::Number>,
    #[serde(default, deserialize_with = "optional_field")]
    include_thoughts: Option<bool>,
    #[serde(default, deserialize_with = "optional_field")]
    thinking_level: Option<ThinkingLevel>,
}

/// `z.enum(['minimal', 'low', 'medium', 'high'])`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ThinkingLevel {
    Minimal,
    Low,
    Medium,
    High,
}

#[derive(Debug, Deserialize)]
#[allow(
    dead_code,
    reason = "validated only; the request is built from the raw namespace"
)]
struct SafetySetting {
    category: HarmCategory,
    threshold: HarmBlockThreshold,
}

#[derive(Debug, Deserialize)]
enum HarmCategory {
    #[serde(rename = "HARM_CATEGORY_UNSPECIFIED")]
    Unspecified,
    #[serde(rename = "HARM_CATEGORY_HATE_SPEECH")]
    HateSpeech,
    #[serde(rename = "HARM_CATEGORY_DANGEROUS_CONTENT")]
    DangerousContent,
    #[serde(rename = "HARM_CATEGORY_HARASSMENT")]
    Harassment,
    #[serde(rename = "HARM_CATEGORY_SEXUALLY_EXPLICIT")]
    SexuallyExplicit,
    #[serde(rename = "HARM_CATEGORY_CIVIC_INTEGRITY")]
    CivicIntegrity,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum HarmBlockThreshold {
    #[serde(rename = "HARM_BLOCK_THRESHOLD_UNSPECIFIED")]
    Unspecified,
    BlockLowAndAbove,
    BlockMediumAndAbove,
    BlockOnlyHigh,
    BlockNone,
    Off,
}

#[derive(Debug, Deserialize)]
enum MediaResolution {
    #[serde(rename = "MEDIA_RESOLUTION_UNSPECIFIED")]
    Unspecified,
    #[serde(rename = "MEDIA_RESOLUTION_LOW")]
    Low,
    #[serde(rename = "MEDIA_RESOLUTION_MEDIUM")]
    Medium,
    #[serde(rename = "MEDIA_RESOLUTION_HIGH")]
    High,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(
    dead_code,
    reason = "validated only; the request is built from the raw namespace"
)]
struct ImageConfig {
    #[serde(default, deserialize_with = "optional_field")]
    aspect_ratio: Option<AspectRatio>,
    #[serde(default, deserialize_with = "optional_field")]
    image_size: Option<ImageSize>,
    #[serde(default, deserialize_with = "optional_field")]
    person_generation: Option<PersonGeneration>,
    #[serde(default, deserialize_with = "optional_field")]
    prominent_people: Option<ProminentPeople>,
    #[serde(default, deserialize_with = "optional_field")]
    image_output_options: Option<ImageOutputOptions>,
}

#[derive(Debug, Deserialize)]
enum AspectRatio {
    #[serde(rename = "1:1")]
    R1x1,
    #[serde(rename = "2:3")]
    R2x3,
    #[serde(rename = "3:2")]
    R3x2,
    #[serde(rename = "3:4")]
    R3x4,
    #[serde(rename = "4:3")]
    R4x3,
    #[serde(rename = "4:5")]
    R4x5,
    #[serde(rename = "5:4")]
    R5x4,
    #[serde(rename = "9:16")]
    R9x16,
    #[serde(rename = "16:9")]
    R16x9,
    #[serde(rename = "21:9")]
    R21x9,
    #[serde(rename = "1:8")]
    R1x8,
    #[serde(rename = "8:1")]
    R8x1,
    #[serde(rename = "1:4")]
    R1x4,
    #[serde(rename = "4:1")]
    R4x1,
}

#[derive(Debug, Deserialize)]
enum ImageSize {
    #[serde(rename = "1K")]
    OneK,
    #[serde(rename = "2K")]
    TwoK,
    #[serde(rename = "4K")]
    FourK,
    #[serde(rename = "512")]
    Px512,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum PersonGeneration {
    #[serde(rename = "PERSON_GENERATION_UNSPECIFIED")]
    Unspecified,
    AllowAll,
    AllowAdult,
    AllowNone,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum ProminentPeople {
    #[serde(rename = "PROMINENT_PEOPLE_UNSPECIFIED")]
    Unspecified,
    #[serde(rename = "ALLOW_PROMINENT_PEOPLE")]
    Allow,
    #[serde(rename = "BLOCK_PROMINENT_PEOPLE")]
    Block,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(
    dead_code,
    reason = "validated only; the request is built from the raw namespace"
)]
struct ImageOutputOptions {
    #[serde(default, deserialize_with = "optional_field")]
    mime_type: Option<ImageMimeType>,
    /// `z.number()`.
    #[serde(default, deserialize_with = "optional_field")]
    compression_quality: Option<serde_json::Number>,
}

#[derive(Debug, Deserialize)]
enum ImageMimeType {
    #[serde(rename = "image/jpeg")]
    Jpeg,
    #[serde(rename = "image/png")]
    Png,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(
    dead_code,
    reason = "validated only; the request is built from the raw namespace"
)]
struct RetrievalConfig {
    #[serde(default, deserialize_with = "optional_field")]
    lat_lng: Option<LatLng>,
}

#[derive(Debug, Deserialize)]
#[allow(
    dead_code,
    reason = "validated only; the request is built from the raw namespace"
)]
struct LatLng {
    latitude: serde_json::Number,
    longitude: serde_json::Number,
}

/// `serviceTier` and `sharedRequestType`: `standard`, `flex` or `priority`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ServiceTier {
    Standard,
    Flex,
    Priority,
}

/// `z.enum(['shared'])`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum RequestType {
    Shared,
}

/// `parseProviderOptions` with `googleLanguageModelOptions` under the
/// namespace's read keys in turn (`google`; for Vertex `googleVertex`, then
/// `vertex`, then `google`; google-language-model.ts:173-191).
///
/// # Errors
///
/// `InvalidArgument` "invalid <namespace> provider options" when the first
/// namespace present does not match the schema.
pub(super) fn validate_provider_options(
    namespace: Namespace,
    provider_options: Option<&SharedProviderOptions>,
) -> Result<(), AiMuxError> {
    namespace.parse::<GoogleLanguageModelOptions>(provider_options)?;
    Ok(())
}
