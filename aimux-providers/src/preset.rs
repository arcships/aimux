//! Presets: a runtime table parsed once from the embedded registry.
//!
//! A preset is an OpenAI-compatible vendor described by data: a name, a
//! default base URL, the environment variable of its key (or no key at all),
//! an optional environment variable for the base URL and optional template
//! parameters. [`lookup`] finds a descriptor by name; [`PresetProvider::create`]
//! assembles an
//! [`OpenAICompatibleProvider`]; the vendor families that have their own
//! package ([`crate::groq`], [`crate::deepseek`]) contribute their dialect.
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
//! - A preset with [`AuthMode::None`] (a local server) resolves no key, injects
//!   no placeholder and sends no `Authorization` header, unless the caller
//!   gives an explicit key.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock};

use serde::Deserialize;

use futures::future::BoxFuture;

use aimux_core::embedding_model::EmbeddingModel;
use aimux_core::error::AiMuxError;
use aimux_core::image_model::ImageModel;
use aimux_core::language_model::LanguageModel;
use aimux_core::model_catalogue::RuntimeModel;
use aimux_core::provider::{Provider, ProviderDiscovery};
use aimux_provider_utils::{
    FetchFunction, HeaderMapOpt, Resolvable, load_api_key, validate_base_url,
};

use crate::deepseek;
use crate::groq;
use crate::openai_compatible::config::{BaseUrl, ChatDialect};
use crate::openai_compatible::{
    Assembly, ChatProfile, OpenAICompatibleChatModel, OpenAICompatibleEmbeddingModel,
    OpenAICompatibleImageModel, OpenAICompatibleProvider, TransformRequestBody,
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

/// Which chat behavior a preset uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresetFamily {
    /// The generic OpenAI-compatible chat model with the preset defaults.
    OpenAICompatible,
    /// The Groq package's dialect.
    Groq,
    /// The DeepSeek package's dialect.
    DeepSeek,
}

/// A template parameter whose value is derived from another parameter: looked
/// up in `map`, else `otherwise` with `{from}` replaced.
#[derive(Debug, Clone, Copy)]
pub struct DeriveSpec {
    pub from: &'static str,
    pub map: &'static [(&'static str, &'static str)],
    pub otherwise: &'static str,
}

/// A parameter of a templated base URL (`{name}` in the template).
#[derive(Debug, Clone, Copy)]
pub struct ParamSpec {
    pub name: &'static str,
    /// Environment variables tried in order when the parameter is not given.
    pub env: &'static [&'static str],
    pub default: Option<&'static str>,
    /// Set for a derived parameter, which the caller cannot give.
    pub derive: Option<DeriveSpec>,
}

