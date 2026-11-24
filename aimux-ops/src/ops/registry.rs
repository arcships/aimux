//! Explicit provider registries and model references. Resolution never uses a gateway.

use crate::dispatch::serialize_result;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use aimux_core::error::AiMuxError;
use aimux_core::provider::{LanguageApi, Provider};
use aimux_core::provider_registry::{
    ProviderRegistry, ProviderRegistryOptions, create_provider_registry,
};
use aimux_providers::{EnvLookupPolicy, ProviderConfigDocument, providers_from_config_with_policy};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::dispatch::{CallFuture, Ctx, Handler, Params, Target, decode};
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
            b"registry.create" => &[(&["entries", "*", "provider"], &[HandleKind::Provider])],
            b"registry.update" => &[(&["set", "*", "provider"], &[HandleKind::Provider])],
            b"model.wrap" => &[(&["middleware", "*"], &[HandleKind::Middleware])],
            _ => &[],
        },
        required_capabilities: &[],
        cancellation: Cancellation::None,
        result_ownership: "owned",
        request_byte_policy: RequestBytePolicy::Json,
        side_effects: matches!(
            name.as_bytes(),
            b"registry.create" | b"registry.fromConfig" | b"registry.update"
        ),
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
        "registry.fromConfig",
        &[],
        TypeRef::of::<RegistryFromConfigParams>(),
        TypeRef::of::<HandleResult>(),
        &[
            "aimux_providers::providers_from_config_with_policy",
            "aimux_core::provider_registry::create_provider_registry",
        ],
    ),
    descriptor(
        "registry.list",
        &[HandleKind::Registry],
        TypeRef::of::<RegistryListParams>(),
        TypeRef::of::<RegistryListResult>(),
        &["aimux_core::provider_registry::ProviderRegistry::provider_ids"],
    ),
    descriptor(
        "registry.provider",
        &[HandleKind::Registry],
        TypeRef::of::<RegistryProviderParams>(),
        TypeRef::of::<HandleResult>(),
        &["aimux_core::provider_registry::ProviderRegistry::provider"],
    ),
    descriptor(
        "registry.update",
        &[HandleKind::Registry],
        TypeRef::of::<RegistryUpdateParams>(),
        TypeRef::of::<RegistryUpdateResult>(),
        &[
            "aimux_core::provider_registry::ProviderRegistry::update_providers",
            "aimux_core::provider_registry::ProviderRegistry::with_updated_providers",
        ],
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
        create as crate::dispatch::CallHandler,
        from_config,
        list,
        provider,
        update,
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
    #[serde(default)]
    separator: Option<String>,
    entries: Vec<RegistryEntry>,
}

/// Trusted-caller operation: constructs a provider registry from a declarative
/// configuration document and optional `{ "env": ... }` references. Untrusted
/// callers must not be permitted to supply arbitrary `baseUrl` values with
/// omitted credentials.
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RegistryFromConfigParams {
    #[serde(default)]
    namespace: String,
    #[serde(default)]
    separator: Option<String>,
    /// Optional allowlist gating explicit `{ "env": "VAR" }` (`StringOrEnv`)
    /// interpolation inside `config`. Does not restrict implicit per-provider
    /// default environment variable fallbacks when credential or `baseUrl`
    /// settings are omitted.
    #[serde(default)]
    env_allowlist: Option<Vec<String>>,
    config: ProviderConfigDocument,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct RegistryListParams {}

#[derive(Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct RegistryListResult {
    namespace: String,
    separator: String,
    keys: Vec<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RegistryUpdateParams {
    #[serde(default)]
    set: Vec<RegistryEntry>,
    #[serde(default)]
    remove: Vec<String>,
    #[serde(default)]
    replace: bool,
    #[serde(default)]
    derive: bool,
}

#[derive(Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct RegistryUpdateResult {
    keys: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    handle: Option<HandleId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    kind: Option<HandleKind>,
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
    Chat,
    Responses,
    Completion,
    Messages,
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
    OpsError::Protocol(ProtocolError::invalid_params(message))
}

fn handle(value: &str) -> Result<HandleId, OpsError> {
    value.parse().map_err(invalid)
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
    #[schemars(with = "Value")]
    capabilities: ModelCapabilities<'a>,
    #[schemars(with = "Value")]
    source: Option<ModelSource>,
}

