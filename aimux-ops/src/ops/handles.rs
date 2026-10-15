//! Built-in runtime capabilities and their owner-scoped lifetime.

use std::sync::Arc;

use aimux_core::download::{create_default_download_function, create_download};
use aimux_core::{AbortSignal, AiMuxError};
use aimux_provider_utils::fetch::{ProxyConfig, ReqwestFetch};
use aimux_provider_utils::ws::TungsteniteConnector;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::dispatch::{CallFuture, Ctx, Handler, Target, resolve_secondary};
use crate::envelope::{BindingError, OpsError, ProtocolError};
use crate::handle::{Entry, HandleId, HandleKind, HandleRegistry};
use crate::op::{Cancellation, OpDescriptor, OpMode, RequestBytePolicy, Scope};

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AuditMode {
    #[default]
    Disabled,
    Shared,
    Strict,
}

use crate::ops::operation::EmptyParams;

#[derive(Serialize)]
struct HandleResult {
    handle: HandleId,
    kind: HandleKind,
}

#[derive(Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
enum TransportParams {
    #[serde(rename = "fetch")]
    Fetch {
        #[serde(default, deserialize_with = "deserialize_proxy")]
        proxy: ProxyConfig,
    },
    #[serde(rename = "ws")]
    WebSocket {
        #[serde(default, deserialize_with = "deserialize_proxy")]
        proxy: ProxyConfig,
    },
    #[serde(rename = "download")]
    Download {
        #[serde(default, rename = "maxBytes")]
        max_bytes: Option<usize>,
    },
}

