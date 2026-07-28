//! Independent SSE parsing tests.
//!
//! The TS SDK has no standalone SSE parser tests — `parseJsonEventStream` is
//! only exercised indirectly through provider tests. These tests fill that gap,
//! covering the edge cases listed in the task against [`aimux_stream::SseStream`].
//!
//! The expected behavior mirrors `eventsource-parser`'s `EventSourceParserStream`
//! (which the TS `parseJsonEventStream` pipes through):
//!   - an event is dispatched only on a terminating blank line (`\n\n`/`\r\n\r\n`);
//!   - an event with no `data:` line is NOT dispatched (covers comment-only,
//!     `event:`/`id:`/`retry:`-only, and blank-line keep-alives);
//!   - exactly one leading U+0020 SPACE after the `:` is stripped from a field
//!     value (per the SSE spec);
//!   - a partial event at EOF (no terminating blank line) is dropped.

use aimux_stream::{SseError, SseEvent, SseStream};
use bytes::Bytes;
use futures::stream::{self, StreamExt};

/// Feed `chunks` (in arrival order) into a [`SseStream`] and collect all
/// emitted events. Each chunk simulates one `Result<Bytes, _>` item from a
/// real byte stream, so splitting a single event across chunks exercises the
/// cross-chunk buffering path.
async fn collect_events(chunks: Vec<&str>) -> Vec<Result<SseEvent, SseError>> {
    let items: Vec<Result<Bytes, std::io::Error>> = chunks
        .into_iter()
        .map(|s| Ok(Bytes::copy_from_slice(s.as_bytes())))
        .collect();
    let stream = SseStream::new(stream::iter(items));
    stream.collect::<Vec<_>>().await
}

fn data(event: &SseEvent) -> &str {
    &event.data
}

// ── the 12 required edge cases ────────────────────────────────────────────

#[tokio::test]
async fn single_complete_event() {
    let events = collect_events(vec!["data: hello\n\n"]).await;
    assert_eq!(events.len(), 1);
    let event = events[0].as_ref().unwrap();
    assert_eq!(data(event), "hello");
    assert!(event.event.is_none());
    assert!(event.id.is_none());
    assert!(event.retry.is_none());
}

#[tokio::test]
async fn multiple_consecutive_events() {
    let events = collect_events(vec!["data: first\n\ndata: second\n\n"]).await;
    assert_eq!(events.len(), 2);
    assert_eq!(data(events[0].as_ref().unwrap()), "first");
    assert_eq!(data(events[1].as_ref().unwrap()), "second");
}

#[tokio::test]
async fn event_spanning_two_chunks_half_line_split() {
    // A single event whose data line arrives in two pieces.
    let events = collect_events(vec!["data: hel", "lo\n\n"]).await;
    assert_eq!(events.len(), 1);
    assert_eq!(data(events[0].as_ref().unwrap()), "hello");
}

#[tokio::test]
async fn event_spanning_many_tiny_chunks() {
    // The same event byte-split across many tiny chunks.
    let events = collect_events(vec!["da", "ta: ", "wor", "ld", "\n", "\n"]).await;
    assert_eq!(events.len(), 1);
    assert_eq!(data(events[0].as_ref().unwrap()), "world");
}

#[tokio::test]
async fn multi_line_data_field() {
    let events = collect_events(vec!["data: line1\ndata: line2\n\n"]).await;
    assert_eq!(events.len(), 1);
    assert_eq!(data(events[0].as_ref().unwrap()), "line1\nline2");
}

#[tokio::test]
async fn event_field() {
    let events = collect_events(vec!["event: message\ndata: payload\n\n"]).await;
    assert_eq!(events.len(), 1);
    let event = events[0].as_ref().unwrap();
    assert_eq!(event.event.as_deref(), Some("message"));
    assert_eq!(data(event), "payload");
}

#[tokio::test]
async fn id_field() {
    let events = collect_events(vec!["id: 42\ndata: payload\n\n"]).await;
    assert_eq!(events.len(), 1);
    let event = events[0].as_ref().unwrap();
    assert_eq!(event.id.as_deref(), Some("42"));
    assert_eq!(data(event), "payload");
}

#[tokio::test]
async fn retry_field() {
    let events = collect_events(vec!["retry: 5000\ndata: payload\n\n"]).await;
    assert_eq!(events.len(), 1);
    let event = events[0].as_ref().unwrap();
    assert_eq!(event.retry, Some(5000));
    assert_eq!(data(event), "payload");
}

#[tokio::test]
async fn comment_lines_starting_with_colon_are_ignored() {
    // A comment line within an event block does not affect the event.
    let events = collect_events(vec![": this is a comment\ndata: hello\n\n"]).await;
    assert_eq!(events.len(), 1);
    assert_eq!(data(events[0].as_ref().unwrap()), "hello");
}

#[tokio::test]
async fn comment_only_event_is_not_emitted() {
    // An event consisting solely of a comment has no data line → not dispatched.
    let events = collect_events(vec![": just a comment\n\n"]).await;
    assert!(events.is_empty());
}

#[tokio::test]
async fn done_sentinel_is_emitted_as_data() {
    // The parser emits `data: [DONE]` like any other event; higher layers
    // (e.g. the OpenAI provider) filter the sentinel. This mirrors the TS
    // `parseJsonEventStream`, which drops `[DONE]` after parsing — the SSE
    // parser itself does not special-case it.
    let events = collect_events(vec!["data: [DONE]\n\n"]).await;
    assert_eq!(events.len(), 1);
    assert_eq!(data(events[0].as_ref().unwrap()), "[DONE]");
}

