//! Dev fixture generator: writes a `Recording` (RFC-0023) as NDJSON for
//! testing the console's mock mode / import without real API keys.
//!
//! Usage: `cargo run -p aimux-web --example gen_fixture [out.jsonl]`

use aimux_core::content::ContentPart;
use aimux_core::language_model_message::LanguageModelPromptMessage;
use aimux_core::message::Role;
use aimux_core::options::CallOptions;
use aimux_core::recording::{
    HttpExchange, HttpRecord, InputRecord, OutcomeRecord, OutcomeStatus, ProviderRecord, Recording,
    ResponseRecord, TimingRecord,
};

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "fixture.jsonl".to_string());

    let prompt = vec![
        LanguageModelPromptMessage {
            role: Role::System,
            content: vec![ContentPart::text(
                "You are a helpful assistant. Always use the calculator tool.",
            )],
            provider_options: None,
        },
        LanguageModelPromptMessage {
            role: Role::User,
            content: vec![ContentPart::text("what is 1 + 1?")],
            provider_options: None,
        },
    ];

    let options = CallOptions {
        prompt: prompt.clone(),
        max_output_tokens: Some(64),
        temperature: Some(0.0),
        ..Default::default()
    };

    let rec = Recording {
        schema: 1,
        call_id: "call-fixture-1".into(),
        recorded_at: "2026-08-14T00:00:00.000Z".into(),
        input: InputRecord {
            prompt: prompt.clone(),
            options: serde_json::to_value(&options).unwrap(),
        },
        provider: ProviderRecord {
            provider: "openai".into(),
            model_id: "gpt-4o-mini".into(),
            base_url: None,
            api_key_source: "none".into(),
            profile: None,
            provider_options: None,
        },
        exchanges: vec![HttpExchange {
            attempt: 0,
            request: HttpRecord {
                method: "POST".into(),
                url: "https://api.openai.com/v1/chat/completions".into(),
                headers: vec![],
                body: Some("{\"model\":\"gpt-4o-mini\",\"messages\":[...]}".into()),
            },
            response: Some(ResponseRecord {
                status: 200,
                headers: vec![],
                body: Some("{\"choices\":[{\"message\":{\"content\":\"1 + 1 = 2\"}}]}".into()),
                stream_chunks: None,
                ttfb_ms: None,
            }),
            timing: TimingRecord {
                latency_ms: 320,
                ttfb_ms: None,
            },
            error: None,
            finalized: true,
        }],
        outcome: OutcomeRecord {
            status: OutcomeStatus::Success,
            finish_reason: Some("stop".into()),
            error: None,
            usage: Some(serde_json::json!({
                "input_tokens": { "total": 12, "cache_read": 0 },
                "output_tokens": { "total": 5 }
            })),
        },
        complete: true,
        transport_closed: true,
        session_id: Some("sess-fixture".into()),
        step: Some(0),
    };

    let jsonl = serde_json::to_string(&rec).unwrap();
    std::fs::write(&out, format!("{jsonl}\n")).unwrap();
    println!("wrote {out}");
}
