//! Unified configuration-driven provider construction and policy-gated `{ "env": ... }` resolution.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use aimux_core::error::AiMuxError;
use aimux_core::provider::Provider;
use aimux_provider_utils::{
    AwsCredentials, FetchFunction, Resolvable, StringOrEnv, package_descriptor::EnvVarRef,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::descriptors::package_descriptor;
use crate::preset::{self, PresetSettings};

/// Host policy controlling which environment variables may be read for `{ "env": "VAR" }` fields.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum EnvLookupPolicy {
    /// Any syntactically valid environment variable may be read (trusted local host default).
    #[default]
    AllowAll,
    /// Only explicitly allowlisted environment variables may be read.
    AllowList(BTreeSet<String>),
    /// Disallow all `{ "env": ... }` lookups (e.g. untrusted remote requests).
    DenyAll,
}

impl EnvLookupPolicy {
    /// Construct an allowlist policy from variable names.
    #[must_use]
    pub fn allow_list<'a>(vars: impl IntoIterator<Item = &'a str>) -> Self {
        Self::AllowList(vars.into_iter().map(str::to_owned).collect())
    }
}

/// Access mode for the `codex` package in declarative configuration.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum CodexModeSpec {
    ApiKey {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        api_key: Option<StringOrEnv>,
    },
    ChatGptAccount {
        token: StringOrEnv,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        account_id: Option<String>,
    },
}

impl std::fmt::Debug for CodexModeSpec {
    /// Never prints literal tokens or keys.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ApiKey { api_key } => f.debug_struct("ApiKey").field("api_key", api_key).finish(),
            Self::ChatGptAccount { token, account_id } => f
                .debug_struct("ChatGptAccount")
                .field("token", token)
                .field("account_id", account_id)
                .finish(),
        }
    }
}

/// Secret-safe declarative settings shared across provider packages.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderConfigSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<StringOrEnv>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_token: Option<StringOrEnv>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_token: Option<StringOrEnv>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_key_id: Option<StringOrEnv>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_access_key: Option<StringOrEnv>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_token: Option<StringOrEnv>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login: Option<StringOrEnv>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<StringOrEnv>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_mode: Option<CodexModeSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organization: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speech_base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub originator: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cx: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probability_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub use_deployment_based_urls: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_usage: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_structured_outputs: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_multi_part_tool_content: Option<bool>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub headers: HashMap<String, Option<String>>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub query_params: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub params: HashMap<String, String>,
}

impl std::fmt::Debug for ProviderConfigSettings {
    /// Never prints secret literals or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderConfigSettings")
            .field("api_key", &self.api_key)
            .field("auth_token", &self.auth_token)
            .field("access_token", &self.access_token)
            .field("access_key_id", &self.access_key_id)
            .field("secret_access_key", &self.secret_access_key)
            .field("session_token", &self.session_token)
            .field("login", &self.login)
            .field("password", &self.password)
            .field("codex_mode", &self.codex_mode)
            .field("base_url", &self.base_url)
            .field("name", &self.name)
            .field("organization", &self.organization)
            .field("project", &self.project)
            .field("location", &self.location)
            .field("region", &self.region)
            .field("resource_name", &self.resource_name)
            .field("speech_base_url", &self.speech_base_url)
            .field("api_version", &self.api_version)
            .field("workspace_id", &self.workspace_id)
            .field("originator", &self.originator)
            .field("cx", &self.cx)
            .field("endpoint", &self.endpoint)
            .field("probability_source", &self.probability_source)
            .field("use_deployment_based_urls", &self.use_deployment_based_urls)
            .field("include_usage", &self.include_usage)
            .field(
                "supports_structured_outputs",
                &self.supports_structured_outputs,
            )
            .field(
                "supports_multi_part_tool_content",
                &self.supports_multi_part_tool_content,
            )
            .field("headers", &self.headers.len())
            .field("query_params", &self.query_params)
            .field("params", &self.params)
            .finish()
    }
}

