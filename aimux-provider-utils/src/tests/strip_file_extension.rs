//! Rust translation of `packages/provider-utils/src/strip-file-extension.test.ts`
//! (@ai-sdk/provider-utils).

use crate::strip_file_extension;

/// upstream: @ai-sdk/provider-utils src/strip-file-extension.test.ts › stripFileExtension › should strip the extension from a filename
#[test]
fn should_strip_the_extension_from_a_filename() {
    assert_eq!(strip_file_extension("report.pdf"), "report");
}

/// upstream: @ai-sdk/provider-utils src/strip-file-extension.test.ts › stripFileExtension › should return the input when there is no extension
#[test]
fn should_return_the_input_when_there_is_no_extension() {
    assert_eq!(strip_file_extension("report"), "report");
}

/// upstream: @ai-sdk/provider-utils src/strip-file-extension.test.ts › stripFileExtension › should strip all extension segments for multi-dot filenames
#[test]
fn should_strip_all_extension_segments_for_multi_dot_filenames() {
    assert_eq!(strip_file_extension("archive.tar.gz"), "archive");
}

/// upstream: @ai-sdk/provider-utils src/strip-file-extension.test.ts › stripFileExtension › should strip a trailing dot
#[test]
fn should_strip_a_trailing_dot() {
    assert_eq!(strip_file_extension("report."), "report");
}
