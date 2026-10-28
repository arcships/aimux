//! Presets: a runtime table parsed once from the embedded registry.
//!
//! A preset is an OpenAI-compatible vendor described by data: a name, a
//! default base URL, the environment variable of its key (or no key at all),
//! an optional environment variable for the base URL and optional template
//! parameters. [`lookup`] finds a descriptor by name; [`create`] assembles an
//! [`OpenAICompatibleProvider`] from it. A vendor that has its own package is
//! not a preset: [`crate::create_provider`] builds it through that package.
//!
//! What is evaluated when, following the AI SDK's `loadApiKey` /
//! `loadSetting` timing:
//!
//! - **Creating** a preset validates the explicit settings only: a base URL
//!   must be `http(s)` with a host, the `params` keys must be declared by the
//!   descriptor and their values must be plain host or path segments. It reads
//!   no environment variable; creating any valid row with default settings
//!   cannot fail.
//! - **Requests** evaluate the rest, on every request: the API key (the
//!   explicit one, or the descriptor's environment variable; unset fails the
//!   request with `LoadApiKey`) and the base URL (explicit, then the
//!   descriptor's base-URL variable, then the template expanded from
//!   explicit parameters, environment and defaults; a missing parameter fails
//!   the request with `InvalidArgument` naming it).
//! - A preset with [`AuthMode::None`] (a local server) needs no key and injects
//!   no placeholder. It sends `Authorization` only with an explicit key or,
//!   when the row names one, a set and nonempty key variable (a runtime
//!   started with `--api-key`).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock};

use serde::Deserialize;
use serde_json::Value;

use aimux_core::error::AiMuxError;
use aimux_core::types::Usage;
use aimux_provider_utils::{FetchFunction, HeaderMapOpt, Resolvable, validate_base_url};

use crate::openai_compatible::chat::usage_from_raw;
use crate::openai_compatible::{
    Assembly, BaseUrl, ChatSettings, ConvertUsage, OpenAICompatibleProvider, ResolveBaseUrl,
    TransformRequestBody,
};
use crate::shared::Credential;

/// How a preset authenticates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMode {
    /// `Authorization: Bearer <key>`; the key comes from the settings or from
    /// the descriptor's environment variable.
    ApiKey,
    /// No credential (a local server): no key is resolved and no
    /// `Authorization` header is sent.
    None,
}

/// A template parameter whose value is derived from another parameter: looked
/// up in `map`, else `otherwise` with `{from}` replaced.
#[derive(Debug, Clone)]
pub struct DeriveSpec {
    pub from: String,
    pub map: Vec<(String, String)>,
    pub otherwise: String,
}

/// A parameter of a templated base URL (`{name}` in the template).
#[derive(Debug, Clone)]
pub struct ParamSpec {
    pub name: String,
    /// Environment variables tried in order when the parameter is not given.
    pub env: Vec<String>,
    pub default: Option<String>,
    /// Set for a derived parameter, which the caller cannot give.
    pub derive: Option<DeriveSpec>,
}

/// One row of the embedded provider registry.
#[derive(Debug, Clone)]
pub struct PresetDescriptor {
    pub name: String,
    pub display: String,
    /// Default base URL; a template when `params` is not empty.
    pub base_url: String,
    /// Environment variable of the API key. Optional for [`AuthMode::None`],
    /// where an unset or empty variable sends no credential.
    pub env_var: String,
    pub auth: AuthMode,
    /// Environment variable holding the base URL (local servers).
    pub base_url_env: Option<String>,
    /// The only max-token key the vendor accepts (`"max_tokens"` or
    /// `"max_completion_tokens"`); `None` sends `max_tokens`.
    pub max_tokens_key: Option<String>,
    /// How chat usage is read (`profile.convert_usage`); `None`: the generic
    /// OpenAI-shaped usage.
    pub(crate) convert_usage: Option<fn(Option<&Value>) -> Usage>,
    pub params: Vec<ParamSpec>,
    /// `(package, version)` of the `ai-sdk-{package}/{version}` user-agent
    /// (`profile.user_agent`). `openai-compatible` by default;
    /// `amazon-bedrock` (Bedrock Mantle) also appends it a second time in
    /// the transport, like `@ai-sdk/amazon-bedrock`'s mantle provider.
    pub user_agent: (&'static str, &'static str),
    /// The chat models that take a `json_schema` response format
    /// (`profile.structured_output_models`: exact ids, or a prefix ending in
    /// `*`). `None`: every model; `[]`: none,
    /// which sends `json_object` with a warning for a schema. Rows of vendors
    /// with an AI SDK package follow its `supportsStructuredOutputs`.
    pub structured_output_models: Option<Vec<String>>,
}

