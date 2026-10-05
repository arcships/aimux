//! By-name creation for the bindings.
//!
//! The Rust surface for creating providers is the factories,
//! [`create_provider`](crate::create_provider) and the registry. The FFI and
//! the Node / Python bindings still expose an older shape (a provider name
//! plus a JSON options object, and providers registered at runtime from a
//! JSON document); this module is that shape, implemented on
//! `create_provider`. It is not re-exported from the crate root and goes away
//! when the bindings mirror the Rust surface.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

use serde::Deserialize;
use serde_json::Value;

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;
use aimux_provider_utils::{Resolvable, load_api_key, validate_base_url};

use crate::openai_compatible::config::{BaseUrl, ChatDialect};
use crate::openai_compatible::{Assembly, ChatProfile, OpenAICompatibleProvider};
use crate::preset::PresetSettings;
use crate::shared::Credential;

/// Chat capabilities of an external provider entry, expressed as data
/// (RFC-0020). The `Default` impl and the per-field `#[serde(default =
/// "default_true")]` both yield `true` for the three `supports_*` flags,
/// matching the OpenAI-compatible baseline of the registry presets - so a
/// profile that is omitted entirely (or present but with individual fields
/// missing) stays at full capability.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct ProviderProfile {
    #[serde(default = "default_true")]
    pub supports_top_k: bool,
    #[serde(default = "default_true")]
    pub supports_tools: bool,
    #[serde(default = "default_true")]
    pub supports_response_format: bool,
    /// Streaming usage rides in `chunk[key].usage` instead of `chunk.usage`.
    pub stream_usage_key: Option<String>,
    /// `"max_tokens"` or `"max_completion_tokens"`: the only max-token key the
    /// endpoint accepts. Any other value is rejected at registration.
    pub max_tokens_key: Option<String>,
}

impl Default for ProviderProfile {
    fn default() -> Self {
        Self {
            supports_top_k: true,
            supports_tools: true,
            supports_response_format: true,
            stream_usage_key: None,
            max_tokens_key: None,
        }
    }
}

fn default_true() -> bool {
    true
}

impl ProviderProfile {
    /// The chat behavior this profile describes.
    fn chat_profile(&self) -> Result<ChatProfile, AiMuxError> {
        let max_tokens_key = match self.max_tokens_key.as_deref() {
            None => None,
            Some("max_tokens") => Some("max_tokens"),
            Some("max_completion_tokens") => Some("max_completion_tokens"),
            Some(other) => {
                return Err(AiMuxError::InvalidArgument(format!(
                    "profile.max_tokens_key must be \"max_tokens\" or \"max_completion_tokens\", \
                     got {other:?}"
                )));
            }
        };
        let mut dialect = ChatDialect::baseline();
        dialect.supports_top_k = self.supports_top_k;
        dialect.supports_tools = self.supports_tools;
        dialect.supports_response_format = self.supports_response_format;
        dialect.stream_usage_key = self.stream_usage_key.clone();
        dialect.max_tokens_key = max_tokens_key;
        Ok(ChatProfile {
            include_usage: true,
            supports_structured_outputs: true,
            supports_multi_part_tool_content: false,
            dialect,
        })
    }
}

/// Per-call construction options for [`provider`] (overrides the registry entry).
///
/// Deserializing a JSON object that carries `max_retries` or `body_overrides`
/// fails: retry is a call-level option (`CallOptions::max_retries`) and
/// request-body overrides are gone, so neither is silently ignored here. The
/// FFI `config_json` and the Node/Python configs reach this check and report it
/// as `InvalidArgument`.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "ProviderOptionsWire")]
pub struct ProviderOptions {
    /// Override the registry base URL.
    pub base_url: Option<String>,
    /// Extra headers merged into every request.
    pub headers: Option<HashMap<String, String>>,
    /// OpenAI organization ID (`OpenAI-Organization` header).
    pub organization: Option<String>,
    /// OpenAI project ID (`OpenAI-Project` header).
    pub project: Option<String>,
    /// Values of the preset's template parameters (`account_id`, `region`,
    /// `project`, ...); only the ones the registry row declares are accepted.
    pub params: Option<HashMap<String, String>>,
}

