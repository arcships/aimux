//! Standard vendor settings, factory constructors, and singleton helpers.

use aimux_core::error::AiMuxError;
use aimux_provider_utils::{
    AuthScheme, Credential, FetchFunction, HeaderMapOpt, ProviderHeaders, Resolvable,
    validate_base_url,
};

use super::EndpointConfig;
use super::aisdk_pins::Package;

/// Static configuration needed to construct a standard five-field vendor provider.
pub(crate) struct VendorFactoryConfig {
    pub display: &'static str,
    pub default_base_url: &'static str,
    pub api_key_env_var: &'static str,
    pub default_name: &'static str,
    pub auth: AuthScheme,
    pub extra_headers: Vec<(String, String)>,
    pub package: Option<Package>,
}

/// Validated runtime fields held by a standard vendor provider.
pub(crate) struct VendorParts {
    name: String,
    base_url: String,
    headers: ProviderHeaders,
    fetch: Option<FetchFunction>,
}

impl VendorParts {
    /// Build an [`EndpointConfig`] for the named modality method (`"search"`, `"image"`, etc.).
    pub(crate) fn model_config(&self, method: &str) -> EndpointConfig {
        EndpointConfig::fixed(
            format!("{}.{method}", self.name),
            self.base_url.clone(),
            self.headers.clone(),
            self.fetch.clone(),
            None,
        )
    }
}

/// Validate `base_url` and assemble [`VendorParts`] from standard five-field settings.
pub(crate) fn build_vendor_parts(
    base_url: Option<String>,
    api_key: Option<Resolvable<String>>,
    user_headers: Option<HeaderMapOpt>,
    name: Option<String>,
    fetch: Option<FetchFunction>,
    cfg: VendorFactoryConfig,
) -> Result<VendorParts, AiMuxError> {
    let base_url = match base_url.as_deref() {
        Some(url) => validate_base_url(url)?,
        None => cfg.default_base_url.to_string(),
    };
    let mut headers = ProviderHeaders::new(
        Credential::explicit_or_env(api_key, cfg.api_key_env_var, cfg.display),
        cfg.auth,
        cfg.extra_headers,
        user_headers,
    );
    let fetch = match cfg.package {
        Some(pkg) => {
            headers = headers.with_user_agent(pkg.name, pkg.version);
            Some(crate::shared::package_fetch(fetch, pkg))
        }
        None => fetch,
    };
    Ok(VendorParts {
        name: name.unwrap_or_else(|| cfg.default_name.to_string()),
        base_url,
        headers,
        fetch,
    })
}

/// Redacted `Debug` implementation helper shared by standard `*ProviderSettings` structs.
pub(crate) fn debug_vendor_settings(
    f: &mut std::fmt::Formatter<'_>,
    struct_name: &str,
    base_url: &Option<String>,
    api_key: &Option<Resolvable<String>>,
    headers: &Option<HeaderMapOpt>,
    name: &Option<String>,
    has_fetch: bool,
) -> std::fmt::Result {
    f.debug_struct(struct_name)
        .field("base_url", base_url)
        .field("api_key", api_key)
        .field(
            "headers",
            &headers.as_ref().map(std::collections::HashMap::len),
        )
        .field("name", name)
        .field("fetch", &has_fetch)
        .finish()
}

/// Generate a standard 5-field `*ProviderSettings` (with redacted `Debug`),
/// `create_<vendor>`, `<vendor>()` (`OnceLock`), and `<Vendor>Provider` with `model_config`.
macro_rules! define_vendor_provider {
    (
        settings: $Settings:ident,
        provider: $Provider:ident,
        create: $create_fn:ident,
        singleton: $singleton_fn:ident,
        display: $display:literal,
        default_base_url: $default_base_url:expr,
        api_key_env_var: $api_key_env_var:expr,
        default_name: $default_name:expr,
        auth: $auth:expr,
        extra_headers: $extra_headers:expr,
        package: $package:expr $(,)?
    ) => {
        #[doc = concat!("Settings of [`", stringify!($create_fn), "`].")]
        ///
        /// Every field is optional. Nothing here is evaluated when the provider is
        /// created except `base_url` and `name`; `api_key` and `headers` are
        /// evaluated on every request.
        #[derive(Clone, Default)]
        pub struct $Settings {
            /// Base URL for the API calls; trailing slashes are removed.
            pub base_url: Option<String>,
            /// The API key. `None` loads the environment variable when a request is
            /// made and fails that request with `AiMuxError::LoadApiKey` if it is
            /// unset. An explicit value is used as given, `""` included: it never
            /// falls back to the environment. A
            /// [`Resolvable::Future`](::aimux_provider_utils::Resolvable::Future) is
            /// awaited once, a
            /// [`Resolvable::AsyncFn`](::aimux_provider_utils::Resolvable::AsyncFn)
            /// on every request.
            pub api_key: Option<::aimux_provider_utils::Resolvable<String>>,
            /// Extra headers on every request. A `None` value removes the header,
            /// including the credential. Per-call headers win over these.
            pub headers: Option<::aimux_provider_utils::HeaderMapOpt>,
            /// The provider name, prefix of the `provider()` strings.
            pub name: Option<String>,
            /// The transport: a mock, a signing decorator, a proxy-aware client.
            /// `None` uses the process default, resolved per request.
            pub fetch: Option<::aimux_provider_utils::FetchFunction>,
        }

        impl ::std::fmt::Debug for $Settings {
            /// Never prints the key or header values.
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                crate::shared::debug_vendor_settings(
                    f,
                    stringify!($Settings),
                    &self.base_url,
                    &self.api_key,
                    &self.headers,
                    &self.name,
                    self.fetch.is_some(),
                )
            }
        }

        #[doc = concat!("Create a ", $display, " provider.")]
        ///
        /// # Errors
        ///
        /// Returns
        /// [`AiMuxError::invalid_argument`](::aimux_core::error::AiMuxError::invalid_argument)
        /// when `base_url` is not an `http(s)` URL with a host. That is the only
        /// way this fails: the key is loaded per request, not here.
        pub fn $create_fn(
            settings: $Settings,
        ) -> ::std::result::Result<$Provider, ::aimux_core::error::AiMuxError> {
            let parts = crate::shared::build_vendor_parts(
                settings.base_url,
                settings.api_key,
                settings.headers,
                settings.name,
                settings.fetch,
                crate::shared::VendorFactoryConfig {
                    display: $display,
                    default_base_url: $default_base_url,
                    api_key_env_var: $api_key_env_var,
                    default_name: $default_name,
                    auth: $auth,
                    extra_headers: $extra_headers,
                    package: $package,
                },
            )?;
            ::std::result::Result::Ok($Provider(parts))
        }

        #[doc = concat!(
            "The default provider: [`",
            stringify!($create_fn),
            "`] with default settings, created on first use. Creating it reads nothing from the environment and cannot fail; a missing key surfaces from the first request instead."
        )]
        pub fn $singleton_fn() -> &'static $Provider {
            static DEFAULT: ::std::sync::OnceLock<$Provider> = ::std::sync::OnceLock::new();
            DEFAULT.get_or_init(|| {
                $create_fn($Settings::default())
                    .expect(concat!("default ", $display, " settings are always valid"))
            })
        }

        #[doc = concat!(
            "A ",
            $display,
            " provider. Cheap to clone the models out of; it holds no HTTP client."
        )]
        pub struct $Provider(crate::shared::VendorParts);

        impl $Provider {
            fn model_config(&self, method: &str) -> crate::shared::EndpointConfig {
                self.0.model_config(method)
            }
        }
    };
}

pub(crate) use define_vendor_provider;
