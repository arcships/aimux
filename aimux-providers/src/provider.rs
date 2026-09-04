//! Registry-backed provider construction (RFC-0017 phase 4).
//!
//! Single source of truth: [`provider_registry.json`](provider_registry.json),
//! edited by hand (one row per provider). All built-in
//! OpenAI-compatible providers are looked up by name at runtime — there are no
//! per-provider `XxxConfig`/`XxxProvider` types anymore (retired in phase 4).
//!
//! ```
//! use aimux_providers::{provider, ProviderOptions};
//!
//! # fn smoke() -> Result<(), aimux_core::error::AiMuxError> {
//! // Key from env var (GROQ_API_KEY), base URL & profile from the registry.
//! let model = provider("groq", None, "llama-3.3-70b", None)?;
//! # let _ = model;
//! # Ok(())
//! # }
//! ```

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

use serde::Deserialize;
use serde_json::Value;

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;
use aimux_provider_utils::RetryConfig;

use crate::openai::{OpenAICompatProfile, OpenAIConfig, OpenAIProvider};

/// One entry of `provider-registry.json` (registry slice).
#[derive(Debug, Clone, Deserialize)]
struct RegistryEntry {
    name: String,
    display: String,
    base_url: String,
    env_var: String,
    #[serde(default)]
    profile: ProviderProfile,
}

/// Provider capability profile, expressed as data (non-default fields only
/// in the registry JSON).
///
/// Shared between the built-in registry and the runtime overlay layer
/// (RFC-0020). The `Default` impl and the per-field
/// `#[serde(default = "default_true")]` both yield `true` for the three
/// `supports_*` flags, matching the OpenAI-compatible baseline — so a
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
    pub stream_usage_key: Option<String>,
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

static REGISTRY: OnceLock<Vec<RegistryEntry>> = OnceLock::new();

/// Load + validate the embedded registry exactly once.
fn registry() -> &'static [RegistryEntry] {
    REGISTRY.get_or_init(|| {
        let raw = include_str!("provider_registry.json");
        let entries: Vec<RegistryEntry> = serde_json::from_str(raw)
            .unwrap_or_else(|e| panic!("provider_registry.json is invalid: {e}"));
        for entry in &entries {
            assert!(!entry.name.is_empty(), "registry entry missing name");
            assert!(
                !entry.display.is_empty(),
                "registry entry '{}' missing display",
                entry.name
            );
            assert!(
                !entry.base_url.is_empty(),
                "registry entry '{}' missing base_url",
                entry.name
            );
            assert!(
                !entry.env_var.is_empty(),
                "registry entry '{}' missing env_var",
                entry.name
            );
        }
        entries
    })
}

/// Detect unexpanded placeholder syntax in a registry `base_url`.
///
/// Templated entries (cloudflare, neon, snowflake, oci, ...) carry account- or
/// region-scoped placeholders that the caller must fill in via
/// [`ProviderOptions::base_url`]. Recognized forms: `{VAR}` (covers `${var}`
/// too) and `<host>`. If a new placeholder form is introduced in the
/// registry, extend this function.
fn base_url_has_placeholder(base_url: &str) -> bool {
    base_url.contains('{') || base_url.contains('<')
}

/// Per-call construction options for [`provider`] (overrides the registry entry).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ProviderOptions {
    /// Override the registry base URL.
    pub base_url: Option<String>,
    /// Extra headers merged into every request.
    pub headers: Option<HashMap<String, String>>,
    /// OpenAI organization ID (`OpenAI-Organization` header).
    pub organization: Option<String>,
    /// OpenAI project ID (`OpenAI-Project` header).
    pub project: Option<String>,
    /// Retry count override; `Some(0)` disables retries.
    pub max_retries: Option<u32>,
    /// Request-body overrides (deep-merged; RFC-0017 phase 1).
    pub body_overrides: Option<Value>,
}

