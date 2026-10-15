//! Provider operations through shared dispatch and cached error envelopes.
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, SystemTime};

use aimux_core::custom_provider::{CustomProviderOptions, custom_provider};
use aimux_core::model_catalogue::RuntimeModel;
use aimux_core::provider::ProviderDiscovery;
use aimux_core::{AiMuxError, Provider};
use aimux_ops::dispatch::{Ctx, call, descriptor};
use aimux_ops::envelope::OpsError;
use aimux_ops::handle::{Entry, HandleId, HandleKind, HandleRegistry, Owner};
use futures::future::BoxFuture;
use serde_json::{Value, json};

fn ctx(owner: Owner) -> Ctx {
    Ctx {
        owner,
        abort: None,
        now: SystemTime::now,
    }
}
fn id(value: &Value) -> HandleId {
    serde_json::from_value(value["handle"].clone()).unwrap()
}
fn get(owner: Owner, value: &Value, kind: HandleKind) -> Entry {
    HandleRegistry::global()
        .get(owner, id(value), &[kind])
        .unwrap()
}
fn insert(owner: Owner, entry: Entry) -> HandleId {
    HandleRegistry::global().insert(owner, entry).unwrap()
}
async fn jev(owner: Owner, source: &str) -> Value {
    call(ctx(owner), HandleId(0), "provider.create", json!({
        "package":"jev", "settings":{"apiKey":"test-key", "endpoint":"http://localhost:1/decision", "probabilitySource":source, "headers":{"x-test":"v"}}
    })).await.unwrap()
}

#[tokio::test]
async fn jev_decision_construction_custom_aliases_and_wrap_keep_children_alive() {
    for source in ["native", "logit_scoring", "model_estimate"] {
        let owner = Owner::new_node().unwrap();
        let provider = jev(owner, source).await;
        let decision = call(
            ctx(owner),
            id(&provider),
            "provider.model",
            json!({"method":"decision", "modelId":"original"}),
        )
        .await
        .unwrap();
        let Entry::Decision(original) = get(owner, &decision, HandleKind::Decision) else {
            panic!()
        };
        let custom = call(ctx(owner), HandleId(0), "provider.custom", json!({
            "decisionModels":{"alias":decision["handle"]}, "defaultMethod":"decision", "fallbackProvider":provider["handle"]
        })).await.unwrap();
        let wrapped = call(ctx(owner), id(&custom), "provider.wrap", json!({}))
            .await
            .unwrap();
        for value in [&decision, &provider, &custom] {
            call(ctx(owner), id(value), "handle.drop", json!({}))
                .await
                .unwrap();
        }
        let resolved = call(
            ctx(owner),
            id(&wrapped),
            "provider.model",
            json!({"modelId":"alias"}),
        )
        .await
        .unwrap();
        let Entry::Decision(model) = get(owner, &resolved, HandleKind::Decision) else {
            panic!()
        };
        assert!(Arc::ptr_eq(&original, &model));
        assert_eq!(model.model_id(), "original");
        let fallback = call(
            ctx(owner),
            id(&wrapped),
            "provider.model",
            json!({"modelId":"fallback"}),
        )
        .await
        .unwrap();
        let Entry::Decision(model) = get(owner, &fallback, HandleKind::Decision) else {
            panic!()
        };
        assert_eq!(model.model_id(), "fallback");
        HandleRegistry::global().release_owner(owner);
    }
}

