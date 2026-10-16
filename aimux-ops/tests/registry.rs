//! Registry-family integration through the actual dispatcher and owner registry.
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use aimux_core::AiMuxError;
use aimux_core::custom_provider::{CustomProviderOptions, custom_provider};
use aimux_core::decision_model::{
    DecisionCallOptions, DecisionModel, DecisionQuestionType, DecisionResult,
};
use aimux_core::image_model::{ImageCallOptions, ImageModel, ImageResult};
use aimux_core::image_model_middleware::ImageModelMiddleware;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_middleware::LanguageModelMiddleware;
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateResult, StreamResult};
use aimux_ops::dispatch::{Ctx, call};
use aimux_ops::envelope::OpsError;
use aimux_ops::handle::{Entry, HandleId, HandleKind, HandleRegistry, Middleware, Owner};
use futures::future::BoxFuture;
use serde_json::{Value, json};

struct Model;
impl LanguageModel for Model {
    fn provider(&self) -> &str {
        "fixture"
    }
    fn model_id(&self) -> &str {
        "actual:model:id"
    }
    fn do_generate<'a, 'b, 'f>(
        &'a self,
        _: &'b CallOptions,
    ) -> BoxFuture<'f, Result<GenerateResult, AiMuxError>>
    where
        'a: 'f,
        'b: 'f,
        Self: 'f,
    {
        Box::pin(async { Err(AiMuxError::Other("generation reached".into())) })
    }
    fn do_stream<'a, 'b, 'f>(
        &'a self,
        _: &'b CallOptions,
    ) -> BoxFuture<'f, Result<StreamResult, AiMuxError>>
    where
        'a: 'f,
        'b: 'f,
        Self: 'f,
    {
        Box::pin(async { Err(AiMuxError::Other("stream reached".into())) })
    }
}
impl ImageModel for Model {
    fn provider(&self) -> &str {
        "fixture"
    }
    fn model_id(&self) -> &str {
        "actual:model:id"
    }
    fn max_images_per_call(&self) -> Option<u32> {
        Some(1)
    }
    fn do_generate<'a, 'b, 'f>(
        &'a self,
        _: &'b ImageCallOptions,
    ) -> BoxFuture<'f, Result<ImageResult, AiMuxError>>
    where
        'a: 'f,
        'b: 'f,
        Self: 'f,
    {
        Box::pin(async { Err(AiMuxError::Other("image reached".into())) })
    }
}
impl DecisionModel for Model {
    fn provider(&self) -> &str {
        "fixture"
    }
    fn model_id(&self) -> &str {
        "actual:model:id"
    }
    fn supported_question_types(&self) -> &[DecisionQuestionType] {
        &[DecisionQuestionType::Boolean]
    }
    fn do_decide<'a, 'b, 'f>(
        &'a self,
        _: &'b DecisionCallOptions,
    ) -> BoxFuture<'f, Result<DecisionResult, AiMuxError>>
    where
        'a: 'f,
        'b: 'f,
        Self: 'f,
    {
        Box::pin(async { Err(AiMuxError::Other("decision reached".into())) })
    }
}

