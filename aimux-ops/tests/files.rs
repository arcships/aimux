//! Real-dispatch files-family coverage; no network or FFI runtime.
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use aimux_core::files_model::{Files, UploadFileCallOptions, UploadFileResult};
use aimux_core::provider::Provider;
use aimux_core::{AbortSignal, AiMuxError};
use aimux_ops::dispatch::{self, Ctx};
use aimux_ops::envelope::{BindingError, OpsError};
use aimux_ops::handle::{Entry, HandleId, HandleKind, HandleRegistry, Owner};
use aimux_ops::op::{Cancellation, RequestBytePolicy};
use serde_json::{Value, json};
use tokio::sync::Notify;

#[derive(Default)]
enum Reply {
    #[default]
    Success,
    RetryableError,
    WaitForAbort,
}

#[derive(Default)]
struct MockFiles {
    calls: Mutex<Vec<UploadFileCallOptions>>,
    reply: Reply,
    started: Notify,
}

impl Files for MockFiles {
    fn provider(&self) -> &str {
        "mock"
    }

    fn upload_file<'a, 'b, 'f>(
        &'a self,
        options: &'b UploadFileCallOptions,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<UploadFileResult, AiMuxError>> + Send + 'f>,
    >
    where
        'a: 'f,
        'b: 'f,
        Self: 'f,
    {
        Box::pin(async move {
            self.calls.lock().unwrap().push(options.clone());
            self.started.notify_one();
            match self.reply {
                Reply::Success => Ok(UploadFileResult {
                    provider_reference: [("mock".into(), "file-1".into())].into(),
                    media_type: Some(options.media_type.clone()),
                    filename: options.filename.clone(),
                    provider_metadata: Some(
                        serde_json::from_value(json!({"mock":{"detail":null}})).unwrap(),
                    ),
                    warnings: vec![],
                }),
                Reply::RetryableError => {
                    let mut error = aimux_core::error::ApiCallError::new(
                        "upload failed",
                        "https://example.invalid/upload",
                        json!({"data":"redacted"}),
                    );
                    error.is_retryable = true;
                    error.status_code = Some(503);
                    error.response_headers = Some(
                        [("retry-after".into(), "Tue, 14 Nov 2023 22:13:30 GMT".into())].into(),
                    );
                    Err(AiMuxError::ApiCall(Box::new(error)))
                }
                Reply::WaitForAbort => {
                    options.abort_signal.as_ref().unwrap().cancelled().await;
                    Err(AiMuxError::Aborted("provider cancelled".into()))
                }
            }
        })
    }
}

struct MockProvider {
    files: Option<Arc<dyn Files>>,
    calls: AtomicUsize,
}
impl Provider for MockProvider {
    fn language_model(
        &self,
        _: &str,
    ) -> Result<Arc<dyn aimux_core::language_model::LanguageModel>, AiMuxError> {
        Err(AiMuxError::UnsupportedFunctionality("language".into()))
    }
    fn embedding_model(
        &self,
        _: &str,
    ) -> Result<Arc<dyn aimux_core::embedding_model::EmbeddingModel>, AiMuxError> {
        Err(AiMuxError::UnsupportedFunctionality("embedding".into()))
    }
    fn image_model(
        &self,
        _: &str,
    ) -> Result<Arc<dyn aimux_core::image_model::ImageModel>, AiMuxError> {
        Err(AiMuxError::UnsupportedFunctionality("image".into()))
    }
    fn files(&self) -> Option<Arc<dyn Files>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.files.clone()
    }
}

struct Resources(Owner);
impl Resources {
    fn new() -> Self {
        Self(Owner::new_node().unwrap())
    }
    fn insert(&self, entry: Entry) -> HandleId {
        HandleRegistry::global().insert(self.0, entry).unwrap()
    }
}
impl Drop for Resources {
    fn drop(&mut self) {
        HandleRegistry::global().release_owner(self.0);
    }
}