/// One row of the embedded provider registry.
#[derive(Debug, Clone, Copy)]
pub struct PresetDescriptor {
    pub name: &'static str,
    pub display: &'static str,
    pub family: PresetFamily,
    /// Default base URL; a template when `params` is not empty.
    pub base_url: &'static str,
    /// Environment variable of the API key; empty for [`AuthMode::None`].
    pub env_var: &'static str,
    pub auth: AuthMode,
    /// Environment variable holding the base URL (local servers).
    pub base_url_env: Option<&'static str>,
    /// The only max-token key the vendor accepts (`"max_tokens"` or
    /// `"max_completion_tokens"`); `None` sends `max_tokens`.
    pub max_tokens_key: Option<&'static str>,
    pub params: &'static [ParamSpec],
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
    name: &'static str,
    display: &'static str,
    base_url: &'static str,
    #[serde(default, deserialize_with = "registry_optional")]
    env_var: Option<&'static str>,
    #[serde(default, deserialize_with = "registry_optional")]
    auth: Option<&'static str>,
    #[serde(default, deserialize_with = "registry_optional")]
    family: Option<&'static str>,
    #[serde(default, deserialize_with = "registry_optional")]
    base_url_env: Option<&'static str>,
    #[serde(default)]
    params: Vec<RegistryParam>,
    #[serde(default)]
    profile: HashMap<&'static str, &'static str>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RegistryParam {
    name: &'static str,
    #[serde(default, deserialize_with = "registry_optional")]
    env: Option<Vec<&'static str>>,
    #[serde(default, deserialize_with = "registry_optional")]
    default: Option<&'static str>,
    #[serde(default, deserialize_with = "registry_optional")]
    derive: Option<RegistryDerive>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RegistryDerive {
    from: &'static str,
    map: HashMap<&'static str, &'static str>,
    otherwise: &'static str,
}

// Missing optional fields are allowed; explicit null is not a valid value.
fn registry_optional<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}

fn load_registry() -> Vec<PresetEntry> {
    // Leak the parsed JSON once so descriptors can borrow even escaped strings.
    let json: Vec<serde_json::Value> = serde_json::from_str(include_str!("provider_registry.json"))
        .expect("invalid provider_registry.json: expected an array of rows");
    let json = json.leak();
    let mut rows: Vec<RegistryRow> = json
        .iter()
        .map(|row| {
            RegistryRow::deserialize(row)
                .unwrap_or_else(|error| panic!("registry row {}: {error}", row["name"]))
        })
        .collect();
    rows.sort_by_key(|row| row.name);
    let identifier = regex::Regex::new(r"^[a-z][a-z0-9_]*$").unwrap();
    let env_name = regex::Regex::new(r"^[A-Z][A-Z0-9_]*$").unwrap();
    let placeholder = regex::Regex::new(r"\{([^{}]*)\}").unwrap();
    let mut seen = HashSet::new();
    let mut entries = Vec::with_capacity(rows.len());
    for row in rows {
        let check =
            |valid: bool, message: &str| assert!(valid, "registry row '{}': {message}", row.name);
        check(identifier.is_match(row.name), "invalid name");
        check(seen.insert(row.name), "duplicate registry name");
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
        let auth = match row.auth.unwrap_or("api_key") {
            "api_key" => AuthMode::ApiKey,
            "none" => AuthMode::None,
            value => panic!("registry row '{}': unknown auth {value:?}", row.name),
        };
        check(
            match auth {
                AuthMode::ApiKey => row.env_var.is_some_and(|var| !var.is_empty()),
                AuthMode::None => row.env_var.is_none(),
            },
            "env_var must be nonempty for api_key and absent for none",
        );
        let family = match row.family.unwrap_or("openai-compatible") {
            "openai-compatible" => PresetFamily::OpenAICompatible,
            "groq" => PresetFamily::Groq,
            "deepseek" => PresetFamily::DeepSeek,
            value => panic!("registry row '{}': unknown family {value:?}", row.name),
        };
        check(
            row.base_url_env.is_none_or(|var| env_name.is_match(var)),
            "invalid base_url_env name",
        );
        check(
            row.profile.keys().all(|key| *key == "max_tokens_key"),
            "unknown profile key",
        );
        let max_tokens_key = row.profile.get("max_tokens_key").copied();
        check(
            max_tokens_key.is_none_or(|key| matches!(key, "max_tokens" | "max_completion_tokens")),
            "invalid max_tokens_key",
        );
        let mut params = Vec::with_capacity(row.params.len());
        for param in row.params {
            check(identifier.is_match(param.name), "bad param name");
            let derive = param.derive.map(|derive| {
                check(
                    param.env.is_none() && param.default.is_none(),
                    "derived param has env/default",
                );
                DeriveSpec {
                    from: derive.from,
                    map: derive.map.into_iter().collect::<Vec<_>>().leak(),
                    otherwise: derive.otherwise,
                }
            });
            params.push(ParamSpec {
                name: param.name,
                env: param.env.unwrap_or_default().leak(),
                default: param.default,
                derive,
            });
        }
        let names: HashSet<_> = params.iter().map(|param| param.name).collect();
        check(names.len() == params.len(), "duplicate param names");
        for param in &params {
            if let Some(derive) = param.derive {
                check(
                    params
                        .iter()
                        .any(|source| source.name == derive.from && source.derive.is_none()),
                    "derive.from must name a plain param",
                );
            }
        }
        let placeholders: HashSet<_> = placeholder
            .captures_iter(row.base_url)
            .map(|c| c.get(1).unwrap().as_str())
            .collect();
        check(placeholders == names, "URL placeholders differ from params");
        entries.push(PresetEntry {
            descriptor: Box::leak(Box::new(PresetDescriptor {
                name: row.name,
                display: row.display,
                family,
                base_url: row.base_url,
                env_var: row.env_var.unwrap_or_default(),
                auth,
                base_url_env: row.base_url_env,
                max_tokens_key,
                params: params.leak(),
            })),
        });
    }
    entries
}

/// Create the provider of the registry row `name`: the row's base URL, key
/// variable and dialect, with `settings` overriding individual fields. The
/// result is an ordinary [`OpenAICompatibleProvider`].
///
/// # Errors
///
/// [`AiMuxError::NoSuchProvider`] for a name that is not in the registry;
/// otherwise as [`PresetProvider::create`].
pub fn create(
    name: &str,
    settings: PresetSettings,
) -> Result<OpenAICompatibleProvider, AiMuxError> {
    let entry = lookup(name).ok_or_else(|| AiMuxError::NoSuchProvider {
        provider_id: name.to_string(),
    })?;
    PresetProvider::create(entry.descriptor, settings).map(PresetProvider::into_inner)
}

/// Every preset, in name order. Invalid registry data panics on first use.
pub fn entries() -> impl Iterator<Item = &'static PresetEntry> {
    static TABLE: OnceLock<Vec<PresetEntry>> = OnceLock::new();
    TABLE.get_or_init(load_registry).iter()
}

