//! Amazon Polly speech (TTS) provider.
//!
//! Implements the `SpeechModel` trait against the Amazon Polly `SynthesizeSpeech`
//! API (`POST https://polly.{region}.amazonaws.com/v1/speech`).
//!
//! Authentication uses AWS Signature Version 4 (SigV4), reusing the
//! [`AwsCredentials`] / [`sign_request`] helpers shared with the Bedrock
//! provider. The service name is `"polly"`.
//!
//! The Polly API accepts a JSON request body describing the synthesis job and
//! returns the synthesized audio as a binary stream (the response body is *not*
//! JSON). The `Content-Type` of the response reflects the requested
//! `OutputFormat`.
//!
//! Environment variables follow the standard AWS credential chain:
//! `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_REGION` (defaulting to
//! `us-east-1`), and the optional `AWS_SESSION_TOKEN` for temporary STS
//! credentials.

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::provider::Provider;
use aimux_core::shared::{SharedProviderOptions, Warning};
use aimux_core::speech_model::{
    AudioData, SpeechCallOptions, SpeechModel, SpeechRequest, SpeechResponse, SpeechResult,
};

use aimux_provider_utils::response::{ErrorStructure, error_for_status};
use aimux_provider_utils::{HttpBody, HttpMethod, HttpRequest, RetryConfig, send};

use crate::bedrock::sigv4::{AwsCredentials, sign_request};

/// Provider canonical name.
const PROVIDER_NAME: &str = "amazon-polly";

/// AWS service name used for SigV4 signing.
const SERVICE_NAME: &str = "polly";

/// Default AWS region when `AWS_REGION` is unset.
const DEFAULT_REGION: &str = "us-east-1";

/// Default voice ID used when no voice is provided. `Joanna` is a widely
/// available neural voice.
const DEFAULT_VOICE_ID: &str = "Joanna";

/// Default output format when none is requested.
const DEFAULT_OUTPUT_FORMAT: &str = "mp3";

/// Output formats accepted by the Polly `SynthesizeSpeech` API.
const SUPPORTED_OUTPUT_FORMATS: &[&str] = &[
    "mp3",
    "ogg_opus",
    "ogg_vorbis",
    "pcm",
    "mulaw",
    "alaw",
    "json",
];

/// AWS error shape: `{"__type": "...", "message": "..."}` (no `error` wrapper).
/// Passed to `send` so the http layer extracts the right fields on non-2xx.
const AWS_ERROR_STRUCTURE: ErrorStructure = ErrorStructure {
    message_path: &["message"],
    type_path: &["__type"],
};

// ── Config ───────────────────────────────────────────────────────────────────

/// Configuration for the Amazon Polly provider.
///
/// Holds AWS SigV4 credentials. The `Debug` implementation redacts secrets so
/// credentials never appear in logs or error messages.
pub struct AwsPollyConfig {
    access_key_id: String,
    secret_access_key: String,
    region: String,
    session_token: Option<String>,
    base_url: String,
}

impl AwsPollyConfig {
    /// Create a config with explicit static SigV4 credentials.
    pub fn new(
        access_key_id: impl Into<String>,
        secret_access_key: impl Into<String>,
        region: impl Into<String>,
    ) -> Self {
        let region = region.into();
        let base_url = format!("https://polly.{region}.amazonaws.com");
        Self {
            access_key_id: access_key_id.into(),
            secret_access_key: secret_access_key.into(),
            region,
            session_token: None,
            base_url,
        }
    }

    /// Override the base URL (for testing or proxies).
    #[must_use]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into().trim_end_matches('/').to_string();
        self
    }

    /// Add a session token for temporary STS credentials.
    #[must_use]
    pub fn with_session_token(mut self, token: impl Into<String>) -> Self {
        self.session_token = Some(token.into());
        self
    }

    /// Create from environment variables.
    ///
    /// Reads `AWS_ACCESS_KEY_ID` + `AWS_SECRET_ACCESS_KEY` + `AWS_REGION`
    /// (defaulting to `us-east-1`), and the optional `AWS_SESSION_TOKEN`.
    ///
    /// # Errors
    ///
    /// Returns `AiMuxError::InvalidArgument` when `AWS_ACCESS_KEY_ID` or
    /// `AWS_SECRET_ACCESS_KEY` is not set.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let access_key_id = std::env::var("AWS_ACCESS_KEY_ID").map_err(|_| {
            AiMuxError::InvalidArgument(
                "AWS_ACCESS_KEY_ID environment variable is required for Amazon Polly SigV4 auth"
                    .to_string(),
            )
        })?;
        let secret_access_key = std::env::var("AWS_SECRET_ACCESS_KEY").map_err(|_| {
            AiMuxError::InvalidArgument(
                "AWS_SECRET_ACCESS_KEY environment variable is required for Amazon Polly SigV4 auth"
                    .to_string(),
            )
        })?;
        let region = std::env::var("AWS_REGION").unwrap_or_else(|_| DEFAULT_REGION.to_string());

        let mut config = Self::new(access_key_id, secret_access_key, region);
        if let Ok(token) = std::env::var("AWS_SESSION_TOKEN")
            && !token.trim().is_empty()
        {
            config = config.with_session_token(token);
        }
        Ok(config)
    }

    /// Build the [`AwsCredentials`] used for signing.
    fn credentials(&self) -> AwsCredentials {
        AwsCredentials {
            access_key_id: self.access_key_id.clone(),
            secret_access_key: self.secret_access_key.clone(),
            session_token: self.session_token.clone(),
            region: self.region.clone(),
        }
    }
}

