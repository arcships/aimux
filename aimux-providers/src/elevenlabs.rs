//! ElevenLabs speech (TTS) provider.
//!
//! Aligned with Vercel AI SDK `createElevenLabs`
//! (`reference/ai/packages/elevenlabs/src/elevenlabs-provider.ts`) and
//! `ElevenLabsSpeechModel`
//! (`reference/ai/packages/elevenlabs/src/elevenlabs-speech-model.ts`).
//!
//! Endpoint: `POST https://api.elevenlabs.io/v1/text-to-speech/{voiceId}`
//!
//! Authentication: `xi-api-key` header.
//!
//! The ElevenLabs TTS API accepts `text` and `model_id` in the request body and
//! returns raw binary audio. The `output_format` is passed as a query parameter.
//! `instructions` is not supported and emits a warning.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::shared::{SharedProviderOptions, Warning};
use aimux_core::speech_model::{
    AudioData, SpeechCallOptions, SpeechModel, SpeechRequest, SpeechResponse, SpeechResult,
};

use aimux_core::embedding_model::EmbeddingModel;
use aimux_core::image_model::ImageModel;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;
use aimux_provider_utils::{
    FetchFunction, HeaderMapOpt, HeadersFn, HttpBody, Resolvable, validate_base_url,
};

use crate::shared::{
    AuthScheme, Credential, EndpointConfig, TransformRequestBody, credential_headers,
};

pub(crate) mod options;

/// ElevenLabs errors: `{"detail": {"status": "invalid_api_key", "message": ...}}`
/// where `detail.status` is the machine code. FastAPI validation errors carry
/// `detail` as a plain string or a `[{loc, msg, type}]` list instead.
fn elevenlabs_error_parts(data: &Value) -> aimux_provider_utils::ProviderErrorParts {
    let detail = data.get("detail");
    let message = detail
        .and_then(|value| value.get("message"))
        .and_then(Value::as_str)
        .or_else(|| detail.and_then(Value::as_str))
        .or_else(|| {
            detail
                .and_then(Value::as_array)
                .and_then(|items| items.first())
                .and_then(|item| item.get("msg"))
                .and_then(Value::as_str)
        })
        .or_else(|| {
            data.get("error")
                .and_then(|value| value.get("message"))
                .and_then(Value::as_str)
        })
        .or_else(|| data.get("error").and_then(Value::as_str))
        .or_else(|| data.get("message").and_then(Value::as_str))
        .unwrap_or("ElevenLabs request failed")
        .to_string();
    aimux_provider_utils::ProviderErrorParts {
        message,
        provider_code: detail
            .and_then(|value| value.get("status"))
            .and_then(Value::as_str)
            .map(str::to_owned),
    }
}

fn elevenlabs_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    aimux_provider_utils::create_json_error_response_handler(elevenlabs_error_parts)
}

// ── Settings ─────────────────────────────────────────────────────────────────

const DEFAULT_BASE_URL: &str = "https://api.elevenlabs.io";
const API_KEY_ENV_VAR: &str = "ELEVENLABS_API_KEY";
const DEFAULT_NAME: &str = "elevenlabs";

/// Settings of [`create_elevenlabs`] (the AI SDK's `ElevenLabsProviderSettings`).
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except `base_url` and `name`; `api_key` and `headers` are
/// evaluated on every request, including the WebSocket handshake of realtime
/// transcription.
#[derive(Clone, Default)]
pub struct ElevenLabsProviderSettings {
    /// Base URL for the API calls. Default `https://api.elevenlabs.io`; a
    /// trailing slash is removed.
    pub base_url: Option<String>,
    /// The API key, sent as `xi-api-key`. `None` loads `ELEVENLABS_API_KEY`
    /// when a request is made and fails that request with
    /// `AiMuxError::LoadApiKey` if it is unset. An explicit value is used as
    /// given, `""` included: it never falls back to the environment. A
    /// [`Resolvable::Future`] is awaited once, an [`Resolvable::AsyncFn`] on
    /// every request.
    pub api_key: Option<Resolvable<String>>,
    /// Extra headers on every request. A `None` value removes the header,
    /// including `xi-api-key`. Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The provider name, the prefix of the `provider()` strings
    /// (`"{name}.speech"`, `"{name}.transcription"`). Default `"elevenlabs"`.
    /// The providerOptions key stays `elevenlabs`.
    pub name: Option<String>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
    /// Rewrites every JSON request body once, after it is serialized and
    /// before it is sent.
    pub transform_request_body: Option<TransformRequestBody>,
    /// Opens the WebSocket of realtime transcription (the AI SDK's
    /// `webSocket`). `None` uses the built-in tungstenite connector.
    #[cfg(feature = "realtime")]
    pub web_socket: Option<Arc<dyn aimux_provider_utils::ws::WsConnector>>,
}

