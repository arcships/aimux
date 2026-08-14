//! Demo fixture generator — several `Recording`s (RFC-0023) with varied
//! providers / sessions / statuses for the console preview screenshots.
//!
//! Usage: `cargo run -p aimux-web --example gen_demo_fixtures [out.jsonl]`
//!
//! The first two recordings are crafted so the console's mock mode matches:
//! - `call-demo-pg`  prompt = [user: "what is 1 + 1?"]            (Playground)
//! - `call-demo-agent` prompt = [system: <agent system>, user: "what is 17 * 19?"]  (Agent)
//!
//! The rest only enrich the Traces / Sessions listing.

use aimux_core::content::ContentPart;
use aimux_core::language_model_message::LanguageModelPromptMessage;
use aimux_core::message::Role;
use aimux_core::options::CallOptions;
use aimux_core::recording::{
    HttpExchange, HttpRecord, InputRecord, OutcomeRecord, OutcomeStatus, ProviderRecord, Recording,
    ResponseRecord, TimingRecord,
};

const AGENT_SYSTEM: &str =
    "You are a helpful assistant. Always use the calculator tool for arithmetic, then answer.";

fn msg(role: Role, text: &str) -> LanguageModelPromptMessage {
    LanguageModelPromptMessage {
        role,
        content: vec![ContentPart::text(text)],
        provider_options: None,
    }
}

#[allow(clippy::too_many_arguments)]
fn recording(
    call_id: &str,
    recorded_at: &str,
    provider: &str,
    model_id: &str,
    prompt: Vec<LanguageModelPromptMessage>,
    session: Option<&str>,
    step: Option<u32>,
    status: OutcomeStatus,
    finish: Option<&str>,
    body: &str,
    latency_ms: u64,
    usage: serde_json::Value,
) -> Recording {
    let options = CallOptions {
        prompt: prompt.clone(),
        max_output_tokens: Some(64),
        temperature: Some(0.0),
        ..Default::default()
    };
    Recording {
        schema: 1,
        call_id: call_id.into(),
        recorded_at: recorded_at.into(),
        input: InputRecord {
            prompt: prompt.clone(),
            options: serde_json::to_value(&options).unwrap(),
        },
        provider: ProviderRecord {
            provider: provider.into(),
            model_id: model_id.into(),
            base_url: None,
            api_key_source: "none".into(),
            profile: None,
            provider_options: None,
        },
        exchanges: vec![HttpExchange {
            attempt: 0,
            request: HttpRecord {
                method: "POST".into(),
                url: "https://api.example.com/v1/chat/completions".to_string(),
                headers: vec![],
                body: Some("{\"model\":\"demo\",\"messages\":[...]}".into()),
            },
            response: Some(ResponseRecord {
                status: 200,
                headers: vec![],
                body: Some(body.into()),
                stream_chunks: None,
                ttfb_ms: None,
            }),
            timing: TimingRecord {
                latency_ms,
                ttfb_ms: None,
            },
            error: None,
            finalized: true,
        }],
        outcome: OutcomeRecord {
            status,
            finish_reason: finish.map(str::to_string),
            error: if status == OutcomeStatus::Error {
                Some("upstream 500: provider returned an error".into())
            } else {
                None
            },
            usage: Some(usage),
        },
        complete: true,
        transport_closed: true,
        session_id: session.map(str::to_string),
        step,
    }
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "demo.jsonl".to_string());

    // OpenAI chat.completions SSE bodies so mock streaming emits TextDelta.
    let sse_pg = r#"data: {"id":"chatcmpl-demo-pg","model":"gpt-4o-mini"}

data: {"choices":[{"index":0,"delta":{"role":"assistant","content":"1 + 1 = 2"}}]}

data: {"choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":12,"completion_tokens":5}}

data: [DONE]"#;

    let sse_agent = r#"data: {"id":"chatcmpl-demo-agent","model":"gpt-4o"}

data: {"choices":[{"index":0,"delta":{"role":"assistant","content":"17 × 19 = 323"}}]}

data: {"choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":64,"completion_tokens":9}}