struct Rename(&'static str, Arc<AtomicUsize>);
impl LanguageModelMiddleware for Rename {
    fn override_model_id(&self, _: &dyn LanguageModel) -> Option<String> {
        self.1.fetch_add(1, Ordering::SeqCst);
        Some(self.0.into())
    }
}
impl ImageModelMiddleware for Rename {
    fn override_model_id(&self, _: &dyn ImageModel) -> Option<String> {
        self.1.fetch_add(1, Ordering::SeqCst);
        Some(self.0.into())
    }
    fn override_max_images_per_call(&self, _: &dyn ImageModel) -> Option<u32> {
        Some(3)
    }
    fn override_supports_file_inputs(&self, _: &dyn ImageModel) -> Option<Option<bool>> {
        Some(Some(true))
    }
}

struct Env(Owner);
impl Env {
    fn new() -> Self {
        Self(Owner::new_node().unwrap())
    }
    fn insert(&self, entry: Entry) -> HandleId {
        HandleRegistry::global().insert(self.0, entry).unwrap()
    }
    async fn call(&self, id: HandleId, name: &str, params: Value) -> Result<Value, OpsError> {
        call(
            Ctx {
                owner: self.0,
                abort: None,
                now: epoch_clock,
            },
            id,
            name,
            params,
        )
        .await
    }
    fn provider(&self) -> HandleId {
        self.insert(Entry::Provider(Arc::new(custom_provider(
            CustomProviderOptions {
                language_models: HashMap::from([(
                    "alias:with:colon".into(),
                    Arc::new(Model) as Arc<dyn LanguageModel>,
                )]),
                image_models: HashMap::from([(
                    "alias:with:colon".into(),
                    Arc::new(Model) as Arc<dyn ImageModel>,
                )]),
                decision_models: HashMap::from([(
                    "alias:with:colon".into(),
                    Arc::new(Model) as Arc<dyn DecisionModel>,
                )]),
                ..Default::default()
            },
        ))))
    }
    async fn registry(&self, provider: HandleId) -> HandleId {
        id(&self.call(HandleId(0), "registry.create", json!({"namespace":"tenant", "entries":[{"key":"key:with:colon", "provider":provider}]})).await.unwrap())
    }
    async fn resolve(&self, registry: HandleId, method: &str) -> HandleId {
        id(&self
            .call(registry, "model.resolve", reference(method))
            .await
            .unwrap())
    }
    async fn drop_handle(&self, handle: HandleId) {
        self.call(handle, "handle.drop", json!({})).await.unwrap();
    }
}
impl Drop for Env {
    fn drop(&mut self) {
        HandleRegistry::global().release_owner(self.0);
    }
}
fn epoch_clock() -> SystemTime {
    UNIX_EPOCH
}
fn id(result: &Value) -> HandleId {
    serde_json::from_value(result["handle"].clone()).unwrap()
}
fn reference(method: &str) -> Value {
    json!({"modelRef":{"namespace":"tenant", "key":"key:with:colon", "method":method, "modelId":"alias:with:colon"}})
}

#[tokio::test]
async fn decision_resolves_wraps_and_survives_parent_drop() {
    let env = Env::new();
    let provider = env.provider();
    let registry = env.registry(provider).await;
    let returned = env
        .call(
            registry,
            "registry.provider",
            json!({"key":"key:with:colon"}),
        )
        .await
        .unwrap();
    let original = HandleRegistry::global()
        .get(env.0, provider, &[HandleKind::Provider])
        .unwrap();
    let copy = HandleRegistry::global()
        .get(env.0, id(&returned), &[HandleKind::Provider])
        .unwrap();
    let (Entry::Provider(a), Entry::Provider(b)) = (original, copy) else {
        panic!()
    };
    assert!(Arc::ptr_eq(&a, &b));
    let model = env.resolve(registry, "decision").await;
    let wrapped = env
        .call(model, "model.wrap", json!({"middleware":[]}))
        .await
        .unwrap();
    assert_eq!(wrapped["kind"], "Decision");
    assert_ne!(id(&wrapped), model);
    env.drop_handle(provider).await;
    env.drop_handle(id(&returned)).await;
    env.drop_handle(registry).await;
    env.drop_handle(model).await;
    let info = env
        .call(id(&wrapped), "model.info", json!({}))
        .await
        .unwrap();
    assert_eq!(info["provider"], "fixture");
    assert_eq!(info["modelId"], "actual:model:id");
    assert_eq!(info["modality"], "decision");
    assert_eq!(
        info["capabilities"],
        json!({"decide":true,"supportedQuestionTypes":["boolean"]})
    );
    assert_eq!(
        info["source"],
        json!({"references":[reference("decision")["modelRef"]]})
    );
    assert_eq!(info.as_object().unwrap().len(), 5);
}

#[tokio::test]
async fn language_and_image_middleware_use_core_order_and_keep_source() {
    for (method, kind) in [
        ("languageModel", HandleKind::Language),
        ("imageModel", HandleKind::Image),
    ] {
        let env = Env::new();
        let provider = env.provider();
        let registry = env.registry(provider).await;
        let model = env.resolve(registry, method).await;
        let calls = Arc::new(AtomicUsize::new(0));
        let middleware: Vec<_> = ["outer", "inner"]
            .into_iter()
            .map(|name| {
                let value = Arc::new(Rename(name, calls.clone()));
                env.insert(Entry::Middleware(Arc::new(
                    if kind == HandleKind::Language {
                        Middleware::Language(value)
                    } else {
                        Middleware::Image(value)
                    },
                )))
            })
            .collect();
        let wrapped = env
            .call(model, "model.wrap", json!({"middleware":middleware}))
            .await
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        env.drop_handle(model).await;
        env.drop_handle(registry).await;
        env.drop_handle(provider).await;
        for middleware in middleware {
            env.drop_handle(middleware).await;
        }
        let info = env
            .call(id(&wrapped), "model.info", json!({}))
            .await
            .unwrap();
        assert_eq!(info["modelId"], "outer");
        assert_eq!(
            info["source"],
            json!({"references":[reference(method)["modelRef"]]})
        );
        let entry = HandleRegistry::global()
            .get(env.0, id(&wrapped), &[kind])
            .unwrap();
        match entry {
            Entry::Language(model) => {
                assert_eq!(
                    model
                        .do_generate(&CallOptions::default())
                        .await
                        .unwrap_err()
                        .to_string(),
                    "generation reached"
                );
            }
            Entry::Image(model) => {
                assert_eq!(info["capabilities"]["maxImagesPerCall"], 3);
                assert_eq!(info["capabilities"]["supportsFileInputs"], true);
                assert_eq!(
                    model
                        .do_generate(&ImageCallOptions::new("x"))
                        .await
                        .unwrap_err()
                        .to_string(),
                    "image reached"
                );
            }
            _ => panic!(),
        }
    }
}

#[tokio::test]
async fn secondary_handles_are_owner_checked_without_disclosure() {
    let env = Env::new();
    let other = Env::new();
    let foreign = other.provider();
    let unknown = HandleId(u64::MAX);
    let primary = env.insert(Entry::Language(Arc::new(Model)));
    for name in ["registry.create", "model.wrap"] {
        let params = |handle| {
            if name == "registry.create" {
                json!({"entries":[{"key":"p","provider":handle}]})
            } else {
                json!({"middleware":[handle]})
            }
        };
        let primary = if name == "registry.create" {
            HandleId(0)
        } else {
            primary
        };
        let foreign = env.call(primary, name, params(foreign)).await.unwrap_err();
        let missing = env.call(primary, name, params(unknown)).await.unwrap_err();
        assert_eq!(foreign.code(), 203);
        assert_eq!(foreign.envelope(UNIX_EPOCH), missing.envelope(UNIX_EPOCH));
        assert!(foreign.envelope(UNIX_EPOCH).error.get("actual").is_none());
    }
}

#[tokio::test]
async fn entire_middleware_chain_is_checked_before_any_hook() {
    let env = Env::new();
    let other = Env::new();
    let primary = env.insert(Entry::Language(Arc::new(Model)));
    let calls = Arc::new(AtomicUsize::new(0));
    let good = env.insert(Entry::Middleware(Arc::new(Middleware::Language(Arc::new(
        Rename("good", calls.clone()),
    )))));
    let wrong = env.insert(Entry::Middleware(Arc::new(Middleware::Image(Arc::new(
        Rename("wrong", calls.clone()),
    )))));
    let foreign = other.insert(Entry::Middleware(Arc::new(Middleware::Language(Arc::new(
        Rename("foreign", calls.clone()),
    )))));
    for (bad, code) in [(wrong, 301), (foreign, 203), (HandleId(u64::MAX), 203)] {
        let error = env
            .call(primary, "model.wrap", json!({"middleware":[good,bad]}))
            .await
            .unwrap_err();
        assert_eq!(error.code(), code);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
    let primary = env.insert(Entry::Decision(Arc::new(Model)));
    assert_eq!(
        env.call(primary, "model.wrap", json!({"middleware":[good]}))
            .await
            .unwrap_err()
            .code(),
        8
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn strict_controls_and_reference_errors_cross_projected_boundary() {
    let env = Env::new();
    let provider = env.provider();
    let registry = env.registry(provider).await;
    let model = env.resolve(registry, "languageModel").await;
    let duplicate =
        json!({"entries":[{"key":"p","provider":provider},{"key":"p","provider":provider}]});
    let mut cases = vec![
        (HandleId(0), "registry.create", duplicate, 301),
        (
            HandleId(0),
            "registry.create",
            json!({"entries":[],"unknown":true}),
            301,
        ),
        (
            HandleId(0),
            "registry.create",
            json!({"entries":[{"key":"p","provider":provider,"settings":{}}]}),
            301,
        ),
        (
            HandleId(0),
            "registry.create",
            json!({"entries":[{"key":"","provider":provider}]}),
            301,
        ),
        (registry, "registry.provider", json!({"key":"missing"}), 10),
        (
            registry,
            "registry.provider",
            json!({"key":"key:with:colon","unknown":true}),
            301,
        ),
        (model, "model.info", json!({"settings":true}), 301),
        (
            model,
            "model.wrap",
            json!({"middleware":[],"unknown":true}),
            301,
        ),
    ];
    for method in ["decisionModel", "unknown", "LanguageModel"] {
        cases.push((registry, "model.resolve", reference(method), 301));
    }
    for (field, value, code) in [
        ("namespace", "other", 301),
        ("key", "missing", 10),
        ("modelId", "missing", 9),
    ] {
        let mut params = reference("languageModel");
        params["modelRef"][field] = json!(value);
        cases.push((registry, "model.resolve", params, code));
    }
    let mut extra = reference("languageModel");
    extra["modelRef"]["settings"] = json!({});
    cases.push((registry, "model.resolve", extra, 301));
    for value in [
        json!(provider.0),
        json!("+1"),
        json!("18446744073709551616"),
    ] {
        cases.push((
            HandleId(0),
            "registry.create",
            json!({"entries":[{"key":"p","provider":value}]}),
            301,
        ));
    }
    let middleware = env.insert(Entry::Middleware(Arc::new(Middleware::Language(Arc::new(
        Rename("unused", Arc::new(AtomicUsize::new(0))),
    )))));
    for value in [
        json!(middleware.0),
        json!("+1"),
        json!("18446744073709551616"),
    ] {
        cases.push((model, "model.wrap", json!({"middleware":[value]}), 301));
    }
    for (primary, name, params, code) in cases {
        let error = env.call(primary, name, params.clone()).await.unwrap_err();
        assert_eq!(error.code(), code, "{name}: {params}");
        assert!(matches!(error, OpsError::Projected(_)), "{name}: {params}");
        assert_eq!(
            error.envelope(UNIX_EPOCH),
            error.envelope(SystemTime::now())
        );
    }
}

#[test]
fn descriptors_publish_real_types_and_secondary_fields() {
    for (name, field) in [
        ("registry.create", "entries.*.provider"),
        ("model.wrap", "middleware.*"),
    ] {
        let descriptor = aimux_ops::dispatch::descriptor(name).unwrap();
        assert_eq!(descriptor.runtime_handle_fields[0].0, field);
        assert!(
            descriptor
                .request_type
                .starts_with("aimux_ops::ops::registry::")
        );
        assert!(
            descriptor
                .result_type
                .starts_with("aimux_ops::ops::registry::")
        );
    }
    let manifest = aimux_ops::manifest::manifest();
    for name in [
        "registry.create",
        "registry.provider",
        "model.resolve",
        "model.info",
        "model.wrap",
    ] {
        let descriptor = aimux_ops::dispatch::descriptor(name).unwrap();
        for id in [descriptor.request_type, descriptor.result_type] {
            assert!(
                manifest["types"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|entry| entry["id"] == id && entry["rustPath"] == id)
            );
        }
    }
}