impl std::fmt::Debug for ElevenLabsProviderSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = f.debug_struct("ElevenLabsProviderSettings");
        debug
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key)
            .field(
                "headers",
                &self.headers.as_ref().map(std::collections::HashMap::len),
            )
            .field("name", &self.name)
            .field("fetch", &self.fetch.is_some())
            .field(
                "transform_request_body",
                &self.transform_request_body.is_some(),
            );
        #[cfg(feature = "realtime")]
        debug.field("web_socket", &self.web_socket.is_some());
        debug.finish()
    }
}

/// Create an ElevenLabs provider.
///
/// # Errors
///
/// Returns `AiMuxError::InvalidArgument` when `base_url` is not an `http(s)`
/// URL with a host. That is the only way this fails: the key is loaded per
/// request, not here.
pub fn create_elevenlabs(
    settings: ElevenLabsProviderSettings,
) -> Result<ElevenLabsProvider, AiMuxError> {
    let base_url = match settings.base_url.as_deref() {
        Some(url) => validate_base_url(url)?,
        None => DEFAULT_BASE_URL.to_string(),
    };
    Ok(ElevenLabsProvider {
        name: settings.name.unwrap_or_else(|| DEFAULT_NAME.to_string()),
        base_url,
        headers: credential_headers(
            Credential::explicit_or_env(settings.api_key, API_KEY_ENV_VAR, "ElevenLabs"),
            AuthScheme::Header("xi-api-key"),
            Vec::new(),
            settings.headers,
        ),
        fetch: settings.fetch,
        transform_request_body: settings.transform_request_body,
        #[cfg(feature = "realtime")]
        web_socket: settings.web_socket,
    })
}

/// The default provider: `create_elevenlabs` with default settings, created
/// on first use. Creating it reads nothing from the environment and cannot
/// fail; a missing key surfaces from the first request instead.
pub fn elevenlabs() -> &'static ElevenLabsProvider {
    static DEFAULT: OnceLock<ElevenLabsProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_elevenlabs(ElevenLabsProviderSettings::default())
            .expect("default ElevenLabs settings are always valid")
    })
}

// ── Provider ─────────────────────────────────────────────────────────────────

/// ElevenLabs provider (the AI SDK's `ElevenLabsProvider`) — creates speech
/// and transcription models. Cheap to clone the models out of; it holds no
/// HTTP client.
pub struct ElevenLabsProvider {
    name: String,
    base_url: String,
    headers: HeadersFn,
    fetch: Option<FetchFunction>,
    transform_request_body: Option<TransformRequestBody>,
    #[cfg(feature = "realtime")]
    web_socket: Option<Arc<dyn aimux_provider_utils::ws::WsConnector>>,
}

impl ElevenLabsProvider {
    fn model_config(&self, method: &str) -> EndpointConfig {
        EndpointConfig::fixed(
            format!("{}.{method}", self.name),
            self.base_url.clone(),
            self.headers.clone(),
            self.fetch.clone(),
            self.transform_request_body.clone(),
        )
    }

    /// A speech (TTS) model (e.g. `"eleven_multilingual_v2"`); `provider()`
    /// is `"{name}.speech"`.
    #[must_use]
    pub fn speech(&self, model_id: &str) -> ElevenLabsSpeechModel {
        ElevenLabsSpeechModel::from_config(model_id.to_string(), self.model_config("speech"))
    }

    /// A transcription (STT) model (e.g. `"scribe_v1"`); `provider()` is
    /// `"{name}.transcription"`. Uses the `/v1/speech-to-text` endpoint.
    #[must_use]
    pub fn transcription(&self, model_id: &str) -> ElevenLabsTranscriptionModel {
        ElevenLabsTranscriptionModel::from_config(
            model_id.to_string(),
            self.model_config("transcription"),
            #[cfg(feature = "realtime")]
            self.web_socket.clone(),
        )
    }
}

impl Provider for ElevenLabsProvider {
    fn language_model(&self, model_id: &str) -> Result<Arc<dyn LanguageModel>, AiMuxError> {
        Err(AiMuxError::no_such_model(model_id, "languageModel"))
    }

    fn embedding_model(&self, model_id: &str) -> Result<Arc<dyn EmbeddingModel>, AiMuxError> {
        Err(AiMuxError::no_such_model(model_id, "embeddingModel"))
    }

    fn image_model(&self, model_id: &str) -> Result<Arc<dyn ImageModel>, AiMuxError> {
        Err(AiMuxError::no_such_model(model_id, "imageModel"))
    }

    fn transcription_model(
        &self,
        model_id: &str,
    ) -> Option<Result<Arc<dyn TranscriptionModel>, AiMuxError>> {
        Some(Ok(Arc::new(self.transcription(model_id))))
    }

    fn speech_model(&self, model_id: &str) -> Option<Result<Arc<dyn SpeechModel>, AiMuxError>> {
        Some(Ok(Arc::new(self.speech(model_id))))
    }
}

// ── Speech model ─────────────────────────────────────────────────────────────

/// The default voice ID used when no voice is provided.
const DEFAULT_VOICE_ID: &str = "21m00Tcm4TlvDq8ikWAM";

