//! Exercise real decision dispatch without network or a second decision engine.
use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime},
};

use aimux_core::{
    AbortSignal, AiMuxError, Provider,
    decision_model::{
        DecideOptions, DecisionCallOptions, DecisionModel, DecisionQuestionType, DecisionResult,
        decide, decision_model_properties,
    },
    error::ApiCallError,
};
use aimux_ops::{
    Ctx, Entry, HandleId, HandleKind, HandleRegistry, OpsError, Owner, dispatch, manifest::manifest,
};
use serde_json::{Value, json};

#[derive(Clone, Copy)]
enum Reply {
    Valid,
    Invalid,
    Error,
    Pending,
    Retry,
}

struct Model {
    reply: Reply,
    calls: AtomicUsize,
    seen: Mutex<Vec<DecisionCallOptions>>,
    entered: tokio::sync::Notify,
    supported: &'static [DecisionQuestionType],
}
impl Model {
    fn new(reply: Reply) -> Arc<Self> {
        Arc::new(Self {
            reply,
            calls: AtomicUsize::new(0),
            seen: Mutex::new(Vec::new()),
            entered: tokio::sync::Notify::new(),
            supported: DecisionQuestionType::ALL,
        })
    }
}

impl DecisionModel for Model {
    fn provider(&self) -> &str {
        "test-decision"
    }
    fn model_id(&self) -> &str {
        "test-model"
    }
    fn supported_question_types(&self) -> &[DecisionQuestionType] {
        self.supported
    }

    // Match DecisionModel's async-trait ABI without adding a dependency to the
    // production crate solely for this test double.
    fn do_decide<'a, 'b, 'f>(
        &'a self,
        options: &'b DecisionCallOptions,
    ) -> Pin<Box<dyn Future<Output = Result<DecisionResult, AiMuxError>> + Send + 'f>>
    where
        'a: 'f,
        'b: 'f,
        Self: 'f,
    {
        Box::pin(async move {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            self.seen.lock().unwrap().push(options.clone());
            self.entered.notify_one();
            match self.reply {
                Reply::Pending => std::future::pending::<()>().await,
                Reply::Error | Reply::Retry if call == 0 || matches!(self.reply, Reply::Error) => {
                    let mut error =
                        ApiCallError::new("busy", "https://example.test/decide", json!({}));
                    error.status_code = Some(503);
                    error.is_retryable = true;
                    error.response_headers = Some(match self.reply {
                        Reply::Error => {
                            [("retry-after".into(), "Thu, 01 Jan 1970 00:16:45 GMT".into())].into()
                        }
                        _ => [("retry-after-ms".into(), "0".into())].into(),
                    });
                    return Err(AiMuxError::ApiCall(Box::new(error)));
                }
                _ => {}
            }
            let answers = if matches!(self.reply, Reply::Invalid) {
                json!({})
            } else {
                json!({
                    "choice":{"type":"choice","choice":"yes","probabilities":{"yes":0.75,"no":0.25}},
                    "score":{"type":"score","score":0.5},
                    "boolean":{"type":"boolean","probability":0.25}
                })
            };
            Ok(serde_json::from_value(json!({
                "answers":answers,
                "usage":{"inputTokens":3,"outputTokens":2},
                "warnings":[],
                "providerMetadata":{"test-decision":{"probabilitySource":"native"}},
                "response":{"id":"response-1","timestamp":"2026-10-15T00:00:00.000Z","modelId":"test-model"}
            })).unwrap())
        })
    }
}

