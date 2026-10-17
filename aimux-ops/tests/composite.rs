use aimux_core::LanguageModel;
use aimux_core::error::AiMuxError;
use aimux_core::operation::OperationSettings;
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateResult, StreamResult};
use aimux_ops::dispatch::{self, Ctx};
use aimux_ops::envelope::OpsError;
use aimux_ops::handle::{Entry, HandleId, HandleKind, HandleRegistry, Owner};
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

struct FailingChild(Arc<AtomicUsize>, bool);

// Spell out async-trait's return type to avoid an ops-only macro dependency.
impl LanguageModel for FailingChild {
    fn provider(&self) -> &str {
        "mock"
    }
    fn model_id(&self) -> &str {
        "mock"
    }
    fn do_generate<'a, 'b, 'f>(
        &'a self,
        _: &'b CallOptions,
    ) -> Pin<Box<dyn Future<Output = Result<GenerateResult, AiMuxError>> + Send + 'f>>
    where
        'a: 'f,
        'b: 'f,
        Self: 'f,
    {
        Box::pin(async move {
            self.0.fetch_add(1, Ordering::SeqCst);
            if self.1 {
                return Ok(GenerateResult {
                    content: vec![],
                    finish_reason: aimux_core::types::FinishReason {
                        unified: aimux_core::types::FinishReasonUnified::Stop,
                        raw: None,
                    },
                    usage: Default::default(),
                    warnings: vec![],
                    provider_metadata: None,
                    response: None,
                    request: None,
                });
            }
            Err(AiMuxError::ApiCall(Box::new(aimux_core::ApiCallError {
                is_retryable: true,
                response_headers: Some(std::collections::HashMap::from([(
                    "retry-after-ms".into(),
                    "0".into(),
                )])),
                ..aimux_core::ApiCallError::new("mock", "https://example.test", Value::Null)
            })))
        })
    }
    fn do_stream<'a, 'b, 'f>(
        &'a self,
        _: &'b CallOptions,
    ) -> Pin<Box<dyn Future<Output = Result<StreamResult, AiMuxError>> + Send + 'f>>
    where
        'a: 'f,
        'b: 'f,
        Self: 'f,
    {
        Box::pin(async { Err(AiMuxError::Other("mock stream".into())) })
    }
}

fn child(owner: Owner) -> (String, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let id = HandleRegistry::global()
        .insert(
            owner,
            Entry::Language(Arc::new(FailingChild(calls.clone(), false))),
        )
        .unwrap();
    (id.0.to_string(), calls)
}

fn ctx(owner: Owner) -> Ctx {
    Ctx {
        owner,
        abort: None,
        now: std::time::SystemTime::now,
    }
}

async fn construct(owner: Owner, name: &str, params: Value) -> Result<Value, OpsError> {
    dispatch::call(ctx(owner), HandleId(0), name, params).await
}

fn language(owner: Owner, id: &str) -> Result<Arc<dyn LanguageModel>, OpsError> {
    match dispatch::resolve_secondary(
        HandleRegistry::global(),
        &ctx(owner),
        &Value::String(id.to_owned()),
        &[HandleKind::Language],
    )? {
        Entry::Language(model) => Ok(model),
        _ => panic!("typed resolver returned a non-language entry"),
    }
}

fn remove(owner: Owner, id: &str) {
    drop(HandleRegistry::global().remove(owner, HandleId(id.parse().unwrap())));
}

