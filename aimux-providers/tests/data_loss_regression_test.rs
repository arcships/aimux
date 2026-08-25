//! Precise-assertion regression tests for the `fix/stop-dropping-response-data`
//! branch (PR1 findings 1–35: "provider returned data, aimux silently dropped it").
//!
//! # Why this file exists
//!
//! The pre-existing cassette replay suite (`cassette_full_test.rs` &c.) only
//! asserts weak conditions such as "text is non-empty". Every one of the 35
//! data-loss bugs survived that suite unchanged, because dropping a field never
//! makes the text empty. These tests therefore assert **exact values** taken
//! from the real recorded cassettes: a URL, a base64 prefix, a signature, a
//! `retrievedAt` timestamp. If a field is dropped again, the equality fails.
//!
//! # Conventions
//!
//! - Every test is named `finding_N_...` so it maps back to
//!   `reference/PR1-FINDINGS.md`.
//! - Response bodies come from the checked-in cassettes under
//!   `tests/cassettes/<provider>/`; no new fixtures are invented. The cassette
//!   is loaded from disk, mounted verbatim on a `wiremock` server, and the
//!   provider is pointed at that server.
//! - Assertions compare against literals copied out of the cassette, not
//!   against re-reads of the cassette (a re-read would pass even if both the
//!   producer and the assertion regressed together).

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use futures::StreamExt;
use serde_json::{Value, json};
use wiremock::matchers::{method, path as path_matcher, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::content::ContentPart;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelPrompt, LanguageModelPromptMessage};
use aimux_core::message::Role;
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateContent, StreamResult};
use aimux_core::shared::{FileBytes, FileData};
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::{ProviderTool, Tool};

use aimux_providers::anthropic::AnthropicConfig;
use aimux_providers::anthropic::model::AnthropicModel;
use aimux_providers::bedrock::{BedrockAuth, BedrockConfig, BedrockModel, event_stream};
use aimux_providers::{
    CohereConfig, CohereProvider, GoogleConfig, GoogleProvider, MistralConfig, MistralProvider,
    OpenAIConfig, OpenAIProvider,
};

// ═══════════════════════════════════════════════════════════════════════════
// Cassette loading
// ═══════════════════════════════════════════════════════════════════════════

/// A cassette's recorded exchange, reduced to what a mock needs.
struct Cassette {
    /// `request.path`, percent-decoded (Bedrock records `%3A` for `:`).
    request_path: String,
    status: u16,
    /// `response.body` verbatim (a raw JSON / SSE string).
    body: String,
}

/// Load one cassette by `<provider>/<file>.json` relative to `tests/cassettes`.
///
/// Panics loudly on any problem — a missing or malformed fixture is a broken
/// test, not a runtime condition.
fn cassette(rel: &str) -> Cassette {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests/cassettes");
    p.push(rel);
    let text = fs::read_to_string(&p)
        .unwrap_or_else(|e| panic!("cannot read cassette {}: {e}", p.display()));
    let v: Value = serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("invalid JSON in cassette {}: {e}", p.display()));

    let request_path = v["request"]["path"]
        .as_str()
        .unwrap_or_else(|| panic!("{rel}: missing request.path"))
        .replace("%3A", ":");
    let status = v["response"]["status"]
        .as_u64()
        .unwrap_or_else(|| panic!("{rel}: missing response.status")) as u16;
    let body = v["response"]["body"]
        .as_str()
        .unwrap_or_else(|| panic!("{rel}: response.body must be a raw string"))
        .to_string();

    Cassette {
        request_path,
        status,
        body,
    }
}

/// Mount a cassette's response on `server` at its recorded path.
async fn mount(server: &MockServer, c: &Cassette) {
    Mock::given(method("POST"))
        .and(path_matcher(c.request_path.clone()))
        .respond_with(
            ResponseTemplate::new(c.status)
                .insert_header("content-type", "application/json")
                .set_body_string(c.body.clone()),
        )
        .mount(server)
        .await;
}

/// The parsed JSON of a cassette's response body (used to lift real payloads
/// into synthetic streaming frames — the *values* still come from the wire).
fn cassette_json(rel: &str) -> Value {
    serde_json::from_str(&cassette(rel).body)
        .unwrap_or_else(|e| panic!("{rel}: response.body is not JSON: {e}"))
}

// ═══════════════════════════════════════════════════════════════════════════
// Shared helpers
// ═══════════════════════════════════════════════════════════════════════════

fn user_prompt(text: &str) -> LanguageModelPrompt {
    vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![ContentPart::text(text)],
        ..Default::default()
    }]
}

fn opts() -> CallOptions {
    CallOptions::new(user_prompt("Hello"))
}

fn opts_with_tools(tools: Vec<Tool>) -> CallOptions {
    let mut o = opts();
    o.tools = Some(tools);
    o
}

fn provider_tool(id: &str, name: &str) -> Tool {
    Tool::Provider(ProviderTool {
        id: id.to_string(),
        name: name.to_string(),
        args: json!({}),
    })
}

async fn collect(result: StreamResult) -> Vec<StreamPart> {
    let mut parts = Vec::new();
    let mut stream = result.stream;
    while let Some(p) = stream.next().await {
        parts.push(p.expect("stream part must be Ok"));
    }
    parts
}

/// The base64 payload of a `GenerateContent::File`, or panic.
fn file_base64(c: &GenerateContent) -> &str {
    match c {
        GenerateContent::File {
            data: FileData::Data {
                data: FileBytes::Base64(s),
            },
            ..
        } => s,
        other => panic!("expected File with base64 data, got {other:?}"),
    }
}

fn files(content: &[GenerateContent]) -> Vec<&GenerateContent> {
    content
        .iter()
        .filter(|c| matches!(c, GenerateContent::File { .. }))
        .collect()
}

fn texts(content: &[GenerateContent]) -> Vec<&str> {
    content
        .iter()
        .filter_map(|c| match c {
            GenerateContent::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn reasonings(content: &[GenerateContent]) -> Vec<(&str, Option<&Value>)> {
    content
        .iter()
        .filter_map(|c| match c {
            GenerateContent::Reasoning {
                text,
                provider_metadata,
            } => Some((text.as_str(), provider_metadata.as_ref())),
            _ => None,
        })
        .collect()
}

/// `(tool_call_id, tool_name, input, provider_executed, dynamic, thought_signature, metadata)`
type ToolCallView<'a> = (
    &'a str,
    &'a str,
    Value,
    Option<bool>,
    Option<bool>,
    Option<&'a str>,
    Option<&'a Value>,
);

fn tool_calls(content: &[GenerateContent]) -> Vec<ToolCallView<'_>> {
    content
        .iter()
        .filter_map(|c| match c {
            GenerateContent::ToolCall {
                tool_call_id,
                tool_name,
                input,
                provider_executed,
                dynamic,
                thought_signature,
                provider_metadata,
            } => {
                // Provider results intentionally carry the exact wire string;
                // parse only in this assertion helper so nested data-loss
                // checks remain readable without weakening that boundary.
                let parsed_input = input
                    .as_str()
                    .and_then(|raw| serde_json::from_str(raw).ok())
                    .unwrap_or_else(|| input.clone());
                Some((
                    tool_call_id.as_str(),
                    tool_name.as_str(),
                    parsed_input,
                    *provider_executed,
                    *dynamic,
                    thought_signature.as_deref(),
                    provider_metadata.as_ref(),
                ))
            }
            _ => None,
        })
        .collect()
}

/// `(tool_call_id, tool_name, result, is_error, dynamic, metadata)`
type ToolResultView<'a> = (
    &'a str,
    &'a str,
    &'a Value,
    Option<bool>,
    Option<bool>,
    Option<&'a Value>,
);

