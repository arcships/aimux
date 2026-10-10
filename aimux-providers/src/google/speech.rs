//! Google Gemini speech (TTS) model — implements `SpeechModel`.
//!
//! Aligned with the AI SDK's `GoogleSpeechModel`
//! (`google/src/google-speech-model.ts`, `google-speech-model-options.ts`,
//! `google-speech-input.ts`, `google-speech-api.ts`).
//!
//! Endpoint: `POST {base_url}/models/{model}:generateContent` with
//! `responseModalities: ["AUDIO"]`. The audio comes back base64 encoded in the
//! first inline-data part. Vertex reuses this model under a `google.vertex.*`
//! provider name and then reads its options under `googleVertex`/`vertex`.

use async_trait::async_trait;
use base64::Engine;
use serde::de::{Deserialize, Deserializer, Error as _};
use serde::{Deserialize as DeriveDeserialize, Serialize};
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::shared::Warning;
use aimux_core::speech_model::{
    AudioData, SpeechCallOptions, SpeechModel, SpeechRequest, SpeechResponse, SpeechResult,
};

use super::options::{GOOGLE, Namespace, google_metadata, no_null};
use crate::shared::EndpointConfig;

const DEFAULT_VOICE: &str = "Kore";
/// Gemini TTS returns raw PCM at 24kHz when the response does not specify a rate.
const DEFAULT_SAMPLE_RATE: u32 = 24000;

// ── Provider options (`google-speech-model-options.ts`) ─────────────────────

/// `z.never().optional()`: the key must not be present.
fn never<'de, D: Deserializer<'de>>(_: D) -> Result<Option<()>, D::Error> {
    Err(D::Error::custom("Custom voices are not supported"))
}

/// `z.string().min(1).optional()`.
fn non_empty<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    let value = String::deserialize(deserializer)?;
    if value.is_empty() {
        return Err(D::Error::custom("speaker must not be empty"));
    }
    Ok(Some(value))
}

#[derive(Clone, Default, Serialize, DeriveDeserialize)]
#[serde(rename_all = "camelCase")]
struct SpeechMetadata {
    #[serde(
        default,
        deserialize_with = "non_empty",
        skip_serializing_if = "Option::is_none"
    )]
    speaker: Option<String>,
    #[serde(
        default,
        deserialize_with = "no_null",
        skip_serializing_if = "Option::is_none"
    )]
    style: Option<String>,
}

#[derive(DeriveDeserialize)]
#[serde(rename_all = "camelCase")]
struct Turn {
    text: String,
    #[serde(default, deserialize_with = "no_null")]
    speech_metadata: Option<SpeechMetadata>,
}

#[derive(Clone, Serialize, DeriveDeserialize)]
#[serde(rename_all = "camelCase")]
struct PrebuiltVoiceConfig {
    voice_name: String,
}

#[derive(Clone, Serialize, DeriveDeserialize)]
#[serde(rename_all = "camelCase")]
struct VoiceConfig {
    prebuilt_voice_config: PrebuiltVoiceConfig,
    #[serde(
        default,
        rename = "voice",
        deserialize_with = "never",
        skip_serializing
    )]
    _voice: Option<()>,
}

#[derive(Clone, Serialize, DeriveDeserialize)]
#[serde(rename_all = "camelCase")]
struct SpeakerVoiceConfig {
    speaker: String,
    voice_config: VoiceConfig,
}

#[derive(Clone, Serialize, DeriveDeserialize)]
#[serde(rename_all = "camelCase")]
struct MultiSpeakerVoiceConfig {
    speaker_voice_configs: Vec<SpeakerVoiceConfig>,
}

#[derive(DeriveDeserialize)]
#[serde(rename_all = "camelCase")]
struct SpeechOptions {
    #[serde(default, deserialize_with = "no_null")]
    speech_metadata: Option<SpeechMetadata>,
    #[serde(default, deserialize_with = "no_null")]
    turns: Option<Vec<Turn>>,
    #[serde(default, deserialize_with = "no_null")]
    multi_speaker_voice_config: Option<MultiSpeakerVoiceConfig>,
}

fn parse_options(
    raw: Option<&aimux_core::shared::JsonObject>,
) -> Result<SpeechOptions, AiMuxError> {
    let invalid = |error: String| {
        AiMuxError::InvalidArgument(format!(
            "Invalid argument for parameter providerOptions: {error}"
        ))
    };
    let Some(raw) = raw else {
        return Ok(SpeechOptions {
            speech_metadata: None,
            turns: None,
            multi_speaker_voice_config: None,
        });
    };
    let options: SpeechOptions =
        serde_json::from_value(Value::Object(raw.clone())).map_err(|e| invalid(e.to_string()))?;
    if options.turns.as_ref().is_some_and(Vec::is_empty) {
        return Err(invalid("turns must contain at least one turn".into()));
    }
    Ok(options)
}

