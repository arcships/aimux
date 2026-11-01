//! Settings and provider options of the Amazon Bedrock chat model
//! (amazon-bedrock-chat-language-model-options.ts).

use aimux_provider_utils::optional_field;
use serde::Deserialize;
use serde_json::{Map, Value};

/// The chat family override supported by the upstream provider.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AmazonBedrockChatModelFamily {
    Anthropic,
}

impl AmazonBedrockChatModelFamily {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
        }
    }
}

/// Optional family override for opaque inference-profile IDs.
#[derive(Clone, Debug, Default)]
pub struct AmazonBedrockChatModelSettings {
    pub model_family: Option<AmazonBedrockChatModelFamily>,
}

/// `amazonBedrockLanguageModelChatOptions`. Only checked here: the request
/// body is built from the namespace as given.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(
    dead_code,
    reason = "parsed for its validation; the body reads the raw namespace"
)]
struct ChatOptions {
    #[serde(default, deserialize_with = "optional_field")]
    structured_output_mode: Option<StructuredOutputMode>,
    #[serde(default, deserialize_with = "optional_field")]
    additional_model_request_fields: Option<Map<String, Value>>,
    #[serde(default, deserialize_with = "optional_field")]
    reasoning_config: Option<ReasoningConfig>,
    #[serde(default, deserialize_with = "optional_field")]
    anthropic_beta: Option<Vec<String>>,
    #[serde(default, deserialize_with = "optional_field")]
    service_tier: Option<ServiceTier>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
enum StructuredOutputMode {
    OutputFormat,
    JsonTool,
    Auto,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ServiceTier {
    Reserved,
    Priority,
    Default,
    Flex,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(
    dead_code,
    reason = "parsed for its validation; the body reads the raw namespace"
)]
struct ReasoningConfig {
    #[serde(default, rename = "type", deserialize_with = "optional_field")]
    kind: Option<ReasoningType>,
    #[serde(default, deserialize_with = "optional_field")]
    budget_tokens: Option<serde_json::Number>,
    #[serde(default, deserialize_with = "optional_field")]
    max_reasoning_effort: Option<MaxReasoningEffort>,
    #[serde(default, deserialize_with = "optional_field")]
    display: Option<ReasoningDisplay>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ReasoningType {
    Enabled,
    Disabled,
    Adaptive,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum MaxReasoningEffort {
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ReasoningDisplay {
    Omitted,
    Summarized,
}

/// Check the Bedrock namespace of `provider_options` against
/// `amazonBedrockLanguageModelChatOptions`
/// (amazon-bedrock-chat-language-model.ts:156-167).
///
/// # Errors
///
/// `InvalidArgument` "invalid amazonBedrock provider options" (or `bedrock`)
/// when it does not match.
pub(super) fn validate_options(
    provider_options: Option<&aimux_core::shared::SharedProviderOptions>,
) -> Result<(), aimux_core::AiMuxError> {
    super::options::parse::<ChatOptions>(provider_options).map(|_| ())
}

/// `amazonBedrockFilePartProviderOptions`: the options of a document or
/// text file part, and of a file in a tool result.
#[derive(Debug, Deserialize)]
#[allow(
    dead_code,
    reason = "validated only; the converter reads the raw namespace"
)]
pub(super) struct FilePartOptions {
    #[serde(default, deserialize_with = "optional_field")]
    citations: Option<Citations>,
}

/// `citations`: `z.object({ enabled: z.boolean() })`.
#[derive(Debug, Deserialize)]
#[allow(
    dead_code,
    reason = "validated only; the converter reads the raw namespace"
)]
struct Citations {
    enabled: bool,
}

/// `amazonBedrockTextPartProviderOptions`: the options of a user text part.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(
    dead_code,
    reason = "validated only; the converter reads the raw namespace"
)]
pub(super) struct TextPartOptions {
    #[serde(default, deserialize_with = "optional_field")]
    guard_content: Option<bool>,
    #[serde(default, deserialize_with = "optional_field")]
    guard_content_qualifiers: Option<Vec<GuardContentQualifier>>,
}

/// `guardContentQualifiers` items.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum GuardContentQualifier {
    GroundingSource,
    Query,
    GuardContent,
}

/// `amazonBedrockImagePartProviderOptions`: the options of a user image part.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(
    dead_code,
    reason = "validated only; the converter reads the raw namespace"
)]
pub(super) struct ImagePartOptions {
    #[serde(default, deserialize_with = "optional_field")]
    guard_content: Option<bool>,
}

/// `amazonBedrockReasoningMetadataSchema`
/// (amazon-bedrock-reasoning-metadata.ts): the options of an assistant
/// reasoning part.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(
    dead_code,
    reason = "validated only; the converter reads the raw namespace"
)]
pub(super) struct ReasoningMetadata {
    #[serde(default, deserialize_with = "optional_field")]
    signature: Option<String>,
    #[serde(default, deserialize_with = "optional_field")]
    redacted_data: Option<String>,
    #[serde(default, deserialize_with = "optional_field")]
    redacted_content: Option<String>,
}
