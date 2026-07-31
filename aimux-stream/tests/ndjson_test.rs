//! NDJSON stream decoder tests — cross-chunk UTF-8 reassembly, invalid-UTF-8
//! rejection, and bounded-buffer behavior.

use aimux_stream::{NdjsonError, NdjsonStream};
use bytes::Bytes;
use futures::stream::{self, StreamExt};
use serde::Deserialize;

#[derive(Debug, Deserialize, PartialEq)]
struct Line {
    text: String,
}

/// Feed raw byte chunks into an [`NdjsonStream`] and collect the results.
async fn collect_lines(chunks: Vec<Vec<u8>>) -> Vec<Result<Line, NdjsonError>> {
    let items: Vec<Result<Bytes, std::io::Error>> =
        chunks.into_iter().map(|c| Ok(Bytes::from(c))).collect();
    let stream = NdjsonStream::new(stream::iter(items));
    stream.collect::<Vec<_>>().await
}

// ── cross-chunk UTF-8 reassembly (P0-03) ─────────────────────────────────

#[tokio::test]
async fn emoji_ndjson_line_split_at_every_byte_boundary() {
    // An NDJSON line whose value contains multi-byte emoji. Splitting the full
    // line at *every* byte position must still reassemble to the intact text.
    // The old per-chunk `String::from_utf8_lossy` decoder would corrupt a code
    // point split across two chunks into two U+FFFDs.
    let payload = "{\"text\":\"😀👋\"}\n";
    let bytes = payload.as_bytes();
    for split in 1..bytes.len() {
        let (a, b) = bytes.split_at(split);
        let lines = collect_lines(vec![a.to_vec(), b.to_vec()]).await;
        assert_eq!(
            lines.len(),
            1,
            "split at byte {} produced {} results",
            split,
            lines.len()
        );
        let line = lines[0]
            .as_ref()
            .unwrap_or_else(|e| panic!("split at byte {} errored: {:?}", split, e));
        assert_eq!(
            line.text, "😀👋",
            "split at byte {} corrupted the text",
            split
        );
    }
}

#[tokio::test]
async fn invalid_utf8_line_returns_utf8_error() {
    // `{"text":"` followed by 0xF0 0x9F — the first two bytes of 😀
    // (U+1F600 = F0 9F 98 80) without the trailing bytes — is an incomplete
    // 4-byte sequence. Strict decoding of the complete line surfaces a `Utf8`
    // error instead of silently producing replacement chars.
    let chunk = vec![
        b'{', b'"', b't', b'e', b'x', b't', b'"', b':', b'"', 0xF0, 0x9F, b'\n',
    ];
    let lines = collect_lines(vec![chunk]).await;
    assert_eq!(lines.len(), 1, "expected exactly one result");
    match &lines[0] {
        Err(NdjsonError::Utf8(_)) => {}
        other => panic!("expected NdjsonError::Utf8, got {:?}", other),
    }
}

// ── bounded buffers (P1-10) ─────────────────────────────────────────────

#[tokio::test]
async fn oversized_line_returns_line_too_large() {
    // A complete line longer than the 5-byte limit is rejected.
    let stream = NdjsonStream::with_max_line_size(
        stream::iter(vec![Ok::<_, std::io::Error>(Bytes::from_static(
            b"{\"text\":\"hi\"}\n",
        ))]),
        5,
    );
    let results: Vec<Result<Line, _>> = stream.collect().await;
    assert_eq!(results.len(), 1);
    assert!(matches!(results[0], Err(NdjsonError::LineTooLarge)));
}

#[tokio::test]
async fn buffer_growing_past_limit_without_newline_returns_line_too_large() {
    // No newline ever arrives, so the buffer would grow unboundedly; the limit
    // trips instead (previously this allocated forever).
    let stream = NdjsonStream::with_max_line_size(
        stream::iter(vec![Ok::<_, std::io::Error>(Bytes::from_static(
            b"{\"text\":\"no newline here\"",
        ))]),
        5,
    );
    let results: Vec<Result<Line, _>> = stream.collect().await;
    assert_eq!(results.len(), 1);
    assert!(matches!(results[0], Err(NdjsonError::LineTooLarge)));
}