/// Wire shape of [`ProviderOptions`]: the same keys plus the two removed ones,
/// kept only so their presence can be reported instead of dropped.
#[derive(Deserialize)]
struct ProviderOptionsWire {
    base_url: Option<String>,
    headers: Option<HashMap<String, String>>,
    organization: Option<String>,
    project: Option<String>,
    params: Option<HashMap<String, String>>,
    max_retries: Option<Value>,
    body_overrides: Option<Value>,
}

impl TryFrom<ProviderOptionsWire> for ProviderOptions {
    type Error = String;

    fn try_from(wire: ProviderOptionsWire) -> Result<Self, Self::Error> {
        if wire.max_retries.is_some() {
            return Err(MAX_RETRIES_REJECTED.to_string());
        }
        if wire.body_overrides.is_some() {
            return Err(BODY_OVERRIDES_REJECTED.to_string());
        }
        Ok(Self {
            base_url: wire.base_url,
            headers: wire.headers,
            organization: wire.organization,
            project: wire.project,
            params: wire.params,
        })
    }
}

const MAX_RETRIES_REJECTED: &str = "`max_retries` is a call-level option: pass it in the call \
     options (`maxRetries`), not in the provider configuration";

const BODY_OVERRIDES_REJECTED: &str = "`body_overrides` is no longer supported: request-body \
     overrides were removed from the provider configuration and the call options";

/// Report a provider configuration that still carries `max_retries` or
/// `body_overrides`, for configuration shapes that are not [`ProviderOptions`]
/// JSON (the Node and Python `ProviderConfig`). Pass whether each key was
/// given.
///
/// # Errors
///
/// Returns [`AiMuxError::InvalidArgument`] naming the first removed key that
/// was given.
pub fn reject_removed_provider_options(
    max_retries: bool,
    body_overrides: bool,
) -> Result<(), AiMuxError> {
    if max_retries {
        return Err(AiMuxError::InvalidArgument(MAX_RETRIES_REJECTED.into()));
    }
    if body_overrides {
        return Err(AiMuxError::InvalidArgument(BODY_OVERRIDES_REJECTED.into()));
    }
    Ok(())
}

/// A language model by provider name: [`provider_handle`] then
/// `language_model(model_id)`.
///
/// # Errors
///
/// As [`provider_handle`], plus whatever the provider returns for the id.
pub fn provider(
    name: impl AsRef<str>,
    api_key: Option<String>,
    model_id: &str,
    options: Option<ProviderOptions>,
) -> Result<Arc<dyn LanguageModel>, AiMuxError> {
    let p = provider_handle(name, api_key, options)?;
    p.language_model(model_id)
}

// ── Runtime overlay layer (RFC-0020) ─────────────────────────────────────────