/// Settings of a preset factory. Every field is optional.
#[derive(Clone, Default)]
pub struct PresetSettings {
    /// The API key. `None` loads the descriptor's environment variable when a
    /// request is made (nothing at all for [`AuthMode::None`]). An explicit
    /// value is used as given, `""` included, and never falls back to the
    /// environment.
    pub api_key: Option<Resolvable<String>>,
    /// Base URL for the API calls; wins over the descriptor's variable and
    /// template. A trailing slash is removed.
    pub base_url: Option<String>,
    /// Extra headers on every request; a `None` value removes the header.
    pub headers: Option<HeaderMapOpt>,
    /// The transport. `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
    /// Values of the descriptor's template parameters. Only declared,
    /// non-derived parameters are accepted; a value must be a plain segment
    /// (`A-Z a-z 0-9 . _ -`).
    pub params: HashMap<String, String>,
    /// Rewrites every JSON request body once, before it is sent.
    pub transform_request_body: Option<TransformRequestBody>,
}

impl std::fmt::Debug for PresetSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PresetSettings")
            .field("api_key", &self.api_key)
            .field("base_url", &self.base_url)
            .field(
                "headers",
                &self.headers.as_ref().map(std::collections::HashMap::len),
            )
            .field("fetch", &self.fetch.is_some())
            .field("params", &self.params)
            .field(
                "transform_request_body",
                &self.transform_request_body.is_some(),
            )
            .finish()
    }
}

/// A registry descriptor found by the by-name entry points.
#[derive(Clone, Copy)]
pub struct PresetEntry {
    pub descriptor: &'static PresetDescriptor,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RegistryRow {
    name: String,
    display: String,
    base_url: String,
    #[serde(default, deserialize_with = "registry_optional")]
    env_var: Option<String>,
    #[serde(default, deserialize_with = "registry_optional")]
    auth: Option<String>,
    #[serde(default, deserialize_with = "registry_optional")]
    base_url_env: Option<String>,
    #[serde(default)]
    params: Vec<RegistryParam>,
    #[serde(default)]
    profile: RegistryProfile,
}