// ── Speech input (`google-speech-input.ts`) ─────────────────────────────────

/// The transcript and voice kind of a speech request.
#[derive(Debug, PartialEq, Eq)]
pub struct GoogleSpeechInput {
    pub text: String,
    pub uses_custom_voice: bool,
}

/// Inspects speech input without replacing provider option validation. Also
/// used by intermediaries that need the transcript or voice kind before
/// invoking a model. `provider_options` is the whole providerOptions record
/// and is read leniently: anything malformed is left to provider validation.
#[must_use]
pub fn google_speech_input(
    text: &str,
    voice: Option<&str>,
    provider_options: Option<&Value>,
) -> GoogleSpeechInput {
    let options = provider_options
        .and_then(|options| options.get(GOOGLE))
        .filter(|google| google.is_object());
    let mut text = text.to_owned();
    if let Some(turns) = options
        .and_then(|options| options.get("turns"))
        .and_then(Value::as_array)
    {
        let texts: Vec<&str> = turns
            .iter()
            .map_while(|turn| turn.get("text").and_then(Value::as_str))
            .collect();
        if !turns.is_empty() && texts.len() == turns.len() {
            text = texts.concat();
        }
    }
    let speakers = options
        .and_then(|options| options.get("multiSpeakerVoiceConfig"))
        .and_then(|config| config.get("speakerVoiceConfigs"))
        .and_then(Value::as_array);
    let uses_custom_voice = voice
        .is_some_and(|voice| voice.starts_with("voice_") || voice.starts_with("voicekey_"))
        || speakers.is_some_and(|speakers| {
            speakers.iter().any(|speaker| {
                speaker
                    .get("voiceConfig")
                    .and_then(Value::as_object)
                    .is_some_and(|config| config.contains_key("voice"))
            })
        });
    GoogleSpeechInput {
        text,
        uses_custom_voice,
    }
}

// ── Response (`google-speech-api.ts`) ───────────────────────────────────────

#[derive(DeriveDeserialize)]
#[serde(rename_all = "camelCase")]
struct InlineData {
    mime_type: Option<String>,
    data: Option<String>,
}

#[derive(DeriveDeserialize)]
#[serde(rename_all = "camelCase")]
struct ResponsePart {
    inline_data: Option<InlineData>,
}

#[derive(DeriveDeserialize)]
struct ResponseContent {
    parts: Option<Vec<ResponsePart>>,
}

#[derive(DeriveDeserialize)]
struct ResponseCandidate {
    content: Option<ResponseContent>,
}

#[derive(DeriveDeserialize)]
struct SpeechResponseBody {
    candidates: Option<Vec<ResponseCandidate>>,
}

// ── Model ───────────────────────────────────────────────────────────────────

/// A Google Gemini speech (TTS) model.
pub struct GoogleSpeechModel {
    model_id: String,
    config: EndpointConfig,
    namespace: Namespace,
}

impl GoogleSpeechModel {
    pub(crate) fn from_config(
        model_id: String,
        config: EndpointConfig,
        namespace: Namespace,
    ) -> Self {
        Self {
            model_id,
            config,
            namespace,
        }
    }
}

struct Args {
    body: Value,
    warnings: Vec<Warning>,
    output_format: &'static str,
    structured: bool,
}

fn unsupported(feature: &str, details: impl Into<String>) -> Warning {
    Warning::Unsupported {
        feature: feature.into(),
        details: Some(details.into()),
    }
}

