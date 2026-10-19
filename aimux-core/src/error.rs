//! Error types for aimux-core.

use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use ts_rs::TS;

/// Projects response retry headers to milliseconds using a caller-supplied clock.
///
/// A numeric `retry-after-ms` takes precedence, even when negative or out of
/// range. An absent or malformed millisecond value falls back to
/// `retry-after` seconds, then an HTTP date. Only finite, non-negative delays
/// fitting in `u64` are returned; fractional milliseconds are truncated.
#[must_use]
pub fn retry_after_ms_from_headers(
    headers: &HashMap<String, String>,
    now: SystemTime,
) -> Option<u64> {
    fn header<'a>(headers: &'a HashMap<String, String>, name: &str) -> Option<&'a str> {
        headers
            .get(name)
            .or_else(|| {
                headers
                    .iter()
                    .find(|(key, _)| key.eq_ignore_ascii_case(name))
                    .map(|(_, value)| value)
            })
            .map(|value| value.trim())
    }

    fn milliseconds(value: f64) -> Option<u64> {
        // u64::MAX rounds up to 2^64 as f64, so the upper bound is exclusive.
        if value.is_finite() && (0.0..u64::MAX as f64).contains(&value) {
            Some(value.trunc() as u64)
        } else {
            None
        }
    }

    if let Some(value) =
        header(headers, "retry-after-ms").and_then(|value| value.parse::<f64>().ok())
    {
        return milliseconds(value);
    }

    let value = header(headers, "retry-after")?;
    if let Ok(seconds) = value.parse::<f64>() {
        return milliseconds(seconds * 1_000.0);
    }

    let date = httpdate::parse_http_date(value).ok()?;
    let delay = date.duration_since(now).ok()?;
    u64::try_from(delay.as_millis()).ok()
}

/// What a failed API call observed — the field set of the AI SDK's
/// `APICallError` (`statusCode` / `responseBody`), which this project's error
/// taxonomy mirrors (docs/internal/vercel-research/07-kernel-infrastructure.md).
///
/// Carried by [`AiMuxError::ApiCall`] alone. There are no per-status
/// variants: a 401, 404 or 429 is an `ApiCall` error whose classification is
/// read from `status_code`, exactly as the AI SDK reads
/// `APICallError.statusCode`. Transport failures (connection reset, DNS,
/// body read) are `ApiCall` errors too — no response arrived, so
/// `status_code` is `None` and `is_retryable` is `true`, exactly as the AI
/// SDK's `handleFetchError` builds an `APICallError` with no `statusCode`
/// and `isRetryable: true`. `message` holds the provider's text verbatim when
/// one is available; transport and parsing failures use a library message
/// with the source detail appended. The HTTP status is always a field, never
/// baked into the string; `Display` composes the human-readable form from the
/// fields at print time, so nothing downstream has to parse it back out.
///
/// Every producer must provide the sanitized request URL and request values.
/// This keeps transport and response failures self-contained and matches the
/// AI SDK's `APICallError` contract.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts-export", ts(export))]
pub struct ApiCallError {
    /// Sanitized request URL. Required for every API-derived failure.
    pub url: String,
    /// Sanitized request values used to create the API request.
    pub request_body_values: serde_json::Value,
    /// HTTP status of the response, when it came from one
    /// (`APICallError.statusCode`). The HTTP layer fills the observed status
    /// for every response-derived error. An error frame inside a stream
    /// carries the status its provider maps it to, as upstream's provider
    /// stream errors do (OpenAI infers one from the frame's `code`/`type`,
    /// Anthropic from its error type); errors built without an HTTP exchange
    /// leave it `None`.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_code: Option<u16>,
    /// Provider's machine-readable code (OpenAI's `code`/`type`,
    /// e.g. `"rate_limit_exceeded"` vs `"insufficient_quota"`). Never an HTTP
    /// status. Our normalized take on `APICallError.data`.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_code: Option<String>,
    /// Human-readable failure text. Provider text is verbatim when available;
    /// locally detected transport/parse failures include their source detail.
    /// Never includes an HTTP status prefix.
    pub message: String,
    /// The raw response body, verbatim (`APICallError.responseBody`) — the
    /// lossless evidence when `message`/`provider_code` are extractions.
    /// `None` when the error did not come from a response body.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_body: Option<String>,
    /// Sanitized response headers.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_headers: Option<HashMap<String, String>>,
    /// Parsed provider error data.
    #[ts(optional)]
    #[serde(
        default,
        deserialize_with = "crate::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub data: Option<serde_json::Value>,
    /// Whether retrying can help (`APICallError.isRetryable`) — stored at
    /// construction, exactly like the AI SDK: the response path computes it
    /// from the status (408/409/429 or 5xx), and the transport path (no response, so
    /// no status to compute from) sets it to `true` explicitly.
    #[serde(default)]
    pub is_retryable: bool,
    /// The error this failure was built from (`APICallError.cause`): the
    /// transport error with its source chain, the body parse error, or the
    /// aimux error a response handler returned.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause: Option<ErrorCause>,
}

impl ApiCallError {
    /// Create an API failure with its required request context.
    pub fn new(
        message: impl Into<String>,
        url: impl Into<String>,
        request_body_values: serde_json::Value,
    ) -> Self {
        Self {
            url: url.into(),
            request_body_values,
            status_code: None,
            provider_code: None,
            message: message.into(),
            response_body: None,
            response_headers: None,
            data: None,
            is_retryable: false,
            cause: None,
        }
    }

    /// Set the error this failure was built from.
    #[must_use]
    pub fn caused_by(self, cause: impl Into<ErrorCause>) -> Self {
        Self {
            cause: Some(cause.into()),
            ..self
        }
    }
}

