//! File access and uploads use the canonical core DTOs without upload retries.

use aimux_core::files_model::UploadFileCallOptions;
use serde::Deserialize;
use serde_json::Value;

use crate::dispatch::{CallFuture, Ctx, Handler, Target, decode};
use crate::envelope::{BindingError, OpsError, ProtocolError};
use crate::handle::{Entry, HandleId, HandleKind, HandleRegistry};
use crate::op::{Cancellation, OpDescriptor, OpMode, RequestBytePolicy, Scope};

/// Runtime control is lifted separately from the canonical upload options.
#[derive(Deserialize, schemars::JsonSchema)]
pub struct UploadFileParams {
    #[serde(flatten)]
    pub options: UploadFileCallOptions,
    pub abort: Option<HandleId>,
}

pub(crate) static OPS: &[OpDescriptor] = &[
    OpDescriptor {
        name: "provider.files",
        since_ops_version: "1.0",
        mode: OpMode::Call,
        allowed_handle_kinds: &[HandleKind::Provider],
        request_type: "EmptyParams",
        result_type: "HandleId",
        event_type: None,
        scope: Scope::Current,
        terminal_type: None,
        runtime_handle_fields: &[],
        required_capabilities: &["files"],
        cancellation: Cancellation::None,
        side_effects: false,
        result_ownership: "owned",
        request_byte_policy: RequestBytePolicy::Json,
        retry_owner: "none",
        source_symbols: &[
            "aimux_core::provider::Provider::files",
            "aimux_ffi::aimux_openai_files_new",
            "aimux_ffi::aimux_openai_files_new_with_base",
        ],
    },
    OpDescriptor {
        name: "files.upload",
        since_ops_version: "1.0",
        mode: OpMode::Call,
        allowed_handle_kinds: &[HandleKind::Files],
        request_type: "aimux_ops::ops::files::UploadFileParams",
        result_type: "aimux_core::files_model::UploadFileResult",
        event_type: None,
        scope: Scope::Current,
        terminal_type: None,
        runtime_handle_fields: &[("abort", &[HandleKind::Abort])],
        required_capabilities: &["uploadFile"],
        cancellation: Cancellation::Abort,
        side_effects: true,
        result_ownership: "owned",
        request_byte_policy: RequestBytePolicy::JsonWithBase64,
        retry_owner: "none",
        source_symbols: &[
            "aimux_core::files_model::Files::upload_file",
            "aimux_ffi::aimux_file_upload",
        ],
    },
];

pub(crate) fn register(r: &mut Vec<(&'static OpDescriptor, Handler)>) {
    r.push((&OPS[0], Handler::Call(provider_files)));
    r.push((&OPS[1], Handler::Call(upload)));
}

fn provider_files(ctx: Ctx, target: Target, params: Value) -> CallFuture {
    Box::pin(async move {
        decode::<super::operation::EmptyParams>(params)?;
        let Target::Handle(Entry::Provider(provider)) = target else {
            return Err(invalid_target(&target, HandleKind::Provider));
        };
        let files = provider.files().ok_or_else(|| {
            aimux_core::AiMuxError::UnsupportedFunctionality(
                "provider does not support files".into(),
            )
        })?;
        let id = HandleRegistry::global().insert(ctx.owner, Entry::Files(files))?;
        serialize(id)
    })
}

// Core business DTOs stay open; close their control keys at the ops boundary.
fn parse_options(params: Value) -> Result<UploadFileParams, OpsError> {
    reject_unknown(
        &params,
        &["data", "mediaType", "filename", "providerOptions", "abort"],
    )?;
    if let Some(data) = params.get("data") {
        let fields: &[&str] = match data.get("type").and_then(Value::as_str) {
            Some("data") => &["type", "data"],
            Some("text") => &["type", "text"],
            _ => &["type"],
        };
        reject_unknown(data, fields)?;
    }
    decode(params)
}

fn reject_unknown(value: &Value, allowed: &[&str]) -> Result<(), OpsError> {
    if let Some(fields) = value.as_object() {
        for field in fields.keys() {
            if !allowed.contains(&field.as_str()) {
                return Err(
                    ProtocolError::InvalidParams(format!("unknown field `{field}`")).into(),
                );
            }
        }
    }
    Ok(())
}

fn invalid_target(target: &Target, expected: HandleKind) -> OpsError {
    BindingError::InvalidHandle {
        expected: vec![expected],
        actual: match target {
            Target::Global => None,
            Target::Handle(entry) => Some(entry.kind()),
        },
    }
    .into()
}

fn serialize(value: impl serde::Serialize) -> Result<Value, OpsError> {
    serde_json::to_value(value)
        .map_err(|error| BindingError::ResultSerialization(error.to_string()).into())
}

fn upload(ctx: Ctx, target: Target, params: Value) -> CallFuture {
    Box::pin(async move {
        let Target::Handle(Entry::Files(model)) = target else {
            return Err(invalid_target(&target, HandleKind::Files));
        };
        let mut options = parse_options(params)?.options;
        // Dispatch has resolved the optional abort handle with the caller's owner.
        options.abort_signal = ctx.abort;
        let result = model.upload_file(&options).await?;
        serialize(result)
    })
}
