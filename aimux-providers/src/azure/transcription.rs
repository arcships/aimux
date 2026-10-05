//! Per-request Azure transcription routing and the Azure Speech REST protocol.

use std::collections::HashSet;

use aimux_core::error::AiMuxError;
use aimux_core::shared::{SharedProviderOptions, Warning, provider_namespace};
use aimux_core::transcription_model::{
    AudioInput, TranscriptionCallOptions, TranscriptionModel, TranscriptionResponse,
    TranscriptionResult, TranscriptionSegment, TranscriptionStreamOptions,
    TranscriptionStreamResult,
};
use aimux_provider_utils::{HttpBody, MultipartForm};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::openai::{OpenAITranscriptionModel, config::OpenAIModelConfig};

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct AzureOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    api: Option<Api>,
    #[serde(skip_serializing_if = "Option::is_none")]
    timestamps: Option<Timestamps>,
    #[serde(skip_serializing_if = "Option::is_none")]
    transcribe_style: Option<TranscribeStyle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    locales: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    diarization: Option<Diarization>,
    #[serde(skip_serializing_if = "Option::is_none")]
    phrase_list: Option<PhraseList>,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum Api {
    Openai,
    Speech,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum Timestamps {
    Word,
    Segment,
    None,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum TranscribeStyle {
    Verbatim,
    Clean,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Diarization {
    enabled: bool,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PhraseList {
    phrases: Vec<String>,
}

fn is_default_speech(model_id: &str) -> bool {
    model_id.eq_ignore_ascii_case("mai-transcribe-2")
}

fn parse_options(options: Option<&SharedProviderOptions>) -> Result<AzureOptions, AiMuxError> {
    let Some(azure) = options.and_then(|options| options.get("azure")) else {
        return Ok(AzureOptions::default());
    };
    // Optional in the upstream schema means absent, never explicitly null.
    if azure.values().any(Value::is_null) {
        return Err(AiMuxError::InvalidArgument(
            "Invalid Azure transcription options: null field".into(),
        ));
    }
    let parsed: AzureOptions =
        serde_json::from_value(Value::Object(azure.clone())).map_err(|error| {
            AiMuxError::InvalidArgument(format!("Invalid Azure transcription options: {error}"))
        })?;
    if parsed
        .locales
        .as_ref()
        .is_some_and(|locales| locales.len() != 1)
    {
        return Err(AiMuxError::InvalidArgument(
            "Azure transcription locales must contain one language".into(),
        ));
    }
    Ok(parsed)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SpeechResponse {
    combined_phrases: Vec<CombinedPhrase>,
    duration_milliseconds: Option<f64>,
    phrases: Option<Vec<Phrase>>,
}
#[derive(Deserialize)]
struct CombinedPhrase {
    text: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Phrase {
    text: String,
    offset_milliseconds: Option<f64>,
    duration_milliseconds: Option<f64>,
    locale: Option<String>,
    #[serde(rename = "speaker")]
    _speaker: Option<f64>,
    #[serde(rename = "confidence")]
    _confidence: Option<f64>,
    #[serde(rename = "words")]
    _words: Option<Vec<Word>>,
}
#[derive(Deserialize)]
struct Word {
    #[serde(rename = "text")]
    _text: String,
    #[serde(rename = "offsetMilliseconds")]
    _offset_milliseconds: Option<f64>,
    #[serde(rename = "durationMilliseconds")]
    _duration_milliseconds: Option<f64>,
}

/// Azure's transcription model, resolving the API from provider options on every call.
pub struct AzureTranscriptionModel {
    model_id: String,
    openai: OpenAITranscriptionModel,
    speech: OpenAIModelConfig,
}

impl AzureTranscriptionModel {
    pub(crate) fn new(
        model_id: String,
        openai: OpenAITranscriptionModel,
        speech: OpenAIModelConfig,
    ) -> Self {
        Self {
            model_id,
            openai,
            speech,
        }
    }

    fn uses_speech(&self, options: &AzureOptions) -> bool {
        matches!(
            options.api.unwrap_or(if is_default_speech(&self.model_id) {
                Api::Speech
            } else {
                Api::Openai
            }),
            Api::Speech
        )
    }
}

#[async_trait::async_trait]
impl TranscriptionModel for AzureTranscriptionModel {
    fn provider(&self) -> &str {
        "azure.transcription"
    }
    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_generate(
        &self,
        options: &TranscriptionCallOptions,
    ) -> Result<TranscriptionResult, AiMuxError> {
        let mut azure = parse_options(options.provider_options.as_ref())?;
        if !self.uses_speech(&azure) {
            let mut result = self.openai.do_generate(options).await?;
            azure.api = None;
            let speech_options = serde_json::to_value(azure).expect("serializable options");
            for key in [
                "timestamps",
                "transcribeStyle",
                "locales",
                "diarization",
                "phraseList",
            ] {
                if speech_options.get(key).is_some() {
                    result.warnings.push(Warning::Unsupported {
                        feature: format!("providerOptions.azure.{key}"),
                        details: Some("This option requires the Azure Speech API.".into()),
                    });
                }
            }
            return Ok(result);
        }
        let timestamp = chrono::Utc::now().to_rfc3339();
        let audio = match &options.audio {
            AudioInput::Binary(audio) => audio.clone(),
            AudioInput::Base64(audio) => {
                let audio: String = audio
                    .chars()
                    .filter(|character| !matches!(character, '\t' | '\n' | '\x0c' | '\r' | ' '))
                    .map(|character| match character {
                        '-' => '+',
                        '_' => '/',
                        other => other,
                    })
                    .collect();
                if audio.ends_with('=') && !audio.len().is_multiple_of(4) {
                    return Err(AiMuxError::InvalidArgument("invalid base64 padding".into()));
                }
                let config = base64::engine::general_purpose::GeneralPurposeConfig::new()
                    .with_decode_padding_mode(base64::engine::DecodePaddingMode::Indifferent)
                    .with_decode_allow_trailing_bits(true);
                base64::Engine::decode(
                    &base64::engine::general_purpose::GeneralPurpose::new(
                        &base64::alphabet::STANDARD,
                        config,
                    ),
                    audio,
                )
                .map_err(|error| AiMuxError::InvalidArgument(format!("invalid base64: {error}")))?
            }
        };
        let mut definition = json!({"enhancedMode": {
            "enabled": true,
            "model": if is_default_speech(&self.model_id) { "MAI-Transcribe-2" } else { &self.model_id },
            "modelOptions": {"timestamps": azure.timestamps.take().unwrap_or(Timestamps::Segment)},
        }});
        if let Some(style) = azure.transcribe_style.take() {
            definition["enhancedMode"]["modelOptions"]["transcribeStyle"] = json!(style);
        }
        azure.api = None;
        definition
            .as_object_mut()
            .expect("definition object")
            .extend(
                serde_json::to_value(azure)
                    .expect("serializable options")
                    .as_object()
                    .expect("options object")
                    .clone(),
            );
        let media_type = options.media_type.to_lowercase();
        let subtype = media_type.split('/').nth(1).unwrap_or_default();
        let extension = match subtype {
            "mpeg" => "mp3",
            "x-wav" => "wav",
            "opus" => "ogg",
            "mp4" | "x-m4a" => "m4a",
            other => other,
        };
        let mut form = MultipartForm::new();
        form.file(
            "audio",
            &format!("audio.{extension}"),
            &options.media_type,
            &audio,
        )?;
        form.text("definition", &definition.to_string())?;
        let (body, content_type) = form.finish();
        let response = aimux_provider_utils::post_to_api(
            self.speech.http_request(
                self.speech.url("")?,
                self.speech
                    .request_headers(options.headers.as_ref())
                    .await?,
                options,
            ),
            HttpBody::Bytes(body, content_type),
            aimux_provider_utils::create_json_response_handler::<SpeechResponse>(),
            aimux_provider_utils::create_standard_json_error_response_handler(),
        )
        .await?;
        let parsed = response.value;
        let phrases = parsed.phrases.unwrap_or_default();
        let languages: HashSet<_> = phrases
            .iter()
            .filter_map(|phrase| phrase.locale.as_ref())
            .map(|locale| locale.split('-').next().unwrap_or_default().to_lowercase())
            .collect();
        let language = if languages.len() == 1 {
            languages.into_iter().next().filter(|language| {
                language.len() == 2 && language.bytes().all(|byte| byte.is_ascii_lowercase())
            })
        } else {
            None
        };
        let segments = phrases
            .iter()
            .filter_map(|phrase| {
                Some(TranscriptionSegment {
                    text: phrase.text.clone(),
                    start_second: phrase.offset_milliseconds? / 1000.0,
                    end_second: (phrase.offset_milliseconds? + phrase.duration_milliseconds?)
                        / 1000.0,
                })
            })
            .collect();
        let raw_body = response.raw_value.unwrap_or(Value::Null);
        // Schema parsing strips unknown keys but retains absent versus null fields.
        let mut metadata_phrases = raw_body
            .get("phrases")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for phrase in &mut metadata_phrases {
            if let Some(phrase) = phrase.as_object_mut() {
                phrase.retain(|key, _| {
                    matches!(
                        key.as_str(),
                        "text"
                            | "offsetMilliseconds"
                            | "durationMilliseconds"
                            | "locale"
                            | "speaker"
                            | "confidence"
                            | "words"
                    )
                });
                if let Some(words) = phrase.get_mut("words").and_then(Value::as_array_mut) {
                    for word in words {
                        if let Some(word) = word.as_object_mut() {
                            word.retain(|key, _| {
                                matches!(
                                    key.as_str(),
                                    "text" | "offsetMilliseconds" | "durationMilliseconds"
                                )
                            });
                        }
                    }
                }
            }
        }
        Ok(TranscriptionResult {
            text: parsed
                .combined_phrases
                .into_iter()
                .map(|phrase| phrase.text)
                .collect::<Vec<_>>()
                .join(" "),
            segments,
            language,
            duration_in_seconds: parsed
                .duration_milliseconds
                .map(|duration| duration / 1000.0),
            warnings: Vec::new(),
            request: None,
            provider_metadata: Some(provider_namespace(
                "azure",
                json!({"phrases": metadata_phrases}),
            )?),
            response: TranscriptionResponse {
                timestamp: Some(timestamp),
                model_id: Some(self.model_id.clone()),
                headers: Some(response.response_headers),
                body: Some(raw_body),
            },
        })
    }

    async fn do_stream(
        &self,
        options: TranscriptionStreamOptions,
    ) -> Result<TranscriptionStreamResult, AiMuxError> {
        if self.uses_speech(&parse_options(options.provider_options.as_ref())?) {
            return Err(AiMuxError::UnsupportedFunctionality(
                "streaming transcription with the Azure Speech API".into(),
            ));
        }
        self.openai.do_stream(options).await
    }
}