/// Why the retry wrapper stopped retrying.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts-export", ts(export))]
pub enum RetryErrorReason {
    /// Every permitted retry attempt failed with a retryable error.
    MaxRetriesExceeded,
    /// A later attempt produced a non-retryable error.
    ErrorNotRetryable,
}

/// Complete error history for a retried model operation.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts-export", ts(export))]
pub struct RetryError {
    pub reason: RetryErrorReason,
    pub errors: Vec<AiMuxError>,
}

static EMPTY_RETRY_HISTORY_ERROR: LazyLock<AiMuxError> =
    LazyLock::new(|| AiMuxError::other("Retry failed without recorded errors"));

impl RetryError {
    /// The final attempt error, or a stable invalid-history error when a
    /// deserialized or manually constructed value has no attempts.
    #[must_use]
    pub fn last_error(&self) -> &AiMuxError {
        match self.errors.last() {
            Some(error) => error,
            None => &EMPTY_RETRY_HISTORY_ERROR,
        }
    }
}

/// The text the AI SDK's `getErrorMessage` gives for an attempt error
/// (`@ai-sdk/provider` `errors/get-error-message.ts`), which the retry
/// wrapper puts after "Last error:"
/// (`@ai-sdk/provider-utils` `retry-with-exponential-backoff.ts:97`).
///
/// For an API failure that is the provider's message alone: the AI SDK's
/// `APICallError.message` never carries the HTTP status, so neither does this
/// text. (JavaScript's `Error.toString()` also prefixes the class name, such as
/// `AI_APICallError: `; Aimux errors have no JavaScript class name, so only
/// the message part is reproduced.) Every other variant is its own message.
fn retry_attempt_message(error: &AiMuxError) -> String {
    match error {
        AiMuxError::ApiCall(api) if !api.message.is_empty() => api.message.clone(),
        AiMuxError::ApiCall(api) => api.to_string(),
        other => other.to_string(),
    }
}

impl std::fmt::Display for RetryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let attempts = self.errors.len();
        let Some(last_error) = self.errors.last() else {
            return f.write_str("Retry failed without recorded errors");
        };
        let last_error = retry_attempt_message(last_error);
        match self.reason {
            RetryErrorReason::MaxRetriesExceeded => {
                write!(
                    f,
                    "Failed after {attempts} attempts. Last error: {last_error}"
                )
            }
            RetryErrorReason::ErrorNotRetryable => write!(
                f,
                "Failed after {attempts} attempts with non-retryable error: '{last_error}'"
            ),
        }
    }
}

/// The last attempt, like the AI SDK's `RetryError.lastError`.
impl std::error::Error for RetryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.errors.last().map(|error| error as _)
    }
}

impl std::error::Error for ApiCallError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.cause.as_ref().map(|cause| cause as _)
    }
}

impl std::fmt::Display for ApiCallError {
    /// `HTTP {status}: {message}` when a status is known (just `HTTP {status}`
    /// if the response had no body), else `{message}`. `provider_code` and
    /// `response_body` are machine-readable only and stay out of the text.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (self.status_code, self.message.is_empty()) {
            (Some(status), false) => write!(f, "HTTP {}: {}", status, self.message),
            (Some(status), true) => write!(f, "HTTP {status}"),
            (None, _) => f.write_str(&self.message),
        }
    }
}

/// An error from outside aimux (a transport, parser or I/O error), kept as
/// its message and the messages of its own sources, so it can be cloned,
/// serialized and sent across the bindings with the error built from it.
///
/// The JSON form is a JavaScript `Error`'s own `{ message, cause }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts-export", ts(export))]
pub struct SourceError {
    /// The error's `Display` text.
    pub message: String,
    /// The error's own `source()`, recursively.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause: Option<Box<SourceError>>,
}

impl SourceError {
    /// Record `error` and its whole `source()` chain.
    #[must_use]
    pub fn new(error: &(dyn std::error::Error + 'static)) -> Self {
        Self {
            message: error.to_string(),
            cause: error.source().map(|source| Box::new(Self::new(source))),
        }
    }

    /// A failure described by `message`, caused by `source` and its chain.
    #[must_use]
    pub fn with_cause(
        message: impl Into<String>,
        source: &(dyn std::error::Error + 'static),
    ) -> Self {
        Self {
            message: message.into(),
            cause: Some(Box::new(Self::new(source))),
        }
    }
}

/// A failure detected in place, with nothing underneath it.
impl From<String> for SourceError {
    fn from(message: String) -> Self {
        Self {
            message,
            cause: None,
        }
    }
}

impl From<&str> for SourceError {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}

impl std::fmt::Display for SourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for SourceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.cause.as_deref().map(|cause| cause as _)
    }
}

/// The error another error was built from: the AI SDK's `cause`.
///
/// An aimux error stays whole, as the AI SDK nests a `JSONParseError` in an
/// `APICallError`; any other error is a [`SourceError`]. The JSON form is the
/// nested error itself: a `name`-tagged aimux error, or `{ message, cause }`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(untagged)]
#[cfg_attr(feature = "ts-export", ts(export))]
pub enum ErrorCause {
    AiMux(Box<AiMuxError>),
    Source(SourceError),
}