struct Fixture {
    owner: Owner,
    id: HandleId,
    model: Arc<Model>,
}
impl Fixture {
    fn new(reply: Reply) -> Self {
        let model = Model::new(reply);
        Self::with_model(model)
    }
    fn with_model(model: Arc<Model>) -> Self {
        let owner = Owner::new_node().unwrap();
        let id = HandleRegistry::global()
            .insert(owner, Entry::Decision(model.clone()))
            .unwrap();
        Self { owner, id, model }
    }
    fn ctx(&self) -> Ctx {
        ctx(self.owner)
    }
    async fn call(&self, op: &str, params: Value) -> Result<Value, OpsError> {
        dispatch::call(self.ctx(), self.id, op, params).await
    }
    fn abort(&self) -> (HandleId, AbortSignal) {
        let signal = AbortSignal::new();
        let id = HandleRegistry::global()
            .insert(self.owner, Entry::Abort(Arc::new(signal.clone())))
            .unwrap();
        (id, signal)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        HandleRegistry::global().release_owner(self.owner);
    }
}
fn now() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(1_000)
}
fn ctx(owner: Owner) -> Ctx {
    Ctx {
        owner,
        abort: None,
        now,
    }
}
fn options() -> Value {
    json!({"state":{"文本":"原始状态 🦀"},"questions":{
        "choice":{"type":"choice","instructions":"选一个","criteria":{"yes":null,"no":"不"}},
        "score":{"type":"score","instructions":"评分","criteria":[null,"高"]},
        "boolean":{"type":"boolean","instructions":"是否？","criteria":{"true":null}}
    },"maxRetries":0,"headers":{"X-Request":"value"},
      "providerOptions":{"test-decision":{"arbitrary":{"nested":[1,null,true]}}}})
}
fn envelope(error: &OpsError) -> Value {
    serde_json::to_value(error.envelope(now())).unwrap()
}

#[tokio::test]
async fn decide_matches_core_for_all_state_shapes_and_question_types() {
    for state in [
        json!("文本 🦀"),
        json!({"key":[1,null,true]}),
        json!(["state",{"n":2}]),
    ] {
        let fixture = Fixture::new(Reply::Valid);
        let mut options = options();
        options["state"] = state;
        let expected = decide(
            fixture.model.as_ref(),
            serde_json::from_value(options.clone()).unwrap(),
        )
        .await
        .unwrap();
        let actual = fixture
            .call("decision.decide", json!({"options":options}))
            .await
            .unwrap();
        assert_eq!(actual, serde_json::to_value(expected).unwrap());
        assert_eq!(actual["usage"]["totalTokens"], 5);
        let seen = fixture.model.seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(
            serde_json::to_value(&seen[0]).unwrap(),
            serde_json::to_value(&seen[1]).unwrap()
        );
    }
}

#[tokio::test]
async fn capabilities_match_existing_providers_and_children_survive_parent_drop() {
    for source in ["native", "logit_scoring", "model_estimate"] {
        let mut config = aimux_providers::JevConfig::new("secret")
            .with_endpoint("http://127.0.0.1:1/never-called");
        config.probability_source = source.parse().unwrap();
        let owner = Owner::new_python().unwrap();
        let provider: Arc<dyn Provider> = Arc::new(aimux_providers::JevProvider::new(config));
        let model = provider.decision_model("jev-latest").unwrap().unwrap();
        let expected = decision_model_properties(model.as_ref());
        let registry = HandleRegistry::global();
        let parent = registry.insert(owner, Entry::Provider(provider)).unwrap();
        let child = registry.insert(owner, Entry::Decision(model)).unwrap();
        drop(registry.remove(owner, parent));
        let mut context = ctx(owner);
        let aborted = AbortSignal::new();
        aborted.abort();
        context.abort = Some(aborted);
        let actual = dispatch::call(context, child, "decision.capabilities", json!({}))
            .await
            .unwrap();
        assert_eq!(actual, expected);
        assert_eq!(
            actual["supportedQuestionTypes"],
            json!(["choice", "score", "boolean"])
        );
        assert!(!actual.to_string().contains("secret"));
        assert_eq!(registry.release_owner(owner), 1);
    }
    let provider = aimux_providers::create_openai(Default::default()).unwrap();
    let model = provider.decision_model("gpt-4o").unwrap().unwrap();
    let owner = Owner::new_node().unwrap();
    let id = HandleRegistry::global()
        .insert(owner, Entry::Decision(model.clone()))
        .unwrap();
    assert_eq!(
        dispatch::call(ctx(owner), id, "decision.capabilities", json!({}))
            .await
            .unwrap(),
        decision_model_properties(model.as_ref())
    );
    HandleRegistry::global().release_owner(owner);
}

