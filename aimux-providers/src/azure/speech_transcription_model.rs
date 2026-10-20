//! The Azure Speech REST protocol (azure-speech-transcription-model.ts).

use std::collections::HashSet;

use aimux_core::error::AiMuxError;
use aimux_core::shared::provider_namespace;
use aimux_core::transcription_model::{
    AudioInput, TranscriptionCallOptions, TranscriptionResponse, TranscriptionResult,
    TranscriptionSegment,
};
use aimux_provider_utils::{HttpBody, MultipartForm};
use serde::Deserialize;
use serde_json::{Value, json};

use super::options::NAMESPACE;
use super::speech_transcription_model_options::Timestamps;
use super::transcription_model_options::{AzureOptions, is_default_speech};
use crate::openai::config::OpenAIModelConfig;

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

/// One call to the Azure Speech API (the AI SDK's
/// `AzureSpeechTranscriptionModel.doGenerate`), with the already parsed
/// `providerOptions.azure`.
pub(super) async fn do_generate(
    model_id: &str,
    speech: &OpenAIModelConfig,
    options: &TranscriptionCallOptions,
    mut azure: AzureOptions,
) -> Result<TranscriptionResult, AiMuxError> {
    let timestamp = aimux_core::Timestamp::now();
    let audio = match &options.audio {
        AudioInput::Url(_) => return Err(AudioInput::url_error()),
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
        "model": if is_default_speech(model_id) { "MAI-Transcribe-2" } else { model_id },
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
        speech.http_request(
            speech.url("")?,
            speech.request_headers(options.headers.as_ref()).await?,
            options,
        ),
        HttpBody::Bytes(body, content_type),
        aimux_provider_utils::create_json_response_handler::<SpeechResponse>(),
        speech_failed_response_handler(),
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
                end_second: (phrase.offset_milliseconds? + phrase.duration_milliseconds?) / 1000.0,
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
            NAMESPACE,
            json!({"phrases": metadata_phrases}),
        )?),
        response: TranscriptionResponse {
            timestamp: Some(timestamp),
            model_id: Some(model_id.to_owned()),
            headers: Some(response.response_headers),
            body: Some(raw_body),
        },
    })
}

/// The speech model's error handler: `{ error: { message } }` or
/// `{ message }` (azure-speech-transcription-model.ts:72-79). The code is
/// read from `type`, else `code`, as the shared handler that layer 5 removed
/// from provider-utils did.
fn speech_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    aimux_provider_utils::create_json_error_response_handler(|data| {
        let error = data.get("error").unwrap_or(data);
        aimux_provider_utils::ProviderErrorParts {
            message: error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            provider_code: error
                .get("type")
                .or_else(|| error.get("code"))
                .and_then(Value::as_str)
                .map(str::to_owned),
        }
    })
}