impl ErrorCause {
    /// The cause for `error`: an aimux error is kept whole, any other error
    /// is recorded with its whole `source()` chain.
    #[must_use]
    pub fn source(error: &(dyn std::error::Error + 'static)) -> Self {
        match error.downcast_ref::<AiMuxError>() {
            Some(error) => error.clone().into(),
            None => Self::Source(SourceError::new(error)),
        }
    }

    fn as_error(&self) -> &(dyn std::error::Error + 'static) {
        match self {
            Self::AiMux(error) => error.as_ref(),
            Self::Source(error) => error,
        }
    }
}

impl From<AiMuxError> for ErrorCause {
    fn from(error: AiMuxError) -> Self {
        Self::AiMux(Box::new(error))
    }
}

impl From<SourceError> for ErrorCause {
    fn from(error: SourceError) -> Self {
        Self::Source(error)
    }
}

impl std::fmt::Display for ErrorCause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self.as_error(), f)
    }
}

impl std::error::Error for ErrorCause {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.as_error().source()
    }
}

/// Unified error type for all aimux operations.
///
/// Variants are cut by *what the caller does about it*, not by where the
/// failure came from. `ApiCallError` is boxed only to keep the Rust enum
/// compact; serde and every binding still observe the same object shape.
///
/// A variant has a `cause` exactly where its AI SDK class takes one; it is
/// what `source()` returns. `Retry` reports its
/// last attempt and `ToolCallRepair` its `cause` the same way.
#[derive(Debug, Clone, Serialize, Deserialize, TS, Error)]
#[serde(from = "AiMuxErrorWire", into = "AiMuxErrorWire")]
#[ts(as = "AiMuxErrorWire")]
#[cfg_attr(feature = "ts-export", ts(export))]
pub enum AiMuxError {
    #[error("API call error: {0}")]
    ApiCall(#[source] Box<ApiCallError>),

    #[error(transparent)]
    Retry(RetryError),

    #[error("JSON parse error: {message}")]
    JsonParse {
        message: String,
        #[source]
        cause: Option<ErrorCause>,
    },

    #[error("{message}")]
    NoOutputGenerated {
        message: String,
        #[source]
        cause: Option<ErrorCause>,
    },

    /// Malformed response data: the AI SDK's `InvalidResponseDataError`, and
    /// its `TypeValidationError` for a well-formed body of the wrong shape.
    #[error("invalid response data: {message}")]
    InvalidResponseData {
        message: String,
        #[source]
        cause: Option<ErrorCause>,
    },

    #[error("Tool call \"{tool_call_id}\" not found for approval request \"{approval_id}\".")]
    ToolCallNotFoundForApproval {
        tool_call_id: String,
        approval_id: String,
    },

    /// A model requested a tool that was not present in the call's tool set.
    /// Message templates for the three tool variants match the AI SDK verbatim
    /// (`NoSuchToolError` / `InvalidToolInputError` / `ToolCallRepairError`):
    /// the AI SDK feeds these strings back to the model as tool-error content,
    /// so the wording — including the available-tools list — is contract.
    #[error("Model tried to call unavailable tool '{tool_name}'. {}", no_such_tool_availability(.available_tools))]
    NoSuchTool {
        tool_name: String,
        available_tools: Option<Vec<String>>,
        /// Original argument text, when supplied by the provider. Absent in
        /// older serialized errors and errors constructed without a call.
        tool_input: Option<String>,
    },

    /// A tool call could not be parsed or did not satisfy its input schema.
    /// `cause` is the parse error (`AI_JSONParseError`) or the schema
    /// validation failure.
    #[error("Invalid input for tool {tool_name}: {}", cause_message(.cause))]
    InvalidToolInput {
        tool_name: String,
        tool_input: String,
        #[source]
        cause: ErrorCause,
    },

    /// The repair callback failed or returned a call that was still invalid.
    #[error("Error repairing tool call: {cause}")]
    ToolCallRepair {
        original_error: Box<AiMuxError>,
        #[source]
        cause: Box<AiMuxError>,
    },

    #[error("invalid argument: {message}")]
    InvalidArgument {
        message: String,
        #[source]
        cause: Option<ErrorCause>,
    },

    #[error("invalid prompt: {message}")]
    InvalidPrompt {
        message: String,
        #[source]
        cause: Option<ErrorCause>,
    },

    /// No API key was passed and the fallback environment variable is unset
    /// (the AI SDK's `LoadAPIKeyError`). Pre-HTTP: no request was made.
    #[error(
        "No API key found for {description}. \
         Please provide it via the `api_key` parameter \
         or set the `{env_var}` environment variable."
    )]
    LoadApiKey {
        /// The environment variable consulted as the fallback.
        env_var: String,
        /// What the key is for, e.g. `"OpenAI"`.
        description: String,
    },

    /// A required provider setting was not passed and its fallback
    /// environment variable is unset (the AI SDK's `LoadSettingError`).
    #[error(
        "No value found for setting `{name}`. \
         Please provide it via the `{name}` parameter \
         or set the `{env_var}` environment variable."
    )]
    LoadSetting {
        /// The environment variable consulted as the fallback.
        env_var: String,
        /// The setting's parameter name, e.g. `"region"`.
        name: String,
    },

    /// The prompt has tool calls without tool results (the AI SDK's
    /// `MissingToolResultsError`, `ai` `error/missing-tool-result-error.ts`).
    /// Message verbatim from upstream (`:15-20`).
    #[error("{}", missing_tool_results_message(.tool_call_ids))]
    MissingToolResults { tool_call_ids: Vec<String> },

    /// The model response did not satisfy an enforced tool choice (the AI
    /// SDK's `ToolChoiceViolationError`).
    #[error("{0}")]
    ToolChoiceViolation(Box<ToolChoiceViolationError>),

    /// The access token has expired (or was invalidated) and must be refreshed
    /// by the caller. RFC-0018 subscription mode: the library maps a 401 from
    /// the Codex subscription endpoint to this variant; the integrator
    /// orchestrates `codex_refresh` + retry.
    ///
    /// This is *not* a status-code avatar: `Auth`-style 401s and token
    /// expiry share the same status, so `status_code` alone cannot express
    /// the difference — the variant carries the extra bit ("refresh helps").
    #[error("token expired: {0}")]
    TokenExpired(String),

    #[error("unsupported functionality: {0}")]
    UnsupportedFunctionality(String),

    /// Registry-level "model id does not resolve" (the AI SDK's
    /// `NoSuchModelError { modelId, modelType }`). Pre-HTTP: the upstream API
    /// was never called, so there is no status.
    #[error("no such model: {model_id}")]
    NoSuchModel {
        model_id: String,
        /// What kind of model was requested (`"languageModel"`,
        /// `"imageModel"`, …), the AI SDK's `modelType`.
        model_type: String,
    },

    /// Registry-level "provider name does not resolve" (the AI SDK's
    /// `NoSuchProviderError`).
    ///
    /// The message adds up to three close names from `available_providers`
    /// (an aimux addition to the upstream text; the fields are unchanged).
    #[error("No such provider: {provider_id}{} (available providers: {})",
        did_you_mean(.provider_id, .available_providers), .available_providers.join(","))]
    NoSuchProvider {
        provider_id: String,
        model_id: String,
        model_type: String,
        available_providers: Vec<String>,
    },

    #[error("request timed out: {0}")]
    Timeout(String),

    #[error("{0}")]
    Aborted(String),

    /// Any other failure: the AI SDK's base `AISDKError`.
    #[error("{message}")]
    Other {
        message: String,
        #[source]
        cause: Option<ErrorCause>,
    },
}

