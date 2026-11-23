//! Provider lifecycle and the existing model-discovery extension.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, Weak};

use aimux_core::custom_provider::{CustomProviderOptions, custom_provider};
use aimux_core::error::AiMuxError;
use aimux_core::image_model_middleware::ImageModelMiddleware;
use aimux_core::language_model_middleware::LanguageModelMiddleware;
use aimux_core::provider::{LanguageApi, Provider};
use aimux_core::wrap_provider::wrap_provider;
use aimux_provider_utils::{FetchFunction, ModalityKind, StringOrEnv};
use aimux_providers::{
    CodexModeSpec, EnvLookupPolicy, ProviderConfigSettings, ProviderEntrySpec,
    provider_from_config_with_transport,
};
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
    ("chatModels.*", &[HandleKind::Language]),
    ("responsesModels.*", &[HandleKind::Language]),
    ("completionModels.*", &[HandleKind::Language]),
    ("messagesModels.*", &[HandleKind::Language]),
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
    descriptor(
        "provider.descriptors",
        &[],
        TypeRef::of::<ProviderDescriptorsParams>(),
        TypeRef::of::<ProviderDescriptorsResult>(),
        false,
        false,
        &[
            "aimux_providers::package_descriptors",
            "aimux_providers::package_descriptor",
        ],
        &[],
    ),
];

pub(crate) fn register(r: &mut Vec<(&'static OpDescriptor, Handler)>) {
    let handlers: [fn(Ctx, Target, Value) -> CallFuture; 7] = [
        create,
        default_provider,
        model,
        invoke,
        wrap,
        custom,
        provider_descriptors,
    ];
    r.extend(
        OPS.iter()
            .zip(handlers)
            .map(|(op, handler)| (op, Handler::Call(handler))),
    );
}

/// Trusted-caller operation: constructs a provider handle from package settings
/// and optional `{ "env": ... }` references. Untrusted callers must not be
/// permitted to supply arbitrary `baseUrl` values with omitted credentials.
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProviderCreateParams {
    package: String,
    #[serde(default)]
    settings: ProviderSettings,
    /// Optional allowlist gating explicit `{ "env": "VAR" }` (`StringOrEnv`)
    /// interpolation in `settings`. Does not restrict implicit per-provider
    /// default environment variable fallbacks when credential or `baseUrl`
    /// settings are omitted.
    #[serde(default)]
    env_allowlist: Option<Vec<String>>,
}

#[derive(Default, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProviderDescriptorsParams {
    #[serde(default)]
    package: Option<String>,
}

type ProviderDescriptorsResult = Vec<aimux_provider_utils::PackageDescriptor>;

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProviderDefaultParams {
    package: String,
}

#[derive(Default, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProviderSettings {
    api_key: Option<StringOrEnv>,
    auth_token: Option<StringOrEnv>,
    access_token: Option<StringOrEnv>,
    access_key_id: Option<StringOrEnv>,
    secret_access_key: Option<StringOrEnv>,
    session_token: Option<StringOrEnv>,
    login: Option<StringOrEnv>,
    password: Option<StringOrEnv>,
    codex_mode: Option<CodexModeSpec>,
    base_url: Option<String>,
    name: Option<String>,
    #[serde(default)]
    headers: HashMap<String, Option<String>>,
    organization: Option<String>,
    project: Option<String>,
    location: Option<String>,
    region: Option<String>,
    resource_name: Option<String>,
    speech_base_url: Option<String>,
    api_version: Option<String>,
    workspace_id: Option<String>,
    originator: Option<String>,
    cx: Option<String>,
    use_deployment_based_urls: Option<bool>,
    include_usage: Option<bool>,
    supports_structured_outputs: Option<bool>,
    supports_multi_part_tool_content: Option<bool>,
    #[serde(default)]
    query_params: HashMap<String, String>,
    #[serde(default)]
    params: HashMap<String, String>,
    fetch: Option<HandleId>,
    web_socket: Option<HandleId>,
    credential_resolver: Option<HandleId>,
    endpoint: Option<String>,
    probability_source: Option<String>,
}

