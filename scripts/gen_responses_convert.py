#!/usr/bin/env python3
"""Generate `openai/responses/responses_convert.rs` (RFC-0012 §3.5 shared framework).

Extracts the byte-identical non-streaming parser and streaming SSE reducer from
`openai/responses/mod.rs` (which Azure duplicates verbatim) into a shared module,
so OpenAI and Azure call one implementation. The bodies are copied verbatim from
the OpenAI source to guarantee zero behavioral drift; only module-qualified or
caller-local identifiers that must resolve differently in the new module are
rewritten:
  - `convert::parse_usage`        -> `parse_usage`          (imported here)
  - `provider_key_stream`          -> `provider_key`          (the param)
  - `request_result.warnings`     -> `request_warnings`      (the param)
The `event_iter` declaration lives *inside* the `stream!` block in the source
(lines 414-415), so it is captured along with `first_event`/`sse_stream` — it is
NOT pre-declared here.
"""

from pathlib import Path

REPO = Path("/media/eric8810/fast-deliver/code/aimux")
SRC = REPO / "aimux-providers/src/openai/responses/mod.rs"
OUT = REPO / "aimux-providers/src/openai/responses/responses_convert.rs"

lines = SRC.read_text(encoding="utf-8").splitlines(keepends=True)


def find(marker: str, after: int = 0) -> int:
    """Return the 1-indexed line number of the first line whose stripped
    content equals `marker`, searching from line `after` (1-indexed) onward.
    """
    for i in range(after, len(lines)):
        if lines[i].strip() == marker:
            return i + 1
    raise ValueError(f"marker not found: {marker!r}")


# do_generate core: from the "Top-level error field" comment through the `})`
# that closes `Ok(GenerateResult { ... })`.
gen_start = find("// Top-level error field.")
ok_line = find("Ok(GenerateResult {", after=gen_start)
gen_close = find("})", after=ok_line)
generate_body = "".join(lines[gen_start - 1 : gen_close])

# do_stream reducer: the interior of `async_stream::stream! { ... }`, i.e. from
# the line after the `stream! {` opener up to (excluding) the `};` closer. The
# closer is the `};` that immediately precedes `Ok(StreamResult {` — scanning
# backward from that return avoids matching inner `match { };` closers.
stream_open = find("let stream = async_stream::stream! {")
ok_stream = find("Ok(StreamResult {", after=stream_open)
sc = ok_stream - 1
while sc > stream_open and lines[sc - 1].strip() != "};":
    sc -= 1
stream_close = sc
stream_body = "".join(lines[stream_open : stream_close - 1])

# Rewrite identifiers so the verbatim bodies resolve inside the shared module.
generate_body = generate_body.replace("convert::parse_usage", "parse_usage")
generate_body = generate_body.replace("request_result.warnings", "request_warnings")
stream_body = stream_body.replace("convert::parse_usage", "parse_usage")
stream_body = stream_body.replace("provider_key_stream", "provider_key")


def reindent(block: str, from_n: int, to_n: int) -> str:
    """Strip `from_n` leading spaces and prepend `to_n` (net shift to_n-from_n)."""
    out = []
    for line in block.splitlines(keepends=True):
        if line.strip() == "":
            out.append("\n")
            continue
        if line.startswith(" " * from_n):
            out.append(" " * to_n + line[from_n:])
        else:
            out.append(line)
    return "".join(out)