// ProxyConfig is the canonical settings type, but its serde reader is open.
// Close the control object without defining a second transport settings type.
fn deserialize_proxy<'de, D>(deserializer: D) -> Result<ProxyConfig, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    let object = value
        .as_object()
        .ok_or_else(|| <D::Error as serde::de::Error>::custom("expected proxy settings"))?;
    if object.keys().any(|key| {
        !matches!(
            key.as_str(),
            "http_url" | "https_url" | "all_url" | "no_proxy"
        )
    }) {
        return Err(serde::de::Error::custom("unknown proxy setting"));
    }
    serde_json::from_value(value).map_err(serde::de::Error::custom)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CredentialParams {
    kind: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MiddlewareParams {
    kind: String,
    trace_store: Option<HandleId>,
    scope_name: Option<String>,
    audit_mode: Option<AuditMode>,
}

fn params<T: DeserializeOwned>(value: Value) -> Result<T, OpsError> {
    // Do not echo caller-supplied data (in particular credential material).
    serde_json::from_value(value)
        .map_err(|_| OpsError::Protocol(ProtocolError::InvalidParams("invalid parameters".into())))
}

fn unsupported(message: &'static str) -> OpsError {
    OpsError::AiMux(AiMuxError::UnsupportedFunctionality(message.into()))
}

fn insert(ctx: &Ctx, entry: Entry) -> Result<Value, OpsError> {
    let kind = entry.kind();
    let id = HandleRegistry::global().insert(ctx.owner, entry)?;
    serde_json::to_value(HandleResult { handle: id, kind })
        .map_err(|error| BindingError::ResultSerialization(error.to_string()).into())
}

fn abort_create(ctx: Ctx, _target: Target, value: Value) -> CallFuture {
    Box::pin(async move {
        let _: EmptyParams = params(value)?;
        insert(&ctx, Entry::Abort(Arc::new(AbortSignal::new())))
    })
}

fn abort_signal(_ctx: Ctx, target: Target, value: Value) -> CallFuture {
    Box::pin(async move {
        let _: EmptyParams = params(value)?;
        let Target::Handle(Entry::Abort(signal)) = target else {
            return Err(OpsError::Protocol(ProtocolError::InvalidState(
                "abort.signal requires an Abort target".into(),
            )));
        };
        signal.abort();
        Ok(Value::Null)
    })
}

fn transport_create(ctx: Ctx, _target: Target, value: Value) -> CallFuture {
    Box::pin(async move {
        let entry = match params::<TransportParams>(value)? {
            TransportParams::Fetch { proxy } => Entry::Fetch(Arc::new(ReqwestFetch::new(proxy))),
            TransportParams::WebSocket { proxy } => {
                Entry::WebSocket(Arc::new(TungsteniteConnector::new(proxy)))
            }
            TransportParams::Download { max_bytes } => {
                Entry::Download(Arc::new(create_default_download_function(
                    Some(create_download(max_bytes)),
                    ctx.abort.clone(),
                )))
            }
        };
        insert(&ctx, entry)
    })
}

fn credential_create(_ctx: Ctx, _target: Target, value: Value) -> CallFuture {
    Box::pin(async move {
        let request: CredentialParams = params(value)?;
        let _ = request.kind;
        // Resolvable callbacks and explicit provider credentials are not built-in
        // RFC-0040 resolvers. The baseline has no such resolver to construct.
        Err(unsupported("built-in credential resolvers are unavailable"))
    })
}

fn middleware_create(ctx: Ctx, _target: Target, value: Value) -> CallFuture {
    Box::pin(async move {
        let request: MiddlewareParams = params(value)?;
        if request.kind != "trace" {
            return Err(unsupported("requested built-in middleware is unavailable"));
        }
        let id = request.trace_store.ok_or_else(|| {
            OpsError::Protocol(ProtocolError::InvalidParams(
                "traceStore must be a decimal handle string".into(),
            ))
        })?;
        // Use the same owner-aware resolver as dispatch, including when the
        // store is dropped between preflight and handler execution.
        let Entry::TraceStore(store) = resolve_secondary(
            HandleRegistry::global(),
            &ctx,
            &serde_json::to_value(id)
                .map_err(|error| BindingError::ResultSerialization(error.to_string()))?,
            &[HandleKind::TraceStore],
        )?
        else {
            unreachable!("typed resolver returned a different kind")
        };
        let mode = request
            .audit_mode
            .map(|mode| match mode {
                AuditMode::Disabled => aimux_core::trace::store::TraceAuditMode::Off,
                AuditMode::Shared => aimux_core::trace::store::TraceAuditMode::Shared,
                AuditMode::Strict => aimux_core::trace::store::TraceAuditMode::Strict,
            })
            .unwrap_or_else(|| store.audit_mode());
        insert(
            &ctx,
            Entry::Middleware(Arc::new(crate::handle::Middleware::Language(Arc::new(
                TraceMiddleware {
                    store,
                    scope_name: request.scope_name,
                    audit_mode: mode,
                },
            )))),
        )
    })
}

struct TraceMiddleware {
    store: Arc<aimux_core::trace::RingTraceStore>,
    scope_name: Option<String>,
    audit_mode: aimux_core::trace::store::TraceAuditMode,
}

impl aimux_core::language_model_middleware::LanguageModelMiddleware for TraceMiddleware {
    fn wrap_model(
        &self,
        model: Arc<dyn aimux_core::LanguageModel>,
    ) -> Option<Arc<dyn aimux_core::LanguageModel>> {
        use aimux_core::trace::store::TraceAuditMode;
        let mut layer = aimux_core::trace::TraceLayer::new(model, self.store.clone());
        if let Some(name) = &self.scope_name {
            layer = layer.with_scope_name(name);
        }
        match self.audit_mode {
            TraceAuditMode::Off => {}
            TraceAuditMode::Shared => layer = layer.with_rules_auditor(false),
            TraceAuditMode::Strict => layer = layer.with_rules_auditor(true),
        }
        Some(Arc::new(layer))
    }
}

// Dispatch owns raw-ID admission and teardown before ordinary handle lookup.
fn drop_requires_id(_ctx: Ctx, _target: Target, _value: Value) -> CallFuture {
    Box::pin(async {
        Err(OpsError::Protocol(ProtocolError::InvalidState(
            "dispatch must route handle.drop before handle lookup".into(),
        )))
    })
}

pub(crate) static OPS: &[OpDescriptor] = &[
    OpDescriptor {
        name: "abort.create",
        since_ops_version: "1.0",
        mode: OpMode::Call,
        allowed_handle_kinds: &[],
        request_type: "EmptyParams",
        result_type: "aimux_ops::ops::handles::HandleResult",
        event_type: None,
        scope: Scope::Current,
        terminal_type: None,
        runtime_handle_fields: &[],
        required_capabilities: &[],
        cancellation: Cancellation::None,
        side_effects: true,
        result_ownership: "owned",
        request_byte_policy: RequestBytePolicy::Json,
        retry_owner: "none",
        source_symbols: &["aimux_core::abort_signal::AbortSignal::new"],
    },
    OpDescriptor {
        name: "abort.signal",
        since_ops_version: "1.0",
        mode: OpMode::Call,
        allowed_handle_kinds: &[HandleKind::Abort],
        request_type: "EmptyParams",
        result_type: "()",
        event_type: None,
        scope: Scope::Current,
        terminal_type: None,
        runtime_handle_fields: &[],
        required_capabilities: &[],
        cancellation: Cancellation::None,
        side_effects: true,
        result_ownership: "owned",
        request_byte_policy: RequestBytePolicy::Json,
        retry_owner: "none",
        source_symbols: &["aimux_core::abort_signal::AbortSignal::abort"],
    },
    OpDescriptor {
        name: "transport.create",
        since_ops_version: "1.0",
        mode: OpMode::Call,
        allowed_handle_kinds: &[],
        request_type: "aimux_ops::ops::handles::TransportParams",
        result_type: "aimux_ops::ops::handles::HandleResult",
        event_type: None,
        scope: Scope::Current,
        terminal_type: None,
        runtime_handle_fields: &[],
        required_capabilities: &[],
        cancellation: Cancellation::None,
        side_effects: true,
        result_ownership: "owned",
        request_byte_policy: RequestBytePolicy::Json,
        retry_owner: "none",
        source_symbols: &[
            "aimux_provider_utils::fetch::ReqwestFetch::new",
            "aimux_provider_utils::ws::TungsteniteConnector::new",
            "aimux_core::download::create_download",
            "aimux_core::download::create_default_download_function",
        ],
    },
    OpDescriptor {
        name: "credential.create",
        since_ops_version: "1.0",
        mode: OpMode::Call,
        allowed_handle_kinds: &[],
        request_type: "aimux_ops::ops::handles::CredentialParams",
        result_type: "aimux_ops::ops::handles::HandleResult",
        event_type: None,
        scope: Scope::Future,
        terminal_type: None,
        runtime_handle_fields: &[],
        required_capabilities: &[],
        cancellation: Cancellation::None,
        side_effects: true,
        result_ownership: "owned",
        request_byte_policy: RequestBytePolicy::Json,
        retry_owner: "none",
        source_symbols: &[],
    },
    OpDescriptor {
        name: "middleware.create",
        since_ops_version: "1.0",
        mode: OpMode::Call,
        allowed_handle_kinds: &[],
        request_type: "aimux_ops::ops::handles::MiddlewareParams",
        result_type: "aimux_ops::ops::handles::HandleResult",
        event_type: None,
        scope: Scope::Current,
        terminal_type: None,
        runtime_handle_fields: &[("traceStore", &[HandleKind::TraceStore])],
        required_capabilities: &[],
        cancellation: Cancellation::None,
        side_effects: true,
        result_ownership: "owned",
        request_byte_policy: RequestBytePolicy::Json,
        retry_owner: "none",
        source_symbols: &["aimux_core::trace::TraceLayer::new"],
    },
    OpDescriptor {
        name: "handle.drop",
        since_ops_version: "1.0",
        mode: OpMode::Call,
        allowed_handle_kinds: &[
            HandleKind::Provider,
            HandleKind::Registry,
            HandleKind::Language,
            HandleKind::Embedding,
            HandleKind::Image,
            HandleKind::Speech,
            HandleKind::Transcription,
            HandleKind::Reranking,
            HandleKind::Video,
            HandleKind::Search,
            HandleKind::Files,
            HandleKind::Operation,
            HandleKind::Abort,
            HandleKind::Fetch,
            HandleKind::WebSocket,
            HandleKind::Download,
            HandleKind::CredentialResolver,
            HandleKind::Middleware,
            HandleKind::TraceStore,
            HandleKind::Recorder,
            HandleKind::SessionStore,
            HandleKind::ReplaySession,
            HandleKind::Decision,
        ],
        request_type: "EmptyParams",
        result_type: "()",
        event_type: None,
        scope: Scope::Current,
        terminal_type: None,
        runtime_handle_fields: &[],
        required_capabilities: &[],
        cancellation: Cancellation::None,
        side_effects: true,
        result_ownership: "owned",
        request_byte_policy: RequestBytePolicy::Json,
        retry_owner: "none",
        source_symbols: &["aimux_ops::handle::HandleRegistry::remove"],
    },
];
pub(crate) fn register(registry: &mut Vec<(&'static OpDescriptor, Handler)>) {
    registry.extend(OPS.iter().zip([
        Handler::Call(abort_create),
        Handler::Call(abort_signal),
        Handler::Call(transport_create),
        Handler::Call(credential_create),
        Handler::Call(middleware_create),
        Handler::Call(drop_requires_id),
    ]));
}
