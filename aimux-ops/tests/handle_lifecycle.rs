//! RFC-0039 §3.3 handle lifetime and owner-isolation gate.
//!
//! Exhaustion needs an isolated allocator seeded near u64::MAX; the frozen
//! registry API exposes only the process-global allocator. That boundary must
//! be covered by the handle implementation's unit tests, not by exhausting the
//! process-global allocator here. Realtime construction is tested in operation.rs.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use aimux_core::custom_provider::{CustomProviderOptions, custom_provider};
use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::options::CallOptions;
use aimux_core::provider::Provider;
use aimux_core::provider_registry::{ProviderRegistryOptions, create_provider_registry};
use aimux_core::result::{GenerateContent, GenerateResult, StreamResult};
use aimux_core::router::{FallbackPolicy, RouterConfig, RouterModel, RuleRouter};
use aimux_ops::dispatch::{self, Ctx};
use aimux_ops::envelope::{BindingError, OpsError};
use aimux_ops::handle::{Entry, HandleId, HandleKind, HandleRegistry, Owner};
use futures::future::BoxFuture;
use serde_json::json;

const OWNER: Owner = Owner::Node(44);
const LANGUAGE: &[HandleKind] = &[HandleKind::Language];
const WAIT: Duration = Duration::from_secs(5);

static REENTRIES: AtomicUsize = AtomicUsize::new(0);

struct Model {
    reenter: bool,
}

fn reenter_registry() {
    REENTRIES.fetch_add(1, Ordering::SeqCst);
    let registry = HandleRegistry::global();
    let id = registry.insert(OWNER, entry()).unwrap();
    assert!(registry.get(OWNER, id, LANGUAGE).is_ok());
    assert!(registry.remove(OWNER, id).is_some());
}

impl LanguageModel for Model {
    fn provider(&self) -> &str {
        if self.reenter {
            reenter_registry();
        }
        "lifecycle"
    }

    fn model_id(&self) -> &str {
        "model"
    }

    // Match async_trait's expanded signature without adding a dev dependency
    // to the crate manifest.
    fn do_generate<'a, 'b, 'f>(
        &'a self,
        _options: &'b CallOptions,
    ) -> BoxFuture<'f, Result<GenerateResult, AiMuxError>>
    where
        'a: 'f,
        'b: 'f,
        Self: 'f,
    {
        Box::pin(async move {
            tokio::task::yield_now().await;
            if self.reenter {
                reenter_registry();
            }
            Ok(GenerateResult {
                content: vec![GenerateContent::Text {
                    text: "still alive".into(),
                    provider_metadata: None,
                }],
                finish_reason: aimux_core::types::FinishReason {
                    unified: aimux_core::types::FinishReasonUnified::Stop,
                    raw: None,
                },
                usage: Default::default(),
                warnings: Vec::new(),
                provider_metadata: None,
                request: None,
                response: None,
            })
        })
    }

    fn do_stream<'a, 'b, 'f>(
        &'a self,
        _options: &'b CallOptions,
    ) -> BoxFuture<'f, Result<StreamResult, AiMuxError>>
    where
        'a: 'f,
        'b: 'f,
        Self: 'f,
    {
        Box::pin(async {
            Err(AiMuxError::UnsupportedFunctionality(
                "unused in this gate".into(),
            ))
        })
    }
}

fn entry() -> Entry {
    Entry::Language(Arc::new(Model { reenter: false }))
}

fn language(entry: Entry) -> Arc<dyn LanguageModel> {
    match entry {
        Entry::Language(model) => model,
        _ => panic!("expected a language model"),
    }
}

fn invalid_handle(
    result: Result<Entry, OpsError>,
    actual: Option<HandleKind>,
) -> serde_json::Value {
    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("lookup unexpectedly succeeded"),
    };
    match &error {
        OpsError::Binding(BindingError::InvalidHandle {
            expected,
            actual: found,
        }) => {
            assert_eq!(expected, LANGUAGE);
            assert_eq!(*found, actual);
        }
        _ => panic!("expected InvalidHandle"),
    }
    assert_eq!(error.code(), 203);
    serde_json::to_value(error.envelope(SystemTime::UNIX_EPOCH)).unwrap()
}

async fn assert_working(model: Arc<dyn LanguageModel>) {
    let result = model
        .do_generate(&CallOptions::new(Vec::new()))
        .await
        .unwrap();
    assert!(
        matches!(result.content.as_slice(), [GenerateContent::Text { text, .. }] if text == "still alive")
    );
}