#[derive(Serialize)]
#[serde(untagged, rename_all_fields = "camelCase")]
enum ModelCapabilities<'a> {
    Language {
        generate: bool,
        stream: bool,
    },
    Embedding {
        embed: bool,
        max_embeddings_per_call: Option<u32>,
        supports_parallel_calls: bool,
    },
    Image {
        generate: bool,
        max_images_per_call: Option<u32>,
        supports_file_inputs: Option<bool>,
        supports_mask_inputs: Option<bool>,
    },
    Generate {
        generate: bool,
    },
    Reranking {
        rerank: bool,
    },
    Video {
        start: bool,
        status: bool,
        max_videos_per_call: Option<u32>,
    },
    Search {
        search: bool,
    },
    Decision {
        decide: bool,
        supported_question_types: &'a [aimux_core::decision_model::DecisionQuestionType],
    },
}

#[derive(Serialize)]
struct ModelSource {
    references: Vec<ModelRef>,
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

fn insert(owner: Owner, entry: Entry) -> Result<String, OpsError> {
    let kind = entry.kind();
    let id = HandleRegistry::global().insert(owner, entry)?;
    serialize_result(HandleResult { handle: id, kind })
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

fn parse_separator(separator: Option<String>) -> Result<String, OpsError> {
    match separator {
        Some(sep) if sep.is_empty() => Err(invalid("registry separator must not be empty")),
        Some(sep) => Ok(sep),
        None => Ok(ProviderRegistryOptions::default().separator),
    }
}

fn resolve_entry_list(
    ctx: &Ctx,
    entries: Vec<RegistryEntry>,
) -> Result<BTreeMap<String, Arc<dyn Provider>>, OpsError> {
    let mut providers = BTreeMap::new();
    for registration in entries {
        if registration.key.is_empty() {
            return Err(invalid("registry entry key must not be empty"));
        }
        if providers.contains_key(&registration.key) {
            return Err(invalid(format!(
                "duplicate registry key: {}",
                registration.key
            )));
        }
        let provider = HandleRegistry::global().get(
            ctx.owner,
            handle(&registration.provider)?,
            &[HandleKind::Provider],
        )?;
        let Entry::Provider(provider) = provider else {
            return Err(wrong(&provider, &[HandleKind::Provider]));
        };
        providers.insert(registration.key, provider);
    }
    Ok(providers)
}

fn create(ctx: Ctx, target: Target, params: Params<'_>) -> CallFuture<'_> {
    Box::pin(async move {
        if let Target::Handle(entry) = target {
            return Err(wrong(&entry, &[]));
        }
        let params: RegistryCreateParams = decode(params)?;
        let separator = parse_separator(params.separator)?;
        let providers = resolve_entry_list(&ctx, params.entries)?;
        let entry = Entry::Registry(Arc::new(create_provider_registry(
            providers,
            ProviderRegistryOptions {
                separator,
                ..ProviderRegistryOptions::default()
            },
        )));
        metadata(
            ctx.owner,
            &entry,
            Some(Metadata::Namespace(params.namespace)),
        );
        insert(ctx.owner, entry)
    })
}

fn from_config(ctx: Ctx, target: Target, params: Params<'_>) -> CallFuture<'_> {
    Box::pin(async move {
        if let Target::Handle(entry) = target {
            return Err(wrong(&entry, &[]));
        }
        let params: RegistryFromConfigParams = decode(params)?;
        let separator = parse_separator(params.separator)?;
        let policy = match params.env_allowlist {
            Some(vars) => EnvLookupPolicy::AllowList(vars.into_iter().collect()),
            None => EnvLookupPolicy::AllowAll,
        };
        let providers =
            providers_from_config_with_policy(&params.config, &policy).map_err(OpsError::AiMux)?;
        for (key, spec) in &params.config.providers {
            if let Some(provider) = providers.get(key) {
                crate::ops::provider::register_package_default_method(provider, &spec.package)?;
            }
        }
        let entry = Entry::Registry(Arc::new(create_provider_registry(
            providers,
            ProviderRegistryOptions {
                separator,
                ..ProviderRegistryOptions::default()
            },
        )));
        metadata(
            ctx.owner,
            &entry,
            Some(Metadata::Namespace(params.namespace)),
        );
        insert(ctx.owner, entry)
    })
}

fn list(ctx: Ctx, target: Target, params: Params<'_>) -> CallFuture<'_> {
    Box::pin(async move {
        let entry = target_entry(target, &[HandleKind::Registry])?;
        let _: RegistryListParams = decode(params)?;
        let Entry::Registry(registry) = &entry else {
            return Err(wrong(&entry, &[HandleKind::Registry]));
        };
        let namespace = match metadata(ctx.owner, &entry, None) {
            Some(Metadata::Namespace(namespace)) => namespace,
            _ => String::new(),
        };
        serialize_result(RegistryListResult {
            namespace,
            separator: registry.separator().to_string(),
            keys: registry.provider_ids(),
        })
    })
}

fn update(ctx: Ctx, target: Target, params: Params<'_>) -> CallFuture<'_> {
    Box::pin(async move {
        let entry = target_entry(target, &[HandleKind::Registry])?;
        let params: RegistryUpdateParams = decode(params)?;
        let Entry::Registry(registry) = &entry else {
            return Err(wrong(&entry, &[HandleKind::Registry]));
        };
        if params.replace && !params.remove.is_empty() {
            return Err(invalid("remove cannot be combined with replace"));
        }
        let mut remove_set = BTreeSet::new();
        for key in &params.remove {
            if key.is_empty() {
                return Err(invalid("registry removal key must not be empty"));
            }
            remove_set.insert(key.as_str());
        }
        let resolved_set = resolve_entry_list(&ctx, params.set)?;
        for key in resolved_set.keys() {
            if remove_set.contains(key.as_str()) {
                return Err(invalid(format!(
                    "key '{key}' cannot appear in both set and remove"
                )));
            }
        }
        if !params.derive {
            let keys = registry.update_providers(&params.remove, resolved_set, params.replace);
            return serialize_result(RegistryUpdateResult {
                keys,
                handle: None,
                kind: None,
            });
        }

        let derived = registry.with_updated_providers(&params.remove, resolved_set, params.replace);
        let keys = derived.provider_ids();
        let namespace = match metadata(ctx.owner, &entry, None) {
            Some(Metadata::Namespace(namespace)) => namespace,
            _ => String::new(),
        };
        let new_entry = Entry::Registry(Arc::new(derived));
        metadata(ctx.owner, &new_entry, Some(Metadata::Namespace(namespace)));
        let kind = new_entry.kind();
        let id = HandleRegistry::global().insert(ctx.owner, new_entry)?;
        serialize_result(RegistryUpdateResult {
            keys,
            handle: Some(id),
            kind: Some(kind),
        })
    })
}

fn provider(ctx: Ctx, target: Target, params: Params<'_>) -> CallFuture<'_> {
    Box::pin(async move {
        let entry = target_entry(target, &[HandleKind::Registry])?;
        let params: RegistryProviderParams = decode(params)?;
        let Entry::Registry(registry) = &entry else {
            return Err(wrong(&entry, &[HandleKind::Registry]));
        };
        let provider = registry.provider(&params.key).map_err(OpsError::AiMux)?;
        insert(ctx.owner, Entry::Provider(provider))
    })
}

fn resolve(ctx: Ctx, target: Target, params: Params<'_>) -> CallFuture<'_> {
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

fn unsupported(method: &str) -> AiMuxError {
    AiMuxError::UnsupportedFunctionality(format!("provider does not support {method}"))
}

fn offered_entry<T>(
    model: Option<Result<T, AiMuxError>>,
    method: &str,
    wrap: impl FnOnce(T) -> Entry,
) -> Result<Entry, AiMuxError> {
    model.ok_or_else(|| unsupported(method))?.map(wrap)
}

fn resolve_model(registry: &ProviderRegistry, reference: &ModelRef) -> Result<Entry, OpsError> {
    // Look up the exact key instead of constructing a combined id: both keys
    // and model ids may themselves contain the registry separator.
    let provider = registry.provider(&reference.key).map_err(OpsError::AiMux)?;
    let id = reference.model_id.as_str();
    let result = match reference.method {
        ModelMethod::LanguageModel => provider.language_model(id).map(Entry::Language),
        ModelMethod::Chat => offered_entry(
            provider.language_model_with_api(LanguageApi::Chat, id),
            "chat",
            Entry::Language,
        ),
        ModelMethod::Responses => offered_entry(
            provider.language_model_with_api(LanguageApi::Responses, id),
            "responses",
            Entry::Language,
        ),
        ModelMethod::Completion => offered_entry(
            provider.language_model_with_api(LanguageApi::Completion, id),
            "completion",
            Entry::Language,
        ),
        ModelMethod::Messages => offered_entry(
            provider.language_model_with_api(LanguageApi::Messages, id),
            "messages",
            Entry::Language,
        ),
        ModelMethod::EmbeddingModel => provider.embedding_model(id).map(Entry::Embedding),
        ModelMethod::ImageModel => provider.image_model(id).map(Entry::Image),
        ModelMethod::SpeechModel => {
            offered_entry(provider.speech_model(id), "speechModel", Entry::Speech)
        }
        ModelMethod::TranscriptionModel => offered_entry(
            provider.transcription_model(id),
            "transcriptionModel",
            Entry::Transcription,
        ),
        ModelMethod::RerankingModel => offered_entry(
            provider.reranking_model(id),
            "rerankingModel",
            Entry::Reranking,
        ),
        ModelMethod::VideoModel => {
            offered_entry(provider.video_model(id), "videoModel", Entry::Video)
        }
        ModelMethod::SearchModel => {
            offered_entry(provider.search_model(id), "searchModel", Entry::Search)
        }
        ModelMethod::Decision => {
            offered_entry(provider.decision_model(id), "decision", Entry::Decision)
        }
    };
    result.map_err(OpsError::AiMux)
}

fn info(ctx: Ctx, target: Target, params: Params<'_>) -> CallFuture<'_> {
    Box::pin(async move {
        let entry = target_entry(target, MODELS)?;
        let _: ModelInfoParams = decode(params)?;
        model_info(ctx.owner, &entry)
    })
}

fn model_info(owner: Owner, entry: &Entry) -> Result<String, OpsError> {
    let source = match metadata(owner, entry, None) {
        Some(Metadata::Sources(references)) => Some(ModelSource { references }),
        _ => None,
    };
    let (provider, model_id, modality, capabilities) = match entry {
        Entry::Language(m) => (
            m.provider(),
            m.model_id(),
            "language",
            ModelCapabilities::Language {
                generate: true,
                stream: true,
            },
        ),
        Entry::Embedding(m) => (
            m.provider(),
            m.model_id(),
            "embedding",
            ModelCapabilities::Embedding {
                embed: true,
                max_embeddings_per_call: m.max_embeddings_per_call(),
                supports_parallel_calls: m.supports_parallel_calls(),
            },
        ),
        Entry::Image(m) => (
            m.provider(),
            m.model_id(),
            "image",
            ModelCapabilities::Image {
                generate: true,
                max_images_per_call: m.max_images_per_call(),
                supports_file_inputs: m.supports_file_inputs(),
                supports_mask_inputs: m.supports_mask_inputs(),
            },
        ),
        Entry::Speech(m) => (
            m.provider(),
            m.model_id(),
            "speech",
            ModelCapabilities::Generate { generate: true },
        ),
        Entry::Transcription(m) => (
            m.provider(),
            m.model_id(),
            "transcription",
            ModelCapabilities::Generate { generate: true },
        ),
        Entry::Reranking(m) => (
            m.provider(),
            m.model_id(),
            "reranking",
            ModelCapabilities::Reranking { rerank: true },
        ),
        Entry::Video(m) => (
            m.provider(),
            m.model_id(),
            "video",
            ModelCapabilities::Video {
                start: true,
                status: true,
                max_videos_per_call: m.max_videos_per_call(),
            },
        ),
        Entry::Search(m) => (
            m.provider(),
            m.model_id(),
            "search",
            ModelCapabilities::Search { search: true },
        ),
        Entry::Decision(m) => (
            m.provider(),
            m.model_id(),
            "decision",
            ModelCapabilities::Decision {
                decide: true,
                supported_question_types: m.supported_question_types(),
            },
        ),
        _ => return Err(wrong(entry, MODELS)),
    };
    serialize_result(ModelInfo {
        provider,
        model_id,
        modality,
        capabilities,
        source,
    })
}

fn wrap(ctx: Ctx, target: Target, params: Params<'_>) -> CallFuture<'_> {
    Box::pin(async move {
        let entry = target_entry(target, MODELS)?;
        let params: ModelWrapParams = decode(params)?;
        // Resolve the complete chain before any middleware hook can run.
        let mut middleware = Vec::with_capacity(params.middleware.len());
        for id in &params.middleware {
            let resolved =
                HandleRegistry::global().get(ctx.owner, handle(id)?, &[HandleKind::Middleware])?;
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
    fn parse_result(json: String) -> Value {
        serde_json::from_str(&json).unwrap()
    }

    use super::*;
    use aimux_core::custom_provider::{CustomProviderOptions, custom_provider};
    use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel, EmbeddingResult};
    use serde_json::json;

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
        Ctx { owner, abort: None }
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
            ]})
            .into(),
        )
        .await
        .unwrap();
        result_entry(owner, &parse_result(result), HandleKind::Registry)
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
    fn handles_follow_the_handle_id_rule() {
        for bad in ["", " 1", "+1", "-1", "1.0", "18446744073709551616"] {
            assert!(handle(bad).is_err(), "{bad}");
        }
        assert_eq!(handle("01").unwrap().0, 1);
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
            ]})
            .into(),
        )
        .await;
        assert!(matches!(
            result,
            Err(OpsError::Protocol(ProtocolError::InvalidParams { .. }))
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
            ]})
            .into(),
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
            json!({"key": "key:with:colon"}).into(),
        )
        .await
        .unwrap();
        let returned = result_entry(owner, &parse_result(result), HandleKind::Provider);
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
        let result = resolve(ctx(owner), Target::Handle(registry), params.clone().into())
            .await
            .unwrap();
        let model = result_entry(owner, &parse_result(result), HandleKind::Embedding);
        let info = info(ctx(owner), Target::Handle(model), json!({}).into())
            .await
            .unwrap();
        let info: Value = serde_json::from_str(&info).unwrap();
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
                .into()
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
                    reference("tenant", key, method, model).into()
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
        assert_eq!(
            parse_result(model_info(owner, &model).unwrap())["source"],
            Value::Null
        );
        let result = wrap(
            ctx(owner),
            Target::Handle(model),
            json!({"middleware": []}).into(),
        )
        .await
        .unwrap();
        assert_eq!(
            result_entry(owner, &parse_result(result), HandleKind::Embedding).kind(),
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
            ]})
            .into(),
        )
        .await
        .unwrap();
        let second = result_entry(owner, &parse_result(second), HandleKind::Registry);
        let a = reference(
            "tenant",
            "key:with:colon",
            "embeddingModel",
            "alias:with:colons",
        );
        let b = reference("other", "second", "embeddingModel", "alias:with:colons");
        let result = resolve(ctx(owner), Target::Handle(first.clone()), a.clone().into())
            .await
            .unwrap();
        let model = result_entry(owner, &parse_result(result), HandleKind::Embedding);
        resolve(ctx(owner), Target::Handle(first), a.clone().into())
            .await
            .unwrap();
        resolve(ctx(owner), Target::Handle(second), b.clone().into())
            .await
            .unwrap();
        let wrapped = wrap(
            ctx(owner),
            Target::Handle(model),
            json!({"middleware": []}).into(),
        )
        .await
        .unwrap();
        let wrapped = result_entry(owner, &parse_result(wrapped), HandleKind::Embedding);
        assert_eq!(
            parse_result(model_info(owner, &wrapped).unwrap())["source"]["references"],
            json!([a["modelRef"], b["modelRef"]])
        );
        assert_eq!(
            parse_result(model_info(Owner::Python(999), &wrapped).unwrap())["source"],
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
                json!({"middleware": [id.0.to_string()]}).into()
            )
            .await,
            Err(OpsError::Binding(BindingError::InvalidHandle { .. }))
        ));
        assert!(matches!(
            info(ctx(owner), Target::Handle(input.clone()), json!({}).into()).await,
            Err(OpsError::Binding(_))
        ));
        assert!(matches!(
            wrap(
                ctx(owner),
                Target::Handle(input),
                json!({"middleware": []}).into()
            )
            .await,
            Err(OpsError::Binding(_))
        ));
    }

    #[tokio::test]
    async fn from_config_list_update_separator_and_chat_method_resolution_work() {
        let owner = owner();
        let created = from_config(
            ctx(owner),
            Target::Global,
            json!({
                "namespace": "cfg",
                "separator": "/",
                "config": {
                    "providers": {
                        "gw": {
                            "package": "openai_compatible",
                            "settings": {
                                "baseUrl": "https://gw.example/v1",
                                "apiKey": "literal-key"
                            }
                        }
                    }
                }
            })
            .into(),
        )
        .await
        .unwrap();
        let created = parse_result(created);
        let reg = result_entry(owner, &created, HandleKind::Registry);
        let listed = parse_result(
            list(ctx(owner), Target::Handle(reg.clone()), json!({}).into())
                .await
                .unwrap(),
        );
        assert_eq!(listed["namespace"], "cfg");
        assert_eq!(listed["separator"], "/");
        assert_eq!(listed["keys"], json!(["gw"]));

        // Direct sub-method resolution (`method: "chat"`) on config-built provider.
        let model_ref = json!({
            "modelRef": {
                "namespace": "cfg",
                "key": "gw",
                "method": "chat",
                "modelId": "llama3"
            }
        });
        let resolved = parse_result(
            resolve(
                ctx(owner),
                Target::Handle(reg.clone()),
                model_ref.clone().into(),
            )
            .await
            .unwrap(),
        );
        let model = result_entry(owner, &resolved, HandleKind::Language);
        let model_meta = parse_result(
            info(ctx(owner), Target::Handle(model), json!({}).into())
                .await
                .unwrap(),
        );
        assert_eq!(model_meta["provider"], "gw.chat");
        assert_eq!(
            model_meta["source"]["references"],
            json!([model_ref["modelRef"]])
        );

        // In-place update (`set` + `remove`).
        let (extra_id, _) = fixture(owner);
        let in_place = parse_result(
            update(
                ctx(owner),
                Target::Handle(reg.clone()),
                json!({
                    "set": [{"key": "extra", "provider": extra_id.0.to_string()}],
                    "remove": ["gw"]
                })
                .into(),
            )
            .await
            .unwrap(),
        );
        assert_eq!(in_place["keys"], json!(["extra"]));
        assert!(in_place.get("handle").is_none());
        assert_eq!(
            parse_result(
                list(ctx(owner), Target::Handle(reg.clone()), json!({}).into())
                    .await
                    .unwrap()
            )["keys"],
            json!(["extra"])
        );

        // Derived update (`derive: true`) leaves original intact and preserves namespace.
        let (other_id, _) = fixture(owner);
        let derived = parse_result(
            update(
                ctx(owner),
                Target::Handle(reg.clone()),
                json!({
                    "set": [{"key": "replacement", "provider": other_id.0.to_string()}],
                    "replace": true,
                    "derive": true
                })
                .into(),
            )
            .await
            .unwrap(),
        );
        assert_eq!(derived["kind"], "Registry");
        assert_eq!(derived["keys"], json!(["replacement"]));
        let derived_reg = result_entry(owner, &derived, HandleKind::Registry);
        let derived_list = parse_result(
            list(
                ctx(owner),
                Target::Handle(derived_reg.clone()),
                json!({}).into(),
            )
            .await
            .unwrap(),
        );
        assert_eq!(derived_list["namespace"], "cfg");
        assert_eq!(derived_list["separator"], "/");
        assert_eq!(derived_list["keys"], json!(["replacement"]));
        assert_eq!(
            parse_result(
                list(ctx(owner), Target::Handle(reg), json!({}).into())
                    .await
                    .unwrap()
            )["keys"],
            json!(["extra"])
        );

        // Rejects empty separator and env var blocked by allowlist.
        assert!(matches!(
            create(
                ctx(owner),
                Target::Global,
                json!({"separator": "", "entries": []}).into()
            )
            .await,
            Err(OpsError::Protocol(ProtocolError::InvalidParams { .. }))
        ));
        assert!(matches!(
            from_config(
                ctx(owner),
                Target::Global,
                json!({
                    "envAllowlist": ["ALLOWED"],
                    "config": {
                        "providers": {
                            "bad": {
                                "package": "openai",
                                "settings": { "apiKey": { "env": "DISALLOWED" } }
                            }
                        }
                    }
                })
                .into()
            )
            .await,
            Err(OpsError::AiMux(AiMuxError::InvalidArgument { .. }))
        ));
        HandleRegistry::global().release_owner(owner);
    }

    #[tokio::test]
    async fn from_config_registers_default_method_sidecar_for_extracted_providers() {
        let owner = owner();
        let created = crate::dispatch::call(
            ctx(owner),
            HandleId(0),
            "registry.fromConfig",
            json!({
                "namespace": "cfg",
                "config": {
                    "providers": {
                        "gw": {
                            "package": "openai_compatible",
                            "settings": {
                                "baseUrl": "https://gw.example/v1",
                                "apiKey": "literal-key"
                            }
                        },
                        "search": {
                            "package": "tavily",
                            "settings": {
                                "apiKey": "tavily-key"
                            }
                        }
                    }
                }
            }),
        )
        .await
        .unwrap();
        let reg_id: HandleId = serde_json::from_value(created["handle"].clone()).unwrap();

        let gw_provider = crate::dispatch::call(
            ctx(owner),
            reg_id,
            "registry.provider",
            json!({"key": "gw"}),
        )
        .await
        .unwrap();
        let gw_id: HandleId = serde_json::from_value(gw_provider["handle"].clone()).unwrap();
        let gw_model = crate::dispatch::call(
            ctx(owner),
            gw_id,
            "provider.model",
            json!({"modelId": "llama3"}),
        )
        .await
        .unwrap();
        assert_eq!(gw_model["kind"], "Language");

        let wrapped = crate::dispatch::call(ctx(owner), gw_id, "provider.wrap", json!({}))
            .await
            .unwrap();
        let wrapped_id: HandleId = serde_json::from_value(wrapped["handle"].clone()).unwrap();
        let wrapped_model = crate::dispatch::call(
            ctx(owner),
            wrapped_id,
            "provider.model",
            json!({"modelId": "llama3"}),
        )
        .await
        .unwrap();
        assert_eq!(wrapped_model["kind"], "Language");

        let tavily_provider = crate::dispatch::call(
            ctx(owner),
            reg_id,
            "registry.provider",
            json!({"key": "search"}),
        )
        .await
        .unwrap();
        let tavily_id: HandleId =
            serde_json::from_value(tavily_provider["handle"].clone()).unwrap();
        let search_model = crate::dispatch::call(
            ctx(owner),
            tavily_id,
            "provider.model",
            json!({"modelId": "tavily-search"}),
        )
        .await
        .unwrap();
        assert_eq!(search_model["kind"], "Search");

        HandleRegistry::global().release_owner(owner);
    }

    #[tokio::test]
    async fn resolve_and_provider_model_share_unsupported_functionality_code() {
        let owner = owner();
        let created = crate::dispatch::call(
            ctx(owner),
            HandleId(0),
            "registry.fromConfig",
            json!({
                "namespace": "cfg",
                "config": {
                    "providers": {
                        "gw": {
                            "package": "openai_compatible",
                            "settings": {
                                "baseUrl": "https://gw.example/v1",
                                "apiKey": "literal-key"
                            }
                        }
                    }
                }
            }),
        )
        .await
        .unwrap();
        let reg_id: HandleId = serde_json::from_value(created["handle"].clone()).unwrap();
        let prov = crate::dispatch::call(
            ctx(owner),
            reg_id,
            "registry.provider",
            json!({"key": "gw"}),
        )
        .await
        .unwrap();
        let prov_id: HandleId = serde_json::from_value(prov["handle"].clone()).unwrap();

        for method in ["responses", "messages", "speechModel", "decision"] {
            let resolve_err = crate::dispatch::call(
                ctx(owner),
                reg_id,
                "model.resolve",
                reference("cfg", "gw", method, "m1"),
            )
            .await
            .unwrap_err();
            let provider_err = crate::dispatch::call(
                ctx(owner),
                prov_id,
                "provider.model",
                json!({"method": method, "modelId": "m1"}),
            )
            .await
            .unwrap_err();
            assert_eq!(resolve_err.code(), 8, "method {method}");
            assert_eq!(resolve_err.code(), provider_err.code(), "method {method}");
            assert_eq!(
                resolve_err.envelope().error["name"],
                provider_err.envelope().error["name"],
                "method {method}"
            );
        }

        HandleRegistry::global().release_owner(owner);
    }

    #[tokio::test]
    async fn update_rejects_partial_failures_and_swaps_atomically() {
        let owner = owner();
        let (p1, _) = fixture(owner);
        let (p2, _) = fixture(owner);
        let (p3, _) = fixture(owner);
        let not_a_provider = HandleRegistry::global()
            .insert(owner, Entry::Embedding(Arc::new(Embedding)))
            .unwrap();

        let created = parse_result(
            create(
                ctx(owner),
                Target::Global,
                json!({
                    "namespace": "ns",
                    "entries": [
                        {"key": "a", "provider": p1.0.to_string()},
                        {"key": "b", "provider": p1.0.to_string()}
                    ]
                })
                .into(),
            )
            .await
            .unwrap(),
        );
        let reg_id: HandleId = serde_json::from_value(created["handle"].clone()).unwrap();
        let reg_entry = result_entry(owner, &created, HandleKind::Registry);
        let Entry::Registry(reg_arc) = reg_entry.clone() else {
            panic!()
        };

        // Any failure in `set` or `remove` leaves prior state 100% untouched.
        for bad_params in [
            json!({
                "set": [
                    {"key": "c", "provider": p2.0.to_string()},
                    {"key": "", "provider": p3.0.to_string()}
                ],
                "remove": ["a"]
            }),
            json!({
                "set": [
                    {"key": "c", "provider": p2.0.to_string()},
                    {"key": "c", "provider": p3.0.to_string()}
                ],
                "remove": ["a"]
            }),
            json!({
                "set": [
                    {"key": "c", "provider": p2.0.to_string()},
                    {"key": "d", "provider": not_a_provider.0.to_string()}
                ],
                "remove": ["a"]
            }),
            json!({
                "set": [{"key": "c", "provider": p2.0.to_string()}],
                "remove": ["a", "c"]
            }),
            json!({
                "set": [{"key": "c", "provider": p2.0.to_string()}],
                "remove": ["a", ""]
            }),
            json!({
                "set": [{"key": "c", "provider": p2.0.to_string()}],
                "remove": ["a"],
                "replace": true
            }),
        ] {
            assert!(
                crate::dispatch::call(ctx(owner), reg_id, "registry.update", bad_params)
                    .await
                    .is_err()
            );
            assert_eq!(reg_arc.provider_ids(), vec!["a", "b"]);
            let Entry::Provider(p1_arc) = HandleRegistry::global()
                .get(owner, p1, &[HandleKind::Provider])
                .unwrap()
            else {
                unreachable!()
            };
            assert!(Arc::ptr_eq(&reg_arc.provider("a").unwrap(), &p1_arc));
        }

        // Concurrent readers never observe a torn intermediate state during remove + set.
        let writer_reg = reg_arc.clone();
        let p2_arc = match HandleRegistry::global()
            .get(owner, p2, &[HandleKind::Provider])
            .unwrap()
        {
            Entry::Provider(p) => p,
            _ => unreachable!(),
        };
        let reader_reg = reg_arc.clone();
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop_reader = stop.clone();
        let reader = std::thread::spawn(move || {
            while !stop_reader.load(std::sync::atomic::Ordering::Relaxed) {
                let ids = reader_reg.provider_ids();
                assert!(
                    ids == vec!["a", "b"] || ids == vec!["c", "d"],
                    "observed torn intermediate registry state: {ids:?}"
                );
            }
        });
        for i in 0..200 {
            if i % 2 == 0 {
                let set = BTreeMap::from([
                    ("c".to_string(), p2_arc.clone()),
                    ("d".to_string(), p2_arc.clone()),
                ]);
                let keys = writer_reg.update_providers(&["a".into(), "b".into()], set, false);
                assert_eq!(keys, vec!["c", "d"]);
            } else {
                let set = BTreeMap::from([
                    ("a".to_string(), p2_arc.clone()),
                    ("b".to_string(), p2_arc.clone()),
                ]);
                let keys = writer_reg.update_providers(&["c".into(), "d".into()], set, false);
                assert_eq!(keys, vec!["a", "b"]);
            }
        }
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        reader.join().unwrap();

        HandleRegistry::global().release_owner(owner);
    }
}