/// A row's chat settings that differ from the preset defaults.
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RegistryProfile {
    #[serde(default, deserialize_with = "registry_optional")]
    max_tokens_key: Option<String>,
    #[serde(default, deserialize_with = "registry_optional")]
    convert_usage: Option<String>,
    #[serde(default, deserialize_with = "registry_optional")]
    user_agent: Option<String>,
    #[serde(default, deserialize_with = "registry_optional")]
    structured_output_models: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RegistryParam {
    name: String,
    #[serde(default, deserialize_with = "registry_optional")]
    env: Option<Vec<String>>,
    #[serde(default, deserialize_with = "registry_optional")]
    default: Option<String>,
    #[serde(default, deserialize_with = "registry_optional")]
    derive: Option<RegistryDerive>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RegistryDerive {
    from: String,
    map: HashMap<String, String>,
    otherwise: String,
}

// Missing optional fields are allowed; explicit null is not a valid value.
fn registry_optional<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}

/// Parse and check the registry rows in `json`. Any row that breaks the
/// schema panics with the row's name and the rule it breaks.
fn parse_registry(json: &str) -> Vec<PresetDescriptor> {
    let json: Vec<serde_json::Value> =
        serde_json::from_str(json).expect("invalid registry: expected an array of rows");
    let mut rows: Vec<RegistryRow> = json
        .iter()
        .map(|row| {
            RegistryRow::deserialize(row)
                .unwrap_or_else(|error| panic!("registry row {}: {error}", row["name"]))
        })
        .collect();
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    let identifier = regex::Regex::new(r"^[a-z][a-z0-9_]*$").unwrap();
    let env_name = regex::Regex::new(r"^[A-Z][A-Z0-9_]*$").unwrap();
    let placeholder = regex::Regex::new(r"\{([^{}]*)\}").unwrap();
    let mut seen = HashSet::new();
    let mut descriptors = Vec::with_capacity(rows.len());
    for row in rows {
        let name = row.name.clone();
        let check = |valid: bool, message: &str| assert!(valid, "registry row '{name}': {message}");
        check(identifier.is_match(&row.name), "invalid name");
        check(seen.insert(row.name.clone()), "duplicate registry name");
        check(!row.display.is_empty(), "missing display");
        check(
            row.base_url.starts_with("http://") || row.base_url.starts_with("https://"),
            "base_url must start with http(s)://",
        );
        check(row.base_url.is_ascii(), "base_url must be ASCII");
        check(
            !row.base_url.contains(['<', '>', '$']),
            "base_url contains <, > or $",
        );
        let auth = match row.auth.as_deref().unwrap_or("api_key") {
            "api_key" => AuthMode::ApiKey,
            "none" => AuthMode::None,
            value => panic!("registry row '{name}': unknown auth {value:?}"),
        };
        check(
            match auth {
                AuthMode::ApiKey => row.env_var.as_ref().is_some_and(|var| !var.is_empty()),
                AuthMode::None => row
                    .env_var
                    .as_ref()
                    .is_none_or(|var| env_name.is_match(var)),
            },
            "env_var must be nonempty for api_key and a valid name if present for none",
        );
        check(
            row.base_url_env
                .as_ref()
                .is_none_or(|var| env_name.is_match(var)),
            "invalid base_url_env name",
        );
        // bedrock-mantle-provider.ts:236-240 (getHeaders) names the
        // amazon-bedrock package, not openai-compatible.
        let package = match row.profile.user_agent.as_deref() {
            None => crate::shared::aisdk_pins::OPENAI_COMPATIBLE,
            Some("amazon-bedrock") => crate::shared::aisdk_pins::AMAZON_BEDROCK,
            Some(value) => panic!("registry row '{name}': unknown user_agent {value:?}"),
        };
        let user_agent = (package.name, package.version);
        let max_tokens_key = row.profile.max_tokens_key;
        check(
            max_tokens_key
                .as_deref()
                .is_none_or(|key| matches!(key, "max_tokens" | "max_completion_tokens")),
            "invalid max_tokens_key",
        );
        let convert_usage: Option<fn(Option<&Value>) -> Usage> =
            match row.profile.convert_usage.as_deref() {
                None => None,
                Some("alibaba") => Some(alibaba_usage),
                Some("moonshotai") => Some(moonshotai_usage),
                Some(value) => panic!("registry row '{name}': unknown convert_usage {value:?}"),
            };
        let mut params = Vec::with_capacity(row.params.len());
        for param in row.params {
            check(identifier.is_match(&param.name), "bad param name");
            let derive = param.derive.map(|derive| {
                check(
                    param.env.is_none() && param.default.is_none(),
                    "derived param has env/default",
                );
                DeriveSpec {
                    from: derive.from,
                    map: derive.map.into_iter().collect(),
                    otherwise: derive.otherwise,
                }
            });
            params.push(ParamSpec {
                name: param.name,
                env: param.env.unwrap_or_default(),
                default: param.default,
                derive,
            });
        }
        let names: HashSet<&str> = params.iter().map(|param| param.name.as_str()).collect();
        check(names.len() == params.len(), "duplicate param names");
        for param in &params {
            if let Some(derive) = &param.derive {
                check(
                    params
                        .iter()
                        .any(|source| source.name == derive.from && source.derive.is_none()),
                    "derive.from must name a plain param",
                );
            }
        }
        let placeholders: HashSet<&str> = placeholder
            .captures_iter(&row.base_url)
            .map(|c| c.get(1).unwrap().as_str())
            .collect();
        check(placeholders == names, "URL placeholders differ from params");
        descriptors.push(PresetDescriptor {
            name: row.name,
            display: row.display,
            base_url: row.base_url,
            env_var: row.env_var.unwrap_or_default(),
            auth,
            base_url_env: row.base_url_env,
            max_tokens_key,
            convert_usage,
            params,
            user_agent,
            structured_output_models: row.profile.structured_output_models,
        });
    }
    descriptors
}

/// Create the provider of the registry row `name`: the row's base URL, key
/// variable and chat settings, with `settings` overriding individual fields. The
/// result is an ordinary [`OpenAICompatibleProvider`]; nothing is read from
/// the environment here.
///
/// # Errors
///
/// [`AiMuxError::NoSuchProvider`] for a name that is not in the registry,
/// `InvalidArgument` for an unusable explicit `base_url`, an undeclared (or
/// derived) `params` key, or a `params` value that is not a plain segment.
pub fn create(
    name: &str,
    settings: PresetSettings,
) -> Result<OpenAICompatibleProvider, AiMuxError> {
    let entry = lookup(name).ok_or_else(|| AiMuxError::NoSuchProvider {
        provider_id: name.to_string(),
        model_id: name.to_string(),
        model_type: String::new(),
        available_providers: entries()
            .map(|entry| entry.descriptor.name.to_string())
            .collect(),
    })?;
    assemble(entry.descriptor, settings)
}

/// Every preset, in name order. Invalid registry data panics on first use.
///
/// The registry is parsed once into a process-wide table; the entries borrow
/// from it.
pub fn entries() -> impl Iterator<Item = &'static PresetEntry> {
    static DESCRIPTORS: OnceLock<Vec<PresetDescriptor>> = OnceLock::new();
    static TABLE: OnceLock<Vec<PresetEntry>> = OnceLock::new();
    TABLE
        .get_or_init(|| {
            DESCRIPTORS
                .get_or_init(|| parse_registry(include_str!("provider_registry.json")))
                .iter()
                .map(|descriptor| PresetEntry { descriptor })
                .collect()
        })
        .iter()
}