/// Configuration for a single provider entry (`package` + typed `settings`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderEntrySpec {
    pub package: String,
    #[serde(default)]
    pub settings: ProviderConfigSettings,
}

/// Declarative provider registry configuration document.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderConfigDocument {
    pub providers: BTreeMap<String, ProviderEntrySpec>,
}

fn is_valid_env_name(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().enumerate().all(|(idx, byte)| {
            if idx == 0 {
                byte.is_ascii_uppercase() || byte == b'_'
            } else {
                byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_'
            }
        })
}

fn resolve_string_or_env(
    value: &StringOrEnv,
    policy: &EnvLookupPolicy,
) -> Result<String, AiMuxError> {
    match value {
        StringOrEnv::Literal(literal) => Ok(literal.clone()),
        StringOrEnv::Env(EnvVarRef { env }) => {
            if !is_valid_env_name(env) {
                return Err(AiMuxError::invalid_argument(format!(
                    "invalid environment variable name '{env}'"
                )));
            }
            let permitted = match policy {
                EnvLookupPolicy::AllowAll => true,
                EnvLookupPolicy::AllowList(allowed) => allowed.contains(env),
                EnvLookupPolicy::DenyAll => false,
            };
            if !permitted {
                return Err(AiMuxError::invalid_argument(format!(
                    "environment variable '{env}' is not permitted by policy"
                )));
            }
            std::env::var(env).map_err(|_| AiMuxError::LoadApiKey {
                env_var: env.clone(),
                description: "provider configuration setting".to_string(),
            })
        }
    }
}

fn resolve_opt_cred(
    value: Option<&StringOrEnv>,
    policy: &EnvLookupPolicy,
) -> Result<Option<String>, AiMuxError> {
    value.map(|v| resolve_string_or_env(v, policy)).transpose()
}

fn populated_fields(settings: &ProviderConfigSettings) -> Vec<&'static str> {
    let mut fields = Vec::new();
    let checks = [
        (settings.api_key.is_some(), "apiKey"),
        (settings.auth_token.is_some(), "authToken"),
        (settings.access_token.is_some(), "accessToken"),
        (settings.access_key_id.is_some(), "accessKeyId"),
        (settings.secret_access_key.is_some(), "secretAccessKey"),
        (settings.session_token.is_some(), "sessionToken"),
        (settings.login.is_some(), "login"),
        (settings.password.is_some(), "password"),
        (settings.codex_mode.is_some(), "codexMode"),
        (settings.base_url.is_some(), "baseUrl"),
        (settings.name.is_some(), "name"),
        (settings.organization.is_some(), "organization"),
        (settings.project.is_some(), "project"),
        (settings.location.is_some(), "location"),
        (settings.region.is_some(), "region"),
        (settings.resource_name.is_some(), "resourceName"),
        (settings.speech_base_url.is_some(), "speechBaseUrl"),
        (settings.api_version.is_some(), "apiVersion"),
        (settings.workspace_id.is_some(), "workspaceId"),
        (settings.originator.is_some(), "originator"),
        (settings.cx.is_some(), "cx"),
        (settings.endpoint.is_some(), "endpoint"),
        (settings.probability_source.is_some(), "probabilitySource"),
        (
            settings.use_deployment_based_urls.is_some(),
            "useDeploymentBasedUrls",
        ),
        (settings.include_usage.is_some(), "includeUsage"),
        (
            settings.supports_structured_outputs.is_some(),
            "supportsStructuredOutputs",
        ),
        (
            settings.supports_multi_part_tool_content.is_some(),
            "supportsMultiPartToolContent",
        ),
        (!settings.headers.is_empty(), "headers"),
        (!settings.query_params.is_empty(), "queryParams"),
        (!settings.params.is_empty(), "params"),
    ];
    for (present, name) in checks {
        if present {
            fields.push(name);
        }
    }
    fields
}