data: [DONE]"#;

    let recs = vec![
        // Playground mock target: [user] only, same model as the Agent target
        // so one `openai/gpt-4o` mock model serves both pages.
        recording(
            "call-demo-pg",
            "2026-08-14T09:31:10.000Z",
            "openai",
            "gpt-4o",
            vec![msg(Role::User, "what is 1 + 1?")],
            Some("sess-playground"),
            Some(0),
            OutcomeStatus::Success,
            Some("stop"),
            sse_pg,
            320,
            serde_json::json!({
                "input_tokens": { "total": 12, "cache_read": 0 },
                "output_tokens": { "total": 5 }
            }),
        ),
        // Agent mock target: [system (agent), user] — matches the Agent page's default def.
        recording(
            "call-demo-agent",
            "2026-08-14T09:32:02.000Z",
            "openai",
            "gpt-4o",
            vec![
                msg(Role::System, AGENT_SYSTEM),
                msg(Role::User, "what is 17 * 19?"),
            ],
            Some("sess-agent"),
            Some(0),
            OutcomeStatus::Success,
            Some("stop"),
            sse_agent,
            412,
            serde_json::json!({
                "input_tokens": { "total": 64, "cache_read": 0 },
                "output_tokens": { "total": 9 }
            }),
        ),
        // Enrichment: a multi-step session (steps 1 & 2) on deepseek.
        recording(
            "call-demo-ds-1",
            "2026-08-14T09:28:40.000Z",
            "deepseek",
            "deepseek-chat",
            vec![
                msg(Role::System, "You are a careful technical analyst."),
                msg(Role::User, "Explain how prefix caching works."),
            ],
            Some("sess-deepseek"),
            Some(0),
            OutcomeStatus::Success,
            Some("stop"),
            "{\"choices\":[{\"message\":{\"content\":\"Prefix caching reuses the KV cache...\"}}]}",
            810,
            serde_json::json!({
                "input_tokens": { "total": 2300, "cache_read": 0 },
                "output_tokens": { "total": 210 }
            }),
        ),
        recording(
            "call-demo-ds-2",
            "2026-08-14T09:29:01.000Z",
            "deepseek",
            "deepseek-chat",
            vec![
                msg(Role::System, "You are a careful technical analyst."),
                msg(Role::User, "Explain how prefix caching works."),
                msg(Role::Assistant, "Prefix caching reuses the KV cache..."),
                msg(Role::User, "What is the hit rate formula?"),
            ],
            Some("sess-deepseek"),
            Some(1),
            OutcomeStatus::Success,
            Some("stop"),
            "{\"choices\":[{\"message\":{\"content\":\"cache_read / input_total\"}}]}",
            796,
            serde_json::json!({
                "input_tokens": { "total": 2650, "cache_read": 2300 },
                "output_tokens": { "total": 18 }
            }),
        ),
        // Enrichment: an errored call (shows the error badge).
        recording(
            "call-demo-err",
            "2026-08-14T09:25:15.000Z",
            "groq",
            "llama-3.3-70b-versatile",
            vec![msg(Role::User, "hi")],
            Some("sess-err"),
            Some(0),
            OutcomeStatus::Error,
            None,
            "{}",
            1200,
            serde_json::json!({ "input_tokens": null, "output_tokens": null }),
        ),
        // Enrichment: an older successful call on a different model.
        recording(
            "call-demo-anth",
            "2026-08-14T09:10:05.000Z",
            "anthropic",
            "claude-3-5-sonnet-latest",
            vec![msg(Role::User, "Write a haiku about Rust.")],
            None,
            None,
            OutcomeStatus::Success,
            Some("stop"),
            "{\"choices\":[{\"message\":{\"content\":\"Ownership, borrowing — memory safe by design...\"}}]}",
            540,
            serde_json::json!({
                "input_tokens": { "total": 42, "cache_read": 0 },
                "output_tokens": { "total": 24 }
            }),
        ),
    ];

    let mut out_str = String::new();
    for r in &recs {
        out_str.push_str(&serde_json::to_string(r).unwrap());
        out_str.push('\n');
    }
    std::fs::write(&out, out_str).unwrap();
    println!("wrote {} recordings to {out}", recs.len());
}
