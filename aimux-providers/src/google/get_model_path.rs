//! `get-model-path.ts`.

/// Resolve a model id into the URL path segment used by the Gemini API.
///
/// Mirrors `getModelPath` in `get-model-path.ts`:
/// - Ids that already contain a `/` (e.g. `models/foo`, `tunedModels/bar`) are
///   passed through unchanged.
/// - Bare ids are prefixed with `models/`.
#[cfg(test)]
#[must_use]
pub(super) fn get_model_path(model_id: &str) -> String {
    if model_id.contains('/') {
        model_id.to_string()
    } else {
        format!("models/{model_id}")
    }
}