fn validate_settings_for_package(
    package: &str,
    settings: &ProviderConfigSettings,
) -> Result<aimux_provider_utils::PackageDescriptor, AiMuxError> {
    let descriptor = package_descriptor(package).ok_or_else(|| {
        let available_providers = crate::descriptors::package_descriptors()
            .into_iter()
            .map(|d| d.id)
            .collect();
        AiMuxError::NoSuchProvider {
            provider_id: package.to_string(),
            model_id: package.to_string(),
            model_type: String::new(),
            available_providers,
        }
    })?;
    for field in populated_fields(settings) {
        if !descriptor.settings.iter().any(|spec| spec.name == field) {
            return Err(AiMuxError::invalid_argument(format!(
                "provider '{package}' does not accept setting '{field}'"
            )));
        }
    }
    Ok(descriptor)
}

fn build_anthropic_aws(
    settings: &ProviderConfigSettings,
    policy: &EnvLookupPolicy,
    fetch: Option<FetchFunction>,
) -> Result<Arc<dyn Provider>, AiMuxError> {
    let api_key = resolve_opt_cred(settings.api_key.as_ref(), policy)?;
    let access_key_id = resolve_opt_cred(settings.access_key_id.as_ref(), policy)?;
    let secret_access_key = resolve_opt_cred(settings.secret_access_key.as_ref(), policy)?;
    let session_token = resolve_opt_cred(settings.session_token.as_ref(), policy)?;
    let has_sigv4 =
        access_key_id.is_some() || secret_access_key.is_some() || session_token.is_some();
    if api_key.is_some() && has_sigv4 {
        return Err(AiMuxError::invalid_argument(
            "provider 'anthropic_aws' accepts either `apiKey` or AWS SigV4 credentials, not both",
        ));
    }
    let auth = match (api_key, access_key_id, secret_access_key) {
        (Some(key), None, None) => Some(crate::anthropic_aws::AnthropicAwsAuth::ApiKey(
            Resolvable::Value(key),
        )),
        (None, Some(access_key_id), Some(secret_access_key)) => Some(
            crate::anthropic_aws::AnthropicAwsAuth::SigV4(Resolvable::Value(AwsCredentials {
                access_key_id,
                secret_access_key,
                session_token,
                region: settings.region.clone().unwrap_or_default(),
            })),
        ),
        (None, None, None) => None,
        _ => {
            return Err(AiMuxError::invalid_argument(
                "provider 'anthropic_aws' SigV4 configuration requires both `accessKeyId` and `secretAccessKey`",
            ));
        }
    };
    let headers = (!settings.headers.is_empty()).then(|| settings.headers.clone());
    Ok(Arc::new(crate::anthropic_aws::create_anthropic_aws(
        crate::anthropic_aws::AnthropicAwsProviderSettings {
            base_url: settings.base_url.clone(),
            region: settings.region.clone(),
            auth,
            workspace_id: settings.workspace_id.clone(),
            headers,
            name: settings.name.clone(),
            fetch,
            ..Default::default()
        },
    )?))
}

fn build_codex(
    settings: &ProviderConfigSettings,
    policy: &EnvLookupPolicy,
    fetch: Option<FetchFunction>,
) -> Result<Arc<dyn Provider>, AiMuxError> {
    if settings.codex_mode.is_some() && settings.api_key.is_some() {
        return Err(AiMuxError::invalid_argument(
            "provider 'codex' accepts either `codexMode` or `apiKey`, not both",
        ));
    }
    let mode = match &settings.codex_mode {
        Some(CodexModeSpec::ApiKey { api_key }) => {
            let key = resolve_opt_cred(api_key.as_ref(), policy)?;
            crate::codex::CodexMode::ApiKey(key.map(Resolvable::Value))
        }
        Some(CodexModeSpec::ChatGptAccount { token, account_id }) => {
            let token = resolve_string_or_env(token, policy)?;
            crate::codex::CodexMode::ChatGptAccount {
                token: Resolvable::Value(token),
                account_id: account_id.clone(),
            }
        }
        None => match resolve_opt_cred(settings.api_key.as_ref(), policy)? {
            Some(key) => crate::codex::CodexMode::ApiKey(Some(Resolvable::Value(key))),
            None => crate::codex::CodexMode::default(),
        },
    };
    let headers = (!settings.headers.is_empty()).then(|| settings.headers.clone());
    Ok(Arc::new(crate::codex::create_codex(
        crate::codex::CodexProviderSettings {
            mode,
            base_url: settings.base_url.clone(),
            headers,
            name: settings.name.clone(),
            originator: settings.originator.clone(),
            fetch,
            ..Default::default()
        },
    )?))
}