fn tool_results(content: &[GenerateContent]) -> Vec<ToolResultView<'_>> {
    content
        .iter()
        .filter_map(|c| match c {
            GenerateContent::ToolResult {
                tool_call_id,
                tool_name,
                result,
                is_error,
                dynamic,
                provider_metadata,
                ..
            } => Some((
                tool_call_id.as_str(),
                tool_name.as_str(),
                result,
                *is_error,
                *dynamic,
                provider_metadata.as_ref(),
            )),
            _ => None,
        })
        .collect()
}

/// `(id, source_type, url, title, metadata)`
type SourceView<'a> = (
    &'a str,
    &'a str,
    Option<&'a str>,
    Option<&'a str>,
    Option<&'a Value>,
);

fn sources(content: &[GenerateContent]) -> Vec<SourceView<'_>> {
    content
        .iter()
        .filter_map(|c| match c {
            GenerateContent::Source {
                id,
                source_type,
                url,
                title,
                provider_metadata,
            } => Some((
                id.as_str(),
                source_type.as_str(),
                url.as_deref(),
                title.as_deref(),
                provider_metadata.as_ref(),
            )),
            _ => None,
        })
        .collect()
}

// ═══════════════════════════════════════════════════════════════════════════
// Finding 1 — Gemini `inlineData` never read (generate + stream)
//
// Cassettes: gemini/nano_banana_image_generation_smoke.json,
//            gemini/test_google_image_and_text_output.json
// ═══════════════════════════════════════════════════════════════════════════

/// PNG magic bytes, base64-encoded: every image in the two Gemini image
/// cassettes starts with this. Asserting the prefix (rather than "non-empty")
/// proves the *actual* bytes survived rather than some placeholder.
const PNG_BASE64_PREFIX: &str = "iVBORw0KGgoAAAANSUhEUgAA";

/// The exact base64 length recorded in `nano_banana_image_generation_smoke`.
const NANO_BANANA_BASE64_LEN: usize = 258_820;

fn google_at(uri: &str) -> GoogleProvider {
    GoogleProvider::new(GoogleConfig::new("test-api-key").with_base_url(format!("{uri}/v1beta")))
}

#[tokio::test]
async fn finding_1_gemini_inline_data_surfaces_as_file() {
    let c = cassette("gemini/nano_banana_image_generation_smoke.json");
    let server = MockServer::start().await;
    mount(&server, &c).await;

    let result = google_at(&server.uri())
        .model("gemini-2.5-flash-image")
        .do_generate(&opts())
        .await
        .expect("do_generate should succeed");

    let f = files(&result.content);
    assert_eq!(f.len(), 1, "exactly one inlineData part → one File");
    match f[0] {
        GenerateContent::File { media_type, .. } => {
            assert_eq!(media_type, "image/png");
        }
        other => panic!("expected File, got {other:?}"),
    }
    let b64 = file_base64(f[0]);
    assert!(
        b64.starts_with(PNG_BASE64_PREFIX),
        "image bytes must be the recorded PNG, got prefix {:?}",
        &b64[..b64.len().min(32)]
    );
    assert_eq!(
        b64.len(),
        NANO_BANANA_BASE64_LEN,
        "the whole base64 payload must survive, not a truncated head"
    );
}

#[tokio::test]
async fn finding_1_gemini_image_and_text_output_both_survive() {
    let c = cassette("gemini/test_google_image_and_text_output.json");
    let server = MockServer::start().await;
    mount(&server, &c).await;

    let result = google_at(&server.uri())
        .model("gemini-2.5-flash-image")
        .do_generate(&opts())
        .await
        .expect("do_generate should succeed");

    // Text part first, then the inlineData part — order matters, upstream
    // walks `parts` in order.
    assert_eq!(result.content.len(), 2, "one text + one file");
    assert!(
        matches!(result.content[0], GenerateContent::Text { .. }),
        "text part comes first"
    );
    assert!(
        matches!(result.content[1], GenerateContent::File { .. }),
        "inlineData part comes second"
    );

    let t = texts(&result.content);
    assert_eq!(t.len(), 1);
    assert!(
        t[0].starts_with("Once, in a hidden cenote, lived an axolotl named Pip"),
        "recorded story text must survive verbatim, got {:?}",
        &t[0][..t[0].len().min(60)]
    );

    let f = files(&result.content);
    assert_eq!(f.len(), 1);
    match f[0] {
        GenerateContent::File { media_type, .. } => assert_eq!(media_type, "image/png"),
        other => panic!("expected File, got {other:?}"),
    }
    let b64 = file_base64(f[0]);
    assert!(b64.starts_with(PNG_BASE64_PREFIX));
    assert_eq!(b64.len(), 2_580_504);
}

/// The streaming path had the same hole. The chunk is built from the cassette's
/// own `parts` array so the bytes are the recorded ones.
#[tokio::test]
async fn finding_1_gemini_inline_data_streams_as_file_part() {
    let body = cassette_json("gemini/nano_banana_image_generation_smoke.json");
    let candidate = &body["candidates"][0];
    let sse = format!(
        "data: {}\n\n",
        json!({ "candidates": [{ "content": candidate["content"].clone(), "finishReason": "STOP" }] })
    );

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path_matcher(
            "/v1beta/models/gemini-2.5-flash-image:streamGenerateContent",
        ))
        .and(query_param("alt", "sse"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse),
        )
        .mount(&server)
        .await;

    let parts = collect(
        google_at(&server.uri())
            .model("gemini-2.5-flash-image")
            .do_stream(&opts())
            .await
            .expect("do_stream should succeed"),
    )
    .await;

    let file_parts: Vec<_> = parts
        .iter()
        .filter_map(|p| match p {
            StreamPart::File {
                data:
                    FileData::Data {
                        data: FileBytes::Base64(b64),
                    },
                media_type,
                ..
            } => Some((b64.as_str(), media_type.as_str())),
            _ => None,
        })
        .collect();
    assert_eq!(file_parts.len(), 1, "expected one StreamPart::File");
    assert_eq!(file_parts[0].1, "image/png");
    assert!(file_parts[0].0.starts_with(PNG_BASE64_PREFIX));
    assert_eq!(file_parts[0].0.len(), NANO_BANANA_BASE64_LEN);
}

