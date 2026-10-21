//! Provider lifecycle and the existing model-discovery extension.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, Weak};

use aimux_core::custom_provider::{CustomProviderOptions, custom_provider};
use aimux_core::error::AiMuxError;
use aimux_core::image_model_middleware::ImageModelMiddleware;
use aimux_core::language_model_middleware::LanguageModelMiddleware;
use aimux_core::provider::Provider;
use aimux_core::wrap_provider::wrap_provider;
use aimux_provider_utils::Resolvable;
use aimux_providers::preset::PresetSettings;
use serde::Deserialize;
use serde_json::Value;

use crate::dispatch::{CallFuture, Ctx, Handler, Target, decode};
use crate::envelope::{BindingError, OpsError, ProtocolError};
use crate::handle::{Entry, HandleId, HandleKind, HandleRegistry, Middleware, Owner};
use crate::op::{Cancellation, OpDescriptor, OpMode, RequestBytePolicy, Scope, TypeRef};

const CREATE_HANDLES: &[(&str, &[HandleKind])] = &[
    ("settings.fetch", &[HandleKind::Fetch]),
    ("settings.webSocket", &[HandleKind::WebSocket]),
    (
        "settings.credentialResolver",
        &[HandleKind::CredentialResolver],
    ),
];
const INVOKE_HANDLES: &[(&str, &[HandleKind])] = &[("abort", &[HandleKind::Abort])];
const WRAP_HANDLES: &[(&str, &[HandleKind])] = &[
    ("languageModelMiddleware.*", &[HandleKind::Middleware]),
    ("imageModelMiddleware.*", &[HandleKind::Middleware]),
];
const CUSTOM_HANDLES: &[(&str, &[HandleKind])] = &[
    ("languageModels.*", &[HandleKind::Language]),
    ("embeddingModels.*", &[HandleKind::Embedding]),
    ("imageModels.*", &[HandleKind::Image]),
    ("speechModels.*", &[HandleKind::Speech]),
    ("transcriptionModels.*", &[HandleKind::Transcription]),
    ("rerankingModels.*", &[HandleKind::Reranking]),
    ("videoModels.*", &[HandleKind::Video]),
    ("searchModels.*", &[HandleKind::Search]),
    ("decisionModels.*", &[HandleKind::Decision]),
    ("files", &[HandleKind::Files]),
    ("fallbackProvider", &[HandleKind::Provider]),
    ("languageModelMiddleware.*", &[HandleKind::Middleware]),
    ("imageModelMiddleware.*", &[HandleKind::Middleware]),
];

#[allow(clippy::too_many_arguments)]
const fn descriptor(
    name: &'static str,
    kinds: &'static [HandleKind],
    request_type: TypeRef,
    result_type: TypeRef,
    side_effects: bool,
    cancellable: bool,
    source_symbols: &'static [&'static str],
    runtime_handle_fields: &'static [(&'static str, &'static [HandleKind])],
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
        runtime_handle_fields,
        required_capabilities: &[],
        cancellation: if cancellable {
            Cancellation::Abort
        } else {
            Cancellation::None
        },
        result_ownership: "owned",
        request_byte_policy: RequestBytePolicy::Json,
        side_effects,
        retry_owner: "none",
        source_symbols,
    }
}

pub(crate) static OPS: &[OpDescriptor] = &[
    descriptor(
        "provider.create",
        &[],
        TypeRef::of::<ProviderCreateParams>(),
        TypeRef::of::<HandleResult>(),
        true,
        false,
        &[
            "aimux_providers::create_provider",
            "aimux_providers::JevProvider::new",
        ],
        CREATE_HANDLES,
    ),
    descriptor(
        "provider.default",
        &[],
        TypeRef::of::<ProviderDefaultParams>(),
        TypeRef::of::<HandleResult>(),
        true,
        false,
        &[
            "aimux_providers::create_provider",
            "aimux_providers::JevProvider::new",
        ],
        &[],
    ),
    descriptor(
        "provider.model",
        &[HandleKind::Provider],
        TypeRef::of::<ProviderModelParams>(),
        TypeRef::of::<HandleResult>(),
        true,
        false,
        &["aimux_core::provider::Provider"],
        &[],
    ),
    descriptor(
        "provider.invoke",
        &[HandleKind::Provider],
        TypeRef::of::<ProviderInvokeParams>(),
        TypeRef::of::<ProviderInvokeResult>(),
        false,
        true,
        &["aimux_core::provider::ProviderDiscovery::list_models"],
        INVOKE_HANDLES,
    ),
    descriptor(
        "provider.wrap",
        &[HandleKind::Provider],
        TypeRef::of::<ProviderWrapParams>(),
        TypeRef::of::<HandleResult>(),
        true,
        false,
        &["aimux_core::wrap_provider::wrap_provider"],
        WRAP_HANDLES,
    ),
    descriptor(
        "provider.custom",
        &[],
        TypeRef::of::<ProviderCustomParams>(),
        TypeRef::of::<HandleResult>(),
        true,
        false,
        &["aimux_core::custom_provider::custom_provider"],
        CUSTOM_HANDLES,
    ),
];

