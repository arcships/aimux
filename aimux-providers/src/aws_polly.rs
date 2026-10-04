//! Amazon Polly speech (TTS) provider.
//!
//! Implements the `SpeechModel` trait against the Amazon Polly `SynthesizeSpeech`
//! API (`POST https://polly.{region}.amazonaws.com/v1/speech`).
//!
//! Authentication uses AWS Signature Version 4 (SigV4), applied by the shared
//! [`SigV4Fetch`] transport decorator over the final request bytes (the same
//! decorator Bedrock uses). The service name is `"polly"`.
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
//!
//! [`create_aws_polly`] takes [`AwsPollyProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`AwsPollyProvider`]. The credentials are not
//! read there: they are loaded for every request, from the settings or from the
//! AWS environment variables.
//! [`aws_polly()`] is the default instance; it reads nothing and cannot fail.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::shared::{SharedProviderOptions, Warning};
use aimux_core::speech_model::{
    AudioData, SpeechCallOptions, SpeechModel, SpeechRequest, SpeechResponse, SpeechResult,
};
use aimux_provider_utils::HttpBody;
use aimux_provider_utils::{
    AwsCredentials, FetchFunction, HeaderMapOpt, HeadersFn, Resolvable, SigV4Fetch, default_fetch,
    load_optional_setting, load_setting, validate_base_url,
};

use crate::shared::{Endpoint, EndpointConfig};

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
fn aws_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    aimux_provider_utils::create_json_error_response_handler(aws_error_parts)
}

fn aws_error_parts(data: &Value) -> aimux_provider_utils::ProviderErrorParts {
    aimux_provider_utils::ProviderErrorParts {
        message: data
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("Amazon Polly request failed")
            .to_string(),
        provider_code: data
            .get("__type")
            .and_then(Value::as_str)
            .map(str::to_string),
    }
}

// ── Config ───────────────────────────────────────────────────────────────────

pub(crate) mod options;

const DEFAULT_NAME: &str = "amazon-polly";
const REGION_ENV_VAR: &str = "AWS_REGION";
const ACCESS_KEY_ID_ENV_VAR: &str = "AWS_ACCESS_KEY_ID";
const SECRET_ACCESS_KEY_ENV_VAR: &str = "AWS_SECRET_ACCESS_KEY";
const SESSION_TOKEN_ENV_VAR: &str = "AWS_SESSION_TOKEN";

/// Settings of [`create_aws_polly`].
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except `base_url` and `name`; the region, the credentials and
/// `headers` are evaluated on every request.
#[derive(Clone, Default)]
pub struct AwsPollyProviderSettings {
    /// The AWS region. `None` loads `AWS_REGION` when a request is made and
    /// falls back to `us-east-1`.
    pub region: Option<String>,
    /// The AWS access key id of SigV4 signing. `None` loads
    /// `AWS_ACCESS_KEY_ID` when a request is made and fails that request with
    /// `AiMuxError::LoadSetting` if it is unset.
    pub access_key_id: Option<String>,
    /// The AWS secret access key of SigV4 signing. `None` loads
    /// `AWS_SECRET_ACCESS_KEY`.
    pub secret_access_key: Option<String>,
    /// The AWS session token of temporary credentials. When `access_key_id`
    /// and `secret_access_key` are both given only this field is used; when
    /// either comes from the environment the token also falls back to
    /// `AWS_SESSION_TOKEN`.
    pub session_token: Option<String>,
    /// Dynamic AWS credentials. When set they are used instead of
    /// `access_key_id`, `secret_access_key` and `session_token`; it is
    /// resolved on every request, so an [`Resolvable::AsyncFn`] can hand out
    /// rotating STS credentials. Its `region` is used only when neither
    /// `region` nor `AWS_REGION` names one.
    pub credential_provider: Option<Resolvable<AwsCredentials>>,
    /// Base URL for the API calls. Default `https://polly.{region}.amazonaws.com`;
    /// a trailing slash is removed.
    pub base_url: Option<String>,
    /// Extra headers on every request. A `None` value removes the header.
    /// Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The provider name, the prefix of the `provider()` string
    /// (`"{name}.speech"`). Default `"amazon-polly"`. The providerOptions key
    /// is `aws_polly`.
    pub name: Option<String>,
    /// The transport SigV4 signing wraps. `None` uses the process default,
    /// resolved per request.
    pub fetch: Option<FetchFunction>,
}

