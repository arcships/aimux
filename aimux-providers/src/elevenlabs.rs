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

use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::shared::{SharedProviderOptions, Warning};
use aimux_core::speech_model::{
    AudioData, SpeechCallOptions, SpeechModel, SpeechRequest, SpeechResponse, SpeechResult,
};

use aimux_provider_utils::response::DEFAULT_ERROR_STRUCTURE;
use aimux_provider_utils::{HttpBody, HttpMethod, HttpRequest, RetryConfig, load_api_key, send};

// ── Config ───────────────────────────────────────────────────────────────────

/// Configuration for the ElevenLabs provider.
#[derive(Debug, Clone)]
pub struct ElevenLabsConfig {
    pub api_key: String,
    /// Base URL for the ElevenLabs API (no trailing slash). Defaults to
    /// `https://api.elevenlabs.io`.
    pub base_url: String,
    /// Extra headers merged into every request.
    pub headers: Option<HashMap<String, String>>,
}

impl ElevenLabsConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://api.elevenlabs.io".to_string(),
            headers: None,
        }
    }

    /// Override the base URL (for testing or proxies).
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into().trim_end_matches('/').to_string();
        self
    }

    /// Attach extra headers merged into every request.
    pub fn with_headers(mut self, headers: HashMap<String, String>) -> Self {
        self.headers = Some(headers);
        self
    }

    /// Create from environment variable `ELEVENLABS_API_KEY`.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let api_key = load_api_key(None, "ELEVENLABS_API_KEY", "ElevenLabs")?;
        Ok(Self::new(api_key))
    }
}

// ── Provider ─────────────────────────────────────────────────────────────────

/// ElevenLabs provider — creates `ElevenLabsSpeechModel` instances.
pub struct ElevenLabsProvider {
    config: ElevenLabsConfig,
}

impl ElevenLabsProvider {
    pub fn new(config: ElevenLabsConfig) -> Self {
        Self { config }
    }

    /// Create a speech (TTS) model instance for the given model name (e.g.
    /// `"eleven_multilingual_v2"`).
    pub fn speech(&self, model_id: &str) -> ElevenLabsSpeechModel {
        ElevenLabsSpeechModel::new(model_id.to_string(), self.config.clone())
    }

    /// Create a transcription (STT) model instance for the given model name
    /// (e.g. `"scribe_v1"`). Uses the `/v1/speech-to-text` endpoint.
    pub fn transcription(&self, model_id: &str) -> ElevenLabsTranscriptionModel {
        ElevenLabsTranscriptionModel::new(model_id.to_string(), self.config.clone())
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
    config: ElevenLabsConfig,
}

impl ElevenLabsSpeechModel {
    pub fn new(model_id: String, config: ElevenLabsConfig) -> Self {
        Self { model_id, config }
    }

    fn build_headers(&self, extra: Option<&HashMap<String, String>>) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        headers.insert("xi-api-key".to_string(), self.config.api_key.clone());
        if let Some(ref config_headers) = self.config.headers {
            for (k, v) in config_headers {
                headers.insert(k.clone(), v.clone());
            }
        }
        if let Some(extra) = extra {
            for (k, v) in extra {
                headers.insert(k.clone(), v.clone());
            }
        }
        headers
    }

    fn endpoint(&self, voice_id: &str) -> String {
        format!("{}/v1/text-to-speech/{}", self.config.base_url, voice_id)
    }
}

