//! Registry-backed provider construction (RFC-0017 phase 4, RFC-0036 section 5).
//!
//! Single source of truth: `provider_registry.json`, parsed once into the
//! runtime table in [`crate::preset`]. The by-name entry points look up a
//! descriptor and call [`PresetProvider::create`], adding the runtime overlay
//! (RFC-0020) and the argument shapes the bindings use.
//!
//! An unknown name is [`AiMuxError::NoSuchProvider`]: it never falls back to
//! another provider. An overlay entry or a registry row that cannot be built
//! is a specific `InvalidArgument` (unusable URL, missing template parameter,
//! removed setting).
//!
//! ```
//! use aimux_providers::{provider, ProviderOptions};
//!
//! # fn smoke() -> Result<(), aimux_core::error::AiMuxError> {
//! // Key from env var (GROQ_API_KEY), base URL & dialect from the registry.
//! let model = provider("groq", None, "llama-3.3-70b", None)?;
//! # let _ = model;
//! # Ok(())
//! # }
//! ```

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

use futures::future::BoxFuture;
use serde::Deserialize;
use serde_json::Value;

use aimux_core::embedding_model::EmbeddingModel;
use aimux_core::error::AiMuxError;
use aimux_core::image_model::ImageModel;
use aimux_core::language_model::LanguageModel;
use aimux_core::model_catalogue::RuntimeModel;
use aimux_core::provider::{Provider, ProviderDiscovery};
use aimux_provider_utils::{Resolvable, load_api_key, validate_base_url};

use crate::openai_compatible::config::{BaseUrl, ChatDialect};
use crate::openai_compatible::{Assembly, ChatProfile, OpenAICompatibleProvider};
use crate::preset;
use crate::preset::{PresetDescriptor, PresetProvider, PresetSettings};
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

/// Build a language model for a provider by name.
///
/// Lookup order: runtime overlay (RFC-0020 [`register_provider`]) → the
/// registry presets ([`crate::preset`]) → [`AiMuxError::NoSuchProvider`].
///
/// - `api_key = None` reads the provider's env var from the registry entry
///   (or the external entry's `env_var` / `api_key` field) - now, so a missing
///   key is reported here rather than by the first request. A keyless preset
///   (`auth: none`) needs none.
/// - `options` overrides individual fields of the resolved entry
///   (replaces the retired `with_base_url` etc.).
/// - Unknown names return [`AiMuxError::NoSuchProvider`] naming the requested
///   provider; built-in names are available through [`provider_names`]
///   (overlay-registered names are not).
/// - The model's `provider()` is `"{name}.chat"` (`"groq.chat"`).
///
/// # Errors
///
/// Returns [`AiMuxError::NoSuchProvider`] for unknown names,
/// `InvalidArgument` for invalid entries/options, and key-resolution errors
/// from the registry env var.
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

/// Test-only helper: remove a name from the overlay so tests are hermetic.
#[cfg(test)]
pub(crate) fn clear_overlay(name: &str) {
    overlays().write().unwrap().remove(name);
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

// ── Unified lookup (registry presets + overlay layer) ───────────────────────

/// What the by-name entry points hand out: one of the two provider kinds,
/// behind a single `Provider` + `ProviderDiscovery` face.
enum Resolved {
    Preset(PresetProvider),
    External(OpenAICompatibleProvider),
}

struct ResolvedProvider(Resolved);

impl Provider for ResolvedProvider {
    fn language_model(&self, model_id: &str) -> Result<Arc<dyn LanguageModel>, AiMuxError> {
        match &self.0 {
            Resolved::Preset(p) => p.language_model(model_id),
            Resolved::External(p) => p.language_model(model_id),
        }
    }

    fn embedding_model(&self, model_id: &str) -> Result<Arc<dyn EmbeddingModel>, AiMuxError> {
        match &self.0 {
            Resolved::Preset(p) => p.embedding_model(model_id),
            Resolved::External(p) => p.embedding_model(model_id),
        }
    }

    fn image_model(&self, model_id: &str) -> Result<Arc<dyn ImageModel>, AiMuxError> {
        match &self.0 {
            Resolved::Preset(p) => p.image_model(model_id),
            Resolved::External(p) => p.image_model(model_id),
        }
    }
}

impl ProviderDiscovery for ResolvedProvider {
    fn list_models(&self) -> BoxFuture<'_, Result<Vec<RuntimeModel>, AiMuxError>> {
        match &self.0 {
            Resolved::Preset(p) => p.list_models(),
            Resolved::External(p) => p.list_models(),
        }
    }
}

