//! Host-side tool-call repair (RFC-0035).
//!
//! The closure path (`ToolCallRepair`) and the data path
//! (`apply_tool_call_repair`) must be indistinguishable: the bindings rely on
//! the second producing exactly what Rust callers get from the first.

use aimux_core::error::AiMuxError;
use aimux_core::generate::{
    GenerateTextOptions, GenerateTextResult, generate_text_result_to_chat_completion,
};
use aimux_core::message::ModelPrompt;
use aimux_core::parse_tool_call::{
    RawToolCall, ToolCallRepair, ToolCallRepairReply, apply_tool_call_repair,
    apply_tool_call_repair_to_result, parse_tool_call, tool_call_repair_context,
    tool_call_repair_inputs,
};
use aimux_core::tool::{FunctionTool, Tool, ToolCall};
use serde_json::{Value, json};

fn weather_tool() -> Tool {
    FunctionTool::new(
        "weather",
        json!({
            "type": "object",
            "properties": { "city": { "type": "string" } },
            "required": ["city"],
        }),
    )
    .into()
}

fn raw(name: &str, input: &str) -> RawToolCall {
    RawToolCall {
        tool_call_id: "call-1".into(),
        tool_name: name.into(),
        input: input.into(),
        provider_executed: None,
        dynamic: None,
        thought_signature: None,
        provider_metadata: None,
    }
}

/// The invalid `ToolCall` a host reads off `GenerateTextResult.tool_calls`.
async fn invalid_call(input: &str) -> ToolCall {
    let call = parse_tool_call(
        raw("weather", input),
        Some(&[weather_tool()]),
        None,
        &[],
        None,
    )
    .await;
    assert_eq!(call.invalid, Some(true));
    call
}

/// Run the closure path with the callback result the given reply models.
async fn via_closure(
    input: &str,
    outcome: Result<Option<RawToolCall>, AiMuxError>,
) -> aimux_core::tool::ToolCall {
    let repair = ToolCallRepair::new(move |_| {
        let outcome = outcome.clone();
        async move { outcome }
    });
    parse_tool_call(
        raw("weather", input),
        Some(&[weather_tool()]),
        Some(&repair),
        &[],
        None,
    )
    .await
}

const BAD: &str = r#"{"town":"Singapore"}"#;

// ── closure / host-side equivalence ─────────────────────────────────────────

