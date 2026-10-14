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

/// Whether `base_url` points somewhere other than `provider`'s own default
/// endpoint ([`aimux_providers::provider_default_base_url`]). Stored and
/// environment keys are never sent to such a URL: it came from the caller.
///
/// When the default cannot be determined (an unknown provider, an endpoint
/// built from settings such as a region or a template), every given URL
/// counts as custom.
pub fn is_custom_base_url(provider: &str, base_url: Option<&str>) -> bool {
    let Some(url) = base_url else {
        return false;
    };
    aimux_providers::provider_default_base_url(provider)
        .is_none_or(|default| default.trim_end_matches('/') != url.trim_end_matches('/'))
}

/// Providers that read credentials from the server environment besides the
/// `api_key` setting: AWS credentials (SigV4 signing, session token),
/// the Vertex access token, the Codex sign-in, the DataForSEO login. An empty
/// `api_key` does not stop them, so the console does not call them at a
/// caller-chosen base URL.
const AMBIENT_CREDENTIALS: [&str; 6] = [
    "amazon_bedrock",
    "anthropic_aws",
    "aws_polly",
    "codex",
    "dataforseo",
    "google_vertex",
];

/// Whether `provider` may read credentials other than the `api_key` setting
/// from the server environment ([`AMBIENT_CREDENTIALS`]).
pub fn reads_ambient_credentials(provider: &str) -> bool {
    AMBIENT_CREDENTIALS.contains(&provider)
}

#[cfg(test)]
mod tests {
    use super::{AMBIENT_CREDENTIALS, is_custom_base_url};

    #[test]
    fn only_the_providers_own_default_is_not_custom() {
        assert!(!is_custom_base_url("openai", None));
        assert!(!is_custom_base_url(
            "openai",
            Some("https://api.openai.com/v1")
        ));
        assert!(!is_custom_base_url(
            "openai",
            Some("https://api.openai.com/v1/")
        ));
        assert!(!is_custom_base_url(
            "groq",
            Some("https://api.groq.com/openai/v1")
        ));
        assert!(!is_custom_base_url(
            "openrouter",
            Some("https://openrouter.ai/api/v1")
        ));
        assert!(is_custom_base_url(
            "openai",
            Some("https://attacker.example/v1")
        ));
        assert!(is_custom_base_url(
            "groq",
            Some("https://api.openai.com/v1")
        ));
        // No known default: every given URL counts as custom.
        assert!(is_custom_base_url(
            "no-such-provider",
            Some("https://x.example")
        ));
        assert!(is_custom_base_url(
            "amazon_bedrock",
            Some("https://bedrock-runtime.us-east-1.amazonaws.com")
        ));
    }

    #[test]
    fn ambient_credential_providers_are_built_in() {
        let names = aimux_providers::provider_names();
        for name in AMBIENT_CREDENTIALS {
            assert!(names.contains(&name), "{name}");
        }
    }
}