#[tokio::test]
async fn strict_controls_reject_unknown_fields_aliases_and_invalid_shapes_before_model() {
    let fixture = Fixture::new(Reply::Valid);
    for request in [
        json!({}),
        json!({"options":null}),
        json!({"options":{},"abort":null}),
        json!({"options":options(),"modelId":"old-provider-shape"}),
        json!({"options":options(),"abort_signal":"1"}),
        json!({"options":options(),"unknown":true}),
    ] {
        assert_eq!(
            fixture
                .call("decision.decide", request)
                .await
                .unwrap_err()
                .code(),
            301
        );
    }
    for request in [json!({"modelId":"old"}), json!({"unknown":true})] {
        assert_eq!(
            fixture
                .call("decision.capabilities", request)
                .await
                .unwrap_err()
                .code(),
            301
        );
    }
    assert_eq!(fixture.model.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn business_validation_and_unsupported_types_match_core_before_model() {
    let fixture = Fixture::new(Reply::Valid);
    let mut request = options();
    request["questions"] = json!({});
    let expected = decide(
        fixture.model.as_ref(),
        serde_json::from_value(request.clone()).unwrap(),
    )
    .await
    .unwrap_err();
    let error = fixture
        .call("decision.decide", json!({"options":request}))
        .await
        .unwrap_err();
    assert_eq!(envelope(&error), envelope(&OpsError::from(expected)));
    assert_eq!(fixture.model.calls.load(Ordering::SeqCst), 0);

    let mut model = Model::new(Reply::Valid);
    Arc::get_mut(&mut model).unwrap().supported = &[DecisionQuestionType::Boolean];
    let fixture = Fixture::with_model(model);
    let expected = decide(
        fixture.model.as_ref(),
        serde_json::from_value(options()).unwrap(),
    )
    .await
    .unwrap_err();
    let error = fixture
        .call("decision.decide", json!({"options":options()}))
        .await
        .unwrap_err();
    assert_eq!(envelope(&error), envelope(&OpsError::from(expected)));
    assert_eq!(error.code(), 8);
    assert_eq!(fixture.model.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn invalid_model_answers_keep_core_error_and_cached_projection() {
    let fixture = Fixture::new(Reply::Invalid);
    let expected = decide(
        fixture.model.as_ref(),
        serde_json::from_value(options()).unwrap(),
    )
    .await
    .unwrap_err();
    let error = fixture
        .call("decision.decide", json!({"options":options()}))
        .await
        .unwrap_err();
    assert!(matches!(error, OpsError::Projected(_)));
    assert_eq!(envelope(&error), envelope(&OpsError::from(expected)));
    assert_eq!(error.code(), 3);
    assert_eq!(
        serde_json::to_value(error.envelope(SystemTime::UNIX_EPOCH)).unwrap(),
        envelope(&error)
    );
}

#[tokio::test]
async fn retry_and_provider_errors_remain_owned_by_core() {
    let fixture = Fixture::new(Reply::Error);
    let error = fixture
        .call("decision.decide", json!({"options":options()}))
        .await
        .unwrap_err();
    assert_eq!(error.code(), 11);
    assert_eq!(envelope(&error)["retryAfterMs"], 5_000);
    assert_eq!(
        serde_json::to_value(error.envelope(SystemTime::UNIX_EPOCH)).unwrap(),
        envelope(&error)
    );
    assert_eq!(fixture.model.calls.load(Ordering::SeqCst), 1);
    let fixture = Fixture::new(Reply::Retry);
    let mut request = options();
    request["maxRetries"] = json!(1);
    fixture
        .call("decision.decide", json!({"options":request}))
        .await
        .unwrap();
    assert_eq!(fixture.model.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn secondary_handles_are_typed_owner_checked_and_nondisclosing() {
    let fixture = Fixture::new(Reply::Valid);
    let foreign = Fixture::new(Reply::Valid);
    let (abort, _) = foreign.abort();
    let unknown = fixture
        .call(
            "decision.decide",
            json!({"options":options(),"abort":u64::MAX.to_string()}),
        )
        .await
        .unwrap_err();
    let foreign_error = fixture
        .call(
            "decision.decide",
            json!({"options":options(),"abort":abort}),
        )
        .await
        .unwrap_err();
    assert_eq!(envelope(&unknown), envelope(&foreign_error));
    assert_eq!(foreign_error.code(), 203);
    let wrong = fixture
        .call(
            "decision.decide",
            json!({"options":options(),"abort":fixture.id}),
        )
        .await
        .unwrap_err();
    assert_eq!(wrong.code(), 203);
    assert_eq!(envelope(&wrong)["error"]["actual"], "Decision");
    for abort in [
        json!(0),
        json!("0"),
        json!("18446744073709551616"),
        json!("-1"),
    ] {
        let error = fixture
            .call(
                "decision.decide",
                json!({"options":options(),"abort":abort}),
            )
            .await
            .unwrap_err();
        assert!(matches!(error.code(), 203 | 301));
    }
    assert_eq!(fixture.model.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn primary_handles_reject_global_wrong_kind_foreign_and_dropped() {
    let fixture = Fixture::new(Reply::Valid);
    let foreign = Fixture::new(Reply::Valid);
    let (wrong, _) = fixture.abort();
    for op in ["decision.capabilities", "decision.decide"] {
        let params = if op == "decision.decide" {
            json!({"options":options()})
        } else {
            json!({})
        };
        let unknown = dispatch::call(fixture.ctx(), HandleId(u64::MAX), op, params.clone())
            .await
            .unwrap_err();
        let foreign_error = dispatch::call(fixture.ctx(), foreign.id, op, params.clone())
            .await
            .unwrap_err();
        assert_eq!(envelope(&unknown), envelope(&foreign_error));
        for id in [HandleId(0), wrong] {
            assert_eq!(
                dispatch::call(fixture.ctx(), id, op, params.clone())
                    .await
                    .unwrap_err()
                    .code(),
                203
            );
        }
    }
    drop(HandleRegistry::global().remove(fixture.owner, fixture.id));
    assert_eq!(
        fixture
            .call("decision.capabilities", json!({}))
            .await
            .unwrap_err()
            .code(),
        203
    );
    assert_eq!(
        fixture
            .call("decision.decide", json!({"options": options()}))
            .await
            .unwrap_err()
            .code(),
        203
    );
    assert_eq!(fixture.model.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn abort_from_context_or_secondary_prevents_model_execution() {
    let fixture = Fixture::new(Reply::Valid);
    let (id, signal) = fixture.abort();
    signal.abort();
    let error = fixture
        .call("decision.decide", json!({"options":options(),"abort":id}))
        .await
        .unwrap_err();
    assert_eq!(error.code(), 13);
    let mut context = fixture.ctx();
    context.abort = Some(signal);
    let error = dispatch::call(
        context,
        fixture.id,
        "decision.decide",
        json!({"options":options()}),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), 13);
    assert_eq!(fixture.model.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn abort_reaches_running_model_and_timeout_uses_core() {
    let fixture = Fixture::new(Reply::Pending);
    let (id, signal) = fixture.abort();
    let request = fixture.call("decision.decide", json!({"options":options(),"abort":id}));
    let cancel = async {
        fixture.model.entered.notified().await;
        let seen_signal = fixture.model.seen.lock().unwrap()[0]
            .abort_signal
            .clone()
            .unwrap();
        signal.abort();
        assert!(seen_signal.is_aborted());
    };
    let (result, ()) = tokio::join!(request, cancel);
    assert_eq!(result.unwrap_err().code(), 13);
    let mut options = options();
    options["timeout"] = json!({"totalMs":10});
    assert_eq!(
        fixture
            .call("decision.decide", json!({"options":options}))
            .await
            .unwrap_err()
            .code(),
        12
    );
}

#[test]
fn descriptors_publish_decision_controls_and_canonical_types() {
    let descriptor = dispatch::descriptor("decision.decide").unwrap();
    assert_eq!(descriptor.allowed_handle_kinds, &[HandleKind::Decision]);
    assert_eq!(
        descriptor.runtime_handle_fields,
        &[("abort", &[HandleKind::Abort][..])]
    );
    assert_eq!(descriptor.retry_owner, "core");
    assert_eq!(
        (descriptor.result_type.id)().into_owned(),
        "aimux_core::decision_model::DecideResult"
    );
    let manifest = manifest();
    let ty = manifest["types"]
        .as_array()
        .unwrap()
        .iter()
        .find(|ty| ty["id"] == (descriptor.request_type.id)().into_owned())
        .unwrap();
    assert_eq!(
        ty["rustPath"],
        "aimux_ops::ops::decision::DecisionDecideRequest"
    );
    assert!(
        serde_json::from_value::<aimux_ops::ops::decision::DecisionDecideRequest>(
            json!({"options": options(), "abort": null})
        )
        .is_ok()
    );
    let _: DecideOptions = serde_json::from_value(options()).unwrap();
}
