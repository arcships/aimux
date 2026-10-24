//! Shared, lossless error projection for every ops adapter.

use std::fmt;
use std::time::SystemTime;

use aimux_core::error::AiMuxError;
pub use aimux_core::error::SourceError;
use aimux_core::recording::RecordingError;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::handle::HandleKind;

#[derive(Debug)]
pub enum OpsError {
    Projected(Box<ProjectedError>),
    AiMux(AiMuxError),
    Recording(RecordingError),
    Binding(BindingError),
    Protocol(ProtocolError),
}

#[derive(Debug, Clone)]
pub enum BindingError {
    NullPointer {
        argument: &'static str,
    },
    InvalidUtf8 {
        argument: &'static str,
    },
    InvalidWireJson {
        argument: String,
        message: String,
        /// The JSON parser's error.
        cause: Option<SourceError>,
    },
    InvalidHandle {
        expected: Vec<HandleKind>,
        actual: Option<HandleKind>,
    },
    ReentrantCall,
    ResultSerialization {
        message: String,
        /// The serializer's error.
        cause: Option<SourceError>,
    },
    CallbackFailure(String),
}

#[derive(Debug, Clone)]
pub enum ProtocolError {
    InvalidOp(String),
    InvalidParams {
        message: String,
        /// The decoder's error, when the params did not decode.
        cause: Option<SourceError>,
    },
    UnsupportedOpsVersion(String),
    ConcurrentConsumer,
    InvalidState(String),
    ResourceLimit(String),
    HandleExhausted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ErrorDomain {
    AiMux,
    Recording,
    Binding,
    Protocol,
}

/// Keep the payload open so newer error variants, provider data, and explicit
/// JSON nulls survive decoding and re-encoding by an older adapter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(deny_unknown_fields)]
pub struct ErrorEnvelope {
    pub kind: ErrorDomain,
    pub code: i32,
    pub message: String,
    pub error: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "u64")]
    pub retry_after_ms: Option<u64>,
}

#[derive(Debug)]
pub struct ProjectedError {
    pub envelope: ErrorEnvelope,
    pub source: OpsError,
}

impl OpsError {
    /// Capture a boundary clock once. Reprojection preserves the original envelope.
    #[must_use]
    pub fn project(self, now: SystemTime) -> Self {
        if matches!(self, Self::Projected(_)) {
            self
        } else {
            let envelope = self.envelope(now);
            Self::Projected(Box::new(ProjectedError {
                envelope,
                source: self,
            }))
        }
    }
    pub(crate) fn project_with_clock(self, now: fn() -> SystemTime) -> Self {
        if matches!(self, Self::Projected(_)) {
            self
        } else {
            self.project(now())
        }
    }
    #[must_use]
    pub fn unprojected(&self) -> &Self {
        match self {
            Self::Projected(error) => error.source.unprojected(),
            error => error,
        }
    }

    /// Project once using the clock captured at the dispatch boundary. Adapters
    /// should retain this envelope rather than recompute date-based hints.
    #[must_use]
    pub fn envelope(&self, now: SystemTime) -> ErrorEnvelope {
        let (kind, error) = match self {
            Self::Projected(error) => return error.envelope.clone(),
            Self::AiMux(error) => match serde_json::to_value(error) {
                Ok(payload) => (ErrorDomain::AiMux, payload),
                Err(error) => {
                    return Self::Binding(BindingError::result_serialization(&error)).envelope(now);
                }
            },
            Self::Recording(error) => (ErrorDomain::Recording, recording_payload(error)),
            Self::Binding(error) => (ErrorDomain::Binding, error.payload()),
            Self::Protocol(error) => (ErrorDomain::Protocol, error.payload()),
        };
        let retry_after_ms = match self {
            Self::AiMux(AiMuxError::ApiCall(api)) => api
                .response_headers
                .as_ref()
                .and_then(|headers| crate::retry_hint::retry_after_ms(headers, now)),
            _ => None,
        };
        ErrorEnvelope {
            kind,
            code: self.code(),
            message: self.to_string(),
            error,
            retry_after_ms,
        }
    }