fn build_jev(
    settings: &ProviderConfigSettings,
    policy: &EnvLookupPolicy,
    fetch: Option<FetchFunction>,
) -> Result<Arc<dyn Provider>, AiMuxError> {
    if fetch.is_some() {
        return Err(AiMuxError::invalid_argument(
            "provider 'jev' does not support custom fetch transport",
        ));
    }
    let mut config = match resolve_opt_cred(settings.api_key.as_ref(), policy)? {
        Some(key) => crate::jev::JevConfig::new(key),
        None => crate::jev::JevConfig::from_env()?,
    };
    if let Some(endpoint) = settings.endpoint.as_deref().filter(|s| !s.is_empty()) {
        config = config.with_endpoint(endpoint);
    }
    if let Some(source) = settings.probability_source.as_deref() {
        config.probability_source = source.parse()?;
    }
    config.headers = settings
        .headers
        .iter()
        .filter_map(|(k, v)| v.as_ref().map(|val| (k.clone(), val.clone())))
        .collect();
    Ok(Arc::new(crate::jev::JevProvider::new(config)))
}

fn build_native_provider(
    package: &str,
    settings: &ProviderConfigSettings,
    policy: &EnvLookupPolicy,
    fetch: Option<FetchFunction>,
) -> Result<Arc<dyn Provider>, AiMuxError> {
    let api_key = resolve_opt_cred(settings.api_key.as_ref(), policy)?;
    let headers = (!settings.headers.is_empty()).then(|| settings.headers.clone());
    let base_url = settings.base_url.clone();
    let name = settings.name.clone();

    macro_rules! named_resolvable {
        ($module:ident, $create:ident, $settings:ident) => {
            Ok(Arc::new(crate::$module::$create(
                #[allow(clippy::needless_update)]
                crate::$module::$settings {
                    api_key: api_key.map(Resolvable::Value),
                    base_url,
                    headers,
                    name,
                    fetch,
                    ..Default::default()
                },
            )?))
        };
    }

    macro_rules! named_string {
        ($module:ident, $create:ident, $settings:ident) => {
            Ok(Arc::new(crate::$module::$create(
                #[allow(clippy::needless_update)]
                crate::$module::$settings {
                    api_key,
                    base_url,
                    headers,
                    name,
                    fetch,
                    ..Default::default()
                },
            )?))
        };
    }

    macro_rules! unnamed_string {
        ($module:ident, $create:ident, $settings:ident) => {
            Ok(Arc::new(crate::$module::$create(
                #[allow(clippy::needless_update)]
                crate::$module::$settings {
                    api_key,
                    base_url,
                    headers,
                    fetch,
                    ..Default::default()
                },
            )?))
        };
    }

    match package {
        "anthropic_aws" => build_anthropic_aws(settings, policy, fetch),
        "codex" => build_codex(settings, policy, fetch),
        "openai" => Ok(Arc::new(crate::openai::create_openai(
            crate::openai::OpenAIProviderSettings {
                base_url,
                api_key,
                organization: settings.organization.clone(),
                project: settings.project.clone(),
                headers,
                name,
                fetch,
                ..Default::default()
            },
        )?)),
        "anthropic" => Ok(Arc::new(crate::anthropic::create_anthropic(
            crate::anthropic::AnthropicProviderSettings {
                base_url,
                api_key,
                auth_token: resolve_opt_cred(settings.auth_token.as_ref(), policy)?,
                headers,
                name,
                fetch,
                ..Default::default()
            },
        )?)),
        "azure" => Ok(Arc::new(crate::azure::create_azure(
            crate::azure::AzureOpenAIProviderSettings {
                resource_name: settings.resource_name.clone(),
                base_url,
                speech_base_url: settings.speech_base_url.clone(),
                api_key: api_key.map(Resolvable::Value),
                token_provider: resolve_opt_cred(settings.access_token.as_ref(), policy)?
                    .map(Resolvable::Value),
                headers,
                fetch,
                api_version: settings.api_version.clone(),
                use_deployment_based_urls: settings.use_deployment_based_urls.unwrap_or(false),
            },
        )?)),
        "amazon_bedrock" => Ok(Arc::new(crate::bedrock::create_amazon_bedrock(
            crate::bedrock::AmazonBedrockProviderSettings {
                region: settings.region.clone(),
                api_key: api_key.map(Resolvable::Value),
                access_key_id: resolve_opt_cred(settings.access_key_id.as_ref(), policy)?,
                secret_access_key: resolve_opt_cred(settings.secret_access_key.as_ref(), policy)?,
                session_token: resolve_opt_cred(settings.session_token.as_ref(), policy)?,
                base_url,
                headers,
                fetch,
                ..Default::default()
            },
        )?)),
        "aws_polly" => Ok(Arc::new(crate::aws_polly::create_aws_polly(
            crate::aws_polly::AwsPollyProviderSettings {
                region: settings.region.clone(),
                access_key_id: resolve_opt_cred(settings.access_key_id.as_ref(), policy)?,
                secret_access_key: resolve_opt_cred(settings.secret_access_key.as_ref(), policy)?,
                session_token: resolve_opt_cred(settings.session_token.as_ref(), policy)?,
                base_url,
                headers,
                name,
                fetch,
                ..Default::default()
            },
        )?)),
        "dataforseo" => Ok(Arc::new(crate::dataforseo::create_dataforseo(
            crate::dataforseo::DataforseoProviderSettings {
                base_url,
                login: resolve_opt_cred(settings.login.as_ref(), policy)?.map(Resolvable::Value),
                password: resolve_opt_cred(settings.password.as_ref(), policy)?
                    .map(Resolvable::Value),
                headers,
                name,
                fetch,
            },
        )?)),
        "google_pse" => Ok(Arc::new(crate::google_pse::create_google_pse(
            crate::google_pse::GooglePseProviderSettings {
                base_url,
                api_key: api_key.map(Resolvable::Value),
                cx: settings.cx.clone(),
                headers,
                name,
                fetch,
            },
        )?)),
        "google_vertex" => Ok(Arc::new(crate::vertex::create_google_vertex(
            crate::vertex::VertexProviderSettings {
                api_key: api_key.map(Resolvable::Value),
                location: settings.location.clone(),
                project: settings.project.clone(),
                base_url,
                headers: headers.map(Resolvable::Value),
                access_token: resolve_opt_cred(settings.access_token.as_ref(), policy)?
                    .map(Resolvable::Value),
                fetch,
                ..Default::default()
            },
        )?)),
        "cohere" => unnamed_string!(cohere, create_cohere, CohereProviderSettings),
        "deepseek" => unnamed_string!(deepseek, create_deepseek, DeepSeekProviderSettings),
        "google" => named_string!(google, create_google, GoogleProviderSettings),
        "groq" => unnamed_string!(groq, create_groq, GroqProviderSettings),
        "mistral" => unnamed_string!(mistral, create_mistral, MistralProviderSettings),
        "xai" => unnamed_string!(xai, create_xai, XAIProviderSettings),
        "assemblyai" => {
            named_resolvable!(assemblyai, create_assemblyai, AssemblyAIProviderSettings)
        }
        "black_forest_labs" => named_resolvable!(
            black_forest_labs,
            create_black_forest_labs,
            BlackForestLabsProviderSettings
        ),
        "cartesia" => named_resolvable!(cartesia, create_cartesia, CartesiaProviderSettings),
        "deepgram" => named_resolvable!(deepgram, create_deepgram, DeepgramProviderSettings),
        "elevenlabs" => {
            named_resolvable!(elevenlabs, create_elevenlabs, ElevenLabsProviderSettings)
        }
        "exa_ai" => named_resolvable!(exa_ai, create_exa_ai, ExaAiProviderSettings),
        "fal" => named_resolvable!(fal, create_fal, FalProviderSettings),
        "firecrawl" => named_resolvable!(firecrawl, create_firecrawl, FirecrawlProviderSettings),
        "gladia" => named_resolvable!(gladia, create_gladia, GladiaProviderSettings),
        "huggingface" => {
            named_resolvable!(huggingface, create_huggingface, HuggingFaceProviderSettings)
        }
        "hume" => named_resolvable!(hume, create_hume, HumeProviderSettings),
        "jina_ai" => named_resolvable!(jina_ai, create_jina_ai, JinaAiProviderSettings),
        "klingai" => named_resolvable!(klingai, create_klingai, KlingAIProviderSettings),
        "linkup" => named_resolvable!(linkup, create_linkup, LinkupProviderSettings),
        "lmnt" => named_resolvable!(lmnt, create_lmnt, LMNTProviderSettings),
        "luma" => named_resolvable!(luma, create_luma, LumaProviderSettings),
        "parallel_ai" => {
            named_resolvable!(parallel_ai, create_parallel_ai, ParallelAiProviderSettings)
        }
        "prodia" => named_resolvable!(prodia, create_prodia, ProdiaProviderSettings),
        "recraft" => named_resolvable!(recraft, create_recraft, RecraftProviderSettings),
        "replicate" => named_resolvable!(replicate, create_replicate, ReplicateProviderSettings),
        "revai" => named_resolvable!(revai, create_revai, RevaiProviderSettings),
        "runwayml" => named_resolvable!(runwayml, create_runwayml, RunwaymlProviderSettings),
        "searxng" => named_resolvable!(searxng, create_searxng, SearxngProviderSettings),
        "serper" => named_resolvable!(serper, create_serper, SerperProviderSettings),
        "stability" => named_resolvable!(stability, create_stability, StabilityProviderSettings),
        "tavily" => named_resolvable!(tavily, create_tavily, TavilyProviderSettings),
        "tinyfish" => named_resolvable!(tinyfish, create_tinyfish, TinyfishProviderSettings),
        "voyage" => named_resolvable!(voyage, create_voyage, VoyageProviderSettings),
        "you_com" => named_resolvable!(you_com, create_you_com, YouComProviderSettings),
        other => Err(AiMuxError::invalid_argument(format!(
            "unsupported native package '{other}'"
        ))),
    }
}