/// The AI SDK's `getErrorMessage(cause)`: an error's own message, without the
/// variant prefix [`AiMuxError`]'s `Display` adds.
fn cause_message(cause: &ErrorCause) -> String {
    match cause {
        ErrorCause::AiMux(error) => match error.as_ref() {
            AiMuxError::JsonParse { message, .. }
            | AiMuxError::InvalidResponseData { message, .. }
            | AiMuxError::InvalidArgument { message, .. }
            | AiMuxError::InvalidPrompt { message, .. } => message.clone(),
            other => other.to_string(),
        },
        ErrorCause::Source(error) => error.message.clone(),
    }
}

impl AiMuxError {
    /// A [`JsonParse`](Self::JsonParse) error with no cause.
    #[must_use]
    pub fn json_parse(message: impl Into<String>) -> Self {
        Self::JsonParse {
            message: message.into(),
            cause: None,
        }
    }

    /// A [`NoOutputGenerated`](Self::NoOutputGenerated) error with no cause.
    #[must_use]
    pub fn no_output_generated(message: impl Into<String>) -> Self {
        Self::NoOutputGenerated {
            message: message.into(),
            cause: None,
        }
    }

    /// An [`InvalidResponseData`](Self::InvalidResponseData) error with no cause.
    #[must_use]
    pub fn invalid_response_data(message: impl Into<String>) -> Self {
        Self::InvalidResponseData {
            message: message.into(),
            cause: None,
        }
    }

    /// An [`InvalidArgument`](Self::InvalidArgument) error with no cause.
    #[must_use]
    pub fn invalid_argument(message: impl Into<String>) -> Self {
        Self::InvalidArgument {
            message: message.into(),
            cause: None,
        }
    }

    /// An [`InvalidPrompt`](Self::InvalidPrompt) error with no cause.
    #[must_use]
    pub fn invalid_prompt(message: impl Into<String>) -> Self {
        Self::InvalidPrompt {
            message: message.into(),
            cause: None,
        }
    }

    /// An [`Other`](Self::Other) error with no cause.
    #[must_use]
    pub fn other(message: impl Into<String>) -> Self {
        Self::Other {
            message: message.into(),
            cause: None,
        }
    }
}

/// The JSON form of [`AiMuxError`]: internally tagged by `name`, with the AI
/// SDK's error names (`AI_APICallError`, ...). `AiMuxError` serializes through
/// this mirror because its tuple variants cannot carry an internal tag.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "name", rename_all_fields = "camelCase")]
pub enum AiMuxErrorWire {
    #[serde(rename = "AI_APICallError")]
    ApiCall(Box<ApiCallError>),
    #[serde(rename = "AI_RetryError")]
    Retry(RetryError),
    #[serde(rename = "AI_JSONParseError")]
    JsonParse {
        message: String,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cause: Option<ErrorCause>,
    },
    #[serde(rename = "AI_NoOutputGeneratedError")]
    NoOutputGenerated {
        message: String,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cause: Option<ErrorCause>,
    },
    #[serde(rename = "AI_InvalidResponseDataError")]
    InvalidResponseData {
        message: String,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cause: Option<ErrorCause>,
    },
    #[serde(rename = "AI_ToolCallNotFoundForApprovalError")]
    ToolCallNotFoundForApproval {
        tool_call_id: String,
        approval_id: String,
    },
    #[serde(rename = "AI_NoSuchToolError")]
    NoSuchTool {
        tool_name: String,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        available_tools: Option<Vec<String>>,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_input: Option<String>,
    },
    #[serde(rename = "AI_InvalidToolInputError")]
    InvalidToolInput {
        tool_name: String,
        tool_input: String,
        cause: ErrorCause,
    },
    #[serde(rename = "AI_ToolCallRepairError")]
    ToolCallRepair {
        original_error: Box<AiMuxError>,
        cause: Box<AiMuxError>,
    },
    #[serde(rename = "AI_InvalidArgumentError")]
    InvalidArgument {
        message: String,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cause: Option<ErrorCause>,
    },
    #[serde(rename = "AI_InvalidPromptError")]
    InvalidPrompt {
        message: String,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cause: Option<ErrorCause>,
    },
    #[serde(rename = "AI_LoadAPIKeyError")]
    LoadApiKey {
        env_var: String,
        description: String,
    },
    #[serde(rename = "AI_LoadSettingError")]
    LoadSetting {
        env_var: String,
        /// `name` is the union's tag key, so the setting's name is `settingName`.
        #[serde(rename = "settingName")]
        name: String,
    },
    #[serde(rename = "AI_TokenExpiredError")]
    TokenExpired { message: String },
    #[serde(rename = "AI_UnsupportedFunctionalityError")]
    UnsupportedFunctionality { message: String },
    #[serde(rename = "AI_NoSuchModelError")]
    NoSuchModel {
        model_id: String,
        #[serde(default)]
        model_type: String,
    },
    #[serde(rename = "AI_NoSuchProviderError")]
    NoSuchProvider {
        provider_id: String,
        model_id: String,
        model_type: String,
        available_providers: Vec<String>,
    },
    #[serde(rename = "AI_MissingToolResultsError")]
    MissingToolResults { tool_call_ids: Vec<String> },
    #[serde(rename = "AI_ToolChoiceViolationError")]
    ToolChoiceViolation(Box<ToolChoiceViolationError>),
    #[serde(rename = "TimeoutError")]
    Timeout { message: String },
    #[serde(rename = "AbortError")]
    Aborted { message: String },
    #[serde(rename = "AI_Error")]
    Other {
        message: String,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cause: Option<ErrorCause>,
    },
}