// ═══════════════════════════════════════════════════════════════════════════
// Finding 3 — Gemini `part.thought` never read (thinking merged into the answer)
// Finding 5 — Gemini `thoughtSignature` dropped
//
// Cassette: gemini/test_google_model_thinking_part.json
//   parts[0] = { text: "**A Safe Street-Crossing Guide…", thought: true }
//   parts[1] = { text: "Crossing the street safely…", thoughtSignature: "Eqoe…" }
// ═══════════════════════════════════════════════════════════════════════════

/// First 16 chars of the real `thoughtSignature` on parts[1].
const GEMINI_THOUGHT_SIG_PREFIX: &str = "EqoeCqceAdHtim+c";
const GEMINI_THOUGHT_SIG_LEN: usize = 5180;

#[tokio::test]
async fn finding_3_gemini_thought_part_becomes_reasoning_not_text() {
    let c = cassette("gemini/test_google_model_thinking_part.json");
    let server = MockServer::start().await;
    mount(&server, &c).await;

    let result = google_at(&server.uri())
        .model("gemini-3-pro-preview")
        .do_generate(&opts())
        .await
        .expect("do_generate should succeed");

    let r = reasonings(&result.content);
    assert_eq!(r.len(), 1, "the `thought: true` part is the only reasoning");
    assert!(
        r[0].0
            .starts_with("**A Safe Street-Crossing Guide: My Thought Process**"),
        "reasoning must be the thought part verbatim, got {:?}",
        &r[0].0[..r[0].0.len().min(60)]
    );
    assert_eq!(
        r[0].0.chars().count(),
        2238,
        "the full thought text, not a fragment"
    );

    let t = texts(&result.content);
    assert_eq!(t.len(), 1, "only the non-thought part is answer text");
    assert!(
        t[0].starts_with("Crossing the street safely is a fundamental skill"),
        "answer text must be the non-thought part"
    );
    // The regression this guards: the thought text used to be concatenated
    // into the answer.
    assert!(
        !t[0].contains("My Thought Process"),
        "the thinking must NOT leak into the answer text"
    );
    assert_eq!(t[0].chars().count(), 3017);
}

#[tokio::test]
async fn finding_5_gemini_thought_signature_reaches_provider_metadata() {
    let c = cassette("gemini/test_google_model_thinking_part.json");
    let server = MockServer::start().await;
    mount(&server, &c).await;

    let result = google_at(&server.uri())
        .model("gemini-3-pro-preview")
        .do_generate(&opts())
        .await
        .expect("do_generate should succeed");

    // The signature rides on the *text* part in this cassette.
    let sig = result
        .content
        .iter()
        .find_map(|c| match c {
            GenerateContent::Text {
                provider_metadata: Some(m),
                ..
            } => m["google"]["thoughtSignature"].as_str(),
            _ => None,
        })
        .expect("text part must carry providerMetadata.google.thoughtSignature");
    assert!(
        sig.starts_with(GEMINI_THOUGHT_SIG_PREFIX),
        "thoughtSignature must be the recorded one, got {:?}",
        &sig[..sig.len().min(24)]
    );
    assert_eq!(
        sig.len(),
        GEMINI_THOUGHT_SIG_LEN,
        "the signature must be echoed back byte-for-byte (truncation breaks the next turn)"
    );
}