/// The default output format used when no output format is provided.
const DEFAULT_OUTPUT_FORMAT: &str = "mp3_44100_128";

/// An ElevenLabs speech (TTS) model.
pub struct ElevenLabsSpeechModel {
    model_id: String,
    config: EndpointConfig,
}

impl ElevenLabsSpeechModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }
}

#[async_trait]
impl SpeechModel for ElevenLabsSpeechModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_generate(&self, options: &SpeechCallOptions) -> Result<SpeechResult, AiMuxError> {
        let (body, query_params, warnings, voice_id) = build_request(options, &self.model_id)?;

        let exchange = self.config.exchange(options.headers.as_ref()).await?;

        let endpoint = exchange.url(&format!("/v1/text-to-speech/{voice_id}"));
        let url = if query_params.is_empty() {
            endpoint
        } else {
            let qs: Vec<String> = query_params
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            format!("{endpoint}?{}", qs.join("&"))
        };

        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(url, options),
            exchange.transform_body(Value::Object(body.clone())),
            aimux_provider_utils::create_binary_response_handler(),
            elevenlabs_failed_response_handler(),
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

/// Build the ElevenLabs TTS request body, query params, warnings, and voice id.
///
/// Mirrors the TS `ElevenLabsSpeechModel.getArgs`:
/// - `voice` defaults to `"21m00Tcm4TlvDq8ikWAM"`.
/// - `output_format` is mapped to the ElevenLabs format and sent as a query
///   parameter (defaults to `"mp3_44100_128"`).
/// - `speed` is placed inside `voice_settings`.
/// - `language` is placed in the request body as `language_code`.
/// - `instructions` is not supported and emits a warning.
/// - Provider options (`elevenlabs` key) are applied: `voiceSettings`,
///   `languageCode`, `pronunciationDictionaryLocators`, `seed`, `previousText`,
///   `nextText`, `previousRequestIds`, `nextRequestIds`,
///   `applyTextNormalization`, `applyLanguageTextNormalization`, `enableLogging`.
#[allow(clippy::type_complexity)]
fn build_request(
    options: &SpeechCallOptions,
    model_id: &str,
) -> Result<
    (
        Map<String, Value>,
        Vec<(String, String)>,
        Vec<Warning>,
        String,
    ),
    AiMuxError,
> {
    let mut warnings = Vec::new();

    let voice_id = options
        .voice
        .clone()
        .unwrap_or_else(|| DEFAULT_VOICE_ID.to_string());

    let mut body = Map::new();
    body.insert("text".to_string(), json!(options.text));
    body.insert("model_id".to_string(), json!(model_id));

    // Map outputFormat to ElevenLabs format (as query param).
    let mut query_params: Vec<(String, String)> = Vec::new();
    let output_format = options
        .output_format
        .as_deref()
        .unwrap_or(DEFAULT_OUTPUT_FORMAT);
    let mapped_format = map_output_format(output_format);
    query_params.push(("output_format".to_string(), mapped_format));

    // Add language code if provided.
    if let Some(ref language) = options.language {
        body.insert("language_code".to_string(), json!(language));
    }

    // Build voice_settings.
    let mut voice_settings = Map::new();
    if let Some(speed) = options.speed {
        voice_settings.insert("speed".to_string(), json!(speed));
    }

    // Parse and apply provider-specific options.
    let elevenlabs_options = parse_elevenlabs_provider_options(options.provider_options.as_ref());
    if let Some(ref opts) = elevenlabs_options {
        // Voice settings from provider options.
        if let Some(ref vs) = opts.voice_settings {
            if let Some(stability) = vs.stability {
                voice_settings.insert("stability".to_string(), json!(stability));
            }
            if let Some(similarity_boost) = vs.similarity_boost {
                voice_settings.insert("similarity_boost".to_string(), json!(similarity_boost));
            }
            if let Some(style) = vs.style {
                voice_settings.insert("style".to_string(), json!(style));
            }
            if let Some(use_speaker_boost) = vs.use_speaker_boost {
                voice_settings.insert("use_speaker_boost".to_string(), json!(use_speaker_boost));
            }
        }

        // Add language code from provider options if not already set.
        if let Some(ref lc) = opts.language_code
            && !body.contains_key("language_code")
        {
            body.insert("language_code".to_string(), json!(lc));
        }

        // Pronunciation dictionary locators.
        if let Some(ref locators) = opts.pronunciation_dictionary_locators {
            let mapped: Vec<Value> = locators
                .iter()
                .map(|loc| {
                    let mut m = Map::new();
                    m.insert(
                        "pronunciation_dictionary_id".to_string(),
                        json!(loc.pronunciation_dictionary_id),
                    );
                    if let Some(ref vid) = loc.version_id {
                        m.insert("version_id".to_string(), json!(vid));
                    }
                    Value::Object(m)
                })
                .collect();
            body.insert(
                "pronunciation_dictionary_locators".to_string(),
                json!(mapped),
            );
        }

        if let Some(seed) = opts.seed {
            body.insert("seed".to_string(), json!(seed));
        }
        if let Some(ref pt) = opts.previous_text {
            body.insert("previous_text".to_string(), json!(pt));
        }
        if let Some(ref nt) = opts.next_text {
            body.insert("next_text".to_string(), json!(nt));
        }
        if let Some(ref prids) = opts.previous_request_ids {
            body.insert("previous_request_ids".to_string(), json!(prids));
        }
        if let Some(ref nrids) = opts.next_request_ids {
            body.insert("next_request_ids".to_string(), json!(nrids));
        }
        if let Some(ref atn) = opts.apply_text_normalization {
            body.insert("apply_text_normalization".to_string(), json!(atn));
        }
        if let Some(atln) = opts.apply_language_text_normalization {
            body.insert("apply_language_text_normalization".to_string(), json!(atln));
        }

        // enable_logging is a query parameter.
        if let Some(el) = opts.enable_logging {
            query_params.push(("enable_logging".to_string(), el.to_string()));
        }
    }

    // Only add voice_settings if there are settings to add.
    if !voice_settings.is_empty() {
        body.insert("voice_settings".to_string(), Value::Object(voice_settings));
    }

    if options.instructions.is_some() {
        warnings.push(Warning::Unsupported {
            feature: "instructions".to_string(),
            details: Some(
                "ElevenLabs speech models do not support instructions. Instructions parameter was ignored."
                    .to_string(),
            ),
        });
    }

    Ok((body, query_params, warnings, voice_id))
}

/// Map a generic output format to the ElevenLabs-specific format string.
///
/// Known short names (e.g. `"mp3"`) are expanded to the full ElevenLabs format
/// (e.g. `"mp3_44100_128"`). Unknown formats are passed through as-is.
fn map_output_format(format: &str) -> String {
    match format {
        "mp3" => "mp3_44100_128".to_string(),
        "mp3_32" => "mp3_44100_32".to_string(),
        "mp3_64" => "mp3_44100_64".to_string(),
        "mp3_96" => "mp3_44100_96".to_string(),
        "mp3_128" => "mp3_44100_128".to_string(),
        "mp3_192" => "mp3_44100_192".to_string(),
        "pcm" => "pcm_44100".to_string(),
        "pcm_16000" => "pcm_16000".to_string(),
        "pcm_22050" => "pcm_22050".to_string(),
        "pcm_24000" => "pcm_24000".to_string(),
        "pcm_44100" => "pcm_44100".to_string(),
        "ulaw" => "ulaw_8000".to_string(),
        other => other.to_string(),
    }
}

// ── Provider options parsing ─────────────────────────────────────────────────

/// Parsed `elevenlabs` speech provider options.
#[derive(Debug, Default)]
struct ElevenLabsSpeechProviderOptions {
    language_code: Option<String>,
    voice_settings: Option<ElevenLabsVoiceSettings>,
    pronunciation_dictionary_locators: Option<Vec<ElevenLabsPronunciationLocator>>,
    seed: Option<u64>,
    previous_text: Option<String>,
    next_text: Option<String>,
    previous_request_ids: Option<Vec<String>>,
    next_request_ids: Option<Vec<String>>,
    apply_text_normalization: Option<String>,
    apply_language_text_normalization: Option<bool>,
    enable_logging: Option<bool>,
}

#[derive(Debug, Default)]
struct ElevenLabsVoiceSettings {
    stability: Option<f64>,
    similarity_boost: Option<f64>,
    style: Option<f64>,
    use_speaker_boost: Option<bool>,
}

#[derive(Debug)]
struct ElevenLabsPronunciationLocator {
    pronunciation_dictionary_id: String,
    version_id: Option<String>,
}

/// Extract ElevenLabs-specific speech options from the shared provider options.
fn parse_elevenlabs_provider_options(
    options: Option<&SharedProviderOptions>,
) -> Option<ElevenLabsSpeechProviderOptions> {
    let provider_opts = options::elevenlabs_options(options)?;
    let opts = provider_opts.as_object()?;

    let voice_settings = opts
        .get("voiceSettings")
        .and_then(|vs| vs.as_object())
        .map(|vs| ElevenLabsVoiceSettings {
            stability: vs.get("stability").and_then(serde_json::Value::as_f64),
            similarity_boost: vs
                .get("similarityBoost")
                .and_then(serde_json::Value::as_f64),
            style: vs.get("style").and_then(serde_json::Value::as_f64),
            use_speaker_boost: vs
                .get("useSpeakerBoost")
                .and_then(serde_json::Value::as_bool),
        });

    let pronunciation_dictionary_locators = opts
        .get("pronunciationDictionaryLocators")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|loc| loc.as_object())
                .map(|loc| ElevenLabsPronunciationLocator {
                    pronunciation_dictionary_id: loc
                        .get("pronunciationDictionaryId")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    version_id: loc
                        .get("versionId")
                        .and_then(|v| v.as_str())
                        .map(std::string::ToString::to_string),
                })
                .collect()
        });

    Some(ElevenLabsSpeechProviderOptions {
        language_code: opts
            .get("languageCode")
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string),
        voice_settings,
        pronunciation_dictionary_locators,
        seed: opts.get("seed").and_then(serde_json::Value::as_u64),
        previous_text: opts
            .get("previousText")
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string),
        next_text: opts
            .get("nextText")
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string),
        previous_request_ids: opts
            .get("previousRequestIds")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(std::string::ToString::to_string)
                    .collect()
            }),
        next_request_ids: opts
            .get("nextRequestIds")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(std::string::ToString::to_string)
                    .collect()
            }),
        apply_text_normalization: opts
            .get("applyTextNormalization")
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string),
        apply_language_text_normalization: opts
            .get("applyLanguageTextNormalization")
            .and_then(serde_json::Value::as_bool),
        enable_logging: opts
            .get("enableLogging")
            .and_then(serde_json::Value::as_bool),
    })
}

