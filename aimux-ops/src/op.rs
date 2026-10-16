use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Serialize, Serializer};
use std::borrow::Cow;

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

/// A Rust wire type and its schema in the generator's active contract.
#[derive(Clone, Copy, Debug)]
pub struct TypeRef {
    pub id: fn() -> Cow<'static, str>,
    pub schema: fn(&mut SchemaGenerator) -> Schema,
}

impl TypeRef {
    #[must_use]
    pub const fn of<T: JsonSchema>() -> Self {
        Self {
            id: T::schema_id,
            schema: SchemaGenerator::subschema_for::<T>,
        }
    }
}

impl Serialize for TypeRef {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&(self.id)())
    }
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
    pub request_type: TypeRef,
    pub result_type: TypeRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_type: Option<TypeRef>,
    pub scope: Scope,
    pub terminal_type: Option<TypeRef>,
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
