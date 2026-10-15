//! Real modality registration, owner-aware abort lifting and cached projection.
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use aimux_core::error::ApiCallError;
use aimux_core::search_model::{SearchCallOptions, SearchModel, SearchResult};
use aimux_core::{AbortSignal, AiMuxError};
use aimux_ops::dispatch::{self, Ctx};
use aimux_ops::envelope::{BindingError, OpsError};
use aimux_ops::handle::{Entry, HandleId, HandleKind, HandleRegistry, Owner};
use aimux_ops::op::{Cancellation, RequestBytePolicy, Scope};
use serde_json::{Value, json};
use tokio::sync::Notify;

enum Behavior {
    Ready,
    ApiError,
    Pending,
}

struct Search {
    behavior: Behavior,
    calls: Mutex<Vec<(Value, Option<AbortSignal>)>>,
    started: Notify,
}

impl SearchModel for Search {
    fn provider(&self) -> &str {
        "mock"
    }
    fn model_id(&self) -> &str {
        "search"
    }
    fn do_search<'life0, 'life1, 'async_trait>(
        &'life0 self,
        options: &'life1 SearchCallOptions,
    ) -> Pin<Box<dyn Future<Output = Result<SearchResult, AiMuxError>> + Send + 'async_trait>>
    where
        'life0: 'async_trait,
        'life1: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async move {
            self.calls.lock().unwrap().push((
                serde_json::to_value(options).unwrap(),
                options.abort_signal.clone(),
            ));
            self.started.notify_one();
            match self.behavior {
                Behavior::Ready => {
                    Ok(serde_json::from_value(json!({"results": [], "warnings": []})).unwrap())
                }
                Behavior::Pending => std::future::pending().await,
                Behavior::ApiError => {
                    let mut error = ApiCallError::new("limited", "https://mock.test", json!({}));
                    error.status_code = Some(429);
                    error.is_retryable = true;
                    error.response_headers = Some(std::collections::HashMap::from([(
                        "retry-after".into(),
                        "Tue, 14 Nov 2023 22:13:25 GMT".into(),
                    )]));
                    Err(AiMuxError::ApiCall(Box::new(error)))
                }
            }
        })
    }
}

fn now() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000)
}

struct Fixture {
    owner: Owner,
    foreign: Owner,
    handle: HandleId,
    model: Arc<Search>,
}

impl Fixture {
    fn new(behavior: Behavior) -> Self {
        let owner = Owner::new_node().unwrap();
        let foreign = Owner::new_python().unwrap();
        let model = Arc::new(Search {
            behavior,
            calls: Mutex::new(vec![]),
            started: Notify::new(),
        });
        let handle = HandleRegistry::global()
            .insert(owner, Entry::Search(model.clone()))
            .unwrap();
        Self {
            owner,
            foreign,
            handle,
            model,
        }
    }

    fn ctx(&self) -> Ctx {
        Ctx {
            owner: self.owner,
            abort: None,
            now,
        }
    }

    fn abort(&self, owner: Owner, signal: &AbortSignal) -> HandleId {
        HandleRegistry::global()
            .insert(owner, Entry::Abort(Arc::new(signal.clone())))
            .unwrap()
    }

    async fn call(&self, params: Value) -> Result<Value, OpsError> {
        dispatch::call(self.ctx(), self.handle, "search.search", params).await
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        HandleRegistry::global().release_owner(self.owner);
        HandleRegistry::global().release_owner(self.foreign);
    }
}

#[test]
fn manifest_uses_canonical_paths_and_complete_modality_metadata() {
    for (name, module, prefix, kind, bytes) in [
        (
            "embedding.embed",
            "embedding",
            "Embedding",
            HandleKind::Embedding,
            false,
        ),
        (
            "embedding.embedMany",
            "embedding",
            "Embedding",
            HandleKind::Embedding,
            false,
        ),
        ("image.generate", "image", "Image", HandleKind::Image, true),
        (
            "speech.generate",
            "speech",
            "Speech",
            HandleKind::Speech,
            false,
        ),
        (
            "transcription.generate",
            "transcription",
            "Transcription",
            HandleKind::Transcription,
            true,
        ),
        (
            "reranking.rerank",
            "reranking",
            "Reranking",
            HandleKind::Reranking,
            false,
        ),
        ("video.generate", "video", "Video", HandleKind::Video, true),
        (
            "search.search",
            "search",
            "Search",
            HandleKind::Search,
            false,
        ),
    ] {
        let descriptor = dispatch::descriptor(name).unwrap();
        assert_eq!(descriptor.allowed_handle_kinds, &[kind]);
        assert_eq!(
            descriptor.request_type,
            format!("aimux_core::{module}_model::{prefix}CallOptions")
        );
        assert_eq!(
            descriptor.result_type,
            format!("aimux_core::{module}_model::{prefix}Result")
        );
        assert_eq!(
            descriptor.runtime_handle_fields,
            &[("abort", &[HandleKind::Abort][..])]
        );
        assert_eq!(descriptor.scope, Scope::Current);
        assert_eq!(descriptor.cancellation, Cancellation::Abort);
        assert_eq!(
            descriptor.request_byte_policy,
            if bytes {
                RequestBytePolicy::JsonWithBase64
            } else {
                RequestBytePolicy::Json
            }
        );
        assert!(descriptor.required_capabilities.is_empty());
        assert!(descriptor.event_type.is_none());
        assert!(descriptor.terminal_type.is_none());
        assert_eq!(descriptor.result_ownership, "owned");
        assert_eq!(descriptor.retry_owner, "core");
        let manifest = aimux_ops::manifest::manifest();
        for ty in [descriptor.request_type, descriptor.result_type] {
            let found = manifest["types"]
                .as_array()
                .unwrap()
                .iter()
                .find(|entry| entry["id"] == ty)
                .unwrap();
            assert_eq!(found["rustPath"], ty);
        }
    }
}

