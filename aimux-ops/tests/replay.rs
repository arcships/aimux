//! Replay through the real dispatcher, registry, manifest and core replay paths.
use std::sync::Arc;
use std::time::SystemTime;

use aimux_core::custom_provider::{CustomProviderOptions, custom_provider};
use aimux_core::language_model::LanguageModel;
use aimux_core::options::CallOptions;
use aimux_core::recording::{
    HttpExchange, HttpRecord, InputRecord, OutcomeStatus, ProviderRecord, Recording,
    RecordingOperation, ResponseRecord, TimingRecord,
};
use aimux_core::replay::{MockDecisionReplayModel, MockReplayModel};
use aimux_ops::dispatch::{self, Ctx};
use aimux_ops::envelope::OpsError;
use aimux_ops::handle::{Entry, HandleId, HandleRegistry, Owner};
use aimux_ops::op::{Cancellation, Scope};
use serde_json::{Value, json};

struct Harness(Owner);
impl Harness {
    fn new() -> Self {
        Self(Owner::new_node().unwrap())
    }
    fn ctx(&self) -> Ctx {
        Ctx {
            owner: self.0,
            abort: None,
            now: SystemTime::now,
        }
    }
    fn insert(&self, entry: Entry) -> HandleId {
        HandleRegistry::global().insert(self.0, entry).unwrap()
    }
    async fn call(&self, id: HandleId, op: &str, params: Value) -> Result<Value, OpsError> {
        dispatch::call(self.ctx(), id, op, params).await
    }
    async fn create(&self, recordings: &[Recording]) -> HandleId {
        let jsonl = recordings
            .iter()
            .map(|r| serde_json::to_string(r).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        let result = self
            .call(
                HandleId(0),
                "replay.create",
                json!({"recordingsJsonl":jsonl}),
            )
            .await
            .unwrap();
        serde_json::from_value(result["handle"].clone()).unwrap()
    }
}
impl Drop for Harness {
    fn drop(&mut self) {
        HandleRegistry::global().release_owner(self.0);
    }
}

fn fixture() -> Recording {
    let cassette: Value = serde_json::from_str(include_str!(
        "../../aimux-providers/tests/cassettes/openai/completions_api_agent_prompt.json"
    ))
    .unwrap();
    let options: CallOptions = serde_json::from_value(json!({
        "prompt": [{"role":"user","content":[{"type":"text","text":"Hi"}]}]
    }))
    .unwrap();
    let mut rec = Recording::new(
        "replay-test",
        InputRecord::from_call_options(&options),
        ProviderRecord::new("openai", "openai.chat", "gpt-4o"),
    );
    rec.complete = true;
    rec.transport_closed = true;
    rec.exchanges.push(HttpExchange {
        step: None,
        attempt: 1,
        exchange_index: 1,
        request: HttpRecord {
            method: "POST".into(),
            url: "https://api.openai.com/v1/chat/completions".into(),
            headers: vec![("authorization".into(), "[REDACTED]".into())],
            body: Some(serde_json::to_string(&cassette["request"]["body"]).unwrap()),
        },
        response: Some(ResponseRecord {
            status: 200,
            headers: vec![("content-type".into(), "application/json".into())],
            body: Some(cassette["response"]["body"].as_str().unwrap().into()),
            stream_chunks: None,
            ttfb_ms: None,
        }),
        timing: TimingRecord {
            latency_ms: 0,
            ttfb_ms: None,
        },
        error: None,
        finalized: true,
    });
    rec
}

fn decision_fixture() -> Recording {
    let mut rec = fixture();
    rec.call_id = "decision-test".into();
    rec.provider = ProviderRecord::new("jev", "jev", "jev-model");
    rec.input.operation = RecordingOperation::Decision;
    rec.input.prompt.clear();
    rec.input.options = json!({"state":"test", "questions":{
        "approve":{"type":"boolean", "instructions":"approve?"}
    }, "maxRetries":0});
    rec.exchanges.clear();
    rec.outcome.status = OutcomeStatus::Success;
    rec.outcome.decision_result = Some(json!({
        "answers":{"approve":{"type":"boolean", "probability":0.75}},
        "usage":{"inputTokens":1,"outputTokens":2,"totalTokens":3}, "warnings":[],
        "response":{"id":"recorded-decision","timestamp":"2026-10-15T00:00:00Z","modelId":"jev-model"}
    }));
    rec
}

fn target(kind: &str, handle: HandleId, call_id: &str) -> Value {
    json!({"callId":call_id,"target":{"kind":kind,"handle":handle}})
}

// Response IDs/timestamps and per-step call IDs/timings vary between calls.
fn stable(mut value: Value) -> Value {
    for step in value["steps"].as_array_mut().into_iter().flatten() {
        let step = step.as_object_mut().unwrap();
        step.remove("callId");
        step.remove("performance");
    }
    fn normalize(value: &mut Value) {
        match value {
            Value::Object(map) => {
                if let Some(response) = map.get_mut("response").and_then(Value::as_object_mut) {
                    response.remove("id");
                    response.remove("timestamp");
                }
                for value in map.values_mut() {
                    normalize(value);
                }
            }
            Value::Array(values) => {
                for value in values {
                    normalize(value);
                }
            }
            _ => {}
        }
    }
    normalize(&mut value);
    value
}

#[test]
fn metadata_keeps_transport_future_and_declares_nested_handles_and_cancellation() {
    let capabilities = dispatch::descriptor("replay.capabilities").unwrap();
    assert_eq!(capabilities.scope, Scope::Future);
    let live = dispatch::descriptor("replay.live").unwrap();
    assert_eq!(live.cancellation, Cancellation::Abort);
    assert!(
        live.runtime_handle_fields
            .iter()
            .any(|(path, _)| *path == "target.handle")
    );
    assert!(
        live.runtime_handle_fields
            .iter()
            .any(|(path, _)| *path == "abort")
    );
    assert!(live.side_effects);
    for name in [
        "replay.create",
        "replay.inspect",
        "replay.dryRun",
        "replay.live",
    ] {
        assert_eq!(dispatch::descriptor(name).unwrap().scope, Scope::Current);
    }
}

#[tokio::test]
async fn creation_inspection_and_planning_use_real_session_payloads() {
    let h = Harness::new();
    let id = h.create(&[fixture(), decision_fixture()]).await;
    let Entry::ReplaySession(model) = HandleRegistry::global()
        .get(h.0, id, &[aimux_ops::HandleKind::ReplaySession])
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(model.recordings().len(), 2);
    let inspect = h.call(id, "replay.inspect", json!({})).await.unwrap();
    assert_eq!(inspect["recordings"][0]["callId"], "replay-test");
    assert_eq!(inspect["recordings"][0]["exchangeCount"], 1);
    assert_eq!(inspect["recordings"][1]["operation"], "decision");
    let plan = h.call(id, "replay.dryRun", json!({})).await.unwrap();
    assert_eq!(plan["calls"].as_array().unwrap().len(), 2);
    for call in plan["calls"].as_array().unwrap() {
        assert_eq!(call["liveSupported"], true);
        assert_eq!(call["requiresTarget"], true);
    }
}

#[tokio::test]
async fn future_capabilities_return_projected_unsupported_even_with_complete_http_data() {
    let h = Harness::new();
    for complete in [true, false] {
        let mut rec = fixture();
        rec.complete = complete;
        let id = h.create(&[rec]).await;
        let error = h
            .call(id, "replay.capabilities", json!({}))
            .await
            .unwrap_err();
        assert_eq!(error.code(), 8);
        assert!(matches!(error, OpsError::Projected(_)));
        assert_eq!(
            error.envelope(SystemTime::UNIX_EPOCH).error["name"],
            "AI_UnsupportedFunctionalityError"
        );
    }
}

#[tokio::test]
async fn invalid_jsonl_empty_duplicate_and_unknown_schemas_are_rejected() {
    let h = Harness::new();
    let raw = serde_json::to_value(fixture()).unwrap();
    let mut future = raw.clone();
    future["schema"] = json!(3);
    for jsonl in [
        String::new(),
        "\n  \n".into(),
        "{broken".into(),
        format!("{raw}\n{raw}"),
        future.to_string(),
    ] {
        let error = h
            .call(
                HandleId(0),
                "replay.create",
                json!({"recordingsJsonl":jsonl}),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code(), 301);
    }
}

#[tokio::test]
async fn canonical_recording_reader_retains_registered_g_b_schema_support() {
    let h = Harness::new();
    let mut raw = serde_json::to_value(fixture()).unwrap();
    raw["schema"] = json!(1);
    let id = h
        .call(
            HandleId(0),
            "replay.create",
            json!({"recordingsJsonl":format!("\n{raw}\n\n")}),
        )
        .await
        .unwrap();
    let id = serde_json::from_value(id["handle"].clone()).unwrap();
    let info = h.call(id, "replay.inspect", json!({})).await.unwrap();
    assert_eq!(info["recordings"][0]["callId"], "replay-test");
}

#[tokio::test]
async fn controls_are_strict_including_nested_target_fields_and_wire_ids() {
    let h = Harness::new();
    let id = h.create(&[fixture()]).await;
    for op in ["replay.inspect", "replay.dryRun", "replay.capabilities"] {
        assert_eq!(
            h.call(id, op, json!({"unknown":true}))
                .await
                .unwrap_err()
                .code(),
            301
        );
    }
    assert_eq!(
        h.call(
            HandleId(0),
            "replay.create",
            json!({"recordingsJsonl":"", "settings":{}})
        )
        .await
        .unwrap_err()
        .code(),
        301
    );
    let model = h.insert(Entry::Language(Arc::new(MockReplayModel::new(
        "openai.chat",
        "gpt-4o",
        vec![fixture()],
    ))));
    for bad in [
        json!({"callId":"replay-test"}),
        json!({"call_id":"replay-test","target":{"kind":"model","handle":model}}),
        json!({"callId":"replay-test","target":{"kind":"model","handle":model,"apiKey":"legacy"}}),
        json!({"callId":"replay-test","target":{"kind":"unknown","handle":model}}),
        json!({"callId":"replay-test","target":{"kind":"model","handle":model.0}}),
        json!({"callId":"replay-test","target":{"kind":"model","handle":"+1"}}),
        json!({"callId":"replay-test","target":{"kind":"model","handle":"18446744073709551616"}}),
        json!({"callId":"replay-test","target":{"kind":"model","handle":model},"unknown":true}),
    ] {
        assert_eq!(
            h.call(id, "replay.live", bad).await.unwrap_err().code(),
            301
        );
    }
}

#[tokio::test]
async fn language_live_matches_the_existing_core_result_twice() {
    let h = Harness::new();
    let rec = fixture();
    let id = h.create(std::slice::from_ref(&rec)).await;
    let model = Arc::new(MockReplayModel::new(
        "openai.chat",
        "gpt-4o",
        vec![rec.clone()],
    ));
    let model_id = h.insert(Entry::Language(model.clone()));
    let expected = stable(
        serde_json::to_value(
            aimux_core::replay::replay_with_model(&rec, model.as_ref(), None)
                .await
                .unwrap(),
        )
        .unwrap(),
    );
    for _ in 0..2 {
        let actual = h
            .call(id, "replay.live", target("model", model_id, "replay-test"))
            .await
            .unwrap();
        assert_eq!(stable(actual.clone()), expected);
        let decoded: aimux_core::generate::GenerateTextResult =
            serde_json::from_value(actual).unwrap();
        assert_eq!(decoded.text(), "Hello! How can I assist you today?");
    }
}

#[tokio::test]
async fn language_registry_live_reuses_the_explicit_registered_provider() {
    let h = Harness::new();
    let id = h.create(&[fixture()]).await;
    let model: Arc<dyn LanguageModel> = Arc::new(MockReplayModel::new(
        "openai.chat",
        "gpt-4o",
        vec![fixture()],
    ));
    let provider = custom_provider(CustomProviderOptions {
        language_models: [("gpt-4o".into(), model)].into(),
        ..Default::default()
    });
    let providers: std::collections::BTreeMap<String, Arc<dyn aimux_core::provider::Provider>> =
        [(
            "openai".into(),
            Arc::new(provider) as Arc<dyn aimux_core::provider::Provider>,
        )]
        .into();
    let registry = h.insert(Entry::Registry(Arc::new(
        aimux_core::create_provider_registry(providers, Default::default()),
    )));
    let actual = h
        .call(
            id,
            "replay.live",
            target("registry", registry, "replay-test"),
        )
        .await
        .unwrap();
    let decoded: aimux_core::generate::GenerateTextResult = serde_json::from_value(actual).unwrap();
    assert_eq!(decoded.text(), "Hello! How can I assist you today?");
}

#[tokio::test]
async fn decision_live_matches_existing_core_for_model_and_registry_targets() {
    let h = Harness::new();
    let rec = decision_fixture();
    let id = h.create(std::slice::from_ref(&rec)).await;
    let model =
        Arc::new(MockDecisionReplayModel::new("jev", "jev-model", vec![rec.clone()]).unwrap());
    let expected = serde_json::to_value(
        aimux_core::replay::replay_decision_with_model(&rec, model.as_ref())
            .await
            .unwrap(),
    )
    .unwrap();
    let model_id = h.insert(Entry::Decision(model.clone()));
    let provider = custom_provider(CustomProviderOptions {
        decision_models: [(
            "jev-model".into(),
            model as Arc<dyn aimux_core::DecisionModel>,
        )]
        .into(),
        ..Default::default()
    });
    let providers: std::collections::BTreeMap<String, Arc<dyn aimux_core::provider::Provider>> =
        [(
            "jev".into(),
            Arc::new(provider) as Arc<dyn aimux_core::provider::Provider>,
        )]
        .into();
    let registry = h.insert(Entry::Registry(Arc::new(
        aimux_core::create_provider_registry(providers, Default::default()),
    )));
    for (kind, handle) in [("model", model_id), ("registry", registry)] {
        assert_eq!(
            h.call(id, "replay.live", target(kind, handle, "decision-test"))
                .await
                .unwrap(),
            expected
        );
    }
}

#[tokio::test]
async fn foreign_primary_and_secondary_handles_match_unknown_without_disclosure() {
    let h = Harness::new();
    let foreign = Harness::new();
    let id = h.create(&[fixture()]).await;
    let foreign_id = foreign.create(&[fixture()]).await;
    let unknown = HandleId(u64::MAX);
    for op in ["replay.inspect", "replay.capabilities", "replay.dryRun"] {
        let a = h.call(foreign_id, op, json!({})).await.unwrap_err();
        let b = h.call(unknown, op, json!({})).await.unwrap_err();
        assert_eq!(
            serde_json::to_value(a.envelope(SystemTime::UNIX_EPOCH)).unwrap(),
            serde_json::to_value(b.envelope(SystemTime::UNIX_EPOCH)).unwrap()
        );
    }
    let model = foreign.insert(Entry::Language(Arc::new(MockReplayModel::new(
        "openai.chat",
        "gpt-4o",
        vec![fixture()],
    ))));
    let a = h
        .call(id, "replay.live", target("model", model, "replay-test"))
        .await
        .unwrap_err();
    let b = h
        .call(id, "replay.live", target("model", unknown, "replay-test"))
        .await
        .unwrap_err();
    assert_eq!(a.code(), 203);
    assert_eq!(
        serde_json::to_value(a.envelope(SystemTime::UNIX_EPOCH)).unwrap(),
        serde_json::to_value(b.envelope(SystemTime::UNIX_EPOCH)).unwrap()
    );
}

#[tokio::test]
async fn unknown_call_and_target_kind_mismatches_do_not_replay() {
    let h = Harness::new();
    let id = h.create(&[fixture()]).await;
    let model = h.insert(Entry::Language(Arc::new(MockReplayModel::new(
        "openai.chat",
        "gpt-4o",
        vec![fixture()],
    ))));
    assert_eq!(
        h.call(id, "replay.live", target("model", model, "missing"))
            .await
            .unwrap_err()
            .code(),
        301
    );
    assert_eq!(
        h.call(id, "replay.live", target("registry", model, "replay-test"))
            .await
            .unwrap_err()
            .code(),
        203
    );
    assert_eq!(
        h.call(id, "replay.live", target("model", id, "replay-test"))
            .await
            .unwrap_err()
            .code(),
        203
    );
}

#[tokio::test]
async fn live_accepts_context_and_owner_checked_wire_abort_signals() {
    let h = Harness::new();
    let foreign = Harness::new();
    let id = h.create(&[fixture()]).await;
    let model = h.insert(Entry::Language(Arc::new(MockReplayModel::new(
        "openai.chat",
        "gpt-4o",
        vec![fixture()],
    ))));
    let signal = aimux_core::AbortSignal::new();
    signal.abort();
    let mut ctx = h.ctx();
    ctx.abort = Some(signal.clone());
    assert_eq!(
        dispatch::call(
            ctx,
            id,
            "replay.live",
            target("model", model, "replay-test")
        )
        .await
        .unwrap_err()
        .code(),
        13
    );
    let abort = h.insert(Entry::Abort(Arc::new(signal.clone())));
    let mut params = target("model", model, "replay-test");
    params["abort"] = json!(abort);
    assert_eq!(
        h.call(id, "replay.live", params.clone())
            .await
            .unwrap_err()
            .code(),
        13
    );
    params["abort"] = json!(foreign.insert(Entry::Abort(Arc::new(signal))));
    let foreign_error = h.call(id, "replay.live", params.clone()).await.unwrap_err();
    params["abort"] = json!(HandleId(u64::MAX));
    let unknown_error = h.call(id, "replay.live", params).await.unwrap_err();
    assert_eq!(foreign_error.code(), 203);
    assert_eq!(
        serde_json::to_value(foreign_error.envelope(SystemTime::UNIX_EPOCH)).unwrap(),
        serde_json::to_value(unknown_error.envelope(SystemTime::UNIX_EPOCH)).unwrap()
    );
}