/// The names of every preset, in order.
pub fn names() -> impl Iterator<Item = &'static str> {
    entries().map(|entry| entry.descriptor.name)
}

/// The preset called `name`, or `None`; unknown names never fall back.
#[must_use]
pub fn lookup(name: &str) -> Option<&'static PresetEntry> {
    entries().find(|entry| entry.descriptor.name == name)
}

/// A preset provider: an [`OpenAICompatibleProvider`] configured from a
/// [`PresetDescriptor`].
pub struct PresetProvider {
    descriptor: &'static PresetDescriptor,
    explicit_key: bool,
    inner: OpenAICompatibleProvider,
}

impl PresetProvider {
    /// Create a preset provider from its descriptor.
    ///
    /// # Errors
    ///
    /// `InvalidArgument` for an unusable explicit `base_url`, an undeclared (or
    /// derived) `params` key, or a `params` value that is not a plain segment.
    /// Nothing is read from the environment.
    pub fn create(
        descriptor: &'static PresetDescriptor,
        settings: PresetSettings,
    ) -> Result<Self, AiMuxError> {
        for (name, value) in &settings.params {
            let declared = descriptor
                .params
                .iter()
                .find(|spec| spec.name == name && spec.derive.is_none());
            if declared.is_none() {
                return Err(AiMuxError::InvalidArgument(format!(
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
                BaseUrl::Fixed(validate_base_url(descriptor.base_url)?)
            }
            None => {
                let explicit = settings.params;
                BaseUrl::Lazy(Arc::new(move || resolve_base_url(descriptor, &explicit)))
            }
        };

        let explicit_key = settings.api_key.is_some();
        let credential = match (descriptor.auth, settings.api_key) {
            (_, Some(key)) => Credential::Explicit(key),
            (AuthMode::ApiKey, None) => Credential::Env {
                var: descriptor.env_var.to_string(),
                description: descriptor.display.to_string(),
            },
            (AuthMode::None, None) => Credential::None,
        };

        let profile = match descriptor.family {
            PresetFamily::OpenAICompatible => {
                let mut dialect = ChatDialect::baseline();
                dialect.supports_top_k = true;
                dialect.max_tokens_key = descriptor.max_tokens_key;
                ChatProfile {
                    include_usage: true,
                    supports_structured_outputs: true,
                    supports_multi_part_tool_content: false,
                    dialect,
                }
            }
            PresetFamily::Groq => groq::profile(),
            PresetFamily::DeepSeek => deepseek::profile(),
        };

        Ok(Self {
            descriptor,
            explicit_key,
            inner: OpenAICompatibleProvider::assemble(Assembly {
                name: descriptor.name.to_string(),
                base_url,
                credential,
                fixed_headers: Vec::new(),
                headers: settings.headers,
                query_params: None,
                fetch: settings.fetch,
                transform_request_body: settings.transform_request_body,
                profile,
            })?,
        })
    }

    /// The OpenAI-compatible provider this preset assembled.
    #[must_use]
    pub fn into_inner(self) -> OpenAICompatibleProvider {
        self.inner
    }

    /// The descriptor this preset was created from.
    #[must_use]
    pub fn descriptor(&self) -> &'static PresetDescriptor {
        self.descriptor
    }

    /// A chat model; `provider()` is `"{name}.chat"`.
    #[must_use]
    pub fn chat(&self, model_id: &str) -> OpenAICompatibleChatModel {
        self.inner.chat(model_id)
    }

    /// An embedding model; `provider()` is `"{name}.embedding"`.
    #[must_use]
    pub fn embedding(&self, model_id: &str) -> OpenAICompatibleEmbeddingModel {
        self.inner.embedding(model_id)
    }

    /// An image model; `provider()` is `"{name}.image"`.
    #[must_use]
    pub fn image(&self, model_id: &str) -> OpenAICompatibleImageModel {
        self.inner.image(model_id)
    }

    /// The provider as a function: the default language model, the chat model.
    #[must_use]
    pub fn call(&self, model_id: &str) -> Arc<dyn LanguageModel> {
        self.inner.call(model_id)
    }

    /// Evaluate now what a request would evaluate: the base URL, and the key's
    /// environment variable when no key was given. For entry points that
    /// should refuse a provider that cannot be used (the by-name registry
    /// lookup); the factories and default instances do not call it.
    ///
    /// # Errors
    ///
    /// `InvalidArgument` for a base URL that cannot be resolved,
    /// `LoadApiKey` for an unset key variable.
    pub(crate) fn check_ready(&self) -> Result<(), AiMuxError> {
        self.inner.resolve_base_url()?;
        if self.descriptor.auth == AuthMode::ApiKey && !self.explicit_key {
            load_api_key(None, self.descriptor.env_var, self.descriptor.display)?;
        }
        Ok(())
    }
}

impl Provider for PresetProvider {
    fn language_model(&self, model_id: &str) -> Result<Arc<dyn LanguageModel>, AiMuxError> {
        Ok(self.call(model_id))
    }

    fn embedding_model(&self, model_id: &str) -> Result<Arc<dyn EmbeddingModel>, AiMuxError> {
        Ok(Arc::new(self.embedding(model_id)))
    }

    fn image_model(&self, model_id: &str) -> Result<Arc<dyn ImageModel>, AiMuxError> {
        Ok(Arc::new(self.image(model_id)))
    }
}

impl ProviderDiscovery for PresetProvider {
    /// `GET {base_url}/models`: one exchange, no retry. A keyless preset sends
    /// no `Authorization` header.
    fn list_models(&self) -> BoxFuture<'_, Result<Vec<RuntimeModel>, AiMuxError>> {
        self.inner.list_models()
    }
}

// ── Base URL resolution ──────────────────────────────────────────────────────

fn settable_params(descriptor: &PresetDescriptor) -> Vec<&'static str> {
    descriptor
        .params
        .iter()
        .filter(|spec| spec.derive.is_none())
        .map(|spec| spec.name)
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
        Err(AiMuxError::InvalidArgument(format!(
            "preset '{}': template parameter `{name}` must be a plain segment \
             (letters, digits, `.`, `_`, `-`; no `/`, `@`, `:`, `?`, `#` or spaces), got {value:?}",
            descriptor.name
        )))
    }
}