/// Build a language model for a provider by name.
///
/// Lookup order: runtime overlay (RFC-0020 [`register_provider`]) → built-in
/// registry → [`AiMuxError::NoSuchProvider`].
///
/// - `api_key = None` reads the provider's env var from the registry entry
///   (or the external entry's `env_var` / `api_key` field).
/// - `options` overrides individual fields of the resolved entry
///   (replaces the retired `with_base_url` etc.).
/// - Unknown names return [`AiMuxError::NoSuchProvider`] naming the requested
///   provider; built-in names are enumerated by the generated `ProviderName`
///   (overlay-registered names are not).
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
) -> Result<Box<dyn LanguageModel>, AiMuxError> {
    let p = provider_handle(name, api_key, options)?;
    p.language_model(model_id)
}

// ── Runtime overlay layer (RFC-0020) ─────────────────────────────────────────

/// A provider entry registered at runtime via [`register_provider`] /
/// [`load_providers_from_json`]. Overrides a same-named built-in entry (whole
/// replacement, not deep merge) or adds a new one.
///
/// Only OpenAI-compatible providers can be registered this way — native
/// protocols (anthropic/google/bedrock…) are code implementations and cannot
/// be described by config data.
#[derive(Debug, Clone, Deserialize)]
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
    /// Provider capability profile. All fields optional, defaults to full().
    #[serde(default)]
    pub profile: ProviderProfile,
    // --- Fields equivalent to ProviderOptions (provider-level config) ---
    /// Extra headers merged into every request.
    pub headers: Option<HashMap<String, String>>,
    /// OpenAI organization ID (`OpenAI-Organization` header).
    pub organization: Option<String>,
    /// OpenAI project ID (`OpenAI-Project` header).
    pub project: Option<String>,
    /// Retry count override; `Some(0)` disables retries.
    pub max_retries: Option<u32>,
    /// Request-body overrides (deep-merged; RFC-0017 phase 1).
    pub body_overrides: Option<Value>,
    /// Free-form note for the user; the library ignores this.
    pub comment: Option<String>,
}

fn default_openai_compat() -> String {
    "openai_compat".to_string()
}