/// The names of every preset, in order.
pub fn names() -> impl Iterator<Item = &'static str> {
    entries().map(|entry| entry.descriptor.name.as_str())
}

/// The preset called `name`, or `None`; unknown names never fall back.
#[must_use]
pub fn lookup(name: &str) -> Option<&'static PresetEntry> {
    entries().find(|entry| entry.descriptor.name == name)
}

fn assemble(
    descriptor: &'static PresetDescriptor,
    settings: PresetSettings,
) -> Result<OpenAICompatibleProvider, AiMuxError> {
    for (name, value) in &settings.params {
        let declared = descriptor
            .params
            .iter()
            .find(|spec| &spec.name == name && spec.derive.is_none());
        if declared.is_none() {
            return Err(AiMuxError::invalid_argument(format!(
                "preset '{}' has no template parameter `{name}`; declared parameters: [{}]",
                descriptor.name,
                settable_params(descriptor).join(", ")
            )));
        }
        check_param_value(descriptor, name, value)?;
    }

    let base_url = match settings.base_url.as_deref() {
        Some(url) => BaseUrl::Fixed(validate_base_url(url)?),
        None if descriptor.params.is_empty() && descriptor.base_url_env.is_none() => {
            BaseUrl::Fixed(validate_base_url(&descriptor.base_url)?)
        }
        None => BaseUrl::Dynamic(Arc::new(PresetBaseUrl {
            descriptor,
            params: settings.params,
        })),
    };

    let credential = match (descriptor.auth, settings.api_key) {
        (_, Some(key)) => Credential::Explicit(key),
        (AuthMode::ApiKey, None) => Credential::Env {
            var: descriptor.env_var.clone(),
            description: descriptor.display.clone(),
        },
        (AuthMode::None, None) if descriptor.env_var.is_empty() => Credential::None,
        (AuthMode::None, None) => Credential::OptionalEnv(descriptor.env_var.clone()),
    };

    let chat = ChatSettings {
        include_usage: true,
        supports_structured_outputs: true,
        structured_output_models: descriptor.structured_output_models.as_deref(),
        supports_top_k: true,
        max_tokens_key: descriptor.max_tokens_key.as_deref(),
        convert_usage: descriptor
            .convert_usage
            .map(|convert| Arc::new(convert) as ConvertUsage),
        ..ChatSettings::default()
    };

    // The Bedrock Mantle transport appends the package part a second time,
    // after `ai-sdk-provider-utils/<v>` (amazon-bedrock-sigv4-fetch.ts:126-130,
    // the API-key fetch of bedrock-mantle-provider.ts). The assembly names
    // the package's provider-utils in that part.
    let (package, fetch) = match descriptor.user_agent.0 {
        "amazon-bedrock" => {
            let package = crate::shared::aisdk_pins::AMAZON_BEDROCK;
            let fetch: FetchFunction = Arc::new(crate::shared::UserAgentFetch {
                inner: settings.fetch,
                provider_utils: None,
                suffix: Some(format!("ai-sdk-{}/{}", package.name, package.version)),
            });
            (package, Some(fetch))
        }
        _ => (crate::shared::aisdk_pins::OPENAI_COMPATIBLE, settings.fetch),
    };
    OpenAICompatibleProvider::assemble(Assembly {
        name: descriptor.name.clone(),
        base_url,
        credential,
        fixed_headers: Vec::new(),
        headers: settings.headers,
        query_params: None,
        fetch,
        transform_request_body: settings.transform_request_body,
        chat,
        package,
    })
}

