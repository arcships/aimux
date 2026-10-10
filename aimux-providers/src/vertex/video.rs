//! Google Vertex AI video generation model — implements `VideoModel`.
//!
//! Aligned with Vercel AI SDK `GoogleVertexVideoModel`
//! (`reference/ai/packages/google-vertex/src/google-vertex-video-model.ts`).
//!
//! Uses the Long Running Operations API:
//! 1. POST `{base_url}/models/{model}:predictLongRunning` → returns operation name
//! 2. POST fetchPredictOperation — polled by Core until `done: true`
//! 3. Return video URL(s)

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::error::{AiMuxError, ApiCallError};
use aimux_core::video_model::{
    VideoCallOptions, VideoData, VideoModel, VideoOperationStart, VideoOperationStatus,
    VideoResponse, VideoResult,
};

use crate::google::options::Namespace;
use crate::shared::EndpointConfig;
use aimux_core::shared::{Warning, provider_namespace};
use aimux_core::video_model::{VideoFile, VideoFileData, VideoFrameType};

/// A Google Vertex AI video generation model.
///
/// Does **not** hold an HTTP client — the `aimux-provider-utils` API helpers use the process-wide shared
/// `Client` internally (RFC-0009 §4.1).
pub struct VertexVideoModel {
    model_id: String,
    config: EndpointConfig,
}

impl VertexVideoModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }
}

fn convert_image(file: &VideoFile, warnings: &mut Vec<Warning>) -> Option<Value> {
    match file {
        VideoFile::Url { url, .. } if url.starts_with("gs://") => {
            Some(json!({"gcsUri": url, "mimeType": "image/png"}))
        }
        VideoFile::Url { .. } => {
            warnings.push(Warning::Unsupported {
                feature: "URL-based image input".into(),
                details: Some("Vertex AI video models require base64-encoded images or GCS URIs. URL will be ignored.".into()),
            });
            None
        }
        VideoFile::File { media_type, data } => {
            let data = match data {
                VideoFileData::Base64(data) => data.clone(),
                VideoFileData::Binary(data) => {
                    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, data)
                }
            };
            Some(
                json!({"bytesBase64Encoded": data, "mimeType": if media_type.is_empty() { "image/png" } else { media_type }}),
            )
        }
    }
}