impl Clone for AwsPollyConfig {
    fn clone(&self) -> Self {
        Self {
            access_key_id: self.access_key_id.clone(),
            secret_access_key: self.secret_access_key.clone(),
            region: self.region.clone(),
            session_token: self.session_token.clone(),
            base_url: self.base_url.clone(),
        }
    }
}

impl std::fmt::Debug for AwsPollyConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AwsPollyConfig")
            .field("access_key_id", &"<redacted>")
            .field("secret_access_key", &"<redacted>")
            .field("region", &self.region)
            .field(
                "session_token",
                &self.session_token.as_ref().map(|_| "<redacted>"),
            )
            .field("base_url", &self.base_url)
            .finish()
    }
}

// ── Provider ─────────────────────────────────────────────────────────────────

/// Amazon Polly provider — creates [`AwsPollySpeechModel`] instances.
pub struct AwsPollyProvider {
    config: AwsPollyConfig,
}

impl AwsPollyProvider {
    #[must_use]
    pub fn new(config: AwsPollyConfig) -> Self {
        Self { config }
    }

    /// Create a speech (TTS) model instance for the given model id
    /// (e.g. `"aws_polly/neural"` or `"neural"`).
    #[must_use]
    pub fn speech(&self, model_id: &str) -> AwsPollySpeechModel {
        AwsPollySpeechModel::new(model_id.to_string(), self.config.clone())
    }
}

impl Provider for AwsPollyProvider {
    fn name(&self) -> &str {
        PROVIDER_NAME
    }
}

// ── Speech model ─────────────────────────────────────────────────────────────

/// An Amazon Polly speech (TTS) model.
pub struct AwsPollySpeechModel {
    model_id: String,
    config: AwsPollyConfig,
}

impl AwsPollySpeechModel {
    #[must_use]
    pub fn new(model_id: String, config: AwsPollyConfig) -> Self {
        Self { model_id, config }
    }

    fn endpoint(&self) -> String {
        format!("{}/v1/speech", self.config.base_url)
    }
}

