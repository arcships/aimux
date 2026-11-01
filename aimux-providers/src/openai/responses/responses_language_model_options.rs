//! The provider-options namespace and profile of the Responses model
//! (openai-responses-language-model-options.ts).

use aimux_core::shared::{JsonObject, SharedProviderOptions};
use aimux_provider_utils::optional_field;
use serde::{Deserialize, Deserializer};
use serde_json::Value;

/// The providerOptions keys a Responses model reads, in order of precedence
/// (the first one present wins as a whole), and the key it writes response
/// metadata under. The AI SDK's `OpenAIResponsesLanguageModel` picks
/// `providerOptionsName` from the host: `openai` for the OpenAI API, `azure`
/// for Azure (which still falls back to `openai` when no `azure` options were
/// given).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResponsesNamespace {
    read: &'static [&'static str],
    write: &'static str,
}

impl ResponsesNamespace {
    /// The OpenAI API: reads and writes `openai`.
    pub const OPENAI: Self = Self::new(&["openai"], "openai");

    /// A host namespace. `read` is non-empty and in order of precedence;
    /// `write` is the metadata key.
    #[must_use]
    pub const fn new(read: &'static [&'static str], write: &'static str) -> Self {
        Self { read, write }
    }

    /// The key response metadata is written under.
    #[must_use]
    pub fn write_key(self) -> &'static str {
        self.write
    }

    /// The first present options object among the read keys.
    pub(crate) fn find(self, provider_options: &SharedProviderOptions) -> Option<&JsonObject> {
        self.read.iter().find_map(|key| provider_options.get(*key))
    }

    /// [`find`](Self::find) for `CallOptions::provider_options`.
    pub(crate) fn find_in(self, provider_options: &SharedProviderOptions) -> Option<&JsonObject> {
        self.read.iter().find_map(|key| provider_options.get(*key))
    }

    /// `parseProviderOptions` with `openaiLanguageModelResponsesOptionsSchema`
    /// under the read keys in turn (openai-responses-language-model.ts:324-339):
    /// the first namespace present must match the schema. The request reads
    /// that namespace as given ([`find_in`](Self::find_in)).
    ///
    /// # Errors
    ///
    /// `InvalidArgument` "invalid <namespace> provider options" when it does
    /// not, with the serde error as its cause.
    pub(crate) fn validate(
        self,
        provider_options: Option<&SharedProviderOptions>,
    ) -> Result<(), aimux_core::AiMuxError> {
        for key in self.read {
            if aimux_provider_utils::parse_provider_options::<ResponsesOptions>(
                key,
                provider_options,
            )?
            .is_some()
            {
                break;
            }
        }
        Ok(())
    }
}

/// `openaiLanguageModelResponsesOptionsSchema`. `.optional()` keys fail on
/// `null`; `.nullish()` keys read it as absent. `metadata` (`z.any()`) is not
/// checked. Only checked: the request reads the namespace as given.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(
    dead_code,
    reason = "validated only; the request reads the raw namespace"
)]
struct ResponsesOptions {
    #[serde(default)]
    conversation: Option<String>,
    #[serde(default)]
    include: Option<Vec<Include>>,
    #[serde(default, deserialize_with = "optional_field")]
    include_web_search_sources: Option<bool>,
    #[serde(default)]
    instructions: Option<String>,
    /// `z.union([z.boolean(), z.number().min(1).max(TOP_LOGPROBS_MAX)])`.
    #[serde(default, deserialize_with = "logprobs")]
    logprobs: Option<Value>,
    #[serde(default)]
    max_tool_calls: Option<serde_json::Number>,
    #[serde(default)]
    parallel_tool_calls: Option<bool>,
    #[serde(default)]
    previous_response_id: Option<String>,
    #[serde(default)]
    prompt_cache_key: Option<String>,
    #[serde(default, deserialize_with = "optional_field")]
    prompt_cache_options: Option<PromptCacheOptions>,
    #[serde(default)]
    prompt_cache_retention: Option<PromptCacheRetention>,
    #[serde(default)]
    reasoning_effort: Option<String>,
    #[serde(default, deserialize_with = "optional_field")]
    reasoning_effort_update: Option<ReasoningEffortUpdate>,
    #[serde(default, deserialize_with = "optional_field")]
    reasoning_mode: Option<ReasoningMode>,
    #[serde(default, deserialize_with = "optional_field")]
    reasoning_context: Option<ReasoningContext>,
    #[serde(default)]
    reasoning_summary: Option<String>,
    #[serde(default)]
    safety_identifier: Option<String>,
    #[serde(default)]
    service_tier: Option<ServiceTier>,
    #[serde(default)]
    store: Option<bool>,
    #[serde(default, deserialize_with = "optional_field")]
    pass_through_unsupported_files: Option<bool>,
    #[serde(default)]
    strict_json_schema: Option<bool>,
    #[serde(default)]
    text_verbosity: Option<TextVerbosity>,
    #[serde(default)]
    truncation: Option<Truncation>,
    #[serde(default)]
    user: Option<String>,
    #[serde(default, deserialize_with = "optional_field")]
    system_message_mode: Option<SystemMessageMode>,
    #[serde(default, deserialize_with = "optional_field")]
    force_reasoning: Option<bool>,
    #[serde(default)]
    context_management: Option<Vec<ContextManagement>>,
    #[serde(default, deserialize_with = "optional_field")]
    compaction_trigger: Option<bool>,
    #[serde(default, deserialize_with = "optional_field")]
    allowed_tools: Option<AllowedTools>,
}