pub(crate) fn register(r: &mut Vec<(&'static OpDescriptor, Handler)>) {
    let handlers: [fn(Ctx, Target, Value) -> CallFuture; 6] =
        [create, default_provider, model, invoke, wrap, custom];
    r.extend(
        OPS.iter()
            .zip(handlers)
            .map(|(op, handler)| (op, Handler::Call(handler))),
    );
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProviderCreateParams {
    package: String,
    #[serde(default)]
    settings: ProviderSettings,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProviderDefaultParams {
    package: String,
}

#[derive(Default, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProviderSettings {
    api_key: Option<String>,
    base_url: Option<String>,
    #[serde(default)]
    headers: HashMap<String, Option<String>>,
    organization: Option<String>,
    project: Option<String>,
    #[serde(default)]
    params: HashMap<String, String>,
    fetch: Option<HandleId>,
    web_socket: Option<HandleId>,
    credential_resolver: Option<HandleId>,
    endpoint: Option<String>,
    probability_source: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProviderModelParams {
    method: Option<String>,
    model_id: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProviderInvokeParams {
    method: String,
    #[serde(rename = "abort")]
    _abort: Option<HandleId>,
}

type ProviderInvokeResult = Vec<aimux_core::model_catalogue::RuntimeModel>;

#[derive(Default, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProviderWrapParams {
    #[serde(default)]
    language_model_middleware: Vec<HandleId>,
    #[serde(default)]
    image_model_middleware: Vec<HandleId>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProviderCustomParams {
    #[serde(default)]
    language_models: HashMap<String, HandleId>,
    #[serde(default)]
    embedding_models: HashMap<String, HandleId>,
    #[serde(default)]
    image_models: HashMap<String, HandleId>,
    #[serde(default)]
    speech_models: HashMap<String, HandleId>,
    #[serde(default)]
    transcription_models: HashMap<String, HandleId>,
    #[serde(default)]
    reranking_models: HashMap<String, HandleId>,
    #[serde(default)]
    video_models: HashMap<String, HandleId>,
    #[serde(default)]
    search_models: HashMap<String, HandleId>,
    #[serde(default)]
    decision_models: HashMap<String, HandleId>,
    files: Option<HandleId>,
    fallback_provider: Option<HandleId>,
    default_method: Option<String>,
    #[serde(default)]
    language_model_middleware: Vec<HandleId>,
    #[serde(default)]
    image_model_middleware: Vec<HandleId>,
}

#[derive(serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct HandleResult {
    handle: HandleId,
    kind: HandleKind,
}

#[derive(Clone, Copy)]
enum ModelMethod {
    Language,
    Embedding,
    Image,
    Speech,
    Transcription,
    Reranking,
    Video,
    Search,
    Decision,
}

impl ModelMethod {
    fn parse(method: &str) -> Result<Self, OpsError> {
        match method {
            "languageModel" => Ok(Self::Language),
            "embeddingModel" => Ok(Self::Embedding),
            "imageModel" => Ok(Self::Image),
            "speechModel" => Ok(Self::Speech),
            "transcriptionModel" => Ok(Self::Transcription),
            "rerankingModel" => Ok(Self::Reranking),
            "videoModel" => Ok(Self::Video),
            "searchModel" => Ok(Self::Search),
            "decision" => Ok(Self::Decision),
            _ => Err(unsupported(method)),
        }
    }
}

/// The Decision-only package provider.create builds itself rather than through the factory.
const JEV: &str = "jev";

/// Every package name provider.create accepts, sorted.
pub(crate) fn packages() -> Vec<&'static str> {
    let mut names = aimux_providers::provider_names();
    names.push(JEV);
    names.sort_unstable();
    names
}

// Entry::Provider holds the canonical trait object. Keep only weak, per-object
// descriptor metadata here; package configuration stays in aimux-providers.
struct ProviderDescriptor {
    provider: Weak<dyn Provider>,
    default_method: Option<ModelMethod>,
}

fn descriptors() -> &'static Mutex<Vec<ProviderDescriptor>> {
    static DESCRIPTORS: OnceLock<Mutex<Vec<ProviderDescriptor>>> = OnceLock::new();
    DESCRIPTORS.get_or_init(|| Mutex::new(Vec::new()))
}

fn default_method(provider: &Arc<dyn Provider>) -> Result<Option<ModelMethod>, OpsError> {
    let descriptors = descriptors().lock().map_err(|_| invalid_state())?;
    let weak = Arc::downgrade(provider);
    Ok(descriptors
        .iter()
        .find(|d| Weak::ptr_eq(&d.provider, &weak))
        .and_then(|d| d.default_method))
}

fn insert_provider(
    owner: Owner,
    provider: Arc<dyn Provider>,
    default_method: Option<ModelMethod>,
) -> Result<Value, OpsError> {
    {
        let mut descriptors = descriptors().lock().map_err(|_| invalid_state())?;
        descriptors.retain(|d| d.provider.strong_count() != 0);
        descriptors.push(ProviderDescriptor {
            provider: Arc::downgrade(&provider),
            default_method,
        });
    }
    insert(owner, Entry::Provider(provider))
}

fn invalid_state() -> OpsError {
    OpsError::Protocol(ProtocolError::InvalidState(
        "provider descriptor lock poisoned".into(),
    ))
}

fn unsupported(method: &str) -> OpsError {
    OpsError::AiMux(AiMuxError::UnsupportedFunctionality(format!(
        "provider does not support {method}"
    )))
}

fn invalid_handle(expected: &[HandleKind], actual: HandleKind) -> OpsError {
    OpsError::Binding(BindingError::InvalidHandle {
        expected: expected.to_vec(),
        actual: Some(actual),
    })
}

fn provider(target: Target) -> Result<Arc<dyn Provider>, OpsError> {
    match target {
        Target::Handle(Entry::Provider(provider)) => Ok(provider),
        Target::Handle(entry) => Err(invalid_handle(&[HandleKind::Provider], entry.kind())),
        Target::Global => Err(OpsError::Binding(BindingError::InvalidHandle {
            expected: vec![HandleKind::Provider],
            actual: None,
        })),
    }
}

fn insert(owner: Owner, entry: Entry) -> Result<Value, OpsError> {
    let kind = entry.kind();
    let handle = HandleRegistry::global().insert(owner, entry)?;
    serde_json::to_value(HandleResult { handle, kind })
        .map_err(|error| OpsError::Binding(BindingError::ResultSerialization(error.to_string())))
}

fn get(owner: Owner, handle: HandleId, kind: HandleKind) -> Result<Entry, OpsError> {
    HandleRegistry::global().get(owner, handle, &[kind])
}

fn create(ctx: Ctx, _entry: Target, params: Value) -> CallFuture {
    Box::pin(async move {
        let params: ProviderCreateParams = decode(params)?;
        create_package(ctx.owner, &params.package, params.settings)
    })
}

fn default_provider(ctx: Ctx, _entry: Target, params: Value) -> CallFuture {
    Box::pin(async move {
        let params: ProviderDefaultParams = decode(params)?;
        create_package(ctx.owner, &params.package, ProviderSettings::default())
    })
}

fn create_package(
    owner: Owner,
    package: &str,
    settings: ProviderSettings,
) -> Result<Value, OpsError> {
    // Resolve every runtime slot before constructing anything. A valid but
    // unavailable slot is a capability error, never silently ignored.
    let fetch = match settings.fetch {
        Some(handle) => match get(owner, handle, HandleKind::Fetch)? {
            Entry::Fetch(fetch) => Some(fetch),
            entry => return Err(invalid_handle(&[HandleKind::Fetch], entry.kind())),
        },
        None => None,
    };
    if let Some(handle) = settings.web_socket {
        get(owner, handle, HandleKind::WebSocket)?;
        return Err(unsupported("webSocket settings"));
    }
    if let Some(handle) = settings.credential_resolver {
        get(owner, handle, HandleKind::CredentialResolver)?;
        return Err(unsupported("credentialResolver settings"));
    }
    if package == JEV {
        if fetch.is_some()
            || settings.base_url.is_some()
            || settings.organization.is_some()
            || settings.project.is_some()
            || !settings.params.is_empty()
        {
            return Err(OpsError::AiMux(AiMuxError::invalid_argument(
                "Jev accepts apiKey, endpoint, headers and probabilitySource settings",
            )));
        }
        let mut config = match settings.api_key {
            Some(key) => aimux_providers::JevConfig::new(key),
            None => aimux_providers::JevConfig::from_env().map_err(OpsError::AiMux)?,
        };
        if let Some(endpoint) = settings.endpoint.filter(|endpoint| !endpoint.is_empty()) {
            config = config.with_endpoint(endpoint);
        }
        if let Some(source) = settings.probability_source {
            config.probability_source = source.parse().map_err(OpsError::AiMux)?;
        }
        config.headers = settings
            .headers
            .into_iter()
            .filter_map(|(key, value)| value.map(|value| (key, value)))
            .collect();
        return insert_provider(
            owner,
            Arc::new(aimux_providers::JevProvider::new(config)),
            Some(ModelMethod::Decision),
        );
    }
    if settings.endpoint.is_some() || settings.probability_source.is_some() {
        return Err(OpsError::AiMux(AiMuxError::invalid_argument(
            "endpoint and probabilitySource settings require the Jev provider",
        )));
    }
    let mut headers = settings.headers;
    if let Some(org) = settings.organization {
        headers
            .entry("OpenAI-Organization".into())
            .or_insert(Some(org));
    }
    if let Some(project) = settings.project {
        headers
            .entry("OpenAI-Project".into())
            .or_insert(Some(project));
    }
    let provider = aimux_providers::create_provider(
        package,
        PresetSettings {
            api_key: settings.api_key.map(Resolvable::Value),
            base_url: settings.base_url,
            headers: (!headers.is_empty()).then_some(headers),
            fetch,
            params: settings.params,
            ..PresetSettings::default()
        },
    )
    .map_err(OpsError::AiMux)?;
    // Preset descriptors construct language models. Vendor factories expose
    // no default-method metadata in this baseline; require an explicit method.
    let method = aimux_providers::preset::lookup(package).map(|_| ModelMethod::Language);
    insert_provider(owner, provider, method)
}

fn model(ctx: Ctx, entry: Target, params: Value) -> CallFuture {
    Box::pin(async move {
        let params: ProviderModelParams = decode(params)?;
        let provider = provider(entry)?;
        let method = match params.method {
            Some(method) => ModelMethod::parse(&method)?,
            None => {
                default_method(&provider)?.ok_or_else(|| unsupported("default model method"))?
            }
        };
        let id = &params.model_id;
        let model = match method {
            ModelMethod::Language => {
                Entry::Language(provider.language_model(id).map_err(OpsError::AiMux)?)
            }
            ModelMethod::Embedding => {
                Entry::Embedding(provider.embedding_model(id).map_err(OpsError::AiMux)?)
            }
            ModelMethod::Image => Entry::Image(provider.image_model(id).map_err(OpsError::AiMux)?),
            ModelMethod::Speech => Entry::Speech(
                provider
                    .speech_model(id)
                    .ok_or_else(|| unsupported("speechModel"))?
                    .map_err(OpsError::AiMux)?,
            ),
            ModelMethod::Transcription => Entry::Transcription(
                provider
                    .transcription_model(id)
                    .ok_or_else(|| unsupported("transcriptionModel"))?
                    .map_err(OpsError::AiMux)?,
            ),
            ModelMethod::Reranking => Entry::Reranking(
                provider
                    .reranking_model(id)
                    .ok_or_else(|| unsupported("rerankingModel"))?
                    .map_err(OpsError::AiMux)?,
            ),
            ModelMethod::Video => Entry::Video(
                provider
                    .video_model(id)
                    .ok_or_else(|| unsupported("videoModel"))?
                    .map_err(OpsError::AiMux)?,
            ),
            ModelMethod::Decision => Entry::Decision(
                provider
                    .decision_model(id)
                    .ok_or_else(|| unsupported("decision"))?
                    .map_err(OpsError::AiMux)?,
            ),
            ModelMethod::Search => Entry::Search(
                provider
                    .search_model(id)
                    .ok_or_else(|| unsupported("searchModel"))?
                    .map_err(OpsError::AiMux)?,
            ),
        };
        insert(ctx.owner, model)
    })
}

fn invoke(ctx: Ctx, entry: Target, params: Value) -> CallFuture {
    Box::pin(async move {
        let params: ProviderInvokeParams = decode(params)?;
        if params.method != "listModels" {
            return Err(unsupported(&params.method));
        }
        let provider = provider(entry)?;
        let discovery = provider
            .discovery()
            .ok_or_else(|| unsupported("listModels"))?;
        let models: ProviderInvokeResult = match ctx.abort {
            Some(signal) => tokio::select! {
                biased;
                _ = signal.cancelled() => return Err(OpsError::AiMux(AiMuxError::from_abort_signal(&signal))),
                result = discovery.list_models() => result,
            },
            None => discovery.list_models().await,
        }.map_err(OpsError::AiMux)?;
        serde_json::to_value(models).map_err(|error| {
            OpsError::Binding(BindingError::ResultSerialization(error.to_string()))
        })
    })
}

type LanguageMiddleware = Vec<Arc<dyn LanguageModelMiddleware>>;
type ImageMiddleware = Vec<Arc<dyn ImageModelMiddleware>>;

fn middleware(
    owner: Owner,
    params: ProviderWrapParams,
) -> Result<(LanguageMiddleware, ImageMiddleware), OpsError> {
    let mut language = Vec::with_capacity(params.language_model_middleware.len());
    for handle in params.language_model_middleware {
        match get(owner, handle, HandleKind::Middleware)? {
            Entry::Middleware(middleware) => match middleware.as_ref() {
                Middleware::Language(middleware) => language.push(middleware.clone()),
                Middleware::Image(_) => {
                    return Err(unsupported("languageModelMiddleware capability"));
                }
            },
            entry => return Err(invalid_handle(&[HandleKind::Middleware], entry.kind())),
        }
    }
    let mut image = Vec::with_capacity(params.image_model_middleware.len());
    for handle in params.image_model_middleware {
        match get(owner, handle, HandleKind::Middleware)? {
            Entry::Middleware(middleware) => match middleware.as_ref() {
                Middleware::Image(middleware) => image.push(middleware.clone()),
                Middleware::Language(_) => {
                    return Err(unsupported("imageModelMiddleware capability"));
                }
            },
            entry => return Err(invalid_handle(&[HandleKind::Middleware], entry.kind())),
        }
    }
    Ok((language, image))
}

fn wrap(ctx: Ctx, entry: Target, params: Value) -> CallFuture {
    Box::pin(async move {
        let params: ProviderWrapParams = decode(params)?;
        let provider = provider(entry)?;
        let method = default_method(&provider)?;
        let (language, image) = middleware(ctx.owner, params)?;
        insert_provider(
            ctx.owner,
            wrap_provider(provider, &language, &image),
            method,
        )
    })
}

fn custom(ctx: Ctx, _entry: Target, params: Value) -> CallFuture {
    Box::pin(async move {
        let params: ProviderCustomParams = decode(params)?;
        let mut options = CustomProviderOptions::default();
        macro_rules! models {
            ($field:ident, $kind:ident) => {
                for (id, handle) in params.$field {
                    match get(ctx.owner, handle, HandleKind::$kind)? {
                        Entry::$kind(model) => {
                            options.$field.insert(id, model);
                        }
                        entry => return Err(invalid_handle(&[HandleKind::$kind], entry.kind())),
                    }
                }
            };
        }
        models!(language_models, Language);
        models!(embedding_models, Embedding);
        models!(image_models, Image);
        models!(speech_models, Speech);
        models!(transcription_models, Transcription);
        models!(reranking_models, Reranking);
        models!(video_models, Video);
        models!(search_models, Search);
        models!(decision_models, Decision);
        if let Some(handle) = params.files {
            options.files = match get(ctx.owner, handle, HandleKind::Files)? {
                Entry::Files(files) => Some(files),
                entry => return Err(invalid_handle(&[HandleKind::Files], entry.kind())),
            };
        }
        if let Some(handle) = params.fallback_provider {
            options.fallback_provider = Some(provider(Target::Handle(get(
                ctx.owner,
                handle,
                HandleKind::Provider,
            )?))?);
        }
        let method = match params.default_method {
            Some(method) => Some(ModelMethod::parse(&method)?),
            None => match &options.fallback_provider {
                Some(provider) => default_method(provider)?,
                None => None,
            },
        };
        let (language, image) = middleware(
            ctx.owner,
            ProviderWrapParams {
                language_model_middleware: params.language_model_middleware,
                image_model_middleware: params.image_model_middleware,
            },
        )?;
        let provider: Arc<dyn Provider> = Arc::new(custom_provider(options));
        let provider = if language.is_empty() && image.is_empty() {
            provider
        } else {
            wrap_provider(provider, &language, &image)
        };
        insert_provider(ctx.owner, provider, method)
    })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, SystemTime};

    use futures::future::BoxFuture;

    use super::*;
    use aimux_core::embedding_model::EmbeddingModel;
    use aimux_core::files_model::Files;
    use aimux_core::image_model::ImageModel;
    use aimux_core::language_model::LanguageModel;
    use aimux_core::model_catalogue::RuntimeModel;
    use aimux_core::provider::ProviderDiscovery;
    use aimux_provider_utils::fetch::{Fetch, FetchError, FetchRequest, FetchResponse};
    use serde_json::json;

    fn owner() -> Owner {
        Owner::new_node().unwrap()
    }

    fn ctx(owner: Owner) -> Ctx {
        Ctx {
            owner,
            abort: None,
            now: SystemTime::now,
        }
    }

    async fn call(
        owner: Owner,
        handle: HandleId,
        op: &str,
        params: Value,
    ) -> Result<Value, OpsError> {
        crate::dispatch::call(ctx(owner), handle, op, params)
            .await
            .map_err(|error| match error {
                OpsError::Projected(error) => error.source,
                error => error,
            })
    }

    fn id(value: &Value) -> HandleId {
        HandleId(value["handle"].as_str().unwrap().parse().unwrap())
    }

    fn handle(owner: Owner, entry: Entry) -> HandleId {
        HandleRegistry::global().insert(owner, entry).unwrap()
    }

    fn entry(owner: Owner, result: &Value, kind: HandleKind) -> Entry {
        HandleRegistry::global()
            .get(owner, id(result), &[kind])
            .unwrap()
    }

    fn package(name: &str) -> Arc<dyn Provider> {
        aimux_providers::create_provider(name, PresetSettings::default()).unwrap()
    }

    #[derive(Clone, Copy)]
    enum Discovery {
        Success,
        Error,
        Pending,
    }

    struct MockProvider {
        base: Arc<dyn Provider>,
        discovery: Discovery,
        calls: Arc<AtomicUsize>,
    }

    impl Provider for MockProvider {
        fn language_model(&self, id: &str) -> Result<Arc<dyn LanguageModel>, AiMuxError> {
            self.base.language_model(id)
        }
        fn embedding_model(&self, id: &str) -> Result<Arc<dyn EmbeddingModel>, AiMuxError> {
            self.base.embedding_model(id)
        }
        fn image_model(&self, id: &str) -> Result<Arc<dyn ImageModel>, AiMuxError> {
            self.base.image_model(id)
        }
        fn files(&self) -> Option<Arc<dyn Files>> {
            self.base.files()
        }
        fn discovery(&self) -> Option<&dyn ProviderDiscovery> {
            Some(self)
        }
    }

    impl ProviderDiscovery for MockProvider {
        fn list_models(&self) -> BoxFuture<'_, Result<Vec<RuntimeModel>, AiMuxError>> {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::Relaxed);
                match self.discovery {
                    Discovery::Success => Ok(vec![RuntimeModel {
                        id: "mock-model".into(),
                        owned_by: None,
                        created: None,
                    }]),
                    Discovery::Error => Err(AiMuxError::invalid_argument("discovery failed")),
                    Discovery::Pending => futures::future::pending().await,
                }
            })
        }
    }

    fn mock(owner: Owner, discovery: Discovery) -> (HandleId, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let provider = MockProvider {
            base: package("openai"),
            discovery,
            calls: calls.clone(),
        };
        (handle(owner, Entry::Provider(Arc::new(provider))), calls)
    }

    struct LanguageOverride;
    impl LanguageModelMiddleware for LanguageOverride {
        fn override_provider(&self, _: &dyn LanguageModel) -> Option<String> {
            Some("wrapped.language".into())
        }
    }

    struct ImageOverride;
    impl ImageModelMiddleware for ImageOverride {
        fn override_provider(&self, _: &dyn ImageModel) -> Option<String> {
            Some("wrapped.image".into())
        }
    }

    struct MockFetch(AtomicUsize);
    impl Fetch for MockFetch {
        fn fetch<'a, 'f>(
            &'a self,
            _: FetchRequest,
        ) -> BoxFuture<'f, Result<FetchResponse, FetchError>>
        where
            'a: 'f,
            Self: 'f,
        {
            Box::pin(async move {
                self.0.fetch_add(1, Ordering::Relaxed);
                Err(FetchError::Other("mock transport".into()))
            })
        }
    }

    #[tokio::test]
    async fn create_uses_the_existing_package_factory() {
        let owner = owner();
        let result = call(owner, HandleId(0), "provider.create", json!({
            "package": "openai", "settings": {"apiKey": "test-key", "baseUrl": "http://localhost:1/v1", "headers": {"x-test": "value"}}
        })).await.unwrap();
        assert_eq!(result["kind"], "Provider");
        let Entry::Provider(provider) = entry(owner, &result, HandleKind::Provider) else {
            panic!()
        };
        assert_eq!(provider.specification_version(), "v4");
        assert_eq!(
            provider.language_model("model").unwrap().model_id(),
            "model"
        );
        assert_eq!(result.as_object().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn create_rejects_unknown_packages_and_unknown_control_fields() {
        let owner = owner();
        assert!(matches!(
            call(
                owner,
                HandleId(0),
                "provider.create",
                json!({"package":"missing-package"})
            )
            .await,
            Err(OpsError::AiMux(AiMuxError::NoSuchProvider { .. }))
        ));
        for params in [
            json!({"package":"openai", "callback":{}}),
            json!({"package":"openai", "settings":{"fetch":{"callback":"host"}}}),
            json!({"package":"openai", "settings":{"unknown":1}}),
            json!({"package":"openai", "settings":null}),
        ] {
            assert!(matches!(
                call(owner, HandleId(0), "provider.create", params).await,
                Err(OpsError::Protocol(ProtocolError::InvalidParams(_)))
            ));
        }
    }

    #[tokio::test]
    async fn default_uses_preset_descriptor_and_requires_global_scope() {
        let owner = owner();
        let result = call(
            owner,
            HandleId(0),
            "provider.default",
            json!({"package":"ollama"}),
        )
        .await
        .unwrap();
        let model = call(
            owner,
            id(&result),
            "provider.model",
            json!({"modelId":"local"}),
        )
        .await
        .unwrap();
        assert_eq!(model["kind"], "Language");
        assert!(matches!(
            call(
                owner,
                id(&result),
                "provider.default",
                json!({"package":"openai"})
            )
            .await,
            Err(OpsError::Binding(BindingError::InvalidHandle { .. }))
        ));
        assert!(matches!(
            call(
                owner,
                HandleId(0),
                "provider.default",
                json!({"package":"openai", "settings":{}})
            )
            .await,
            Err(OpsError::Protocol(ProtocolError::InvalidParams(_)))
        ));
    }

    #[tokio::test]
    async fn model_returns_all_eight_exact_handle_kinds() {
        let owner = owner();
        for (package, method, kind) in [
            ("openai", "languageModel", HandleKind::Language),
            ("openai", "embeddingModel", HandleKind::Embedding),
            ("openai", "imageModel", HandleKind::Image),
            ("openai", "speechModel", HandleKind::Speech),
            ("openai", "transcriptionModel", HandleKind::Transcription),
            ("cohere", "rerankingModel", HandleKind::Reranking),
            ("google", "videoModel", HandleKind::Video),
            ("tavily", "searchModel", HandleKind::Search),
        ] {
            let fallback = handle(owner, Entry::Provider(self::package(package)));
            let provider = call(
                owner,
                HandleId(0),
                "provider.custom",
                json!({"fallbackProvider":fallback.0.to_string()}),
            )
            .await
            .unwrap();
            let result = call(
                owner,
                id(&provider),
                "provider.model",
                json!({"method":method, "modelId":"test-model"}),
            )
            .await
            .unwrap();
            assert_eq!(entry(owner, &result, kind).kind(), kind);
            assert!(
                HandleRegistry::global()
                    .remove(owner, id(&result))
                    .is_some()
            );
            assert!(
                HandleRegistry::global()
                    .remove(owner, id(&provider))
                    .is_some()
            );
            assert!(HandleRegistry::global().remove(owner, fallback).is_some());
        }
    }

    #[tokio::test]
    async fn model_rejects_unavailable_methods_and_missing_default_metadata() {
        let owner = owner();
        let (provider, _) = mock(owner, Discovery::Success);
        for params in [
            json!({"modelId":"x"}),
            json!({"method":"speechModel", "modelId":"x"}),
            json!({"method":"skills", "modelId":"x"}),
            json!({"method":"LanguageModel", "modelId":"x"}),
        ] {
            assert!(matches!(
                call(owner, provider, "provider.model", params).await,
                Err(OpsError::AiMux(AiMuxError::UnsupportedFunctionality(_)))
            ));
        }
        assert!(matches!(
            call(
                owner,
                provider,
                "provider.model",
                json!({"method":"languageModel"})
            )
            .await,
            Err(OpsError::Protocol(ProtocolError::InvalidParams(_)))
        ));
    }

    #[tokio::test]
    async fn invoke_returns_only_runtime_discovery_and_forbids_arbitrary_rpc() {
        let owner = owner();
        let (provider, calls) = mock(owner, Discovery::Success);
        let result = call(
            owner,
            provider,
            "provider.invoke",
            json!({"method":"listModels"}),
        )
        .await
        .unwrap();
        assert_eq!(result, json!([{"id":"mock-model"}]));
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert!(matches!(
            call(
                owner,
                provider,
                "provider.invoke",
                json!({"method":"deleteEverything"})
            )
            .await,
            Err(OpsError::AiMux(AiMuxError::UnsupportedFunctionality(_)))
        ));
        assert!(matches!(
            call(
                owner,
                provider,
                "provider.invoke",
                json!({"method":"listModels", "args":{}})
            )
            .await,
            Err(OpsError::Protocol(ProtocolError::InvalidParams(_)))
        ));
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn invoke_preserves_discovery_errors_and_reports_unsupported_discovery() {
        let owner = owner();
        let (provider, _) = mock(owner, Discovery::Error);
        assert!(
            matches!(call(owner, provider, "provider.invoke", json!({"method":"listModels"})).await,
            Err(OpsError::AiMux(AiMuxError::InvalidArgument { message, .. })) if message == "discovery failed")
        );
        let provider = handle(
            owner,
            Entry::Provider(Arc::new(custom_provider(CustomProviderOptions::default()))),
        );
        assert!(matches!(
            call(
                owner,
                provider,
                "provider.invoke",
                json!({"method":"listModels"})
            )
            .await,
            Err(OpsError::AiMux(AiMuxError::UnsupportedFunctionality(_)))
        ));
    }

    #[tokio::test]
    async fn invoke_cancels_pending_discovery() {
        let owner = owner();
        let (provider, _) = mock(owner, Discovery::Pending);
        let abort = aimux_core::AbortSignal::new();
        let mut context = ctx(owner);
        context.abort = Some(abort.clone());
        let future = crate::dispatch::call(
            context,
            provider,
            "provider.invoke",
            json!({"method":"listModels"}),
        );
        let cancel = async move {
            tokio::task::yield_now().await;
            abort.abort();
        };
        let result = tokio::time::timeout(Duration::from_secs(1), async {
            tokio::join!(future, cancel).0
        })
        .await
        .unwrap();
        assert!(matches!(
            result.as_ref().map_err(OpsError::unprojected),
            Err(OpsError::AiMux(AiMuxError::Aborted(_)))
        ));
    }

    #[tokio::test]
    async fn wrap_applies_each_middleware_and_keeps_files_and_discovery() {
        let owner = owner();
        let (provider, _) = mock(owner, Discovery::Success);
        let language = handle(
            owner,
            Entry::Middleware(Arc::new(Middleware::Language(Arc::new(LanguageOverride)))),
        );
        let image = handle(
            owner,
            Entry::Middleware(Arc::new(Middleware::Image(Arc::new(ImageOverride)))),
        );
        let result = call(owner, provider, "provider.wrap", json!({"languageModelMiddleware":[language.0.to_string()], "imageModelMiddleware":[image.0.to_string()]})).await.unwrap();
        assert!(HandleRegistry::global().remove(owner, provider).is_some());
        assert!(HandleRegistry::global().remove(owner, language).is_some());
        assert!(HandleRegistry::global().remove(owner, image).is_some());
        let Entry::Provider(wrapped) = entry(owner, &result, HandleKind::Provider) else {
            panic!()
        };
        assert_eq!(
            wrapped.language_model("x").unwrap().provider(),
            "wrapped.language"
        );
        assert_eq!(
            wrapped.image_model("x").unwrap().provider(),
            "wrapped.image"
        );
        assert!(wrapped.files().is_some());
        assert_eq!(
            wrapped.discovery().unwrap().list_models().await.unwrap()[0].id,
            "mock-model"
        );
        assert!(wrapped.embedding_model("x").is_ok());
    }

    #[tokio::test]
    async fn wrap_checks_each_middleware_capability_kind_and_owner() {
        let owner = owner();
        let (provider, _) = mock(owner, Discovery::Success);
        let image = handle(
            owner,
            Entry::Middleware(Arc::new(Middleware::Image(Arc::new(ImageOverride)))),
        );
        assert!(matches!(
            call(
                owner,
                provider,
                "provider.wrap",
                json!({"languageModelMiddleware":[image.0.to_string()]})
            )
            .await,
            Err(OpsError::AiMux(AiMuxError::UnsupportedFunctionality(_)))
        ));
        assert!(matches!(
            call(
                owner,
                provider,
                "provider.wrap",
                json!({"imageModelMiddleware":[provider.0.to_string()]})
            )
            .await,
            Err(OpsError::Binding(BindingError::InvalidHandle {
                actual: Some(HandleKind::Provider),
                ..
            }))
        ));
        let foreign = handle(
            self::owner(),
            Entry::Middleware(Arc::new(Middleware::Image(Arc::new(ImageOverride)))),
        );
        assert!(matches!(
            call(
                owner,
                provider,
                "provider.wrap",
                json!({"imageModelMiddleware":[foreign.0.to_string()]})
            )
            .await,
            Err(OpsError::Binding(BindingError::InvalidHandle {
                actual: None,
                ..
            }))
        ));
    }

    #[tokio::test]
    async fn custom_keeps_aliases_default_method_and_strong_references() {
        let owner = owner();
        let language = package("openai").language_model("original").unwrap();
        let child = handle(owner, Entry::Language(language.clone()));
        let result = call(owner, HandleId(0), "provider.custom", json!({"languageModels":{"alias":child.0.to_string()}, "defaultMethod":"languageModel"})).await.unwrap();
        assert!(HandleRegistry::global().remove(owner, child).is_some());
        let resolved = call(
            owner,
            id(&result),
            "provider.model",
            json!({"modelId":"alias"}),
        )
        .await
        .unwrap();
        let Entry::Language(model) = entry(owner, &resolved, HandleKind::Language) else {
            panic!()
        };
        assert!(Arc::ptr_eq(&language, &model));
        assert!(matches!(
            call(
                owner,
                id(&result),
                "provider.model",
                json!({"modelId":"missing"})
            )
            .await,
            Err(OpsError::AiMux(AiMuxError::NoSuchModel { .. }))
        ));
        let wrapped = call(owner, id(&result), "provider.wrap", json!({}))
            .await
            .unwrap();
        assert!(
            call(
                owner,
                id(&wrapped),
                "provider.model",
                json!({"modelId":"alias"})
            )
            .await
            .is_ok()
        );
    }

    #[tokio::test]
    async fn custom_resolves_all_eight_model_maps() {
        let owner = owner();
        let openai = package("openai");
        let mappings = [
            (
                "languageModels",
                "languageModel",
                Entry::Language(openai.language_model("x").unwrap()),
            ),
            (
                "embeddingModels",
                "embeddingModel",
                Entry::Embedding(openai.embedding_model("x").unwrap()),
            ),
            (
                "imageModels",
                "imageModel",
                Entry::Image(openai.image_model("x").unwrap()),
            ),
            (
                "speechModels",
                "speechModel",
                Entry::Speech(openai.speech_model("x").unwrap().unwrap()),
            ),
            (
                "transcriptionModels",
                "transcriptionModel",
                Entry::Transcription(openai.transcription_model("x").unwrap().unwrap()),
            ),
            (
                "rerankingModels",
                "rerankingModel",
                Entry::Reranking(package("cohere").reranking_model("x").unwrap().unwrap()),
            ),
            (
                "videoModels",
                "videoModel",
                Entry::Video(package("google").video_model("x").unwrap().unwrap()),
            ),
            (
                "searchModels",
                "searchModel",
                Entry::Search(package("tavily").search_model("x").unwrap().unwrap()),
            ),
        ];
        for (field, method, model) in mappings {
            let kind = model.kind();
            let model = handle(owner, model);
            let provider = call(
                owner,
                HandleId(0),
                "provider.custom",
                json!({(field):{"alias":model.0.to_string()}}),
            )
            .await
            .unwrap();
            let resolved = call(
                owner,
                id(&provider),
                "provider.model",
                json!({"method":method,"modelId":"alias"}),
            )
            .await
            .unwrap();
            assert_eq!(entry(owner, &resolved, kind).kind(), kind);
        }
    }

    #[tokio::test]
    async fn custom_rejects_wrong_kind_owner_and_host_function_fields() {
        let owner = owner();
        let (provider, _) = mock(owner, Discovery::Success);
        assert!(matches!(
            call(
                owner,
                HandleId(0),
                "provider.custom",
                json!({"languageModels":{"alias":provider.0.to_string()}})
            )
            .await,
            Err(OpsError::Binding(BindingError::InvalidHandle {
                actual: Some(HandleKind::Provider),
                ..
            }))
        ));
        let foreign = handle(
            self::owner(),
            Entry::Language(package("openai").language_model("x").unwrap()),
        );
        assert!(matches!(
            call(
                owner,
                HandleId(0),
                "provider.custom",
                json!({"languageModels":{"alias":foreign.0.to_string()}})
            )
            .await,
            Err(OpsError::Binding(BindingError::InvalidHandle {
                actual: None,
                ..
            }))
        ));
        for params in [
            json!({"languageModels":{"x":1}}),
            json!({"languageModels":{"x":"-1"}}),
            json!({"languageModels":{"x":"18446744073709551616"}}),
            json!({"callback":{}}),
            json!({"skills":{}}),
        ] {
            assert!(matches!(
                call(owner, HandleId(0), "provider.custom", params).await,
                Err(OpsError::Protocol(ProtocolError::InvalidParams(_)))
            ));
        }
    }

    #[tokio::test]
    async fn custom_preserves_fallback_extensions_and_preset_default() {
        let owner = owner();
        let (fallback, _) = mock(owner, Discovery::Success);
        let result = call(
            owner,
            HandleId(0),
            "provider.custom",
            json!({"fallbackProvider":fallback.0.to_string()}),
        )
        .await
        .unwrap();
        assert!(HandleRegistry::global().remove(owner, fallback).is_some());
        let Entry::Provider(custom) = entry(owner, &result, HandleKind::Provider) else {
            panic!()
        };
        assert!(custom.files().is_some());
        assert_eq!(
            call(
                owner,
                id(&result),
                "provider.invoke",
                json!({"method":"listModels"})
            )
            .await
            .unwrap(),
            json!([{"id":"mock-model"}])
        );
        let fallback = call(
            owner,
            HandleId(0),
            "provider.default",
            json!({"package":"ollama"}),
        )
        .await
        .unwrap();
        let result = call(
            owner,
            HandleId(0),
            "provider.custom",
            json!({"fallbackProvider":fallback["handle"]}),
        )
        .await
        .unwrap();
        assert!(
            call(owner, id(&result), "provider.model", json!({"modelId":"x"}))
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn create_validates_runtime_slot_kind_and_owner() {
        let owner = owner();
        let (provider, _) = mock(owner, Discovery::Success);
        for field in ["fetch", "webSocket", "credentialResolver"] {
            assert!(matches!(
                call(
                    owner,
                    HandleId(0),
                    "provider.create",
                    json!({"package":"openai", "settings":{(field):provider.0.to_string()}})
                )
                .await,
                Err(OpsError::Binding(BindingError::InvalidHandle {
                    actual: Some(HandleKind::Provider),
                    ..
                }))
            ));
        }
        let foreign = handle(
            self::owner(),
            Entry::Fetch(Arc::new(MockFetch(AtomicUsize::new(0)))),
        );
        assert!(matches!(
            call(
                owner,
                HandleId(0),
                "provider.create",
                json!({"package":"openai", "settings":{"fetch":foreign.0.to_string()}})
            )
            .await,
            Err(OpsError::Binding(BindingError::InvalidHandle {
                actual: None,
                ..
            }))
        ));
    }

    #[tokio::test]
    async fn create_injects_fetch_and_models_retain_it_after_drop() {
        let owner = owner();
        let fetch = Arc::new(MockFetch(AtomicUsize::new(0)));
        let transport = handle(owner, Entry::Fetch(fetch.clone()));
        let provider = call(
            owner,
            HandleId(0),
            "provider.create",
            json!({"package":"ollama", "settings":{"fetch":transport.0.to_string()}}),
        )
        .await
        .unwrap();
        let model = call(
            owner,
            id(&provider),
            "provider.model",
            json!({"modelId":"local"}),
        )
        .await
        .unwrap();
        assert!(HandleRegistry::global().remove(owner, transport).is_some());
        assert!(
            HandleRegistry::global()
                .remove(owner, id(&provider))
                .is_some()
        );
        let Entry::Language(model) = entry(owner, &model, HandleKind::Language) else {
            panic!()
        };
        let result = model
            .do_generate(&aimux_core::options::CallOptions::new(vec![]))
            .await;
        assert!(result.is_err());
        assert_eq!(fetch.0.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn every_provider_scoped_op_rejects_foreign_owner_and_wrong_kind() {
        let owner = owner();
        let (provider, _) = mock(owner, Discovery::Success);
        let language = handle(
            owner,
            Entry::Language(package("openai").language_model("x").unwrap()),
        );
        for (op, params) in [
            (
                "provider.model",
                json!({"method":"languageModel", "modelId":"x"}),
            ),
            ("provider.invoke", json!({"method":"listModels"})),
            ("provider.wrap", json!({})),
        ] {
            assert!(matches!(
                call(self::owner(), provider, op, params.clone()).await,
                Err(OpsError::Binding(BindingError::InvalidHandle {
                    actual: None,
                    ..
                }))
            ));
            assert!(matches!(
                call(owner, language, op, params).await,
                Err(OpsError::Binding(BindingError::InvalidHandle {
                    actual: Some(HandleKind::Language),
                    ..
                }))
            ));
        }
    }

    #[test]
    fn descriptors_register_only_the_six_owned_contract_ops() {
        let mut handlers = Vec::new();
        register(&mut handlers);
        assert_eq!(handlers.len(), 6);
        assert_eq!(
            handlers.iter().map(|(d, _)| d.name).collect::<Vec<_>>(),
            vec![
                "provider.create",
                "provider.default",
                "provider.model",
                "provider.invoke",
                "provider.wrap",
                "provider.custom"
            ]
        );
        for (descriptor, _) in handlers {
            assert!(matches!(descriptor.mode, OpMode::Call));
            assert!(descriptor.event_type.is_none());
            assert_eq!(descriptor.since_ops_version, "1.0");
        }
    }
}