#[test]
fn ids_are_nonzero_monotonic_and_never_reused_after_drop() {
    let registry = HandleRegistry::global();
    let mut previous = 0;
    for _ in 0..1_024 {
        let id = registry.insert(OWNER, entry()).unwrap();
        assert!(id.0 > previous);
        previous = id.0;
        assert!(registry.remove(OWNER, id).is_some());
        invalid_handle(registry.get(OWNER, id, LANGUAGE), None);
    }
}

#[test]
fn every_owner_pair_is_isolated_and_foreign_errors_reveal_nothing() {
    let registry = HandleRegistry::global();
    let owners = [
        Owner::CProcess,
        Owner::Node(44),
        Owner::Node(45),
        Owner::Python(44),
        Owner::Python(45),
    ];
    for owner in owners {
        // A Provider is deliberately different from the requested Language kind:
        // a foreign-owner error must hide even that type distinction.
        let provider = Arc::new(custom_provider(CustomProviderOptions::default()));
        let id = registry.insert(owner, Entry::Provider(provider)).unwrap();
        for caller in owners {
            if caller == owner {
                continue;
            }
            let foreign = invalid_handle(registry.get(caller, id, LANGUAGE), None);
            let unknown = invalid_handle(registry.get(caller, HandleId(0), LANGUAGE), None);
            assert_eq!(foreign, unknown, "foreign lookup leaked details");
            assert!(registry.remove(caller, id).is_none());
            assert!(registry.get(owner, id, &[HandleKind::Provider]).is_ok());
        }
        invalid_handle(
            registry.get(owner, id, LANGUAGE),
            Some(HandleKind::Provider),
        );
        assert!(registry.remove(owner, id).is_some());
    }
}

#[test]
fn remove_is_idempotent_for_live_dropped_zero_and_unknown_handles() {
    let registry = HandleRegistry::global();
    assert!(registry.remove(OWNER, HandleId(0)).is_none());
    assert!(registry.remove(OWNER, HandleId(u64::MAX)).is_none());
    assert!(registry.remove(OWNER, HandleId(u64::MAX)).is_none());
    let id = registry.insert(OWNER, entry()).unwrap();
    assert!(registry.remove(OWNER, id).is_some());
    assert!(registry.remove(OWNER, id).is_none());
    // An allocated-and-removed ID is guaranteed unknown even while other tests
    // allocate from the same process-global registry.
    invalid_handle(registry.get(OWNER, id, LANGUAGE), None);
    invalid_handle(registry.get(OWNER, HandleId(0), LANGUAGE), None);
}

#[tokio::test]
async fn an_admitted_call_survives_removal_and_can_reenter_after_await() {
    let registry = HandleRegistry::global();
    let id = registry
        .insert(OWNER, Entry::Language(Arc::new(Model { reenter: true })))
        .unwrap();
    let admitted = language(registry.get(OWNER, id, LANGUAGE).unwrap());
    let options = CallOptions::new(Vec::new());
    let call = admitted.do_generate(&options);
    assert!(registry.remove(OWNER, id).is_some());
    invalid_handle(registry.get(OWNER, id, LANGUAGE), None);
    let result = tokio::time::timeout(WAIT, call).await.unwrap().unwrap();
    assert!(
        matches!(result.content.as_slice(), [GenerateContent::Text { text, .. }] if text == "still alive")
    );
}

#[tokio::test]
async fn models_survive_provider_and_registry_removal() {
    let handles = HandleRegistry::global();
    let provider: Arc<dyn Provider> = Arc::new(custom_provider(CustomProviderOptions {
        language_models: HashMap::from([(
            "model".into(),
            Arc::new(Model { reenter: false }) as Arc<dyn LanguageModel>,
        )]),
        ..Default::default()
    }));
    let provider_id = handles
        .insert(OWNER, Entry::Provider(provider.clone()))
        .unwrap();
    let registry = Arc::new(create_provider_registry(
        BTreeMap::from([("local".into(), provider)]),
        ProviderRegistryOptions::default(),
    ));
    let registry_id = handles.insert(OWNER, Entry::Registry(registry)).unwrap();
    let resolved = match handles
        .get(OWNER, registry_id, &[HandleKind::Registry])
        .unwrap()
    {
        Entry::Registry(registry) => registry.language_model("local:model").unwrap(),
        _ => panic!("expected registry"),
    };
    let model_id = handles.insert(OWNER, Entry::Language(resolved)).unwrap();
    drop(handles.remove(OWNER, provider_id).unwrap());
    drop(handles.remove(OWNER, registry_id).unwrap());
    assert_working(language(handles.get(OWNER, model_id, LANGUAGE).unwrap())).await;
    drop(handles.remove(OWNER, model_id).unwrap());
}