struct SplitProviderSettings {
    config: ProviderConfigSettings,
    fetch: Option<HandleId>,
    web_socket: Option<HandleId>,
    credential_resolver: Option<HandleId>,
}

impl ProviderSettings {
    fn split(self) -> SplitProviderSettings {
        SplitProviderSettings {
            config: ProviderConfigSettings {
                api_key: self.api_key,
                auth_token: self.auth_token,
                access_token: self.access_token,
                access_key_id: self.access_key_id,
                secret_access_key: self.secret_access_key,
                session_token: self.session_token,
                login: self.login,
                password: self.password,
                codex_mode: self.codex_mode,
                base_url: self.base_url,
                name: self.name,
                organization: self.organization,
                project: self.project,
                location: self.location,
                region: self.region,
                resource_name: self.resource_name,
                speech_base_url: self.speech_base_url,
                api_version: self.api_version,
                workspace_id: self.workspace_id,
                originator: self.originator,
                cx: self.cx,
                endpoint: self.endpoint,
                probability_source: self.probability_source,
                use_deployment_based_urls: self.use_deployment_based_urls,
                include_usage: self.include_usage,
                supports_structured_outputs: self.supports_structured_outputs,
                supports_multi_part_tool_content: self.supports_multi_part_tool_content,
                headers: self.headers,
                query_params: self.query_params,
                params: self.params,
            },
            fetch: self.fetch,
            web_socket: self.web_socket,
            credential_resolver: self.credential_resolver,
        }
    }
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
    chat_models: HashMap<String, HandleId>,
    #[serde(default)]
    responses_models: HashMap<String, HandleId>,
    #[serde(default)]
    completion_models: HashMap<String, HandleId>,
    #[serde(default)]
    messages_models: HashMap<String, HandleId>,
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
    Chat,
    Responses,
    Completion,
    Messages,
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
    fn from_modality_kind(kind: ModalityKind) -> Self {
        match kind {
            ModalityKind::LanguageModel => Self::Language,
            ModalityKind::EmbeddingModel => Self::Embedding,
            ModalityKind::ImageModel => Self::Image,
            ModalityKind::SpeechModel => Self::Speech,
            ModalityKind::TranscriptionModel => Self::Transcription,
            ModalityKind::RerankingModel => Self::Reranking,
            ModalityKind::VideoModel => Self::Video,
            ModalityKind::SearchModel => Self::Search,
            ModalityKind::Decision => Self::Decision,
        }
    }