fn env_value(names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        std::env::var(name)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

/// The base URL a request goes to: the descriptor's base-URL variable, else
/// the template expanded from `explicit`, the environment and the defaults.
fn resolve_base_url(
    descriptor: &PresetDescriptor,
    explicit: &HashMap<String, String>,
) -> Result<String, AiMuxError> {
    if let Some(var) = descriptor.base_url_env
        && let Some(url) = env_value(&[var])
    {
        return validate_base_url(&url)
            .map_err(|e| AiMuxError::InvalidArgument(format!("{var}: {e}")));
    }

    let mut values: HashMap<&str, String> = HashMap::new();
    for spec in descriptor
        .params
        .iter()
        .filter(|spec| spec.derive.is_none())
    {
        let value = explicit
            .get(spec.name)
            .cloned()
            .or_else(|| env_value(spec.env))
            .or_else(|| spec.default.map(str::to_string))
            .ok_or_else(|| {
                let source = if spec.env.is_empty() {
                    String::new()
                } else {
                    format!(" or set {}", spec.env.join(" / "))
                };
                AiMuxError::InvalidArgument(format!(
                    "preset '{}' needs the template parameter `{}`: pass it in `params`{source}, \
                     or pass a concrete `base_url`",
                    descriptor.name, spec.name
                ))
            })?;
        check_param_value(descriptor, spec.name, &value)?;
        values.insert(spec.name, value);
    }
    for spec in descriptor.params.iter() {
        let Some(derive) = spec.derive else { continue };
        let from = values.get(derive.from).cloned().ok_or_else(|| {
            AiMuxError::InvalidArgument(format!(
                "preset '{}': derived parameter `{}` depends on undeclared `{}`",
                descriptor.name, spec.name, derive.from
            ))
        })?;
        let derived = derive
            .map
            .iter()
            .find(|(key, _)| *key == from)
            .map(|(_, host)| (*host).to_string())
            .unwrap_or_else(|| {
                derive
                    .otherwise
                    .replace(&format!("{{{}}}", derive.from), &from)
            });
        values.insert(spec.name, derived);
    }

    let mut url = descriptor.base_url.to_string();
    for (name, value) in &values {
        url = url.replace(&format!("{{{name}}}"), value);
    }
    if url.chars().any(|c| matches!(c, '{' | '}' | '<' | '>')) {
        return Err(AiMuxError::InvalidArgument(format!(
            "preset '{}': base URL {:?} still has an unexpanded placeholder after expanding its \
             parameters",
            descriptor.name, url
        )));
    }
    validate_base_url(&url)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VERTEX: PresetDescriptor = PresetDescriptor {
        name: "vertex_test",
        display: "Vertex Test",
        family: PresetFamily::OpenAICompatible,
        base_url: "https://{host}/v1/projects/{project}/locations/{location}/endpoints/openapi",
        env_var: "AIMUX_TEST_VERTEX_TOKEN",
        auth: AuthMode::ApiKey,
        base_url_env: None,
        max_tokens_key: None,
        params: &[
            ParamSpec {
                name: "project",
                env: &[],
                default: None,
                derive: None,
            },
            ParamSpec {
                name: "location",
                env: &[],
                default: Some("global"),
                derive: None,
            },
            ParamSpec {
                name: "host",
                env: &[],
                default: None,
                derive: Some(DeriveSpec {
                    from: "location",
                    map: &[
                        ("global", "aiplatform.googleapis.com"),
                        ("eu", "aiplatform.eu.rep.googleapis.com"),
                        ("us", "aiplatform.us.rep.googleapis.com"),
                    ],
                    otherwise: "{location}-aiplatform.googleapis.com",
                }),
            },
        ],
    };

    fn explicit(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    #[test]
    fn the_host_follows_the_location() {
        for (location, host) in [
            ("global", "aiplatform.googleapis.com"),
            ("us", "aiplatform.us.rep.googleapis.com"),
            ("eu", "aiplatform.eu.rep.googleapis.com"),
            ("us-central1", "us-central1-aiplatform.googleapis.com"),
        ] {
            let url = resolve_base_url(
                &VERTEX,
                &explicit(&[("project", "p1"), ("location", location)]),
            )
            .unwrap();
            assert_eq!(
                url,
                format!("https://{host}/v1/projects/p1/locations/{location}/endpoints/openapi")
            );
        }
    }

    #[test]
    fn a_missing_parameter_names_itself() {
        let error = resolve_base_url(&VERTEX, &HashMap::new()).unwrap_err();
        assert!(
            matches!(&error, AiMuxError::InvalidArgument(m) if m.contains("`project`")),
            "{error:?}"
        );
    }

    #[test]
    fn values_that_could_change_the_url_are_refused() {
        for bad in [
            "a/b", "a@b", "a:80", "a?x=1", "a#f", "", "..", "a b", "{x}", "p\n",
        ] {
            assert!(
                check_param_value(&VERTEX, "project", bad).is_err(),
                "{bad:?} must be refused"
            );
        }
        for good in ["my-project", "p_1", "example.com", "123456"] {
            assert!(check_param_value(&VERTEX, "project", good).is_ok());
        }
    }
}
