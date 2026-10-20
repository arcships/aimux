//! Project the shared envelope once, then materialize it on the JS thread.

use aimux_ops::envelope::{BindingError, ErrorEnvelope, OpsError};
use napi::bindgen_prelude::*;
use napi::{Env, sys};

pub struct WireResult<T>(pub(crate) std::result::Result<T, ErrorEnvelope>);

pub(crate) fn envelope(error: OpsError) -> ErrorEnvelope {
    error.envelope()
}

pub(crate) fn parse(
    argument: &str,
    json: &str,
) -> std::result::Result<serde_json::Value, ErrorEnvelope> {
    serde_json::from_str(json).map_err(|error| {
        envelope(OpsError::Binding(BindingError::InvalidWireJson {
            argument: argument.to_owned(),
            message: error.to_string(),
        }))
    })
}

pub(crate) fn serialize(
    value: &impl serde::Serialize,
) -> std::result::Result<String, ErrorEnvelope> {
    serde_json::to_string(value).map_err(|error| {
        envelope(OpsError::Binding(BindingError::ResultSerialization(
            error.to_string(),
        )))
    })
}

pub(crate) fn throwable(env: &Env, value: ErrorEnvelope) -> napi::Result<napi::Error> {
    let mut object = env.create_error(napi::Error::from_reason(value.message.clone()))?;
    object.set("name", "AimuxError")?;
    object.set(
        "kind",
        serde_json::to_value(value.kind)
            .map_err(|error| napi::Error::from_reason(error.to_string()))?,
    )?;
    object.set("code", value.code)?;
    object.set("error", value.error)?;
    Ok(napi::Error::from(object.to_unknown()))
}

impl<T: ToNapiValue> ToNapiValue for WireResult<T> {
    unsafe fn to_napi_value(env: sys::napi_env, value: Self) -> napi::Result<sys::napi_value> {
        match value.0 {
            Ok(value) => unsafe { T::to_napi_value(env, value) },
            Err(error) => Err(throwable(&Env::from_raw(env), error)?),
        }
    }
}

impl<T: TypeName> TypeName for WireResult<T> {
    fn type_name() -> &'static str {
        T::type_name()
    }
    fn value_type() -> ValueType {
        T::value_type()
    }
}