fn ctx(owner: Owner) -> Ctx {
    Ctx { owner, abort: None }
}
fn text_params() -> Value {
    json!({"data":{"type":"text","text":"hello 中文😀"},"mediaType":"text/plain"})
}
fn assert_invalid_handle(error: &OpsError, expected: HandleKind, actual: Option<HandleKind>) {
    match error {
        OpsError::Binding(BindingError::InvalidHandle {
            expected: found,
            actual: kind,
        }) => {
            assert_eq!(found, &[expected]);
            assert_eq!(*kind, actual);
        }
        error => panic!("expected InvalidHandle, got {error}"),
    }
}

#[tokio::test]
async fn provider_files_returns_an_owned_handle_that_survives_parent_drop() {
    let resources = Resources::new();
    let model = Arc::new(MockFiles::default());
    let provider = Arc::new(MockProvider {
        files: Some(model.clone()),
        calls: AtomicUsize::new(0),
    });
    let parent = resources.insert(Entry::Provider(provider.clone()));
    let result = dispatch::call(ctx(resources.0), parent, "provider.files", json!({}))
        .await
        .unwrap();
    assert!(
        result
            .as_str()
            .is_some_and(|id| id.parse::<u64>().unwrap() > 0)
    );
    let child: HandleId = serde_json::from_value(result).unwrap();
    assert!(
        HandleRegistry::global()
            .get(resources.0, child, &[HandleKind::Files])
            .is_ok()
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    dispatch::call(ctx(resources.0), parent, "handle.drop", json!({}))
        .await
        .unwrap();
    drop(provider);
    let result = dispatch::call(ctx(resources.0), child, "files.upload", text_params())
        .await
        .unwrap();
    assert_eq!(result["providerReference"], json!({"mock":"file-1"}));
    assert!(result.get("filename").is_none());
    assert_eq!(model.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn provider_files_validates_controls_and_reports_missing_capability() {
    let resources = Resources::new();
    let provider = Arc::new(MockProvider {
        files: None,
        calls: AtomicUsize::new(0),
    });
    let id = resources.insert(Entry::Provider(provider.clone()));
    for params in [json!({"modelId":"unused"}), Value::Null, json!([])] {
        let error = dispatch::call(ctx(resources.0), id, "provider.files", params)
            .await
            .unwrap_err();
        assert_eq!(error.code(), 301);
    }
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    let error = dispatch::call(ctx(resources.0), id, "provider.files", json!({}))
        .await
        .unwrap_err();
    assert_eq!(error.code(), 8);
    assert!(matches!(
        &error,
        OpsError::AiMux(AiMuxError::UnsupportedFunctionality(_))
    ));
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn uploads_canonical_base64_and_preserves_options_and_context_abort() {
    let resources = Resources::new();
    let model = Arc::new(MockFiles::default());
    let id = resources.insert(Entry::Files(model.clone()));
    let params = json!({
        "data":{"type":"data","data":"AAEC/w=="},
        "mediaType":"application/octet-stream","filename":"测试😀.bin",
        "providerOptions":{"mock":{"custom":true,"nested":{"anything":[null,1]}}}
    });
    let signal = AbortSignal::new();
    let mut context = ctx(resources.0);
    context.abort = Some(signal.clone());
    let result = dispatch::call(context, id, "files.upload", params.clone())
        .await
        .unwrap();
    assert_eq!(
        result,
        json!({
            "providerReference":{"mock":"file-1"},"mediaType":"application/octet-stream",
            "filename":"测试😀.bin","providerMetadata":{"mock":{"detail":null}},"warnings":[]
        })
    );
    signal.abort();
    let calls = model.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].abort_signal.as_ref().unwrap().is_aborted());
    assert_eq!(serde_json::to_value(&calls[0]).unwrap(), params);
}

#[tokio::test]
async fn retryable_upload_error_is_not_retried() {
    let resources = Resources::new();
    let model = Arc::new(MockFiles {
        reply: Reply::RetryableError,
        ..Default::default()
    });
    let id = resources.insert(Entry::Files(model.clone()));
    let error = dispatch::call(ctx(resources.0), id, "files.upload", text_params())
        .await
        .unwrap_err();
    assert!(matches!(&error, OpsError::AiMux(AiMuxError::ApiCall(api))
        if api.is_retryable && api.status_code == Some(503)));
    let envelope = error.envelope();
    assert_eq!(envelope.code, 11);
    assert_eq!(envelope.error["name"], "AI_APICallError");
    assert_eq!(envelope.error["isRetryable"], true);
    assert_eq!(
        envelope.error["responseHeaders"]["retry-after"],
        "Tue, 14 Nov 2023 22:13:30 GMT"
    );
    assert_eq!(model.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn rejects_invalid_controls_before_upload() {
    let resources = Resources::new();
    let model = Arc::new(MockFiles::default());
    let id = resources.insert(Entry::Files(model.clone()));
    for params in [
        json!({"data":{"type":"data","data":[1,2]},"mediaType":"text/plain"}),
        json!({"data":{"type":"text","text":"ok","extra":1},"mediaType":"text/plain"}),
        json!({"data":{"type":"text","text":"ok"},"mediaType":"text/plain","retry":3}),
        json!({"data":{"type":"text","text":"ok"},"media_type":"text/plain"}),
        json!({"data":{"type":"text","text":"ok"},"mediaType":"text/plain","abortSignal":null}),
        json!({"data":{"type":"url","url":"https://example.invalid"},"mediaType":"text/plain"}),
        json!({"data":{"type":"data","data":""},"mediaType":42}),
        json!({"mediaType":"text/plain"}),
        json!({}),
        Value::Null,
        json!([]),
    ] {
        let error = dispatch::call(ctx(resources.0), id, "files.upload", params.clone())
            .await
            .unwrap_err();
        assert_eq!(error.code(), 301, "{params}: {error}");
    }
    assert!(model.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn secondary_abort_is_owner_checked_and_typed_before_upload() {
    let resources = Resources::new();
    let foreign = Resources::new();
    let model = Arc::new(MockFiles::default());
    let id = resources.insert(Entry::Files(model.clone()));
    let foreign_abort = foreign.insert(Entry::Abort(Arc::new(AbortSignal::new())));
    let mut envelopes = Vec::new();
    for (abort, actual) in [
        (foreign_abort, None),
        (HandleId(u64::MAX), None),
        (id, Some(HandleKind::Files)),
    ] {
        let mut params = text_params();
        params["abort"] = json!(abort);
        let error = dispatch::call(ctx(resources.0), id, "files.upload", params)
            .await
            .unwrap_err();
        assert_invalid_handle(&error, HandleKind::Abort, actual);
        envelopes.push(error.envelope());
    }
    assert_eq!(envelopes[0], envelopes[1]);
    for abort in [
        json!(1),
        json!("-1"),
        json!("18446744073709551616"),
        json!("invalid"),
    ] {
        let mut params = text_params();
        params["abort"] = abort;
        let error = dispatch::call(ctx(resources.0), id, "files.upload", params)
            .await
            .unwrap_err();
        assert_eq!(error.code(), 301);
    }
    assert!(model.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn owned_abort_handle_cancels_an_inflight_upload() {
    let resources = Resources::new();
    let model = Arc::new(MockFiles {
        reply: Reply::WaitForAbort,
        ..Default::default()
    });
    let id = resources.insert(Entry::Files(model.clone()));
    let signal = AbortSignal::new();
    let abort = resources.insert(Entry::Abort(Arc::new(signal.clone())));
    let mut params = text_params();
    params["abort"] = json!(abort);
    let upload = tokio::spawn(dispatch::call(ctx(resources.0), id, "files.upload", params));
    tokio::time::timeout(Duration::from_secs(2), model.started.notified())
        .await
        .unwrap();
    signal.abort();
    let error = tokio::time::timeout(Duration::from_secs(2), upload)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(error.code(), 13);
    let calls = model.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].abort_signal.as_ref().unwrap().is_aborted());
}

#[tokio::test]
async fn already_aborted_request_never_calls_provider() {
    let resources = Resources::new();
    let model = Arc::new(MockFiles::default());
    let id = resources.insert(Entry::Files(model.clone()));
    let signal = AbortSignal::new();
    signal.abort();
    let abort = resources.insert(Entry::Abort(Arc::new(signal)));
    let mut params = text_params();
    params["abort"] = json!(abort);
    let error = dispatch::call(ctx(resources.0), id, "files.upload", params)
        .await
        .unwrap_err();
    assert_eq!(error.code(), 13);
    assert!(model.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn primary_handles_reject_global_unknown_dropped_foreign_and_wrong_kind() {
    let resources = Resources::new();
    let foreign = Resources::new();
    let model = Arc::new(MockFiles::default());
    let provider = Arc::new(MockProvider {
        files: Some(model.clone()),
        calls: AtomicUsize::new(0),
    });
    let wrong_kind = resources.insert(Entry::Abort(Arc::new(AbortSignal::new())));
    for (name, expected, entry, params) in [
        (
            "provider.files",
            HandleKind::Provider,
            Entry::Provider(provider.clone()),
            json!({}),
        ),
        (
            "files.upload",
            HandleKind::Files,
            Entry::Files(model.clone()),
            text_params(),
        ),
    ] {
        let dropped = resources.insert(entry.clone());
        drop(HandleRegistry::global().remove(resources.0, dropped));
        let other = foreign.insert(entry);
        let mut envelopes = Vec::new();
        for id in [HandleId(0), HandleId(u64::MAX), dropped, other] {
            let error = dispatch::call(ctx(resources.0), id, name, params.clone())
                .await
                .unwrap_err();
            assert_invalid_handle(&error, expected, None);
            envelopes.push(error.envelope());
        }
        assert!(envelopes.windows(2).all(|pair| pair[0] == pair[1]));
        let error = dispatch::call(ctx(resources.0), wrong_kind, name, params)
            .await
            .unwrap_err();
        assert_invalid_handle(&error, expected, Some(HandleKind::Abort));
    }
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    assert!(model.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn one_shot_text_upload_is_not_capped_by_stream_item_limit() {
    let resources = Resources::new();
    let model = Arc::new(MockFiles::default());
    let id = resources.insert(Entry::Files(model.clone()));
    let mut params = text_params();
    params["data"]["text"] = json!("x".repeat(1024 * 1024 + 1));
    params["abort"] = Value::Null;
    dispatch::call(ctx(resources.0), id, "files.upload", params)
        .await
        .unwrap();
    assert_eq!(model.calls.lock().unwrap().len(), 1);
}

#[test]
fn files_descriptors_use_concrete_types_and_abort_metadata() {
    let provider = dispatch::descriptor("provider.files").unwrap();
    assert_eq!(
        (provider.request_type.id)().into_owned(),
        "aimux_ops::ops::operation::EmptyParams"
    );
    assert_eq!(
        (provider.result_type.id)().into_owned(),
        "aimux_ops::handle::HandleId"
    );
    assert_eq!(provider.required_capabilities, &["files"]);
    let upload = dispatch::descriptor("files.upload").unwrap();
    assert_eq!(
        (upload.request_type.id)().into_owned(),
        "aimux_ops::ops::files::UploadFileParams"
    );
    assert_eq!(
        (upload.result_type.id)().into_owned(),
        "aimux_core::files_model::UploadFileResult"
    );
    assert_eq!(upload.cancellation, Cancellation::Abort);
    assert_eq!(
        upload.runtime_handle_fields,
        &[("abort", &[HandleKind::Abort][..])]
    );
    assert_eq!(
        upload.request_byte_policy,
        RequestBytePolicy::JsonWithBase64
    );
    assert_eq!(upload.retry_owner, "none");
    assert!(upload.side_effects);
    let manifest = aimux_ops::manifest::manifest();
    for id in [
        (upload.request_type.id)().into_owned(),
        (upload.result_type.id)().into_owned(),
    ] {
        let ty = manifest["types"]
            .as_array()
            .unwrap()
            .iter()
            .find(|ty| ty["id"] == id)
            .unwrap();
        assert_eq!(ty["rustPath"], id);
    }
}
