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
//!   request (OAuth bearer token or Express API key; project and location),
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

use crate::anthropic::AnthropicMessagesModel;
use crate::anthropic::config::{AnthropicModelConfig, AnthropicModelHooks, TransformRequestBody};
use crate::anthropic::options::CANONICAL;
use crate::shared::Endpoint;

/// `anthropic_version` envelope value required by the Vertex AI `rawPredict` /
/// `streamRawPredict` endpoints.
const ANTHROPIC_VERTEX_VERSION: &str = "vertex-2023-10-16";

/// The provider string of every Anthropic model on Vertex.
const PROVIDER: &str = "googleVertex.anthropic.messages";

/// An Anthropic Claude language model served via Vertex AI.
pub type VertexAnthropicModel = AnthropicMessagesModel;

/// Resolves the endpoint of a request: the `.../publishers/anthropic/models`
/// base URL and the provider headers.
pub(super) type EndpointFn =
    Arc<dyn Fn() -> BoxFuture<'static, Result<Endpoint, AiMuxError>> + Send + Sync>;

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

/// The configuration of one request against `endpoint`.
fn request_config(
    endpoint: Endpoint,
    fetch: Option<FetchFunction>,
    transform_request_body: Option<TransformRequestBody>,
) -> AnthropicModelConfig {
    let Endpoint { base_url, headers } = endpoint;
    let models = base_url.clone();
    let url_base = base_url.clone();
    AnthropicModelConfig {
        provider: PROVIDER.to_string(),
        url: Arc::new(move |path| format!("{url_base}{path}")),
        headers: Resolvable::Value(headers),
        fetch,
        supported_urls: SupportedUrls::default(),
        transform_request_body,
        base_url,
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
        resolve: None,
    }
}

/// The model `model_id` on the Vertex endpoint `endpoint` resolves to for each
/// request.
pub(super) fn model(
    model_id: &str,
    endpoint: EndpointFn,
    fetch: Option<FetchFunction>,
    transform_request_body: Option<TransformRequestBody>,
) -> VertexAnthropicModel {
    let resolve_fetch = fetch.clone();
    let resolve_transform = transform_request_body.clone();
    let mut config = request_config(
        Endpoint {
            base_url: String::new(),
            headers: Default::default(),
        },
        fetch,
        transform_request_body,
    );
    // The model's own fields only describe its identity; every request is
    // built from the endpoint resolved for it.
    config.resolve = Some(Arc::new(move || {
        let endpoint = endpoint.clone();
        let fetch = resolve_fetch.clone();
        let transform = resolve_transform.clone();
        Box::pin(async move { Ok(request_config(endpoint().await?, fetch, transform)) })
    }));
    AnthropicMessagesModel::with_config(model_id.to_string(), config)
}
