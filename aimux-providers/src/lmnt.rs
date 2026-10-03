//! LMNT speech (TTS) provider.
//!
//! Aligned with Vercel AI SDK `createLMNT`
//! (`reference/ai/packages/lmnt/src/lmnt-provider.ts`) and `LMNTSpeechModel`
//! (`reference/ai/packages/lmnt/src/lmnt-speech-model.ts`).
//!
//! Endpoint: `POST https://api.lmnt.com/v1/ai/speech/bytes`
//!
//! Authentication: `x-api-key` header.
//!
//! The LMNT TTS API accepts `model`, `text`, `voice`, `response_format`,
//! `speed`, and `language` in the request body, and returns raw binary audio.
//! Provider options (`lmnt` key) support `conversational`, `length`, `seed`,
//! `speed`, `temperature`, `topP`, `sampleRate`, and `format`.
//!
//! [`create_lmnt`] takes [`LMNTProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`LMNTProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `LMNT_API_KEY`.
//! [`lmnt()`] is the default instance; it reads nothing and cannot fail.

use std::sync::OnceLock;

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::shared::{SharedProviderOptions, Warning};
use aimux_core::speech_model::{
    AudioData, SpeechCallOptions, SpeechModel, SpeechRequest, SpeechResponse, SpeechResult,
};
use aimux_provider_utils::{FetchFunction, HeaderMapOpt, HeadersFn, Resolvable, validate_base_url};

use crate::shared::{AuthScheme, Credential, EndpointConfig, credential_headers};

fn lmnt_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    aimux_provider_utils::create_json_error_response_handler(|data| {
        let error = data.get("error");
        aimux_provider_utils::ProviderErrorParts {
            message: error
                .and_then(|value| value.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("LMNT request failed")
                .to_string(),
            provider_code: error.and_then(|value| value.get("code")).and_then(
                |value| match value {
                    Value::String(s) => Some(s.clone()),
                    Value::Number(n) => Some(n.to_string()),
                    _ => None,
                },
            ),
        }
    })
}

// ── Config ───────────────────────────────────────────────────────────────────

pub(crate) mod options;

const DEFAULT_BASE_URL: &str = "https://api.lmnt.com";
const API_KEY_ENV_VAR: &str = "LMNT_API_KEY";
const DEFAULT_NAME: &str = "lmnt";

/// Settings of [`create_lmnt`].
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except `base_url` and `name`; `api_key` and `headers` are
/// evaluated on every request.
#[derive(Clone, Default)]
pub struct LMNTProviderSettings {
    /// Base URL for the API calls. Default `https://api.lmnt.com`; a
    /// trailing slash is removed.
    pub base_url: Option<String>,
    /// The API key. `None` loads `LMNT_API_KEY` when a request is made and
    /// fails that request with `AiMuxError::LoadApiKey` if it is unset. An
    /// explicit value is used as given, `""` included: it never falls back to
    /// the environment. A [`Resolvable::Future`] is awaited once, an
    /// [`Resolvable::AsyncFn`] on every request.
    pub api_key: Option<Resolvable<String>>,
    /// Extra headers on every request. A `None` value removes the header,
    /// including the credential. Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The provider name, the prefix of the `provider()` strings (`"{name}.speech"`).
    /// Default `"lmnt"`. The providerOptions key stays `lmnt`.
    pub name: Option<String>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
}

impl std::fmt::Debug for LMNTProviderSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LMNTProviderSettings")
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key)
            .field(
                "headers",
                &self.headers.as_ref().map(std::collections::HashMap::len),
            )
            .field("name", &self.name)
            .field("fetch", &self.fetch.is_some())
            .finish()
    }
}

/// Create a LMNT provider.
///
/// # Errors
///
/// Returns `AiMuxError::InvalidArgument` when `base_url` is not an `http(s)`
/// URL with a host. That is the only way this fails: the key is loaded per
/// request, not here.
pub fn create_lmnt(settings: LMNTProviderSettings) -> Result<LMNTProvider, AiMuxError> {
    let base_url = match settings.base_url.as_deref() {
        Some(url) => validate_base_url(url)?,
        None => DEFAULT_BASE_URL.to_string(),
    };
    Ok(LMNTProvider {
        name: settings.name.unwrap_or_else(|| DEFAULT_NAME.to_string()),
        base_url,
        headers: credential_headers(
            Credential::explicit_or_env(settings.api_key, API_KEY_ENV_VAR, "LMNT"),
            AuthScheme::Header("x-api-key"),
            Vec::new(),
            settings.headers,
        ),
        fetch: settings.fetch,
    })
}

/// The default provider: `create_lmnt` with default settings, created on
/// first use. Creating it reads nothing from the environment and cannot fail;
/// a missing key surfaces from the first request instead.
pub fn lmnt() -> &'static LMNTProvider {
    static DEFAULT: OnceLock<LMNTProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_lmnt(LMNTProviderSettings::default())
            .expect("default LMNT settings are always valid")
    })
}

/// A LMNT provider. Cheap to clone the models out of; it holds no HTTP
/// client.
pub struct LMNTProvider {
    name: String,
    base_url: String,
    headers: HeadersFn,
    fetch: Option<FetchFunction>,
}

impl LMNTProvider {
    fn model_config(&self, method: &str) -> EndpointConfig {
        EndpointConfig::fixed(
            format!("{}.{method}", self.name),
            self.base_url.clone(),
            self.headers.clone(),
            self.fetch.clone(),
            None,
        )
    }

    /// A speech (TTS) model (e.g. `"aurora"`); `provider()` is `"{name}.speech"`.
    #[must_use]
    pub fn speech(&self, model_id: &str) -> LMNTSpeechModel {
        LMNTSpeechModel::from_config(model_id.to_string(), self.model_config("speech"))
    }
}

