use serde::Serialize;

use crate::handle::HandleKind;

pub const OPS_VERSION: &str = "1.0";
pub const ABI_VERSION: u32 = 1 << 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OpMode {
    Call,
    Stream,
    BinaryPush,
    Poll,
}

/// Metadata and stable wire type references for one operation.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpDescriptor {
    pub name: &'static str,
    pub since_ops_version: &'static str,
    pub mode: OpMode,
    /// An empty list requires global handle 0.
    pub allowed_handle_kinds: &'static [HandleKind],
    pub request_type: &'static str,
    pub result_type: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_type: Option<&'static str>,
    pub scope: Scope,
    pub terminal_type: Option<&'static str>,
    pub runtime_handle_fields: &'static [(&'static str, &'static [HandleKind])],
    pub required_capabilities: &'static [&'static str],
    pub cancellation: Cancellation,
    pub result_ownership: &'static str,
    pub request_byte_policy: RequestBytePolicy,
    pub side_effects: bool,
    pub retry_owner: &'static str,
    pub source_symbols: &'static [&'static str],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Scope {
    Current,
    Future,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Cancellation {
    None,
    Abort,
    Operation,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RequestBytePolicy {
    Json,
    JsonWithBase64,
    Binary,
}
