//! Trace decorators reached through model and provider wrapping.
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateResult, StreamResult};
use aimux_core::trace::RingTraceStore;
use aimux_core::{AiMuxError, LanguageModel};
use aimux_ops::{Ctx, Entry, HandleId, HandleKind, HandleRegistry, Owner, dispatch};
use futures::StreamExt;
use serde_json::{Value, json};
use std::{future::Future, pin::Pin, sync::Arc};

struct Model;
impl Model {
    fn result() -> GenerateResult {
        serde_json::from_value(json!({
            "content":[{"type":"text","text":"traced"}],
            "finishReason":{"unified":"stop","raw":"stop"},
            "usage":{"inputTokens":{},"outputTokens":{}}, "warnings":[],
            "request":{"body":"{\"messages\":[{\"role\":\"user\",\"content\":\"hello\"}]}"}
        }))
        .unwrap()
    }
}
impl LanguageModel for Model {
    fn provider(&self) -> &str {
        "openai.chat"
    }
    fn model_id(&self) -> &str {
        "trace-test"
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
        Box::pin(async { Ok(Self::result()) })
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
        Box::pin(async {
            let result = Self::result();
            Ok(StreamResult {
                request: result.request,
                response: None,
                stream: Box::pin(futures::stream::iter([Ok(
                    aimux_core::stream_part::StreamPart::Finish {
                        finish_reason: result.finish_reason,
                        usage: result.usage,
                        provider_metadata: None,
                    },
                )])),
            })
        })
    }
}

fn context(owner: Owner) -> Ctx {
    Ctx { owner, abort: None }
}
async fn call(owner: Owner, handle: HandleId, op: &str, params: Value) -> Value {
    dispatch::call(context(owner), handle, op, params)
        .await
        .unwrap()
}
fn model(owner: Owner, id: HandleId) -> Arc<dyn LanguageModel> {
    let Entry::Language(model) = HandleRegistry::global()
        .get(owner, id, &[HandleKind::Language])
        .unwrap()
    else {
        panic!("language model")
    };
    model
}
fn store(owner: Owner, id: HandleId) -> Arc<RingTraceStore> {
    let Entry::TraceStore(store) = HandleRegistry::global()
        .get(owner, id, &[HandleKind::TraceStore])
        .unwrap()
    else {
        panic!("store")
    };
    store
}

fn records(sink: &RingTraceStore) -> Vec<aimux_core::trace::TraceRecord> {
    let mut bytes = Vec::new();
    sink.export_jsonl(&mut bytes).unwrap();
    String::from_utf8(bytes)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[tokio::test]
async fn model_and_provider_wrap_trace_generation_streams_and_retain_dependencies() {
    for mode in ["off", "shared", "strict"] {
        let owner = Owner::new_node().unwrap();
        let registry = HandleRegistry::global();
        let trace: HandleId = serde_json::from_value(
            call(
                owner,
                HandleId(0),
                "trace.create",
                json!({"auditMode":mode}),
            )
            .await,
        )
        .unwrap();
        let sink = store(owner, trace);
        let middleware = call(
            owner,
            HandleId(0),
            "middleware.create",
            json!({"kind":"trace","traceStore":trace,"scopeName":"endpoint-a"}),
        )
        .await;
        let original = registry
            .insert(owner, Entry::Language(Arc::new(Model)))
            .unwrap();
        let wrapped = call(
            owner,
            original,
            "model.wrap",
            json!({"middleware":[middleware["handle"]]}),
        )
        .await;
        let wrapped_id = serde_json::from_value(wrapped["handle"].clone()).unwrap();
        let provider = aimux_core::custom_provider::custom_provider(
            aimux_core::custom_provider::CustomProviderOptions {
                language_models: [(
                    "trace-test".into(),
                    Arc::new(Model) as Arc<dyn LanguageModel>,
                )]
                .into_iter()
                .collect(),
                ..Default::default()
            },
        );
        let provider_id = registry
            .insert(owner, Entry::Provider(Arc::new(provider)))
            .unwrap();
        let provider = call(
            owner,
            provider_id,
            "provider.wrap",
            json!({"languageModelMiddleware":[middleware["handle"]]}),
        )
        .await;
        let provider_id = serde_json::from_value(provider["handle"].clone()).unwrap();
        let resolved = call(
            owner,
            provider_id,
            "provider.model",
            json!({"method":"languageModel","modelId":"trace-test"}),
        )
        .await;
        let resolved_id = serde_json::from_value(resolved["handle"].clone()).unwrap();
        let middleware_id = serde_json::from_value(middleware["handle"].clone()).unwrap();
        assert!(registry.remove(owner, middleware_id).is_some());
        assert!(registry.remove(owner, trace).is_some());
        assert!(registry.remove(owner, original).is_some());
        assert!(registry.remove(owner, provider_id).is_some());
        let options = CallOptions::new(vec![]);
        model(owner, wrapped_id)
            .do_generate(&options)
            .await
            .unwrap();
        let mut stream = model(owner, resolved_id)
            .do_stream(&options)
            .await
            .unwrap()
            .stream;
        while let Some(part) = stream.next().await {
            part.unwrap();
        }
        drop(stream);
        let records = records(&sink);
        assert_eq!(records.len(), 2, "{mode}");
        assert!(
            records
                .iter()
                .all(|record| record.verdict.is_some() == (mode != "off"))
        );
        assert!(
            records
                .iter()
                .all(|record| record.provider == "openai.chat")
        );
        registry.release_owner(owner);
        assert_eq!(Arc::strong_count(&sink), 1);
    }
}

#[tokio::test]
async fn explicit_disabled_audit_overrides_store_policy() {
    let owner = Owner::new_node().unwrap();
    let trace = call(
        owner,
        HandleId(0),
        "trace.create",
        json!({"auditMode":"strict"}),
    )
    .await;
    let trace = serde_json::from_value(trace).unwrap();
    let sink = store(owner, trace);
    let middleware = call(
        owner,
        HandleId(0),
        "middleware.create",
        json!({"kind":"trace","traceStore":trace,"auditMode":"disabled"}),
    )
    .await;
    let original = HandleRegistry::global()
        .insert(owner, Entry::Language(Arc::new(Model)))
        .unwrap();
    let wrapped = call(
        owner,
        original,
        "model.wrap",
        json!({"middleware":[middleware["handle"]]}),
    )
    .await;
    model(
        owner,
        serde_json::from_value(wrapped["handle"].clone()).unwrap(),
    )
    .do_generate(&CallOptions::new(vec![]))
    .await
    .unwrap();
    assert_eq!(records(&sink).len(), 1);
    assert!(records(&sink)[0].verdict.is_none());
    HandleRegistry::global().release_owner(owner);
}
