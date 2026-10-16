//! Explicit provider registries and model references. Resolution never uses a gateway.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock, Weak};

use aimux_core::error::AiMuxError;
use aimux_core::provider_registry::{
    ProviderRegistry, ProviderRegistryOptions, create_provider_registry,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::dispatch::{CallFuture, Ctx, Handler, Target, decode, resolve_secondary};
use crate::envelope::{BindingError, OpsError, ProtocolError};
use crate::handle::{Entry, HandleId, HandleKind, HandleRegistry, Middleware, Owner};
use crate::op::{Cancellation, OpDescriptor, OpMode, RequestBytePolicy, Scope, TypeRef};

const MODELS: &[HandleKind] = &[
    HandleKind::Language,
    HandleKind::Embedding,
    HandleKind::Image,
    HandleKind::Speech,
    HandleKind::Transcription,
    HandleKind::Reranking,
    HandleKind::Video,
    HandleKind::Search,
    HandleKind::Decision,
];

const fn descriptor(
    name: &'static str,
    kinds: &'static [HandleKind],
    request_type: TypeRef,
    result_type: TypeRef,
    symbols: &'static [&'static str],
) -> OpDescriptor {
    OpDescriptor {
        name,
        since_ops_version: "1.0",
        mode: OpMode::Call,
        allowed_handle_kinds: kinds,
        request_type,
        result_type,
        event_type: None,
        scope: Scope::Current,
        terminal_type: None,
        runtime_handle_fields: match name.as_bytes() {
            b"registry.create" => &[("entries.*.provider", &[HandleKind::Provider])],
            b"model.wrap" => &[("middleware.*", &[HandleKind::Middleware])],
            _ => &[],
        },
        required_capabilities: &[],
        cancellation: Cancellation::None,
        result_ownership: "owned",
        request_byte_policy: RequestBytePolicy::Json,
        side_effects: matches!(name.as_bytes(), b"registry.create"),
        retry_owner: "none",
        source_symbols: symbols,
    }
}

pub(crate) static OPS: &[OpDescriptor] = &[
    descriptor(
        "registry.create",
        &[],
        TypeRef::of::<RegistryCreateParams>(),
        TypeRef::of::<HandleResult>(),
        &["aimux_core::provider_registry::create_provider_registry"],
    ),
    descriptor(
        "registry.provider",
        &[HandleKind::Registry],
        TypeRef::of::<RegistryProviderParams>(),
        TypeRef::of::<HandleResult>(),
        &["aimux_core::provider_registry::ProviderRegistry::provider"],
    ),
    descriptor(
        "model.resolve",
        &[HandleKind::Registry],
        TypeRef::of::<ModelResolveParams>(),
        TypeRef::of::<HandleResult>(),
        &["aimux_core::provider_registry::ProviderRegistry"],
    ),
    descriptor(
        "model.info",
        MODELS,
        TypeRef::of::<ModelInfoParams>(),
        TypeRef::of::<ModelInfo>(),
        &["aimux_core::language_model::LanguageModel::provider"],
    ),
    descriptor(
        "model.wrap",
        MODELS,
        TypeRef::of::<ModelWrapParams>(),
        TypeRef::of::<HandleResult>(),
        &[
            "aimux_core::language_model_middleware::wrap_language_model",
            "aimux_core::image_model_middleware::wrap_image_model",
        ],
    ),
];

