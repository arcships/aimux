//! Composite constructors retain language models, never their registry IDs.

use std::sync::Arc;

use aimux_core::LanguageModel;
use aimux_core::moa::{MoaConfig, MoaModel};
use aimux_core::router::{
    FallbackPolicy, Router, RouterConfig, RouterModel, RuleRouter, WeightedRouter,
};
use serde::Deserialize;
use serde_json::Value;

use crate::dispatch::{CallFuture, Ctx, Handler, Target, decode, resolve_secondary};
use crate::envelope::{OpsError, ProtocolError};
use crate::handle::{Entry, HandleKind, HandleRegistry, Owner};
use crate::op::{
    Cancellation, OPS_VERSION, OpDescriptor, OpMode, RequestBytePolicy, Scope, TypeRef,
};

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RouterParams {
    children: Vec<String>,
    strategy: Strategy,
    #[serde(default)]
    fallback: Fallback,
    provider_name: Option<String>,
    model_id: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
enum Strategy {
    Rule {},
    Weighted { weights: Vec<f64> },
}

#[derive(Default, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
enum Fallback {
    #[default]
    OnError,
    None,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MoaParams {
    references: Vec<String>,
    aggregator: String,
    #[serde(flatten)]
    config: MoaConfig,
}

fn invalid(message: impl Into<String>) -> OpsError {
    OpsError::Protocol(ProtocolError::invalid_params(message))
}

fn language(ctx: &Ctx, id: &str) -> Result<Arc<dyn LanguageModel>, OpsError> {
    match resolve_secondary(
        HandleRegistry::global(),
        ctx,
        &Value::String(id.to_owned()),
        &[HandleKind::Language],
    )? {
        Entry::Language(model) => Ok(model),
        _ => Err(invalid("expected a Language entry")),
    }
}

fn languages(ctx: &Ctx, ids: &[String]) -> Result<Vec<Arc<dyn LanguageModel>>, OpsError> {
    ids.iter().map(|id| language(ctx, id)).collect()
}

fn store(owner: Owner, model: Arc<dyn LanguageModel>) -> Result<Value, OpsError> {
    let id = HandleRegistry::global().insert(owner, Entry::Language(model))?;
    Ok(Value::String(id.0.to_string()))
}

fn router(ctx: Ctx, _target: Target, params: Value) -> CallFuture {
    Box::pin(async move {
        let params: RouterParams = decode(params)?;
        if params.children.is_empty() {
            return Err(invalid("router children must not be empty"));
        }
        let models = languages(&ctx, &params.children)?;
        let strategy: Box<dyn Router> = match params.strategy {
            Strategy::Rule {} => Box::new(RuleRouter),
            Strategy::Weighted { weights } => Box::new(WeightedRouter::new(weights)),
        };
        let fallback = match params.fallback {
            Fallback::OnError => FallbackPolicy::OnError,
            Fallback::None => FallbackPolicy::None,
        };
        let mut config = RouterConfig::default();
        if let Some(name) = params.provider_name {
            config.provider_name = name;
        }
        if let Some(id) = params.model_id {
            config.model_id = id;
        }
        // Core gives each child its own OperationSettings retry budget, including
        // stream setup; it never retries a stream after setup succeeds.
        store(
            ctx.owner,
            Arc::new(RouterModel::new(models, strategy, fallback, config)),
        )
    })
}

fn moa(ctx: Ctx, _target: Target, params: Value) -> CallFuture {
    Box::pin(async move {
        let params: MoaParams = decode(params)?;
        let references = languages(&ctx, &params.references)?;
        let aggregator = language(&ctx, &params.aggregator)?;
        store(
            ctx.owner,
            Arc::new(MoaModel::new(references, aggregator, params.config)),
        )
    })
}

pub(crate) static OPS: &[OpDescriptor] = &[
    OpDescriptor {
        name: "composite.router",
        since_ops_version: OPS_VERSION,
        mode: OpMode::Call,
        allowed_handle_kinds: &[],
        request_type: TypeRef::of::<RouterParams>(),
        result_type: TypeRef::of::<crate::handle::HandleId>(),
        event_type: None,
        scope: Scope::Current,
        terminal_type: None,
        runtime_handle_fields: &[("children.*", &[HandleKind::Language])],
        required_capabilities: &[],
        cancellation: Cancellation::None,
        result_ownership: "owned",
        request_byte_policy: RequestBytePolicy::Json,
        side_effects: true,
        retry_owner: "operation",
        source_symbols: &["aimux_router_new", "RouterModel::new"],
    },
    OpDescriptor {
        name: "composite.moa",
        since_ops_version: OPS_VERSION,
        mode: OpMode::Call,
        allowed_handle_kinds: &[],
        request_type: TypeRef::of::<MoaParams>(),
        result_type: TypeRef::of::<crate::handle::HandleId>(),
        event_type: None,
        scope: Scope::Current,
        terminal_type: None,
        runtime_handle_fields: &[
            ("references.*", &[HandleKind::Language]),
            ("aggregator", &[HandleKind::Language]),
        ],
        required_capabilities: &[],
        cancellation: Cancellation::None,
        result_ownership: "owned",
        request_byte_policy: RequestBytePolicy::Json,
        side_effects: true,
        retry_owner: "operation",
        source_symbols: &["aimux_moa_new", "MoaModel::new"],
    },
];

pub(crate) fn register(r: &mut Vec<(&'static OpDescriptor, Handler)>) {
    r.push((&OPS[0], Handler::Call(router)));
    r.push((&OPS[1], Handler::Call(moa)));
}
