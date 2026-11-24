//! MiniMax media provider factory and configuration.

use std::sync::OnceLock;

use aimux_core::error::AiMuxError;
use aimux_provider_utils::{AuthScheme, FetchFunction, HeaderMapOpt, Resolvable};

use crate::shared::{EndpointConfig, VendorFactoryConfig, VendorParts, build_vendor_parts};

use super::image_model::MiniMaxImageModel;
use super::video_model::MiniMaxVideoModel;

/// The international endpoint.
pub(crate) const DEFAULT_BASE_URL: &str = "https://api.minimax.io";
/// The China endpoint every China host spelling is normalized to.
pub const MINIMAX_CN_BASE_URL: &str = "https://api.minimax.cn";
pub(crate) const API_KEY_ENV_VAR: &str = "MINIMAX_API_KEY";
const DEFAULT_NAME: &str = "minimax";

/// The hosts of the China endpoint (`api.minimaxi.com` is its older name).
const CN_HOSTS: [&str; 2] = ["api.minimaxi.com", "api.minimax.cn"];

/// The path suffixes a chat base URL carries and the media paths do not.
/// They are removed from the end repeatedly, so `/anthropic/v1` and `/v1/v2`
/// go too; any other prefix (a relay's) stays.
const CHAT_SUFFIXES: [&str; 4] = ["/anthropic/v1", "/anthropic", "/v1", "/v2"];

/// Settings of [`create_minimax`].
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except `base_url` and `name`; `api_key` and `headers` are
/// evaluated on every request.
#[derive(Clone, Default)]
pub struct MiniMaxProviderSettings {
    /// Base URL for the API calls. Default `https://api.minimax.io`.
    ///
    /// A chat base URL is accepted as is: the trailing `/anthropic`,
    /// `/anthropic/v1`, `/v1` and `/v2` segments are removed (repeatedly) and
    /// the media paths are appended to what is left, so a relay keeps its
    /// prefix. `api.minimaxi.com` and `api.minimax.cn` become
    /// `https://api.minimax.cn`. A base URL with a query or a fragment is
    /// rejected: the media path is appended to the string, so a query would
    /// end up inside the path and no request would reach its endpoint.
    pub base_url: Option<String>,
    /// The API key. `None` loads `MINIMAX_API_KEY` when a request is made and
    /// fails that request with `AiMuxError::LoadApiKey` if it is unset. An
    /// explicit value is used as given, `""` included: it never falls back
    /// to the environment. A [`Resolvable::Future`] is awaited once, an
    /// [`Resolvable::AsyncFn`] on every request.
    pub api_key: Option<Resolvable<String>>,
    /// Extra headers on every request. A `None` value removes the header,
    /// including the credential. Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The provider name, the prefix of the `provider()` strings
    /// (`"{name}.video"`, `"{name}.image"`). Default `"minimax"`. The
    /// providerOptions key stays `minimax`.
    pub name: Option<String>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
}

impl std::fmt::Debug for MiniMaxProviderSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        crate::shared::debug_vendor_settings(
            f,
            "MiniMaxProviderSettings",
            &self.base_url,
            &self.api_key,
            &self.headers,
            &self.name,
            self.fetch.is_some(),
        )
    }
}

/// The media base URL of `base_url`: the China host spellings normalized to
/// [`MINIMAX_CN_BASE_URL`] and the chat path suffixes removed. The query and
/// fragment are rejected rather than removed: the media path is appended to
/// the string, so a query left in place would swallow it.
///
/// # Errors
///
/// `InvalidArgument` when `base_url` is not an `http(s)` URL with a host, or
/// when it has a query or a fragment.
pub(crate) fn normalize_base_url(base_url: &str) -> Result<String, AiMuxError> {
    let validated = aimux_provider_utils::validate_base_url(base_url)?;
    let mut url = url::Url::parse(&validated).map_err(|e| AiMuxError::InvalidArgument {
        message: format!("invalid base URL: {e}"),
        cause: Some(aimux_core::ErrorCause::source(&e)),
    })?;
    if url.query().is_some() || url.fragment().is_some() {
        return Err(AiMuxError::invalid_argument(
            "MiniMax base URL must not include a query or a fragment",
        ));
    }
    let is_cn = url
        .host_str()
        .is_some_and(|host| CN_HOSTS.iter().any(|cn| host.eq_ignore_ascii_case(cn)));
    let mut path = url.path().trim_end_matches('/').to_string();
    while let Some(stripped) = CHAT_SUFFIXES
        .iter()
        .find_map(|suffix| strip_suffix_ignore_ascii_case(&path, suffix))
    {
        path = stripped.trim_end_matches('/').to_string();
    }
    if is_cn {
        let cn = url::Url::parse(MINIMAX_CN_BASE_URL).expect("a valid constant");
        url.set_scheme(cn.scheme())
            .and_then(|()| url.set_host(cn.host_str()).map_err(|_| ()))
            .and_then(|()| url.set_port(None))
            .map_err(|()| AiMuxError::invalid_argument("invalid MiniMax base URL"))?;
    }
    url.set_path(&path);
    Ok(aimux_provider_utils::without_trailing_slash(url.as_str()))
}

fn strip_suffix_ignore_ascii_case<'a>(path: &'a str, suffix: &str) -> Option<&'a str> {
    let split = path.len().checked_sub(suffix.len())?;
    (path.is_char_boundary(split) && path[split..].eq_ignore_ascii_case(suffix))
        .then(|| &path[..split])
}