fn build_provider_with_key_and_fetch(
    spec: &ProviderEntrySpec,
    default_name: Option<&str>,
    policy: &EnvLookupPolicy,
    fetch: Option<FetchFunction>,
) -> Result<Arc<dyn Provider>, AiMuxError> {
    let descriptor = validate_settings_for_package(&spec.package, &spec.settings)?;
    match descriptor.kind {
        aimux_provider_utils::PackageKind::Protocol => {
            let name = spec
                .settings
                .name
                .as_deref()
                .or(default_name)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    AiMuxError::invalid_argument(
                        "provider 'openai_compatible' requires a non-empty `name` setting",
                    )
                })?
                .to_owned();
            let base_url = spec
                .settings
                .base_url
                .clone()
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| {
                    AiMuxError::invalid_argument(
                        "provider 'openai_compatible' requires a non-empty `baseUrl` setting",
                    )
                })?;
            let api_key = resolve_opt_cred(spec.settings.api_key.as_ref(), policy)?;
            let headers =
                (!spec.settings.headers.is_empty()).then(|| spec.settings.headers.clone());
            let query_params = (!spec.settings.query_params.is_empty())
                .then(|| spec.settings.query_params.clone());
            Ok(Arc::new(
                crate::openai_compatible::create_openai_compatible(
                    crate::openai_compatible::OpenAICompatibleProviderSettings {
                        name,
                        base_url,
                        api_key,
                        headers,
                        query_params,
                        fetch,
                        include_usage: spec.settings.include_usage,
                        supports_structured_outputs: spec.settings.supports_structured_outputs,
                        supports_multi_part_tool_content: spec
                            .settings
                            .supports_multi_part_tool_content,
                        ..Default::default()
                    },
                )?,
            ))
        }
        aimux_provider_utils::PackageKind::Specialized => build_jev(&spec.settings, policy, fetch),
        aimux_provider_utils::PackageKind::Preset => {
            let api_key = resolve_opt_cred(spec.settings.api_key.as_ref(), policy)?;
            let headers =
                (!spec.settings.headers.is_empty()).then(|| spec.settings.headers.clone());
            Ok(Arc::new(preset::create(
                &descriptor.id,
                PresetSettings {
                    api_key: api_key.map(Resolvable::Value),
                    base_url: spec.settings.base_url.clone(),
                    headers,
                    fetch,
                    params: spec.settings.params.clone(),
                    transform_request_body: None,
                },
            )?))
        }
        aimux_provider_utils::PackageKind::Native => {
            build_native_provider(&descriptor.id, &spec.settings, policy, fetch)
        }
    }
}