#[derive(Deserialize)]
struct ProvidersConfig {
    providers: Vec<ExternalProviderEntry>,
}

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
/// Validation failures (empty name, non-`http(s)://` base_url, unsupported
/// protocol) return [`AiMuxError::InvalidArgument`] — they never panic.
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
/// Returns `AiMuxError::JsonParse` for malformed JSON and propagates each
/// entry's `register_provider` validation error.
pub fn load_providers_from_json(json: &str) -> Result<(), AiMuxError> {
    let config: ProvidersConfig = serde_json::from_str(json).map_err(|e| {
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
    if entry.protocol != "openai_compat" {
        return Err(AiMuxError::InvalidArgument(format!(
            "external provider '{}' has unsupported protocol {:?}; only \"openai_compat\" is supported",
            entry.name, entry.protocol
        )));
    }
    Ok(())
}

// ── Unified lookup (built-in registry + overlay layer) ──────────────────────

/// A resolved provider entry — the common shape both the built-in registry
/// and the runtime overlay produce, so the downstream construction logic
/// (placeholder check, key resolution, config build) is shared.
struct ResolvedEntry {
    name: String,
    display: String,
    base_url: String,
    env_var: String,
    profile: ProviderProfile,
    /// Entry-level api_key (external entries only). `"env:VAR"` references are
    /// resolved at lookup time; literal strings are used as-is. Built-in
    /// registry entries always have `None` here.
    api_key: Option<String>,
    /// Provider-level config carried by external entries (None for built-ins,
    /// which rely on ProviderOptions for per-call overrides).
    headers: Option<HashMap<String, String>>,
    organization: Option<String>,
    project: Option<String>,
    max_retries: Option<u32>,
    body_overrides: Option<Value>,
}

impl ResolvedEntry {
    /// Resolve a built-in [`RegistryEntry`] into the common shape.
    fn from_registry(entry: &RegistryEntry) -> Self {
        Self {
            name: entry.name.clone(),
            display: entry.display.clone(),
            base_url: entry.base_url.clone(),
            env_var: entry.env_var.clone(),
            profile: entry.profile.clone(),
            api_key: None,
            headers: None,
            organization: None,
            project: None,
            max_retries: None,
            body_overrides: None,
        }
    }

    /// Resolve an [`ExternalProviderEntry`] into the common shape.
    fn from_external(entry: &ExternalProviderEntry) -> Self {
        Self {
            name: entry.name.clone(),
            display: entry.display.clone().unwrap_or_else(|| entry.name.clone()),
            base_url: entry.base_url.clone(),
            env_var: entry.env_var.clone().unwrap_or_default(),
            profile: entry.profile.clone(),
            api_key: entry.api_key.clone(),
            headers: entry.headers.clone(),
            organization: entry.organization.clone(),
            project: entry.project.clone(),
            max_retries: entry.max_retries,
            body_overrides: entry.body_overrides.clone(),
        }
    }
}

/// Build a **provider handle** for a built-in or externally-registered provider
/// by name (RFC-0027 + RFC-0020 overlay).
///
/// Lookup order: runtime overlay (RFC-0020) → built-in registry → NoSuchProvider.
///
/// Unlike [`provider`] (which binds to a single `model_id` and returns a
/// `LanguageModel`), this returns the [`Provider`] itself, so callers can call
/// [`Provider::list_models`] for runtime discovery, then
/// [`Provider::language_model`] on a chosen id.
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
) -> Result<Box<dyn Provider>, AiMuxError> {
    let name = name.as_ref();

    // 1. Runtime overlay (RFC-0020) — registered entries take precedence.
    let resolved = if let Some(ext) = overlays().read().unwrap().get(name) {
        ResolvedEntry::from_external(ext)
    } else {
        // 2. Built-in registry.
        let entry = registry().iter().find(|e| e.name == name).ok_or_else(|| {
            AiMuxError::NoSuchProvider {
                // Display derives from the id alone; valid names are discoverable
                // via the generated `ProviderName` — listing 250 names here would
                // ride along in every error, across the C ABI.
                provider_id: name.to_string(),
            }
        })?;
        ResolvedEntry::from_registry(entry)
    };

    // Reject base_urls that still carry unexpanded placeholders
    // (e.g. cloudflare's `{CLOUDFLARE_ACCOUNT_ID}`, snowflake's
    // `<account-identifier>`). These entries are intentionally templated —
    // the caller MUST supply a concrete base_url via ProviderOptions.
    let base_url_overridden = options.as_ref().is_some_and(|o| o.base_url.is_some());
    if !base_url_overridden && base_url_has_placeholder(&resolved.base_url) {
        return Err(AiMuxError::InvalidArgument(format!(
            "provider '{name}' has a templated base_url {:?} with an unexpanded \
             placeholder; pass a concrete `base_url` via ProviderOptions to use it",
            resolved.base_url
        )));
    }

    // Resolve the api key. Priority: explicit parameter > entry-level api_key
    // (supports "env:VAR" references) > entry env_var (read from environment).
    let (key, source) = resolve_key(&resolved, api_key)?;

    let mut config = build_resolved_config(&resolved, key, options);
    config = config.with_api_key_source(source.as_deref());
    Ok(Box::new(OpenAIProvider::new(config)))
}

/// Resolve the api key for a [`ResolvedEntry`]. Returns `(key, source)` where
/// `source` is the origin tag for RFC-0023 replay reconstruction.
///
/// Priority: explicit `api_key` parameter > entry-level `api_key` field
/// (supports `"env:VAR"` references, resolved against the environment) >
/// entry `env_var` (read from the environment via [`load_api_key`]).
fn resolve_key(
    entry: &ResolvedEntry,
    api_key: Option<String>,
) -> Result<(String, Option<String>), AiMuxError> {
    if let Some(key) = api_key {
        return Ok((key, Some("explicit".to_string())));
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
            return Ok((val, Some(format!("env:{var}"))));
        }
        return Ok((entry_key.clone(), Some("explicit".to_string())));
    }
    if entry.env_var.is_empty() {
        return Err(AiMuxError::InvalidArgument(format!(
            "provider '{}' has no api_key parameter, entry-level api_key, or env_var to read from",
            entry.name
        )));
    }
    let key = aimux_provider_utils::load_api_key(None, &entry.env_var, &entry.display)?;
    Ok((key, Some(format!("env:{}", entry.env_var))))
}