#[async_trait]
impl VideoModel for VertexVideoModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }
    fn model_id(&self) -> &str {
        &self.model_id
    }
    fn max_videos_per_call(&self) -> Option<u32> {
        Some(4)
    }

    async fn do_start(
        &self,
        options: &VideoCallOptions,
    ) -> Result<VideoOperationStart, AiMuxError> {
        let mut warnings = Vec::new();
        let provider_options = Namespace::Vertex
            .write_keys()
            .iter()
            .find_map(|key| options.provider_options.get(*key));
        let mut instance = Map::new();
        if let Some(prompt) = &options.prompt {
            instance.insert("prompt".into(), json!(prompt));
        }
        let frames = options.frame_images.as_deref().unwrap_or(&[]);
        let start_image = frames
            .iter()
            .find(|f| f.frame_type == VideoFrameType::FirstFrame)
            .map(|f| &f.image)
            .or(options.image.as_ref());
        if let Some(image) = start_image.and_then(|file| convert_image(file, &mut warnings)) {
            instance.insert("image".into(), image);
        }
        if let Some(image) = frames
            .iter()
            .find(|f| f.frame_type == VideoFrameType::LastFrame)
            .and_then(|f| convert_image(&f.image, &mut warnings))
        {
            instance.insert("lastFrame".into(), image);
        }
        if frames.is_empty()
            && let Some(references) = &options.input_references
            && !references.is_empty()
        {
            instance.insert(
                "referenceImages".into(),
                Value::Array(
                    references
                        .iter()
                        .filter_map(|file| convert_image(file, &mut warnings))
                        .map(|image| json!({"image": image, "referenceType": "asset"}))
                        .collect(),
                ),
            );
        } else if let Some(references) = provider_options
            .and_then(|p| p.get("referenceImages"))
            .filter(|value| !value.is_null())
        {
            instance.insert("referenceImages".into(), references.clone());
        }
        let instances = vec![Value::Object(instance)];
        let mut parameters = Map::new();
        parameters.insert("sampleCount".into(), json!(options.n));
        if let Some(resolution) = options.resolution {
            let resolution = resolution.to_string();
            let mapped = match resolution.as_str() {
                "1280x720" => "720p",
                "1920x1080" => "1080p",
                "3840x2160" => "4k",
                _ => &resolution,
            };
            parameters.insert("resolution".into(), json!(mapped));
        }
        if let Some(ar) = options.aspect_ratio {
            parameters.insert("aspectRatio".to_string(), json!(ar.to_string()));
        }
        if let Some(seed) = options.seed.filter(|seed| *seed != 0) {
            parameters.insert("seed".to_string(), json!(seed));
        }
        if let Some(duration) = options.duration.filter(|duration| *duration != 0) {
            parameters.insert("durationSeconds".to_string(), json!(duration));
        }
        if let Some(ga) = options.generate_audio {
            parameters.insert("generateAudio".to_string(), json!(ga));
        }

        if let Some(provider_options) = provider_options {
            for (key, value) in provider_options {
                if ![
                    "pollIntervalMs",
                    "pollTimeoutMs",
                    "referenceImages",
                    "generateAudio",
                ]
                .contains(&key.as_str())
                    && (!value.is_null()
                        || !["personGeneration", "negativePrompt", "gcsOutputDirectory"]
                            .contains(&key.as_str()))
                {
                    parameters.insert(key.clone(), value.clone());
                }
            }
            if options.generate_audio.is_none()
                && let Some(value) = provider_options.get("generateAudio")
                && !value.is_null()
            {
                parameters.insert("generateAudio".into(), value.clone());
            }
        }

        let body = json!({
            "instances": instances,
            "parameters": Value::Object(parameters),
        });

        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let url = exchange.url(&format!("/models/{}:predictLongRunning", self.model_id));

        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(url, options),
            body,
            aimux_provider_utils::create_json_response_handler(),
            crate::google::google_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;
        let predict_response: Value = resp.value;
        let operation_name = predict_response
            .get("name")
            .and_then(|v| v.as_str())
            .filter(|name| !name.is_empty())
            .ok_or_else(|| {
                AiMuxError::InvalidResponseData("No operation name returned from API".to_string())
            })?
            .to_string();

        Ok(VideoOperationStart {
            operation: json!({ "operationName": operation_name }),
            warnings,
            provider_metadata: None,
            response: VideoResponse {
                timestamp: Some(chrono::Utc::now().to_rfc3339()),
                model_id: Some(self.model_id.clone()),
                headers: Some(response_headers),
            },
        })
    }

    async fn do_status(
        &self,
        operation: &Value,
        options: &VideoCallOptions,
    ) -> Result<VideoOperationStatus, AiMuxError> {
        let operation_name = operation
            .get("operationName")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                AiMuxError::InvalidArgument(
                    "google.vertex operation reference is missing operationName".to_string(),
                )
            })?;

        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let poll_url = exchange.url(&format!("/models/{}:fetchPredictOperation", self.model_id));
        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(poll_url.clone(), options),
            json!({"operationName": operation_name}),
            aimux_provider_utils::create_json_response_handler::<Value>(),
            crate::google::google_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;
        let response_body = resp.raw_value.as_ref().map(ToString::to_string);
        let raw_body: Value = resp.value;
        if raw_body.get("done").and_then(serde_json::Value::as_bool) != Some(true) {
            return Ok(VideoOperationStatus::Pending);
        }
        if let Some(err) = raw_body.get("error").filter(|value| !value.is_null()) {
            let msg = err
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown error");
            return Err(AiMuxError::ApiCall(Box::new(ApiCallError {
                status_code: Some(200),
                provider_code: err
                    .get("status")
                    .and_then(|v| v.as_str())
                    .map(std::string::ToString::to_string),
                response_body,
                ..ApiCallError::new(msg, poll_url, serde_json::json!({}))
            })));
        }
        let outputs = raw_body
            .get("response")
            .and_then(|r| r.get("videos"))
            .and_then(Value::as_array)
            .filter(|outputs| !outputs.is_empty())
            .ok_or_else(|| {
                AiMuxError::InvalidResponseData(format!(
                    "No videos in response. Response: {raw_body}"
                ))
            })?;
        let mut video_metadata = Vec::new();
        let videos: Vec<VideoData> = outputs
            .iter()
            .filter_map(|video| {
                let media_type = video
                    .get("mimeType")
                    .and_then(Value::as_str)
                    .filter(|mime| !mime.is_empty())
                    .unwrap_or("video/mp4")
                    .to_string();
                if let Some(data) = video
                    .get("bytesBase64Encoded")
                    .and_then(Value::as_str)
                    .filter(|data| !data.is_empty())
                {
                    let mut metadata = Map::new();
                    if let Some(mime) = video.get("mimeType") {
                        metadata.insert("mimeType".into(), mime.clone());
                    }
                    video_metadata.push(Value::Object(metadata));
                    Some(VideoData::Base64 {
                        data: data.to_string(),
                        media_type,
                    })
                } else if let Some(url) = video
                    .get("gcsUri")
                    .and_then(Value::as_str)
                    .filter(|url| !url.is_empty())
                {
                    let mut metadata = Map::new();
                    metadata.insert("gcsUri".into(), json!(url));
                    if let Some(mime) = video.get("mimeType") {
                        metadata.insert("mimeType".into(), mime.clone());
                    }
                    video_metadata.push(Value::Object(metadata));
                    Some(VideoData::Url {
                        url: url.to_string(),
                        media_type,
                    })
                } else {
                    None
                }
            })
            .collect();
        let payload = json!({"videos": video_metadata});
        let metadata = crate::google::options::VERTEX_VIDEO_METADATA_KEYS
            .into_iter()
            .flat_map(|key| {
                provider_namespace(key, payload.clone())
                    .expect("provider metadata must be an object")
            })
            .collect();

        if videos.is_empty() {
            return Err(AiMuxError::InvalidResponseData(
                "No valid videos in response".to_string(),
            ));
        }

        Ok(VideoOperationStatus::Completed(VideoResult {
            videos,
            warnings: Vec::new(),
            provider_metadata: Some(metadata),
            response: VideoResponse {
                timestamp: Some(chrono::Utc::now().to_rfc3339()),
                model_id: Some(self.model_id.clone()),
                headers: Some(response_headers),
            },
        }))
    }
}
