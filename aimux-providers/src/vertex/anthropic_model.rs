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
//! - credentials and the base URL are the Vertex provider's, resolved on every
//!   request (OAuth bearer token; project and location),
//! - errors are Google-shaped,
//! - no URL sources, no structured-output beta and no `strict` tool
//!   definitions.
//!
//! Reference: <https://docs.cloud.google.com/claude-on-vertex-ai>

use std::sync::Arc;

use futures::future::BoxFuture;
use serde_json::{Value, json};

use aimux_core::AiMuxError;
use aimux_core::language_model::SupportedUrls;
use aimux_provider_utils::{FetchFunction, Resolvable};

use super::{Publisher, Resolver};
use crate::anthropic::AnthropicMessagesModel;
use crate::anthropic::config::{
    AnthropicEndpoint, AnthropicModelConfig, AnthropicModelHooks, MessagesUrl, ResolveEndpoint,
};
use crate::anthropic::options::options_name_of;
use crate::shared::Endpoint;

/// `anthropic_version` envelope value required by the Vertex AI `rawPredict` /
/// `streamRawPredict` endpoints.
const ANTHROPIC_VERTEX_VERSION: &str = "vertex-2023-10-16";

/// The provider string of every Anthropic model on Vertex.
const PROVIDER: &str = "googleVertex.anthropic.messages";

/// An Anthropic Claude language model served via Vertex AI.
pub type VertexAnthropicModel = AnthropicMessagesModel;

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

/// The Vertex endpoint of Anthropic's models, resolved for each request: the
/// `.../publishers/anthropic/models` base URL and the provider headers.
struct VertexAnthropicEndpoint(Arc<Resolver>);

impl ResolveEndpoint for VertexAnthropicEndpoint {
    fn endpoint(&self) -> BoxFuture<'_, Result<Endpoint, AiMuxError>> {
        Box::pin(self.0.endpoint(Publisher::Anthropic))
    }
}

/// The model `model_id` on the Vertex endpoint `resolver` resolves for each
/// request.
pub(super) fn model(
    model_id: &str,
    resolver: Arc<Resolver>,
    fetch: Option<FetchFunction>,
) -> VertexAnthropicModel {
    // The model's own `base_url` and `headers` only describe its identity;
    // every request uses the endpoint resolved for it.
    let config = AnthropicModelConfig {
        provider: PROVIDER.to_string(),
        headers: Resolvable::Value(Default::default()),
        fetch,
        supported_urls: SupportedUrls::default(),
        base_url: String::new(),
        provider_options_name: options_name_of(PROVIDER),
        hooks: AnthropicModelHooks {
            messages_url: MessagesUrl::RawPredict,
            prepare_body: Some(raw_predict_envelope),
            failed_response_handler: crate::google::google_failed_response_handler,
            supports_native_structured_output: false,
            supports_strict_tools: false,
        },
        endpoint: AnthropicEndpoint::PerRequest(Arc::new(VertexAnthropicEndpoint(resolver))),
    };
    AnthropicMessagesModel::with_config(model_id.to_string(), config)
}