/// Maps each `AiMuxError` variant to its `AiMuxErrorWire` twin and back:
/// tuple variants carry their payload as `message`, struct variants keep
/// their fields.
macro_rules! wire_variants {
    (
        newtype: $($nv:ident),* ;
        message: $($mv:ident),* ;
        fields: $($fv:ident { $($f:ident),* }),*
    ) => {
        impl From<AiMuxError> for AiMuxErrorWire {
            fn from(e: AiMuxError) -> Self {
                match e {
                    $(AiMuxError::$nv(v) => Self::$nv(v),)*
                    $(AiMuxError::$mv(message) => Self::$mv { message },)*
                    $(AiMuxError::$fv { $($f),* } => Self::$fv { $($f),* },)*
                }
            }
        }
        impl From<AiMuxErrorWire> for AiMuxError {
            fn from(e: AiMuxErrorWire) -> Self {
                match e {
                    $(AiMuxErrorWire::$nv(v) => Self::$nv(v),)*
                    $(AiMuxErrorWire::$mv { message } => Self::$mv(message),)*
                    $(AiMuxErrorWire::$fv { $($f),* } => Self::$fv { $($f),* },)*
                }
            }
        }
    };
}

wire_variants! {
    newtype: ApiCall, Retry, ToolChoiceViolation;
    message: TokenExpired, UnsupportedFunctionality, Timeout, Aborted;
    fields: JsonParse { message, cause },
        NoOutputGenerated { message, cause },
        InvalidResponseData { message, cause },
        ToolCallNotFoundForApproval { tool_call_id, approval_id },
        NoSuchTool { tool_name, available_tools, tool_input },
        InvalidToolInput { tool_name, tool_input, cause },
        ToolCallRepair { original_error, cause },
        InvalidArgument { message, cause },
        InvalidPrompt { message, cause },
        LoadApiKey { env_var, description },
        LoadSetting { env_var, name },
        NoSuchModel { model_id, model_type },
        NoSuchProvider { provider_id, model_id, model_type, available_providers },
        MissingToolResults { tool_call_ids },
        Other { message, cause }
}

/// `". Did you mean 'a', 'b'?"` for the names in `candidates` closest to
/// `name`, or `""` when none is close. Names compare case-insensitively with
/// `-` read as `_` (the built-in names use `_`); a candidate is close when its
/// edit distance (adjacent swaps count once) is at most 2 and at most a third
/// of the longer name. At most three names, nearest first.
fn did_you_mean(name: &str, candidates: &[String]) -> String {
    fn normalize(name: &str) -> Vec<char> {
        name.chars()
            .map(|c| {
                if c == '-' {
                    '_'
                } else {
                    c.to_ascii_lowercase()
                }
            })
            .collect()
    }
    let wanted = normalize(name);
    let mut close: Vec<(usize, &str)> = candidates
        .iter()
        .filter_map(|candidate| {
            let distance = edit_distance(&wanted, &normalize(candidate));
            let longer = wanted.len().max(candidate.chars().count());
            (distance <= 2 && distance * 3 <= longer).then_some((distance, candidate.as_str()))
        })
        .collect();
    close.sort_unstable();
    close.dedup_by(|a, b| a.1 == b.1);
    if close.is_empty() {
        return String::new();
    }
    let names: Vec<String> = close
        .iter()
        .take(3)
        .map(|(_, name)| format!("'{name}'"))
        .collect();
    format!(". Did you mean {}?", names.join(", "))
}

/// Optimal-string-alignment distance: insertions, deletions, substitutions
/// and swaps of two adjacent characters each cost one.
fn edit_distance(a: &[char], b: &[char]) -> usize {
    let mut rows = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in rows.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in rows[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut best = (rows[i - 1][j] + 1)
                .min(rows[i][j - 1] + 1)
                .min(rows[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                best = best.min(rows[i - 2][j - 2] + 1);
            }
            rows[i][j] = best;
        }
    }
    rows[a.len()][b.len()]
}