// ════════════════════════════════════════════════════════════════════════════
// Transcription (STT) model
// ════════════════════════════════════════════════════════════════════════════

use aimux_core::transcription_model::{
    AudioInput, TranscriptionCallOptions, TranscriptionModel, TranscriptionRequest,
    TranscriptionResponse, TranscriptionResult, TranscriptionSegment,
};
use aimux_provider_utils::{MultipartForm, media_type_to_extension};
use serde::Deserialize;

/// ElevenLabs transcription response word.
#[derive(Debug, Deserialize)]
struct ElevenLabsTranscriptionWord {
    text: String,
    #[serde(default)]
    start: Option<f64>,
    #[serde(default)]
    end: Option<f64>,
}

/// ElevenLabs transcription API response body.
#[derive(Debug, Deserialize)]
struct ElevenLabsTranscriptionResponse {
    language_code: String,
    #[allow(dead_code)]
    language_probability: f64,
    text: String,
    #[serde(default)]
    words: Option<Vec<ElevenLabsTranscriptionWord>>,
}

/// ElevenLabs transcription (STT) model — implements `TranscriptionModel`.
///
/// Aligned with Vercel AI SDK `ElevenLabsTranscriptionModel`
/// (`reference/ai/packages/elevenlabs/src/elevenlabs-transcription-model.ts`).
///
/// Endpoint: `POST {base_url}/v1/speech-to-text` (multipart form-data)
pub struct ElevenLabsTranscriptionModel {
    model_id: String,
    config: EndpointConfig,
    #[cfg(feature = "realtime")]
    web_socket: Option<Arc<dyn aimux_provider_utils::ws::WsConnector>>,
}