/// A provider entry registered at runtime via [`register_provider`] /
/// [`load_providers_from_json`]. Overrides a same-named built-in entry (whole
/// replacement, not deep merge) or adds a new one.
///
/// Only OpenAI-compatible providers can be registered this way - native
/// protocols (anthropic/google/bedrock...) are code implementations and cannot
/// be described by config data. The JSON form accepts exactly these fields;
/// `max_retries` and `body_overrides` (removed) and any unknown field are
/// `InvalidArgument`.
#[derive(Clone, Deserialize)]
pub struct ExternalProviderEntry {
    /// Provider name used for `provider("name", ...)` lookup. Required.
    pub name: String,
    /// Human-readable name. Defaults to `name` if absent.
    pub display: Option<String>,
    /// API base URL. Required, must be a valid `http(s)://` URL.
    pub base_url: String,
    /// Env var name to read the API key from. Optional.
    pub env_var: Option<String>,
    /// `"env:VAR_NAME"` reference (recommended) or a literal key string
    /// (supported but discouraged). Optional.
    pub api_key: Option<String>,
    /// Protocol kind. Only `"openai_compat"` is accepted; other values error.
    #[serde(default = "default_openai_compat")]
    pub protocol: String,
    /// Provider capability profile. All fields optional, defaults to full.
    #[serde(default)]
    pub profile: ProviderProfile,
    // --- Fields equivalent to ProviderOptions (provider-level config) ---
    /// Extra headers merged into every request.
    pub headers: Option<HashMap<String, String>>,
    /// OpenAI organization ID (`OpenAI-Organization` header).
    pub organization: Option<String>,
    /// OpenAI project ID (`OpenAI-Project` header).
    pub project: Option<String>,
    /// Free-form note for the user; the library ignores this.
    pub comment: Option<String>,
}

impl std::fmt::Debug for ExternalProviderEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExternalProviderEntry")
            .field("name", &self.name)
            .field("display", &self.display)
            .field("base_url", &self.base_url)
            .field("env_var", &self.env_var)
            .field("api_key", &self.api_key.is_some())
            .field("protocol", &self.protocol)
            .field("profile", &self.profile)
            .field("headers", &self.headers.as_ref().map(HashMap::len))
            .field("organization", &self.organization)
            .field("project", &self.project)
            .field("comment", &self.comment)
            .finish()
    }
}

fn default_openai_compat() -> String {
    "openai_compat".to_string()
}

#[derive(Deserialize)]
struct ProvidersConfig {
    providers: Vec<ExternalProviderEntry>,
}

/// The keys of an external entry's JSON object.
const EXTERNAL_ENTRY_KEYS: [&str; 11] = [
    "name",
    "display",
    "base_url",
    "env_var",
    "api_key",
    "protocol",
    "profile",
    "headers",
    "organization",
    "project",
    "comment",
];

static OVERLAYS: OnceLock<RwLock<HashMap<String, ExternalProviderEntry>>> = OnceLock::new();

fn overlays() -> &'static RwLock<HashMap<String, ExternalProviderEntry>> {
    OVERLAYS.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Register (or replace) an external provider entry.
///
/// Validation failures (empty name, non-`http(s)://` base_url, templated
/// base_url, unsupported protocol, unusable profile) return
/// [`AiMuxError::InvalidArgument`] - they never panic.
///
/// # Errors
///
/// Returns [`AiMuxError::InvalidArgument`] when entry validation fails (empty
/// name, non-`http(s)://` base URL, unsupported protocol).
pub fn register_provider(entry: ExternalProviderEntry) -> Result<(), AiMuxError> {
    validate_external_entry(&entry)?;
    let mut overlays = overlays().write().unwrap();
    overlays.insert(entry.name.clone(), entry);
    Ok(())
}

/// Whether `name` was registered at runtime via [`register_provider`] /
/// [`load_providers_from_json`] (RFC-0020 overlay). Used by the replay path
/// to recognize externally-registered OpenAI-compatible providers.
#[must_use]
pub fn is_external_provider(name: &str) -> bool {
    overlays().read().unwrap().contains_key(name)
}

