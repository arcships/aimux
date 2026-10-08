//! Port of the `eventsource-parser` test suite against [`aimux_stream::SseStream`].
//!
//! `eventsource-parser` is the parser behind the AI SDK's `parseJsonEventStream`.
//! This file ports `test/parse.test.ts` and `test/stream.test.ts` from v3.1.1
//! (MIT), driven by the same fixtures (`test/fixtures.ts`; `test/multibyte.ts`
//! is checked in as `tests/fixtures/eventsource_parser_multibyte.json`). Test
//! names follow the upstream titles.
//!
//! Adaptations forced by the Rust API (`SseStream` yields whole events, has no
//! callbacks, and works on bytes):
//! - `retry` is reported on the dispatched event instead of through `onRetry`,
//!   so a `retry:` block with no `data:` is dropped, and upstream's
//!   `reconnect-interval` events become `SseEvent::retry`.
//! - Upstream's `onError` for an invalid `retry` value has no counterpart: the
//!   value is ignored, which is what is asserted.
//! - There is no `maxBufferSize`: like `parseJsonEventStream`, the stream is
//!   unbounded, so the upstream overflow cases are not ported.
//! - A leading U+FEFF is the UTF-8 BOM at byte level. Upstream's
//!   "invalid byte-order mark" case feeds a decoded U+FEFF that the JS
//!   `TextDecoderStream` would already have stripped, so only the "multiple
//!   places" case is ported.
//!
//! Not ported (no Rust counterpart): `onComment` call counts, `reset()`
//! (3 tests), `onError` `ParseError` payloads (3 tests), the "function passed
//! to `createParser`" guard, and the `maxBufferSize` / `onError: 'terminate'`
//! stream tests (no size limit here).
//!
//! The `aimux` module holds the few tests for behaviour upstream does not have.

use aimux_stream::{SseError, SseEvent, SseStream};
use bytes::Bytes;
use futures::stream::{self, StreamExt};
use serde_json::Value;
use sha2::{Digest, Sha256};

// ── harness ──────────────────────────────────────────────────────────────

type Item = Result<SseEvent, SseError>;
type Triple<'a> = (Option<&'a str>, Option<&'a str>, &'a str);

async fn run_bytes(chunks: Vec<Vec<u8>>) -> Vec<Item> {
    let items: Vec<Result<Bytes, std::io::Error>> =
        chunks.into_iter().map(|c| Ok(Bytes::from(c))).collect();
    SseStream::new(stream::iter(items))
        .collect::<Vec<_>>()
        .await
}

async fn run(chunks: Vec<String>) -> Vec<Item> {
    run_bytes(chunks.into_iter().map(String::into_bytes).collect()).await
}

/// Feed `chunks` and return the events; any error item fails the test.
async fn events(chunks: Vec<String>) -> Vec<SseEvent> {
    run(chunks)
        .await
        .into_iter()
        .map(|item| item.unwrap_or_else(|e| panic!("unexpected error item: {e:?}")))
        .collect()
}

fn s(chunks: &[&str]) -> Vec<String> {
    chunks.iter().map(|c| (*c).to_string()).collect()
}

/// `(id, event, data)` of each event.
fn triples(events: &[SseEvent]) -> Vec<Triple<'_>> {
    events
        .iter()
        .map(|e| (e.id.as_deref(), e.event.as_deref(), e.data.as_str()))
        .collect()
}

// ── `eventsource-encoder` (used by upstream's fixtures) ──────────────────

#[derive(Default)]
struct Msg<'a> {
    event: Option<&'a str>,
    retry: Option<u64>,
    id: Option<&'a str>,
    data: Option<&'a str>,
}

fn encode_data(text: &str) -> String {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = normalized.split('\n').collect();
    let mut out = String::new();
    for (i, line) in lines.iter().enumerate() {
        out.push_str("data: ");
        out.push_str(line);
        out.push_str(if i + 1 == lines.len() { "\n\n" } else { "\n" });
    }
    out
}

fn encode_comment(comment: &str) -> String {
    let normalized = comment.replace("\r\n", "\n").replace('\r', "\n");
    format!(": {}\n\n", normalized.replace('\n', "\n: "))
}