/// Build a **provider handle** for a built-in or externally-registered provider
/// by name (RFC-0027 + RFC-0020 overlay).
///
/// Lookup order: runtime overlay (RFC-0020) → registry presets → NoSuchProvider.
///
/// Unlike [`provider`] (which binds to a single `model_id` and returns a
/// `LanguageModel`), this returns the [`Provider`] itself, so callers can call
/// [`Provider::language_model`] on a chosen id. For runtime model discovery
/// build the handle with [`provider_discovery`] instead.
///
/// Same key/options semantics as [`provider`].
///
/// # Errors
///
/// Returns [`AiMuxError::NoSuchProvider`] for unknown names, `InvalidArgument`
/// for an unexpanded templated base URL or invalid options, and key-resolution
/// errors.
pub fn provider_handle(
    name: impl AsRef<str>,
    api_key: Option<String>,
    options: Option<ProviderOptions>,
) -> Result<Arc<dyn Provider>, AiMuxError> {
    resolve_provider(name.as_ref(), api_key, options).map(|p| p as Arc<dyn Provider>)
}

/// Build a **discovery handle** for a built-in or externally-registered
/// provider by name: the [`ProviderDiscovery`] side of [`provider_handle`]
/// (RFC-0027), used to call [`ProviderDiscovery::list_models`].
///
/// Same lookup, key and options semantics as [`provider_handle`].
///
/// # Errors
///
/// Same as [`provider_handle`].
pub fn provider_discovery(
    name: impl AsRef<str>,
    api_key: Option<String>,
    options: Option<ProviderOptions>,
) -> Result<Arc<dyn ProviderDiscovery>, AiMuxError> {
    resolve_provider(name.as_ref(), api_key, options).map(|p| p as Arc<dyn ProviderDiscovery>)
}

/// Shared lookup behind [`provider_handle`] and [`provider_discovery`].
fn resolve_provider(
    name: &str,
    api_key: Option<String>,
    options: Option<ProviderOptions>,
) -> Result<Arc<ResolvedProvider>, AiMuxError> {
    // 1. Runtime overlay (RFC-0020) - registered entries take precedence.
    let external = overlays().read().unwrap().get(name).cloned();
    let resolved = if let Some(entry) = external {
        Resolved::External(build_external(&entry, api_key, options)?)
    } else {
        // 2. The registry presets. An unknown name is an error, never a
        //    different provider.
        let entry = preset::lookup(name).ok_or_else(|| AiMuxError::NoSuchProvider {
            // Display derives from the id alone; valid names are discoverable
            // via `provider_names()` - listing hundreds of names here would
            // ride along in every error, across the C ABI.
            provider_id: name.to_string(),
        })?;
        let preset = PresetProvider::create(entry.descriptor, preset_settings(api_key, options))?;
        // The by-name entry point refuses a provider that cannot be used:
        // an unexpandable template or an unset key variable fails here, not
        // on the first request.
        preset.check_ready()?;
        Resolved::Preset(preset)
    };
    Ok(Arc::new(ResolvedProvider(resolved)))
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

/// Convenience: build from the env-var key of the registry entry.
///
/// # Errors
///
/// Propagates the errors of [`provider`] (unknown provider, missing env-var
/// key, invalid options).
pub fn provider_from_env(
    name: impl AsRef<str>,
    model_id: &str,
    options: Option<ProviderOptions>,
) -> Result<Arc<dyn LanguageModel>, AiMuxError> {
    provider(name, None, model_id, options)
}

/// Names of all built-in registry providers.
pub fn provider_names() -> impl Iterator<Item = &'static str> {
    preset::names()
}