impl GoogleSpeechModel {
    fn args(&self, options: &SpeechCallOptions) -> Result<Args, AiMuxError> {
        let mut warnings = Vec::new();
        let voice = options.voice.as_deref().unwrap_or(DEFAULT_VOICE);
        let text = options.text.as_str();
        let instructions = options.instructions.as_deref();

        // Vertex reads `googleVertex`/`vertex` (then `google`); every other
        // Google provider reads `google`. The constructor decides.
        let raw = self.namespace.read(options.provider_options.as_ref());
        let google = parse_options(raw)?;

        // Older Gemini families require prompt-based directions. Default newer
        // and custom model IDs to structured speech.
        let structured =
            !self.model_id.starts_with("gemini-2.5-") && !self.model_id.starts_with("gemini-3.1-");

        let input = google_speech_input(
            text,
            Some(voice),
            raw.map(|raw| {
                Value::Object(
                    [(GOOGLE.to_string(), Value::Object(raw.clone()))]
                        .into_iter()
                        .collect(),
                )
            })
            .as_ref(),
        );
        if input.uses_custom_voice {
            return Err(AiMuxError::InvalidArgument(
                "Invalid argument for parameter voice: Custom voices are not supported. Use a prebuilt voice instead.".into(),
            ));
        }

        // Multi-speaker (provider option) takes precedence over the single voice.
        let multi = google.multi_speaker_voice_config.as_ref();
        let speech_config = match multi {
            Some(multi) => json!({"multiSpeakerVoiceConfig": multi}),
            None => json!({"voiceConfig": {"prebuiltVoiceConfig": {"voiceName": voice}}}),
        };

        // Older models expect directions in the prompt. Prepending them to a
        // labelled multi-speaker transcript would break speaker parsing.
        let mut prompt_text = text.to_owned();
        if let Some(instructions) = instructions
            && !structured
        {
            if multi.is_some() {
                warnings.push(unsupported(
                    "instructions",
                    "Google Gemini TTS ignores `instructions` when `multiSpeakerVoiceConfig` is set, \
                     because prepending them would break multi-speaker transcript parsing.",
                ));
            } else {
                prompt_text = format!("{instructions}: {text}");
            }
        }

        let mut parts = vec![json!({"text": prompt_text})];
        if structured {
            if google.turns.is_some() && google.speech_metadata.is_some() {
                return Err(AiMuxError::InvalidArgument(
                    "Invalid argument for parameter providerOptions: Set speechMetadata on each turn when using turns."
                        .into(),
                ));
            }
            if google.turns.is_some() && !text.is_empty() {
                warnings.push(unsupported(
                    "text",
                    "Google TTS turns replace the top-level text.",
                ));
            }
            let source: Vec<(&str, Option<&SpeechMetadata>)> = match &google.turns {
                Some(turns) => turns
                    .iter()
                    .map(|turn| (turn.text.as_str(), turn.speech_metadata.as_ref()))
                    .collect(),
                None => vec![(text, google.speech_metadata.as_ref())],
            };
            parts = Vec::new();
            for (part_text, metadata) in source {
                let style = metadata
                    .and_then(|m| m.style.clone())
                    .or_else(|| instructions.map(str::to_owned));
                let speaker = metadata.and_then(|m| m.speaker.clone());
                if let Some(multi) = multi
                    && !multi
                        .speaker_voice_configs
                        .iter()
                        .any(|config| Some(&config.speaker) == speaker.as_ref())
                {
                    return Err(AiMuxError::InvalidArgument(
                        "Invalid argument for parameter speechMetadata.speaker: Every multi-speaker turn must specify a speechMetadata.speaker matching a configured speaker."
                            .into(),
                    ));
                }
                let mut part = json!({"text": part_text});
                if style.is_some() || speaker.is_some() {
                    part["speechMetadata"] = json!(SpeechMetadata { speaker, style });
                }
                parts.push(part);
            }
        } else if google.turns.is_some() || google.speech_metadata.is_some() {
            return Err(AiMuxError::InvalidArgument(
                "Invalid argument for parameter providerOptions: Structured speech metadata and turns require Gemini 3.8 TTS."
                    .into(),
            ));
        }

        if input.text.is_empty() {
            return Err(AiMuxError::InvalidArgument(
                "Invalid argument for parameter text: Speech input must contain a non-empty transcript."
                    .into(),
            ));
        }

        if options.speed.is_some() {
            warnings.push(unsupported(
                "speed",
                "Google Gemini TTS models do not support the `speed` option. It was ignored.",
            ));
        }
        if options.language.is_some() {
            warnings.push(unsupported(
                "language",
                "Google Gemini TTS models do not support the `language` option. \
                 Language is detected automatically from the input text.",
            ));
        }

        let formats: &[(&str, &'static str)] = if structured {
            &[
                ("wav", "AUDIO_WAV"),
                ("audio/wav", "AUDIO_WAV"),
                ("pcm", "AUDIO_L16"),
                ("audio/l16", "AUDIO_L16"),
                ("mulaw", "AUDIO_MULAW"),
                ("audio/mulaw", "AUDIO_MULAW"),
                ("alaw", "AUDIO_ALAW"),
                ("audio/alaw", "AUDIO_ALAW"),
            ]
        } else {
            &[("wav", "AUDIO_WAV"), ("pcm", "AUDIO_L16")]
        };
        let requested = options.output_format.as_deref();
        let mut output_format = "AUDIO_WAV";
        if let Some(requested) = requested {
            match formats.iter().find(|(name, _)| *name == requested) {
                Some((_, mime)) => output_format = mime,
                None => warnings.push(unsupported(
                    "outputFormat",
                    format!("Unsupported output format: {requested}. Using wav instead."),
                )),
            }
        }

        let mut generation_config = json!({
            "responseModalities": ["AUDIO"],
            "speechConfig": speech_config,
        });
        if structured && requested.is_some() {
            generation_config["responseFormat"] = json!({"audio": {"mimeType": output_format}});
        }
        Ok(Args {
            body: json!({
                "contents": [{"role": "user", "parts": parts}],
                "generationConfig": generation_config,
            }),
            warnings,
            output_format,
            structured,
        })
    }
}

#[async_trait]
impl SpeechModel for GoogleSpeechModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_generate(&self, options: &SpeechCallOptions) -> Result<SpeechResult, AiMuxError> {
        let timestamp = chrono::Utc::now().to_rfc3339();
        let Args {
            body,
            mut warnings,
            output_format,
            structured,
        } = self.args(options)?;

        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let response = aimux_provider_utils::post_json_to_api(
            exchange.request(
                exchange.url(&format!("/models/{}:generateContent", self.model_id)),
                options,
            ),
            body.clone(),
            aimux_provider_utils::create_json_response_handler::<SpeechResponseBody>(),
            super::google_failed_response_handler(),
        )
        .await?;