#[tokio::test]
async fn abort_is_optional_and_owned_signal_is_lifted_without_entering_core_json() {
    let f = Fixture::new(Behavior::Ready);
    for params in [json!({"query":"q"}), json!({"query":"q", "abort":null})] {
        f.call(params).await.unwrap();
    }
    let signal = AbortSignal::new();
    let abort = f.abort(f.owner, &signal);
    let mut params = json!({"query":"q", "abort":abort, "maxRetries":0,
        "headers":{"x-test":"value"}, "providerOptions":{"mock":{"open":{"abort":"data", "nested":[null,"值","🚀"]}}}});
    f.call(params.clone()).await.unwrap();
    params.as_object_mut().unwrap().remove("abort");
    signal.abort();
    let calls = f.model.calls.lock().unwrap();
    assert_eq!(calls.len(), 3);
    assert!(calls[0].1.is_none());
    assert!(calls[1].1.is_none());
    assert_eq!(calls[2].0, params);
    assert!(calls[2].1.as_ref().unwrap().is_aborted());
}

#[tokio::test]
async fn primary_and_secondary_foreign_handles_match_unknown_and_never_call_provider() {
    let f = Fixture::new(Behavior::Ready);
    let signal = AbortSignal::new();
    let foreign_abort = f.abort(f.foreign, &signal);
    let mut errors = vec![];
    for abort in [foreign_abort, HandleId(u64::MAX)] {
        let error = f
            .call(json!({"query":"q", "abort":abort}))
            .await
            .unwrap_err();
        assert!(matches!(
            error.unprojected(),
            OpsError::Binding(BindingError::InvalidHandle { actual: None, .. })
        ));
        errors.push(error.envelope(now()));
    }
    assert_eq!(errors[0], errors[1]);
    let wrong_kind = f
        .call(json!({"query":"q", "abort":f.handle}))
        .await
        .unwrap_err();
    assert!(matches!(
        wrong_kind.unprojected(),
        OpsError::Binding(BindingError::InvalidHandle {
            actual: Some(HandleKind::Search),
            ..
        })
    ));
    for invalid in [
        json!(7),
        json!("+7"),
        json!("18446744073709551616"),
        json!({}),
    ] {
        assert_eq!(
            f.call(json!({"query":"q", "abort":invalid}))
                .await
                .unwrap_err()
                .code(),
            301
        );
    }
    let mut foreign_ctx = f.ctx();
    foreign_ctx.owner = f.foreign;
    let foreign = dispatch::call(
        foreign_ctx.clone(),
        f.handle,
        "search.search",
        json!({"query":"q"}),
    )
    .await
    .unwrap_err();
    let unknown = dispatch::call(
        foreign_ctx,
        HandleId(u64::MAX),
        "search.search",
        json!({"query":"q"}),
    )
    .await
    .unwrap_err();
    assert_eq!(foreign.envelope(now()), unknown.envelope(now()));
    assert!(f.model.calls.lock().unwrap().is_empty());
    assert!(!signal.is_aborted());
}

#[tokio::test]
async fn owned_abort_cancels_before_and_during_provider_call() {
    let f = Fixture::new(Behavior::Pending);
    let signal = AbortSignal::new();
    let abort = f.abort(f.owner, &signal);
    signal.abort();
    assert_eq!(
        f.call(json!({"query":"q", "abort":abort}))
            .await
            .unwrap_err()
            .code(),
        13
    );
    assert!(f.model.calls.lock().unwrap().is_empty());

    let signal = AbortSignal::new();
    let abort = f.abort(f.owner, &signal);
    let future = f.call(json!({"query":"q", "abort":abort}));
    let cancel = async {
        f.model.started.notified().await;
        signal.abort();
    };
    let (result, ()) = tokio::time::timeout(Duration::from_secs(2), async {
        tokio::join!(future, cancel)
    })
    .await
    .unwrap();
    assert_eq!(result.unwrap_err().code(), 13);
    let calls = f.model.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].1.as_ref().unwrap().is_aborted());
}

#[tokio::test]
async fn provider_failure_keeps_payload_and_one_retry_hint_projection() {
    let f = Fixture::new(Behavior::ApiError);
    let error = f
        .call(json!({"query":"q", "maxRetries":0}))
        .await
        .unwrap_err();
    assert!(matches!(error, OpsError::Projected(_)));
    assert!(matches!(
        error.unprojected(),
        OpsError::AiMux(AiMuxError::ApiCall(_))
    ));
    let envelope = error.envelope(now());
    assert_eq!(envelope.code, 11);
    assert_eq!(envelope.error["name"], "AI_APICallError");
    assert_eq!(envelope.error["statusCode"], 429);
    assert_eq!(envelope.error["isRetryable"], true);
    assert_eq!(envelope.retry_after_ms, Some(5000));
    assert_eq!(error.envelope(now() + Duration::from_secs(60)), envelope);
    assert_eq!(f.model.calls.lock().unwrap().len(), 1);
}