/// Resolve a [`ResolvedEntry`] + per-call [`ProviderOptions`] into a fully-wired
/// `OpenAIConfig`. Provider-level config from the entry (headers/org/project/
/// retries/body_overrides) is applied first; per-call options override them.
fn build_resolved_config(
    entry: &ResolvedEntry,
    key: String,
    options: Option<ProviderOptions>,
) -> OpenAIConfig {
    let mut config = OpenAIConfig::new(key)
        .with_base_url(entry.base_url.clone())
        .with_provider(entry.name.clone())
        .with_profile(profile_from_registry(&entry.profile));

    // Provider-level config (from the entry, built-in or external).
    if let Some(headers) = &entry.headers {
        config = config.with_headers(headers.clone());
    }
    if let Some(org) = &entry.organization {
        config = config.with_org_id(org.clone());
    }
    if let Some(project) = &entry.project {
        config = config.with_project(project.clone());
    }
    if let Some(max_retries) = entry.max_retries {
        config = config.with_retry_config(RetryConfig {
            max_retries,
            ..RetryConfig::default()
        });
    }
    if let Some(overrides) = &entry.body_overrides {
        config = config.with_body_overrides(overrides.clone());
    }

    // Per-call ProviderOptions override on top.
    if let Some(opts) = options {
        if let Some(url) = opts.base_url {
            config = config.with_base_url(url);
        }
        if let Some(headers) = opts.headers {
            config = config.with_headers(headers);
        }
        if let Some(org) = opts.organization {
            config = config.with_org_id(org);
        }
        if let Some(project) = opts.project {
            config = config.with_project(project);
        }
        if let Some(max_retries) = opts.max_retries {
            config = config.with_retry_config(RetryConfig {
                max_retries,
                ..RetryConfig::default()
            });
        }
        if let Some(overrides) = opts.body_overrides {
            config = config.with_body_overrides(overrides);
        }
    }
    config
}

