//! Provider options of the Vertex embedding model
//! (google-vertex-embedding-model-options.ts).

use crate::google::internal::Namespace;
use aimux_core::error::AiMuxError;
use aimux_core::shared::SharedProviderOptions;

/// `googleVertexEmbeddingModelOptions`. Every key is `.optional()`: absent
/// is `None`, `null` fails.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct VertexEmbeddingProviderOptions {
    /// `z.number()`, sent as given.
    #[serde(default, deserialize_with = "aimux_provider_utils::optional_field")]
    pub(super) output_dimensionality: Option<serde_json::Number>,
    #[serde(default, deserialize_with = "aimux_provider_utils::optional_field")]
    pub(super) task_type: Option<TaskType>,
    #[serde(default, deserialize_with = "aimux_provider_utils::optional_field")]
    pub(super) title: Option<String>,
    #[serde(default, deserialize_with = "aimux_provider_utils::optional_field")]
    pub(super) auto_truncate: Option<bool>,
}

/// The `taskType` enum, sent under the same name.
#[derive(Debug, Clone, Copy, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(super) enum TaskType {
    SemanticSimilarity,
    Classification,
    Clustering,
    RetrievalDocument,
    RetrievalQuery,
    QuestionAnswering,
    FactVerification,
    CodeRetrievalQuery,
}

/// `parseProviderOptions` with `googleVertexEmbeddingModelOptions` under
/// `googleVertex`, then `vertex`, then `google`
/// (google-vertex-embedding-model.ts:70-90); all `None` when none is present.
///
/// # Errors
///
/// `InvalidArgument` "invalid <namespace> provider options" when the first
/// namespace present does not match the schema.
pub(super) fn parse_vertex_provider_options(
    options: Option<&SharedProviderOptions>,
) -> Result<VertexEmbeddingProviderOptions, AiMuxError> {
    Ok(Namespace::Vertex.parse(options)?.unwrap_or_default())
}
