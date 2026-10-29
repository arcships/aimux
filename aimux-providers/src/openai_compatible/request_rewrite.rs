//! How a host of the Chat Completions API changes the request body (aimux
//! extension, RFC-0046 §3.5).
//!
//! Upstream these are the `transformRequestBody` callbacks the Vertex
//! partner packages hand `createOpenAICompatible`. Here the rewrite is a
//! closed enum: the host picks a variant, the rewrite itself lives with the
//! protocol, and the protocol never names a host. A user's own
//! `transform_request_body` runs after it.

use serde_json::Value;

/// A host's rewrite of the finished chat request body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CompatibleBodyRewrite {
    /// The MaaS endpoint on Vertex AI: a default `max_tokens` for the models
    /// that need one (google-vertex-maas-provider.ts:16-27,
    /// `maxOutputTokensByModel`).
    VertexMaas,
    /// The Grok endpoint on Vertex AI takes no `reasoning_effort`
    /// (google-vertex-xai-provider.ts:131-134,
    /// `transformGoogleVertexXaiRequestBody`).
    VertexXai,
}

impl CompatibleBodyRewrite {
    /// `body` rewritten for the host.
    pub(crate) fn apply(self, mut body: Value) -> Value {
        match self {
            Self::VertexMaas => {
                let default = match body.get("model").and_then(Value::as_str) {
                    Some(
                        "meta/llama-4-maverick-17b-128e-instruct-maas"
                        | "meta/llama-4-scout-17b-16e-instruct-maas",
                    ) => 8192,
                    _ => return body,
                };
                if let Some(object) = body.as_object_mut()
                    && !object.contains_key("max_tokens")
                {
                    object.insert("max_tokens".into(), default.into());
                }
                body
            }
            Self::VertexXai => {
                if let Some(object) = body.as_object_mut() {
                    object.remove("reasoning_effort");
                }
                body
            }
        }
    }
}