#[tokio::test]
async fn blank_lines_and_heartbeat_emit_nothing() {
    // Bare blank lines are keep-alives / heartbeats with no data → not emitted.
    let events = collect_events(vec!["\n\n\n\n"]).await;
    assert!(events.is_empty());
}

#[tokio::test]
async fn crlf_vs_lf_line_endings() {
    // CRLF-terminated single event.
    let events = collect_events(vec!["data: hello\r\n\r\n"]).await;
    assert_eq!(events.len(), 1);
    assert_eq!(data(events[0].as_ref().unwrap()), "hello");
}

#[tokio::test]
async fn crlf_multi_line_data() {
    // CRLF-terminated multi-line data joins with a single `\n`.
    let events = collect_events(vec!["data: line1\r\ndata: line2\r\n\r\n"]).await;
    assert_eq!(events.len(), 1);
    assert_eq!(data(events[0].as_ref().unwrap()), "line1\nline2");
}

#[tokio::test]
async fn incomplete_event_at_eof_without_blank_line_is_dropped() {
    // No terminating blank line → the buffered partial is not a complete event
    // and is dropped (matches eventsource-parser's EventSourceParserStream,
    // which has no flush handler).
    let events = collect_events(vec!["data: hello"]).await;
    assert!(events.is_empty());
}

#[tokio::test]
async fn incomplete_multi_line_event_at_eof_is_dropped() {
    // A buffered data line that never receives its terminating blank line is
    // dropped, even if the data line itself looks complete.
    let events = collect_events(vec!["data: hello\n"]).await;
    assert!(events.is_empty());
}

// ── additional spec-faithfulness coverage ────────────────────────────────

#[tokio::test]
async fn data_without_space_after_colon() {
    // `data:hello` (no space) → value is `hello`.
    let events = collect_events(vec!["data:hello\n\n"]).await;
    assert_eq!(events.len(), 1);
    assert_eq!(data(events[0].as_ref().unwrap()), "hello");
}

#[tokio::test]
async fn only_one_leading_space_is_removed() {
    // Per the SSE spec exactly ONE leading U+0020 SPACE is removed; the rest
    // of the value is preserved verbatim.
    let events = collect_events(vec!["data:  two leading spaces\n\n"]).await;
    assert_eq!(events.len(), 1);
    assert_eq!(data(events[0].as_ref().unwrap()), " two leading spaces");
}

#[tokio::test]
async fn empty_data_value() {
    // `data:` with nothing after it contributes an empty data line.
    let events = collect_events(vec!["data:\n\n"]).await;
    assert_eq!(events.len(), 1);
    assert_eq!(data(events[0].as_ref().unwrap()), "");
}

#[tokio::test]
async fn event_field_without_data_is_not_emitted() {
    // `event:` without a `data:` line → not dispatched.
    let events = collect_events(vec!["event: ping\n\n"]).await;
    assert!(events.is_empty());
}

#[tokio::test]
async fn id_field_without_data_is_not_emitted() {
    let events = collect_events(vec!["id: 99\n\n"]).await;
    assert!(events.is_empty());
}

#[tokio::test]
async fn retry_field_without_data_is_not_emitted() {
    let events = collect_events(vec!["retry: 1000\n\n"]).await;
    assert!(events.is_empty());
}

#[tokio::test]
async fn event_with_all_fields_together() {
    let events = collect_events(vec!["event: update\nid: 7\nretry: 3000\ndata: payload\n\n"]).await;
    assert_eq!(events.len(), 1);
    let e = events[0].as_ref().unwrap();
    assert_eq!(e.event.as_deref(), Some("update"));
    assert_eq!(e.id.as_deref(), Some("7"));
    assert_eq!(e.retry, Some(3000));
    assert_eq!(data(e), "payload");
}

#[tokio::test]
async fn intermixed_comments_between_events() {
    let events = collect_events(vec![
        ": heartbeat\ndata: first\n\n",
        ": another comment\ndata: second\n\n",
    ])
    .await;
    assert_eq!(events.len(), 2);
    assert_eq!(data(events[0].as_ref().unwrap()), "first");
    assert_eq!(data(events[1].as_ref().unwrap()), "second");
}

#[tokio::test]
async fn invalid_retry_value_is_ignored() {
    // A non-numeric `retry:` value is ignored; the event is still dispatched
    // because it has a data line.
    let events = collect_events(vec!["retry: not-a-number\ndata: payload\n\n"]).await;
    assert_eq!(events.len(), 1);
    let e = events[0].as_ref().unwrap();
    assert_eq!(e.retry, None);
    assert_eq!(data(e), "payload");
}

#[tokio::test]
async fn unknown_field_is_ignored() {
    let events = collect_events(vec!["foo: bar\ndata: payload\n\n"]).await;
    assert_eq!(events.len(), 1);
    assert_eq!(data(events[0].as_ref().unwrap()), "payload");
}

#[tokio::test]
async fn mixed_lf_and_crlf_events() {
    // An LF event followed by a CRLF event in the same stream.
    let events = collect_events(vec!["data: lf\n\ndata: crlf\r\n\r\n"]).await;
    assert_eq!(events.len(), 2);
    assert_eq!(data(events[0].as_ref().unwrap()), "lf");
    assert_eq!(data(events[1].as_ref().unwrap()), "crlf");
}