#[tokio::test]
async fn jev_preserves_constructor_errors_and_rejects_unavailable_settings() {
    let owner = Owner::new_node().unwrap();
    for settings in [
        json!({"apiKey":"x", "probabilitySource":"unknown"}),
        json!({"apiKey":"x", "baseUrl":"http://localhost:1"}),
    ] {
        let error = call(
            ctx(owner),
            HandleId(0),
            "provider.create",
            json!({"package":"jev","settings":settings}),
        )
        .await
        .unwrap_err();
        assert_eq!(error.code(), 5);
        assert!(matches!(
            error.unprojected(),
            OpsError::AiMux(AiMuxError::InvalidArgument(_))
        ));
    }
    let provider = jev(owner, "native").await;
    let error = call(
        ctx(owner),
        id(&provider),
        "provider.model",
        json!({"method":"decision", "modelId":" "}),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), 5);
    for method in ["decisionModel", "Decision", "skills"] {
        assert_eq!(
            call(
                ctx(owner),
                id(&provider),
                "provider.model",
                json!({"method":method,"modelId":"x"})
            )
            .await
            .unwrap_err()
            .code(),
            8
        );
    }
    assert_eq!(
        call(
            ctx(owner),
            HandleId(0),
            "provider.create",
            json!({"package":"openai","settings":{"endpoint":"x"}})
        )
        .await
        .unwrap_err()
        .code(),
        5
    );
    HandleRegistry::global().release_owner(owner);
}

#[tokio::test]
async fn every_secondary_slot_hides_foreign_owner_and_matches_unknown() {
    let owner = Owner::new_node().unwrap();
    let foreign_owner = Owner::new_python().unwrap();
    let foreign = insert(
        foreign_owner,
        Entry::Provider(Arc::new(custom_provider(CustomProviderOptions::default()))),
    );
    let primary = insert(
        owner,
        Entry::Provider(Arc::new(custom_provider(CustomProviderOptions::default()))),
    );
    let operations: &[(&str, &[&str])] = &[
        (
            "provider.create",
            &[
                "settings.fetch",
                "settings.webSocket",
                "settings.credentialResolver",
            ],
        ),
        (
            "provider.wrap",
            &["languageModelMiddleware.*", "imageModelMiddleware.*"],
        ),
        (
            "provider.custom",
            &[
                "languageModels.*",
                "embeddingModels.*",
                "imageModels.*",
                "speechModels.*",
                "transcriptionModels.*",
                "rerankingModels.*",
                "videoModels.*",
                "searchModels.*",
                "decisionModels.*",
                "files",
                "fallbackProvider",
                "languageModelMiddleware.*",
                "imageModelMiddleware.*",
            ],
        ),
        ("provider.invoke", &["abort"]),
    ];
    for &(name, paths) in operations {
        let descriptor = descriptor(name).unwrap();
        assert_eq!(
            descriptor
                .runtime_handle_fields
                .iter()
                .map(|(path, _)| *path)
                .collect::<Vec<_>>(),
            paths
        );
        for &path in paths {
            let make = |handle: String| {
                let mut value = if name == "provider.create" {
                    json!({"package":"openai"})
                } else if name == "provider.invoke" {
                    json!({"method":"listModels"})
                } else {
                    json!({})
                };
                let parts: Vec<_> = path.split('.').collect();
                if parts.len() == 1 {
                    value[parts[0]] = json!(handle);
                } else if parts[1] == "*" {
                    value[parts[0]] = if parts[0].ends_with("Middleware") {
                        json!([handle])
                    } else {
                        json!({"alias":handle})
                    };
                } else {
                    value[parts[0]] = json!({(parts[1]):handle});
                }
                value
            };
            let handle = if descriptor.allowed_handle_kinds.is_empty() {
                HandleId(0)
            } else {
                primary
            };
            let foreign_error = call(ctx(owner), handle, name, make(foreign.0.to_string()))
                .await
                .unwrap_err()
                .envelope(SystemTime::UNIX_EPOCH);
            let unknown_error = call(ctx(owner), handle, name, make(u64::MAX.to_string()))
                .await
                .unwrap_err()
                .envelope(SystemTime::UNIX_EPOCH);
            assert_eq!(foreign_error.code, 203, "{name}:{path}");
            assert_eq!(
                serde_json::to_value(foreign_error).unwrap(),
                serde_json::to_value(unknown_error).unwrap(),
                "{name}:{path}"
            );
        }
    }
    HandleRegistry::global().release_owner(owner);
    HandleRegistry::global().release_owner(foreign_owner);
}