fn missing_tool_results_message(tool_call_ids: &[String]) -> String {
    let many = tool_call_ids.len() > 1;
    format!(
        "Tool result{} missing for tool call{} {}.",
        if many { "s are" } else { " is" },
        if many { "s" } else { "" },
        tool_call_ids.join(", ")
    )
}

/// What [`AiMuxError::ToolChoiceViolation`] carries: the field set of the AI
/// SDK's `ToolChoiceViolationError` (`ai` `error/tool-choice-violation-error.ts`).
#[derive(Debug, Clone, Serialize, Deserialize, TS, Error)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts-export", ts(export))]
#[error("{}", tool_choice_violation_message(.tool_choice))]
pub struct ToolChoiceViolationError {
    /// The enforced tool choice (`required`, or `tool` with a name) that the
    /// response did not satisfy.
    pub tool_choice: crate::tool::LanguageModelV4ToolChoice,
    /// Why the model finished.
    pub finish_reason: crate::types::FinishReasonUnified,
    /// The provider that returned the response.
    pub provider: String,
    /// The model that returned the response.
    pub model_id: String,
    /// The content the model returned, before tool-call parsing, so a tool
    /// call returned as text or reasoning can be recovered.
    pub content: Vec<crate::result::GenerateContent>,
}

/// Upstream's message (`tool-choice-violation-error.ts:56-58`).
fn tool_choice_violation_message(tool_choice: &crate::tool::LanguageModelV4ToolChoice) -> String {
    match tool_choice {
        crate::tool::LanguageModelV4ToolChoice::Tool { tool_name } => {
            format!("Model response did not contain a call to the required tool '{tool_name}'.")
        }
        _ => "Model response did not contain a tool call even though tool choice was required."
            .to_owned(),
    }
}

fn no_such_tool_availability(available_tools: &Option<Vec<String>>) -> String {
    match available_tools {
        Some(tools) => format!("Available tools: {}.", tools.join(", ")),
        None => "No tools are available.".to_string(),
    }
}

/// Canonical serde classification: syntactically broken/truncated JSON is a
/// parse failure; well-formed JSON that violates the expected shape is invalid
/// response data. `Io` has no transport context here, so it stays `JsonParse`;
/// call sites with a concrete transport error should preserve that instead.
impl From<serde_json::Error> for AiMuxError {
    fn from(e: serde_json::Error) -> Self {
        use serde_json::error::Category;
        let message = e.to_string();
        let cause = Some(ErrorCause::source(&e));
        match e.classify() {
            Category::Data => AiMuxError::InvalidResponseData { message, cause },
            Category::Syntax | Category::Eof | Category::Io => {
                AiMuxError::JsonParse { message, cause }
            }
        }
    }
}

/// The AI SDK's status-based retryability rule: 408 (timeout), 409
/// (conflict), 429 (rate limit) and every 5xx are worth retrying; everything
/// else is not. One definition — response handlers and providers must not
/// hand-roll this formula.
#[must_use]
pub fn is_retryable_status(status: u16) -> bool {
    matches!(status, 408 | 409 | 429) || status >= 500
}

impl AiMuxError {
    /// Build a [`NoSuchModel`](Self::NoSuchModel) for `model_id`.
    ///
    /// `model_type` is the AI SDK's `modelType` (`"languageModel"`,
    /// `"embeddingModel"`, `"imageModel"`, …).
    #[must_use]
    pub fn no_such_model(model_id: &str, model_type: &str) -> Self {
        Self::NoSuchModel {
            model_id: model_id.to_string(),
            model_type: model_type.to_string(),
        }
    }

    /// Returns `true` for a malformed individual stream frame that does not
    /// prove the underlying transport has failed. Stream reducers may report
    /// these errors and continue polling for later, independently framed data.
    #[must_use]
    pub fn is_recoverable_stream_error(&self) -> bool {
        matches!(
            self,
            Self::JsonParse { .. } | Self::InvalidResponseData { .. }
        )
    }

    /// Convert caller cancellation into the public error model.
    #[must_use]
    pub fn from_abort_signal(_signal: &crate::AbortSignal) -> Self {
        Self::Aborted("request aborted".into())
    }

