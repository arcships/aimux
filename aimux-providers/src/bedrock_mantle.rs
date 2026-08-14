//! Bedrock Mantle provider — a thin OpenAI-compatible wrapper.
//!
//! Exposes an OpenAI-compatible Chat Completions API at
//! `https://bedrock-mantle.{region}.api.aws/v1` (region defaults to `us-east-1`).
//! Provider-specific details are the region-aware base URL and the
//! `BEDROCK_MANTLE_API_KEY` environment variable; everything else is delegated
//! to the shared [`OpenAIProvider`].
//!
//! # Authentication
//!
//! This thin wrapper only supports **Bearer API key** auth via the
//! `BEDROCK_MANTLE_API_KEY` environment variable (the shared OpenAI layer sends
//! it as `Authorization: Bearer <key>`). Bedrock Mantle also accepts AWS SigV4
//! signing, but that cannot ride on the OpenAI shared layer's Bearer auth —
//! SigV4 callers should use the dedicated [`bedrock`](crate::bedrock) provider
//! instead, which signs requests with `service = "bedrock"`.

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;
use aimux_provider_utils::load_api_key;

use crate::openai::{OpenAICompatProfile, OpenAIConfig, OpenAIModel, OpenAIProvider};

const DEFAULT_REGION: &str = "us-east-1";
const ENV_VAR: &str = "BEDROCK_MANTLE_API_KEY";
const PROVIDER_NAME: &str = "bedrock_mantle";

/// Build the Bedrock Mantle base URL for a region:
/// `https://bedrock-mantle.{region}.api.aws/v1`.
fn base_url_for_region(region: &str) -> String {
    format!("https://bedrock-mantle.{}.api.aws/v1", region)
}

/// Resolve the Bedrock Mantle region from the environment.
///
/// Precedence: `BEDROCK_MANTLE_REGION` → `AWS_REGION` → `us-east-1`.
fn resolve_region() -> String {
    std::env::var("BEDROCK_MANTLE_REGION")
        .ok()
        .filter(|r| !r.trim().is_empty())
        .or_else(|| {
            std::env::var("AWS_REGION")
                .ok()
                .filter(|r| !r.trim().is_empty())
        })
        .unwrap_or_else(|| DEFAULT_REGION.to_string())
}

/// Configuration for the Bedrock Mantle provider (wraps [`OpenAIConfig`]).
pub struct BedrockMantleConfig(OpenAIConfig);

impl BedrockMantleConfig {
    /// Create from an API key, deriving the region-aware base URL from the
    /// environment (`BEDROCK_MANTLE_REGION` / `AWS_REGION` / `us-east-1`).
    pub fn new(api_key: impl Into<String>) -> Self {
        let region = resolve_region();
        Self(
            OpenAIConfig::new(api_key)
                .with_base_url(base_url_for_region(&region))
                .with_provider(PROVIDER_NAME)
                .with_profile(OpenAICompatProfile::full()),
        )
    }

    /// Create from the `BEDROCK_MANTLE_API_KEY` environment variable.
    ///
    /// The region is resolved from `BEDROCK_MANTLE_REGION` / `AWS_REGION`
    /// (defaulting to `us-east-1`). Returns an `Auth` error when the API key
    /// is not set.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let key = load_api_key(None, ENV_VAR, "Bedrock Mantle")?;
        Ok(Self::new(key))
    }

    /// Override the base URL (useful for tests / self-hosted endpoints).
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.0 = self.0.with_base_url(url);
        self
    }
}

/// Bedrock Mantle provider — creates [`OpenAIModel`] instances.
pub struct BedrockMantleProvider(OpenAIProvider);

impl BedrockMantleProvider {
    pub fn new(config: BedrockMantleConfig) -> Self {
        Self(OpenAIProvider::new(config.0))
    }

    /// Create a model instance for the given model id
    /// (e.g. `"bedrock_mantle/openai.gpt-oss-120b"`).
    pub fn model(&self, model_id: &str) -> OpenAIModel {
        self.0.model(model_id)
    }
}

impl Provider for BedrockMantleProvider {
    fn name(&self) -> &str {
        PROVIDER_NAME
    }

    fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Ok(Box::new(self.model(model_id)))
    }

    crate::delegate_list_models!();
}
