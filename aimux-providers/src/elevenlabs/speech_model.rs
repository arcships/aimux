//! The ElevenLabs speech model (`elevenlabs-speech-model.ts`) and its
//! options (`elevenlabs-speech-model-options.ts`, `elevenlabs-speech-options.ts`).

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::shared::{SharedProviderOptions, Warning};
use aimux_core::speech_model::{
    AudioData, SpeechCallOptions, SpeechModel, SpeechRequest, SpeechResponse, SpeechResult,
};
use aimux_provider_utils::optional_field;

use crate::shared::EndpointConfig;

use super::error::elevenlabs_failed_response_handler;
use super::options;

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

        let timestamp = aimux_core::Timestamp::now();

        Ok(SpeechResult {
            audio: AudioData::Binary(audio_bytes),
            warnings,
            usage: None,
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
    let elevenlabs_options = parse_elevenlabs_provider_options(options.provider_options.as_ref())?;
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

        if let Some(ref seed) = opts.seed {
            body.insert("seed".to_string(), json!(seed));
        }
        // `if (elevenLabsOptions.previousText)`: an empty string is not sent.
        if let Some(pt) = opts.previous_text.as_ref().filter(|text| !text.is_empty()) {
            body.insert("previous_text".to_string(), json!(pt));
        }
        // `if (elevenLabsOptions.nextText)`: an empty string is not sent.
        if let Some(nt) = opts.next_text.as_ref().filter(|text| !text.is_empty()) {
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

/// `elevenLabsSpeechModelOptionsSchema` (elevenlabs-speech-model-options.ts).
/// Every key is `.optional()`: absent is `None`, `null` fails.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ElevenLabsSpeechProviderOptions {
    #[serde(default, deserialize_with = "optional_field")]
    language_code: Option<String>,
    #[serde(default, deserialize_with = "optional_field")]
    voice_settings: Option<ElevenLabsVoiceSettings>,
    /// At most 3.
    #[serde(default, deserialize_with = "at_most_three")]
    pronunciation_dictionary_locators: Option<Vec<ElevenLabsPronunciationLocator>>,
    /// `z.number().min(0).max(4294967295)`, sent as written.
    #[serde(default, deserialize_with = "seed")]
    seed: Option<serde_json::Number>,
    #[serde(default, deserialize_with = "optional_field")]
    previous_text: Option<String>,
    #[serde(default, deserialize_with = "optional_field")]
    next_text: Option<String>,
    /// At most 3.
    #[serde(default, deserialize_with = "at_most_three")]
    previous_request_ids: Option<Vec<String>>,
    /// At most 3.
    #[serde(default, deserialize_with = "at_most_three")]
    next_request_ids: Option<Vec<String>>,
    #[serde(default, deserialize_with = "optional_field")]
    apply_text_normalization: Option<TextNormalization>,
    #[serde(default, deserialize_with = "optional_field")]
    apply_language_text_normalization: Option<bool>,
    #[serde(default, deserialize_with = "optional_field")]
    enable_logging: Option<bool>,
}

/// `z.enum(['auto', 'on', 'off'])`, sent as written.
#[derive(Debug, Clone, Copy, Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
enum TextNormalization {
    Auto,
    On,
    Off,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ElevenLabsVoiceSettings {
    /// 0 to 1.
    #[serde(default, deserialize_with = "unit_interval")]
    stability: Option<f64>,
    /// 0 to 1.
    #[serde(default, deserialize_with = "unit_interval")]
    similarity_boost: Option<f64>,
    /// 0 to 1.
    #[serde(default, deserialize_with = "unit_interval")]
    style: Option<f64>,
    #[serde(default, deserialize_with = "optional_field")]
    use_speaker_boost: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ElevenLabsPronunciationLocator {
    pronunciation_dictionary_id: String,
    #[serde(default, deserialize_with = "optional_field")]
    version_id: Option<String>,
}

/// `.max(3)` on an `.optional()` array.
fn at_most_three<'de, D, T>(deserializer: D) -> Result<Option<Vec<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    let items = Vec::<T>::deserialize(deserializer)?;
    if items.len() > 3 {
        return Err(serde::de::Error::custom("expected at most 3 items"));
    }
    Ok(Some(items))
}

/// `z.number().min(0).max(1)` on an `.optional()` key.
fn unit_interval<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<f64>, D::Error> {
    let value = f64::deserialize(deserializer)?;
    if (0.0..=1.0).contains(&value) {
        Ok(Some(value))
    } else {
        Err(serde::de::Error::custom("expected a number from 0 to 1"))
    }
}

/// `seed`: `z.number().min(0).max(4294967295)` on an `.optional()` key.
fn seed<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<serde_json::Number>, D::Error> {
    let number = serde_json::Number::deserialize(deserializer)?;
    if number
        .as_f64()
        .is_some_and(|value| (0.0..=4_294_967_295.0).contains(&value))
    {
        Ok(Some(number))
    } else {
        Err(serde::de::Error::custom(
            "expected a number from 0 to 4294967295",
        ))
    }
}

/// `parseProviderOptions({ provider: 'elevenlabs', providerOptions, schema:
/// elevenLabsSpeechModelOptionsSchema })` (elevenlabs-speech-model.ts
/// getArgs).
fn parse_elevenlabs_provider_options(
    options: Option<&SharedProviderOptions>,
) -> Result<Option<ElevenLabsSpeechProviderOptions>, AiMuxError> {
    aimux_provider_utils::parse_provider_options(options::NAMESPACE, options)
}
