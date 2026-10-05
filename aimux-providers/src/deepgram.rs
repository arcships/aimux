//! Deepgram transcription (STT) provider.
//!
//! Aligned with Vercel AI SDK `createDeepgram` / `DeepgramTranscriptionModel`
//! (`reference/ai/packages/deepgram/src/deepgram-transcription-model.ts`).
//!
//! Endpoint: `POST https://api.deepgram.com/v1/listen?{query_params}`
//!
//! The Deepgram API accepts raw audio bytes in the request body (with the
//! `Content-Type` header set to the audio media type) and query parameters for
//! model configuration. It returns a JSON body with `results.channels[0]`
//! containing the transcript, words, and detected language.
//!
//! [`create_deepgram`] takes [`DeepgramProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`DeepgramProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `DEEPGRAM_API_KEY`.
//! [`deepgram()`] is the default instance; it reads nothing and cannot fail.

use std::sync::OnceLock;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

use aimux_core::error::AiMuxError;
use aimux_core::shared::{SharedProviderOptions, Warning};
use aimux_core::transcription_model::{
    AudioInput, TranscriptionCallOptions, TranscriptionModel, TranscriptionRequest,
    TranscriptionResponse, TranscriptionResult, TranscriptionSegment,
};
use aimux_provider_utils::HttpBody;
use aimux_provider_utils::{FetchFunction, HeaderMapOpt, HeadersFn, Resolvable, validate_base_url};

use crate::shared::{AuthScheme, Credential, EndpointConfig, credential_headers};

/// Deepgram errors: `{"err_code": "...", "err_msg": "...", "request_id": ...}`
/// on most endpoints; some return `{"category": "...", "message": "...",
/// "details": ...}` instead (https://developers.deepgram.com/docs/errors).
fn deepgram_error_parts(data: &Value) -> aimux_provider_utils::ProviderErrorParts {
    let message = data
        .get("err_msg")
        .and_then(Value::as_str)
        .or_else(|| data.get("message").and_then(Value::as_str))
        .or_else(|| {
            data.get("error")
                .and_then(|value| value.get("message"))
                .and_then(Value::as_str)
        })
        .or_else(|| data.get("error").and_then(Value::as_str))
        .unwrap_or("Deepgram request failed")
        .to_string();
    aimux_provider_utils::ProviderErrorParts {
        message,
        provider_code: data
            .get("err_code")
            .or_else(|| data.get("category"))
            .and_then(|value| match value {
                Value::String(s) => Some(s.clone()),
                Value::Number(n) => Some(n.to_string()),
                _ => None,
            }),
    }
}

fn deepgram_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    aimux_provider_utils::create_json_error_response_handler(deepgram_error_parts)
}

// ── Config ──────────────────────────────────────────────────────────────────

pub(crate) mod options;

const DEFAULT_BASE_URL: &str = "https://api.deepgram.com";
const API_KEY_ENV_VAR: &str = "DEEPGRAM_API_KEY";
const DEFAULT_NAME: &str = "deepgram";

/// Settings of [`create_deepgram`].
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except `base_url` and `name`; `api_key` and `headers` are
/// evaluated on every request.
#[derive(Clone, Default)]
pub struct DeepgramProviderSettings {
    /// Base URL for the API calls. Default `https://api.deepgram.com`; a
    /// trailing slash is removed.
    pub base_url: Option<String>,
    /// The API key. `None` loads `DEEPGRAM_API_KEY` when a request is made and
    /// fails that request with `AiMuxError::LoadApiKey` if it is unset. An
    /// explicit value is used as given, `""` included: it never falls back to
    /// the environment. A [`Resolvable::Future`] is awaited once, an
    /// [`Resolvable::AsyncFn`] on every request.
    pub api_key: Option<Resolvable<String>>,
    /// Extra headers on every request. A `None` value removes the header,
    /// including the credential. Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The provider name, the prefix of the `provider()` strings (`"{name}.transcription"`).
    /// Default `"deepgram"`. The providerOptions key stays `deepgram`.
    pub name: Option<String>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
}

impl std::fmt::Debug for DeepgramProviderSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeepgramProviderSettings")
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