impl std::fmt::Debug for AwsPollyProviderSettings {
    /// Never prints keys, credentials or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AwsPollyProviderSettings")
            .field("region", &self.region)
            .field("access_key_id", &self.access_key_id.is_some())
            .field("secret_access_key", &self.secret_access_key.is_some())
            .field("session_token", &self.session_token.is_some())
            .field("credential_provider", &self.credential_provider.is_some())
            .field("base_url", &self.base_url)
            .field(
                "headers",
                &self.headers.as_ref().map(std::collections::HashMap::len),
            )
            .field("name", &self.name)
            .field("fetch", &self.fetch.is_some())
            .finish()
    }
}

/// Where the region and the AWS credentials of a request come from.
#[derive(Clone)]
struct Auth {
    region: Option<String>,
    access_key_id: Option<String>,
    secret_access_key: Option<String>,
    session_token: Option<String>,
    credential_provider: Option<Resolvable<AwsCredentials>>,
}

impl Auth {
    /// The region: the setting, `AWS_REGION`, the region of a plain-value
    /// credential provider, then `us-east-1`.
    fn region(&self) -> String {
        if let Some(region) = load_optional_setting(self.region.as_deref(), REGION_ENV_VAR) {
            return region;
        }
        if let Some(Resolvable::Value(credentials)) = &self.credential_provider
            && !credentials.region.is_empty()
        {
            return credentials.region.clone();
        }
        DEFAULT_REGION.to_string()
    }

    /// The credentials of the settings and the environment (no provider).
    fn static_credentials(&self) -> Result<AwsCredentials, AiMuxError> {
        let access_key_id = load_setting(
            self.access_key_id.as_deref(),
            ACCESS_KEY_ID_ENV_VAR,
            "access_key_id",
        )?;
        let secret_access_key = load_setting(
            self.secret_access_key.as_deref(),
            SECRET_ACCESS_KEY_ENV_VAR,
            "secret_access_key",
        )?;
        let session_token = if self.access_key_id.is_some() && self.secret_access_key.is_some() {
            self.session_token.clone()
        } else {
            load_optional_setting(self.session_token.as_deref(), SESSION_TOKEN_ENV_VAR)
                .filter(|token| !token.trim().is_empty())
        };
        Ok(AwsCredentials {
            access_key_id,
            secret_access_key,
            session_token,
            region: self.region(),
        })
    }

    /// The credentials to sign one request with.
    async fn credentials(&self) -> Result<AwsCredentials, AiMuxError> {
        if let Some(provider) = &self.credential_provider {
            let mut credentials = provider.resolve().await?;
            if self.region.is_some() || std::env::var(REGION_ENV_VAR).is_ok() {
                credentials.region = self.region();
            } else if credentials.region.is_empty() {
                credentials.region = DEFAULT_REGION.to_string();
            }
            return Ok(credentials);
        }
        self.static_credentials()
    }

    /// Fail now, with the typed setting error, when signing could not find its
    /// inputs: the transport reports a credential failure as a transport
    /// error, which would hide which setting is missing. A credential provider
    /// is left to the signer.
    fn preflight(&self) -> Result<(), AiMuxError> {
        if self.credential_provider.is_none() {
            self.static_credentials()?;
        }
        Ok(())
    }
}

/// The provider headers: only the caller's, after checking that signing has
/// its credentials (the signature itself is made by the transport).
fn signing_headers(auth: Auth, user: Option<HeaderMapOpt>) -> HeadersFn {
    Resolvable::from_async_fn(move || {
        let auth = auth.clone();
        let user = user.clone();
        async move {
            auth.preflight()?;
            Ok(user.unwrap_or_default())
        }
    })
}

/// Create an Amazon Polly provider.
///
/// # Errors
///
/// Returns `AiMuxError::InvalidArgument` when `base_url` is not an `http(s)`
/// URL with a host. That is the only way this fails: the region and the
/// credentials are loaded per request, not here.
pub fn create_aws_polly(
    settings: AwsPollyProviderSettings,
) -> Result<AwsPollyProvider, AiMuxError> {
    let base_url = settings
        .base_url
        .as_deref()
        .map(validate_base_url)
        .transpose()?;
    let auth = Auth {
        region: settings.region,
        access_key_id: settings.access_key_id,
        secret_access_key: settings.secret_access_key,
        session_token: settings.session_token,
        credential_provider: settings.credential_provider,
    };
    let signing_auth = auth.clone();
    let headers_auth = auth.clone();
    let fetch: FetchFunction = Arc::new(SigV4Fetch::new(
        settings.fetch.unwrap_or_else(default_fetch),
        Resolvable::from_async_fn(move || {
            let auth = signing_auth.clone();
            async move { auth.credentials().await }
        }),
        SERVICE_NAME,
    ));
    Ok(AwsPollyProvider {
        name: settings.name.unwrap_or_else(|| DEFAULT_NAME.to_string()),
        auth,
        base_url,
        // SigV4 signs in the transport; the provider headers carry no credential.
        headers: signing_headers(headers_auth, settings.headers),
        fetch,
    })
}