        // `generate_speech` returns one audio result and Gemini returns one
        // inline audio part per request: take the first inline part with data.
        let inline = response
            .value
            .candidates
            .into_iter()
            .flatten()
            .flat_map(|candidate| {
                candidate
                    .content
                    .and_then(|c| c.parts)
                    .into_iter()
                    .flatten()
            })
            .filter_map(|part| part.inline_data)
            .find(|inline| inline.data.as_deref().is_some_and(|data| !data.is_empty()));
        let mime_type = inline.as_ref().and_then(|inline| inline.mime_type.clone());
        let bytes = match inline.and_then(|inline| inline.data) {
            Some(data) => base64::engine::general_purpose::STANDARD
                .decode(data)
                .map_err(|e| {
                    AiMuxError::InvalidResponseData(format!("invalid base64 audio: {e}"))
                })?,
            None => Vec::new(),
        };

        let sample_rate = mime_type
            .as_deref()
            .and_then(|mime| {
                regex::Regex::new(r"rate=(\d+)")
                    .expect("static pattern")
                    .captures(mime)
                    .and_then(|captures| captures[1].parse::<u32>().ok())
            })
            .unwrap_or(DEFAULT_SAMPLE_RATE);

        // Older models return PCM, which needs a container for default WAV
        // output. Gemini 3.8 returns WAV itself; another header would corrupt it.
        let is_pcm = regex::Regex::new(r"(?i)^audio/(?:l16|pcm)(?:;|$)")
            .expect("static pattern")
            .is_match(mime_type.as_deref().unwrap_or(""))
            || (mime_type.is_none() && !structured);
        let audio = if output_format == "AUDIO_WAV" && is_pcm && !bytes.is_empty() {
            add_wav_header(&bytes, sample_rate)
        } else {
            bytes.clone()
        };

        if output_format == "AUDIO_L16" && !bytes.is_empty() && !structured {
            warnings.push(unsupported(
                "outputFormat",
                format!(
                    "Returning raw PCM audio (signed 16-bit little-endian, mono, {sample_rate} Hz). \
                     These bytes have no container header and are not directly playable; \
                     see providerMetadata.google for the sample rate and mime type."
                ),
            ));
        }

        Ok(SpeechResult {
            audio: AudioData::Binary(audio),
            warnings,
            request: Some(SpeechRequest { body: Some(body) }),
            response: SpeechResponse {
                timestamp: Some(timestamp),
                model_id: Some(self.model_id.clone()),
                headers: Some(response.response_headers),
                body: response.raw_value,
            },
            provider_metadata: Some(google_metadata(
                json!({"sampleRate": sample_rate, "mimeType": mime_type}),
            )),
        })
    }
}

/// Wraps raw signed 16-bit little-endian mono PCM in a minimal 44-byte WAV
/// (RIFF/WAVE) container.
fn add_wav_header(pcm: &[u8], sample_rate: u32) -> Vec<u8> {
    let (channels, bits, block_align): (u16, u16, u16) = (1, 16, 2);
    let data_size = u32::try_from(pcm.len()).unwrap_or(u32::MAX);
    let mut out = Vec::with_capacity(44 + pcm.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_size).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * u32::from(block_align)).to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&bits.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_size.to_le_bytes());
    out.extend_from_slice(pcm);
    out
}