/// `thoughtSignature` on a `functionCall` part must reach
/// `ToolCall.thought_signature` — that is the field the follow-up turn echoes.
#[tokio::test]
async fn finding_5_gemini_function_call_thought_signature_round_trips() {
    let c = cassette("gemini/two_arg_rewrites_chain_blocking_0.json");
    let server = MockServer::start().await;
    mount(&server, &c).await;

    let result = google_at(&server.uri())
        .model("gemini-2.5-flash")
        .do_generate(&opts())
        .await
        .expect("do_generate should succeed");

    let calls = tool_calls(&result.content);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].1, "add");
    assert_eq!(calls[0].2, json!({ "x": 1, "y": 1 }));
    assert_eq!(
        calls[0].5,
        Some("signature_REDACTED_1"),
        "the functionCall thoughtSignature must land on ToolCall.thought_signature"
    );
    assert_eq!(
        calls[0].6.expect("providerMetadata")["google"]["thoughtSignature"],
        json!("signature_REDACTED_1")
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Finding 12 — Gemini `codeExecutionResult` dropped; tool_call_id was empty
//
// Cassette: gemini/test_code_execution.json
//   codeExecutionResult = { id: "h0mwtrhs", outcome: "OUTCOME_OK",
//                           output: "Current day in Utrecht: Tuesday, May 05, 2026\n" }
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn finding_12_gemini_code_execution_result_surfaced_with_matching_call_id() {
    let c = cassette("gemini/test_code_execution.json");
    let server = MockServer::start().await;
    mount(&server, &c).await;

    let result = google_at(&server.uri())
        .model("gemini-3-flash-preview")
        .do_generate(&opts_with_tools(vec![provider_tool(
            "google.code_execution",
            "code_execution",
        )]))
        .await
        .expect("do_generate should succeed");

    let calls = tool_calls(&result.content);
    let results = tool_results(&result.content);
    assert_eq!(calls.len(), 2, "two executableCode parts");
    assert_eq!(results.len(), 2, "two codeExecutionResult parts");

    // Exact values from the cassette's first result.
    assert_eq!(results[0].1, "code_execution");
    assert_eq!(
        results[0].2,
        &json!({
            "outcome": "OUTCOME_OK",
            "output": "Current day in Utrecht: Tuesday, May 05, 2026\n"
        }),
        "the recorded outcome AND output must both survive"
    );
    assert_eq!(
        results[1].2,
        &json!({ "outcome": "OUTCOME_OK", "output": "2026-05-05 20:40:33.367937\n" })
    );

    // The regression: `tool_call_id` used to be the empty string, so a client
    // could not pair the result with its call.
    assert!(
        !results[0].0.is_empty(),
        "ToolResult.tool_call_id must not be empty"
    );
    assert_eq!(
        results[0].0, calls[0].0,
        "result must carry the id of the executableCode call it answers"
    );
    assert_eq!(results[1].0, calls[1].0);
    assert_ne!(
        calls[0].0, calls[1].0,
        "the two code-execution calls need distinct ids"
    );

    // The executableCode payload itself is preserved on the call.
    assert_eq!(calls[0].2["language"], json!("PYTHON"));
    assert_eq!(calls[0].2["id"], json!("h0mwtrhs"));
    assert!(
        calls[0].2["code"]
            .as_str()
            .expect("code")
            .contains("Europe/Amsterdam"),
        "the executed source must survive verbatim"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Finding 14 — grounding sources never extracted
//
// Cassette: gemini/test_google_model_web_search_tool.json
//   groundingMetadata.groundingChunks[].web.{title,uri}
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn finding_14_gemini_grounding_chunks_become_sources() {
    let c = cassette("gemini/test_google_model_web_search_tool.json");
    let server = MockServer::start().await;
    mount(&server, &c).await;

    let result = google_at(&server.uri())
        .model("gemini-2.5-pro")
        .do_generate(&opts_with_tools(vec![provider_tool(
            "google.google_search",
            "google_search",
        )]))
        .await
        .expect("do_generate should succeed");

    let s = sources(&result.content);
    assert_eq!(s.len(), 3, "three groundingChunks → three sources");

    // Exact url + title of the first chunk.
    assert_eq!(s[0].1, "url");
    assert_eq!(
        s[0].2,
        Some("https://www.google.com/search?q=weather+in+San Francisco, CA,+US")
    );
    assert_eq!(
        s[0].3,
        Some("Weather information for San Francisco, CA, US")
    );

    assert_eq!(s[1].3, Some("weather.gov"));
    assert!(
        s[1].2
            .expect("url")
            .starts_with("https://vertexaisearch.cloud.google.com/grounding-api-redirect/AUZIYQF_uqo2G5Goeww8iF1L"),
        "the redirect URL must survive intact, got {:?}",
        s[1].2
    );
    assert_eq!(s[2].3, Some("wunderground.com"));

    // Source ids must be distinct, otherwise callers cannot key on them.
    assert_ne!(s[0].0, s[1].0);
    assert_ne!(s[1].0, s[2].0);
}

// ═══════════════════════════════════════════════════════════════════════════
// Finding 2 — Anthropic `_ => {}` swallowed every server-tool result block
//
// Cassettes under tests/cassettes/anthropic/. `parse_anthropic_content` now
// maps each block to a `GenerateContent::ToolResult` whose payload is
// camel-cased to match the upstream contract.
// ═══════════════════════════════════════════════════════════════════════════

fn anthropic_at(uri: &str) -> AnthropicModel {
    AnthropicModel::new(
        "claude-sonnet-4-0".to_string(),
        AnthropicConfig::new("test-api-key").with_base_url(uri.to_string()),
    )
}

#[tokio::test]
async fn finding_2_anthropic_web_search_result_mapped_and_sources_emitted() {
    let c = cassette("anthropic/test_pause_turn_web_search_vcr_1.json");
    let server = MockServer::start().await;
    mount(&server, &c).await;

    let result = anthropic_at(&server.uri())
        .do_generate(&opts_with_tools(vec![provider_tool(
            "anthropic.web_search_20250305",
            "web_search",
        )]))
        .await
        .expect("do_generate should succeed");

    let results = tool_results(&result.content);
    let first = results
        .iter()
        .find(|(id, ..)| *id == "srvtoolu_01RGq5wiPsxhz5Wk3Nj1w2JU")
        .expect("the first web_search_tool_result block must surface");
    assert_eq!(first.1, "web_search");
    let arr = first.2.as_array().expect("success payload is an array");
    assert_eq!(arr.len(), 10, "all ten recorded results, not just the head");

    // Exact fields of the first recorded search hit, in the camelCase shape.
    assert_eq!(
        arr[0]["url"],
        json!("https://www.iqair.com/us/usa/california/san-francisco")
    );
    assert_eq!(
        arr[0]["title"],
        json!("San Francisco Air Quality Index (AQI) and USA Air Pollution | IQAir")
    );
    assert_eq!(arr[0]["type"], json!("web_search_result"));
    assert_eq!(
        arr[0]["pageAge"],
        json!(null),
        "`page_age` is re-keyed to `pageAge` (null here, but present)"
    );
    assert!(
        arr[0]["encryptedContent"]
            .as_str()
            .expect("encryptedContent")
            .starts_with("EqAHCioIDBgCIiQ5MGZjMWI5Mi1iYzIyLTQzNTMtYWExZi1i"),
        "`encrypted_content` is re-keyed to `encryptedContent` and kept verbatim"
    );

    // The headline of the fix: each result also becomes a Source, which is the
    // only way its URL reaches `result.sources`.
    let s = sources(&result.content);
    assert!(
        s.iter().any(|(_, st, url, title, _)| {
            *st == "url"
                && *url == Some("https://www.iqair.com/us/usa/california/san-francisco")
                && *title
                    == Some("San Francisco Air Quality Index (AQI) and USA Air Pollution | IQAir")
        }),
        "the first hit must appear as a Source; got {:?}",
        s.iter().map(|x| x.2).collect::<Vec<_>>()
    );
    assert!(
        s.len() >= 10,
        "every web_search_result becomes a Source, got {}",
        s.len()
    );
    // Each Source carries `page_age` re-keyed to `pageAge`, and nothing else,
    // under the anthropic key. The expected values are spelled out rather than
    // read back out of `meta`: deriving them from the same object would make the
    // assertion agree with whatever the code produced, including a renamed key.
    // (`get("pageAge").is_some()` is no good either — a missing key and a
    // recorded `null` both read back as `Some(Value::Null)`.)
    let by_url: BTreeMap<&str, &Value> = s
        .iter()
        .filter_map(|(_, _, url, _, m)| Some(((*url)?, (*m)?)))
        .collect();
    for (url, expected_page_age) in [
        (
            "https://www.iqair.com/us/usa/california/san-francisco",
            json!(null),
        ),
        (
            "https://www.aqi.in/us/dashboard/united-states/california/san-francisco",
            json!("3 weeks ago"),
        ),
    ] {
        assert_eq!(
            by_url.get(url).copied(),
            Some(&json!({ "anthropic": { "pageAge": expected_page_age } })),
            "source {url}: providerMetadata must hold exactly anthropic.pageAge"
        );
    }

    // The server_tool_use call itself is surfaced so it round-trips.
    let calls = tool_calls(&result.content);
    assert!(
        calls.iter().any(|(id, name, input, ..)| {
            *id == "srvtoolu_01Co4g1amCdYwEhUNBpoQEgp"
                && *name == "web_search"
                && input["query"] == json!("latest news events San Francisco this week")
        }),
        "the server_tool_use block must surface as a ToolCall"
    );
}

/// A renamed provider tool must be reported under the caller's name.
#[tokio::test]
async fn finding_2_anthropic_web_search_result_uses_the_callers_tool_name() {
    let c = cassette("anthropic/test_pause_turn_web_search_vcr_1.json");
    let server = MockServer::start().await;
    mount(&server, &c).await;

    let result = anthropic_at(&server.uri())
        .do_generate(&opts_with_tools(vec![provider_tool(
            "anthropic.web_search_20250305",
            "mySearch",
        )]))
        .await
        .expect("do_generate should succeed");

    let results = tool_results(&result.content);
    assert!(!results.is_empty());
    assert!(
        results.iter().all(|(_, name, ..)| *name == "mySearch"),
        "wire name `web_search` must be mapped back to the caller's `mySearch`"
    );
}

#[tokio::test]
async fn finding_2_anthropic_web_fetch_result_mapped() {
    let c = cassette("anthropic/test_anthropic_web_fetch_tool.json");
    let server = MockServer::start().await;
    mount(&server, &c).await;

    let result = anthropic_at(&server.uri())
        .do_generate(&opts_with_tools(vec![provider_tool(
            "anthropic.web_fetch_20250910",
            "web_fetch",
        )]))
        .await
        .expect("do_generate should succeed");

    let results = tool_results(&result.content);
    assert_eq!(results.len(), 1, "one web_fetch_tool_result block");
    let (id, name, payload, is_error, _, _) = results[0];
    assert_eq!(id, "srvtoolu_01So85wNUocinTvFfgKCfQeb");
    assert_eq!(name, "web_fetch");
    assert_eq!(is_error, None);

    assert_eq!(payload["type"], json!("web_fetch_result"));
    assert_eq!(payload["url"], json!("https://ai.pydantic.dev"));
    assert_eq!(
        payload["retrievedAt"],
        json!("2025-11-14T23:34:21.151000+00:00"),
        "`retrieved_at` is re-keyed to `retrievedAt` with the recorded timestamp"
    );
    assert_eq!(payload["content"]["type"], json!("document"));
    assert_eq!(payload["content"]["title"], json!("Pydantic AI"));
    assert_eq!(payload["content"]["source"]["type"], json!("text"));
    assert_eq!(
        payload["content"]["source"]["mediaType"],
        json!("text/plain"),
        "`media_type` is re-keyed to `mediaType`"
    );
    assert!(
        payload["content"]["source"]["data"]
            .as_str()
            .expect("fetched document body")
            .starts_with("Pydantic AI\nGenAI Agent Framework, the Pydantic way"),
        "the fetched page body must survive verbatim"
    );

    // The server_tool_use call that produced it.
    let calls = tool_calls(&result.content);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "srvtoolu_01So85wNUocinTvFfgKCfQeb");
    assert_eq!(calls[0].2, json!({ "url": "https://ai.pydantic.dev" }));

    // The thinking block on the same response keeps its signature.
    let r = reasonings(&result.content);
    assert_eq!(r.len(), 1);
    assert!(
        r[0].1.expect("thinking metadata")["anthropic"]["signature"]
            .as_str()
            .expect("signature")
            .starts_with("EsIDCkYICRgCKkAKi/j4a8lGN12CjyS27ZXcPkXHGyTbn1vJENJz"),
        "the thinking signature must survive for the next turn"
    );
}

#[tokio::test]
async fn finding_2_anthropic_bash_code_execution_result_passed_through() {
    let c = cassette("anthropic/test_anthropic_code_execution_files_multi_turn_1.json");
    let server = MockServer::start().await;
    mount(&server, &c).await;

    let result = anthropic_at(&server.uri())
        .do_generate(&opts_with_tools(vec![provider_tool(
            "anthropic.code_execution_20250825",
            "code_execution",
        )]))
        .await
        .expect("do_generate should succeed");

    let results = tool_results(&result.content);
    assert_eq!(results.len(), 1);
    let (id, name, payload, ..) = results[0];
    assert_eq!(id, "srvtoolu_01P9KoHAS2PSorMZSSjLfQ5N");
    assert_eq!(name, "code_execution");
    // `bash_code_execution_tool_result` is passed through unmapped upstream.
    assert_eq!(payload["type"], json!("bash_code_execution_result"));
    assert_eq!(
        payload["stdout"],
        json!("Sum of value column: 100.0\n"),
        "the computed answer lives only in stdout"
    );
    assert_eq!(payload["stderr"], json!(""));
    assert_eq!(payload["return_code"], json!(0));
    assert_eq!(payload["content"], json!([]));

    // The result must be paired with its call id.
    let calls = tool_calls(&result.content);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, id, "call and result share the tool_use_id");
    // The bash variant collapses into the caller's single code_execution
    // tool; the wire name survives in the normalized input's `type`.
    assert_eq!(calls[0].1, "code_execution");
    assert_eq!(calls[0].2["type"], json!("bash_code_execution"));
}

#[tokio::test]
async fn finding_2_anthropic_tool_search_result_lists_tool_references() {
    let c = cassette("anthropic/test_anthropic_native_tool_search.json");
    let server = MockServer::start().await;
    mount(&server, &c).await;

    // Rename the bm25 tool so the mapping (and `tool_search_provider_name`)
    // resolves to the bm25 variant rather than the regex default.
    let result = anthropic_at(&server.uri())
        .do_generate(&opts_with_tools(vec![provider_tool(
            "anthropic.tool_search_bm25_20251119",
            "findTools",
        )]))
        .await
        .expect("do_generate should succeed");

    let results = tool_results(&result.content);
    assert_eq!(results.len(), 1);
    let (id, name, payload, is_error, ..) = results[0];
    assert_eq!(id, "srvtoolu_01AFqvRopGELemw8jFcntaRt");
    assert_eq!(
        name, "findTools",
        "bm25 wire name maps back to the caller's tool name"
    );
    assert_eq!(is_error, None);

    // Success collapses to the tool-reference array, camel-cased.
    assert_eq!(
        payload,
        &json!([
            { "type": "tool_reference", "toolName": "get_exchange_rate" },
            { "type": "tool_reference", "toolName": "mortgage_calculator" },
        ]),
        "both discovered tools must survive, with `tool_name` → `toolName`"
    );

    // The client-executed tool_use that follows is untouched.
    let calls = tool_calls(&result.content);
    assert!(
        calls.iter().any(|(id, name, input, ..)| {
            *id == "toolu_01WYuJPXi77h8uRRgi6F7owo"
                && *name == "get_exchange_rate"
                && input["from_currency"] == json!("USD")
                && input["to_currency"] == json!("EUR")
        }),
        "the follow-up tool_use must still surface"
    );
}

#[tokio::test]
async fn finding_2_anthropic_advisor_result_keeps_text_and_stop_reason() {
    let c = cassette("anthropic/test_anthropic_advisor_tool.json");
    let server = MockServer::start().await;
    mount(&server, &c).await;

    let result = anthropic_at(&server.uri())
        .do_generate(&opts_with_tools(vec![provider_tool(
            "anthropic.advisor_20260301",
            "advisor",
        )]))
        .await
        .expect("do_generate should succeed");

    let results = tool_results(&result.content);
    assert_eq!(results.len(), 1);
    let (id, name, payload, is_error, ..) = results[0];
    assert_eq!(id, "srvtoolu_01HjzmxWnLPCkNoLmrowWNBc");
    assert_eq!(name, "advisor");
    assert_eq!(is_error, None);
    assert_eq!(
        payload,
        &json!({
            "type": "advisor_result",
            "text": "4.\n\n(You're right--it's trivial. Ship it.)",
            "stopReason": "end_turn",
        }),
        "advisor text and `stop_reason` → `stopReason` must both survive"
    );
}

#[tokio::test]
async fn finding_2_anthropic_mcp_tool_use_and_result_are_dynamic_and_named() {
    let c = cassette("anthropic/test_anthropic_mcp_servers.json");
    let server = MockServer::start().await;
    mount(&server, &c).await;

    let result = anthropic_at(&server.uri())
        .do_generate(&opts())
        .await
        .expect("do_generate should succeed");

    // mcp_tool_use → provider-executed + dynamic + serverName metadata.
    let calls = tool_calls(&result.content);
    assert_eq!(calls.len(), 1);
    let (id, name, ref input, provider_executed, dynamic, _, meta) = calls[0];
    assert_eq!(id, "mcptoolu_01SAss3KEwASziHZoMR6HcZU");
    assert_eq!(name, "ask_question");
    assert_eq!(input["repoName"], json!("pydantic/pydantic-ai"));
    assert_eq!(provider_executed, Some(true));
    assert_eq!(dynamic, Some(true));
    assert_eq!(
        meta.expect("mcp_tool_use metadata")["anthropic"],
        json!({ "type": "mcp-tool-use", "serverName": "deepwiki" })
    );

    // mcp_tool_result inherits the name and server of the call it answers.
    let results = tool_results(&result.content);
    assert_eq!(results.len(), 1);
    let (rid, rname, payload, is_error, rdynamic, rmeta) = results[0];
    assert_eq!(rid, "mcptoolu_01SAss3KEwASziHZoMR6HcZU");
    assert_eq!(
        rname, "ask_question",
        "the result inherits the mcp_tool_use name"
    );
    assert_eq!(is_error, Some(false), "`is_error: false` must be preserved");
    assert_eq!(rdynamic, Some(true));
    assert_eq!(
        rmeta.expect("mcp_tool_result metadata")["anthropic"],
        json!({ "type": "mcp-tool-use", "serverName": "deepwiki" })
    );
    assert!(
        payload[0]["text"]
            .as_str()
            .expect("mcp payload text")
            .starts_with("Pydantic AI is a Python agent framework designed to simplify"),
        "the MCP answer body must survive verbatim"
    );
}

#[tokio::test]
async fn finding_2_anthropic_redacted_thinking_surfaced_as_reasoning() {
    let c = cassette("anthropic/test_anthropic_model_thinking_part_redacted.json");
    let server = MockServer::start().await;
    mount(&server, &c).await;

    let result = anthropic_at(&server.uri())
        .do_generate(&opts())
        .await
        .expect("do_generate should succeed");

    let r = reasonings(&result.content);
    assert_eq!(r.len(), 1, "the redacted_thinking block must surface");
    assert_eq!(r[0].0, "", "redacted thinking has no plaintext");
    let data = r[0].1.expect("redacted metadata")["anthropic"]["redactedData"]
        .as_str()
        .expect("redactedData");
    assert!(
        data.starts_with("EvgFCkYIBxgCKkBmxKtCnM3xlr1zpw0Ik4FY0bnznKLdj7THnWO4shd9"),
        "the opaque redacted payload must be preserved byte-for-byte"
    );
    assert_eq!(data.len(), 1020);

    // The visible answer is unaffected.
    let t = texts(&result.content);
    assert_eq!(t.len(), 1);
    assert!(t[0].starts_with("I notice that your message appears to contain a command"));
}

// ═══════════════════════════════════════════════════════════════════════════
// Finding 25 — Cohere per-citation metadata
//
// Cassette: cohere/test_tool_choice_matrix[auto-cohere]_1.json
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn finding_25_cohere_citation_metadata_preserved_field_by_field() {
    let c = cassette("cohere/test_tool_choice_matrix[auto-cohere]_1.json");
    let server = MockServer::start().await;
    mount(&server, &c).await;

    let provider = CohereProvider::new(
        CohereConfig::new("test-key").with_base_url(format!("{}/v2", server.uri())),
    );
    let result = provider
        .model("command-r-plus")
        .do_generate(&opts())
        .await
        .expect("do_generate should succeed");

    let s = sources(&result.content);
    assert_eq!(s.len(), 2, "two citations → two sources");

    let expected_tool_source = json!([{
        "id": "get_weather_9gpb31r7h7mj:0",
        "type": "tool",
        "tool_output": { "content": "Sunny, 22C in Paris" },
    }]);

    // Citation 0 — every field, not just "metadata is Some".
    assert_eq!(s[0].1, "document");
    let m0 = &s[0].4.expect("citation 0 metadata")["cohere"];
    assert_eq!(m0["start"], json!(34));
    assert_eq!(m0["end"], json!(39));
    assert_eq!(m0["text"], json!("sunny"));
    assert_eq!(m0["citationType"], json!("TEXT_CONTENT"));
    assert_eq!(
        m0["sources"], expected_tool_source,
        "the citation's source array (including tool_output) must survive intact"
    );

    // Citation 1.
    let m1 = &s[1].4.expect("citation 1 metadata")["cohere"];
    assert_eq!(m1["start"], json!(44));
    assert_eq!(m1["end"], json!(48));
    assert_eq!(m1["text"], json!("22C."));
    assert_eq!(m1["citationType"], json!("TEXT_CONTENT"));
    assert_eq!(m1["sources"], expected_tool_source);

    // The answer text the citations point into.
    assert_eq!(
        texts(&result.content),
        vec!["The weather in Paris is currently sunny and 22C."]
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Finding 13 — Mistral thinking parts dropped on the non-streaming path
//
// Cassette: mistral/test_reasoning_wire_contract[mistral-small-thinking-high].json
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn finding_13_mistral_thinking_parts_become_reasoning_in_generate() {
    let c = cassette("mistral/test_reasoning_wire_contract[mistral-small-thinking-high].json");
    let server = MockServer::start().await;
    mount(&server, &c).await;

    let provider = MistralProvider::new(
        MistralConfig::new("test-key").with_base_url(format!("{}/v1", server.uri())),
    );
    let result = provider
        .model("mistral-small-latest")
        .do_generate(&opts())
        .await
        .expect("do_generate should succeed");

    // Reasoning was previously reachable only via the streaming path.
    let r = reasonings(&result.content);
    assert_eq!(r.len(), 1, "the `thinking` content part must surface");
    assert_eq!(
        r[0].0,
        "The user is asking for the result of 2+2. The instruction is to reply \
         with just the number, so I should respond with \"4\".",
        "the thinking text must survive verbatim"
    );

    // The visible answer is the plain `text` part only.
    assert_eq!(texts(&result.content), vec!["4"]);

    // Ordering: reasoning precedes text (upstream contract).
    assert!(matches!(
        result.content[0],
        GenerateContent::Reasoning { .. }
    ));
    assert!(matches!(result.content[1], GenerateContent::Text { .. }));
}

// ═══════════════════════════════════════════════════════════════════════════
// Finding 4 / 19 — OpenAI Responses: reasoning `encrypted_content` + `itemId`
// Finding 30    — url_citation annotations
// ═══════════════════════════════════════════════════════════════════════════

fn openai_responses_at(uri: &str, model: &str) -> impl LanguageModel {
    OpenAIProvider::new(OpenAIConfig::new("test-key").with_base_url(format!("{uri}/v1")))
        .responses_model(model)
}

#[tokio::test]
async fn finding_4_openai_responses_reasoning_carries_item_id_and_encrypted_content() {
    let c = cassette("openai/test_openai_responses_model_web_search_tool_with_invalid_region.json");
    let server = MockServer::start().await;
    mount(&server, &c).await;

    let result = openai_responses_at(&server.uri(), "gpt-5")
        .do_generate(&opts())
        .await
        .expect("do_generate should succeed");

    let r = reasonings(&result.content);
    assert_eq!(
        r.len(),
        2,
        "the cassette has two `reasoning` output items (both summary-less)"
    );

    let m0 = r[0].1.expect("reasoning 0 metadata");
    assert_eq!(
        m0["openai"]["itemId"],
        json!("rs_0b4f29854724a3120068c4ab0be6a08191b495f9009b885649"),
        "itemId identifies the reasoning item on the follow-up turn"
    );
    let enc = m0["openai"]["reasoningEncryptedContent"]
        .as_str()
        .expect("reasoningEncryptedContent must be a string, not null");
    assert!(
        enc.starts_with("gAAAAABoxKsml4Y3hqqolEa8BSvPr6mIoOyAbWRJz9FeLHoqX03v4b6K"),
        "the encrypted reasoning blob must be preserved verbatim"
    );
    assert_eq!(
        enc.len(),
        2508,
        "truncating the blob would break reasoning continuity across turns"
    );

    // Both reasoning items are distinct; neither is a placeholder.
    let m1 = r[1].1.expect("reasoning 1 metadata");
    assert_ne!(m1["openai"]["itemId"], m0["openai"]["itemId"]);
    assert!(m1["openai"]["reasoningEncryptedContent"].is_string());
}

#[tokio::test]
async fn finding_30_openai_responses_url_citation_annotation_becomes_source() {
    let c = cassette("openai/test_openai_include_raw_annotations_non_streaming.json");
    let server = MockServer::start().await;
    mount(&server, &c).await;

    let result = openai_responses_at(&server.uri(), "gpt-5.2")
        .do_generate(&opts())
        .await
        .expect("do_generate should succeed");

    let s = sources(&result.content);
    assert_eq!(s.len(), 1, "one url_citation annotation → one Source");
    assert_eq!(s[0].1, "url");
    assert_eq!(
        s[0].2,
        Some("https://www.britannica.com/place/Mount-Columbia?utm_source=openai"),
        "the citation URL (query string included) must survive"
    );
    assert_eq!(
        s[0].3,
        Some("Mount Columbia | mountain, Alberta, Canada | Britannica")
    );

    // The cited text itself is still present.
    let t = texts(&result.content);
    assert_eq!(t.len(), 1);
    assert!(t[0].starts_with("The tallest mountain in Alberta is **Mount Columbia**"));
    // The message item id rides on the text part (finding 19).
    let item_id = result
        .content
        .iter()
        .find_map(|c| match c {
            GenerateContent::Text {
                provider_metadata: Some(m),
                ..
            } => m["openai"]["itemId"].as_str(),
            _ => None,
        })
        .expect("text part must carry providerMetadata.openai.itemId");
    assert_eq!(
        item_id,
        "msg_057daac88567bde400696c45fc489c81909c927a966ee61535"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Finding 10 / 27 — Bedrock reasoning signature + metadata passthrough
//
// The recorded Bedrock cassettes are non-streaming JSON; the streaming frames
// below carry the *same recorded payloads* (signature, performanceConfig,
// serviceTier, guardrail trace) through the AWS event-stream codec.
// ═══════════════════════════════════════════════════════════════════════════

const BEDROCK_MODEL: &str = "us.anthropic.claude-sonnet-4-20250514-v1:0";

fn bedrock_at(uri: &str, model: &str) -> BedrockModel {
    BedrockModel::new(
        model.to_string(),
        BedrockConfig {
            base_url: uri.to_string(),
            auth: BedrockAuth::BearerToken("test-token".to_string()),
            retry_config: aimux_provider_utils::RetryConfig::default(),
            api_key_source: None,
        },
    )
}

/// The real signature recorded in
/// `bedrock/test_bedrock_model_thinking_part_anthropic.json`.
fn bedrock_recorded_reasoning() -> (String, String) {
    let body = cassette_json("bedrock/test_bedrock_model_thinking_part_anthropic.json");
    let rt = &body["output"]["message"]["content"][0]["reasoningContent"]["reasoningText"];
    (
        rt["signature"].as_str().expect("signature").to_string(),
        rt["text"].as_str().expect("text").to_string(),
    )
}

#[tokio::test]
async fn finding_10_bedrock_generate_reasoning_signature_in_provider_metadata() {
    let c = cassette("bedrock/test_bedrock_model_thinking_part_anthropic.json");
    let server = MockServer::start().await;
    mount(&server, &c).await;

    let result = bedrock_at(&server.uri(), BEDROCK_MODEL)
        .do_generate(&opts())
        .await
        .expect("do_generate should succeed");

    let r = reasonings(&result.content);
    assert_eq!(r.len(), 1);
    assert!(
        r[0].0
            .starts_with("This is a straightforward question about crossing the street safely."),
        "the reasoning text must survive"
    );
    let m = r[0].1.expect("reasoning metadata");
    let sig = m["amazonBedrock"]["signature"]
        .as_str()
        .expect("amazonBedrock.signature");
    assert!(
        sig.starts_with("Eu4CCkgIBxABGAIqQCc4A+JUj/DJn5X49FRHzzGDqrWoZCEii+cINeYRllo7"),
        "the recorded signature must be echoed back, got {:?}",
        &sig[..sig.len().min(24)]
    );
    assert_eq!(sig.len(), 496, "the signature must not be truncated");
    assert_eq!(
        m["bedrock"]["signature"], m["amazonBedrock"]["signature"],
        "the signature is mirrored under both provider keys"
    );

    // The visible answer is separate from the reasoning.
    let t = texts(&result.content);
    assert_eq!(t.len(), 1);
    assert!(t[0].starts_with("Here are the basic steps for crossing the street safely:"));
}

fn encode_events(events: &[(&str, &str, Value)]) -> Vec<u8> {
    let mut buf = Vec::new();
    for (mt, et, payload) in events {
        buf.extend_from_slice(&event_stream::encode_message(mt, et, &payload.to_string()));
    }
    buf
}

async fn bedrock_stream_parts(model: &str, events: &[(&str, &str, Value)]) -> Vec<StreamPart> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path_matcher(format!("/model/{model}/converse-stream")))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/vnd.amazon.eventstream")
                .set_body_bytes(encode_events(events)),
        )
        .mount(&server)
        .await;

    collect(
        bedrock_at(&server.uri(), model)
            .do_stream(&opts())
            .await
            .expect("do_stream should succeed"),
    )
    .await
}

#[tokio::test]
async fn finding_10_bedrock_stream_reasoning_signature_emitted_as_metadata_delta() {
    let (signature, text) = bedrock_recorded_reasoning();

    let parts = bedrock_stream_parts(
        BEDROCK_MODEL,
        &[
            ("event", "messageStart", json!({ "role": "assistant" })),
            (
                "event",
                "contentBlockDelta",
                json!({
                    "contentBlockIndex": 0,
                    "delta": { "reasoningContent": { "text": text } }
                }),
            ),
            (
                "event",
                "contentBlockDelta",
                json!({
                    "contentBlockIndex": 0,
                    "delta": { "reasoningContent": { "signature": signature } }
                }),
            ),
            ("event", "messageStop", json!({ "stopReason": "end_turn" })),
        ],
    )
    .await;

    // The text delta.
    let text_deltas: Vec<&str> = parts
        .iter()
        .filter_map(|p| match p {
            StreamPart::ReasoningDelta { delta, .. } if !delta.is_empty() => Some(delta.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text_deltas.len(), 1);
    assert!(text_deltas[0].starts_with("This is a straightforward question"));

    // The signature arrives on a trailing text-less delta and is attached to
    // the concluding ReasoningEnd (dual-key, same shape as the non-streaming
    // path) — it used to be dropped entirely.
    let end_meta = parts
        .iter()
        .find_map(|p| match p {
            StreamPart::ReasoningEnd {
                provider_metadata: Some(m),
                ..
            } => Some(m),
            _ => None,
        })
        .expect("the ReasoningEnd must carry the signature");
    assert_eq!(end_meta["amazonBedrock"]["signature"], json!(signature));
    assert_eq!(end_meta["bedrock"]["signature"], json!(signature));
}

#[tokio::test]
async fn finding_27_bedrock_stream_performance_config_and_service_tier_reach_finish() {
    // Real recorded values.
    let perf =
        cassette_json("bedrock/test_bedrock_model_performance_config.json")["performanceConfig"]
            .clone();
    let tier = cassette_json("bedrock/test_bedrock_model_service_tier.json")["serviceTier"].clone();
    assert_eq!(perf, json!({ "latency": "optimized" }));
    assert_eq!(tier, json!({ "type": "flex" }));

    let parts = bedrock_stream_parts(
        BEDROCK_MODEL,
        &[
            ("event", "messageStart", json!({ "role": "assistant" })),
            (
                "event",
                "contentBlockDelta",
                json!({ "contentBlockIndex": 0, "delta": { "text": "Paris." } }),
            ),
            ("event", "messageStop", json!({ "stopReason": "end_turn" })),
            (
                "event",
                "metadata",
                json!({
                    "usage": { "inputTokens": 13, "outputTokens": 67, "totalTokens": 80 },
                    "performanceConfig": perf,
                    "serviceTier": tier,
                }),
            ),
        ],
    )
    .await;

    let meta = parts
        .iter()
        .find_map(|p| match p {
            StreamPart::Finish {
                provider_metadata, ..
            } => provider_metadata.as_ref(),
            _ => None,
        })
        .expect("Finish must carry provider_metadata");
    assert_eq!(
        meta["amazonBedrock"]["performanceConfig"],
        json!({ "latency": "optimized" })
    );
    assert_eq!(
        meta["amazonBedrock"]["serviceTier"],
        json!({ "type": "flex" })
    );
    assert_eq!(
        meta["bedrock"], meta["amazonBedrock"],
        "both provider keys carry the same payload"
    );
}

#[tokio::test]
async fn finding_27_bedrock_stream_guardrail_trace_reaches_finish() {
    let trace = cassette_json("bedrock/test_bedrock_model_guardrail_config.json")["trace"].clone();
    // Guard against the cassette silently losing its guardrail block.
    assert_eq!(trace["guardrail"]["actionReason"], json!("No action."));

    let parts = bedrock_stream_parts(
        "us.amazon.nova-micro-v1:0",
        &[
            ("event", "messageStart", json!({ "role": "assistant" })),
            (
                "event",
                "contentBlockDelta",
                json!({ "contentBlockIndex": 0, "delta": { "text": "Paris." } }),
            ),
            ("event", "messageStop", json!({ "stopReason": "end_turn" })),
            (
                "event",
                "metadata",
                json!({
                    "usage": { "inputTokens": 13, "outputTokens": 67, "totalTokens": 80 },
                    "trace": trace,
                }),
            ),
        ],
    )
    .await;

    let meta = parts
        .iter()
        .find_map(|p| match p {
            StreamPart::Finish {
                provider_metadata, ..
            } => provider_metadata.as_ref(),
            _ => None,
        })
        .expect("Finish must carry provider_metadata");

    let guardrail = &meta["amazonBedrock"]["trace"]["guardrail"];
    assert_eq!(guardrail["actionReason"], json!("No action."));
    let assessment = &guardrail["inputAssessment"]["xbgw7g293v7o"];
    assert_eq!(
        assessment["appliedGuardrailDetails"]["guardrailId"],
        json!("xbgw7g293v7o")
    );
    assert_eq!(
        assessment["appliedGuardrailDetails"]["guardrailArn"],
        json!("arn:aws:bedrock:us-east-1:353014496775:guardrail/xbgw7g293v7o")
    );
    assert_eq!(
        assessment["invocationMetrics"]["guardrailProcessingLatency"],
        json!(397),
        "nested metrics must not be flattened away"
    );
    assert_eq!(meta["bedrock"]["trace"], meta["amazonBedrock"]["trace"]);
}

#[tokio::test]
async fn finding_26_bedrock_stream_stop_sequence_reaches_finish() {
    let parts = bedrock_stream_parts(
        BEDROCK_MODEL,
        &[
            ("event", "messageStart", json!({ "role": "assistant" })),
            (
                "event",
                "contentBlockDelta",
                json!({ "contentBlockIndex": 0, "delta": { "text": "Hello, World!" } }),
            ),
            (
                "event",
                "messageStop",
                json!({
                    "stopReason": "stop_sequence",
                    "additionalModelResponseFields": { "delta": { "stop_sequence": "STOP" } }
                }),
            ),
            (
                "event",
                "metadata",
                json!({ "usage": { "inputTokens": 4, "outputTokens": 30, "totalTokens": 34 } }),
            ),
        ],
    )
    .await;

    let meta = parts
        .iter()
        .find_map(|p| match p {
            StreamPart::Finish {
                provider_metadata, ..
            } => provider_metadata.as_ref(),
            _ => None,
        })
        .expect("Finish must carry provider_metadata");
    assert_eq!(meta["amazonBedrock"]["stopSequence"], json!("STOP"));
    assert_eq!(meta["bedrock"]["stopSequence"], json!("STOP"));
}