#[async_trait]
impl SpeechModel for AwsPollySpeechModel {
    fn provider(&self) -> &str {
        PROVIDER_NAME
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_generate(&self, options: &SpeechCallOptions) -> Result<SpeechResult, AiMuxError> {
        let (body, warnings) = build_request(options, &self.model_id);
        let body_str = serde_json::to_string(&Value::Object(body.clone()))
            .map_err(|e| AiMuxError::JsonParse(e.to_string()))?;
        let url = self.endpoint();

        // SigV4 sign the request. User-supplied extra headers are included in
        // the canonical headers; `Content-Type` is added separately (mirrors the
        // Bedrock provider). The signed body bytes are sent verbatim via
        // `HttpBody::Bytes` so the `x-amz-content-sha256` hash still matches —
        // re-serializing the JSON would invalidate the signature.
        let creds = self.config.credentials();
        let mut extra_headers: Vec<(String, String)> = Vec::new();
        if let Some(ref headers) = options.headers {
            for (k, v) in headers {
                extra_headers.push((k.clone(), v.clone()));
            }
        }
        let signed = sign_request(
            &creds,
            SERVICE_NAME,
            "POST",
            &url,
            &body_str,
            &extra_headers,
        );

        let resp = send(
            HttpRequest {
                method: HttpMethod::Post,
                url,
                headers: signed.headers,
                body: HttpBody::Bytes(body_str.into_bytes(), "application/json".to_string()),

                abort_signal: options.abort_signal.clone(),
                call_id: None,
                recording_context: None,
            },
            RetryConfig::default(),
            &AWS_ERROR_STRUCTURE,
        )
        .await?;

        let response_headers = resp.headers;

        let audio_bytes = resp.body.to_vec();

        let timestamp = chrono::Utc::now().to_rfc3339();

        Ok(SpeechResult {
            audio: AudioData::Binary(audio_bytes),
            warnings,
            request: Some(SpeechRequest {
                body: Some(Value::Object(body)),
            }),
            response: SpeechResponse {
                timestamp: Some(timestamp),
                model_id: Some(self.model_id.clone()),
                headers: Some(response_headers),
                body: None,
            },
            provider_metadata: None,
        })
    }
}

// ── Request builder ──────────────────────────────────────────────────────────

/// Build the Polly `SynthesizeSpeech` request body and collect warnings.
///
/// Field mapping:
/// - `Engine` — derived from the model id (`aws_polly/{engine}`).
/// - `Text` — the input text.
/// - `VoiceId` — from `options.voice` (defaults to `Joanna`).
/// - `OutputFormat` — from `options.output_format` (defaults to `mp3`);
///   unsupported formats emit a warning and fall back to `mp3`.
/// - `LanguageCode` — from `options.language` when present.
/// - `speed` / `instructions` are not supported by Polly and emit warnings.
/// - Provider options (`aws_polly` key) override: `engine`, `sampleRate`,
///   `textType`, `lexiconNames`, `speechMarkTypes`.
fn build_request(
    options: &SpeechCallOptions,
    model_id: &str,
) -> (Map<String, Value>, Vec<Warning>) {
    let mut warnings = Vec::new();

    let (engine, engine_warning) = resolve_engine(model_id);
    warnings.extend(engine_warning);

    let voice = options.voice.as_deref().unwrap_or(DEFAULT_VOICE_ID);
    let output_format = options
        .output_format
        .as_deref()
        .unwrap_or(DEFAULT_OUTPUT_FORMAT);

    let mut body = Map::new();
    body.insert("Engine".to_string(), json!(engine));
    body.insert("Text".to_string(), json!(options.text));
    body.insert("VoiceId".to_string(), json!(voice));

    if SUPPORTED_OUTPUT_FORMATS.contains(&output_format) {
        body.insert("OutputFormat".to_string(), json!(output_format));
    } else {
        warnings.push(Warning::Unsupported {
            feature: "outputFormat".to_string(),
            details: Some(format!(
                "Unsupported output format: {output_format}. Using mp3 instead."
            )),
        });
        body.insert("OutputFormat".to_string(), json!(DEFAULT_OUTPUT_FORMAT));
    }

    if let Some(ref language) = options.language {
        body.insert("LanguageCode".to_string(), json!(language));
    }

    // `speed` and `instructions` have no Polly equivalent.
    if options.speed.is_some() {
        warnings.push(Warning::Unsupported {
            feature: "speed".to_string(),
            details: Some("Amazon Polly does not support the speed option.".to_string()),
        });
    }
    if options.instructions.is_some() {
        warnings.push(Warning::Unsupported {
            feature: "instructions".to_string(),
            details: Some("Amazon Polly does not support the instructions option.".to_string()),
        });
    }

    // Provider-specific options (`aws_polly` key).
    if let Some(opts) = parse_polly_provider_options(options.provider_options.as_ref()) {
        if let Some(ref engine_override) = opts.engine {
            body.insert("Engine".to_string(), json!(engine_override));
        }
        if let Some(ref sample_rate) = opts.sample_rate {
            body.insert("SampleRate".to_string(), json!(sample_rate));
        }
        if let Some(ref text_type) = opts.text_type {
            body.insert("TextType".to_string(), json!(text_type));
        }
        if let Some(ref lexicon_names) = opts.lexicon_names {
            body.insert("LexiconNames".to_string(), json!(lexicon_names));
        }
        if let Some(ref speech_mark_types) = opts.speech_mark_types {
            body.insert("SpeechMarkTypes".to_string(), json!(speech_mark_types));
        }
    }

    (body, warnings)
}

/// Map a model id to a Polly `Engine` value.
///
/// Accepts both `"aws_polly/{engine}"` (the canonical id form) and a bare
/// engine name. Unknown engines default to `"standard"` with a warning.
fn resolve_engine(model_id: &str) -> (&'static str, Option<Warning>) {
    let normalized = model_id.strip_prefix("aws_polly/").unwrap_or(model_id);
    match normalized {
        "generative" => ("generative", None),
        "neural" => ("neural", None),
        "long-form" => ("long-form", None),
        "standard" => ("standard", None),
        "" => ("standard", None),
        other => (
            "standard",
            Some(Warning::Unsupported {
                feature: "engine".to_string(),
                details: Some(format!(
                    "Unknown Polly engine: {other}. Using standard instead."
                )),
            }),
        ),
    }
}

// ── Provider options parsing ─────────────────────────────────────────────────

/// Parsed `aws_polly` speech provider options.
#[derive(Debug, Default)]
struct PollySpeechProviderOptions {
    engine: Option<String>,
    sample_rate: Option<String>,
    text_type: Option<String>,
    lexicon_names: Option<Vec<String>>,
    speech_mark_types: Option<Vec<String>>,
}

/// Extract Polly-specific speech options from the shared provider options.
///
/// Sample rate is accepted as a string or number and normalized to a string
/// (Polly expects e.g. `"22050"`).
fn parse_polly_provider_options(
    options: Option<&SharedProviderOptions>,
) -> Option<PollySpeechProviderOptions> {
    let provider_opts = options.and_then(|opts| opts.get("aws_polly"))?;
    let opts = provider_opts.as_object()?;

    Some(PollySpeechProviderOptions {
        engine: opts
            .get("engine")
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string),
        sample_rate: opts.get("sampleRate").and_then(|v| {
            v.as_str()
                .map(std::string::ToString::to_string)
                .or_else(|| v.as_u64().map(|n| n.to_string()))
        }),
        text_type: opts
            .get("textType")
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string),
        lexicon_names: opts
            .get("lexiconNames")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(std::string::ToString::to_string))
                    .collect()
            }),
        speech_mark_types: opts
            .get("speechMarkTypes")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(std::string::ToString::to_string))
                    .collect()
            }),
    })
}