/// Load and register multiple external providers from a JSON string
/// (`{ "providers": [ ... ] }`). Useful for binding-layer pass-through.
///
/// # Errors
///
/// Returns `AiMuxError::JsonParse` for malformed JSON or a document of the
/// wrong shape, `InvalidArgument` for an entry carrying a removed
/// (`max_retries`, `body_overrides`) or unknown field, and propagates each
/// entry's `register_provider` validation error.
pub fn load_providers_from_json(json: &str) -> Result<(), AiMuxError> {
    let value: Value = serde_json::from_str(json).map_err(|e| {
        AiMuxError::JsonParse(format!("failed to parse external providers config: {e}"))
    })?;
    if let Some(entries) = value.get("providers").and_then(Value::as_array) {
        for entry in entries.iter().filter_map(Value::as_object) {
            let name = entry
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("<unnamed>");
            reject_removed_provider_options(
                entry.contains_key("max_retries"),
                entry.contains_key("body_overrides"),
            )
            .map_err(|e| AiMuxError::InvalidArgument(format!("external provider '{name}': {e}")))?;
            if let Some(unknown) = entry
                .keys()
                .find(|key| !EXTERNAL_ENTRY_KEYS.contains(&key.as_str()))
            {
                return Err(AiMuxError::InvalidArgument(format!(
                    "external provider '{name}' has an unknown field `{unknown}`; accepted \
                     fields: {}",
                    EXTERNAL_ENTRY_KEYS.join(", ")
                )));
            }
        }
    }
    let config: ProvidersConfig = serde_json::from_value(value).map_err(|e| {
        AiMuxError::JsonParse(format!("failed to parse external providers config: {e}"))
    })?;
    for entry in config.providers {
        register_provider(entry)?;
    }
    Ok(())
}

/// Validate an external entry before inserting it into the overlay.
fn validate_external_entry(entry: &ExternalProviderEntry) -> Result<(), AiMuxError> {
    if entry.name.trim().is_empty() {
        return Err(AiMuxError::InvalidArgument(
            "external provider entry missing `name`".into(),
        ));
    }
    if entry.base_url.trim().is_empty() {
        return Err(AiMuxError::InvalidArgument(format!(
            "external provider '{}' missing `base_url`",
            entry.name
        )));
    }
    if !(entry.base_url.starts_with("https://") || entry.base_url.starts_with("http://")) {
        return Err(AiMuxError::InvalidArgument(format!(
            "external provider '{}' base_url must start with http(s)://, got {:?}",
            entry.name, entry.base_url
        )));
    }
    if entry.base_url.contains(['{', '}', '<', '>']) {
        return Err(AiMuxError::InvalidArgument(format!(
            "external provider '{}' base_url {:?} has a placeholder; an external entry takes a \
             concrete URL",
            entry.name, entry.base_url
        )));
    }
    if entry.protocol != "openai_compat" {
        return Err(AiMuxError::InvalidArgument(format!(
            "external provider '{}' has unsupported protocol {:?}; only \"openai_compat\" is supported",
            entry.name, entry.protocol
        )));
    }
    entry.profile.chat_profile().map_err(|e| {
        AiMuxError::InvalidArgument(format!("external provider '{}': {e}", entry.name))
    })?;
    Ok(())
}

// ── By-name creation for the bindings ────────────────────────────────────────

/// Create a provider by name for a binding: a runtime-registered external
/// entry first, otherwise [`create_provider`](crate::create_provider) (the
/// vendor packages and the registry rows).
///
/// # Errors
///
/// [`AiMuxError::NoSuchProvider`] for an unknown name, `InvalidArgument` for
/// invalid options.
pub fn provider_handle(
    name: impl AsRef<str>,
    api_key: Option<String>,
    options: Option<ProviderOptions>,
) -> Result<Arc<dyn Provider>, AiMuxError> {
    let name = name.as_ref();
    let external = overlays().read().unwrap().get(name).cloned();
    match external {
        Some(entry) => Ok(Arc::new(build_external(&entry, api_key, options)?)),
        None => crate::create_provider(name, preset_settings(api_key, options)),
    }
}