/// Construct one configured provider with optional transport override (used by `aimux-ops`).
///
/// # Errors
///
/// Returns `NoSuchProvider`, `InvalidArgument`, `LoadApiKey`, or factory errors.
pub fn provider_from_config_with_transport(
    spec: &ProviderEntrySpec,
    policy: &EnvLookupPolicy,
    fetch: Option<FetchFunction>,
) -> Result<Arc<dyn Provider>, AiMuxError> {
    build_provider_with_key_and_fetch(spec, None, policy, fetch)
}

/// Construct one provider from a declarative configuration entry with [`EnvLookupPolicy::AllowAll`].
///
/// # Errors
///
/// Returns `NoSuchProvider`, `InvalidArgument`, `LoadApiKey`, or factory errors.
pub fn provider_from_config(spec: &ProviderEntrySpec) -> Result<Arc<dyn Provider>, AiMuxError> {
    provider_from_config_with_policy(spec, &EnvLookupPolicy::AllowAll)
}

/// Construct one provider from a declarative configuration entry under `policy`.
///
/// # Errors
///
/// Returns `NoSuchProvider`, `InvalidArgument`, `LoadApiKey`, or factory errors.
pub fn provider_from_config_with_policy(
    spec: &ProviderEntrySpec,
    policy: &EnvLookupPolicy,
) -> Result<Arc<dyn Provider>, AiMuxError> {
    build_provider_with_key_and_fetch(spec, None, policy, None)
}