impl ElevenLabsTranscriptionModel {
    pub(crate) fn from_config(
        model_id: String,
        config: EndpointConfig,
        #[cfg(feature = "realtime")] web_socket: Option<
            Arc<dyn aimux_provider_utils::ws::WsConnector>,
        >,
    ) -> Self {
        Self {
            model_id,
            config,
            #[cfg(feature = "realtime")]
            web_socket,
        }
    }
}

fn audio_input_to_bytes_stt(audio: &AudioInput) -> Result<Vec<u8>, AiMuxError> {
    match audio {
        AudioInput::Binary(bytes) => Ok(bytes.clone()),
        AudioInput::Base64(b64) => {
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64)
                .map_err(|e| AiMuxError::InvalidArgument(format!("invalid base64: {e}")))
        }
    }
}

/// Realtime streaming model IDs (`scribe_v2_realtime*`) use the WebSocket
/// realtime endpoint; every other model uses the batch REST endpoint.
/// Wire shape per the public API reference (2026-09, RFC-0034 §3).
fn is_realtime_transcription_model_id(model_id: &str) -> bool {
    model_id == "scribe_v2_realtime" || model_id.starts_with("scribe_v2_realtime-")
}

#[async_trait]
impl TranscriptionModel for ElevenLabsTranscriptionModel {
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
        if is_realtime_transcription_model_id(&self.model_id) {
            return Err(AiMuxError::UnsupportedFunctionality(format!(
                "non-streaming transcription is not supported by `{}` \
                 (realtime models stream over a WebSocket session)",
                self.model_id
            )));
        }

        let warnings: Vec<Warning> = Vec::new();

        let audio_bytes = audio_input_to_bytes_stt(&options.audio)?;
        let file_extension = media_type_to_extension(&options.media_type);
        let filename = format!("audio.{file_extension}");

        let mut form = MultipartForm::new();
        form.text("model_id", &self.model_id)?;
        form.file("file", &filename, &options.media_type, &audio_bytes)?;
        form.text("diarize", "true")?;