    #[must_use]
    pub fn code(&self) -> i32 {
        match self {
            Self::Projected(error) => error.envelope.code,
            Self::AiMux(error) => match error {
                AiMuxError::Other { .. } => 1,
                AiMuxError::JsonParse { .. } => 2,
                AiMuxError::InvalidResponseData { .. }
                | AiMuxError::TypeValidation { .. }
                | AiMuxError::ToolCallNotFoundForApproval { .. }
                | AiMuxError::ToolChoiceViolation(_) => 3,
                // Code 4 is retired. Preserve the existing FFI groupings.
                AiMuxError::InvalidArgument { .. } => 5,
                AiMuxError::InvalidPrompt { .. } | AiMuxError::MissingToolResults { .. } => 6,
                AiMuxError::TokenExpired(_) => 7,
                AiMuxError::UnsupportedFunctionality(_) => 8,
                AiMuxError::NoSuchModel { .. } => 9,
                AiMuxError::NoSuchProvider { .. } => 10,
                AiMuxError::ApiCall(_) => 11,
                AiMuxError::Timeout(_) => 12,
                AiMuxError::Aborted(_) => 13,
                AiMuxError::Retry(_) => 14,
                AiMuxError::NoSuchTool { .. } => 15,
                AiMuxError::InvalidToolInput { .. } => 16,
                AiMuxError::ToolCallRepair { .. } => 17,
                AiMuxError::NoOutputGenerated { .. } => 18,
                AiMuxError::LoadApiKey { .. } => 19,
                AiMuxError::LoadSetting { .. } => 20,
            },
            Self::Recording(error) => match error {
                RecordingError::Init { .. } => 100,
                RecordingError::OpenFile { .. } => 101,
                RecordingError::Spawn { .. } => 102,
                RecordingError::WriterGone => 103,
                RecordingError::FlushTimeout => 104,
                RecordingError::Write(_) => 105,
            },
            Self::Binding(error) => match error {
                BindingError::NullPointer { .. } => 200,
                BindingError::InvalidUtf8 { .. } => 201,
                BindingError::InvalidWireJson { .. } => 202,
                BindingError::InvalidHandle { .. } => 203,
                BindingError::ReentrantCall => 204,
                BindingError::ResultSerialization { .. } => 205,
                BindingError::CallbackFailure(_) => 206,
            },
            Self::Protocol(error) => match error {
                ProtocolError::InvalidOp(_) => 300,
                ProtocolError::InvalidParams { .. } => 301,
                ProtocolError::UnsupportedOpsVersion(_) => 302,
                ProtocolError::ConcurrentConsumer => 303,
                ProtocolError::InvalidState(_) => 304,
                ProtocolError::ResourceLimit(_) => 305,
                ProtocolError::HandleExhausted => 306,
            },
        }
    }
}

/// Add `cause` to a payload when there is one.
fn with_cause(mut payload: Value, cause: Option<&SourceError>) -> Value {
    if let Some(cause) = cause {
        payload["cause"] = json!(cause);
    }
    payload
}

fn recording_payload(error: &RecordingError) -> Value {
    match error {
        RecordingError::Init { path, source } => {
            json!({"name": "Init", "path": path.to_string_lossy(), "cause": SourceError::new(source)})
        }
        RecordingError::OpenFile { path, source } => {
            json!({"name": "OpenFile", "path": path.to_string_lossy(), "cause": SourceError::new(source)})
        }
        RecordingError::Spawn { source } => {
            json!({"name": "Spawn", "cause": SourceError::new(source)})
        }
        RecordingError::WriterGone => json!({"name": "WriterGone"}),
        RecordingError::FlushTimeout => json!({"name": "FlushTimeout"}),
        RecordingError::Write(message) => json!({"name": "Write", "message": message}),
    }
}

impl BindingError {
    /// A result the serializer rejected, with its error as the cause.
    #[must_use]
    pub fn result_serialization(error: &(dyn std::error::Error + 'static)) -> Self {
        Self::ResultSerialization {
            message: error.to_string(),
            cause: Some(SourceError::new(error)),
        }
    }

    fn payload(&self) -> Value {
        match self {
            Self::NullPointer { argument } => json!({"name": "NullPointer", "argument": argument}),
            Self::InvalidUtf8 { argument } => json!({"name": "InvalidUtf8", "argument": argument}),
            Self::InvalidWireJson {
                argument,
                message,
                cause,
            } => with_cause(
                json!({"name": "InvalidWireJson", "argument": argument, "message": message}),
                cause.as_ref(),
            ),
            Self::InvalidHandle { expected, actual } => {
                let mut payload = json!({"name": "InvalidHandle", "expected": expected});
                if let Some(actual) = actual {
                    payload["actual"] = json!(actual);
                }
                payload
            }
            Self::ReentrantCall => json!({"name": "ReentrantCall"}),
            Self::ResultSerialization { message, cause } => with_cause(
                json!({"name": "ResultSerialization", "message": message}),
                cause.as_ref(),
            ),
            Self::CallbackFailure(message) => {
                json!({"name": "CallbackFailure", "message": message})
            }
        }
    }
}

impl ProtocolError {
    /// Params that fail a check, with nothing underneath.
    #[must_use]
    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self::InvalidParams {
            message: message.into(),
            cause: None,
        }
    }