/// Purely assemble a provider map from a configuration document with [`EnvLookupPolicy::AllowAll`].
///
/// # Errors
///
/// Returns `InvalidArgument`, `NoSuchProvider`, `LoadApiKey`, or factory errors.
pub fn providers_from_config(
    document: &ProviderConfigDocument,
) -> Result<BTreeMap<String, Arc<dyn Provider>>, AiMuxError> {
    providers_from_config_with_policy(document, &EnvLookupPolicy::AllowAll)
}

/// Purely assemble a provider map from a configuration document under `policy`.
///
/// # Errors
///
/// Returns `InvalidArgument`, `NoSuchProvider`, `LoadApiKey`, or factory errors.
pub fn providers_from_config_with_policy(
    document: &ProviderConfigDocument,
    policy: &EnvLookupPolicy,
) -> Result<BTreeMap<String, Arc<dyn Provider>>, AiMuxError> {
    let mut providers = BTreeMap::new();
    for (key, spec) in &document.providers {
        if key.trim().is_empty() {
            return Err(AiMuxError::invalid_argument(
                "provider configuration key must not be empty",
            ));
        }
        let provider = build_provider_with_key_and_fetch(spec, Some(key.as_str()), policy, None)?;
        providers.insert(key.clone(), provider);
    }
    Ok(providers)
}