crate::impl_single_modality_provider!(LMNTProvider, speech_model, |p, id| p.speech(id));

// ── Speech model ─────────────────────────────────────────────────────────────

/// The default voice ID used when no voice is provided.
const DEFAULT_VOICE_ID: &str = "ava";

/// The output formats accepted by the LMNT TTS API.
const SUPPORTED_OUTPUT_FORMATS: &[&str] = &["mp3", "aac", "mulaw", "raw", "wav"];

/// An LMNT speech (TTS) model.
pub struct LMNTSpeechModel {
    model_id: String,
    config: EndpointConfig,
}

impl LMNTSpeechModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }
}

#[async_trait]
impl SpeechModel for LMNTSpeechModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_generate(&self, options: &SpeechCallOptions) -> Result<SpeechResult, AiMuxError> {
        let (body, warnings) = build_request(options, &self.model_id)?;

        let exchange = self.config.exchange(options.headers.as_ref()).await?;

        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(exchange.url("/v1/ai/speech/bytes"), options),
            Value::Object(body.clone()),
            aimux_provider_utils::create_binary_response_handler(),
            lmnt_failed_response_handler(),
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

/// Build the LMNT TTS request body and collect warnings.
///
/// Mirrors the TS `LMNTSpeechModel.getArgs`:
/// - `voice` defaults to `"ava"`.
/// - `response_format` defaults to `"mp3"`; unsupported formats emit a warning.
/// - `speed` and `language` are forwarded when present.
/// - Provider options (`lmnt` key) override the corresponding fields:
///   `conversational`, `length`, `seed`, `speed`, `temperature`, `topP`
///   (mapped to `top_p`), `sampleRate` (mapped to `sample_rate`), and `format`.
fn build_request(
    options: &SpeechCallOptions,
    model_id: &str,
) -> Result<(Map<String, Value>, Vec<Warning>), AiMuxError> {
    let mut warnings = Vec::new();

    let voice = options.voice.as_deref().unwrap_or(DEFAULT_VOICE_ID);
    let output_format = options.output_format.as_deref().unwrap_or("mp3");

    let mut body = Map::new();
    body.insert("model".to_string(), json!(model_id));
    body.insert("text".to_string(), json!(options.text));
    body.insert("voice".to_string(), json!(voice));
    body.insert("response_format".to_string(), json!("mp3"));
    if let Some(speed) = options.speed {
        body.insert("speed".to_string(), json!(speed));
    }

    if SUPPORTED_OUTPUT_FORMATS.contains(&output_format) {
        body.insert("response_format".to_string(), json!(output_format));
    } else {
        warnings.push(Warning::Unsupported {
            feature: "outputFormat".to_string(),
            details: Some(format!(
                "Unsupported output format: {output_format}. Using mp3 instead."
            )),
        });
    }

    // Add provider-specific options.
    let lmnt_options = parse_lmnt_provider_options(options.provider_options.as_ref());
    if let Some(ref opts) = lmnt_options {
        if let Some(conversational) = opts.conversational {
            body.insert("conversational".to_string(), json!(conversational));
        }
        if let Some(length) = opts.length {
            body.insert("length".to_string(), json!(length));
        }
        if let Some(seed) = opts.seed {
            body.insert("seed".to_string(), json!(seed));
        }
        if let Some(speed) = opts.speed {
            body.insert("speed".to_string(), json!(speed));
        }
        if let Some(temperature) = opts.temperature {
            body.insert("temperature".to_string(), json!(temperature));
        }
        if let Some(top_p) = opts.top_p {
            body.insert("top_p".to_string(), json!(top_p));
        }
        if let Some(sample_rate) = opts.sample_rate {
            body.insert("sample_rate".to_string(), json!(sample_rate));
        }
        // The TS test passes `providerOptions.lmnt.format` but the schema also
        // has a `format` field. If present, override `response_format`.
        if let Some(ref format) = opts.format {
            body.insert("response_format".to_string(), json!(format));
        }
    }

    if let Some(ref language) = options.language {
        body.insert("language".to_string(), json!(language));
    }

    Ok((body, warnings))
}

// ── Provider options parsing ─────────────────────────────────────────────────

/// Parsed `lmnt` speech provider options.
#[derive(Debug, Default)]
struct LMNTSpeechProviderOptions {
    conversational: Option<bool>,
    length: Option<f64>,
    seed: Option<u64>,
    speed: Option<f64>,
    temperature: Option<f64>,
    top_p: Option<f64>,
    sample_rate: Option<u32>,
    format: Option<String>,
}

/// Extract LMNT-specific speech options from the shared provider options.
fn parse_lmnt_provider_options(
    options: Option<&SharedProviderOptions>,
) -> Option<LMNTSpeechProviderOptions> {
    let provider_opts = options::lmnt_options(options)?;
    let opts = provider_opts.as_object()?;

    Some(LMNTSpeechProviderOptions {
        conversational: opts
            .get("conversational")
            .and_then(serde_json::Value::as_bool),
        length: opts.get("length").and_then(serde_json::Value::as_f64),
        seed: opts.get("seed").and_then(serde_json::Value::as_u64),
        speed: opts.get("speed").and_then(serde_json::Value::as_f64),
        temperature: opts.get("temperature").and_then(serde_json::Value::as_f64),
        top_p: opts.get("topP").and_then(serde_json::Value::as_f64),
        sample_rate: opts
            .get("sampleRate")
            .and_then(serde_json::Value::as_u64)
            .map(|n| n as u32),
        format: opts
            .get("format")
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string),
    })
}
