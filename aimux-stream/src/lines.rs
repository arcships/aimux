//! Text line-extraction utility.
//!
//! Extracts a 1-based inclusive line range from `text`, auto-detecting the
//! file's line ending (`\r\n`, `\n`, or `\r`, in that priority). This is the
//! Rust equivalent of `@ai-sdk/provider-utils` `extractLines`.

/// Extract a 1-based inclusive line range from `text`, auto-detecting the
/// file's line ending (`\r\n`, `\n`, or `\r`, in that priority).
///
/// - When neither `start_line` nor `end_line` is provided, the input is
///   returned unchanged.
/// - `end_line` past EOF clamps to the last line.
/// - Mixed line endings are not supported: detection picks one and uses it for
///   both the split and the rejoin.
#[must_use]
pub fn extract_lines(text: &str, start_line: Option<usize>, end_line: Option<usize>) -> String {
    if start_line.is_none() && end_line.is_none() {
        return text.to_string();
    }

    let line_ending = if text.contains("\r\n") {
        "\r\n"
    } else if text.contains('\n') {
        "\n"
    } else if text.contains('\r') {
        "\r"
    } else {
        "\n"
    };

    let lines: Vec<&str> = text.split(line_ending).collect();
    let start = start_line.unwrap_or(1).saturating_sub(1); // 0-based, clamp at 0
    let start = start.min(lines.len());
    let end = end_line.unwrap_or(lines.len()).min(lines.len());

    if start >= end {
        return String::new();
    }

    lines[start..end].join(line_ending)
}