#[tokio::test]
async fn a_composite_survives_removal_of_its_child_handle() {
    let registry = HandleRegistry::global();
    let child_id = registry.insert(OWNER, entry()).unwrap();
    let child = language(registry.get(OWNER, child_id, LANGUAGE).unwrap());
    let composite = RouterModel::new(
        vec![child],
        Box::new(RuleRouter),
        FallbackPolicy::None,
        RouterConfig::default(),
    );
    let composite_id = registry
        .insert(OWNER, Entry::Language(Arc::new(composite)))
        .unwrap();
    drop(registry.remove(OWNER, child_id).unwrap());
    invalid_handle(registry.get(OWNER, child_id, LANGUAGE), None);
    assert_working(language(
        registry.get(OWNER, composite_id, LANGUAGE).unwrap(),
    ))
    .await;
    drop(registry.remove(OWNER, composite_id).unwrap());
}

#[test]
fn concurrent_get_and_remove_preserve_admitted_references_and_unique_ids() {
    let registry = HandleRegistry::global();
    let barrier = Arc::new(Barrier::new(8));
    let workers: Vec<_> = (0..8)
        .map(|worker| {
            let barrier = barrier.clone();
            thread::spawn(move || {
                let owner = Owner::Node(4_400 + worker);
                let mut ids = Vec::new();
                for _ in 0..128 {
                    let id = registry.insert(owner, entry()).unwrap();
                    assert!(ids.last().is_none_or(|previous| *previous < id.0));
                    ids.push(id.0);
                    let admitted = language(registry.get(owner, id, LANGUAGE).unwrap());
                    barrier.wait();
                    thread::scope(|scope| {
                        scope.spawn(|| {
                            drop(registry.remove(owner, id));
                        });
                        scope.spawn(|| match registry.get(owner, id, LANGUAGE) {
                            Ok(found) => assert_eq!(language(found).model_id(), "model"),
                            Err(error) => {
                                invalid_handle(Err(error), None);
                            }
                        });
                    });
                    assert_eq!(admitted.model_id(), "model");
                    invalid_handle(registry.get(owner, id, LANGUAGE), None);
                    assert!(registry.remove(owner, id).is_none());
                }
                ids
            })
        })
        .collect();
    let ids: Vec<_> = workers
        .into_iter()
        .flat_map(|worker| worker.join().unwrap())
        .collect();
    assert_eq!(ids.len(), 1_024);
    assert_eq!(ids.iter().copied().collect::<HashSet<_>>().len(), ids.len());
}

#[test]
fn dispatch_releases_registry_lock_before_calling_model_code() {
    const CHILD: &str = "AIMUX_HANDLE_LIFECYCLE_REENTRY_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let registry = HandleRegistry::global();
            let id = registry
                .insert(OWNER, Entry::Language(Arc::new(Model { reenter: true })))
                .unwrap();
            let registrations = [(
                dispatch::descriptor("model.info").unwrap(),
                dispatch::Handler::Call(info_probe),
            )];
            let dispatcher = dispatch::Dispatcher::new(registry, &registrations);
            let result = dispatcher
                .call(
                    Ctx {
                        owner: OWNER,
                        abort: None,
                        now: SystemTime::now,
                    },
                    id,
                    "model.info",
                    json!({}),
                )
                .await;
            drop(registry.remove(OWNER, id));
            assert!(result.is_ok(), "model.info failed");
            assert!(
                REENTRIES.load(Ordering::SeqCst) > 0,
                "model.info did not invoke the reentrant model"
            );
        });
        return;
    }

    // Isolate a potential synchronous mutex deadlock from the other tests.
    // An async timeout cannot interrupt a blocked executor thread.
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "dispatch_releases_registry_lock_before_calling_model_code",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(
                status.success(),
                "reentrant dispatch child failed: {status}"
            );
            break;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("dispatch deadlocked while model.info re-entered the registry");
        }
        thread::sleep(Duration::from_millis(10));
    }
}

// Use the actual model.info descriptor with a reentrant probe to isolate
// registry lock behavior from model metadata.
fn info_probe(_: Ctx, target: dispatch::Target, _: serde_json::Value) -> dispatch::CallFuture {
    Box::pin(async move {
        if let dispatch::Target::Handle(Entry::Language(model)) = target {
            Ok(json!({"provider": model.provider()}))
        } else {
            panic!("expected language target")
        }
    })
}