        // Parse provider options.
        if let Some(el) = options::elevenlabs_options(options.provider_options.as_ref()) {
            if let Some(v) = el.get("diarize").and_then(serde_json::Value::as_bool) {
                form.text("diarize", &v.to_string())?;
            }
            if let Some(v) = el.get("languageCode").and_then(|v| v.as_str()) {
                form.text("language_code", v)?;
            }
            if let Some(v) = el
                .get("tagAudioEvents")
                .and_then(serde_json::Value::as_bool)
            {
                form.text("tag_audio_events", &v.to_string())?;
            }
            if let Some(v) = el.get("numSpeakers").and_then(serde_json::Value::as_u64) {
                form.text("num_speakers", &v.to_string())?;
            }
            if let Some(v) = el.get("timestampsGranularity").and_then(|v| v.as_str()) {
                form.text("timestamps_granularity", v)?;
            }
            if let Some(v) = el.get("fileFormat").and_then(|v| v.as_str()) {
                form.text("file_format", v)?;
            }
        }

        let (body_bytes, content_type) = form.finish();

        let exchange = self.config.exchange(options.headers.as_ref()).await?;

        let resp = aimux_provider_utils::post_to_api(
            exchange.request(exchange.url("/v1/speech-to-text"), options),
            HttpBody::Bytes(body_bytes, content_type),
            aimux_provider_utils::create_json_response_handler::<ElevenLabsTranscriptionResponse>(),
            elevenlabs_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;
        let raw_body = resp.raw_value.unwrap_or(Value::Null);
        let parsed = resp.value;

        let segments: Vec<TranscriptionSegment> = parsed
            .words
            .as_ref()
            .map(|words| {
                words
                    .iter()
                    .map(|w| TranscriptionSegment {
                        text: w.text.clone(),
                        start_second: w.start.unwrap_or(0.0),
                        end_second: w.end.unwrap_or(0.0),
                    })
                    .collect()
            })
            .unwrap_or_default();

        let duration_in_seconds = parsed
            .words
            .as_ref()
            .and_then(|w| w.last())
            .and_then(|w| w.end);

        let timestamp = chrono::Utc::now().to_rfc3339();

        Ok(TranscriptionResult {
            text: parsed.text,
            segments,
            language: Some(parsed.language_code),
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
    /// Streaming transcription over the ElevenLabs realtime WebSocket
    /// (RFC-0034 §3, wire shape per the public API reference 2026-09).
    ///
    /// Config travels on the URL (no `session.update` message); audio rides
    /// base64 in `input_audio_chunk` JSON frames; `commit_strategy` is fixed
    /// to `manual` (D3) so the stream ends when the caller's audio ends —
    /// the final chunk carries `commit: true`, the resulting
    /// `committed_transcript` is THE final, then the client closes (1000).
    /// A settle window (`chunk_ms`, default 5s) bounds the wait for that
    /// final event so the stream always terminates.
    ///
    /// Live-API smoke pending (no key at implementation time); the wire
    /// shape is pinned field-by-field against the documented reference by
    /// the local mock-server tests — same posture as RFC-0028 D4.
    #[cfg(feature = "realtime")]
    async fn do_stream(
        &self,
        options: aimux_core::transcription_model::TranscriptionStreamOptions,
    ) -> Result<aimux_core::transcription_model::TranscriptionStreamResult, AiMuxError> {
        use aimux_core::transcription_model::{TranscriptionStreamPart, TranscriptionStreamResult};
        use aimux_provider_utils::ws::{WebSocketRequest, WsMessage, ws_connect};
        use futures::StreamExt;

        if !is_realtime_transcription_model_id(&self.model_id) {
            return Err(AiMuxError::UnsupportedFunctionality(format!(
                "streaming transcription is not supported by `{}` \
                 (realtime models such as scribe_v2_realtime only)",
                self.model_id
            )));
        }

        // Parameter surface is deliberately minimal (RFC-0034 D5):
        // languageCode + includeTimestamps. Anything else waits for a user.
        let mut language_code: Option<String> = None;
        let mut include_timestamps = false;
        if let Some(el) = options::elevenlabs_options(options.provider_options.as_ref()) {
            if let Some(v) = el.get("languageCode").and_then(serde_json::Value::as_str) {
                language_code = Some(v.to_string());
            }
            if let Some(v) = el
                .get("includeTimestamps")
                .and_then(serde_json::Value::as_bool)
            {
                include_timestamps = v;
            }
        }

        // Audio format: the realtime endpoint takes pcm_{rate} / ulaw_{rate}.
        let default_rate = if options.input_audio_format.format_type == "audio/pcmu" {
            8_000
        } else {
            16_000
        };
        let sample_rate = options.input_audio_format.rate.unwrap_or(default_rate);
        let audio_format = match options.input_audio_format.format_type.as_str() {
            "audio/pcm" => format!("pcm_{sample_rate}"),
            "audio/pcmu" => format!("ulaw_{sample_rate}"),
            other => {
                return Err(AiMuxError::UnsupportedFunctionality(format!(
                    "ElevenLabs realtime accepts audio/pcm or audio/pcmu input, got {other}"
                )));
            }
        };

        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let base = exchange.base_url();
        let (scheme, host) = if let Some(rest) = base.strip_prefix("https://") {
            ("wss", rest)
        } else if let Some(rest) = base.strip_prefix("http://") {
            ("ws", rest)
        } else {
            ("wss", base)
        };
        // Scoped: the serializer is not `Send`; keep it inside this block so
        // nothing non-Send is alive across the connect await below.
        let ws_url = {
            let mut query = url::form_urlencoded::Serializer::new(String::new());
            query
                .append_pair("model_id", &self.model_id)
                .append_pair("audio_format", &audio_format)
                .append_pair("commit_strategy", "manual");
            if let Some(code) = &language_code {
                query.append_pair("language_code", code);
            }
            if include_timestamps {
                query.append_pair("include_timestamps", "true");
            }
            format!(
                "{scheme}://{host}/v1/speech-to-text/realtime?{}",
                query.finish()
            )
        };

        let header_list = exchange.headers();

        // Connect BEFORE the stream so connect failures surface from
        // do_stream's Result (same contract as the OpenAI realtime path).
        let req = WebSocketRequest {
            url: ws_url.clone(),
            headers: header_list,
            subprotocols: Vec::new(),
            abort_signal: options.abort_signal.clone(),
            timeout: options.timeout,
            connector: self.web_socket.clone(),
        };
        let mut ws = ws_connect(&req).await?;

        // Settle window for the post-commit final event: the server does not
        // close the session (RFC-0034 §3.3), so an empty Finish must be
        // possible. chunk_ms when configured, otherwise 5s.
        let settle = options
            .timeout
            .as_ref()
            .and_then(|t| t.chunk_ms)
            .map(tokio::time::Duration::from_millis)
            .unwrap_or(tokio::time::Duration::from_secs(5));

        let include_raw = options.include_raw_chunks;
        let model_id = self.model_id.clone();
        let error_url = ws_url.clone();
        let mut audio = options.audio;

        let stream = async_stream::stream! {
            // One held chunk: the commit flag must ride the LAST real audio
            // chunk, so each incoming chunk flushes the previous one and the
            // stream-end flushes the held one with commit=true. An
            // empty-audio stream commits via a single empty chunk.
            let mut held: Option<String> = None;
            let mut audio_done = false;
            let mut commit_deadline: Option<tokio::time::Instant> = None;
            // Audio is not sent before the session is established: the
            // server confirms configuration with session_started first.
            let mut session_started = false;

            loop {
                let audio_next = async {
                    if audio_done || !session_started {
                        std::future::pending::<()>().await;
                        None
                    } else {
                        audio.next().await
                    }
                };

                tokio::select! {
                    biased;

                    chunk = audio_next => {
                        // Hold-one-chunk pipeline: the commit flag must ride
                        // the LAST real chunk. A new chunk flushes the
                        // previously held one (commit=false); the stream end
                        // flushes the held one with commit=true; an
                        // empty-audio stream commits a single empty chunk.
                        let (payload, commit) = match chunk {
                            None => {
                                audio_done = true;
                                commit_deadline = Some(tokio::time::Instant::now() + settle);
                                (held.take().unwrap_or_default(), true)
                            }
                            Some(aimux_core::transcription_model::AudioChunk::Binary(bytes)) => {
                                use base64::Engine as _;
                                let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
                                match held.replace(b64) {
                                    Some(previous) => (previous, false),
                                    // First chunk: hold it, nothing to send yet.
                                    None => continue,
                                }
                            }
                            Some(aimux_core::transcription_model::AudioChunk::Base64(b64)) => {
                                match held.replace(b64) {
                                    Some(previous) => (previous, false),
                                    None => continue,
                                }
                            }
                        };
                        let message = serde_json::json!({
                            "message_type": "input_audio_chunk",
                            "audio_base_64": payload,
                            "commit": commit,
                            "sample_rate": sample_rate,
                        });
                        if let Err(e) = ws.send_text(&message.to_string()).await {
                            yield Err(e);
                            break;
                        }
                    }

                    _ = async {
                        match commit_deadline {
                            Some(d) => tokio::time::sleep_until(d).await,
                            None => std::future::pending::<()>().await,
                        }
                    } => {
                        // Server went silent after commit: an empty Finish
                        // beats hanging (RFC-0034 §3.3.3).
                        yield Ok(TranscriptionStreamPart::Finish {
                            text: String::new(),
                            segments: vec![],
                            language: language_code.clone(),
                            duration_in_seconds: None,
                            provider_metadata: None,
                        });
                        ws.close().await;
                        break;
                    }

                    event = ws.next() => {
                        match event {
                            None => {
                                yield Err(AiMuxError::ApiCall(Box::new(
                                    aimux_core::error::ApiCallError::new(
                                        "realtime transcription socket closed before the committed transcript",
                                        error_url.clone(),
                                        serde_json::json!({}),
                                    ),
                                )));
                                break;
                            }
                            Some(Err(e)) => {
                                yield Err(e);
                                break;
                            }
                            Some(Ok(WsMessage::Binary(_))) => {}
                            Some(Ok(WsMessage::Text(text))) => {
                                let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
                                    continue;
                                };
                                if include_raw {
                                    yield Ok(TranscriptionStreamPart::Raw {
                                        raw_value: value.clone(),
                                    });
                                }
                                let event_type = value.get("message_type")
                                    .and_then(|t| t.as_str()).unwrap_or("");
                                match event_type {
                                    "session_started" => {
                                        session_started = true;
                                        yield Ok(TranscriptionStreamPart::StreamStart {
                                            warnings: vec![],
                                        });
                                    }
                                    "partial_transcript" => {
                                        yield Ok(TranscriptionStreamPart::TranscriptPartial {
                                            id: None,
                                            text: value.get("text")
                                                .and_then(|v| v.as_str()).unwrap_or("").to_string(),
                                            start_second: None,
                                            duration_in_seconds: None,
                                            channel_index: None,
                                            provider_metadata: None,
                                        });
                                    }
                                    // include_timestamps=true swaps the event
                                    // shape; both carry `text`, the timestamps
                                    // variant adds `words[]`.
                                    "committed_transcript"
                                    | "committed_transcript_with_timestamps" => {
                                        let text = value.get("text")
                                            .and_then(|v| v.as_str()).unwrap_or("").to_string();
                                        let segments = value.get("words")
                                            .and_then(|v| v.as_array())
                                            .map(|words| {
                                                words.iter().filter_map(|w| {
                                                    let word = w.get("text")
                                                        .and_then(|v| v.as_str())?;
                                                    // Spacing-type entries are
                                                    // layout, not content.
                                                    if word.trim().is_empty() {
                                                        return None;
                                                    }
                                                    Some(TranscriptionSegment {
                                                        text: word.to_string(),
                                                        start_second: w.get("start")
                                                            .and_then(serde_json::Value::as_f64).unwrap_or(0.0),
                                                        end_second: w.get("end")
                                                            .and_then(serde_json::Value::as_f64).unwrap_or(0.0),
                                                    })
                                                }).collect::<Vec<_>>()
                                            })
                                            .unwrap_or_default();
                                        let duration = segments.last().map(|s| s.end_second);
                                        yield Ok(TranscriptionStreamPart::TranscriptFinal {
                                            id: None,
                                            text: text.clone(),
                                            start_second: segments.first()
                                                .map(|s| s.start_second),
                                            end_second: segments.last()
                                                .map(|s| s.end_second),
                                            channel_index: None,
                                            provider_metadata: None,
                                        });
                                        // Manual strategy commits exactly once
                                        // (on our final flag): this committed
                                        // transcript IS the finish.
                                        yield Ok(TranscriptionStreamPart::Finish {
                                            text,
                                            segments,
                                            language: language_code.clone(),
                                            duration_in_seconds: duration,
                                            provider_metadata: None,
                                        });
                                        ws.close().await;
                                        break;
                                    }
                                    "warning" => {
                                        // No part mapping (StreamStart already
                                        // went out); visible via Raw when
                                        // include_raw_chunks is set.
                                    }
                                    other if matches!(
                                        other,
                                        "error" | "auth_error" | "quota_exceeded"
                                        | "commit_throttled" | "unaccepted_terms"
                                        | "rate_limited" | "queue_overflow"
                                        | "resource_exhausted"
                                        | "session_time_limit_exceeded"
                                        | "input_error" | "invalid_request"
                                        | "chunk_size_exceeded"
                                        | "insufficient_audio_activity"
                                        | "transcriber_error"
                                    ) => {
                                        let message = value.get("error")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("realtime transcription error");
                                        // Retryable classification (RFC-0034
                                        // §3.2): three transient names, the
                                        // rest are terminal verdicts.
                                        let is_retryable = matches!(
                                            other,
                                            "rate_limited" | "queue_overflow"
                                            | "resource_exhausted"
                                        );
                                        yield Err(AiMuxError::ApiCall(Box::new(
                                            aimux_core::error::ApiCallError {
                                                is_retryable,
                                                response_body: Some(value.to_string()),
                                                ..aimux_core::error::ApiCallError::new(
                                                    format!("elevenlabs realtime: {message}"),
                                                    error_url.clone(),
                                                    serde_json::json!({}),
                                                )
                                            },
                                        )));
                                        ws.close().await;
                                        break;
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }
        };

        Ok(TranscriptionStreamResult {
            stream: Box::pin(stream),
            request: Some(TranscriptionRequest { body: Some(ws_url) }),
            response: Some(TranscriptionResponse {
                timestamp: Some(chrono::Utc::now().to_rfc3339()),
                model_id: Some(model_id),
                headers: None,
                body: None,
            }),
        })
    }

    /// Without the `realtime` feature the WebSocket path is compiled out.
    #[cfg(not(feature = "realtime"))]
    async fn do_stream(
        &self,
        _options: aimux_core::transcription_model::TranscriptionStreamOptions,
    ) -> Result<aimux_core::transcription_model::TranscriptionStreamResult, AiMuxError> {
        Err(AiMuxError::UnsupportedFunctionality(format!(
            "streaming transcription with `{}` requires building aimux-providers \
             with the `realtime` feature",
            self.model_id
        )))
    }
}