// ── Base URL resolution ──────────────────────────────────────────────────────

fn settable_params(descriptor: &PresetDescriptor) -> Vec<&str> {
    descriptor
        .params
        .iter()
        .filter(|spec| spec.derive.is_none())
        .map(|spec| spec.name.as_str())
        .collect()
}

/// A template parameter value must be a plain host or path segment: letters,
/// digits, `.`, `_` and `-`. That keeps a value from adding a path, user
/// information, a port or a query to the URL it is expanded into.
fn check_param_value(
    descriptor: &PresetDescriptor,
    name: &str,
    value: &str,
) -> Result<(), AiMuxError> {
    let plain = !value.is_empty()
        && value.len() <= 253
        && value.chars().any(|c| c != '.')
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if plain {
        Ok(())
    } else {
        Err(AiMuxError::invalid_argument(format!(
            "preset '{}': template parameter `{name}` must be a plain segment \
             (letters, digits, `.`, `_`, `-`; no `/`, `@`, `:`, `?`, `#` or spaces), got {value:?}",
            descriptor.name
        )))
    }
}

fn env_value(names: &[impl AsRef<str>]) -> Option<String> {
    names.iter().find_map(|name| {
        std::env::var(name.as_ref())
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

/// The base URL of a preset whose URL is read from the environment or
/// expanded from template parameters, evaluated per request so creating the
/// provider (and the preset default instances) never reads the environment
/// and never fails.
#[derive(Debug)]
struct PresetBaseUrl {
    descriptor: &'static PresetDescriptor,
    /// The explicit template parameters of the settings.
    params: HashMap<String, String>,
}

impl ResolveBaseUrl for PresetBaseUrl {
    fn resolve(&self) -> Result<String, AiMuxError> {
        resolve_base_url(self.descriptor, &self.params)
    }
}

/// The base URL a request goes to: the descriptor's base-URL variable, else
/// the template expanded from `explicit`, the environment and the defaults.
fn resolve_base_url(
    descriptor: &PresetDescriptor,
    explicit: &HashMap<String, String>,
) -> Result<String, AiMuxError> {
    if let Some(var) = descriptor.base_url_env.as_deref()
        && let Some(url) = env_value(&[var])
    {
        return validate_base_url(&url).map_err(|e| AiMuxError::InvalidArgument {
            message: format!("{var}: {e}"),
            cause: Some(e.into()),
        });
    }

    let mut values: HashMap<&str, String> = HashMap::new();
    for spec in descriptor
        .params
        .iter()
        .filter(|spec| spec.derive.is_none())
    {
        let value = explicit
            .get(&spec.name)
            .cloned()
            .or_else(|| env_value(&spec.env))
            .or_else(|| spec.default.clone())
            .ok_or_else(|| {
                let source = if spec.env.is_empty() {
                    String::new()
                } else {
                    format!(" or set {}", spec.env.join(" / "))
                };
                AiMuxError::invalid_argument(format!(
                    "preset '{}' needs the template parameter `{}`: pass it in `params`{source}, \
                     or pass a concrete `base_url`",
                    descriptor.name, spec.name
                ))
            })?;
        check_param_value(descriptor, &spec.name, &value)?;
        values.insert(&spec.name, value);
    }
    for spec in descriptor.params.iter() {
        let Some(derive) = &spec.derive else { continue };
        let from = values.get(derive.from.as_str()).cloned().ok_or_else(|| {
            AiMuxError::invalid_argument(format!(
                "preset '{}': derived parameter `{}` depends on undeclared `{}`",
                descriptor.name, spec.name, derive.from
            ))
        })?;
        let derived = derive
            .map
            .iter()
            .find(|(key, _)| *key == from)
            .map(|(_, host)| host.clone())
            .unwrap_or_else(|| {
                derive
                    .otherwise
                    .replace(&format!("{{{}}}", derive.from), &from)
            });
        values.insert(&spec.name, derived);
    }

    let mut url = descriptor.base_url.clone();
    for (name, value) in &values {
        url = url.replace(&format!("{{{name}}}"), value);
    }
    if url.chars().any(|c| matches!(c, '{' | '}' | '<' | '>')) {
        return Err(AiMuxError::invalid_argument(format!(
            "preset '{}': base URL {:?} still has an unexpanded placeholder after expanding its \
             parameters",
            descriptor.name, url
        )));
    }
    validate_base_url(&url)
}

/// Alibaba's `convertUsage`: cache writes from
/// `prompt_tokens_details.cache_creation_input_tokens` (or
/// `cache_write_tokens`), taken out of the uncached input.
fn alibaba_usage(raw: Option<&Value>) -> Usage {
    let mut usage = usage_from_raw(raw);
    if let Some(raw) = raw.filter(|raw| !raw.is_null()) {
        let details = &raw["prompt_tokens_details"];
        let cache_write = tokens(&details["cache_creation_input_tokens"])
            .or_else(|| tokens(&details["cache_write_tokens"]))
            .unwrap_or(0);
        usage.input_tokens.cache_write = Some(cache_write);
        usage.input_tokens.no_cache = usage
            .input_tokens
            .no_cache
            .map(|tokens| tokens.saturating_sub(cache_write));
    }
    usage
}

/// Moonshot AI's `convertUsage`: cache reads from the top-level
/// `cached_tokens`.
fn moonshotai_usage(raw: Option<&Value>) -> Usage {
    let mut usage = usage_from_raw(raw);
    if let Some(cached) = raw.and_then(|raw| tokens(&raw["cached_tokens"])) {
        usage.input_tokens.cache_read = Some(cached);
        usage.input_tokens.no_cache = usage
            .input_tokens
            .total
            .map(|tokens| tokens.saturating_sub(cached));
    }
    usage
}

/// A token count of a raw usage object.
fn tokens(value: &Value) -> Option<u32> {
    value.as_u64().and_then(|value| u32::try_from(value).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The registry is parsed once: every lookup hands out the same
    /// descriptor, owned by the process-wide table (nothing is leaked per
    /// call).
    #[test]
    fn the_table_is_parsed_once_and_shared() {
        let first = lookup("abacus").expect("abacus is a registry row");
        let again = lookup("abacus").expect("abacus is a registry row");
        assert!(std::ptr::eq(first.descriptor, again.descriptor));
        let from_entries = entries()
            .find(|entry| entry.descriptor.name == "abacus")
            .expect("abacus is listed");
        assert!(std::ptr::eq(first.descriptor, from_entries.descriptor));
        assert_eq!(names().count(), entries().count());
    }

    fn registry(rows: serde_json::Value) -> Vec<PresetDescriptor> {
        parse_registry(&rows.to_string())
    }

    fn row(fields: serde_json::Value) -> serde_json::Value {
        let mut row = serde_json::json!({
            "name": "vendor",
            "display": "Vendor",
            "base_url": "https://api.vendor.example/v1",
            "env_var": "VENDOR_API_KEY",
        });
        row.as_object_mut()
            .unwrap()
            .extend(fields.as_object().cloned().unwrap());
        row
    }

    #[test]
    fn a_valid_row_parses_into_a_descriptor() {
        let descriptors = registry(serde_json::json!([
            row(serde_json::json!({})),
            row(serde_json::json!({
                "name": "local",
                "base_url": "http://localhost:{port}/v1",
                "auth": "none",
                "env_var": "LOCAL_API_KEY",
                "base_url_env": "LOCAL_BASE_URL",
                "params": [{ "name": "port", "default": "8080" }],
                "profile": { "max_tokens_key": "max_completion_tokens" },
            })),
        ]));
        assert_eq!(descriptors.len(), 2);
        let local = &descriptors[0];
        assert_eq!(local.name, "local");
        assert_eq!(local.auth, AuthMode::None);
        assert_eq!(local.base_url_env.as_deref(), Some("LOCAL_BASE_URL"));
        assert_eq!(
            local.max_tokens_key.as_deref(),
            Some("max_completion_tokens")
        );
        assert_eq!(local.params[0].default.as_deref(), Some("8080"));
        let vendor = &descriptors[1];
        assert_eq!(vendor.auth, AuthMode::ApiKey);
        assert_eq!(vendor.env_var, "VENDOR_API_KEY");
    }

    #[test]
    #[should_panic(expected = "registry row 'vendor': duplicate registry name")]
    fn a_name_is_listed_once() {
        registry(serde_json::json!([
            row(serde_json::json!({})),
            row(serde_json::json!({}))
        ]));
    }

    #[test]
    #[should_panic(expected = "registry row 'Vendor-1': invalid name")]
    fn a_name_is_a_lowercase_identifier() {
        registry(serde_json::json!([row(
            serde_json::json!({ "name": "Vendor-1" })
        )]));
    }

    #[test]
    #[should_panic(expected = "base_url must start with http(s)://")]
    fn a_base_url_is_http() {
        registry(serde_json::json!([row(
            serde_json::json!({ "base_url": "ftp://vendor" })
        )]));
    }

    #[test]
    #[should_panic(expected = "unknown auth \"oauth\"")]
    fn auth_is_api_key_or_none() {
        registry(serde_json::json!([row(
            serde_json::json!({ "auth": "oauth" })
        )]));
    }

    #[test]
    #[should_panic(expected = "env_var must be nonempty for api_key")]
    fn an_api_key_row_names_its_variable() {
        registry(serde_json::json!([row(
            serde_json::json!({ "env_var": "" })
        )]));
    }

    #[test]
    #[should_panic(expected = "URL placeholders differ from params")]
    fn every_placeholder_is_a_param() {
        registry(serde_json::json!([row(serde_json::json!({
            "base_url": "https://{region}.vendor.example/v1",
        }))]));
    }

    #[test]
    #[should_panic(expected = "derived param has env/default")]
    fn a_derived_param_has_no_env_or_default() {
        registry(serde_json::json!([row(serde_json::json!({
            "base_url": "https://{region}.{host}/v1",
            "params": [
                { "name": "region", "default": "eu" },
                {
                    "name": "host",
                    "default": "x",
                    "derive": { "from": "region", "map": {}, "otherwise": "vendor.example" },
                },
            ],
        }))]));
    }

    #[test]
    #[should_panic(expected = "registry row \"vendor\"")]
    fn an_explicit_null_is_not_a_value() {
        registry(serde_json::json!([row(
            serde_json::json!({ "env_var": null })
        )]));
    }

    #[test]
    #[should_panic(expected = "registry row \"vendor\"")]
    fn an_unknown_profile_key_is_refused() {
        registry(serde_json::json!([row(
            serde_json::json!({ "profile": { "supports_top_k": "false" } })
        )]));
    }

    #[test]
    fn a_structured_output_model_list_is_read() {
        let descriptors = registry(serde_json::json!([
            row(serde_json::json!({
                "profile": { "structured_output_models": ["model-a"] }
            })),
            row(serde_json::json!({ "name": "other", "profile": {} })),
        ]));
        assert_eq!(descriptors[0].structured_output_models, None);
        assert_eq!(
            descriptors[1].structured_output_models,
            Some(vec!["model-a".to_string()])
        );
    }

    #[test]
    #[should_panic(expected = "registry row \"vendor\"")]
    fn an_unknown_field_is_refused() {
        registry(serde_json::json!([row(
            serde_json::json!({ "region": "eu" })
        )]));
    }
}