/// Create a MiniMax media provider.
///
/// # Errors
///
/// Returns `AiMuxError::invalid_argument` when `base_url` is not an `http(s)`
/// URL with a host, or when it carries a query or a fragment. That is the
/// only way this fails: the key is loaded per request, not here.
pub fn create_minimax(settings: MiniMaxProviderSettings) -> Result<MiniMaxProvider, AiMuxError> {
    let base_url = settings
        .base_url
        .as_deref()
        .map(normalize_base_url)
        .transpose()?;
    let parts = build_vendor_parts(
        base_url,
        settings.api_key,
        settings.headers,
        settings.name,
        settings.fetch,
        VendorFactoryConfig {
            display: "MiniMax",
            default_base_url: DEFAULT_BASE_URL,
            api_key_env_var: API_KEY_ENV_VAR,
            default_name: DEFAULT_NAME,
            auth: AuthScheme::Bearer,
            extra_headers: Vec::new(),
            package: None,
        },
    )?;
    Ok(MiniMaxProvider(parts))
}

/// The default provider: [`create_minimax`] with default settings, created
/// on first use. Creating it reads nothing from the environment and cannot
/// fail; a missing key surfaces from the first request instead.
pub fn minimax() -> &'static MiniMaxProvider {
    static DEFAULT: OnceLock<MiniMaxProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_minimax(MiniMaxProviderSettings::default())
            .expect("default MiniMax settings are always valid")
    })
}

/// A MiniMax media provider. Cheap to clone the models out of; it holds no
/// HTTP client.
pub struct MiniMaxProvider(VendorParts);

impl MiniMaxProvider {
    fn model_config(&self, method: &str) -> EndpointConfig {
        self.0.model_config(method)
    }

    /// A video model (`"MiniMax-H3"`, `"MiniMax-H3-Highspeed"`,
    /// `"MiniMax-H3-Lite"`, `"MiniMax-H3-Max"`); `provider()` is
    /// `"{name}.video"`.
    #[must_use]
    pub fn video(&self, model_id: &str) -> MiniMaxVideoModel {
        MiniMaxVideoModel::from_config(model_id.to_string(), self.model_config("video"))
    }

    /// An image model (`"MiniMax-H3-Image"`, `"MiniMax-H3-Image-Highspeed"`);
    /// `provider()` is `"{name}.image"`.
    #[must_use]
    pub fn image(&self, model_id: &str) -> MiniMaxImageModel {
        MiniMaxImageModel::from_config(model_id.to_string(), self.model_config("image"))
    }
}

crate::impl_modality_provider!(MiniMaxProvider {
    image_model: |p, id| p.image(id),
    video_model: |p, id| p.video(id),
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_suffixes_are_removed_and_a_relay_prefix_stays() {
        for (input, expected) in [
            ("https://api.minimax.io", "https://api.minimax.io"),
            ("https://api.minimax.io/", "https://api.minimax.io"),
            ("https://api.minimax.io/v1", "https://api.minimax.io"),
            ("https://api.minimax.io/anthropic", "https://api.minimax.io"),
            (
                "https://api.minimax.io/anthropic/v1",
                "https://api.minimax.io",
            ),
            ("https://api.minimax.io/v1/v2/", "https://api.minimax.io"),
            (
                "https://relay.example/minimax/v1",
                "https://relay.example/minimax",
            ),
            (
                "https://relay.example/minimax/anthropic",
                "https://relay.example/minimax",
            ),
            ("http://127.0.0.1:8080/v1", "http://127.0.0.1:8080"),
            // Only whole trailing segments go.
            ("https://relay.example/apiv1", "https://relay.example/apiv1"),
        ] {
            assert_eq!(normalize_base_url(input).unwrap(), expected, "{input}");
        }
    }

    #[test]
    fn china_hosts_become_the_minimax_cn_endpoint() {
        for input in [
            "https://api.minimaxi.com/v1",
            "http://api.minimaxi.com",
            "https://API.MINIMAXI.COM/anthropic/v1",
            "https://api.minimax.cn/v1",
        ] {
            assert_eq!(
                normalize_base_url(input).unwrap(),
                MINIMAX_CN_BASE_URL,
                "{input}"
            );
        }
    }

    #[test]
    fn a_base_url_with_a_query_or_a_fragment_is_rejected() {
        for input in [
            "https://relay.example/minimax/v1?api-version=2024",
            "https://relay.example/minimax?api-version=2024",
            "https://api.minimax.io/v1#fragment",
            "https://api.minimaxi.com/v1?",
        ] {
            let error = normalize_base_url(input).unwrap_err();
            assert!(
                matches!(error, AiMuxError::InvalidArgument { .. }),
                "{input}: {error:?}"
            );
        }
    }

    #[test]
    fn an_invalid_base_url_fails_creation() {
        for input in [
            "",
            "ftp://api.minimax.io",
            "not a url",
            "https://api.minimax.io/v1?api-version=2024",
        ] {
            assert!(
                create_minimax(MiniMaxProviderSettings {
                    base_url: Some(input.to_string()),
                    ..Default::default()
                })
                .is_err(),
                "{input}"
            );
        }
    }

    #[test]
    fn model_provider_strings_use_the_provider_name() {
        use aimux_core::image_model::ImageModel;
        use aimux_core::video_model::VideoModel;
        let provider = create_minimax(MiniMaxProviderSettings {
            name: Some("my-minimax".into()),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(provider.video("MiniMax-H3").provider(), "my-minimax.video");
        assert_eq!(
            provider.image("MiniMax-H3-Image").provider(),
            "my-minimax.image"
        );
        assert_eq!(minimax().video("MiniMax-H3").provider(), "minimax.video");
    }
}