#[async_trait]
impl SpeechModel for ElevenLabsSpeechModel {
    fn provider(&self) -> &str {
        "elevenlabs.speech"
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_generate(&self, options: &SpeechCallOptions) -> Result<SpeechResult, AiMuxError> {
        let (body, query_params, warnings, voice_id) = build_request(options, &self.model_id)?;

        let headers = self.build_headers(options.headers.as_ref());

        let url = if query_params.is_empty() {
            self.endpoint(&voice_id)
        } else {
            let qs: Vec<String> = query_params
                .iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect();
            format!("{}?{}", self.endpoint(&voice_id), qs.join("&"))
        };

        let resp = send(
            HttpRequest {
                method: HttpMethod::Post,
                url,
                headers: headers
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
                body: HttpBody::Json(Value::Object(body.clone())),

                abort_signal: options.abort_signal.clone(),
                call_id: None,
            },
            RetryConfig::default(),
            &DEFAULT_ERROR_STRUCTURE,
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
    let provider_opts = options.and_then(|opts| opts.get("elevenlabs"))?;
    let opts = provider_opts.as_object()?;

    let voice_settings = opts
        .get("voiceSettings")
        .and_then(|vs| vs.as_object())
        .map(|vs| ElevenLabsVoiceSettings {
            stability: vs.get("stability").and_then(|v| v.as_f64()),
            similarity_boost: vs.get("similarityBoost").and_then(|v| v.as_f64()),
            style: vs.get("style").and_then(|v| v.as_f64()),
            use_speaker_boost: vs.get("useSpeakerBoost").and_then(|v| v.as_bool()),
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
                        .map(|s| s.to_string()),
                })
                .collect()
        });

    Some(ElevenLabsSpeechProviderOptions {
        language_code: opts
            .get("languageCode")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        voice_settings,
        pronunciation_dictionary_locators,
        seed: opts.get("seed").and_then(|v| v.as_u64()),
        previous_text: opts
            .get("previousText")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        next_text: opts
            .get("nextText")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        previous_request_ids: opts
            .get("previousRequestIds")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| s.to_string())
                    .collect()
            }),
        next_request_ids: opts
            .get("nextRequestIds")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| s.to_string())
                    .collect()
            }),
        apply_text_normalization: opts
            .get("applyTextNormalization")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        apply_language_text_normalization: opts
            .get("applyLanguageTextNormalization")
            .and_then(|v| v.as_bool()),
        enable_logging: opts.get("enableLogging").and_then(|v| v.as_bool()),
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
    config: ElevenLabsConfig,
}

impl ElevenLabsTranscriptionModel {
    pub fn new(model_id: String, config: ElevenLabsConfig) -> Self {
        Self { model_id, config }
    }

    fn build_headers(&self, extra: Option<&HashMap<String, String>>) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        headers.insert("xi-api-key".to_string(), self.config.api_key.clone());
        if let Some(ref config_headers) = self.config.headers {
            for (k, v) in config_headers {
                headers.insert(k.clone(), v.clone());
            }
        }
        if let Some(extra) = extra {
            for (k, v) in extra {
                headers.insert(k.clone(), v.clone());
            }
        }
        headers
    }

    fn endpoint(&self) -> String {
        format!("{}/v1/speech-to-text", self.config.base_url)
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

#[async_trait]
impl TranscriptionModel for ElevenLabsTranscriptionModel {
    fn provider(&self) -> &str {
        "elevenlabs"
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_generate(
        &self,
        options: &TranscriptionCallOptions,
    ) -> Result<TranscriptionResult, AiMuxError> {
        let warnings: Vec<Warning> = Vec::new();

        let audio_bytes = audio_input_to_bytes_stt(&options.audio)?;
        let file_extension = media_type_to_extension(&options.media_type);
        let filename = format!("audio.{file_extension}");

        let mut form = MultipartForm::new();
        form.text("model_id", &self.model_id)?;
        form.file("file", &filename, &options.media_type, &audio_bytes)?;
        form.text("diarize", "true")?;

        // Parse provider options.
        if let Some(ref po) = options.provider_options
            && let Some(el) = po.get("elevenlabs")
        {
            if let Some(v) = el.get("diarize").and_then(|v| v.as_bool()) {
                form.text("diarize", &v.to_string())?;
            }
            if let Some(v) = el.get("languageCode").and_then(|v| v.as_str()) {
                form.text("language_code", v)?;
            }
            if let Some(v) = el.get("tagAudioEvents").and_then(|v| v.as_bool()) {
                form.text("tag_audio_events", &v.to_string())?;
            }
            if let Some(v) = el.get("numSpeakers").and_then(|v| v.as_u64()) {
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

        let headers = self.build_headers(options.headers.as_ref());

        let resp = send(
            HttpRequest {
                method: HttpMethod::Post,
                url: self.endpoint(),
                headers: headers
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
                body: HttpBody::Bytes(body_bytes, content_type),

                abort_signal: options.abort_signal.clone(),
                call_id: None,
            },
            RetryConfig::default(),
            &DEFAULT_ERROR_STRUCTURE,
        )
        .await?;

        let response_headers = resp.headers;
        let raw_body: Value = serde_json::from_slice(&resp.body).unwrap_or(Value::Null);

        let parsed: ElevenLabsTranscriptionResponse = serde_json::from_value(raw_body.clone())
            .map_err(|e| AiMuxError::Json(e.to_string()))?;

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
}