#[derive(Debug, Deserialize)]
enum Include {
    #[serde(rename = "reasoning.encrypted_content")]
    ReasoningEncryptedContent,
    #[serde(rename = "file_search_call.results")]
    FileSearchCallResults,
    #[serde(rename = "web_search_call.results")]
    WebSearchCallResults,
    #[serde(rename = "message.output_text.logprobs")]
    MessageOutputTextLogprobs,
}

#[derive(Debug, Deserialize)]
#[allow(
    dead_code,
    reason = "validated only; the request reads the raw namespace"
)]
struct PromptCacheOptions {
    #[serde(default, deserialize_with = "optional_field")]
    mode: Option<PromptCacheMode>,
    #[serde(default, deserialize_with = "optional_field")]
    ttl: Option<PromptCacheTtl>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum PromptCacheMode {
    Implicit,
    Explicit,
}

/// `z.literal('30m')`.
#[derive(Debug, Deserialize)]
enum PromptCacheTtl {
    #[serde(rename = "30m")]
    ThirtyMinutes,
}

#[derive(Debug, Deserialize)]
enum PromptCacheRetention {
    #[serde(rename = "in_memory")]
    InMemory,
    #[serde(rename = "24h")]
    Day,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ReasoningEffortUpdate {
    None,
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ReasoningMode {
    Standard,
    Pro,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ReasoningContext {
    Auto,
    CurrentTurn,
    AllTurns,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ServiceTier {
    Auto,
    Flex,
    Priority,
    Fast,
    Ultrafast,
    Default,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum TextVerbosity {
    Low,
    Medium,
    High,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Truncation {
    Auto,
    Disabled,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum SystemMessageMode {
    System,
    Developer,
    Remove,
}

/// `{ type: 'compaction', compactThreshold: number }`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(
    dead_code,
    reason = "validated only; the request reads the raw namespace"
)]
struct ContextManagement {
    #[serde(rename = "type")]
    kind: Compaction,
    compact_threshold: serde_json::Number,
}

/// `z.literal('compaction')`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Compaction {
    Compaction,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(
    dead_code,
    reason = "validated only; the request reads the raw namespace"
)]
struct AllowedTools {
    /// `z.array(z.string()).min(1)`.
    #[serde(deserialize_with = "tool_names")]
    tool_names: Vec<String>,
    #[serde(default, deserialize_with = "optional_field")]
    mode: Option<AllowedToolsMode>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum AllowedToolsMode {
    Auto,
    Required,
}

/// `logprobs`: a boolean, or a number from 1 to `TOP_LOGPROBS_MAX` (20);
/// `.optional()`, so `null` fails.
fn logprobs<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Value>, D::Error> {
    let value = Value::deserialize(deserializer)?;
    let valid = value.is_boolean()
        || value
            .as_f64()
            .is_some_and(|number| (1.0..=20.0).contains(&number));
    if valid {
        Ok(Some(value))
    } else {
        Err(serde::de::Error::custom(
            "expected a boolean or a number from 1 to 20",
        ))
    }
}

/// `toolNames`: at least one name.
fn tool_names<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
    let names = Vec::<String>::deserialize(deserializer)?;
    if names.is_empty() {
        return Err(serde::de::Error::custom("expected at least one tool name"));
    }
    Ok(names)
}

impl Default for ResponsesNamespace {
    fn default() -> Self {
        Self::OPENAI
    }
}

/// How a host differs in the Responses API model: its namespace, and the
/// prefixes that mark a file part's data as an uploaded file id instead of
/// content to inline (`fileIdPrefixes` in the AI SDK).
#[derive(Debug, Clone, Default)]
pub(crate) struct ResponsesProfile {
    pub(crate) namespace: ResponsesNamespace,
    /// Empty: no file part is treated as a file id.
    pub(crate) file_id_prefixes: Vec<&'static str>,
    pub(crate) explicit_message_item_type: bool,
}

/// Get a value from the host namespace's options (`openai.<key>` for OpenAI).
pub(crate) fn openai_option(
    ns: ResponsesNamespace,
    options: &Option<SharedProviderOptions>,
    key: &str,
) -> Option<Value> {
    options
        .as_ref()
        .and_then(|m| ns.find_in(m))
        .and_then(|o| o.get(key))
        .cloned()
}