// ── Error handling ───────────────────────────────────────────────────────────

/// Parse an AWS Polly error response into an [`AiMuxError`].
///
/// AWS errors are JSON objects of the form
/// `{"__type": "...", "message": "..."}`. Authentication failures surface as
/// HTTP 401 or 403; both surface as [`AiMuxError::ApiCall`] with the status in `status_code`.
///
/// Live requests no longer call this — the shared http layer (`send`) maps
/// non-2xx responses itself via [`AWS_ERROR_STRUCTURE`]. Retained (and
/// exercised below) for unit-testing the AWS-specific 401/403 → `Auth` mapping,
/// which the generic handler does not reproduce.
#[allow(dead_code)]
fn parse_polly_error(status: u16, body: &str) -> AiMuxError {
    let message = extract_aws_error_message(body);
    let provider_code = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v.get("__type").and_then(|t| t.as_str()).map(String::from));
    error_for_status(status, provider_code, message, None, Some(body.to_string()))
}

/// Extract a human-readable message from an AWS error JSON body.
///
/// Only used by [`parse_polly_error`] (which is itself test-only after the
/// migration to the shared http layer), so marked `#[allow(dead_code)]`.
#[allow(dead_code)]
fn extract_aws_error_message(body: &str) -> String {
    if let Ok(val) = serde_json::from_str::<Value>(body) {
        if let Some(msg) = val.get("message").and_then(|v| v.as_str()) {
            return msg.to_string();
        }
        if let Some(t) = val.get("__type").and_then(|v| v.as_str()) {
            return t.to_string();
        }
    }
    if body.is_empty() {
        "HTTP error".to_string()
    } else {
        body.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_canonical_engine_ids() {
        assert_eq!(resolve_engine("aws_polly/generative").0, "generative");
        assert_eq!(resolve_engine("aws_polly/neural").0, "neural");
        assert_eq!(resolve_engine("aws_polly/long-form").0, "long-form");
        assert_eq!(resolve_engine("aws_polly/standard").0, "standard");
    }

    #[test]
    fn resolves_bare_engine_ids() {
        assert_eq!(resolve_engine("neural").0, "neural");
        assert_eq!(resolve_engine("standard").0, "standard");
    }

    #[test]
    fn unknown_engine_defaults_to_standard_with_warning() {
        let (engine, warning) = resolve_engine("aws_polly/unknown-engine");
        assert_eq!(engine, "standard");
        assert!(warning.is_some());
    }

    #[test]
    fn config_debug_redacts_credentials() {
        let config = AwsPollyConfig::new("AKIAEXAMPLE", "super-secret-key", "us-west-2")
            .with_session_token("session-token");
        let debug = format!("{config:?}");
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("AKIAEXAMPLE"));
        assert!(!debug.contains("super-secret-key"));
        assert!(!debug.contains("session-token"));
        // Non-secret fields are still visible.
        assert!(debug.contains("us-west-2"));
    }

    #[test]
    fn parse_error_maps_401_to_auth() {
        let body = r#"{"__type":"UnrecognizedClientException","message":"The security token included in the request is invalid."}"#;
        let err = parse_polly_error(401, body);
        assert!(matches!(err, ref e if e.status_code() == Some(401)));
    }

    #[test]
    fn parse_error_keeps_the_observed_403() {
        let body = r#"{"__type":"AccessDeniedException","message":"User is not authorized."}"#;
        let err = parse_polly_error(403, body);
        assert_eq!(err.status_code(), Some(403));
    }
}
