//! Provider construction for the console (RFC-0029 §8.2).

use std::sync::Arc;

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_providers::PresetSettings;

/// Build a model from a provider name + model id + optional key / base URL.
///
/// The name is any built-in provider (a vendor package or a registry row); a
/// `None` key is read from that provider's environment variable when the
/// request is made.
pub fn build_model(
    provider: &str,
    api_key: Option<String>,
    model_id: &str,
    base_url: Option<&str>,
) -> Result<Arc<dyn LanguageModel>, AiMuxError> {
    aimux_providers::create_provider(
        provider,
        PresetSettings {
            api_key: api_key.map(Into::into),
            base_url: base_url.map(str::to_string),
            ..Default::default()
        },
    )?
    .language_model(model_id)
}