#[tokio::test]
async fn rejects_bad_children_and_control_fields() {
    let owner = Owner::new_node().unwrap();
    let (valid, _) = child(owner);
    for params in [
        serde_json::json!({"children": [], "strategy": {"type": "rule"}}),
        serde_json::json!({"children": [valid]}),
        serde_json::json!({"children": [valid], "strategy": {"type": "unknown"}}),
        serde_json::json!({"children": [valid], "strategy": {"type": "rule", "extra": true}}),
        serde_json::json!({"children": [valid], "strategy": {"type": "rule"}, "extra": true}),
        serde_json::json!({"children": [valid, "0"], "strategy": {"type": "rule"}}),
        serde_json::json!({"children": [1], "strategy": {"type": "rule"}}),
    ] {
        assert!(construct(owner, "composite.router", params).await.is_err());
    }
    assert!(
        construct(
            Owner::new_python().unwrap(),
            "composite.router",
            serde_json::json!({
                "children": [valid], "strategy": {"type": "rule"}
            })
        )
        .await
        .is_err()
    );
    for params in [
        serde_json::json!({"references": [valid]}),
        serde_json::json!({"references": [valid], "aggregator": "0"}),
        serde_json::json!({"references": ["0"], "aggregator": valid}),
        serde_json::json!({"references": [], "aggregator": valid, "extra": true}),
    ] {
        assert!(construct(owner, "composite.moa", params).await.is_err());
    }
    remove(owner, &valid);
    assert!(
        construct(
            owner,
            "composite.router",
            serde_json::json!({
                "children": [valid], "strategy": {"type": "rule"}
            })
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn dropped_router_children_keep_working_with_independent_retry_budgets() {
    let owner = Owner::new_node().unwrap();
    let (first, first_calls) = child(owner);
    let (second, second_calls) = child(owner);
    let id = construct(
        owner,
        "composite.router",
        serde_json::json!({
            "children": [first, second], "strategy": {"type": "rule"}
        }),
    )
    .await
    .unwrap();
    remove(owner, &first);
    remove(owner, &second);
    let model = language(owner, id.as_str().unwrap()).unwrap();
    let options = CallOptions::default();
    let result = OperationSettings {
        max_retries: Some(1),
        ..Default::default()
    }
    .scope(model.do_generate(&options))
    .await;
    assert!(result.is_err());
    assert_eq!(first_calls.load(Ordering::SeqCst), 2);
    assert_eq!(second_calls.load(Ordering::SeqCst), 2);
    remove(owner, id.as_str().unwrap());
}

#[tokio::test]
async fn weighted_strategy_and_no_fallback_are_explicit() {
    let owner = Owner::new_node().unwrap();
    let (first, first_calls) = child(owner);
    let (second, second_calls) = child(owner);
    let id = construct(
        owner,
        "composite.router",
        serde_json::json!({
            "children": [first, second], "strategy": {"type": "weighted", "weights": [0, 1]},
            "fallback": "none"
        }),
    )
    .await
    .unwrap();
    let model = language(owner, id.as_str().unwrap()).unwrap();
    let options = CallOptions::default();
    assert!(
        OperationSettings {
            max_retries: Some(0),
            ..Default::default()
        }
        .scope(model.do_generate(&options))
        .await
        .is_err()
    );
    assert_eq!(first_calls.load(Ordering::SeqCst), 0);
    assert_eq!(second_calls.load(Ordering::SeqCst), 1);
    for handle in [&first, &second, id.as_str().unwrap()] {
        remove(owner, handle);
    }
}

#[tokio::test]
async fn moa_retains_aggregator_after_its_handle_is_dropped() {
    let owner = Owner::new_node().unwrap();
    let (aggregator, calls) = child(owner);
    let id = construct(
        owner,
        "composite.moa",
        serde_json::json!({
            "references": [], "aggregator": aggregator
        }),
    )
    .await
    .unwrap();
    remove(owner, &aggregator);
    let model = language(owner, id.as_str().unwrap()).unwrap();
    let options = CallOptions::default();
    assert!(
        OperationSettings {
            max_retries: Some(0),
            ..Default::default()
        }
        .scope(model.do_generate(&options))
        .await
        .is_err()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    remove(owner, id.as_str().unwrap());
}
#[tokio::test]
async fn moa_retains_references_and_aggregator_and_completes() {
    let owner = Owner::new_node().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let registry = HandleRegistry::global();
    let reference = registry
        .insert(
            owner,
            Entry::Language(Arc::new(FailingChild(calls.clone(), true))),
        )
        .unwrap();
    let aggregator = registry
        .insert(
            owner,
            Entry::Language(Arc::new(FailingChild(calls.clone(), true))),
        )
        .unwrap();
    let id = construct(
        owner,
        "composite.moa",
        serde_json::json!({
            "references": [reference.0.to_string()], "aggregator": aggregator.0.to_string(),
            "failMode": "fail-fast", "stripReferenceTools": false
        }),
    )
    .await
    .unwrap();
    drop(registry.remove(owner, reference));
    drop(registry.remove(owner, aggregator));
    let model = language(owner, id.as_str().unwrap()).unwrap();
    assert!(model.do_generate(&CallOptions::default()).await.is_ok());
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    remove(owner, id.as_str().unwrap());
}
#[tokio::test]
async fn router_completes_after_child_handle_is_dropped() {
    let owner = Owner::new_node().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let child = HandleRegistry::global()
        .insert(
            owner,
            Entry::Language(Arc::new(FailingChild(calls.clone(), true))),
        )
        .unwrap();
    let id = construct(
        owner,
        "composite.router",
        serde_json::json!({
            "children": [child.0.to_string()], "strategy": {"type": "rule"}
        }),
    )
    .await
    .unwrap();
    drop(HandleRegistry::global().remove(owner, child));
    let model = language(owner, id.as_str().unwrap()).unwrap();
    assert!(model.do_generate(&CallOptions::default()).await.is_ok());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    remove(owner, id.as_str().unwrap());
}

#[tokio::test]
async fn every_secondary_slot_hides_foreign_handles_and_rejects_wrong_kinds() {
    let registry = HandleRegistry::global();
    let owner = Owner::new_node().unwrap();
    let foreign = Owner::new_python().unwrap();
    let (valid, calls) = child(owner);
    let (foreign_id, foreign_calls) = child(foreign);
    let wrong = registry
        .insert(
            owner,
            Entry::Abort(Arc::new(aimux_core::AbortSignal::new())),
        )
        .unwrap();
    for slot in 0..3 {
        let params = |id: Value| match slot {
            0 => serde_json::json!({"children": [valid, id], "strategy": {"type":"rule"}}),
            1 => serde_json::json!({"references": [valid, id], "aggregator": valid}),
            _ => serde_json::json!({"references": [valid], "aggregator": id}),
        };
        let name = if slot == 0 {
            "composite.router"
        } else {
            "composite.moa"
        };
        let foreign_error = construct(owner, name, params(Value::String(foreign_id.clone())))
            .await
            .unwrap_err();
        let unknown_error = construct(owner, name, params(Value::String(u64::MAX.to_string())))
            .await
            .unwrap_err();
        assert!(matches!(foreign_error, OpsError::Projected(_)));
        let envelope = foreign_error.envelope(std::time::UNIX_EPOCH);
        assert_eq!(envelope.code, 203);
        assert_eq!(envelope.error["actual"], Value::Null);
        assert_eq!(envelope, unknown_error.envelope(std::time::UNIX_EPOCH));
        assert_eq!(
            envelope,
            foreign_error.envelope(std::time::SystemTime::now())
        );
        let error = construct(owner, name, params(Value::String(wrong.0.to_string())))
            .await
            .unwrap_err();
        assert_eq!(error.code(), 203);
        assert_eq!(
            error.envelope(std::time::UNIX_EPOCH).error["actual"],
            "Abort"
        );
        for malformed in [
            serde_json::json!(1),
            Value::Null,
            serde_json::json!(""),
            serde_json::json!("-1"),
            serde_json::json!("+1"),
            serde_json::json!("1e3"),
            serde_json::json!("18446744073709551616"),
        ] {
            let error = construct(owner, name, params(malformed)).await.unwrap_err();
            assert_eq!(error.code(), 301, "{error:?}");
            assert!(matches!(error, OpsError::Projected(_)));
        }
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(foreign_calls.load(Ordering::SeqCst), 0);
    // Failed construction never inserts a new handle or removes dependencies.
    assert_eq!(registry.release_owner(owner), 2);
    assert_eq!(registry.release_owner(foreign), 1);
}

#[tokio::test]
async fn control_configs_are_strict_and_preserve_model_identity() {
    let owner = Owner::new_node().unwrap();
    let (valid, _) = child(owner);
    for (name, params) in [
        (
            "composite.router",
            serde_json::json!({"children":[valid],"strategy":{"type":"weighted"}}),
        ),
        (
            "composite.router",
            serde_json::json!({"children":[valid],"strategy":{"type":"weighted","weights":[],"extra":1}}),
        ),
        (
            "composite.router",
            serde_json::json!({"children":[valid],"strategy":{"type":"rule"},"fallback":"unexpected"}),
        ),
        (
            "composite.router",
            serde_json::json!({"children":[valid],"strategy":{"type":"rule"},"provider_name":"alias"}),
        ),
        (
            "composite.moa",
            serde_json::json!({"references":[],"aggregator":valid,"provider_name":"alias"}),
        ),
        (
            "composite.moa",
            serde_json::json!({"references":[],"aggregator":valid,"failMode":"unknown"}),
        ),
        (
            "composite.moa",
            serde_json::json!({"references":[],"aggregator":valid,"stripReferenceTools":1}),
        ),
    ] {
        assert_eq!(
            construct(owner, name, params).await.unwrap_err().code(),
            301
        );
    }
    for (name, params) in [
        (
            "composite.router",
            serde_json::json!({"children":[valid],"strategy":{"type":"rule"},"fallback":"on-error","providerName":"custom","modelId":"id"}),
        ),
        (
            "composite.moa",
            serde_json::json!({"references":[],"aggregator":valid,"providerName":"custom","modelId":"id","aggregatorInstructions":"combine","stripReferenceTools":false,"failMode":"best-effort"}),
        ),
    ] {
        let id = construct(owner, name, params).await.unwrap();
        let model = language(owner, id.as_str().unwrap()).unwrap();
        assert_eq!(model.provider(), "custom");
        assert_eq!(model.model_id(), "id");
        remove(owner, id.as_str().unwrap());
    }
    remove(owner, &valid);
}

#[test]
fn descriptors_publish_concrete_types_and_all_secondary_slots() {
    use aimux_ops::op::{Cancellation, OpMode, RequestBytePolicy, Scope};
    for (name, request, slots) in [
        ("composite.router", "RouterParams", vec!["children.*"]),
        (
            "composite.moa",
            "MoaParams",
            vec!["references.*", "aggregator"],
        ),
    ] {
        let op = dispatch::descriptor(name).unwrap();
        assert_eq!(op.mode, OpMode::Call);
        assert_eq!(op.scope, Scope::Current);
        assert!(op.allowed_handle_kinds.is_empty());
        assert_eq!(
            op.request_type,
            format!("aimux_ops::ops::composite::{request}")
        );
        assert_eq!(op.result_type, "HandleId");
        assert_eq!(
            op.runtime_handle_fields
                .iter()
                .map(|(path, _)| *path)
                .collect::<Vec<_>>(),
            slots
        );
        assert!(
            op.runtime_handle_fields
                .iter()
                .all(|(_, kinds)| *kinds == [HandleKind::Language])
        );
        assert_eq!(op.cancellation, Cancellation::None);
        assert_eq!(op.request_byte_policy, RequestBytePolicy::Json);
        assert_eq!(op.result_ownership, "owned");
        assert!(op.event_type.is_none() && op.terminal_type.is_none());
        assert!(op.required_capabilities.is_empty());
        assert!(op.side_effects);
        assert_eq!(op.retry_owner, "operation");
    }
}
