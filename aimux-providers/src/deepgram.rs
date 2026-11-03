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

use aimux_core::ErrorCause;
use aimux_core::error::AiMuxError;
use aimux_core::shared::{SharedProviderOptions, Warning};
use aimux_core::transcription_model::{
    AudioInput, TranscriptionCallOptions, TranscriptionModel, TranscriptionRequest,
    TranscriptionResponse, TranscriptionResult, TranscriptionSegment,
};
use aimux_provider_utils::HttpBody;
use aimux_provider_utils::{FetchFunction, HeaderMapOpt, Resolvable, validate_base_url};

use crate::shared::EndpointConfig;
use aimux_provider_utils::{AuthScheme, Credential, ProviderHeaders};

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

pub(crate) const DEFAULT_BASE_URL: &str = "https://api.deepgram.com";
pub(crate) const API_KEY_ENV_VAR: &str = "DEEPGRAM_API_KEY";
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
/// Returns `AiMuxError::invalid_argument` when `base_url` is not an `http(s)`
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
        headers: ProviderHeaders::new(
            Credential::explicit_or_env(settings.api_key, API_KEY_ENV_VAR, "Deepgram"),
            AuthScheme::Scheme("Token"),
            Vec::new(),
            settings.headers,
        )
        .with_user_agent(
            crate::shared::aisdk_pins::DEEPGRAM.name,
            crate::shared::aisdk_pins::DEEPGRAM.version,
        ),
        fetch: Some(crate::shared::package_fetch(
            settings.fetch,
            crate::shared::aisdk_pins::DEEPGRAM,
        )),
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
    headers: ProviderHeaders,
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

/// `deepgramTranscriptionModelOptionsSchema`
/// (deepgram-transcription-model-options.ts). Every key is `.nullish()`:
/// `null` reads as absent.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeepgramOptions {
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    detect_language: Option<bool>,
    #[serde(default)]
    smart_format: Option<bool>,
    #[serde(default)]
    punctuate: Option<bool>,
    #[serde(default)]
    paragraphs: Option<bool>,
    #[serde(default)]
    summarize: Option<Summarize>,
    #[serde(default)]
    topics: Option<bool>,
    #[serde(default)]
    intents: Option<bool>,
    #[serde(default)]
    sentiment: Option<bool>,
    #[serde(default)]
    detect_entities: Option<bool>,
    #[serde(default)]
    redact: Option<Redact>,
    #[serde(default)]
    replace: Option<String>,
    #[serde(default)]
    search: Option<String>,
    #[serde(default)]
    keyterm: Option<String>,
    #[serde(default)]
    diarize: Option<bool>,
    #[serde(default)]
    utterances: Option<bool>,
    #[serde(default)]
    utt_split: Option<f64>,
    #[serde(default)]
    filler_words: Option<bool>,
}

/// `summarize`: `z.union([z.literal('v2'), z.literal(false)])`.
#[derive(Debug, Clone, Copy)]
enum Summarize {
    V2,
    Off,
}

impl<'de> Deserialize<'de> for Summarize {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match Value::deserialize(deserializer)? {
            Value::String(value) if value == "v2" => Ok(Self::V2),
            Value::Bool(false) => Ok(Self::Off),
            _ => Err(serde::de::Error::custom("expected 'v2' or false")),
        }
    }
}

impl Summarize {
    /// `String(value)`.
    fn as_str(self) -> &'static str {
        match self {
            Self::V2 => "v2",
            Self::Off => "false",
        }
    }
}

/// `redact`: `z.union([z.string(), z.array(z.string())])`.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Redact {
    One(String),
    Many(Vec<String>),
}

impl Redact {
    /// `String(value)`: an array joins its items with commas.
    fn to_query_value(&self) -> String {
        match self {
            Self::One(value) => value.clone(),
            Self::Many(values) => values.join(","),
        }
    }
}

/// `parseProviderOptions({ provider: 'deepgram', providerOptions, schema:
/// deepgramTranscriptionModelOptionsSchema })` (deepgram-transcription-model.ts
/// getArgs); all `None` without a `deepgram` namespace.
fn parse_deepgram_options(
    provider_options: Option<&SharedProviderOptions>,
) -> Result<DeepgramOptions, AiMuxError> {
    Ok(
        aimux_provider_utils::parse_provider_options(options::NAMESPACE, provider_options)?
            .unwrap_or_default(),
    )
}

/// The query parameters of a transcription, in the order upstream builds its
/// body (deepgram-transcription-model.ts getArgs): `model`, then each option
/// that is set, as `String(value)`.
fn query_params(model_id: &str, dg: &DeepgramOptions) -> Vec<(&'static str, String)> {
    let flag = |value: Option<bool>| value.map(|value| value.to_string());
    [
        ("model", Some(model_id.to_string())),
        ("detect_entities", flag(dg.detect_entities)),
        ("detect_language", flag(dg.detect_language)),
        ("diarize", flag(dg.diarize)),
        ("filler_words", flag(dg.filler_words)),
        ("intents", flag(dg.intents)),
        ("keyterm", dg.keyterm.clone()),
        ("language", dg.language.clone()),
        ("paragraphs", flag(dg.paragraphs)),
        ("punctuate", flag(dg.punctuate)),
        ("redact", dg.redact.as_ref().map(Redact::to_query_value)),
        ("replace", dg.replace.clone()),
        ("search", dg.search.clone()),
        ("sentiment", flag(dg.sentiment)),
        ("smart_format", flag(dg.smart_format)),
        (
            "summarize",
            dg.summarize.map(|value| value.as_str().to_string()),
        ),
        ("topics", flag(dg.topics)),
        ("utterances", flag(dg.utterances)),
        ("utt_split", dg.utt_split.map(|value| value.to_string())),
    ]
    .into_iter()
    .filter_map(|(key, value)| value.map(|value| (key, value)))
    .collect()
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
        AudioInput::Base64(b64) => aimux_provider_utils::convert_base64_to_uint8_array(b64)
            .map_err(|e| AiMuxError::InvalidArgument {
                message: format!("invalid base64: {e}"),
                cause: Some(ErrorCause::source(&e)),
            }),
        AudioInput::Url(_) => Err(AudioInput::url_error()),
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
        let dg_options = parse_deepgram_options(options.provider_options.as_ref())?;
        let warnings: Vec<Warning> = Vec::new();

        let params = query_params(&self.model_id, &dg_options);
        // `new URLSearchParams(...).toString()`: form-encoded.
        let query_string = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(params.iter().map(|(key, value)| (*key, value.as_str())))
            .finish();

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

        let timestamp = aimux_core::Timestamp::now();

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