/// `PresetSettings` from the by-name arguments. Organization and project
/// become their headers, below the caller's own headers.
fn preset_settings(api_key: Option<String>, options: Option<ProviderOptions>) -> PresetSettings {
    let mut settings = PresetSettings {
        api_key: api_key.map(Resolvable::Value),
        ..PresetSettings::default()
    };
    if let Some(options) = options {
        let mut headers = aimux_provider_utils::HeaderMapOpt::new();
        if let Some(org) = options.organization {
            headers.insert("OpenAI-Organization".to_string(), Some(org));
        }
        if let Some(project) = options.project {
            headers.insert("OpenAI-Project".to_string(), Some(project));
        }
        for (name, value) in options.headers.into_iter().flatten() {
            headers.insert(name, Some(value));
        }
        settings.base_url = options.base_url;
        settings.headers = (!headers.is_empty()).then_some(headers);
        settings.params = options.params.unwrap_or_default();
    }
    settings
}

/// An external entry as a compatible provider: its profile, its URL (the
/// per-call override wins), its headers and its key, which is resolved now.
fn build_external(
    entry: &ExternalProviderEntry,
    api_key: Option<String>,
    options: Option<ProviderOptions>,
) -> Result<OpenAICompatibleProvider, AiMuxError> {
    let display = entry.display.clone().unwrap_or_else(|| entry.name.clone());
    let options = options.unwrap_or_default();
    if let Some(params) = &options.params
        && !params.is_empty()
    {
        return Err(AiMuxError::InvalidArgument(format!(
            "external provider '{}' has no template parameters; `params` does not apply",
            entry.name
        )));
    }

    let key = resolve_external_key(entry, &display, api_key)?;

    let mut fixed_headers = Vec::new();
    if let Some(org) = options.organization.or_else(|| entry.organization.clone()) {
        fixed_headers.push(("OpenAI-Organization".to_string(), org));
    }
    if let Some(project) = options.project.or_else(|| entry.project.clone()) {
        fixed_headers.push(("OpenAI-Project".to_string(), project));
    }
    let headers = options
        .headers
        .or_else(|| entry.headers.clone())
        .map(|headers| headers.into_iter().map(|(k, v)| (k, Some(v))).collect());
    let base_url = validate_base_url(options.base_url.as_deref().unwrap_or(&entry.base_url))?;

    OpenAICompatibleProvider::assemble(Assembly {
        name: entry.name.clone(),
        base_url: BaseUrl::Fixed(base_url),
        credential: Credential::Explicit(Resolvable::Value(key)),
        fixed_headers,
        headers,
        query_params: None,
        fetch: None,
        transform_request_body: None,
        profile: entry.profile.chat_profile()?,
    })
}

/// Resolve the api key of an external entry.
///
/// Priority: explicit `api_key` parameter > entry-level `api_key` field
/// (supports `"env:VAR"` references, resolved against the environment) >
/// entry `env_var` (read from the environment via [`load_api_key`]).
fn resolve_external_key(
    entry: &ExternalProviderEntry,
    display: &str,
    api_key: Option<String>,
) -> Result<String, AiMuxError> {
    if let Some(key) = api_key {
        return Ok(key);
    }
    if let Some(entry_key) = &entry.api_key
        && !entry_key.is_empty()
    {
        // Entry-level key: "env:VAR" → read env; otherwise treat as literal.
        if let Some(var) = entry_key.strip_prefix("env:") {
            if var.is_empty() {
                return Err(AiMuxError::InvalidArgument(format!(
                    "external provider '{}' has malformed api_key reference {:?} (empty var name)",
                    entry.name, entry_key
                )));
            }
            let val = std::env::var(var).map_err(|_| {
                AiMuxError::InvalidArgument(format!(
                    "external provider '{}' references env var `{var}` via api_key, but it is not set",
                    entry.name
                ))
            })?;
            return Ok(val);
        }
        return Ok(entry_key.clone());
    }
    match entry.env_var.as_deref().filter(|var| !var.is_empty()) {
        Some(var) => load_api_key(None, var, display),
        None => Err(AiMuxError::InvalidArgument(format!(
            "provider '{}' has no api_key parameter, entry-level api_key, or env_var to read from",
            entry.name
        ))),
    }
}