    fn payload(&self) -> Value {
        match self {
            Self::InvalidOp(message) => json!({"name": "InvalidOp", "message": message}),
            Self::InvalidParams { message, cause } => with_cause(
                json!({"name": "InvalidParams", "message": message}),
                cause.as_ref(),
            ),
            Self::UnsupportedOpsVersion(message) => {
                json!({"name": "UnsupportedOpsVersion", "message": message})
            }
            Self::ConcurrentConsumer => json!({"name": "ConcurrentConsumer"}),
            Self::InvalidState(message) => json!({"name": "InvalidState", "message": message}),
            Self::ResourceLimit(message) => json!({"name": "ResourceLimit", "message": message}),
            Self::HandleExhausted => json!({"name": "HandleExhausted"}),
        }
    }
}

impl fmt::Display for BindingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NullPointer { argument } => write!(f, "{argument}: must not be NULL"),
            Self::InvalidUtf8 { argument } => write!(f, "{argument}: must be valid UTF-8"),
            Self::InvalidWireJson {
                argument, message, ..
            } => {
                write!(f, "{argument}: invalid JSON: {message}")
            }
            Self::InvalidHandle { expected, actual } => {
                write!(
                    f,
                    "invalid or expired handle: expected {expected:?}, actual {actual:?}"
                )
            }
            Self::ReentrantCall => {
                f.write_str("re-entrant FFI call from within a callback is not allowed")
            }
            Self::ResultSerialization { message, .. } => {
                write!(f, "could not serialize result: {message}")
            }
            Self::CallbackFailure(message) => write!(f, "host callback failed: {message}"),
        }
    }
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidOp(message) => write!(f, "invalid op: {message}"),
            Self::InvalidParams { message, .. } => write!(f, "invalid params: {message}"),
            Self::UnsupportedOpsVersion(message) => write!(f, "unsupported ops version: {message}"),
            Self::ConcurrentConsumer => f.write_str("operation already has a consumer"),
            Self::InvalidState(message) => write!(f, "invalid state: {message}"),
            Self::ResourceLimit(message) => write!(f, "resource limit: {message}"),
            Self::HandleExhausted => f.write_str("handle IDs exhausted"),
        }
    }
}

impl fmt::Display for OpsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Projected(error) => f.write_str(&error.envelope.message),
            Self::AiMux(error) => error.fmt(f),
            Self::Recording(error) => error.fmt(f),
            Self::Binding(error) => error.fmt(f),
            Self::Protocol(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for BindingError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidWireJson { cause, .. } | Self::ResultSerialization { cause, .. } => {
                cause.as_ref().map(|cause| cause as _)
            }
            _ => None,
        }
    }
}

impl std::error::Error for ProtocolError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidParams { cause, .. } => cause.as_ref().map(|cause| cause as _),
            _ => None,
        }
    }
}
impl std::error::Error for OpsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Projected(error) => Some(&error.source),
            Self::AiMux(error) => Some(error),
            Self::Recording(error) => Some(error),
            Self::Binding(error) => Some(error),
            Self::Protocol(error) => Some(error),
        }
    }
}

impl From<AiMuxError> for OpsError {
    fn from(error: AiMuxError) -> Self {
        Self::AiMux(error)
    }
}

impl From<RecordingError> for OpsError {
    fn from(error: RecordingError) -> Self {
        Self::Recording(error)
    }
}

impl From<BindingError> for OpsError {
    fn from(error: BindingError) -> Self {
        Self::Binding(error)
    }
}

impl From<ProtocolError> for OpsError {
    fn from(error: ProtocolError) -> Self {
        Self::Protocol(error)
    }
}
