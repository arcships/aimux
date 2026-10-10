//! Gemini transcription through Vertex generateContent and Live API.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use super::ProjectLocationFn;
use crate::google::options::{GOOGLE, Namespace};
use crate::google::transcription::parse_offset_seconds;
use crate::shared::EndpointConfig;
use aimux_core::error::AiMuxError;
use aimux_core::shared::provider_namespace;
use aimux_core::transcription_model::{
    AudioInput, TranscriptionCallOptions, TranscriptionModel, TranscriptionResponse,
    TranscriptionResult, TranscriptionSegment, TranscriptionStreamOptions,
    TranscriptionStreamResult,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Word {
    word: Option<String>,
    start_offset: Option<String>,
    end_offset: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AudioTranscription {
    text: Option<String>,
    language_code: Option<String>,
    #[serde(rename = "speakerLabel")]
    _speaker_label: Option<String>,
    words: Option<Vec<Word>>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Part {
    text: Option<String>,
    audio_transcription: Option<AudioTranscription>,
}
#[derive(Deserialize)]
struct Content {
    parts: Option<Vec<Part>>,
}
#[derive(Deserialize)]
struct Candidate {
    content: Option<Content>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Response {
    candidates: Option<Vec<Candidate>>,
    usage_metadata: Option<serde_json::Map<String, Value>>,
}

/// Gemini transcription model, distinct from Cloud Speech-to-Text.
pub struct VertexGeminiTranscriptionModel {
    model_id: String,
    config: EndpointConfig,
    project_location: ProjectLocationFn,
    #[cfg(feature = "realtime")]
    web_socket: Option<std::sync::Arc<dyn aimux_provider_utils::ws::WsConnector>>,
}

impl VertexGeminiTranscriptionModel {
    pub(crate) fn from_config(
        model_id: String,
        config: EndpointConfig,
        project_location: ProjectLocationFn,
        #[cfg(feature = "realtime")] web_socket: Option<
            std::sync::Arc<dyn aimux_provider_utils::ws::WsConnector>,
        >,
    ) -> Self {
        Self {
            model_id,
            config,
            project_location,
            #[cfg(feature = "realtime")]
            web_socket,
        }
    }
}

#[async_trait]
impl TranscriptionModel for VertexGeminiTranscriptionModel {
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
        if self.model_id.contains("-live") {
            return Err(AiMuxError::InvalidArgument(format!(
                "Model '{}' only supports streaming transcription. Use stream_transcribe or a unary model.",
                self.model_id
            )));
        }
        (self.project_location)().await?;
        let timestamp = chrono::Utc::now().to_rfc3339();
        let config = crate::google::transcription::TranscriptionOptions::parse(
            Namespace::Vertex.read(options.provider_options.as_ref()),
        )?
        .audio_transcription_config();
        let audio = match &options.audio {
            AudioInput::Base64(value) => value.clone(),
            AudioInput::Binary(value) => {
                base64::Engine::encode(&base64::engine::general_purpose::STANDARD, value)
            }
        };
        let mut body = json!({"contents": [{"role": "user", "parts": [{"inlineData": {"mimeType": options.media_type, "data": audio}}]}]});
        if config.as_object().is_some_and(|config| !config.is_empty()) {
            body["generationConfig"] = json!({"audioTranscriptionConfig": config});
        }
        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let response = aimux_provider_utils::post_json_to_api(
            exchange.request(
                exchange.url(&format!("/models/{}:generateContent", self.model_id)),
                options,
            ),
            exchange.transform_body(body),
            aimux_provider_utils::create_json_response_handler::<Response>(),
            crate::google::google_failed_response_handler(),
        )
        .await?;
        let parts = response
            .value
            .candidates
            .unwrap_or_default()
            .into_iter()
            .next()
            .and_then(|candidate| candidate.content)
            .and_then(|content| content.parts)
            .unwrap_or_default();
        let mut text: String = parts
            .iter()
            .filter_map(|part| part.text.as_deref())
            .collect();
        if text.is_empty() {
            text = parts
                .iter()
                .filter_map(|part| part.audio_transcription.as_ref()?.text.as_deref())
                .collect();
        }
        let mut language = None;
        let mut segments = Vec::new();
        for part in parts {
            let Some(transcription) = part.audio_transcription else {
                continue;
            };
            if language.is_none() {
                language = transcription.language_code;
            }
            for word in transcription.words.unwrap_or_default() {
                if let (Some(text), Some(start_second), Some(end_second)) = (
                    word.word.clone(),
                    parse_offset_seconds(word.start_offset.as_deref()),
                    parse_offset_seconds(word.end_offset.as_deref()),
                ) {
                    segments.push(TranscriptionSegment {
                        text,
                        start_second,
                        end_second,
                    });
                }
            }
        }
        Ok(TranscriptionResult {
            text,
            segments,
            language,
            duration_in_seconds: None,
            warnings: Vec::new(),
            request: None,
            response: TranscriptionResponse {
                timestamp: Some(timestamp),
                model_id: Some(self.model_id.clone()),
                headers: Some(response.response_headers),
                body: response.raw_value,
            },
            provider_metadata: response.value.usage_metadata.map(|usage| {
                provider_namespace(GOOGLE, json!({"usageMetadata": usage}))
                    .expect("provider metadata must be an object")
            }),
        })
    }

    async fn do_stream(
        &self,
        options: TranscriptionStreamOptions,
    ) -> Result<TranscriptionStreamResult, AiMuxError> {
        if !self.model_id.contains("-live") {
            return Err(AiMuxError::InvalidArgument(format!(
                "Model '{}' does not support streaming transcription. Use a live model.",
                self.model_id
            )));
        }
        #[cfg(feature = "realtime")]
        {
            self.stream_live(options).await
        }
        #[cfg(not(feature = "realtime"))]
        {
            let _ = options;
            Err(AiMuxError::UnsupportedFunctionality(
                "Enable realtime for Vertex Live transcription".into(),
            ))
        }
    }
}

#[cfg(feature = "realtime")]
impl VertexGeminiTranscriptionModel {
    async fn stream_live(
        &self,
        options: TranscriptionStreamOptions,
    ) -> Result<TranscriptionStreamResult, AiMuxError> {
        use aimux_core::transcription_model::TranscriptionRequest;
        use aimux_provider_utils::ws::{WebSocketRequest, ws_connect};

        crate::google::transcription::validate_live_input_audio_format(&options)?;
        let target = (self.project_location)().await?;
        let config = crate::google::transcription::TranscriptionOptions::parse(
            Namespace::Vertex.read(options.provider_options.as_ref()),
        )?
        .audio_transcription_config();
        let setup = json!({"setup": {
            "model": format!("projects/{}/locations/{}/publishers/google/models/{}", target.project, target.location, self.model_id),
            "inputAudioTranscription": config,
        }});
        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let url = format!(
            "wss://{}/ws/google.cloud.aiplatform.v1.LlmBidiService/BidiGenerateContent",
            super::location_host(&target.location)
        );
        let mut socket = ws_connect(&WebSocketRequest::for_transcription(
            url,
            exchange.headers(),
            &options,
            self.web_socket.clone(),
        ))
        .await?;
        socket.send_text(&setup.to_string()).await?;
        let request = Some(TranscriptionRequest {
            body: Some(setup["setup"].to_string()),
        });
        let response = Some(TranscriptionResponse {
            timestamp: Some(chrono::Utc::now().to_rfc3339()),
            model_id: Some(self.model_id.clone()),
            ..Default::default()
        });
        let stream = crate::google::transcription::live_stream(
            socket,
            options.audio,
            options.include_raw_chunks,
            "Vertex",
        );
        Ok(TranscriptionStreamResult {
            stream,
            request,
            response,
        })
    }
}