HEADER = """\ufeff//! Shared Responses API framework (RFC-0012 \u00a73.5).
//!
//! Vendors whose Responses implementations speak the OpenAI wire format
//! (currently OpenAI and Azure OpenAI) share this module for the parts that are
//! byte-identical across them:
//! - non-streaming output parsing \u2014 [`build_responses_generate_result`],
//! - the streaming SSE event reducer \u2014 [`build_responses_event_stream`],
//! - common HTTP header list construction \u2014 [`build_header_list`].
//!
//! Vendors with genuinely different protocols (xAI, HuggingFace, the generic
//! `open_responses` provider) keep their own request/streaming logic and reuse
//! only the small shared helpers where they are byte-identical. Per the RFC,
//! genuinely different streaming loops are **not** force-merged into one
//! function \u2014 only the shared framework is extracted.

use std::collections::HashMap;
use std::pin::Pin;

use futures::{Stream, StreamExt};
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::result::{GenerateContent, GenerateResult};
use aimux_core::stream_part::StreamPart;
use aimux_core::types::{FinishReason, FinishReasonUnified, ResponseMetadata, Usage, Warning};
use aimux_stream::{SseError, SseEvent};

use super::convert::{convert_responses_usage, map_responses_finish_reason, parse_usage};
use super::types::ResponsesUsage;

/// Pinned, boxed stream of model stream parts.
///
/// Matches the `stream` field of [`aimux_core::result::StreamResult`]. Used as
/// the return type of the shared streaming reducer so the boxed trait object
/// does not leak a complex type into call sites.
pub type ResponsesEventStream = Pin<Box<dyn Stream<Item = Result<StreamPart, AiMuxError>> + Send>>;

/// Build the `Vec<(String, String)>` header list for an `HttpRequest`, appending
/// `Content-Type: application/json`.
///
/// Byte-identical copies previously lived in the OpenAI, HuggingFace and xAI
/// responses modules; they now route through this single implementation.
pub fn build_header_list(headers: &HashMap<String, String>) -> Vec<(String, String)> {
    let mut list: Vec<(String, String)> =
        headers.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    list.push(("Content-Type".to_string(), "application/json".to_string()));
    list
}

// -- Non-streaming output parsing --------------------------------------------

/// Parse a non-streaming Responses API JSON body into a [`GenerateResult`].
///
/// Shared verbatim by the OpenAI and Azure providers: both speak the same
/// Responses wire format for non-streaming output (top-level error, `output`
/// array of `message`/`function_call`/`custom_tool_call`/`reasoning` items,
/// `incomplete_details`, `usage`, provider metadata with `responseId` /
/// `reasoningContext` / `serviceTier`). Vendor callers supply the parsed `data`,
/// the request `body`/`response_headers` to attach, and the provider-metadata
/// namespace `provider_key` ("openai" / "azure").
pub fn build_responses_generate_result(
    data: &Value,
    request_warnings: Vec<Warning>,
    provider_key: String,
    body: Value,
    response_headers: HashMap<String, String>,
) -> Result<GenerateResult, AiMuxError> {
"""

generate_body_i = reindent(generate_body, 8, 4)
FOOTER_GENERATE = "}\n\n"

STREAM_HEADER = """// -- Streaming SSE event reducer ---------------------------------------------

/// A tool call being streamed (tracked by `output_index`).
#[allow(dead_code)]
struct OngoingToolCall {
    tool_name: String,
    tool_call_id: String,
}

/// A reasoning item being streamed (tracked by `item_id`).
struct ReasoningState {
    encrypted_content: Option<String>,
    /// summary_index \u2192 status.
    summary_parts: HashMap<usize, SummaryStatus>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum SummaryStatus {
    Active,
    CanConclude,
    Concluded,
}

/// Build the streaming event reducer shared by the OpenAI and Azure providers.
///
/// Both speak the same Responses streaming wire format (the
/// `response.created -> output_item.added -> output_text.delta ->
/// output_item.done -> response.completed` main path, plus
/// `function_call_arguments.delta`, `custom_tool_call_input.delta`,
/// `reasoning_summary_part.added/done` and `reasoning_summary_text.delta`).
///
/// The caller performs the HTTP send (`send_stream`) and hands the peeked
/// `first_event` plus the remainder `sse_stream` to this reducer; an early
/// `error` / `response.failed` surfaces as a clean `Err` here.
pub fn build_responses_event_stream<S>(
    first_event: Option<Result<SseEvent, SseError>>,
    sse_stream: S,
    provider_key: String,
    warnings: Vec<Warning>,
    store_flag: bool,
) -> Result<ResponsesEventStream, AiMuxError>
where
    S: Stream<Item = Result<SseEvent, SseError>> + Unpin + Send + 'static,
{
    // Peek at the first SSE event to detect early errors (before any output).
    if let Some(Ok(ref event)) = first_event
        && let Ok(val) = serde_json::from_str::<Value>(&event.data)
    {
        let etype = val.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if etype == "error" || etype == "response.failed" {
            let message = val
                .get("response")
                .and_then(|r| r.get("error"))
                .and_then(|e| e.get("message"))
                .and_then(|v| v.as_str())
                .or_else(|| {
                    val.get("error")
                        .and_then(|e| e.get("message"))
                        .and_then(|v| v.as_str())
                })
                .unwrap_or("Responses API stream error");
            return Err(AiMuxError::Provider(message.to_string()));
        }
    }

    let stream = async_stream::stream! {
"""

stream_body_i = reindent(stream_body, 12, 8)
STREAM_FOOTER = """    };
    Ok(Box::pin(stream))
}
"""

content = (
    HEADER
    + generate_body_i
    + FOOTER_GENERATE
    + STREAM_HEADER
    + stream_body_i
    + STREAM_FOOTER
)

OUT.write_text(content, encoding="utf-8")
print(f"wrote {OUT} ({len(content.splitlines())} lines)")