fn encode(msg: &Msg<'_>) -> String {
    let mut out = String::new();
    if let Some(event) = msg.event.filter(|e| !e.is_empty()) {
        out.push_str(&format!("event: {event}\n"));
    }
    if let Some(retry) = msg.retry {
        out.push_str(&format!("retry: {retry}\n"));
    }
    if let Some(id) = msg.id {
        out.push_str(&format!("id: {id}\n"));
    }
    if let Some(data) = msg.data {
        out.push_str(&encode_data(data));
    } else if !out.is_empty() {
        out.push_str("\n\n");
    }
    out
}

fn done() -> String {
    encode(&Msg {
        event: Some("done"),
        data: Some("✔"),
        ..Msg::default()
    })
}

fn data_only(data: &str) -> String {
    encode(&Msg {
        data: Some(data),
        ..Msg::default()
    })
}

// ── fixtures (`test/fixtures.ts`) ────────────────────────────────────────

struct Multibyte {
    lines: Vec<String>,
    emojis: Vec<String>,
    expected: Vec<(Option<String>, Option<String>, String)>,
}

fn multibyte() -> Multibyte {
    let root: Value =
        serde_json::from_str(include_str!("fixtures/eventsource_parser_multibyte.json")).unwrap();
    let strings = |key: &str| -> Vec<String> {
        root[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect()
    };
    let optional = |v: &Value| v.as_str().map(str::to_string);
    Multibyte {
        lines: strings("lines"),
        emojis: strings("emojis"),
        expected: root["expected"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| {
                (
                    optional(&e["id"]),
                    optional(&e["event"]),
                    e["data"].as_str().unwrap().to_string(),
                )
            })
            .collect(),
    }
}

/// Split `text` after `units` UTF-16 code units (JS `slice(0, n)` / `slice(n)`).
fn split_utf16(text: &str, units: usize) -> (&str, &str) {
    let mut seen = 0;
    for (index, ch) in text.char_indices() {
        if seen == units {
            return text.split_at(index);
        }
        seen += ch.len_utf16();
        assert!(seen <= units, "UTF-16 cut inside a surrogate pair");
    }
    (text, "")
}

fn multibyte_chunks(mb: &Multibyte) -> Vec<String> {
    let per_message = mb.emojis.len().div_ceil(mb.lines.len());
    let mut chunks = Vec::new();
    for (i, line) in mb.lines.iter().enumerate() {
        let start = (per_message * i).min(mb.emojis.len());
        let end = (per_message * i + per_message).min(mb.emojis.len());
        let line = format!("{line} {}", mb.emojis[start..end].join(" "));
        chunks.push(format!("id: {i}\n"));
        if i % 2 == 0 {
            // Even lines are split into two `data:` lines.
            let (head, tail) = split_utf16(&line, 5);
            chunks.push(format!("data:{head}\n"));
            chunks.push(format!("data:{tail}\n\n"));
        } else {
            chunks.push(format!("data:{line}\n\n"));
        }
    }
    chunks.push(done());
    chunks
}

fn iso_timestamp(i: usize) -> String {
    format!("2026-09-29T05:51:{:02}.{:03}Z", i % 60, i)
}

/// `^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$`
fn is_iso_timestamp(value: &str) -> bool {
    const SHAPE: &[u8] = b"dddd-dd-ddTdd:dd:dd.dddZ";
    value.len() == SHAPE.len()
        && value.bytes().zip(SHAPE).all(|(b, shape)| match shape {
            b'd' => b.is_ascii_digit(),
            other => b == *other,
        })
}

// ── parse.test.ts ────────────────────────────────────────────────────────

#[tokio::test]
async fn basic_unnamed_events_stream() {
    let mut chunks: Vec<String> = (0..5).map(|i| data_only(&i.to_string())).collect();
    chunks.push(done());
    let got = events(chunks).await;
    assert_eq!(
        triples(&got),
        vec![
            (None, None, "0"),
            (None, None, "1"),
            (None, None, "2"),
            (None, None, "3"),
            (None, None, "4"),
            (None, Some("done"), "✔"),
        ]
    );
}

#[tokio::test]
async fn stream_of_time_event_name() {
    let mut chunks: Vec<String> = (0..5)
        .map(|i| {
            encode(&Msg {
                event: Some("time"),
                data: Some(&iso_timestamp(i)),
                ..Msg::default()
            })
        })
        .collect();
    chunks.push(done());
    let got = events(chunks).await;
    assert_eq!(got.len(), 6);
    for event in &got[..5] {
        assert_eq!(event.event.as_deref(), Some("time"));
        assert!(is_iso_timestamp(&event.data), "{}", event.data);
    }
}

#[tokio::test]
async fn stream_of_time_event_names_unbalanced_chunks() {
    let mut chunks = Vec::new();
    let mut ids = Vec::new();
    for i in 0..30 {
        let id = (100_000_000_000_u64 + i as u64 * 7919).to_string();
        let message = encode(&Msg {
            id: Some(&id),
            event: Some("time"),
            data: Some(&iso_timestamp(i)),
            ..Msg::default()
        });
        // Upstream splits at a random offset; the offset is varied
        // deterministically here (the message is ASCII).
        let split = (i * 11 + 3) % message.len();
        chunks.push(message[..split].to_string());
        chunks.push(message[split..].to_string());
        ids.push(id);
    }
    let got = events(chunks).await;
    assert_eq!(got.len(), 30);
    for (event, id) in got.iter().zip(&ids) {
        assert_eq!(event.event.as_deref(), Some("time"));
        assert_eq!(event.id.as_deref(), Some(id.as_str()));
        assert!(is_iso_timestamp(&event.data), "{}", event.data);
    }
}

#[tokio::test]
async fn stream_of_identified_messages_and_retry_interval() {
    let chunks = (1337..1339)
        .map(|id| {
            let id = id.to_string();
            encode(&Msg {
                event: Some("tick"),
                data: Some(&id),
                id: Some(&id),
                retry: Some(50),
            })
        })
        .collect();
    let got = events(chunks).await;
    assert_eq!(
        triples(&got),
        vec![
            (Some("1337"), Some("tick"), "1337"),
            (Some("1338"), Some("tick"), "1338"),
        ]
    );
    assert!(got.iter().all(|e| e.retry == Some(50)));
}

#[tokio::test]
async fn stream_of_heartbeat_comments_unnamed_events() {
    let mut chunks = Vec::new();
    for letter in ['A', 'B', 'C', 'D', 'E'] {
        chunks.push(encode_comment(" ♥"));
        chunks.push(data_only(&letter.to_string()));
    }
    chunks.push(done());
    let got = events(chunks).await;
    assert_eq!(
        triples(&got),
        vec![
            (None, None, "A"),
            (None, None, "B"),
            (None, None, "C"),
            (None, None, "D"),
            (None, None, "E"),
            (None, Some("done"), "✔"),
        ]
    );
}

#[tokio::test]
async fn stream_of_multi_line_data_events() {
    let mut chunks = s(&[
        "event: stock\n",
        "data: YHOO\n",
        "data: +2\n",
        "data: 10\n\n",
        "event: stock\n",
        "data: GOOG\n",
        "data: -8\n",
        "data: 1881\n\n",
    ]);
    chunks.push(done());
    let got = events(chunks).await;
    assert_eq!(
        triples(&got)[..2],
        [
            (None, Some("stock"), "YHOO\n+2\n10"),
            (None, Some("stock"), "GOOG\n-8\n1881"),
        ]
    );
}

#[tokio::test]
async fn stream_of_multi_byte_events() {
    let mb = multibyte();
    let got = events(multibyte_chunks(&mb)).await;
    let got: Vec<_> = got.into_iter().map(|e| (e.id, e.event, e.data)).collect();
    assert_eq!(got, mb.expected);
}

#[tokio::test]
async fn stream_of_multi_byte_events_with_some_empty_lines_thrown_in() {
    let got = events(vec![
        "\n\n\n\nid: 1\ndata: 我現在都看實況不玩遊戲\n\n".to_string(),
        done(),
    ])
    .await;
    assert_eq!(
        triples(&got),
        vec![
            (Some("1"), None, "我現在都看實況不玩遊戲"),
            (None, Some("done"), "✔"),
        ]
    );
}

#[tokio::test]
async fn stream_of_leading_bom() {
    let got = events(vec![
        "\u{FEFF}data: bomful 1\n\n".to_string(),
        "data: bomless 2\n\n".to_string(),
        done(),
    ])
    .await;
    assert_eq!(
        triples(&got),
        vec![
            (None, None, "bomful 1"),
            (None, None, "bomless 2"),
            (None, Some("done"), "✔"),
        ]
    );
}

#[tokio::test]
async fn stream_containing_byte_order_mark_multiple_places() {
    let got = events(vec![
        "\u{FEFF}data: bomful 1\n\n".to_string(),
        "\u{FEFF}data: bomful 2\n\n".to_string(),
        "data: bomless 3\n\n".to_string(),
        done(),
    ])
    .await;
    // Only the BOM at the start of the stream is stripped; the second one
    // turns the field name into an unknown field, so that event has no data.
    assert_eq!(
        triples(&got),
        vec![
            (None, None, "bomful 1"),
            (None, None, "bomless 3"),
            (None, Some("done"), "✔"),
        ]
    );
}

#[tokio::test]
async fn stream_using_carriage_returns() {
    let mut chunks = s(&[
        "data: dog\r",
        "data: bark\r\r",
        "data: cat\r",
        "data: meow\r\r",
    ]);
    chunks.push(done());
    let got = events(chunks).await;
    assert_eq!(
        triples(&got),
        vec![
            (None, None, "dog\nbark"),
            (None, None, "cat\nmeow"),
            (None, Some("done"), "✔"),
        ]
    );
}

#[tokio::test]
async fn stream_using_line_feeds() {
    let mut chunks = s(&[
        "data: cow\n",
        "data: moo\n\n",
        "data: horse\n",
        "data: neigh\n\n",
    ]);
    chunks.push(done());
    let got = events(chunks).await;
    assert_eq!(
        triples(&got),
        vec![
            (None, None, "cow\nmoo"),
            (None, None, "horse\nneigh"),
            (None, Some("done"), "✔"),
        ]
    );
}

#[tokio::test]
async fn stream_using_carriage_returns_and_line_feeds() {
    let mut chunks = s(&[
        "data: sheep\r\n",
        "data: bleat\r\n\r\n",
        "data: pig\r\n",
        "data: oink\r\n\r\n",
    ]);
    chunks.push(done());
    let got = events(chunks).await;
    assert_eq!(
        triples(&got),
        vec![
            (None, None, "sheep\nbleat"),
            (None, None, "pig\noink"),
            (None, Some("done"), "✔"),
        ]
    );
}

#[tokio::test]
async fn stream_with_varying_odd_uses_of_comments() {
    let mb = multibyte();
    let mut chunks = s(&[": Hello\n\n"]);
    chunks.push(":".repeat(300));
    chunks.extend(s(&[
        "\n",
        "data: First\n\n",
        ": Первый",
        ": 第二",
        "\n",
        "data: Second\n\n",
    ]));
    chunks.extend(std::iter::repeat_n(": Moop \n".to_string(), 10));
    chunks.extend(s(&[
        ": ثالث",
        "\n",
        "data: Third\n\n",
        ":നാലാമത്തെ",
        "\n",
        "data: Fourth\n\n",
    ]));
    chunks.push(format!(": {} :", mb.emojis[..100].join(" ")));
    chunks.extend(s(&["\n", "data: Fifth\n\n"]));
    chunks.push(done());
    let got = events(chunks).await;
    assert_eq!(
        triples(&got),
        vec![
            (None, None, "First"),
            (None, None, "Second"),
            (None, None, "Third"),
            (None, None, "Fourth"),
            (None, None, "Fifth"),
            (None, Some("done"), "✔"),
        ]
    );
}

#[tokio::test]
async fn stream_with_even_more_odd_uses_of_comments() {
    let long = "x".repeat(2 * 1024 + 1);
    let chunks = vec![
        "data:1\r\r:\0\n:\r\ndata:2\n\n:".to_string(),
        long.clone(),
        "\rdata:3\n\n:data:fail\r:".to_string(),
        long,
        "\ndata:4\n\n".to_string(),
        "data:5".to_string(),
    ];
    let got = events(chunks).await;
    // No newline after the last message, thus not emitted.
    assert_eq!(
        triples(&got),
        vec![
            (None, None, "1"),
            (None, None, "2"),
            (None, None, "3"),
            (None, None, "4"),
        ]
    );
}

#[tokio::test]
async fn stream_with_empty_event_field() {
    let got = events(vec![
        "event:\ndata: Hello 1\n\n".to_string(),
        "event:\n\n".to_string(),
        done(),
    ])
    .await;
    assert_eq!(
        triples(&got),
        vec![(None, None, "Hello 1"), (None, Some("done"), "✔")]
    );
}

#[tokio::test]
async fn stream_with_empty_retry_field() {
    let got = events(vec![
        encode(&Msg {
            id: Some("1"),
            retry: Some(500),
            data: Some("🥌"),
            ..Msg::default()
        }),
        "id:2\nretry:\ndata:🧹\n\n".to_string(),
        encode(&Msg {
            id: Some("3"),
            data: Some("✅"),
            ..Msg::default()
        }),
    ])
    .await;
    assert_eq!(
        triples(&got),
        vec![
            (Some("1"), None, "🥌"),
            (Some("2"), None, "🧹"),
            (Some("3"), None, "✅"),
        ]
    );
    // The empty `retry` is ignored; `retry: 500` belongs to the first event.
    assert_eq!(
        got.iter().map(|e| e.retry).collect::<Vec<_>>(),
        vec![Some(500), None, None]
    );
}

#[tokio::test]
async fn stream_with_oddly_shaped_data_field() {
    let got = events(vec![
        "data:\n\ndata\ndata\n\ndata:test\n\n".to_string(),
        done(),
    ])
    .await;
    // `data:\n\n` dispatches an event with empty data; `data\ndata\n\n` is two
    // empty data lines, joined by a newline.
    assert_eq!(
        triples(&got),
        vec![
            (None, None, ""),
            (None, None, "\n"),
            (None, None, "test"),
            (None, Some("done"), "✔"),
        ]
    );
}

#[tokio::test]
async fn stream_with_cr_separating_chunks_of_same_event() {
    // A CR at the end of a chunk may be half of a CRLF, so it must not end
    // the line yet: otherwise `A\nB` and `C` would be two events.
    // https://github.com/rexxars/eventsource-parser/issues/17
    let got = events(s(&["data: A\r\n", "data: B\r", "\n", "data: C\r\n", "\n"])).await;
    assert_eq!(triples(&got), vec![(None, None, "A\nB\nC")]);
}

#[tokio::test]
async fn stream_with_partially_incorrect_retry_fields() {
    let got = events(s(&["retry:1000\nretry:2000x\ndata:x\n\n"])).await;
    // `2000x` is not all ASCII digits and is ignored; `1000` is kept.
    assert_eq!(triples(&got), vec![(None, None, "x")]);
    assert_eq!(got[0].retry, Some(1000));
}

#[tokio::test]
async fn stream_with_id_field_containing_a_null_character() {
    // An `id` containing U+0000 is ignored, so the earlier `123` survives.
    let got = events(s(&["id: 123\nid: bad\0id\ndata: hello\n\n"])).await;
    assert_eq!(triples(&got), vec![(Some("123"), None, "hello")]);
}

#[tokio::test]
async fn stream_with_incorrect_retry_fields() {
    let got = events(s(&[
        "\nretry: 500\n\ndata: first\n\nretry: 50x\n\ndata: second\n\n",
    ]))
    .await;
    assert_eq!(
        triples(&got),
        vec![(None, None, "first"), (None, None, "second")]
    );
}

#[tokio::test]
async fn stream_with_unknown_fields_in_the_stream() {
    let got = events(vec![
        "data:abc\n data\ndata\nfoobar:xxx\njustsometext\n:thisisacommentyay\ndata:123\n\n"
            .to_string(),
        done(),
    ])
    .await;
    assert_eq!(
        triples(&got),
        vec![(None, None, "abc\n\n123"), (None, Some("done"), "✔")]
    );
}

#[tokio::test]
async fn stream_with_huge_data_chunks() {
    const TEN_MEGABYTES: usize = 1024 * 1024 * 10;
    const EXPECTED_SHA256: &str =
        "e094a44a2436226ea9feb04e413a28de012b406012ec0eb6b37ad0a19d403660";

    let mb = multibyte();
    let data_chunk = encode_data(&format!(
        "{}\n{}",
        mb.lines.join("\n\n"),
        mb.emojis.join(" ")
    ))
    .trim()
    .to_string();
    let mut chunks = s(&[": hello\n\n"]);
    let mut written = 0;
    while written < TEN_MEGABYTES {
        chunks.push(data_chunk.clone());
        written += data_chunk.len();
    }
    chunks.extend(s(&["\n\n", ": END-OF-STREAM\n\n"]));
    chunks.push(encode(&Msg {
        event: Some("done"),
        data: Some(EXPECTED_SHA256),
        ..Msg::default()
    }));

    let got: Vec<SseEvent> = run(chunks).await.into_iter().map(Result::unwrap).collect();
    assert_eq!(got.len(), 2);
    // JS `String.length` counts UTF-16 code units.
    assert_eq!(got[0].data.encode_utf16().count(), 4_808_512);
    assert_eq!(
        format!("{:x}", Sha256::digest(got[0].data.as_bytes())),
        got[1].data
    );
}

#[tokio::test]
async fn large_input_without_a_terminator_is_not_an_error() {
    // Unbounded, as upstream: a partial block is simply never dispatched.
    assert!(run(vec!["x".repeat(4 * 1024 * 1024)]).await.is_empty());
}

// ── stream.test.ts ───────────────────────────────────────────────────────

#[tokio::test]
async fn can_use_event_source_parser_stream() {
    let chunks = (0..10)
        .map(|i| {
            encode(&Msg {
                event: Some("foo"),
                id: Some(&format!("evt-{i}")),
                data: Some(&format!("Hello {i}")),
                ..Msg::default()
            })
        })
        .collect::<Vec<_>>()
        .concat();
    let got = events(vec![chunks]).await;
    assert_eq!(got.len(), 10);
    assert_eq!(triples(&got)[0], (Some("evt-0"), Some("foo"), "Hello 0"));
    assert_eq!(triples(&got)[9], (Some("evt-9"), Some("foo"), "Hello 9"));
}

// ── aimux-specific behaviour (not covered upstream) ──────────────────────

mod aimux {
    use super::*;

    #[tokio::test]
    async fn invalid_utf8_is_a_terminal_error() {
        // Upstream decodes lossily (`TextDecoder`); SseStream decodes strictly
        // and, like every decoder error, ends the stream after reporting it.
        // `E4 BD` is the start of `你` without its last byte.
        let mut bytes = b"data: ".to_vec();
        bytes.extend_from_slice(&[0xE4, 0xBD]);
        bytes.extend_from_slice(b"\ndata: tail\n\ndata: next\n\n");
        let results = run_bytes(vec![bytes]).await;
        assert_eq!(results.len(), 1);
        assert!(matches!(results[0], Err(SseError::Utf8(_))));
    }

    #[tokio::test]
    async fn multibyte_fixture_split_at_arbitrary_byte_sizes() {
        // Chunks can end inside a code point or inside the BOM; the parser
        // works on bytes, so the result must not depend on where they end.
        let mb = multibyte();
        let mut stream_bytes = "\u{FEFF}".as_bytes().to_vec();
        stream_bytes.extend(multibyte_chunks(&mb).concat().into_bytes());
        for size in [1, 2, 3, 5, 8, 13] {
            let chunks = stream_bytes.chunks(size).map(<[u8]>::to_vec).collect();
            let got: Vec<_> = run_bytes(chunks)
                .await
                .into_iter()
                .map(|item| {
                    let e = item.unwrap_or_else(|err| panic!("chunk size {size}: {err:?}"));
                    (e.id, e.event, e.data)
                })
                .collect();
            assert_eq!(got, mb.expected, "chunk size {size}");
        }
    }

    #[tokio::test]
    async fn transport_error_is_reported_as_a_stream_error() {
        let items: Vec<Result<Bytes, std::io::Error>> = vec![
            Ok(Bytes::from_static(b"data: a\n\n")),
            Err(std::io::Error::other("boom")),
            Ok(Bytes::from_static(b"data: b\n\n")),
        ];
        let results = SseStream::new(stream::iter(items))
            .collect::<Vec<_>>()
            .await;
        // The transport error is terminal: nothing after it is read.
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].as_ref().unwrap().data, "a");
        assert!(matches!(&results[1], Err(SseError::Stream(e)) if e.to_string() == "boom"));
    }
}