#[tokio::test]
async fn both_paths_agree_on_every_branch() {
    let repaired = raw("weather", r#"{"city":"Singapore"}"#);
    let still_bad = raw("weather", r#"{"city":42}"#);
    let cases: Vec<(Result<Option<RawToolCall>, AiMuxError>, ToolCallRepairReply)> = vec![
        (
            Ok(Some(repaired.clone())),
            ToolCallRepairReply::Repaired {
                tool_call: repaired,
            },
        ),
        (
            Ok(Some(still_bad.clone())),
            ToolCallRepairReply::Repaired {
                tool_call: still_bad,
            },
        ),
        (Ok(None), ToolCallRepairReply::Unchanged),
        (
            Err(AiMuxError::Other("boom".into())),
            ToolCallRepairReply::Failed {
                message: "boom".into(),
            },
        ),
    ];

    for (outcome, reply) in cases {
        let from_closure = via_closure(BAD, outcome).await;
        let from_data =
            apply_tool_call_repair(&invalid_call(BAD).await, Some(&[weather_tool()]), reply)
                .unwrap();
        assert_eq!(
            serde_json::to_value(&from_closure).unwrap(),
            serde_json::to_value(&from_data).unwrap(),
        );
    }
}

// ── repair context ──────────────────────────────────────────────────────────

#[tokio::test]
async fn context_of_an_unknown_tool_falls_back_to_an_empty_schema() {
    let unknown = parse_tool_call(
        raw("wether", r#"{"city":"Singapore"}"#),
        Some(&[weather_tool()]),
        None,
        &[],
        None,
    )
    .await;
    let context = tool_call_repair_context(&unknown, Some(&[weather_tool()]), &[], None)
        .unwrap()
        .expect("a tool set was supplied");
    assert_eq!(context["input_schema"]["properties"], json!({}));
    assert!(context["error"]["NoSuchTool"].is_object());
}

// ── result patching ─────────────────────────────────────────────────────────

fn result_with(call: &ToolCall) -> Value {
    json!({
        "tool_calls": [serde_json::to_value(call).unwrap()],
        "response_messages": [{
            "role": "assistant",
            "content": [
                { "type": "text", "text": "checking" },
                {
                    "type": "tool_call",
                    "tool_call_id": call.tool_call_id,
                    "tool_name": call.tool_name,
                    "input": call.input,
                },
            ],
        }],
    })
}

#[tokio::test]
async fn patching_a_generate_object_result_reaches_into_raw() {
    let result = json!({ "object": {"ok": true}, "raw": result_with(&invalid_call(BAD).await) });
    let patched = apply_tool_call_repair_to_result(
        &result,
        Some(&[weather_tool()]),
        "call-1",
        ToolCallRepairReply::Repaired {
            tool_call: raw("weather", r#"{"city":"Singapore"}"#),
        },
    )
    .unwrap();
    assert_eq!(patched["object"], json!({"ok": true}));
    assert_eq!(
        patched["raw"]["tool_calls"][0]["input"],
        json!({"city":"Singapore"})
    );
}

#[tokio::test]
async fn patching_an_aggregated_stream_result_round_trips_through_its_type() {
    // Bindings patch the output of `consume_stream_text` with the same
    // function; the patched document must still decode as that type.
    let call = invalid_call(BAD).await;
    let mut aggregated: aimux_core::result::StreamTextResultAggregated = serde_json::from_value(
        json!({ "text": "", "finish_reason": { "unified": "tool-calls", "raw": null } }),
    )
    .unwrap();
    let shape = result_with(&call);
    aggregated.tool_calls = vec![call];
    aggregated.response_messages =
        serde_json::from_value(shape["response_messages"].clone()).unwrap();

    let patched = apply_tool_call_repair_to_result(
        &serde_json::to_value(&aggregated).unwrap(),
        Some(&[weather_tool()]),
        "call-1",
        ToolCallRepairReply::Repaired {
            tool_call: raw("weather", r#"{"city":"Singapore"}"#),
        },
    )
    .unwrap();
    let decoded: aimux_core::result::StreamTextResultAggregated =
        serde_json::from_value(patched).unwrap();
    assert_eq!(decoded.tool_calls[0].invalid, None);
    assert_eq!(decoded.tool_calls[0].input, json!({"city":"Singapore"}));
}

#[tokio::test]
async fn context_keeps_the_provider_argument_text_verbatim() {
    // A bare JSON string literal parses fine, so `input` holds `Tokyo`
    // without quotes; the host must still see the provider's `"Tokyo"`, or a
    // repair that only renames the tool re-validates the wrong text.
    let call = invalid_call(r#""Tokyo""#).await;
    assert_eq!(call.input, json!("Tokyo"));
    let context = tool_call_repair_context(&call, Some(&[weather_tool()]), &[], None)
        .unwrap()
        .expect("a tool set was supplied");
    assert_eq!(context["tool_call"]["input"], json!(r#""Tokyo""#));

    let spaced = invalid_call(r#"{ "town" : "Tokyo" }"#).await;
    let context = tool_call_repair_context(&spaced, Some(&[weather_tool()]), &[], None)
        .unwrap()
        .unwrap();
    assert_eq!(
        context["tool_call"]["input"],
        json!(r#"{ "town" : "Tokyo" }"#)
    );
}

#[tokio::test]
async fn patching_refuses_a_duplicated_tool_call_id() {
    let call = invalid_call(BAD).await;
    let mut result = result_with(&call);
    let dup = result["tool_calls"][0].clone();
    result["tool_calls"].as_array_mut().unwrap().push(dup);
    let error = apply_tool_call_repair_to_result(
        &result,
        Some(&[weather_tool()]),
        "call-1",
        ToolCallRepairReply::Unchanged,
    )
    .unwrap_err();
    assert!(
        matches!(error, AiMuxError::InvalidArgument(ref m) if m.contains("not unique")),
        "{error:?}"
    );
}

#[tokio::test]
async fn patching_refuses_a_replacement_id_that_already_exists() {
    let call = invalid_call(BAD).await;
    let mut result = result_with(&call);
    let mut other = result["tool_calls"][0].clone();
    other["tool_call_id"] = json!("call-2");
    result["tool_calls"].as_array_mut().unwrap().push(other);

    let error = apply_tool_call_repair_to_result(
        &result,
        Some(&[weather_tool()]),
        "call-1",
        ToolCallRepairReply::Repaired {
            tool_call: RawToolCall {
                tool_call_id: "call-2".into(),
                ..raw("weather", r#"{"city":"Singapore"}"#)
            },
        },
    )
    .unwrap_err();

    assert!(
        matches!(error, AiMuxError::InvalidArgument(ref m) if m.contains("already exists")),
        "{error:?}"
    );
}

#[tokio::test]
async fn patching_a_primitive_input_is_not_replayed_into_the_transcript() {
    // A still-invalid call whose input is a bare string: `tool_calls` keeps it,
    // the replay transcript must not.
    let result = result_with(&invalid_call("not json at all").await);
    let patched = apply_tool_call_repair_to_result(
        &result,
        Some(&[weather_tool()]),
        "call-1",
        ToolCallRepairReply::Unchanged,
    )
    .unwrap();
    assert_eq!(patched["tool_calls"][0]["input"], json!("not json at all"));
    assert_eq!(
        patched["response_messages"][0]["content"][1]["input"],
        json!({})
    );
}

#[tokio::test]
async fn a_patched_result_converts_to_a_chat_completion_that_reflects_the_repair() {
    // The OpenAI non-streaming path on a host: repair the native result, then
    // convert it. The provider's raw text in `raw.content` stays malformed;
    // the completion must take its arguments from the patched `tool_calls`.
    let finish = json!({ "unified": "tool-calls", "raw": null });
    let usage = json!({ "input_tokens": { "total": 1 }, "output_tokens": { "total": 1 } });
    let mut result = result_with(&invalid_call(BAD).await);
    let object = result.as_object_mut().unwrap();
    object.insert("text".into(), json!(""));
    object.insert("finish_reason".into(), finish.clone());
    object.insert("usage".into(), usage.clone());
    object.insert("warnings".into(), json!([]));
    object.insert(
        "raw".into(),
        json!({
            "content": [{ "ToolCall": {
                "tool_call_id": "call-1", "tool_name": "weather", "input": BAD,
            } }],
            "finish_reason": finish,
            "usage": usage,
            "warnings": [],
            "provider_metadata": null,
            "response": { "id": null, "timestamp": null, "model_id": null },
            "request_body": null,
            "response_headers": null,
        }),
    );

    let patched = apply_tool_call_repair_to_result(
        &result,
        Some(&[weather_tool()]),
        "call-1",
        ToolCallRepairReply::Repaired {
            tool_call: raw("weather", r#"{"city":"Singapore"}"#),
        },
    )
    .unwrap();
    let decoded: GenerateTextResult = serde_json::from_value(patched).unwrap();
    let completion = generate_text_result_to_chat_completion(&decoded, "fallback-model");

    assert_eq!(completion.model, "fallback-model");
    let calls = completion.choices[0].message.tool_calls.as_ref().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id, "call-1");
    assert_eq!(
        serde_json::from_str::<Value>(&calls[0].function.arguments).unwrap(),
        json!({"city":"Singapore"})
    );
}

// ── wire shapes ─────────────────────────────────────────────────────────────

#[test]
fn reply_wire_shapes_round_trip_and_reject_unknown_fields() {
    let repaired: ToolCallRepairReply = serde_json::from_str(
        r#"{"type":"repaired","tool_call":{"tool_call_id":"c1","tool_name":"weather","input":"{}"}}"#,
    )
    .unwrap();
    assert!(matches!(repaired, ToolCallRepairReply::Repaired { .. }));
    assert!(serde_json::from_str::<ToolCallRepairReply>(r#"{"type":"unchanged"}"#).is_ok());
    assert!(serde_json::from_str::<ToolCallRepairReply>(r#"{"type":"failed"}"#).is_err());
    // serde's `deny_unknown_fields` reaches the struct variants of an
    // internally tagged enum, but not its unit variants.
    assert!(
        serde_json::from_str::<ToolCallRepairReply>(r#"{"type":"failed","message":"m","x":1}"#)
            .is_err()
    );
}

// ── shared contract fixture ─────────────────────────────────────────────────

/// Run `contract-tests/fixtures/tool-call-repair.json` against the Rust
/// implementation, so the file every binding asserts against cannot drift.
#[test]
fn contract_fixture_matches_the_implementation() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../contract-tests/fixtures/tool-call-repair.json");
    let fixture: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();

    for case in fixture["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let input = &case["input"];
        // Every case carries the prompt and options the call was generated
        // with, exactly as a host would hand them back.
        let options: GenerateTextOptions = match &input["opts"] {
            Value::Null => GenerateTextOptions::default(),
            opts => serde_json::from_value(opts.clone()).unwrap(),
        };
        let prompt: ModelPrompt = match &input["prompt"] {
            Value::Object(object) if object.len() == 1 && object.contains_key("prompt") => {
                serde_json::from_value(object["prompt"].clone()).unwrap()
            }
            Value::Null => ModelPrompt::Text(String::new()),
            prompt => serde_json::from_value(prompt.clone()).unwrap(),
        };
        let (messages, instructions, tools) = tool_call_repair_inputs(prompt, &options);
        let reply = || serde_json::from_value(input["reply"].clone()).unwrap();
        let tool_call = || serde_json::from_value::<ToolCall>(input["tool_call"].clone()).unwrap();

        let actual = match case["function"].as_str().unwrap() {
            "tool_call_repair_context" => {
                tool_call_repair_context(&tool_call(), tools, &messages, instructions)
                    .map(|context| serde_json::to_value(context).unwrap())
            }
            "apply_tool_call_repair" => apply_tool_call_repair(&tool_call(), tools, reply())
                .map(|call| serde_json::to_value(call).unwrap()),
            "apply_tool_call_repair_to_result" => apply_tool_call_repair_to_result(
                &input["result"],
                tools,
                input["tool_call_id"].as_str().unwrap(),
                reply(),
            ),
            other => panic!("{name}: unknown function {other}"),
        };

        match case.get("expected_error").and_then(Value::as_str) {
            Some(variant) => {
                let error = actual.unwrap_err();
                let key = serde_json::to_value(&error).unwrap();
                assert!(
                    key.get(variant).is_some(),
                    "{name}: expected {variant}, got {error}"
                );
            }
            None => assert_eq!(actual.unwrap(), case["expected"], "{name}"),
        }
    }
}