/// The default provider: `create_aws_polly` with default settings, created on
/// first use. Creating it reads nothing from the environment and cannot fail;
/// missing credentials surface from the first request instead.
pub fn aws_polly() -> &'static AwsPollyProvider {
    static DEFAULT: OnceLock<AwsPollyProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_aws_polly(AwsPollyProviderSettings::default())
            .expect("default Amazon Polly settings are always valid")
    })
}

/// An Amazon Polly provider. Speech only; it holds no HTTP client.
pub struct AwsPollyProvider {
    name: String,
    auth: Auth,
    base_url: Option<String>,
    headers: HeadersFn,
    fetch: FetchFunction,
}

impl AwsPollyProvider {
    fn model_config(&self, method: &str) -> EndpointConfig {
        let auth = self.auth.clone();
        let base_url = self.base_url.clone();
        let headers = self.headers.clone();
        EndpointConfig::dynamic(
            format!("{}.{method}", self.name),
            Arc::new(move || {
                let auth = auth.clone();
                let base_url = base_url.clone();
                let headers = headers.clone();
                Box::pin(async move {
                    Ok(Endpoint {
                        base_url: base_url.unwrap_or_else(|| {
                            format!("https://polly.{}.amazonaws.com", auth.region())
                        }),
                        headers: headers.resolve().await?,
                    })
                })
            }),
            Some(self.fetch.clone()),
        )
    }

    /// A speech (TTS) model (an engine id such as `"neural"`); `provider()` is
    /// `"{name}.speech"`.
    #[must_use]
    pub fn speech(&self, model_id: &str) -> AwsPollySpeechModel {
        AwsPollySpeechModel::from_config(model_id.to_string(), self.model_config("speech"))
    }
}

crate::impl_single_modality_provider!(AwsPollyProvider, speech_model, |p, id| p.speech(id));

// ── Speech model ─────────────────────────────────────────────────────────────

/// An Amazon Polly speech (TTS) model.
pub struct AwsPollySpeechModel {
    model_id: String,
    config: EndpointConfig,
}

impl AwsPollySpeechModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }
}

#[async_trait]
impl SpeechModel for AwsPollySpeechModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_generate(&self, options: &SpeechCallOptions) -> Result<SpeechResult, AiMuxError> {
        let (body, warnings) = build_request(options, &self.model_id);
        let body_str = serde_json::to_string(&Value::Object(body.clone()))
            .map_err(|e| AiMuxError::JsonParse(e.to_string()))?;
        // The request is signed by the transport (`SigV4Fetch`), over the exact
        // bytes sent: user-supplied headers and `Content-Type` are part of the
        // signature.
        let exchange = self.config.exchange(options.headers.as_ref()).await?;

        let resp = aimux_provider_utils::post_to_api(
            exchange.request(exchange.url("/v1/speech"), options),
            HttpBody::Bytes(body_str.into_bytes(), "application/json".to_string()),
            aimux_provider_utils::create_binary_response_handler(),
            aws_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;

        let audio_bytes = resp.value.to_vec();

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
    let opts = options::aws_polly_options(options)?;

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
    fn settings_debug_redacts_credentials() {
        let settings = AwsPollyProviderSettings {
            access_key_id: Some("AKIAEXAMPLE".to_string()),
            secret_access_key: Some("super-secret-key".to_string()),
            session_token: Some("session-token".to_string()),
            region: Some("us-west-2".to_string()),
            ..Default::default()
        };
        let debug = format!("{settings:?}");
        assert!(!debug.contains("AKIAEXAMPLE"));
        assert!(!debug.contains("super-secret-key"));
        assert!(!debug.contains("session-token"));
        // Non-secret fields are still visible.
        assert!(debug.contains("us-west-2"));
    }

    #[test]
    fn extracts_aws_error_fields() {
        let data = serde_json::json!({
            "__type": "UnrecognizedClientException",
            "message": "The security token included in the request is invalid."
        });
        let parts = aws_error_parts(&data);
        assert_eq!(
            parts.provider_code.as_deref(),
            Some("UnrecognizedClientException")
        );
        assert_eq!(
            parts.message,
            "The security token included in the request is invalid."
        );
    }

    #[test]
    fn missing_aws_message_uses_provider_fallback() {
        let parts = aws_error_parts(&serde_json::json!({ "__type": "AccessDeniedException" }));
        assert_eq!(parts.message, "Amazon Polly request failed");
    }
}