pub(crate) fn register(r: &mut Vec<(&'static OpDescriptor, Handler)>) {
    for (op, handler) in OPS.iter().zip([
        create as fn(Ctx, Target, Value) -> CallFuture,
        provider,
        resolve,
        info,
        wrap,
    ]) {
        r.push((op, Handler::Call(handler)));
    }
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RegistryCreateParams {
    #[serde(default)]
    namespace: String,
    entries: Vec<RegistryEntry>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RegistryEntry {
    key: String,
    provider: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct RegistryProviderParams {
    key: String,
}

#[derive(Clone, Deserialize, Serialize, PartialEq, Debug, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ModelRef {
    namespace: String,
    key: String,
    method: ModelMethod,
    model_id: String,
}

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Debug, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
enum ModelMethod {
    LanguageModel,
    EmbeddingModel,
    ImageModel,
    SpeechModel,
    TranscriptionModel,
    RerankingModel,
    VideoModel,
    SearchModel,
    #[serde(rename = "decision")]
    Decision,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ModelResolveParams {
    model_ref: ModelRef,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct ModelInfoParams {}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct ModelWrapParams {
    middleware: Vec<String>,
}

fn invalid(message: impl Into<String>) -> OpsError {
    OpsError::Protocol(ProtocolError::InvalidParams(message.into()))
}

fn handle(value: &str) -> Result<HandleId, OpsError> {
    if value.is_empty()
        || !value.bytes().all(|b| b.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return Err(invalid("handle must be a canonical decimal string"));
    }
    value
        .parse::<u64>()
        .map(HandleId)
        .map_err(|_| invalid("handle exceeds u64"))
}

fn wrong(entry: &Entry, expected: &[HandleKind]) -> OpsError {
    OpsError::Binding(BindingError::InvalidHandle {
        expected: expected.to_vec(),
        actual: Some(entry.kind()),
    })
}

#[derive(Serialize, schemars::JsonSchema)]
struct HandleResult {
    handle: HandleId,
    kind: HandleKind,
}

#[derive(Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ModelInfo<'a> {
    provider: &'a str,
    model_id: &'a str,
    modality: &'static str,
    capabilities: Value,
    source: Value,
}

fn target_entry(target: Target, expected: &[HandleKind]) -> Result<Entry, OpsError> {
    match target {
        Target::Handle(entry) if expected.contains(&entry.kind()) => Ok(entry),
        Target::Handle(entry) => Err(wrong(&entry, expected)),
        Target::Global => Err(BindingError::InvalidHandle {
            expected: expected.to_vec(),
            actual: None,
        }
        .into()),
    }
}

fn insert(owner: Owner, entry: Entry) -> Result<Value, OpsError> {
    let kind = entry.kind();
    let id = HandleRegistry::global().insert(owner, entry)?;
    Ok(json!(HandleResult { handle: id, kind }))
}

// Core registries and model traits have no source fields. These weak sidecars
// contain only reference metadata, never settings, credentials, or providers.
// Dead objects are pruned on access, including when an allocator reuses an address.
macro_rules! weak_entries {
    ($($variant:ident($ty:ty)),+ $(,)?) => {
        enum WeakEntry { $($variant(Weak<$ty>)),+ }
        impl WeakEntry {
            fn new(entry: &Entry) -> Option<Self> {
                match entry {
                    $(Entry::$variant(value) => Some(Self::$variant(Arc::downgrade(value)))),+,
                    _ => None,
                }
            }
            fn matches(&self, entry: &Entry) -> bool {
                match (self, entry) {
                    $((Self::$variant(weak), Entry::$variant(value)) => weak.upgrade()
                        .is_some_and(|live| Arc::ptr_eq(&live, value))),+,
                    _ => false,
                }
            }
            fn alive(&self) -> bool {
                match self { $(Self::$variant(weak) => weak.strong_count() > 0),+ }
            }
        }
    };
}

weak_entries! {
    Registry(ProviderRegistry),
    Language(dyn aimux_core::language_model::LanguageModel),
    Embedding(dyn aimux_core::embedding_model::EmbeddingModel),
    Image(dyn aimux_core::image_model::ImageModel),
    Speech(dyn aimux_core::speech_model::SpeechModel),
    Transcription(dyn aimux_core::transcription_model::TranscriptionModel),
    Reranking(dyn aimux_core::reranking_model::RerankingModel),
    Video(dyn aimux_core::video_model::VideoModel),
    Search(dyn aimux_core::search_model::SearchModel),
    Decision(dyn aimux_core::decision_model::DecisionModel),
}

#[derive(Clone)]
enum Metadata {
    Namespace(String),
    Sources(Vec<ModelRef>),
}

struct Sidecar {
    owner: Owner,
    entry: WeakEntry,
    metadata: Metadata,
}

fn sidecars() -> &'static Mutex<Vec<Sidecar>> {
    static SIDECARS: OnceLock<Mutex<Vec<Sidecar>>> = OnceLock::new();
    SIDECARS.get_or_init(|| Mutex::new(Vec::new()))
}

fn metadata(owner: Owner, entry: &Entry, update: Option<Metadata>) -> Option<Metadata> {
    let mut sidecars = sidecars()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    sidecars.retain(|sidecar| sidecar.entry.alive());
    if let Some(sidecar) = sidecars
        .iter_mut()
        .find(|sidecar| sidecar.owner == owner && sidecar.entry.matches(entry))
    {
        if let Some(update) = update {
            match (&mut sidecar.metadata, update) {
                (Metadata::Sources(sources), Metadata::Sources(additions)) => {
                    for source in additions {
                        if !sources.contains(&source) {
                            sources.push(source);
                        }
                    }
                }
                (current, update) => *current = update,
            }
        }
        return Some(sidecar.metadata.clone());
    }
    if let (Some(weak), Some(update)) = (WeakEntry::new(entry), update) {
        sidecars.push(Sidecar {
            owner,
            entry: weak,
            metadata: update.clone(),
        });
        return Some(update);
    }
    None
}

fn create(ctx: Ctx, target: Target, params: Value) -> CallFuture {
    Box::pin(async move {
        if let Target::Handle(entry) = target {
            return Err(wrong(&entry, &[]));
        }
        let params: RegistryCreateParams = decode(params)?;
        let mut providers = BTreeMap::new();
        for registration in params.entries {
            if registration.key.is_empty() {
                return Err(invalid("registry entry key must not be empty"));
            }
            if providers.contains_key(&registration.key) {
                return Err(invalid(format!(
                    "duplicate registry key: {}",
                    registration.key
                )));
            }
            let provider = resolve_secondary(
                HandleRegistry::global(),
                &ctx,
                &json!(handle(&registration.provider)?),
                &[HandleKind::Provider],
            )?;
            let Entry::Provider(provider) = provider else {
                return Err(wrong(&provider, &[HandleKind::Provider]));
            };
            providers.insert(registration.key, provider);
        }
        let entry = Entry::Registry(Arc::new(create_provider_registry(
            providers,
            ProviderRegistryOptions::default(),
        )));
        metadata(
            ctx.owner,
            &entry,
            Some(Metadata::Namespace(params.namespace)),
        );
        insert(ctx.owner, entry)
    })
}

fn provider(ctx: Ctx, target: Target, params: Value) -> CallFuture {
    Box::pin(async move {
        let entry = target_entry(target, &[HandleKind::Registry])?;
        let params: RegistryProviderParams = decode(params)?;
        let Entry::Registry(registry) = &entry else {
            return Err(wrong(&entry, &[HandleKind::Registry]));
        };
        let provider = registry.provider(&params.key).map_err(OpsError::AiMux)?;
        insert(ctx.owner, Entry::Provider(Arc::clone(provider)))
    })
}

fn resolve(ctx: Ctx, target: Target, params: Value) -> CallFuture {
    Box::pin(async move {
        let entry = target_entry(target, &[HandleKind::Registry])?;
        let params: ModelResolveParams = decode(params)?;
        let Entry::Registry(registry) = &entry else {
            return Err(wrong(&entry, &[HandleKind::Registry]));
        };
        let namespace = match metadata(ctx.owner, &entry, None) {
            Some(Metadata::Namespace(namespace)) => namespace,
            _ => String::new(),
        };
        if params.model_ref.namespace != namespace {
            return Err(invalid("modelRef namespace does not match this registry"));
        }
        let model = resolve_model(registry, &params.model_ref)?;
        metadata(
            ctx.owner,
            &model,
            Some(Metadata::Sources(vec![params.model_ref])),
        );
        insert(ctx.owner, model)
    })
}

fn resolve_model(registry: &ProviderRegistry, reference: &ModelRef) -> Result<Entry, OpsError> {
    // Look up the exact key instead of constructing a combined id: both keys
    // and model ids may themselves contain the registry separator.
    let provider = registry.provider(&reference.key).map_err(OpsError::AiMux)?;
    let id = reference.model_id.as_str();
    let result = match reference.method {
        ModelMethod::LanguageModel => provider.language_model(id).map(Entry::Language),
        ModelMethod::EmbeddingModel => provider.embedding_model(id).map(Entry::Embedding),
        ModelMethod::ImageModel => provider.image_model(id).map(Entry::Image),
        ModelMethod::SpeechModel => provider
            .speech_model(id)
            .unwrap_or_else(|| Err(AiMuxError::no_such_model(id, "speechModel")))
            .map(Entry::Speech),
        ModelMethod::TranscriptionModel => provider
            .transcription_model(id)
            .unwrap_or_else(|| Err(AiMuxError::no_such_model(id, "transcriptionModel")))
            .map(Entry::Transcription),
        ModelMethod::RerankingModel => provider
            .reranking_model(id)
            .unwrap_or_else(|| Err(AiMuxError::no_such_model(id, "rerankingModel")))
            .map(Entry::Reranking),
        ModelMethod::VideoModel => provider
            .video_model(id)
            .unwrap_or_else(|| Err(AiMuxError::no_such_model(id, "videoModel")))
            .map(Entry::Video),
        ModelMethod::SearchModel => provider
            .search_model(id)
            .unwrap_or_else(|| Err(AiMuxError::no_such_model(id, "searchModel")))
            .map(Entry::Search),
        ModelMethod::Decision => provider
            .decision_model(id)
            .unwrap_or_else(|| Err(AiMuxError::no_such_model(id, "decisionModel")))
            .map(Entry::Decision),
    };
    result.map_err(OpsError::AiMux)
}

fn info(ctx: Ctx, target: Target, params: Value) -> CallFuture {
    Box::pin(async move {
        let entry = target_entry(target, MODELS)?;
        let _: ModelInfoParams = decode(params)?;
        model_info(ctx.owner, &entry)
    })
}

fn model_info(owner: Owner, entry: &Entry) -> Result<Value, OpsError> {
    let source = match metadata(owner, entry, None) {
        Some(Metadata::Sources(references)) => json!({"references": references}),
        _ => Value::Null,
    };
    let (provider, model_id, modality, capabilities) = match entry {
        Entry::Language(m) => (
            m.provider(),
            m.model_id(),
            "language",
            json!({"generate": true, "stream": true}),
        ),
        Entry::Embedding(m) => (
            m.provider(),
            m.model_id(),
            "embedding",
            json!({"embed": true, "maxEmbeddingsPerCall": m.max_embeddings_per_call(), "supportsParallelCalls": m.supports_parallel_calls()}),
        ),
        Entry::Image(m) => (
            m.provider(),
            m.model_id(),
            "image",
            json!({"generate": true, "maxImagesPerCall": m.max_images_per_call(), "supportsFileInputs": m.supports_file_inputs(), "supportsMaskInputs": m.supports_mask_inputs()}),
        ),
        Entry::Speech(m) => (
            m.provider(),
            m.model_id(),
            "speech",
            json!({"generate": true}),
        ),
        Entry::Transcription(m) => (
            m.provider(),
            m.model_id(),
            "transcription",
            json!({"generate": true}),
        ),
        Entry::Reranking(m) => (
            m.provider(),
            m.model_id(),
            "reranking",
            json!({"rerank": true}),
        ),
        Entry::Video(m) => (
            m.provider(),
            m.model_id(),
            "video",
            json!({"start": true, "status": true, "maxVideosPerCall": m.max_videos_per_call()}),
        ),
        Entry::Search(m) => (
            m.provider(),
            m.model_id(),
            "search",
            json!({"search": true}),
        ),
        Entry::Decision(m) => (
            m.provider(),
            m.model_id(),
            "decision",
            json!({"decide": true, "supportedQuestionTypes": m.supported_question_types()}),
        ),
        _ => return Err(wrong(entry, MODELS)),
    };
    Ok(json!(ModelInfo {
        provider,
        model_id,
        modality,
        capabilities,
        source
    }))
}

fn wrap(ctx: Ctx, target: Target, params: Value) -> CallFuture {
    Box::pin(async move {
        let entry = target_entry(target, MODELS)?;
        let params: ModelWrapParams = decode(params)?;
        // Resolve the complete chain before any middleware hook can run.
        let mut middleware = Vec::with_capacity(params.middleware.len());
        for id in &params.middleware {
            let resolved = resolve_secondary(
                HandleRegistry::global(),
                &ctx,
                &json!(handle(id)?),
                &[HandleKind::Middleware],
            )?;
            let Entry::Middleware(value) = resolved else {
                return Err(wrong(&resolved, &[HandleKind::Middleware]));
            };
            middleware.push(value);
        }
        let sources = metadata(ctx.owner, &entry, None);
        let entry = if middleware.is_empty() {
            entry
        } else {
            match entry {
                Entry::Language(model) => {
                    let chain = middleware
                        .iter()
                        .map(|value| match value.as_ref() {
                            Middleware::Language(value) => Ok(Arc::clone(value)),
                            Middleware::Image(_) => {
                                Err(invalid("image middleware cannot wrap a language model"))
                            }
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    Entry::Language(aimux_core::language_model_middleware::wrap_language_model(
                        model, &chain, None, None,
                    ))
                }
                Entry::Image(model) => {
                    let chain = middleware
                        .iter()
                        .map(|value| match value.as_ref() {
                            Middleware::Image(value) => Ok(Arc::clone(value)),
                            Middleware::Language(_) => {
                                Err(invalid("language middleware cannot wrap an image model"))
                            }
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    Entry::Image(aimux_core::image_model_middleware::wrap_image_model(
                        model, &chain, None, None,
                    ))
                }
                _ => {
                    return Err(AiMuxError::UnsupportedFunctionality(
                        "middleware is only available for language and image models".into(),
                    )
                    .into());
                }
            }
        };
        if let Some(sources) = sources {
            metadata(ctx.owner, &entry, Some(sources));
        }
        insert(ctx.owner, entry)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use aimux_core::custom_provider::{CustomProviderOptions, custom_provider};
    use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel, EmbeddingResult};

    struct Embedding;

    // Spell out the async-trait ABI so this module needs no extra dependency.
    impl EmbeddingModel for Embedding {
        fn provider(&self) -> &str {
            "test"
        }
        fn model_id(&self) -> &str {
            "actual:model:id"
        }
        fn max_embeddings_per_call(&self) -> Option<u32> {
            Some(8)
        }
        fn supports_parallel_calls(&self) -> bool {
            true
        }
        fn do_embed<'a, 'b, 'f>(
            &'a self,
            _: &'b EmbeddingCallOptions,
        ) -> futures::future::BoxFuture<'f, Result<EmbeddingResult, AiMuxError>>
        where
            'a: 'f,
            'b: 'f,
            Self: 'f,
        {
            Box::pin(async { Err(AiMuxError::UnsupportedFunctionality("unused".into())) })
        }
    }

    fn owner() -> Owner {
        Owner::new_node().unwrap()
    }

    fn ctx(owner: Owner) -> Ctx {
        Ctx {
            owner,
            abort: None,
            now: std::time::SystemTime::now,
        }
    }

    fn fixture(owner: Owner) -> (HandleId, Entry) {
        let options = CustomProviderOptions {
            embedding_models: std::collections::HashMap::from([(
                "alias:with:colons".into(),
                Arc::new(Embedding) as Arc<dyn EmbeddingModel>,
            )]),
            ..Default::default()
        };
        let provider = Entry::Provider(Arc::new(custom_provider(options)));
        let id = HandleRegistry::global()
            .insert(owner, provider.clone())
            .unwrap();
        (id, provider)
    }

    fn result_entry(owner: Owner, result: &Value, kind: HandleKind) -> Entry {
        HandleRegistry::global()
            .get(
                owner,
                handle(result["handle"].as_str().unwrap()).unwrap(),
                &[kind],
            )
            .unwrap()
    }

    async fn registry(owner: Owner, provider: &HandleId) -> Entry {
        let result = create(
            ctx(owner),
            Target::Global,
            json!({"namespace": "tenant", "entries": [
                {"key": "key:with:colon", "provider": provider.0.to_string()}
            ]}),
        )
        .await
        .unwrap();
        result_entry(owner, &result, HandleKind::Registry)
    }

    fn reference(namespace: &str, key: &str, method: &str, model_id: &str) -> Value {
        json!({"modelRef": {"namespace": namespace, "key": key, "method": method, "modelId": model_id}})
    }

    #[test]
    fn control_objects_reject_unknown_fields_and_numeric_handles() {
        assert!(decode::<RegistryCreateParams>(json!({"entries": [], "settings": {}})).is_err());
        assert!(
            decode::<RegistryEntry>(json!({"key": "x", "provider": "1", "secret": "x"})).is_err()
        );
        assert!(decode::<RegistryProviderParams>(json!({"key": "x", "extra": true})).is_err());
        assert!(decode::<ModelInfoParams>(json!({"settings": true})).is_err());
        assert!(decode::<ModelWrapParams>(json!({"middleware": [], "extra": 1})).is_err());
        assert!(decode::<RegistryEntry>(json!({"key": "x", "provider": 1})).is_err());
        assert!(decode::<ModelResolveParams>(json!({"modelRef": {"namespace": "x", "key": "x", "method": "embeddingModel", "modelId": "m", "settings": {}}})).is_err());
    }

    #[test]
    fn handles_are_canonical_decimal_strings() {
        for bad in ["", " 1", "+1", "-1", "01", "1.0", "18446744073709551616"] {
            assert!(handle(bad).is_err(), "{bad}");
        }
        assert_eq!(handle("18446744073709551615").unwrap().0, u64::MAX);
    }

    #[tokio::test]
    async fn duplicate_keys_are_rejected() {
        let owner = owner();
        let (id, _entry) = fixture(owner);
        let result = create(
            ctx(owner),
            Target::Global,
            json!({"entries": [
                {"key": "same", "provider": id.0.to_string()},
                {"key": "same", "provider": id.0.to_string()}
            ]}),
        )
        .await;
        assert!(matches!(
            result,
            Err(OpsError::Protocol(ProtocolError::InvalidParams(_)))
        ));
        drop(HandleRegistry::global().remove(owner, id));
    }

    #[tokio::test]
    async fn provider_reference_is_owner_checked() {
        let owner = owner();
        let (id, _entry) = fixture(owner);
        let result = create(
            ctx(Owner::Python(999)),
            Target::Global,
            json!({"entries": [
                {"key": "x", "provider": id.0.to_string()}
            ]}),
        )
        .await;
        assert!(matches!(
            result,
            Err(OpsError::Binding(BindingError::InvalidHandle { .. }))
        ));
        drop(HandleRegistry::global().remove(owner, id));
    }

    #[tokio::test]
    async fn registry_retains_exact_provider_after_original_handle_drop() {
        let owner = owner();
        let (id, original) = fixture(owner);
        let registry = registry(owner, &id).await;
        drop(HandleRegistry::global().remove(owner, id));
        let result = provider(
            ctx(owner),
            Target::Handle(registry),
            json!({"key": "key:with:colon"}),
        )
        .await
        .unwrap();
        let returned = result_entry(owner, &result, HandleKind::Provider);
        let (Entry::Provider(a), Entry::Provider(b)) = (original, returned) else {
            panic!()
        };
        assert!(Arc::ptr_eq(&a, &b));
    }

    #[tokio::test]
    async fn resolution_preserves_exact_ids_and_reports_safe_source() {
        let owner = owner();
        let (id, _input) = fixture(owner);
        let registry = registry(owner, &id).await;
        let params = reference(
            "tenant",
            "key:with:colon",
            "embeddingModel",
            "alias:with:colons",
        );
        let result = resolve(ctx(owner), Target::Handle(registry), params.clone())
            .await
            .unwrap();
        let model = result_entry(owner, &result, HandleKind::Embedding);
        let info = info(ctx(owner), Target::Handle(model), json!({}))
            .await
            .unwrap();
        assert_eq!(info["modelId"], "actual:model:id");
        assert_eq!(info["provider"], "test");
        assert_eq!(info["modality"], "embedding");
        assert_eq!(info["capabilities"]["maxEmbeddingsPerCall"], 8);
        assert_eq!(info["source"]["references"], json!([params["modelRef"]]));
        assert_eq!(info.as_object().unwrap().len(), 5);
    }

    #[tokio::test]
    async fn missing_namespace_key_model_and_modality_do_not_fall_back() {
        let owner = owner();
        let (id, _input) = fixture(owner);
        let registry = registry(owner, &id).await;
        assert!(
            resolve(
                ctx(owner),
                Target::Handle(registry.clone()),
                reference(
                    "other",
                    "key:with:colon",
                    "embeddingModel",
                    "alias:with:colons"
                )
            )
            .await
            .is_err()
        );
        for (key, method, model) in [
            ("missing", "embeddingModel", "alias:with:colons"),
            ("key:with:colon", "embeddingModel", "missing"),
            ("key:with:colon", "speechModel", "alias:with:colons"),
            ("key:with:colon", "languageModel", "alias:with:colons"),
            ("key:with:colon", "imageModel", "alias:with:colons"),
            ("key:with:colon", "transcriptionModel", "alias:with:colons"),
            ("key:with:colon", "rerankingModel", "alias:with:colons"),
            ("key:with:colon", "videoModel", "alias:with:colons"),
            ("key:with:colon", "searchModel", "alias:with:colons"),
            ("key:with:colon", "decision", "alias:with:colons"),
        ] {
            assert!(matches!(
                resolve(
                    ctx(owner),
                    Target::Handle(registry.clone()),
                    reference("tenant", key, method, model)
                )
                .await,
                Err(OpsError::AiMux(_))
            ));
        }
    }

    #[tokio::test]
    async fn direct_models_have_no_invented_source_and_empty_wrap_retains_modality() {
        let owner = owner();
        let model = Entry::Embedding(Arc::new(Embedding));
        assert_eq!(model_info(owner, &model).unwrap()["source"], Value::Null);
        let result = wrap(ctx(owner), Target::Handle(model), json!({"middleware": []}))
            .await
            .unwrap();
        assert_eq!(
            result_entry(owner, &result, HandleKind::Embedding).kind(),
            HandleKind::Embedding
        );
    }

    #[tokio::test]
    async fn shared_model_sources_are_accumulated_and_wrap_preserves_them() {
        let owner = owner();
        let (id, _input) = fixture(owner);
        let first = registry(owner, &id).await;
        let second = create(
            ctx(owner),
            Target::Global,
            json!({"namespace": "other", "entries": [
                {"key": "second", "provider": id.0.to_string()}
            ]}),
        )
        .await
        .unwrap();
        let second = result_entry(owner, &second, HandleKind::Registry);
        let a = reference(
            "tenant",
            "key:with:colon",
            "embeddingModel",
            "alias:with:colons",
        );
        let b = reference("other", "second", "embeddingModel", "alias:with:colons");
        let result = resolve(ctx(owner), Target::Handle(first.clone()), a.clone())
            .await
            .unwrap();
        let model = result_entry(owner, &result, HandleKind::Embedding);
        resolve(ctx(owner), Target::Handle(first), a.clone())
            .await
            .unwrap();
        resolve(ctx(owner), Target::Handle(second), b.clone())
            .await
            .unwrap();
        let wrapped = wrap(ctx(owner), Target::Handle(model), json!({"middleware": []}))
            .await
            .unwrap();
        let wrapped = result_entry(owner, &wrapped, HandleKind::Embedding);
        assert_eq!(
            model_info(owner, &wrapped).unwrap()["source"]["references"],
            json!([a["modelRef"], b["modelRef"]])
        );
        assert_eq!(
            model_info(Owner::Python(999), &wrapped).unwrap()["source"],
            Value::Null
        );
    }

    #[tokio::test]
    async fn wrap_rejects_invalid_middleware_and_wrong_primary_kind() {
        let owner = owner();
        let (id, input) = fixture(owner);
        assert!(matches!(
            wrap(
                ctx(owner),
                Target::Handle(Entry::Embedding(Arc::new(Embedding))),
                json!({"middleware": [id.0.to_string()]})
            )
            .await,
            Err(OpsError::Binding(BindingError::InvalidHandle { .. }))
        ));
        assert!(matches!(
            info(ctx(owner), Target::Handle(input.clone()), json!({})).await,
            Err(OpsError::Binding(_))
        ));
        assert!(matches!(
            wrap(ctx(owner), Target::Handle(input), json!({"middleware": []})).await,
            Err(OpsError::Binding(_))
        ));
    }
}
