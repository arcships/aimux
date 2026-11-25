//! Table-driven gates over the real manifest and real dispatcher.
//! An isolated registry provides typed mock resources; counting handlers replace
//! business I/O while retaining every production descriptor and validation step.
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use aimux_ops::dispatch::{
    self, CallFuture, Ctx, Dispatcher, EventStream, Handler, ResourceRegistry, Target,
};
use aimux_ops::envelope::{BindingError, OpsError, ProtocolError};
use aimux_ops::handle::{Entry, HandleId, HandleKind, HandleRegistry, Owner};
use aimux_ops::op::OpMode;
use futures::StreamExt;
use serde_json::{Value, json};

#[derive(Default)]
struct Registry {
    entries: Mutex<HashMap<u64, (Owner, HandleKind)>>,
    next: AtomicUsize,
}
impl Registry {
    fn insert(&self, owner: Owner, kind: HandleKind) -> HandleId {
        let id = self.next.fetch_add(1, Ordering::Relaxed) as u64 + 1;
        self.entries.lock().unwrap().insert(id, (owner, kind));
        HandleId(id)
    }
}
impl ResourceRegistry for Registry {
    fn get(&self, owner: Owner, id: HandleId, allowed: &[HandleKind]) -> Result<Entry, OpsError> {
        let entries = self.entries.lock().unwrap();
        let actual = entries
            .get(&id.0)
            .filter(|(found, _)| *found == owner)
            .map(|(_, kind)| *kind);
        if actual.is_some_and(|kind| allowed.contains(&kind)) {
            // Handlers only count admissions. No fake model/provider behaviour.
            Ok(Entry::Abort(Arc::new(aimux_core::AbortSignal::new())))
        } else {
            Err(BindingError::InvalidHandle {
                expected: allowed.to_vec(),
                actual,
            }
            .into())
        }
    }
    fn remove(&self, owner: Owner, id: HandleId) -> Option<Entry> {
        let mut entries = self.entries.lock().unwrap();
        if entries.get(&id.0).is_some_and(|(found, _)| *found == owner) {
            entries.remove(&id.0);
            Some(Entry::Abort(Arc::new(aimux_core::AbortSignal::new())))
        } else {
            None
        }
    }
}
static CALLS: AtomicUsize = AtomicUsize::new(0);
fn probe(_: Ctx, _: Target, _: dispatch::Params<'_>) -> CallFuture<'_> {
    CALLS.fetch_add(1, Ordering::SeqCst);
    Box::pin(async { Ok("null".into()) })
}
fn stream_probe(_: Ctx, _: Target, _: Value) -> Result<EventStream, OpsError> {
    CALLS.fetch_add(1, Ordering::SeqCst);
    Ok(Box::pin(futures::stream::iter([Ok(
        json!({"probe": true}),
    )])))
}
fn ctx(owner: Owner) -> Ctx {
    Ctx { owner, abort: None }
}
fn invalid(
    result: Result<(), OpsError>,
    expected: &[HandleKind],
    actual: Option<HandleKind>,
    label: &str,
) {
    let error = result.expect_err(label);
    match &error {
        OpsError::Binding(BindingError::InvalidHandle {
            expected: found,
            actual: kind,
        }) => {
            assert_eq!(found, expected, "{label}");
            assert_eq!(*kind, actual, "{label}");
        }
        error => panic!("{label}: expected InvalidHandle, got {error}"),
    }
}
async fn invoke(
    dispatcher: &Dispatcher<'_>,
    registry: &Registry,
    owner: Owner,
    id: HandleId,
    name: &str,
    mode: OpMode,
) -> Result<(), OpsError> {
    match mode {
        OpMode::Stream => {
            let frames = dispatcher
                .stream(ctx(owner), id, name, json!({}))?
                .collect::<Vec<_>>()
                .await;
            assert_eq!(frames.len(), 2);
            Ok(())
        }
        OpMode::BinaryPush => {
            // Generic JSON dispatch rejects BinaryPush for every target.
            let error = dispatcher
                .call(ctx(owner), id, name, json!({}))
                .await
                .unwrap_err();
            assert!(matches!(
                &error,
                OpsError::Protocol(ProtocolError::InvalidOp(_))
            ));
            // Native binary admission uses this identical owner-aware typed resolver.
            registry
                .get(owner, id, &[HandleKind::Operation])
                .map(|_| ())
        }
        _ => dispatcher
            .call(ctx(owner), id, name, json!({}))
            .await
            .map(|_| ()),
    }
}

#[tokio::test]
async fn every_manifest_op_crosses_every_kind_and_invalid_handle_state() {
    let manifest = aimux_ops::manifest::manifest();
    let operations = manifest["ops"].as_array().unwrap();
    assert_eq!(
        operations.len(),
        76,
        "RFC inventory plus Decision and provider/registry extensions"
    );
    let table: Vec<_> = operations
        .iter()
        .map(|op| {
            let descriptor = dispatch::descriptor(op["name"].as_str().unwrap()).unwrap();
            let handler = if descriptor.mode == OpMode::Stream {
                Handler::Stream(stream_probe)
            } else {
                Handler::Call(probe)
            };
            (descriptor, handler)
        })
        .collect();
    let registry = Registry::default();
    let dispatcher = Dispatcher::new(&registry, &table);
    let owner = Owner::new_node().unwrap();
    let other = Owner::new_python().unwrap();
    let kinds = HandleKind::ALL;
    assert_eq!(
        manifest["handleKinds"],
        serde_json::to_value(kinds).unwrap()
    );
    let mut checked = 0;
    for (descriptor, _) in &table {
        let name = descriptor.name;
        let expected = descriptor.allowed_handle_kinds;
        for &kind in kinds {
            let id = registry.insert(owner, kind);
            let before = CALLS.load(Ordering::SeqCst);
            let result = invoke(&dispatcher, &registry, owner, id, name, descriptor.mode).await;
            let label = format!("{name} x {kind:?}");
            if name == "handle.drop" || expected.contains(&kind) {
                assert!(result.is_ok(), "{label}");
                assert_eq!(
                    CALLS.load(Ordering::SeqCst) - before,
                    usize::from(name != "handle.drop" && descriptor.mode != OpMode::BinaryPush),
                    "{label}"
                );
            } else {
                invalid(result, expected, Some(kind), &label);
                assert_eq!(CALLS.load(Ordering::SeqCst), before, "{label}");
            }
            if name == "handle.drop" {
                assert!(registry.get(owner, id, &[kind]).is_err());
            }
            drop(registry.remove(owner, id));
            checked += 1;
        }
        let dropped = registry.insert(owner, kinds[0]);
        drop(registry.remove(owner, dropped));
        let foreign = registry.insert(other, kinds[0]);
        for (label, id) in [
            ("global", HandleId(0)),
            ("unknown", HandleId(u64::MAX)),
            ("dropped", dropped),
            ("foreign", foreign),
        ] {
            let before = CALLS.load(Ordering::SeqCst);
            let result = invoke(&dispatcher, &registry, owner, id, name, descriptor.mode).await;
            if name == "handle.drop" || (id.0 == 0 && expected.is_empty()) {
                assert!(result.is_ok(), "{name} x {label}");
                assert_eq!(
                    CALLS.load(Ordering::SeqCst) - before,
                    usize::from(name != "handle.drop")
                );
            } else {
                invalid(result, expected, None, &format!("{name} x {label}"));
                assert_eq!(CALLS.load(Ordering::SeqCst), before);
            }
            checked += 1;
        }
        assert!(registry.get(other, foreign, &[kinds[0]]).is_ok());
        drop(registry.remove(other, foreign));
    }
    assert_eq!(checked, operations.len() * (kinds.len() + 4));
    eprintln!("gate matrix: {checked} op/handle combinations");
}

#[test]
fn production_registry_uses_descriptor_allowlists_and_hides_foreign_handles() {
    let registry = HandleRegistry::default();
    let owner = Owner::new_node().unwrap();
    let other = Owner::new_python().unwrap();
    let id = registry
        .insert(
            owner,
            Entry::Abort(Arc::new(aimux_core::AbortSignal::new())),
        )
        .unwrap();
    let dropped = registry
        .insert(
            owner,
            Entry::Abort(Arc::new(aimux_core::AbortSignal::new())),
        )
        .unwrap();
    drop(registry.remove(owner, dropped));
    for op in aimux_ops::manifest::manifest()["ops"].as_array().unwrap() {
        let name = op["name"].as_str().unwrap();
        let expected = dispatch::descriptor(name).unwrap().allowed_handle_kinds;
        let result = registry.get(owner, id, expected).map(|_| ());
        if expected.contains(&HandleKind::Abort) {
            assert!(result.is_ok());
        } else {
            invalid(result, expected, Some(HandleKind::Abort), name);
        }
        for (caller, id) in [
            (owner, HandleId(0)),
            (owner, HandleId(u64::MAX)),
            (owner, dropped),
            (other, id),
        ] {
            invalid(
                registry.get(caller, id, expected).map(|_| ()),
                expected,
                None,
                name,
            );
        }
    }
}

#[tokio::test]
async fn production_dispatch_rejects_invalid_handles_before_acceptance() {
    let registry = HandleRegistry::global();
    let owner = Owner::new_node().unwrap();
    let other = Owner::new_node().unwrap();
    let id = registry
        .insert(
            owner,
            Entry::Abort(Arc::new(aimux_core::AbortSignal::new())),
        )
        .unwrap();
    let dropped = registry
        .insert(
            owner,
            Entry::Abort(Arc::new(aimux_core::AbortSignal::new())),
        )
        .unwrap();
    drop(registry.remove(owner, dropped));
    let foreign = registry
        .insert(
            other,
            Entry::Abort(Arc::new(aimux_core::AbortSignal::new())),
        )
        .unwrap();
    for op in aimux_ops::manifest::manifest()["ops"].as_array().unwrap() {
        let name = op["name"].as_str().unwrap();
        if name == "handle.drop" {
            continue;
        }
        let descriptor = dispatch::descriptor(name).unwrap();
        let expected = descriptor.allowed_handle_kinds;
        let mut cases = vec![(HandleId(u64::MAX), None), (dropped, None), (foreign, None)];
        if !expected.is_empty() {
            cases.push((HandleId(0), None));
        }
        if !expected.contains(&HandleKind::Abort) {
            cases.push((id, Some(HandleKind::Abort)));
        }
        for (handle, actual) in cases {
            let result = if descriptor.mode == OpMode::Stream {
                dispatch::stream(ctx(owner), handle, name, json!({})).map(|_| ())
            } else if descriptor.mode == OpMode::BinaryPush {
                dispatch::operation_push(&ctx(owner), handle, &[])
            } else {
                dispatch::call(ctx(owner), handle, name, json!({}))
                    .await
                    .map(|_| ())
            };
            invalid(result, expected, actual, name);
        }
    }
    registry.release_owner(owner);
    registry.release_owner(other);
}

#[tokio::test]
async fn handle_exhausted_negative_state_behaves_per_contract() {
    use std::collections::BTreeSet;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use aimux_core::AbortSignal;
    use aimux_core::error::AiMuxError;
    use aimux_core::files_model::{Files, UploadFileCallOptions, UploadFileResult};
    use aimux_core::language_model::LanguageModel;
    use aimux_core::options::CallOptions;
    use aimux_core::provider::Provider;
    use aimux_core::recording::{InputRecord, ProviderRecord, Recording};
    use aimux_core::result::{GenerateResult, StreamResult};
    use aimux_core::transcription_model::{
        TranscriptionCallOptions, TranscriptionModel, TranscriptionResult,
        TranscriptionStreamOptions, TranscriptionStreamPart, TranscriptionStreamResult,
    };
    use aimux_ops::dispatch;
    use aimux_ops::envelope::{OpsError, ProtocolError};
    use aimux_ops::handle::{
        Entry, HandleId, HandleKind, HandleRegistry, Owner, test_exhaust_process_handle_allocator,
    };
    use aimux_ops::op::OpMode;
    use serde_json::json;

    if std::env::var_os("AIMUX_HANDLE_EXHAUSTED_CHILD").is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .env("AIMUX_HANDLE_EXHAUSTED_CHILD", "1")
            .args([
                "--exact",
                "handle_exhausted_negative_state_behaves_per_contract",
                "--nocapture",
            ])
            .status()
            .expect("spawn HandleExhausted matrix subprocess");
        assert!(
            status.success(),
            "HandleExhausted matrix subprocess failed: {status}"
        );
        return;
    }

    struct StubLanguage(Arc<AtomicUsize>);
    impl LanguageModel for StubLanguage {
        fn provider(&self) -> &str {
            "stub"
        }
        fn model_id(&self) -> &str {
            "stub-model"
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
            self.0.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Err(AiMuxError::other("stub must not be reached")) })
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
            self.0.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Err(AiMuxError::other("stub must not be reached")) })
        }
    }

    struct StubFiles(Arc<AtomicUsize>);
    impl Files for StubFiles {
        fn provider(&self) -> &str {
            "stub"
        }
        fn upload_file<'a, 'b, 'f>(
            &'a self,
            _: &'b UploadFileCallOptions,
        ) -> Pin<Box<dyn Future<Output = Result<UploadFileResult, AiMuxError>> + Send + 'f>>
        where
            'a: 'f,
            'b: 'f,
            Self: 'f,
        {
            self.0.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Err(AiMuxError::other("stub files must not be reached")) })
        }
    }

    struct StubTranscription(Arc<AtomicUsize>);
    impl TranscriptionModel for StubTranscription {
        fn provider(&self) -> &str {
            "stub"
        }
        fn model_id(&self) -> &str {
            "stub-realtime"
        }
        fn do_generate<'a, 'b, 'f>(
            &'a self,
            _: &'b TranscriptionCallOptions,
        ) -> Pin<Box<dyn Future<Output = Result<TranscriptionResult, AiMuxError>> + Send + 'f>>
        where
            'a: 'f,
            'b: 'f,
            Self: 'f,
        {
            self.0.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Err(AiMuxError::other("stub transcription generate unused")) })
        }
        fn do_stream(
            &self,
            _: TranscriptionStreamOptions,
        ) -> Option<
            Pin<
                Box<dyn Future<Output = Result<TranscriptionStreamResult, AiMuxError>> + Send + '_>,
            >,
        > {
            self.0.fetch_add(1, Ordering::SeqCst);
            Some(Box::pin(async {
                let empty: Pin<
                    Box<
                        dyn futures::Stream<Item = Result<TranscriptionStreamPart, AiMuxError>>
                            + Send,
                    >,
                > = Box::pin(futures::stream::empty());
                Ok(TranscriptionStreamResult {
                    stream: empty,
                    request: None,
                    response: None,
                })
            }))
        }
    }

    struct StubProvider {
        language: Arc<dyn LanguageModel>,
        files: Arc<dyn Files>,
    }
    impl Provider for StubProvider {
        fn language_model(&self, _: &str) -> Result<Arc<dyn LanguageModel>, AiMuxError> {
            Ok(Arc::clone(&self.language))
        }
        fn embedding_model(
            &self,
            _: &str,
        ) -> Result<Arc<dyn aimux_core::EmbeddingModel>, AiMuxError> {
            Err(AiMuxError::UnsupportedFunctionality(
                "stub embedding unused".into(),
            ))
        }
        fn image_model(&self, _: &str) -> Result<Arc<dyn aimux_core::ImageModel>, AiMuxError> {
            Err(AiMuxError::UnsupportedFunctionality(
                "stub image unused".into(),
            ))
        }
        fn files(&self) -> Option<Arc<dyn Files>> {
            Some(Arc::clone(&self.files))
        }
    }

    let provider_calls = Arc::new(AtomicUsize::new(0));
    let stub_language: Arc<dyn LanguageModel> = Arc::new(StubLanguage(Arc::clone(&provider_calls)));
    let stub_files: Arc<dyn Files> = Arc::new(StubFiles(Arc::clone(&provider_calls)));
    let stub_provider: Arc<dyn Provider> = Arc::new(StubProvider {
        language: Arc::clone(&stub_language),
        files: Arc::clone(&stub_files),
    });

    let registry = HandleRegistry::global();
    let owner = Owner::new_node().unwrap();
    let other = Owner::new_node().unwrap();

    let abort_id = registry
        .insert(owner, Entry::Abort(Arc::new(AbortSignal::new())))
        .unwrap();
    let lang_id = registry
        .insert(owner, Entry::Language(Arc::clone(&stub_language)))
        .unwrap();
    let provider_id = registry
        .insert(owner, Entry::Provider(Arc::clone(&stub_provider)))
        .unwrap();
    let files_id = registry
        .insert(owner, Entry::Files(Arc::clone(&stub_files)))
        .unwrap();
    let transcription_id = registry
        .insert(
            owner,
            Entry::Transcription(Arc::new(StubTranscription(Arc::clone(&provider_calls)))),
        )
        .unwrap();
    let trace_created = dispatch::call(ctx(owner), HandleId(0), "trace.create", json!({}))
        .await
        .unwrap();
    let trace_id = HandleId(trace_created.as_str().unwrap().parse().unwrap());
    let middleware_created = dispatch::call(
        ctx(owner),
        HandleId(0),
        "middleware.create",
        json!({"kind": "trace", "traceStore": trace_id.0.to_string()}),
    )
    .await
    .unwrap();
    let middleware_id = HandleId(
        middleware_created["handle"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap(),
    );
    let registry_created = dispatch::call(
        ctx(owner),
        HandleId(0),
        "registry.create",
        json!({"entries": [{"key": "k", "provider": provider_id.0.to_string()}]}),
    )
    .await
    .unwrap();
    let registry_id = HandleId(
        registry_created["handle"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap(),
    );
    let recorder_created = dispatch::call(
        ctx(owner),
        HandleId(0),
        "recorder.create",
        json!({"backend": {"type": "ring"}, "capturePolicy": "full"}),
    )
    .await
    .unwrap();
    let recorder_id = HandleId(
        recorder_created["handle"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap(),
    );
    let session_created = dispatch::call(
        ctx(owner),
        HandleId(0),
        "sessionStore.create",
        json!({"infer": false}),
    )
    .await
    .unwrap();
    let session_id = HandleId(session_created["handle"].as_str().unwrap().parse().unwrap());
    let mut recording = Recording::new(
        "exhaust-replay",
        InputRecord::from_call_options(&CallOptions::default()),
        ProviderRecord::new("openai", "openai.chat", "gpt-4o"),
    );
    recording.complete = true;
    recording.transport_closed = true;
    let recording_jsonl = serde_json::to_string(&recording).unwrap();
    let replay_created = dispatch::call(
        ctx(owner),
        HandleId(0),
        "replay.create",
        json!({"recordingsJsonl": &recording_jsonl}),
    )
    .await
    .unwrap();
    let replay_id = HandleId(replay_created["handle"].as_str().unwrap().parse().unwrap());

    let dropped = registry
        .insert(owner, Entry::Abort(Arc::new(AbortSignal::new())))
        .unwrap();
    drop(registry.remove(owner, dropped));
    let foreign = registry
        .insert(other, Entry::Abort(Arc::new(AbortSignal::new())))
        .unwrap();

    fn manifest_allocating_ops(manifest: &Value) -> BTreeSet<&str> {
        let output_defs = manifest["typeDefs"]["output"].as_object().unwrap();
        let types = manifest["types"].as_array().unwrap();
        let allocating_result_types: BTreeSet<&str> = types
            .iter()
            .filter_map(|ty| {
                let id = ty["id"].as_str()?;
                let output_name = ty["output"].as_str()?;
                let has_handle_prop = output_defs
                    .get(output_name)
                    .and_then(|schema| schema.get("properties"))
                    .and_then(Value::as_object)
                    .is_some_and(|props| props.contains_key("handle"));
                (id.contains("Handle") || has_handle_prop).then_some(id)
            })
            .collect();
        manifest["ops"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|op| {
                op["scope"] != "future"
                    && op["resultType"]
                        .as_str()
                        .is_some_and(|ty| allocating_result_types.contains(ty))
            })
            .map(|op| op["name"].as_str().unwrap())
            .collect()
    }

    async fn assert_allocating_ops_fail_with_handle_exhausted(
        owner: Owner,
        expected_allocating: &BTreeSet<&str>,
        cases: Vec<(&str, HandleId, Value)>,
    ) {
        let mut tested_allocating = BTreeSet::new();
        for (op_name, target, params) in cases {
            let err = dispatch::call(ctx(owner), target, op_name, params)
                .await
                .expect_err(&format!("{op_name} must fail with HandleExhausted"));
            assert!(
                matches!(err, OpsError::Protocol(ProtocolError::HandleExhausted)),
                "{op_name} returned {err:?} instead of HandleExhausted"
            );
            assert_eq!(err.code(), 306, "{op_name}");
            tested_allocating.insert(op_name);
        }
        assert_eq!(&tested_allocating, expected_allocating);
    }

    async fn assert_jsonl_recorder_avoids_disk_side_effects_when_exhausted(owner: Owner) {
        let jsonl_parent =
            std::env::temp_dir().join(format!("aimux-exhausted-jsonl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&jsonl_parent);
        let jsonl_dir = jsonl_parent.join("must-not-exist");
        let jsonl_err = dispatch::call(
            ctx(owner),
            HandleId(0),
            "recorder.create",
            json!({"backend": {"type": "jsonl", "directory": jsonl_dir.to_string_lossy()}, "capturePolicy": "full"}),
        )
        .await
        .expect_err("jsonl recorder.create must fail with HandleExhausted");
        assert_eq!(jsonl_err.code(), 306);
        assert!(
            !jsonl_dir.exists(),
            "jsonl recorder.create must not create {} when handles are exhausted",
            jsonl_dir.display()
        );
        let _ = std::fs::remove_dir_all(&jsonl_parent);
    }

    async fn assert_negative_and_secondary_handles_fail_under_exhaustion(
        owner: Owner,
        manifest_ops: &[Value],
        abort_id: HandleId,
        dropped: HandleId,
        foreign: HandleId,
    ) {
        let future_err = dispatch::call(
            ctx(owner),
            HandleId(0),
            "credential.create",
            json!({"kind": "bearer"}),
        )
        .await
        .unwrap_err();
        assert_eq!(future_err.code(), 8);

        for op in manifest_ops {
            let name = op["name"].as_str().unwrap();
            if name == "handle.drop" {
                continue;
            }
            let descriptor = dispatch::descriptor(name).unwrap();
            let expected = descriptor.allowed_handle_kinds;
            if expected.is_empty() {
                continue;
            }
            let mut cases = vec![
                (HandleId(u64::MAX), None),
                (dropped, None),
                (foreign, None),
                (HandleId(0), None),
            ];
            if !expected.contains(&HandleKind::Abort) {
                cases.push((abort_id, Some(HandleKind::Abort)));
            }
            for (handle, actual) in cases {
                let result = if descriptor.mode == OpMode::Stream {
                    dispatch::stream(ctx(owner), handle, name, json!({})).map(|_| ())
                } else if descriptor.mode == OpMode::BinaryPush {
                    dispatch::operation_push(&ctx(owner), handle, &[])
                } else {
                    dispatch::call(ctx(owner), handle, name, json!({}))
                        .await
                        .map(|_| ())
                };
                invalid(result, expected, actual, &format!("{name} x exhausted"));
            }
        }

        let bad_id = HandleId(u64::MAX).0.to_string();
        let bad_secondary_err = dispatch::call(
            ctx(owner),
            HandleId(0),
            "middleware.create",
            json!({"kind": "trace", "traceStore": bad_id}),
        )
        .await
        .unwrap_err();
        assert_eq!(bad_secondary_err.code(), 203);
    }

    async fn assert_non_allocating_ops_succeed_under_exhaustion(
        owner: Owner,
        cases: Vec<(&str, HandleId, Value)>,
    ) {
        for (op_name, target, params) in cases {
            dispatch::call(ctx(owner), target, op_name, params)
                .await
                .unwrap_or_else(|e| {
                    panic!("{op_name} should succeed under HandleExhausted: {e:?}")
                });
        }
    }

    async fn assert_exhaustion_guard_recovers_after_last_drop(owner: Owner) {
        let recovered = dispatch::call(ctx(owner), HandleId(0), "abort.create", json!({}))
            .await
            .expect("allocator recovers after ExhaustionGuard drop");
        let recovered_id = HandleId(recovered["handle"].as_str().unwrap().parse().unwrap());
        assert_ne!(recovered_id.0, 0);

        let guard_outer = test_exhaust_process_handle_allocator();
        let guard_inner = test_exhaust_process_handle_allocator();
        drop(guard_outer);
        let still_exhausted = dispatch::call(ctx(owner), HandleId(0), "abort.create", json!({}))
            .await
            .unwrap_err();
        assert_eq!(still_exhausted.code(), 306);
        drop(guard_inner);
        let recovered_after_overlap =
            dispatch::call(ctx(owner), HandleId(0), "abort.create", json!({}))
                .await
                .expect("allocator recovers after all overlapping ExhaustionGuards drop");
        assert_ne!(
            recovered_after_overlap["handle"]
                .as_str()
                .unwrap()
                .parse::<u64>()
                .unwrap(),
            0
        );
    }

    let manifest = aimux_ops::manifest::manifest();
    let manifest_ops = manifest["ops"].as_array().unwrap();
    let expected_allocating = manifest_allocating_ops(&manifest);

    let guard = test_exhaust_process_handle_allocator();
    provider_calls.store(0, Ordering::SeqCst);

    let lang_s = lang_id.0.to_string();
    let provider_s = provider_id.0.to_string();
    let trace_s = trace_id.0.to_string();
    let middleware_s = middleware_id.0.to_string();

    assert_allocating_ops_fail_with_handle_exhausted(
        owner,
        &expected_allocating,
        vec![
            ("abort.create", HandleId(0), json!({})),
            ("transport.create", HandleId(0), json!({"kind": "fetch"})),
            (
                "middleware.create",
                HandleId(0),
                json!({"kind": "trace", "traceStore": trace_s}),
            ),
            (
                "provider.create",
                HandleId(0),
                json!({"package": "openai", "settings": {"apiKey": "k"}}),
            ),
            (
                "provider.default",
                HandleId(0),
                json!({"package": "openai"}),
            ),
            (
                "provider.custom",
                HandleId(0),
                json!({"languageModels": {"m": lang_s}}),
            ),
            (
                "provider.model",
                provider_id,
                json!({"method": "languageModel", "modelId": "gpt-4o"}),
            ),
            ("provider.wrap", provider_id, json!({})),
            ("provider.files", provider_id, json!({})),
            (
                "registry.create",
                HandleId(0),
                json!({"entries": [{"key": "k", "provider": provider_s}]}),
            ),
            (
                "registry.fromConfig",
                HandleId(0),
                json!({"config": {"providers": {}}}),
            ),
            ("registry.provider", registry_id, json!({"key": "k"})),
            ("registry.update", registry_id, json!({"derive": true})),
            (
                "model.resolve",
                registry_id,
                json!({
                    "modelRef": {
                        "namespace": "",
                        "key": "k",
                        "method": "languageModel",
                        "modelId": "gpt-4o"
                    }
                }),
            ),
            ("model.wrap", lang_id, json!({"middleware": [middleware_s]})),
            (
                "composite.router",
                HandleId(0),
                json!({"children": [lang_s], "strategy": {"type": "rule"}}),
            ),
            (
                "composite.moa",
                HandleId(0),
                json!({"references": [lang_s], "aggregator": lang_s}),
            ),
            ("transcription.start", transcription_id, json!({})),
            ("trace.create", HandleId(0), json!({})),
            (
                "recorder.create",
                HandleId(0),
                json!({"backend": {"type": "ring"}, "capturePolicy": "full"}),
            ),
            ("sessionStore.create", HandleId(0), json!({"infer": false})),
            (
                "replay.create",
                HandleId(0),
                json!({"recordingsJsonl": recording_jsonl}),
            ),
        ],
    )
    .await;

    assert_jsonl_recorder_avoids_disk_side_effects_when_exhausted(owner).await;
    assert_negative_and_secondary_handles_fail_under_exhaustion(
        owner,
        manifest_ops,
        abort_id,
        dropped,
        foreign,
    )
    .await;
    assert_non_allocating_ops_succeed_under_exhaustion(
        owner,
        vec![
            ("abort.signal", abort_id, json!({})),
            ("model.info", lang_id, json!({})),
            ("registry.update", registry_id, json!({"derive": false})),
            ("trace.aggregate", trace_id, json!({})),
            ("trace.export", trace_id, json!({})),
            ("trace.clear", trace_id, json!({})),
            ("sessionStore.list", session_id, json!({})),
            ("recorder.stats", recorder_id, json!({})),
            ("recorder.export", recorder_id, json!({})),
            ("replay.inspect", replay_id, json!({})),
            ("replay.dryRun", replay_id, json!({})),
        ],
    )
    .await;

    assert_eq!(provider_calls.load(Ordering::SeqCst), 0);
    let _ = files_id;
    drop(guard);

    assert_exhaustion_guard_recovers_after_last_drop(owner).await;

    registry.release_owner(owner);
    registry.release_owner(other);
}
