//! `stripFileExtension` from `@ai-sdk/provider-utils`
//! (`strip-file-extension.ts`).

/// Strips every extension segment from a filename: everything from the first
/// dot on.
///
/// - `"report.pdf"` -> `"report"`
/// - `"archive.tar.gz"` -> `"archive"`
/// - `"filename"` -> `"filename"`
#[must_use]
pub fn strip_file_extension(filename: &str) -> &str {
    filename.split_once('.').map_or(filename, |(name, _)| name)
}
