//! Static, secret-free package descriptors and `{ "env": ... }` references.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Reference to an environment variable carrying a credential or secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnvVarRef {
    pub env: String,
}

/// Configuration string accepting either a literal secret/value or an `{ "env": "VAR" }` reference.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
#[schemars(schema_with = "string_or_env_schema")]
pub enum StringOrEnv {
    Literal(String),
    Env(EnvVarRef),
}

impl StringOrEnv {
    /// Wrap an environment variable reference.
    #[must_use]
    pub fn env(name: impl Into<String>) -> Self {
        Self::Env(EnvVarRef { env: name.into() })
    }
}

impl From<String> for StringOrEnv {
    fn from(value: String) -> Self {
        Self::Literal(value)
    }
}

impl From<&str> for StringOrEnv {
    fn from(value: &str) -> Self {
        Self::Literal(value.to_owned())
    }
}

impl std::fmt::Debug for StringOrEnv {
    /// Never prints literal values; environment variable names are safe to display.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Literal(_) => f.debug_tuple("Literal").field(&"[REDACTED]").finish(),
            Self::Env(env_ref) => f.debug_tuple("Env").field(env_ref).finish(),
        }
    }
}

fn string_or_env_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
    let literal = generator.subschema_for::<String>();
    let env_ref = generator.subschema_for::<EnvVarRef>();
    schemars::json_schema!({
        "x-aimux-codec": "ordered-untagged",
        "anyOf": [literal, env_ref]
    })
}

/// Architectural category of a provider package.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum PackageKind {
    Native,
    Protocol,
    Preset,
    Specialized,
}

/// Authentication mechanism implemented by a provider package.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum PackageAuthKind {
    Bearer,
    Header,
    QueryParam,
    Basic,
    AwsSigV4,
    AwsBearerOrSigV4,
    AnthropicAws,
    GoogleVertex,
    AzureApiKeyOrToken,
    CodexChannel,
    OptionalBearer,
    None,
}

/// Model modality supported by a provider package or registry resolution call.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub enum ModalityKind {
    LanguageModel,
    EmbeddingModel,
    ImageModel,
    SpeechModel,
    TranscriptionModel,
    RerankingModel,
    VideoModel,
    SearchModel,
    #[serde(rename = "decision")]
    Decision,
}

/// Specific language-model constructor method exposed on a [`Provider`].
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub enum LanguageModelMethod {
    Chat,
    Responses,
    Completion,
    Messages,
}

/// Wire shape of a setting field accepted by a provider package.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum SettingFieldKind {
    String,
    Credential,
    Boolean,
    HeaderMap,
    StringMap,
    CodexMode,
}

/// Static description of one setting accepted when constructing a provider package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingFieldSpec {
    pub name: String,
    pub kind: SettingFieldKind,
    pub required: bool,
    pub credential: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env_var: Option<String>,
}

/// Static description of one URL template parameter required by a preset provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TemplateParamDescriptor {
    pub name: String,
    pub env_var: String,
}

/// Secret-free static metadata describing a provider package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageDescriptor {
    pub id: String,
    pub kind: PackageKind,
    pub auth_kind: PackageAuthKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url_env: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_base_url: Option<String>,
    pub supports_default_instance: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_method: Option<ModalityKind>,
    pub modalities: Vec<ModalityKind>,
    pub language_methods: Vec<LanguageModelMethod>,
    pub supports_discovery: bool,
    pub supports_files: bool,
    pub supports_skills: bool,
    pub settings: Vec<SettingFieldSpec>,
    pub template_params: Vec<TemplateParamDescriptor>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_or_env_redacts_literal_debug_and_parses_both_variants() {
        let literal: StringOrEnv = serde_json::from_str("\"sk-secret-value\"").unwrap();
        assert_eq!(literal, StringOrEnv::Literal("sk-secret-value".into()));
        let rendered = format!("{literal:?}");
        assert!(!rendered.contains("sk-secret-value"));
        assert!(rendered.contains("[REDACTED]"));

        let env_ref: StringOrEnv = serde_json::from_str("{\"env\":\"OPENAI_API_KEY\"}").unwrap();
        assert_eq!(env_ref, StringOrEnv::env("OPENAI_API_KEY"));
        let env_debug = format!("{env_ref:?}");
        assert!(env_debug.contains("OPENAI_API_KEY"));

        assert!(
            serde_json::from_str::<StringOrEnv>("{\"env\":\"OPENAI_API_KEY\",\"extra\":1}")
                .is_err()
        );
    }

    #[test]
    fn string_or_env_schema_uses_ordered_untagged_codec() {
        let schema = serde_json::to_value(schemars::schema_for!(StringOrEnv)).unwrap();
        assert_eq!(schema["x-aimux-codec"], "ordered-untagged");
        assert!(schema["anyOf"].is_array());
    }
}