/// Translate a provider profile into the runtime profile.
///
/// `stream_usage_key` / `max_tokens_key` are `&'static str` in
/// `OpenAICompatProfile`; the registry strings are leaked once at first use
/// (bounded: at most one string per field per entry).
fn profile_from_registry(p: &ProviderProfile) -> OpenAICompatProfile {
    OpenAICompatProfile {
        supports_top_k: p.supports_top_k,
        supports_tools: p.supports_tools,
        supports_response_format: p.supports_response_format,
        stream_usage_key: p.stream_usage_key.as_deref().map(|s| {
            let leaked: &'static str = Box::leak(s.to_string().into_boxed_str());
            leaked
        }),
        max_tokens_key: p.max_tokens_key.as_deref().map(|s| {
            let leaked: &'static str = Box::leak(s.to_string().into_boxed_str());
            leaked
        }),
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
) -> Result<Box<dyn LanguageModel>, AiMuxError> {
    provider(name, None, model_id, options)
}

/// Public lookup of a registered provider's runtime profile — used by tests
/// that assert registry wiring (e.g. `max_tokens_key`) without constructing a
/// model. Returns `None` for unknown provider names.
#[must_use]
pub fn provider_registry_entry(name: &str) -> Option<OpenAICompatProfile> {
    registry()
        .iter()
        .find(|e| e.name == name)
        .map(|e| profile_from_registry(&e.profile))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider_name::ProviderName;

    #[test]
    fn provider_builds_groq_model() {
        let model = match provider("groq", Some("sk-test".into()), "llama-3.3-70b", None) {
            Ok(m) => m,
            Err(e) => panic!("groq should construct: {e}"),
        };
        assert_eq!(model.provider(), "groq"); // RFC-0023 C2: registry provider surfaces its real name, not "openai"
        assert_eq!(model.model_id(), "llama-3.3-70b");
    }

    #[test]
    fn provider_applies_registry_profile() {
        // groq entry: supports_top_k=false, stream_usage_key="x_groq",
        // max_tokens_key="max_completion_tokens" — verify the config carries them.
        let entry = registry().iter().find(|e| e.name == "groq").unwrap();
        let profile = profile_from_registry(&entry.profile);
        assert!(!profile.supports_top_k);
        assert_eq!(profile.stream_usage_key, Some("x_groq"));
        assert_eq!(profile.max_tokens_key, Some("max_completion_tokens"));

        // stepfun entry: max_tokens_key="max_tokens".
        let entry = registry().iter().find(|e| e.name == "stepfun").unwrap();
        let profile = profile_from_registry(&entry.profile);
        assert_eq!(profile.max_tokens_key, Some("max_tokens"));
    }

    #[test]
    fn provider_accepts_typed_and_string_names() {
        // Both spellings work: typed ProviderName (recommended) and string.
        let typed = match provider(
            ProviderName::Groq,
            Some("sk-test".into()),
            "llama-3.3-70b",
            None,
        ) {
            Ok(m) => m,
            Err(e) => panic!("typed name should construct: {e}"),
        };
        assert_eq!(typed.model_id(), "llama-3.3-70b");
        let string = match provider("groq", Some("sk-test".into()), "llama-3.3-70b", None) {
            Ok(m) => m,
            Err(e) => panic!("string name should construct: {e}"),
        };
        assert_eq!(string.model_id(), "llama-3.3-70b");
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
                // The 250 names stay out of the error — they ride the C ABI.
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
    fn registry_entries_are_valid() {
        let entries = registry();
        assert_eq!(entries.len(), 251);
        for e in entries {
            assert!(e.name.starts_with(|c: char| c.is_ascii_lowercase()));
            assert!(!e.base_url.is_empty());
            assert!(!e.env_var.is_empty());
        }
    }

    #[test]
    fn registry_no_corrupt_base_urls() {
        // Issue #90 R2: no registry base_url may carry non-ASCII pollution
        // or be a non-URL fragment. Templated placeholders (cloudflare/neon/
        // snowflake/oci) are allowed but rejected at construct time — see
        // provider_handle's base_url_has_placeholder check.
        for e in registry() {
            assert!(
                e.base_url.starts_with("https://") || e.base_url.starts_with("http://"),
                "registry entry '{}' has a non-URL base_url: {:?}",
                e.name,
                e.base_url
            );
            assert!(
                e.base_url.is_ascii(),
                "registry entry '{}' base_url has non-ASCII chars: {:?}",
                e.name,
                e.base_url
            );
        }
    }

    #[test]
    fn registry_fixed_base_urls_are_correct() {
        // Issue #90 R2: pin the corrected base_urls for entries that were
        // broken (regression guard against reverting to the old bad values).
        let by_name: std::collections::HashMap<&str, &str> = registry()
            .iter()
            .map(|e| (e.name.as_str(), e.base_url.as_str()))
            .collect();
        assert_eq!(
            by_name.get("xpersona").copied(),
            Some("https://www.xpersona.co/v1"),
            "xpersona base_url was '/v1' (truncated); must be the full URL"
        );
        assert_eq!(
            by_name.get("moonshotai_cn").copied(),
            Some("https://api.moonshot.cn/anthropic/v1"),
            "moonshotai_cn base_url had a leaked '（Anthropic' annotation suffix"
        );
        assert_eq!(
            by_name.get("zhipuai_coding_plan").copied(),
            Some("https://open.bigmodel.cn/api/coding/paas/v4"),
            "zhipuai_coding_plan base_url was a docs page, not the API endpoint"
        );
        assert_eq!(
            by_name.get("the_grid_ai").copied(),
            Some("https://api.thegrid.ai/v1"),
            "the_grid_ai base_url was a docs page, not the API endpoint"
        );
    }

    #[test]
    fn provider_rejects_templated_base_url_without_override() {
        // Both placeholder forms must be rejected without an override:
        // cloudflare uses `{CLOUDFLARE_ACCOUNT_ID}`, snowflake uses
        // `<account-identifier>` — exercise both so `base_url_has_placeholder`
        // can't silently lose one form.
        for name in ["cloudflare", "snowflake"] {
            let err = match provider(name, Some("dummy".into()), "m", None) {
                Ok(_) => panic!("templated base_url for '{name}' without override must fail"),
                Err(e) => e,
            };
            assert!(
                matches!(err, AiMuxError::InvalidArgument(_)),
                "expected InvalidArgument for '{name}' templated base_url, got {err:?}"
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
        assert!(res.is_ok(), "override should bypass placeholder check");
    }

    #[test]
    fn provider_name_roundtrip() {
        assert_eq!(ProviderName::Groq.as_str(), "groq");
        assert_eq!(
            "deepseek".parse::<ProviderName>().ok(),
            Some(ProviderName::Deepseek)
        );
        assert_eq!("nope".parse::<ProviderName>().ok(), None);
        assert!(ProviderName::all_names().contains("groq"));
        assert_eq!(ProviderName::ALL.len(), 251);
    }

    #[test]
    fn provider_name_matches_registry_json() {
        // Anti-drift: every registry name must exist as a ProviderName variant
        // and round-trip; counts must match (guards against editing the JSON
        // without regenerating provider_name.rs).
        let registry: serde_json::Value =
            serde_json::from_str(include_str!("provider_registry.json"))
                .expect("registry JSON is valid");
        let names: Vec<&str> = registry
            .as_array()
            .expect("registry is an array")
            .iter()
            .map(|e| e["name"].as_str().expect("name is a string"))
            .collect();
        assert_eq!(ProviderName::ALL.len(), names.len());
        for name in &names {
            let variant = name
                .parse::<ProviderName>()
                .unwrap_or_else(|_| panic!("registry name {name} missing from ProviderName"));
            assert_eq!(variant.as_str(), *name);
        }
    }

    // ── RFC-0020: external provider overlay ────────────────────────────────

    #[test]
    fn register_and_lookup_external_provider() {
        clear_overlay("test-relay-new");
        register_provider(ExternalProviderEntry {
            name: "test-relay-new".into(),
            display: Some("Test Relay".into()),
            base_url: "https://relay.test.example/v1".into(),
            env_var: Some("TEST_RELAY_KEY".into()),
            api_key: Some("dummy-key".into()),
            protocol: "openai_compat".into(),
            profile: ProviderProfile::default(),
            headers: None,
            organization: None,
            project: None,
            max_retries: None,
            body_overrides: None,
            comment: None,
        })
        .unwrap();
        let model = provider("test-relay-new", None, "test-model", None).unwrap();
        assert_eq!(model.provider(), "test-relay-new");
        clear_overlay("test-relay-new");
    }

    #[test]
    fn external_provider_overrides_builtin() {
        // Register an overlay for the built-in "groq" name with a different
        // base_url; provider() must resolve to the overlay, not the registry.
        clear_overlay("groq"); // hermetic start (other tests may use "groq")
        register_provider(ExternalProviderEntry {
            name: "groq".into(),
            display: Some("Groq Override".into()),
            base_url: "https://my-groq-relay.example/v1".into(),
            env_var: Some("GROQ_API_KEY".into()),
            api_key: Some("dummy".into()),
            protocol: "openai_compat".into(),
            profile: ProviderProfile::default(),
            headers: None,
            organization: None,
            project: None,
            max_retries: None,
            body_overrides: None,
            comment: None,
        })
        .unwrap();
        // Actually call provider() (not just read the overlay map) to verify
        // the lookup path routes to the overlay. The registry entry for groq
        // has base_url "https://api.groq.com/openai/v1"; the overlay's is
        // different, so config_snapshot().base_url tells us which path ran.
        let model = provider("groq", None, "llama-3.3-70b", None).unwrap();
        let snap = model.config_snapshot();
        assert_eq!(
            snap.base_url.as_deref(),
            Some("https://my-groq-relay.example/v1"),
            "provider() must resolve via the overlay, not the built-in registry"
        );
        clear_overlay("groq");
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
            name: "test-bad-url".into(),
            base_url: "ftp://nope".into(),
            env_var: None,
            api_key: None,
            protocol: "openai_compat".into(),
            display: None,
            profile: ProviderProfile::default(),
            headers: None,
            organization: None,
            project: None,
            max_retries: None,
            body_overrides: None,
            comment: None,
        })
        .unwrap_err();
        assert!(
            matches!(err, AiMuxError::InvalidArgument(ref m) if m.contains("http(s)://")),
            "expected InvalidArgument about scheme, got {err:?}"
        );
        clear_overlay("test-bad-url");
    }

    #[test]
    fn register_provider_rejects_bad_protocol() {
        clear_overlay("test-bad-proto");
        let err = register_provider(ExternalProviderEntry {
            name: "test-bad-proto".into(),
            base_url: "https://ok.example/v1".into(),
            protocol: "anthropic".into(),
            env_var: None,
            api_key: None,
            display: None,
            profile: ProviderProfile::default(),
            headers: None,
            organization: None,
            project: None,
            max_retries: None,
            body_overrides: None,
            comment: None,
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
        let err = register_provider(ExternalProviderEntry {
            name: "  ".into(),
            base_url: "https://ok.example/v1".into(),
            protocol: "openai_compat".into(),
            env_var: None,
            api_key: None,
            display: None,
            profile: ProviderProfile::default(),
            headers: None,
            organization: None,
            project: None,
            max_retries: None,
            body_overrides: None,
            comment: None,
        })
        .unwrap_err();
        assert!(
            matches!(err, AiMuxError::InvalidArgument(ref m) if m.contains("name")),
            "expected InvalidArgument about name, got {err:?}"
        );
    }

    #[test]
    fn external_provider_profile_applied() {
        clear_overlay("test-profile");
        register_provider(ExternalProviderEntry {
            name: "test-profile".into(),
            base_url: "https://profile.test/v1".into(),
            api_key: Some("dummy".into()),
            protocol: "openai_compat".into(),
            profile: ProviderProfile {
                supports_top_k: false,
                supports_tools: false,
                supports_response_format: true,
                stream_usage_key: Some("x_custom".into()),
                max_tokens_key: Some("max_completion_tokens".into()),
            },
            env_var: None,
            display: None,
            headers: None,
            organization: None,
            project: None,
            max_retries: None,
            body_overrides: None,
            comment: None,
        })
        .unwrap();
        let entry = overlays()
            .read()
            .unwrap()
            .get("test-profile")
            .unwrap()
            .clone();
        let p = profile_from_registry(&entry.profile);
        assert!(!p.supports_top_k);
        assert!(!p.supports_tools);
        assert_eq!(p.stream_usage_key, Some("x_custom"));
        assert_eq!(p.max_tokens_key, Some("max_completion_tokens"));
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
        let p = profile_from_registry(&entry.profile);
        assert!(
            p.supports_top_k,
            "omitted profile → supports_top_k must be true"
        );
        assert!(
            p.supports_tools,
            "omitted profile → supports_tools must be true"
        );
        assert!(
            p.supports_response_format,
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
            name: "test-envkey".into(),
            base_url: "https://envkey.test/v1".into(),
            api_key: Some("env:AIMUX_TEST_OVERLAY_KEY".into()),
            protocol: "openai_compat".into(),
            env_var: None,
            display: None,
            profile: ProviderProfile::default(),
            headers: None,
            organization: None,
            project: None,
            max_retries: None,
            body_overrides: None,
            comment: None,
        })
        .unwrap();
        // provider() with api_key=None must resolve via the "env:" reference.
        let model = provider("test-envkey", None, "m", None).unwrap();
        assert_eq!(model.provider(), "test-envkey");
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
            name: "test-envkey-missing".into(),
            base_url: "https://missing.test/v1".into(),
            api_key: Some("env:AIMUX_TEST_OVERLAY_MISSING".into()),
            protocol: "openai_compat".into(),
            env_var: None,
            display: None,
            profile: ProviderProfile::default(),
            headers: None,
            organization: None,
            project: None,
            max_retries: None,
            body_overrides: None,
            comment: None,
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
