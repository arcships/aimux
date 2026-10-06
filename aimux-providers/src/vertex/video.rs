//! Google Vertex AI video generation model — implements `VideoModel`.
//!
//! Aligned with Vercel AI SDK `GoogleVertexVideoModel`
//! (`reference/ai/packages/google-vertex/src/google-vertex-video-model.ts`).
//!
//! Uses the Long Running Operations API:
//! 1. POST `{base_url}/models/{model}:predictLongRunning` → returns operation name
//! 2. POST `:fetchPredictOperation` — polled by Core until `done: true`
//! 3. Return video URL(s)

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::error::{AiMuxError, ApiCallError};
use aimux_core::video_model::{
    VideoCallOptions, VideoData, VideoModel, VideoOperationStart, VideoOperationStatus,
    VideoResponse, VideoResult,
};

use crate::shared::EndpointConfig;

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
        let mut instances = vec![json!({"prompt": options.prompt})];
        if let Some(ref image) = options.image
            && let aimux_core::video_model::VideoFile::Url { url, .. } = image
        {
            instances[0]["image"] = json!({"gcsUri": url, "mimeType": "image/png"});
        }

        let mut parameters = Map::new();
        parameters.insert("sampleCount".to_string(), json!(options.n));
        if let Some(ar) = options.aspect_ratio {
            parameters.insert("aspectRatio".to_string(), json!(ar.to_string()));
        }
        if let Some(seed) = options.seed {
            parameters.insert("seed".to_string(), json!(seed));
        }
        if let Some(duration) = options.duration {
            parameters.insert("durationSeconds".to_string(), json!(duration));
        }
        if let Some(ga) = options.generate_audio {
            parameters.insert("generateAudio".to_string(), json!(ga));
        }

        let body = json!({
            "instances": instances,
            "parameters": Value::Object(parameters),
        });

        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let url = exchange.url(&format!("/models/{}:predictLongRunning", self.model_id));

        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(url, options),
            exchange.transform_body(body),
            aimux_provider_utils::create_json_response_handler(),
            crate::google::google_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;
        let predict_response: Value = resp.value;
        let operation_name = predict_response
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                AiMuxError::InvalidResponseData("No operation name returned from API".to_string())
            })?
            .to_string();

        Ok(VideoOperationStart {
            operation: json!({ "operation_name": operation_name }),
            warnings: Vec::new(),
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
            .get("operation_name")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                AiMuxError::InvalidArgument(
                    "google.vertex operation reference is missing operation_name".to_string(),
                )
            })?;

        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let poll_url = exchange.url(&format!("/models/{}:fetchPredictOperation", self.model_id));
        let body = json!({"operationName": operation_name});
        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(poll_url.clone(), options),
            exchange.transform_body(body),
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
        if let Some(err) = raw_body.get("error") {
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

        let mut videos = Vec::new();
        let mut video_metadata = Vec::new();
        let outputs = raw_body
            .get("response")
            .and_then(|r| r.get("videos"))
            .and_then(Value::as_array)
            .ok_or_else(|| {
                AiMuxError::InvalidResponseData(format!(
                    "No videos in response. Response: {raw_body}"
                ))
            })?;
        if outputs.is_empty() {
            return Err(AiMuxError::InvalidResponseData(format!(
                "No videos in response. Response: {raw_body}"
            )));
        }
        for video in outputs {
            let mime_type = video.get("mimeType").cloned().unwrap_or(Value::Null);
            let media_type = mime_type
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or("video/mp4")
                .to_string();
            if let Some(data) = video
                .get("bytesBase64Encoded")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                videos.push(VideoData::Base64 {
                    data: data.to_string(),
                    media_type,
                });
                let mut metadata = Map::new();
                if video.get("mimeType").is_some() {
                    metadata.insert("mimeType".to_string(), mime_type);
                }
                video_metadata.push(Value::Object(metadata));
            } else if let Some(url) = video
                .get("gcsUri")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                videos.push(VideoData::Url {
                    url: url.to_string(),
                    media_type,
                });
                let mut metadata = Map::new();
                metadata.insert("gcsUri".to_string(), json!(url));
                if video.get("mimeType").is_some() {
                    metadata.insert("mimeType".to_string(), mime_type);
                }
                video_metadata.push(Value::Object(metadata));
            }
        }

        if videos.is_empty() {
            return Err(AiMuxError::InvalidResponseData(
                "No valid videos in response".to_string(),
            ));
        }

        Ok(VideoOperationStatus::Completed(VideoResult {
            videos,
            warnings: Vec::new(),
            provider_metadata: Some(
                ["googleVertex", "google-vertex", "vertex"]
                    .into_iter()
                    .map(|key| {
                        (
                            key.to_string(),
                            [("videos".to_string(), json!(video_metadata))]
                                .into_iter()
                                .collect(),
                        )
                    })
                    .collect(),
            ),
            response: VideoResponse {
                timestamp: Some(chrono::Utc::now().to_rfc3339()),
                model_id: Some(self.model_id.clone()),
                headers: Some(response_headers),
            },
        }))
    }
}
