//! Anthropic partner models on Vertex AI.
//!
//! Claude on Vertex is the Anthropic Messages API behind the
//! `publishers/anthropic/models/{model}:rawPredict` / `:streamRawPredict`
//! endpoints, so its model is the shared
//! [`AnthropicMessagesModel`](crate::anthropic::AnthropicMessagesModel), the
//! Rust form of `createVertexAnthropic`'s `AnthropicLanguageModel`. Only what
//! Vertex does differently is configured here:
//!
//! - the URL carries the model and the mode,
//! - the body drops `model` and gains `anthropic_version: "vertex-2023-10-16"`
//!   (the version is not a header),
//! - credentials are the Vertex ones (OAuth bearer token or Express API key),
//! - errors are Google-shaped,
//! - no URL sources, no structured-output beta and no `strict` tool
//!   definitions.
//!
//! Reference: <https://docs.cloud.google.com/claude-on-vertex-ai>

use std::sync::Arc;

use serde_json::{Value, json};

use aimux_core::language_model::SupportedUrls;
use aimux_provider_utils::{HeaderMapOpt, Resolvable};

use crate::anthropic::AnthropicMessagesModel;
use crate::anthropic::config::{AnthropicModelConfig, AnthropicModelHooks};
use crate::anthropic::options::CANONICAL;

use super::VertexAuth;

/// `anthropic_version` envelope value required by the Vertex AI `rawPredict` /
/// `streamRawPredict` endpoints.
const ANTHROPIC_VERTEX_VERSION: &str = "vertex-2023-10-16";

/// The provider string of every Anthropic model on Vertex.
const PROVIDER: &str = "googleVertex.anthropic.messages";

/// An Anthropic Claude language model served via Vertex AI.
pub type VertexAnthropicModel = AnthropicMessagesModel;

/// Settings of an Anthropic-on-Vertex model.
///
/// `base_url` is the Vertex AI base URL *without* a `/publishers/{publisher}`
/// suffix (e.g. `.../projects/{project}/locations/{location}`); the publisher
/// (`anthropic`) is appended to it. The parent
/// [`super::VertexProvider::anthropic_model`] strips any existing
/// `/publishers/google` suffix from its configured base URL before building
/// these.
#[derive(Debug, Clone)]
pub struct VertexAnthropicSettings {
    pub base_url: String,
    pub auth: VertexAuth,
}

/// The provider headers of the Vertex credential.
fn credential_headers(auth: &VertexAuth) -> Resolvable<HeaderMapOpt> {
    let mut headers = HeaderMapOpt::new();
    match auth {
        VertexAuth::BearerToken(token) => {
            headers.insert("Authorization".to_string(), Some(format!("Bearer {token}")));
        }
        VertexAuth::ApiKey(key) => {
            headers.insert("x-goog-api-key".to_string(), Some(key.clone()));
        }
    }
    Resolvable::Value(headers)
}

/// Wrap a standard Messages request body in the `rawPredict` envelope: drop
/// `model` (the URL carries it) and add `anthropic_version`.
fn raw_predict_envelope(body: Value) -> Value {
    let mut envelope = serde_json::Map::new();
    envelope.insert(
        "anthropic_version".to_string(),
        json!(ANTHROPIC_VERTEX_VERSION),
    );
    if let Value::Object(map) = body {
        envelope.extend(map.into_iter().filter(|(key, _)| key != "model"));
    }
    Value::Object(envelope)
}

/// The model `model_id` on the Vertex endpoint `settings` describe.
pub(super) fn model(model_id: &str, settings: &VertexAnthropicSettings) -> VertexAnthropicModel {
    let base = settings.base_url.clone();
    let models = format!("{base}/publishers/anthropic/models");
    let url_base = base.clone();
    AnthropicMessagesModel::with_config(
        model_id.to_string(),
        AnthropicModelConfig {
            provider: PROVIDER.to_string(),
            url: Arc::new(move |path| format!("{url_base}{path}")),
            headers: credential_headers(&settings.auth),
            fetch: None,
            supported_urls: SupportedUrls::default(),
            transform_request_body: None,
            base_url: base,
            provider_options_name: CANONICAL.to_string(),
            hooks: AnthropicModelHooks {
                request_url: Some(Arc::new(move |model_id, stream| {
                    let method = if stream {
                        "streamRawPredict"
                    } else {
                        "rawPredict"
                    };
                    format!("{models}/{model_id}:{method}")
                })),
                prepare_body: Some(Arc::new(raw_predict_envelope)),
                failed_response_handler: crate::google::google_failed_response_handler,
                supports_native_structured_output: false,
                supports_strict_tools: false,
            },
        },
    )
}