/// Public lookup of a registered provider's descriptor - used by tests that
/// assert registry wiring (e.g. `max_tokens_key`, `auth`) without
/// constructing a model. Returns `None` for unknown provider names.
#[must_use]
pub fn provider_registry_entry(name: &str) -> Option<&'static PresetDescriptor> {
    preset::lookup(name).map(|entry| entry.descriptor)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str) -> ExternalProviderEntry {
        ExternalProviderEntry {
            name: name.into(),
            display: None,
            base_url: "https://relay.test.example/v1".into(),
            env_var: None,
            api_key: None,
            protocol: "openai_compat".into(),
            profile: ProviderProfile::default(),
            headers: None,
            organization: None,
            project: None,
            comment: None,
        }
    }

    #[test]
    fn external_entry_debug_does_not_print_api_key() {
        let mut entry = entry("debug-test");
        entry.api_key = Some("literal-secret-key".into());
        assert!(!format!("{entry:?}").contains("literal-secret-key"));
    }

    #[test]
    fn provider_builds_groq_model() {
        let model = match provider("groq", Some("sk-test".into()), "llama-3.3-70b", None) {
            Ok(m) => m,
            Err(e) => panic!("groq should construct: {e}"),
        };
        // RFC-0036 D-d: the registry provider surfaces `{name}.chat`.
        assert_eq!(model.provider(), "groq.chat");
        assert_eq!(model.model_id(), "llama-3.3-70b");
    }

    #[test]
    fn provider_applies_registry_dialect() {
        // groq and deepseek rows point at their own packages' dialects.
        assert_eq!(
            provider_registry_entry("groq").unwrap().family,
            crate::preset::PresetFamily::Groq
        );
        assert_eq!(
            provider_registry_entry("deepseek").unwrap().family,
            crate::preset::PresetFamily::DeepSeek
        );
        // stepfun entry: max_tokens_key="max_tokens".
        assert_eq!(
            provider_registry_entry("stepfun").unwrap().max_tokens_key,
            Some("max_tokens")
        );
        assert_eq!(
            provider_registry_entry("heroku").unwrap().max_tokens_key,
            Some("max_completion_tokens")
        );
    }

    #[test]
    fn provider_accepts_string_names() {
        let model = match provider("groq", Some("sk-test".into()), "llama-3.3-70b", None) {
            Ok(m) => m,
            Err(e) => panic!("string name should construct: {e}"),
        };
        assert_eq!(model.model_id(), "llama-3.3-70b");
    }

    #[test]
    fn provider_unknown_name_reports_provider_id() {
        let err = match provider("no-such-provider", Some("k".into()), "m", None) {
            Ok(_) => panic!("unknown name must fail"),
            Err(e) => e,
        };
        match err {
            AiMuxError::NoSuchProvider { ref provider_id } => {
                assert_eq!(provider_id, "no-such-provider");
                // Display derives from the single stored fact.
                assert_eq!(err.to_string(), "No such provider: no-such-provider");
                // The names stay out of the error - they ride the C ABI.
                let text = err.to_string();
                assert!(!text.contains("groq"), "must not list the registry: {text}");
            }
            other => panic!("expected NoSuchProvider, got {other:?}"),
        }
    }

    #[test]
    fn provider_missing_env_key_fails() {
        // env var almost certainly unset in CI; the error must be the missing-key
        // error, not a registry problem.
        let err = match provider("abacus", None, "m", None) {
            Ok(_) => panic!("missing env key must fail"),
            Err(e) => e,
        };
        assert!(err.to_string().to_lowercase().contains("api key"));
    }

    #[test]
    fn a_keyless_preset_needs_no_key() {
        let model = provider("ollama", None, "llama3.2", None).expect("auth none needs no key");
        assert_eq!(model.provider(), "ollama.chat");
    }

    #[test]
    fn registry_entries_are_valid() {
        let names: Vec<_> = provider_names().collect();
        assert_eq!(names.len(), 283);
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len(), "names are unique");
        for name in names {
            let d = provider_registry_entry(name).unwrap();
            assert!(name.starts_with(|c: char| c.is_ascii_lowercase()));
            assert!(!d.base_url.is_empty());
            if d.auth == crate::preset::AuthMode::ApiKey {
                assert!(!d.env_var.is_empty(), "{name} needs a key variable");
            }
        }
    }

    #[test]
    fn registry_no_corrupt_base_urls() {
        // Issue #90 R2: no registry base_url may carry non-ASCII pollution
        // or be a non-URL fragment. Templated placeholders (cloudflare, neon,
        // snowflake, oci, vertex, ...) are `{param}` and expanded per request.
        for name in provider_names() {
            let d = provider_registry_entry(name).unwrap();
            assert!(
                d.base_url.starts_with("https://") || d.base_url.starts_with("http://"),
                "registry entry '{}' has a non-URL base_url: {:?}",
                d.name,
                d.base_url
            );
            assert!(
                d.base_url.is_ascii(),
                "registry entry '{}' base_url has non-ASCII chars: {:?}",
                d.name,
                d.base_url
            );
            assert!(
                !d.base_url.contains(['<', '>', '$']),
                "registry entry '{}' base_url has a non-{{param}} placeholder: {:?}",
                d.name,
                d.base_url
            );
        }
    }

    #[test]
    fn registry_fixed_base_urls_are_correct() {
        // Issue #90 R2: pin the corrected base_urls for entries that were
        // broken (regression guard against reverting to the old bad values).
        let url = |name: &str| provider_registry_entry(name).unwrap().base_url;
        assert_eq!(
            url("xpersona"),
            "https://www.xpersona.co/v1",
            "xpersona base_url was '/v1' (truncated); must be the full URL"
        );
        assert_eq!(
            url("moonshotai_cn"),
            "https://api.moonshot.cn/anthropic/v1",
            "moonshotai_cn base_url had a leaked '（Anthropic' annotation suffix"
        );
        assert_eq!(
            url("zhipuai_coding_plan"),
            "https://open.bigmodel.cn/api/coding/paas/v4",
            "zhipuai_coding_plan base_url was a docs page, not the API endpoint"
        );
        assert_eq!(
            url("the_grid_ai"),
            "https://api.thegrid.ai/v1",
            "the_grid_ai base_url was a docs page, not the API endpoint"
        );
    }

    #[test]
    fn provider_rejects_templated_base_url_without_override() {
        // A templated row without its parameters is a specific error: the
        // account / host parts cannot be guessed.
        for name in ["cloudflare", "snowflake", "oci", "neon"] {
            let err = match provider(name, Some("dummy".into()), "m", None) {
                Ok(_) => panic!("templated base_url for '{name}' without override must fail"),
                Err(e) => e,
            };
            assert!(
                matches!(&err, AiMuxError::InvalidArgument(m) if m.contains("template parameter")),
                "expected InvalidArgument naming the parameter for '{name}', got {err:?}"
            );
        }
    }

    #[test]
    fn provider_accepts_templated_base_url_with_override() {
        // With a concrete base_url override, the templated entry must construct
        // fine (key is dummy; we only assert it gets past validation).
        let res = provider(
            "cloudflare",
            Some("dummy".into()),
            "m",
            Some(ProviderOptions {
                base_url: Some("https://example.com/v1".into()),
                ..Default::default()
            }),
        );
        assert!(res.is_ok(), "override should bypass the template");
    }

    #[test]
    fn provider_accepts_template_params() {
        let res = provider(
            "snowflake",
            Some("dummy".into()),
            "m",
            Some(ProviderOptions {
                params: Some(HashMap::from([(
                    "account_identifier".to_string(),
                    "org-acct".to_string(),
                )])),
                ..Default::default()
            }),
        );
        assert!(res.is_ok(), "{:?}", res.err());
    }

    #[test]
    fn provider_rejects_undeclared_or_unsafe_params() {
        for (key, value) in [("nope", "x"), ("account_identifier", "a/b")] {
            let res = provider(
                "snowflake",
                Some("dummy".into()),
                "m",
                Some(ProviderOptions {
                    params: Some(HashMap::from([(key.to_string(), value.to_string())])),
                    ..Default::default()
                }),
            );
            assert!(
                matches!(res, Err(AiMuxError::InvalidArgument(_))),
                "{key}={value} must be refused"
            );
        }
    }

    #[test]
    fn provider_names_list_the_registry() {
        let names: Vec<_> = provider_names().collect();
        assert!(names.contains(&"groq"));
        assert!(names.contains(&"ollama"));
        assert!(names.contains(&"vertex_ai_openai_models"));
    }

    // ── RFC-0020: external provider overlay ────────────────────────────────

    #[test]
    fn register_and_lookup_external_provider() {
        clear_overlay("test-relay-new");
        register_provider(ExternalProviderEntry {
            display: Some("Test Relay".into()),
            env_var: Some("TEST_RELAY_KEY".into()),
            api_key: Some("dummy-key".into()),
            ..entry("test-relay-new")
        })
        .unwrap();
        let model = provider("test-relay-new", None, "test-model", None).unwrap();
        assert_eq!(model.provider(), "test-relay-new.chat");
        clear_overlay("test-relay-new");
    }

    #[tokio::test]
    async fn external_provider_overrides_builtin() {
        use aimux_core::content::ContentPart;
        use aimux_core::language_model_message::LanguageModelPromptMessage;
        use aimux_core::message::Role;
        use aimux_core::options::CallOptions;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        // Register an overlay for the built-in "groq" name with a different
        // base_url; provider() must resolve to the overlay, not the registry.
        // The registry entry for groq points at api.groq.com, so a request
        // that reaches the mock server proves which path ran.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "chatcmpl-test",
                "object": "chat.completion",
                "created": 1711115037,
                "model": "llama-3.3-70b",
                "choices": [{
                    "index": 0,
                    "message": { "role": "assistant", "content": "hi" },
                    "finish_reason": "stop"
                }],
                "usage": { "prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2 }
            })))
            .expect(1)
            .mount(&server)
            .await;

        clear_overlay("groq"); // hermetic start (other tests may use "groq")
        register_provider(ExternalProviderEntry {
            display: Some("Groq Override".into()),
            base_url: server.uri(),
            env_var: Some("GROQ_API_KEY".into()),
            api_key: Some("dummy".into()),
            ..entry("groq")
        })
        .unwrap();
        let model = provider("groq", None, "llama-3.3-70b", None).unwrap();
        let result = model
            .do_generate(&CallOptions::new(vec![LanguageModelPromptMessage {
                role: Role::User,
                content: vec![ContentPart::text("Hello")],
                ..Default::default()
            }]))
            .await;
        clear_overlay("groq");
        result.expect("provider() must resolve via the overlay, not the built-in registry");
        // `expect(1)` above is verified when `server` drops.
    }

    #[test]
    fn load_providers_from_json_parses_and_registers() {
        clear_overlay("test-json-1");
        clear_overlay("test-json-2");
        let json = r#"{
            "providers": [
                {
                    "name": "test-json-1",
                    "base_url": "https://a.test/v1",
                    "api_key": "dummy"
                },
                {
                    "name": "test-json-2",
                    "base_url": "https://b.test/v1",
                    "env_var": "TEST_JSON_2_KEY"
                }
            ]
        }"#;
        load_providers_from_json(json).unwrap();
        assert!(overlays().read().unwrap().contains_key("test-json-1"));
        assert!(overlays().read().unwrap().contains_key("test-json-2"));
        clear_overlay("test-json-1");
        clear_overlay("test-json-2");
    }

    #[test]
    fn register_provider_rejects_bad_base_url() {
        clear_overlay("test-bad-url");
        let err = register_provider(ExternalProviderEntry {
            base_url: "ftp://nope".into(),
            ..entry("test-bad-url")
        })
        .unwrap_err();
        assert!(
            matches!(err, AiMuxError::InvalidArgument(ref m) if m.contains("http(s)://")),
            "expected InvalidArgument about scheme, got {err:?}"
        );
        clear_overlay("test-bad-url");
    }

    #[test]
    fn register_provider_rejects_a_templated_base_url() {
        clear_overlay("test-templated");
        let err = register_provider(ExternalProviderEntry {
            base_url: "https://{tenant}.example/v1".into(),
            ..entry("test-templated")
        })
        .unwrap_err();
        assert!(
            matches!(err, AiMuxError::InvalidArgument(ref m) if m.contains("placeholder")),
            "expected InvalidArgument about the placeholder, got {err:?}"
        );
    }

    #[test]
    fn register_provider_rejects_bad_protocol() {
        clear_overlay("test-bad-proto");
        let err = register_provider(ExternalProviderEntry {
            protocol: "anthropic".into(),
            ..entry("test-bad-proto")
        })
        .unwrap_err();
        assert!(
            matches!(err, AiMuxError::InvalidArgument(ref m) if m.contains("openai_compat")),
            "expected InvalidArgument about protocol, got {err:?}"
        );
        clear_overlay("test-bad-proto");
    }

    #[test]
    fn register_provider_rejects_empty_name() {
        let err = register_provider(entry("  ")).unwrap_err();
        assert!(
            matches!(err, AiMuxError::InvalidArgument(ref m) if m.contains("name")),
            "expected InvalidArgument about name, got {err:?}"
        );
    }

    #[test]
    fn register_provider_rejects_an_unknown_max_tokens_key() {
        let err = register_provider(ExternalProviderEntry {
            profile: ProviderProfile {
                max_tokens_key: Some("max_new_tokens".into()),
                ..ProviderProfile::default()
            },
            ..entry("test-bad-key")
        })
        .unwrap_err();
        assert!(
            matches!(err, AiMuxError::InvalidArgument(ref m) if m.contains("max_tokens_key")),
            "{err:?}"
        );
    }

    #[test]
    fn load_providers_from_json_rejects_removed_and_unknown_fields() {
        for (field, needle) in [
            (r#""max_retries": 3"#, "max_retries"),
            (r#""body_overrides": {"a": 1}"#, "body_overrides"),
            (r#""bogus": true"#, "bogus"),
        ] {
            let json = format!(
                r#"{{ "providers": [ {{ "name": "test-removed", "base_url": "https://x.test/v1", "api_key": "k", {field} }} ] }}"#
            );
            let err = load_providers_from_json(&json).unwrap_err();
            assert!(
                matches!(&err, AiMuxError::InvalidArgument(m) if m.contains(needle)),
                "{field}: expected InvalidArgument naming it, got {err:?}"
            );
            assert!(!is_external_provider("test-removed"), "nothing registered");
        }
    }

    #[test]
    fn external_provider_profile_applied() {
        clear_overlay("test-profile");
        register_provider(ExternalProviderEntry {
            api_key: Some("dummy".into()),
            profile: ProviderProfile {
                supports_top_k: false,
                supports_tools: false,
                supports_response_format: true,
                stream_usage_key: Some("x_custom".into()),
                max_tokens_key: Some("max_completion_tokens".into()),
            },
            base_url: "https://profile.test/v1".into(),
            ..entry("test-profile")
        })
        .unwrap();
        let entry = overlays()
            .read()
            .unwrap()
            .get("test-profile")
            .unwrap()
            .clone();
        let profile = entry.profile.chat_profile().unwrap();
        assert!(!profile.dialect.supports_top_k);
        assert!(!profile.dialect.supports_tools);
        assert_eq!(
            profile.dialect.stream_usage_key.as_deref(),
            Some("x_custom")
        );
        assert_eq!(
            profile.dialect.max_tokens_key,
            Some("max_completion_tokens")
        );
        clear_overlay("test-profile");
    }

    #[test]
    fn external_provider_profile_defaults_to_full() {
        // When `profile` is omitted entirely from the JSON, the entry must
        // still default to the OpenAI-compatible baseline (all three
        // supports_* = true). Regression guard: the derive(Default) on
        // ProviderProfile was previously yielding false for these.
        clear_overlay("test-default-profile");
        load_providers_from_json(r#"{ "providers": [ { "name": "test-default-profile", "base_url": "https://x.test/v1", "api_key": "dummy" } ] }"#)
            .unwrap();
        let entry = overlays()
            .read()
            .unwrap()
            .get("test-default-profile")
            .unwrap()
            .clone();
        let p = entry.profile.chat_profile().unwrap();
        assert!(
            p.dialect.supports_top_k,
            "omitted profile → supports_top_k must be true"
        );
        assert!(
            p.dialect.supports_tools,
            "omitted profile → supports_tools must be true"
        );
        assert!(
            p.dialect.supports_response_format,
            "omitted profile → supports_response_format must be true"
        );
        clear_overlay("test-default-profile");
    }

    #[test]
    fn external_provider_env_var_api_key_resolves() {
        // entry-level api_key = "env:VAR" must read the env var at lookup time.
        clear_overlay("test-envkey");
        // SAFETY: test-only; no other thread is reading this var concurrently.
        unsafe { std::env::set_var("AIMUX_TEST_OVERLAY_KEY", "secret-from-env") };
        register_provider(ExternalProviderEntry {
            base_url: "https://envkey.test/v1".into(),
            api_key: Some("env:AIMUX_TEST_OVERLAY_KEY".into()),
            ..entry("test-envkey")
        })
        .unwrap();
        // provider() with api_key=None must resolve via the "env:" reference.
        let model = provider("test-envkey", None, "m", None).unwrap();
        assert_eq!(model.provider(), "test-envkey.chat");
        // SAFETY: test-only cleanup.
        unsafe { std::env::remove_var("AIMUX_TEST_OVERLAY_KEY") };
        clear_overlay("test-envkey");
    }

    #[test]
    fn external_provider_env_var_api_key_missing_env_fails() {
        clear_overlay("test-envkey-missing");
        // SAFETY: test-only cleanup.
        unsafe { std::env::remove_var("AIMUX_TEST_OVERLAY_MISSING") };
        register_provider(ExternalProviderEntry {
            base_url: "https://missing.test/v1".into(),
            api_key: Some("env:AIMUX_TEST_OVERLAY_MISSING".into()),
            ..entry("test-envkey-missing")
        })
        .unwrap();
        let err = match provider("test-envkey-missing", None, "m", None) {
            Ok(_) => panic!("missing env var must fail"),
            Err(e) => e,
        };
        assert!(
            matches!(err, AiMuxError::InvalidArgument(ref m) if m.contains("AIMUX_TEST_OVERLAY_MISSING")),
            "expected InvalidArgument naming the env var, got {err:?}"
        );
        clear_overlay("test-envkey-missing");
    }

    #[test]
    fn load_providers_from_json_rejects_invalid_json() {
        let err = load_providers_from_json("not json at all").unwrap_err();
        assert!(
            matches!(err, AiMuxError::JsonParse(ref m) if m.contains("parse")),
            "expected JsonParse for malformed input, got {err:?}"
        );
    }
}