/// Create a Deepgram provider.
///
/// # Errors
///
/// Returns `AiMuxError::InvalidArgument` when `base_url` is not an `http(s)`
/// URL with a host. That is the only way this fails: the key is loaded per
/// request, not here.
pub fn create_deepgram(settings: DeepgramProviderSettings) -> Result<DeepgramProvider, AiMuxError> {
    let base_url = match settings.base_url.as_deref() {
        Some(url) => validate_base_url(url)?,
        None => DEFAULT_BASE_URL.to_string(),
    };
    Ok(DeepgramProvider {
        name: settings.name.unwrap_or_else(|| DEFAULT_NAME.to_string()),
        base_url,
        headers: credential_headers(
            Credential::explicit_or_env(settings.api_key, API_KEY_ENV_VAR, "Deepgram"),
            AuthScheme::Scheme("Token"),
            Vec::new(),
            settings.headers,
        ),
        fetch: settings.fetch,
    })
}

/// The default provider: `create_deepgram` with default settings, created on
/// first use. Creating it reads nothing from the environment and cannot fail;
/// a missing key surfaces from the first request instead.
pub fn deepgram() -> &'static DeepgramProvider {
    static DEFAULT: OnceLock<DeepgramProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_deepgram(DeepgramProviderSettings::default())
            .expect("default Deepgram settings are always valid")
    })
}

/// A Deepgram provider. Cheap to clone the models out of; it holds no HTTP
/// client.
pub struct DeepgramProvider {
    name: String,
    base_url: String,
    headers: HeadersFn,
    fetch: Option<FetchFunction>,
}

impl DeepgramProvider {
    fn model_config(&self, method: &str) -> EndpointConfig {
        EndpointConfig::fixed(
            format!("{}.{method}", self.name),
            self.base_url.clone(),
            self.headers.clone(),
            self.fetch.clone(),
            None,
        )
    }

    /// A transcription (STT) model (e.g. `"nova-3"`); `provider()` is `"{name}.transcription"`.
    #[must_use]
    pub fn transcription(&self, model_id: &str) -> DeepgramTranscriptionModel {
        DeepgramTranscriptionModel::from_config(
            model_id.to_string(),
            self.model_config("transcription"),
        )
    }
}

crate::impl_single_modality_provider!(DeepgramProvider, transcription_model, |p, id| p
    .transcription(id));

// ── Provider options ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
struct DeepgramOptions {
    detect_entities: Option<bool>,
    detect_language: Option<bool>,
    filler_words: Option<bool>,
    language: Option<String>,
    punctuate: Option<bool>,
    redact: Option<Value>,
    search: Option<Value>,
    smart_format: Option<bool>,
    summarize: Option<bool>,
    topics: Option<Value>,
    utterances: Option<bool>,
    utt_split: Option<f64>,
    diarize: Option<bool>,
}

fn parse_deepgram_options(provider_options: Option<&SharedProviderOptions>) -> DeepgramOptions {
    let mut opts = DeepgramOptions::default();
    if let Some(dg) = options::deepgram_options(provider_options) {
        opts.detect_entities = dg
            .get("detectEntities")
            .and_then(serde_json::Value::as_bool);
        opts.detect_language = dg
            .get("detectLanguage")
            .and_then(serde_json::Value::as_bool);
        opts.filler_words = dg.get("fillerWords").and_then(serde_json::Value::as_bool);
        opts.language = dg
            .get("language")
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string);
        opts.punctuate = dg.get("punctuate").and_then(serde_json::Value::as_bool);
        opts.redact = dg.get("redact").cloned();
        opts.search = dg.get("search").cloned();
        opts.smart_format = dg.get("smartFormat").and_then(serde_json::Value::as_bool);
        opts.summarize = dg.get("summarize").and_then(serde_json::Value::as_bool);
        opts.topics = dg.get("topics").cloned();
        opts.utterances = dg.get("utterances").and_then(serde_json::Value::as_bool);
        opts.utt_split = dg.get("uttSplit").and_then(serde_json::Value::as_f64);
        opts.diarize = dg.get("diarize").and_then(serde_json::Value::as_bool);
    }
    opts
}

// ── Response schema ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct DeepgramWord {
    word: String,
    start: f64,
    end: f64,
}

#[derive(Debug, Deserialize)]
struct DeepgramAlternative {
    transcript: String,
    #[serde(default)]
    words: Option<Vec<DeepgramWord>>,
}

#[derive(Debug, Deserialize)]
struct DeepgramChannel {
    #[serde(default, rename = "detected_language")]
    detected_language: Option<String>,
    alternatives: Vec<DeepgramAlternative>,
}

#[derive(Debug, Deserialize)]
struct DeepgramResults {
    #[serde(default)]
    channels: Option<Vec<DeepgramChannel>>,
}