    fn parse(method: &str) -> Result<Self, OpsError> {
        match method {
            "languageModel" => Ok(Self::Language),
            "chat" => Ok(Self::Chat),
            "responses" => Ok(Self::Responses),
            "completion" => Ok(Self::Completion),
            "messages" => Ok(Self::Messages),
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
const OPENAI_COMPATIBLE: &str = "openai_compatible";

/// Every package name provider.create accepts, sorted.
pub(crate) fn packages() -> Vec<&'static str> {
    let mut names = aimux_providers::provider_names();
    names.push(JEV);
    names.push(OPENAI_COMPATIBLE);
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

fn push_provider_descriptor(
    provider: &Arc<dyn Provider>,
    default_method: Option<ModelMethod>,
) -> Result<(), OpsError> {
    let mut descriptors = descriptors().lock().map_err(|_| invalid_state())?;
    descriptors.retain(|d| d.provider.strong_count() != 0);
    descriptors.push(ProviderDescriptor {
        provider: Arc::downgrade(provider),
        default_method,
    });
    Ok(())
}

pub(crate) fn register_package_default_method(
    provider: &Arc<dyn Provider>,
    package: &str,
) -> Result<(), OpsError> {
    let method = aimux_providers::package_descriptor(package)
        .and_then(|descriptor| descriptor.default_method)
        .map(ModelMethod::from_modality_kind);
    push_provider_descriptor(provider, method)
}

fn insert_provider(
    owner: Owner,
    provider: Arc<dyn Provider>,
    default_method: Option<ModelMethod>,
) -> Result<Value, OpsError> {
    push_provider_descriptor(&provider, default_method)?;
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
        .map_err(|error| OpsError::Binding(BindingError::result_serialization(&error)))
}

fn get(owner: Owner, handle: HandleId, kind: HandleKind) -> Result<Entry, OpsError> {
    HandleRegistry::global().get(owner, handle, &[kind])
}

fn resolve_fetch_and_unsupported_slots(
    owner: Owner,
    fetch: Option<HandleId>,
    web_socket: Option<HandleId>,
    credential_resolver: Option<HandleId>,
) -> Result<Option<FetchFunction>, OpsError> {
    // Resolve every runtime slot before constructing anything. A valid but
    // unavailable slot is a capability error, never silently ignored.
    let fetch = match fetch {
        Some(handle) => match get(owner, handle, HandleKind::Fetch)? {
            Entry::Fetch(fetch) => Some(fetch),
            entry => return Err(invalid_handle(&[HandleKind::Fetch], entry.kind())),
        },
        None => None,
    };
    if let Some(handle) = web_socket {
        get(owner, handle, HandleKind::WebSocket)?;
        return Err(unsupported("webSocket settings"));
    }
    if let Some(handle) = credential_resolver {
        get(owner, handle, HandleKind::CredentialResolver)?;
        return Err(unsupported("credentialResolver settings"));
    }
    Ok(fetch)
}

fn create(ctx: Ctx, _entry: Target, params: Value) -> CallFuture {
    Box::pin(async move {
        let params: ProviderCreateParams = decode(params)?;
        create_package(
            ctx.owner,
            &params.package,
            params.settings,
            params.env_allowlist,
        )
    })
}

fn default_provider(ctx: Ctx, _entry: Target, params: Value) -> CallFuture {
    Box::pin(async move {
        let params: ProviderDefaultParams = decode(params)?;
        if params.package == OPENAI_COMPATIBLE {
            return Err(OpsError::AiMux(AiMuxError::invalid_argument(
                "provider 'openai_compatible' has no default instance; use provider.create with `name` and `baseUrl`",
            )));
        }
        create_package(
            ctx.owner,
            &params.package,
            ProviderSettings::default(),
            None,
        )
    })
}

fn provider_descriptors(_ctx: Ctx, _entry: Target, params: Value) -> CallFuture {
    Box::pin(async move {
        let params: ProviderDescriptorsParams = decode(params)?;
        let list: ProviderDescriptorsResult = match params.package {
            Some(package) => {
                let descriptor =
                    aimux_providers::package_descriptor(&package).ok_or_else(|| {
                        OpsError::AiMux(AiMuxError::NoSuchProvider {
                            provider_id: package.clone(),
                            model_id: package,
                            model_type: String::new(),
                            available_providers: packages()
                                .into_iter()
                                .map(str::to_owned)
                                .collect(),
                        })
                    })?;
                vec![descriptor]
            }
            None => aimux_providers::package_descriptors(),
        };
        serde_json::to_value(list)
            .map_err(|error| OpsError::Binding(BindingError::result_serialization(&error)))
    })
}

fn create_package(
    owner: Owner,
    package: &str,
    settings: ProviderSettings,
    env_allowlist: Option<Vec<String>>,
) -> Result<Value, OpsError> {
    let split = settings.split();
    let fetch = resolve_fetch_and_unsupported_slots(
        owner,
        split.fetch,
        split.web_socket,
        split.credential_resolver,
    )?;
    let policy = match env_allowlist {
        Some(vars) => EnvLookupPolicy::AllowList(vars.into_iter().collect()),
        None => EnvLookupPolicy::AllowAll,
    };
    let provider = provider_from_config_with_transport(
        &ProviderEntrySpec {
            package: package.to_owned(),
            settings: split.config,
        },
        &policy,
        fetch,
    )
    .map_err(OpsError::AiMux)?;
    let method = aimux_providers::package_descriptor(package)
        .and_then(|descriptor| descriptor.default_method)
        .map(ModelMethod::from_modality_kind);
    insert_provider(owner, provider, method)
}

fn offered_language_entry(
    model: Option<Result<Arc<dyn aimux_core::language_model::LanguageModel>, AiMuxError>>,
    method: &str,
) -> Result<Entry, OpsError> {
    Ok(Entry::Language(
        model
            .ok_or_else(|| unsupported(method))?
            .map_err(OpsError::AiMux)?,
    ))
}

fn build_standard_model(
    provider: &Arc<dyn Provider>,
    method: ModelMethod,
    id: &str,
) -> Result<Entry, OpsError> {
    Ok(match method {
        ModelMethod::Language => {
            Entry::Language(provider.language_model(id).map_err(OpsError::AiMux)?)
        }
        ModelMethod::Chat => offered_language_entry(
            provider.language_model_with_api(LanguageApi::Chat, id),
            "chat",
        )?,
        ModelMethod::Responses => offered_language_entry(
            provider.language_model_with_api(LanguageApi::Responses, id),
            "responses",
        )?,
        ModelMethod::Completion => offered_language_entry(
            provider.language_model_with_api(LanguageApi::Completion, id),
            "completion",
        )?,
        ModelMethod::Messages => offered_language_entry(
            provider.language_model_with_api(LanguageApi::Messages, id),
            "messages",
        )?,
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
    })
}

fn model(ctx: Ctx, entry: Target, params: Value) -> CallFuture {
    Box::pin(async move {
        let params: ProviderModelParams = decode(params)?;
        let provider = provider(entry)?;
        let id = &params.model_id;
        if let Some(ref method_name) = params.method {
            let method = ModelMethod::parse(method_name)?;
            return insert(ctx.owner, build_standard_model(&provider, method, id)?);
        }
        let method =
            default_method(&provider)?.ok_or_else(|| unsupported("default model method"))?;
        insert(ctx.owner, build_standard_model(&provider, method, id)?)
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
        serde_json::to_value(models)
            .map_err(|error| OpsError::Binding(BindingError::result_serialization(&error)))
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
        let resolve_language_api_map = |raw: HashMap<String, HandleId>| -> Result<
            HashMap<String, Arc<dyn aimux_core::language_model::LanguageModel>>,
            OpsError,
        > {
            let mut out = HashMap::with_capacity(raw.len());
            for (id, handle) in raw {
                match get(ctx.owner, handle, HandleKind::Language)? {
                    Entry::Language(model) => {
                        out.insert(id, model);
                    }
                    entry => return Err(invalid_handle(&[HandleKind::Language], entry.kind())),
                }
            }
            Ok(out)
        };
        let chat_models = resolve_language_api_map(params.chat_models)?;
        let responses_models = resolve_language_api_map(params.responses_models)?;
        let completion_models = resolve_language_api_map(params.completion_models)?;
        let messages_models = resolve_language_api_map(params.messages_models)?;
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
        let provider: Arc<dyn Provider> = Arc::new(
            custom_provider(options)
                .with_language_api_models(LanguageApi::Chat, chat_models)
                .with_language_api_models(LanguageApi::Responses, responses_models)
                .with_language_api_models(LanguageApi::Completion, completion_models)
                .with_language_api_models(LanguageApi::Messages, messages_models),
        );
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
    use std::time::Duration;

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
        Ctx { owner, abort: None }
    }

    async fn call(
        owner: Owner,
        handle: HandleId,
        op: &str,
        params: Value,
    ) -> Result<Value, OpsError> {
        crate::dispatch::call(ctx(owner), handle, op, params).await
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
        aimux_providers::create_provider(name, aimux_providers::preset::PresetSettings::default())
            .unwrap()
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
                        limits: None,
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
                Err(OpsError::Protocol(ProtocolError::InvalidParams { .. }))
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
            Err(OpsError::Protocol(ProtocolError::InvalidParams { .. }))
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
            Err(OpsError::Protocol(ProtocolError::InvalidParams { .. }))
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
            Err(OpsError::Protocol(ProtocolError::InvalidParams { .. }))
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
            result.as_ref(),
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
                Err(OpsError::Protocol(ProtocolError::InvalidParams { .. }))
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
    fn descriptors_register_only_the_seven_owned_contract_ops() {
        let mut handlers = Vec::new();
        register(&mut handlers);
        assert_eq!(handlers.len(), 7);
        assert_eq!(
            handlers.iter().map(|(d, _)| d.name).collect::<Vec<_>>(),
            vec![
                "provider.create",
                "provider.default",
                "provider.model",
                "provider.invoke",
                "provider.wrap",
                "provider.custom",
                "provider.descriptors",
            ]
        );
        for (descriptor, _) in handlers {
            assert!(matches!(descriptor.mode, OpMode::Call));
            assert!(descriptor.event_type.is_none());
            assert_eq!(descriptor.since_ops_version, "1.0");
        }
    }

    #[tokio::test]
    async fn descriptors_lists_packages_and_rejects_unknown_package() {
        let owner = owner();
        let all = call(owner, HandleId(0), "provider.descriptors", json!({}))
            .await
            .unwrap();
        let items = all.as_array().unwrap();
        assert_eq!(items.len(), 329);

        let openai = call(
            owner,
            HandleId(0),
            "provider.descriptors",
            json!({"package":"openai"}),
        )
        .await
        .unwrap();
        assert_eq!(openai.as_array().unwrap().len(), 1);
        assert_eq!(openai[0]["id"], "openai");
        assert_eq!(openai[0]["defaultMethod"], "languageModel");
        assert_eq!(
            openai[0]["languageApis"],
            json!(["chat", "responses", "completion"])
        );

        assert!(matches!(
            call(
                owner,
                HandleId(0),
                "provider.descriptors",
                json!({"package":"missing-pkg"})
            )
            .await,
            Err(OpsError::AiMux(AiMuxError::NoSuchProvider { .. }))
        ));
        HandleRegistry::global().release_owner(owner);
    }

    #[tokio::test]
    async fn create_supports_env_refs_allowlist_default_method_and_direct_methods() {
        let owner = owner();
        let err = call(
            owner,
            HandleId(0),
            "provider.create",
            json!({
                "package": "openai_compatible",
                "envAllowlist": ["ALLOWED_ONLY"],
                "settings": {
                    "name": "gw",
                    "baseUrl": "https://gw.example/v1",
                    "apiKey": { "env": "BLOCKED_ENV" }
                }
            }),
        )
        .await
        .unwrap_err();
        assert!(matches!(
            err,
            OpsError::AiMux(AiMuxError::InvalidArgument { .. })
        ));

        assert!(matches!(
            call(
                owner,
                HandleId(0),
                "provider.default",
                json!({"package": "openai_compatible"})
            )
            .await,
            Err(OpsError::AiMux(AiMuxError::InvalidArgument { .. }))
        ));

        let compat = call(
            owner,
            HandleId(0),
            "provider.create",
            json!({
                "package": "openai_compatible",
                "settings": {
                    "name": "my-gw",
                    "baseUrl": "https://gw.example/v1"
                }
            }),
        )
        .await
        .unwrap();
        let chat = call(
            owner,
            id(&compat),
            "provider.model",
            json!({"method": "chat", "modelId": "llama3"}),
        )
        .await
        .unwrap();
        assert_eq!(chat["kind"], "Language");

        let openai = call(
            owner,
            HandleId(0),
            "provider.default",
            json!({"package": "openai"}),
        )
        .await
        .unwrap();
        let default_model = call(
            owner,
            id(&openai),
            "provider.model",
            json!({"modelId": "gpt-4o"}),
        )
        .await
        .unwrap();
        assert_eq!(default_model["kind"], "Language");
        let completion_model = call(
            owner,
            id(&openai),
            "provider.model",
            json!({"method": "completion", "modelId": "gpt-3.5-turbo-instruct"}),
        )
        .await
        .unwrap();
        assert_eq!(completion_model["kind"], "Language");

        let tavily = call(
            owner,
            HandleId(0),
            "provider.default",
            json!({"package": "tavily"}),
        )
        .await
        .unwrap();
        let search_model = call(
            owner,
            id(&tavily),
            "provider.model",
            json!({"modelId": "tavily-search"}),
        )
        .await
        .unwrap();
        assert_eq!(search_model["kind"], "Search");

        let cartesia = call(
            owner,
            HandleId(0),
            "provider.default",
            json!({"package": "cartesia"}),
        )
        .await
        .unwrap();
        assert!(matches!(
            call(
                owner,
                id(&cartesia),
                "provider.model",
                json!({"modelId": "sonic-2"})
            )
            .await,
            Err(OpsError::AiMux(AiMuxError::UnsupportedFunctionality(_)))
        ));

        let codex = call(
            owner,
            HandleId(0),
            "provider.create",
            json!({
                "package": "codex",
                "settings": {
                    "codexMode": { "type": "chatGptAccount", "token": "tok", "accountId": "acct" }
                }
            }),
        )
        .await
        .unwrap();
        assert!(
            call(
                owner,
                id(&codex),
                "provider.model",
                json!({"method": "responses", "modelId": "gpt-5.2-codex"})
            )
            .await
            .is_ok()
        );
        HandleRegistry::global().release_owner(owner);
    }

    #[tokio::test]
    async fn custom_registers_explicit_language_apis_without_fallback() {
        let owner = owner();
        let fallback = call(
            owner,
            HandleId(0),
            "provider.default",
            json!({"package": "openai"}),
        )
        .await
        .unwrap();
        let registered = handle(
            owner,
            Entry::Language(package("openai").language_model("gpt-4o").unwrap()),
        );
        let custom = call(
            owner,
            HandleId(0),
            "provider.custom",
            json!({
                "chatModels": { "custom-chat": registered.0.to_string() },
                "fallbackProvider": fallback["handle"],
            }),
        )
        .await
        .unwrap();

        let chat = call(
            owner,
            id(&custom),
            "provider.model",
            json!({"method": "chat", "modelId": "custom-chat"}),
        )
        .await
        .unwrap();
        assert_eq!(chat["kind"], "Language");

        assert!(matches!(
            call(
                owner,
                id(&custom),
                "provider.model",
                json!({"method": "chat", "modelId": "gpt-4o"}),
            )
            .await,
            Err(OpsError::AiMux(AiMuxError::NoSuchModel { .. }))
        ));

        assert!(matches!(
            call(
                owner,
                id(&custom),
                "provider.model",
                json!({"method": "responses", "modelId": "custom-chat"}),
            )
            .await,
            Err(OpsError::AiMux(AiMuxError::UnsupportedFunctionality(_)))
        ));
        HandleRegistry::global().release_owner(owner);
    }

    #[tokio::test]
    async fn create_rejects_lone_anthropic_aws_session_token() {
        let owner = owner();
        let err = call(
            owner,
            HandleId(0),
            "provider.create",
            json!({
                "package": "anthropic_aws",
                "settings": {
                    "sessionToken": "explicit-tok",
                    "region": "us-east-1",
                    "workspaceId": "ws"
                }
            }),
        )
        .await
        .unwrap_err();
        assert!(matches!(
            err,
            OpsError::AiMux(AiMuxError::InvalidArgument { .. })
        ));
        HandleRegistry::global().release_owner(owner);
    }

    #[tokio::test]
    async fn create_rejects_preset_organization_and_project_in_favor_of_headers() {
        let owner = owner();
        for field in ["organization", "project"] {
            let err = call(
                owner,
                HandleId(0),
                "provider.create",
                json!({
                    "package": "ollama",
                    "settings": { (field): "org-or-proj" }
                }),
            )
            .await
            .unwrap_err();
            assert_eq!(err.code(), 5, "preset must reject {field}");
            assert!(matches!(
                err,
                OpsError::AiMux(AiMuxError::InvalidArgument { .. })
            ));
        }

        let allowed = call(
            owner,
            HandleId(0),
            "provider.create",
            json!({
                "package": "ollama",
                "settings": {
                    "headers": {
                        "OpenAI-Organization": "org-1",
                        "OpenAI-Project": "proj-1"
                    }
                }
            }),
        )
        .await;
        assert!(allowed.is_ok());
        HandleRegistry::global().release_owner(owner);
    }
}
