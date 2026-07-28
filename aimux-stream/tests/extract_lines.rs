//! Rust translation of
//! `packages/provider-utils/src/extract-lines.test.ts`.
//!
//! The TS `extractLines({ text, startLine, endLine })` extracts a 1-based
//! inclusive line range, auto-detecting the line ending (`\r\n`, `\n`, `\r`).
//! The Rust equivalent is [`aimux_stream::extract_lines`], taking
//! `(text, start_line, end_line)` with both bounds optional.

use aimux_stream::extract_lines;

#[test]
fn returns_input_unchanged_when_neither_bound_set() {
    // TS: "returns the input unchanged when neither startLine nor endLine is set"
    assert_eq!(extract_lines("a\nb\nc", None, None), "a\nb\nc");
}

#[test]
fn slices_one_based_inclusive_range_from_lf_file() {
    // TS: "slices a 1-based inclusive range from a \n file"
    assert_eq!(extract_lines("a\nb\nc\nd", Some(2), Some(3)), "b\nc");
}

#[test]
fn preserves_crlf_line_endings() {
    // TS: "preserves \r\n line endings"
    assert_eq!(
        extract_lines("a\r\nb\r\nc\r\nd", Some(2), Some(3)),
        "b\r\nc"
    );
}

#[test]
fn preserves_cr_line_endings() {
    // TS: "preserves \r line endings"
    assert_eq!(extract_lines("a\rb\rc\rd", Some(2), Some(3)), "b\rc");
}

#[test]
fn treats_end_line_past_eof_as_last_line() {
    // TS: "treats endLine past EOF as the last line"
    assert_eq!(extract_lines("a\nb\nc", Some(2), Some(99)), "b\nc");
}

#[test]
fn defaults_start_line_to_one_when_only_end_set() {
    // TS: "defaults startLine to 1 when only endLine is set"
    assert_eq!(extract_lines("a\nb\nc", None, Some(2)), "a\nb");
}

#[test]
fn defaults_end_line_to_last_when_only_start_set() {
    // TS: "defaults endLine to the last line when only startLine is set"
    assert_eq!(extract_lines("a\nb\nc", Some(2), None), "b\nc");
}

#[test]
fn returns_input_unchanged_when_no_line_breaks() {
    // TS: "returns input unchanged when there are no line breaks"
    assert_eq!(extract_lines("one-liner", Some(1), Some(1)), "one-liner");
}

#[test]
fn start_line_zero_is_clamped_to_first_line() {
    // Extra coverage: a 0 (or below) start line clamps to line 1.
    assert_eq!(extract_lines("a\nb\nc", Some(0), Some(2)), "a\nb");
}

#[test]
fn start_past_eof_yields_empty_string() {
    // Extra coverage: a start line beyond EOF yields an empty string.
    assert_eq!(extract_lines("a\nb\nc", Some(10), Some(12)), "");
}