#[derive(Debug, Deserialize)]
struct DeepgramMetadata {
    #[serde(default)]
    duration: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct DeepgramResponse {
    #[serde(default)]
    metadata: Option<DeepgramMetadata>,
    #[serde(default)]
    results: Option<DeepgramResults>,
}

// ── Model ───────────────────────────────────────────────────────────────────

fn audio_input_to_bytes(audio: &AudioInput) -> Result<Vec<u8>, AiMuxError> {
    match audio {
        AudioInput::Binary(bytes) => Ok(bytes.clone()),
        AudioInput::Base64(b64) => {
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64)
                .map_err(|e| AiMuxError::InvalidArgument(format!("invalid base64: {e}")))
        }
    }
}

pub struct DeepgramTranscriptionModel {
    model_id: String,
    config: EndpointConfig,
}

impl DeepgramTranscriptionModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }
}

#[async_trait]
impl TranscriptionModel for DeepgramTranscriptionModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_generate(
        &self,
        options: &TranscriptionCallOptions,
    ) -> Result<TranscriptionResult, AiMuxError> {
        let dg_options = parse_deepgram_options(options.provider_options.as_ref());
        let warnings: Vec<Warning> = Vec::new();

        // Build query parameters.
        let mut params = vec![("model".to_string(), self.model_id.clone())];

        // diarize defaults to true.
        let diarize = dg_options.diarize.unwrap_or(true);
        params.push(("diarize".to_string(), diarize.to_string()));

        if let Some(v) = dg_options.detect_entities {
            params.push(("detect_entities".to_string(), v.to_string()));
        }
        if let Some(v) = dg_options.detect_language {
            params.push(("detect_language".to_string(), v.to_string()));
        }
        if let Some(v) = dg_options.filler_words {
            params.push(("filler_words".to_string(), v.to_string()));
        }
        if let Some(ref v) = dg_options.language {
            params.push(("language".to_string(), v.clone()));
        }
        if let Some(v) = dg_options.punctuate {
            params.push(("punctuate".to_string(), v.to_string()));
        }
        if let Some(v) = dg_options.smart_format {
            params.push(("smart_format".to_string(), v.to_string()));
        }
        if let Some(v) = dg_options.summarize {
            params.push(("summarize".to_string(), v.to_string()));
        }
        if let Some(v) = dg_options.utterances {
            params.push(("utterances".to_string(), v.to_string()));
        }
        if let Some(v) = dg_options.utt_split {
            params.push(("utt_split".to_string(), v.to_string()));
        }
        if let Some(ref v) = dg_options.redact {
            params.push(("redact".to_string(), v.to_string()));
        }
        if let Some(ref v) = dg_options.search {
            params.push(("search".to_string(), v.to_string()));
        }
        if let Some(ref v) = dg_options.topics {
            params.push(("topics".to_string(), v.to_string()));
        }

        let query_string = params
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("&");

        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let url = exchange.url(&format!("/v1/listen?{query_string}"));

        let audio_bytes = audio_input_to_bytes(&options.audio)?;

        let resp = aimux_provider_utils::post_to_api(
            exchange.request(url, options),
            HttpBody::Bytes(audio_bytes, options.media_type.clone()),
            aimux_provider_utils::create_json_response_handler::<DeepgramResponse>(),
            deepgram_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;

        let raw_body = resp.raw_value.unwrap_or(Value::Null);
        let parsed = resp.value;

        let channel = parsed
            .results
            .as_ref()
            .and_then(|r| r.channels.as_ref())
            .and_then(|c| c.first());

        let text = channel
            .and_then(|ch| ch.alternatives.first())
            .map(|alt| alt.transcript.clone())
            .unwrap_or_default();

        let segments: Vec<TranscriptionSegment> = channel
            .and_then(|ch| ch.alternatives.first())
            .and_then(|alt| alt.words.as_ref())
            .map(|words| {
                words
                    .iter()
                    .map(|w| TranscriptionSegment {
                        text: w.word.clone(),
                        start_second: w.start,
                        end_second: w.end,
                    })
                    .collect()
            })
            .unwrap_or_default();

        let language = channel.and_then(|ch| ch.detected_language.clone());

        let duration_in_seconds = parsed.metadata.as_ref().and_then(|m| m.duration);

        let timestamp = chrono::Utc::now().to_rfc3339();

        Ok(TranscriptionResult {
            text,
            segments,
            language,
            duration_in_seconds,
            warnings,
            request: Some(TranscriptionRequest { body: None }),
            response: TranscriptionResponse {
                timestamp: Some(timestamp),
                model_id: Some(self.model_id.clone()),
                headers: Some(response_headers),
                body: Some(raw_body),
            },
            provider_metadata: None,
        })
    }
}
