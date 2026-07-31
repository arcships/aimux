//! Declarative macro for generating OpenAI-compatible thin-wrapper providers.
//!
//! Each invocation of [`declare_openai_compat_provider!`] expands to a
//! `<Type>Config` + `<Type>Provider` pair that is behaviourally identical to
//! the hand-written thin wrappers it replaces (RFC-0012 §3.1).
//!
//! # Parameters
//!
//! - `$name` — lowercase identifier stringified into the provider name used by
//!   `OpenAIConfig::with_provider` and [`Provider::name`](aimux_core::provider::Provider::name).
//!   Must equal the original `PROVIDER_NAME` constant of the wrapper.
//! - `$config` — full `XxxConfig` type name (passed as an ident, not built with
//!   `${concat}`, which is still unstable on stable Rust — see rust-lang/rust
//!   #124225). Preserves the public API exactly.
//! - `$provider` — full `XxxProvider` type name (ident).
//! - `$display` — human-readable name passed to `load_api_key`'s `description`.
//! - `$base_url` — default base URL (inlined; equivalent to `DEFAULT_BASE_URL`).
//! - `$env_var` — environment variable name (inlined; equivalent to `ENV_VAR`).
//! - `$profile` — `OpenAICompatProfile` expression selecting provider behaviour.
//!
//! The lowercase provider name (`"deepseek"`) and its TitleCase type prefix
//! (`DeepSeek`) differ in case, so the provider name and the type names are
//! supplied as separate arguments — one cannot be derived from the other in a
//! declarative macro.

/// Declare one OpenAI-compatible thin-wrapper provider.
///
/// See the module docs for the parameter contract.
macro_rules! declare_openai_compat_provider {
    (
        $name:ident,
        $config:ident,
        $provider:ident,
        $display:literal,
        $base_url:literal,
        $env_var:literal,
        $profile:expr
    ) => {
        pub struct $config($crate::openai::OpenAIConfig);

        impl $config {
            pub fn new(api_key: impl Into<String>) -> Self {
                Self(
                    $crate::openai::OpenAIConfig::new(api_key)
                        .with_base_url($base_url)
                        .with_provider(stringify!($name))
                        .with_profile($profile),
                )
            }

            pub fn from_env() -> Result<Self, aimux_core::error::AiMuxError> {
                let key = aimux_provider_utils::load_api_key(None, $env_var, $display)?;
                Ok(Self::new(key))
            }

            pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
                self.0 = self.0.with_base_url(url);
                self
            }
        }

        pub struct $provider($crate::openai::OpenAIProvider);

        impl $provider {
            pub fn new(config: $config) -> Self {
                Self($crate::openai::OpenAIProvider::new(config.0))
            }

            pub fn model(&self, model_id: &str) -> $crate::openai::OpenAIModel {
                self.0.model(model_id)
            }
        }

        impl aimux_core::provider::Provider for $provider {
            fn name(&self) -> &str {
                stringify!($name)
            }

            fn language_model(
                &self,
                model_id: &str,
            ) -> Result<
                Box<dyn aimux_core::language_model::LanguageModel>,
                aimux_core::error::AiMuxError,
            > {
                Ok(Box::new(self.model(model_id)))
            }
        }

        // Module shim preserving the historical `aimux_providers::<name>::<Type>`
        // path (used by tests and downstream). The submodule just re-exports the
        // types defined above, so `pub use openai_compat_registry::*;` in lib.rs
        // makes `aimux_providers::<name>::<Config>` resolve exactly as before.
        pub mod $name {
            pub use super::{$config, $provider};
        }
    };
}

pub(crate) use declare_openai_compat_provider;