    /// Returns `true` if the error is transient and can be retried.
    ///
    /// Reads the stored [`ApiCallError::is_retryable`] field, filled at
    /// construction (the AI SDK's `APICallError.isRetryable`): 408/409/429/5xx on the
    /// response path, `true` on the transport path. `Timeout` is a spent
    /// caller-side time budget — the AI SDK treats it as part of the abort
    /// family (`isAbortError` matches `"TimeoutError"`) and does not retry
    /// it; neither do we.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        match self {
            AiMuxError::ApiCall(d) => d.is_retryable,
            _ => false,
        }
    }

    /// Returns the retry-after delay hint (in milliseconds) carried by this
    /// error's response headers, if any.
    ///
    /// Mirrors the header-consulting behaviour of
    /// `retryWithExponentialBackoffRespectingRetryHeaders` in the TS SDK.
    #[must_use]
    pub fn retry_after_hint(&self) -> Option<i64> {
        let AiMuxError::ApiCall(detail) = self else {
            return None;
        };
        let headers = detail.response_headers.as_ref()?;
        retry_after_ms_from_headers(headers, SystemTime::now())
            .and_then(|ms| i64::try_from(ms).ok())
    }

    /// Returns the HTTP status code carried by this error, if any.
    ///
    /// This is the status the selected response handler filled when a
    /// response actually arrived, or, for an error frame inside a stream, the
    /// status the provider maps that frame to as upstream does (OpenAI's
    /// `createOpenAIProviderStreamError` infers it from `code`/`type`). An
    /// error built without an HTTP exchange (a missing API key, a bare
    /// constructor, a transport failure) reports `None`: no status was seen,
    /// so none is invented.
    ///
    /// `TokenExpired` reports `Some(401)` by definition, not by fabrication:
    /// its only producer is the codex subscription channel's observed-401
    /// mapping (RFC-0018), which trades the carried fields for the "refresh
    /// helps" bit — the status is part of the variant's contract.
    #[must_use]
    pub fn status_code(&self) -> Option<u16> {
        match self {
            AiMuxError::TokenExpired(_) => Some(401),
            AiMuxError::ApiCall(d) => d.status_code,
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_such_provider(name: &str, available: &[&str]) -> String {
        AiMuxError::NoSuchProvider {
            provider_id: name.into(),
            model_id: name.into(),
            model_type: String::new(),
            available_providers: available.iter().map(|n| (*n).to_string()).collect(),
        }
        .to_string()
    }

    /// A misspelled provider name gets the close built-in names first; the
    /// full list still follows.
    #[test]
    fn no_such_provider_suggests_close_names() {
        let available = ["anthropic", "google_vertex", "groq", "openai", "openrouter"];
        assert_eq!(
            no_such_provider("opeanai", &available),
            "No such provider: opeanai. Did you mean 'openai'? \
             (available providers: anthropic,google_vertex,groq,openai,openrouter)"
        );
        // An adjacent swap counts once.
        assert!(no_such_provider("gorq", &available).contains("Did you mean 'groq'?"));
        // Case and `-` versus `_` do not matter.
        assert!(
            no_such_provider("Google-Vertex", &available).contains("Did you mean 'google_vertex'?")
        );
        assert!(no_such_provider("antropic", &available).contains("Did you mean 'anthropic'?"));
    }

    /// No suggestion when nothing is close or there is nothing to suggest.
    #[test]
    fn no_such_provider_without_a_close_name_keeps_the_plain_message() {
        assert_eq!(
            no_such_provider("acme", &["anthropic", "openai"]),
            "No such provider: acme (available providers: anthropic,openai)"
        );
        assert_eq!(
            no_such_provider("p", &[]),
            "No such provider: p (available providers: )"
        );
    }

    /// At most three names, nearest first, then by name.
    #[test]
    fn no_such_provider_suggests_at_most_three_names() {
        let candidates = ["abcdefxy", "abcdefgz", "abcdefgy", "abcdefgx"];
        assert!(
            no_such_provider("abcdefgh", &candidates)
                .contains("Did you mean 'abcdefgx', 'abcdefgy', 'abcdefgz'?")
        );
        assert!(
            no_such_provider("abcdefgh", &["abcdefxy", "abcdefgz"])
                .contains("Did you mean 'abcdefgz', 'abcdefxy'?")
        );
    }

    fn api_error(message: &str) -> ApiCallError {
        ApiCallError::new(message, "https://example.test", serde_json::json!({}))
    }

    /// A bare constructor saw no HTTP exchange, so it reports no status —
    /// nothing is invented. Response-derived errors get theirs from the field
    /// (filled by the HTTP layer).
    #[test]
    fn bare_constructors_report_no_status() {
        assert_eq!(
            AiMuxError::ApiCall(Box::new(api_error("no api key"))).status_code(),
            None
        );
        assert_eq!(
            AiMuxError::ApiCall(Box::new(ApiCallError {
                is_retryable: true,
                ..api_error("reset")
            }))
            .status_code(),
            None
        );
        assert_eq!(
            AiMuxError::ApiCall(Box::new(api_error("boom"))).status_code(),
            None
        );
    }

    /// `TokenExpired` is definitionally an observed 401 (RFC-0018); the
    /// status is part of the variant's contract, not fabrication.
    #[test]
    fn token_expired_is_a_401_by_contract() {
        assert_eq!(
            AiMuxError::TokenExpired("expired".into()).status_code(),
            Some(401)
        );
        assert!(!AiMuxError::TokenExpired("expired".into()).is_retryable());
    }

    /// H1: the status comes from the field, never from parsing the message.
    #[test]
    fn status_code_reads_the_field_not_the_message() {
        assert_eq!(
            AiMuxError::ApiCall(Box::new(ApiCallError {
                status_code: Some(403),
                ..api_error("forbidden")
            }))
            .status_code(),
            Some(403)
        );
        // A message that merely *looks* like an HTTP prefix carries no status.
        assert_eq!(
            AiMuxError::ApiCall(Box::new(api_error("HTTP 403: forbidden"))).status_code(),
            None
        );
        assert_eq!(
            AiMuxError::Timeout("HTTP 408: total timeout after 1000ms".into()).status_code(),
            None
        );
        assert_eq!(AiMuxError::json_parse("HTTP 400").status_code(), None);
        assert_eq!(AiMuxError::other("HTTP 400").status_code(), None);
    }

    /// A timeout is a spent caller-side time budget, not a transient fault —
    /// the AI SDK files it in the abort family and does not retry it.
    /// Transport failures (no response) are retryable via the stored field.
    #[test]
    fn timeout_is_not_retryable_transport_is() {
        assert!(!AiMuxError::Timeout("total timeout".into()).is_retryable());
        assert!(
            AiMuxError::ApiCall(Box::new(ApiCallError {
                is_retryable: true,
                ..api_error("connection reset")
            }))
            .is_retryable()
        );
        assert!(!AiMuxError::json_parse("parse").is_retryable());
    }

    #[test]
    fn empty_retry_history_round_trips_without_panicking() {
        let json = r#"{"reason":"maxRetriesExceeded","errors":[]}"#;
        let error: RetryError = serde_json::from_str(json).unwrap();

        assert!(error.errors.is_empty());
        assert_eq!(error.to_string(), "Retry failed without recorded errors");
        assert_eq!(
            error.last_error().to_string(),
            "Retry failed without recorded errors"
        );
        assert_eq!(serde_json::to_string(&error).unwrap(), json);
    }

    fn hint_from(name: &str, value: &str) -> Option<i64> {
        AiMuxError::ApiCall(Box::new(ApiCallError {
            response_headers: Some(HashMap::from([(name.into(), value.into())])),
            ..api_error("rate limited")
        }))
        .retry_after_hint()
    }

    /// `str::parse::<f64>` accepts `inf`, `Infinity` and `NaN`; none of them
    /// is a delay, so the hint is absent instead of `i64::MAX`.
    #[test]
    fn non_finite_retry_after_is_no_hint() {
        for value in ["Infinity", "inf", "-Infinity", "NaN"] {
            assert_eq!(
                hint_from("retry-after-ms", value),
                None,
                "retry-after-ms: {value}"
            );
            assert_eq!(
                hint_from("retry-after", value),
                None,
                "retry-after: {value}"
            );
        }
        // Finite seconds that overflow once converted to milliseconds.
        assert_eq!(hint_from("retry-after", "1e308"), None);
        // Finite values keep working.
        assert_eq!(hint_from("retry-after-ms", "250"), Some(250));
        assert_eq!(hint_from("retry-after", "1.5"), Some(1_500));
    }

    /// A 429 is retryable and carries its hint wherever it lives — the
    /// classification is the `status_code` field, not a variant.
    #[test]
    fn rate_limit_is_read_from_the_field() {
        let err = AiMuxError::ApiCall(Box::new(ApiCallError {
            status_code: Some(429),
            response_headers: Some(HashMap::from([("retry-after-ms".into(), "7".into())])),
            is_retryable: true,
            ..api_error("quota exceeded")
        }));
        assert!(err.is_retryable());
        assert_eq!(err.retry_after_hint(), Some(7));
        // A bare ApiCall error (no stored verdict) is not retryable.
        assert!(!AiMuxError::ApiCall(Box::new(api_error("boom"))).is_retryable());

        let json = serde_json::to_string(&err).unwrap();
        let back: AiMuxError = serde_json::from_str(&json).unwrap();
        assert_eq!(back.retry_after_hint(), Some(7));
        assert_eq!(back.status_code(), Some(429));
    }

    fn chain(error: &(dyn std::error::Error + 'static)) -> Vec<String> {
        std::iter::successors(Some(error), |error| error.source())
            .map(ToString::to_string)
            .collect()
    }

    /// A transport chain survives as `cause`: `source()` walks it in Rust and
    /// the wire form carries the same chain to every binding.
    #[test]
    fn api_call_cause_is_its_source_chain_and_round_trips() {
        let os = std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "Connection refused");
        let transport = SourceError::with_cause("tcp connect error", &os);
        let err = AiMuxError::ApiCall(Box::new(
            api_error("Cannot connect to API: tcp connect error").caused_by(transport),
        ));
        assert_eq!(
            chain(&err),
            [
                "API call error: Cannot connect to API: tcp connect error",
                "Cannot connect to API: tcp connect error",
                "tcp connect error",
                "Connection refused",
            ]
        );
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(
            json["cause"],
            serde_json::json!({
                "message": "tcp connect error",
                "cause": { "message": "Connection refused" },
            })
        );
        let back: AiMuxError = serde_json::from_value(json).unwrap();
        assert_eq!(chain(&back), chain(&err));
    }

    #[test]
    fn no_cause_stays_off_the_wire() {
        let json = serde_json::to_value(AiMuxError::invalid_argument("bad")).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "name": "AI_InvalidArgumentError", "message": "bad" })
        );
        let back: AiMuxError = serde_json::from_value(json).unwrap();
        assert!(std::error::Error::source(&back).is_none());
    }

    /// An aimux cause keeps its variant; anything else becomes a `SourceError`.
    #[test]
    fn nested_aimux_cause_keeps_its_name() {
        let inner = AiMuxError::json_parse("Unexpected end of JSON input");
        let err = AiMuxError::InvalidResponseData {
            message: "bad body".into(),
            cause: Some(ErrorCause::source(&inner)),
        };
        assert!(matches!(
            &err,
            AiMuxError::InvalidResponseData { cause: Some(ErrorCause::AiMux(cause)), .. }
                if matches!(cause.as_ref(), AiMuxError::JsonParse { .. })
        ));
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["cause"]["name"], "AI_JSONParseError");
        let back: AiMuxError = serde_json::from_value(json).unwrap();
        assert_eq!(chain(&back), chain(&err));
    }

    #[test]
    fn serde_error_is_its_own_cause() {
        let parse = serde_json::from_str::<serde_json::Value>("{").unwrap_err();
        let err = AiMuxError::from(parse);
        let AiMuxError::JsonParse { message, cause } = &err else {
            panic!("{err:?}")
        };
        assert_eq!(
            cause.as_ref().map(ToString::to_string).as_ref(),
            Some(message)
        );
    }

    /// Like the AI SDK's `RetryError.lastError`.
    #[test]
    fn retry_source_is_the_last_attempt() {
        let err = AiMuxError::Retry(RetryError {
            reason: RetryErrorReason::MaxRetriesExceeded,
            errors: vec![AiMuxError::other("first"), AiMuxError::other("second")],
        });
        let source = std::error::Error::source(&err).unwrap();
        assert_eq!(source.to_string(), "second");
        assert_eq!(
            err.to_string(),
            "Failed after 2 attempts. Last error: second"
        );
    }
}