struct Discovery(Arc<AtomicUsize>);
impl Provider for Discovery {
    fn language_model(&self, id: &str) -> Result<Arc<dyn aimux_core::LanguageModel>, AiMuxError> {
        Err(AiMuxError::no_such_model(id, "languageModel"))
    }
    fn embedding_model(
        &self,
        id: &str,
    ) -> Result<Arc<dyn aimux_core::embedding_model::EmbeddingModel>, AiMuxError> {
        Err(AiMuxError::no_such_model(id, "embeddingModel"))
    }
    fn image_model(
        &self,
        id: &str,
    ) -> Result<Arc<dyn aimux_core::image_model::ImageModel>, AiMuxError> {
        Err(AiMuxError::no_such_model(id, "imageModel"))
    }
    fn discovery(&self) -> Option<&dyn ProviderDiscovery> {
        Some(self)
    }
}
impl ProviderDiscovery for Discovery {
    fn list_models(&self) -> BoxFuture<'_, Result<Vec<RuntimeModel>, AiMuxError>> {
        Box::pin(async move {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(vec![])
        })
    }
}

#[tokio::test]
async fn invoke_resolves_abort_before_any_discovery_call() {
    let owner = Owner::new_node().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let provider = insert(owner, Entry::Provider(Arc::new(Discovery(calls.clone()))));
    let signal = aimux_core::AbortSignal::new();
    signal.abort();
    let abort = insert(owner, Entry::Abort(Arc::new(signal)));
    let error = call(
        ctx(owner),
        provider,
        "provider.invoke",
        json!({"method":"listModels", "abort":abort}),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), 13);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        call(
            ctx(owner),
            provider,
            "provider.invoke",
            json!({"method":"listModels"})
        )
        .await
        .unwrap(),
        json!([])
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    HandleRegistry::global().release_owner(owner);
}

#[tokio::test]
async fn handler_errors_are_projected_once_at_dispatch() {
    static CLOCK: AtomicUsize = AtomicUsize::new(0);
    fn now() -> SystemTime {
        CLOCK.fetch_add(1, Ordering::SeqCst);
        SystemTime::UNIX_EPOCH
    }
    let owner = Owner::new_node().unwrap();
    let provider = insert(
        owner,
        Entry::Provider(Arc::new(custom_provider(CustomProviderOptions::default()))),
    );
    let mut context = ctx(owner);
    context.now = now;
    let error = call(
        context,
        provider,
        "provider.model",
        json!({"method":"unknown","modelId":"x"}),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, OpsError::Projected(_)));
    assert_eq!(CLOCK.load(Ordering::SeqCst), 1);
    let envelope = error.envelope(SystemTime::UNIX_EPOCH + Duration::from_secs(999));
    assert_eq!(envelope.kind, aimux_ops::envelope::ErrorDomain::AiMux);
    assert_eq!(envelope.code, 8);
    assert_eq!(envelope.error["name"], "AI_UnsupportedFunctionalityError");
    assert_eq!(CLOCK.load(Ordering::SeqCst), 1);
    HandleRegistry::global().release_owner(owner);
}

#[tokio::test]
async fn strict_controls_reject_legacy_aliases_and_host_objects() {
    let owner = Owner::new_node().unwrap();
    let provider = insert(
        owner,
        Entry::Provider(Arc::new(custom_provider(CustomProviderOptions::default()))),
    );
    for (name, params) in [
        (
            "provider.create",
            json!({"package":"openai","settings":{"base_url":"x"}}),
        ),
        ("provider.default", json!({"package":"openai","unknown":1})),
        (
            "provider.model",
            json!({"method":"languageModel","modelId":"x","unknown":1}),
        ),
        (
            "provider.invoke",
            json!({"method":"listModels","callback":{}}),
        ),
        (
            "provider.wrap",
            json!({"languageModelMiddleware":[{"callback":{}}]}),
        ),
        (
            "provider.custom",
            json!({"decisionModels":{"x":"18446744073709551616"}}),
        ),
    ] {
        let descriptor = descriptor(name).unwrap();
        let target = if descriptor.allowed_handle_kinds.is_empty() {
            HandleId(0)
        } else {
            provider
        };
        assert_eq!(
            call(ctx(owner), target, name, params)
                .await
                .unwrap_err()
                .code(),
            301,
            "{name}"
        );
    }
    HandleRegistry::global().release_owner(owner);
}
