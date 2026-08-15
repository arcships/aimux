//! Math helpers ported from the Vercel AI SDK TypeScript `util` package.

use thiserror::Error;

/// Errors raised by the util helpers.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum UtilError {
    /// Vectors passed to [`cosine_similarity`] did not have the same length.
    #[error("Vectors must have the same length")]
    VectorLengthMismatch,
}

/// Cosine similarity between two vectors.
///
/// Port of `cosineSimilarity` in `packages/ai/src/util/cosine-similarity.ts`.
///
/// Returns `Ok(0.0)` for empty vectors or when either vector is the zero
/// vector, and `Err(UtilError::VectorLengthMismatch)` when the lengths differ
/// (the TS version throws an `InvalidArgumentError`).
///
/// # Errors
///
/// Returns `UtilError::VectorLengthMismatch` when the two vectors have
/// different lengths.
pub fn cosine_similarity(vector1: &[f64], vector2: &[f64]) -> Result<f64, UtilError> {
    if vector1.len() != vector2.len() {
        return Err(UtilError::VectorLengthMismatch);
    }

    let n = vector1.len();
    if n == 0 {
        return Ok(0.0);
    }

    let mut magnitude_squared_1 = 0.0;
    let mut magnitude_squared_2 = 0.0;
    let mut dot_product = 0.0;

    for i in 0..n {
        let value1 = vector1[i];
        let value2 = vector2[i];
        magnitude_squared_1 += value1 * value1;
        magnitude_squared_2 += value2 * value2;
        dot_product += value1 * value2;
    }

    if magnitude_squared_1 == 0.0 || magnitude_squared_2 == 0.0 {
        Ok(0.0)
    } else {
        Ok(dot_product / (magnitude_squared_1.sqrt() * magnitude_squared_2.sqrt()))
    }
}
