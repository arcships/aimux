//! Provider options of the Vertex embedding model
//! (google-vertex-embedding-model-options.ts).

use crate::google::options::Namespace;
use aimux_core::error::AiMuxError;
use aimux_core::shared::SharedProviderOptions;
use serde_json::Value;

pub(super) struct VertexEmbeddingProviderOptions {
    pub(super) output_dimensionality: Option<Value>,
    pub(super) task_type: Option<String>,
    pub(super) title: Option<String>,
    pub(super) auto_truncate: Option<bool>,
}

/// Parse Vertex embedding provider options.
///
/// Tries `googleVertex`, then legacy `vertex`, then `google`.
pub(super) fn parse_vertex_provider_options(
    options: Option<&SharedProviderOptions>,
) -> Result<VertexEmbeddingProviderOptions, AiMuxError> {
    let provider_opts = Namespace::Vertex.read(options);
    if let Some(opts) = provider_opts {
        for (name, valid) in [
            (
                "outputDimensionality",
                opts.get("outputDimensionality")
                    .is_none_or(Value::is_number),
            ),
            ("title", opts.get("title").is_none_or(Value::is_string)),
            (
                "autoTruncate",
                opts.get("autoTruncate").is_none_or(Value::is_boolean),
            ),
            (
                "taskType",
                opts.get("taskType").is_none_or(|v| {
                    v.as_str().is_some_and(|v| {
                        matches!(
                            v,
                            "SEMANTIC_SIMILARITY"
                                | "CLASSIFICATION"
                                | "CLUSTERING"
                                | "RETRIEVAL_DOCUMENT"
                                | "RETRIEVAL_QUERY"
                                | "QUESTION_ANSWERING"
                                | "FACT_VERIFICATION"
                                | "CODE_RETRIEVAL_QUERY"
                        )
                    })
                }),
            ),
        ] {
            if !valid {
                return Err(AiMuxError::InvalidArgument(format!(
                    "Invalid Vertex embedding provider option: {name}"
                )));
            }
        }
    }

    Ok(VertexEmbeddingProviderOptions {
        output_dimensionality: provider_opts
            .and_then(|o| o.get("outputDimensionality"))
            .cloned(),
        task_type: provider_opts
            .and_then(|o| o.get("taskType"))
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string),
        title: provider_opts
            .and_then(|o| o.get("title"))
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string),
        auto_truncate: provider_opts
            .and_then(|o| o.get("autoTruncate"))
            .and_then(serde_json::Value::as_bool),
    })
}
