//! How a host of the Messages API changes the request URL and body (aimux
//! extension, RFC-0046 §3.5).
//!
//! Upstream these are the `buildRequestUrl` and `transformRequestBody`
//! callbacks the Vertex package hands `AnthropicMessagesLanguageModel`. Here
//! each is a closed enum: the host picks a variant, the rewrite itself lives
//! with the protocol, and the protocol never names a host.

use serde_json::{Value, json};

/// The `anthropic_version` the Vertex `rawPredict` and `streamRawPredict`
/// endpoints expect in the body.
const VERTEX_ANTHROPIC_VERSION: &str = "vertex-2023-10-16";

/// Where the messages call of a model goes (upstream `buildRequestUrl`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum AnthropicRequestUrl {
    /// `{base_url}/messages`: the first-party API and hosts that mirror it,
    /// Claude Platform on AWS among them (its regional host is the base URL
    /// its endpoint resolves).
    #[default]
    Messages,
    /// `{base_url}/{model id}:rawPredict`, or `:streamRawPredict` when
    /// streaming: Anthropic on Vertex AI.
    VertexRawPredict,
}

impl AnthropicRequestUrl {
    /// The URL of the messages call of `model_id` under `base_url`.
    pub(crate) fn build(self, base_url: &str, model_id: &str, stream: bool) -> String {
        match self {
            Self::Messages => format!("{base_url}/messages"),
            Self::VertexRawPredict => {
                let method = if stream {
                    "streamRawPredict"
                } else {
                    "rawPredict"
                };
                format!("{base_url}/{model_id}:{method}")
            }
        }
    }
}

/// A host's rewrite of the finished request body (upstream
/// `transformRequestBody` between packages). Runs before the user's
/// `transform_request_body`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AnthropicBodyRewrite {
    /// The Vertex `rawPredict` envelope: drop `model` (the URL carries it)
    /// and add `anthropic_version`.
    VertexRawPredict,
}

impl AnthropicBodyRewrite {
    /// `body` rewritten for the host.
    pub(crate) fn apply(self, body: Value) -> Value {
        match self {
            Self::VertexRawPredict => {
                let mut envelope = serde_json::Map::new();
                envelope.insert(
                    "anthropic_version".to_string(),
                    json!(VERTEX_ANTHROPIC_VERSION),
                );
                if let Value::Object(map) = body {
                    envelope.extend(map.into_iter().filter(|(key, _)| key != "model"));
                }
                Value::Object(envelope)
            }
        }
    }
}
