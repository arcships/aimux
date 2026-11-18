//! Replicate video generation model.

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::shared::Warning;
use aimux_core::video_model::{
    VideoCallOptions, VideoData, VideoFile, VideoModel, VideoOperationStart, VideoOperationStatus,
    VideoResult,
};

use crate::shared::{EndpointConfig, task_failed, video_response};

use super::provider::replicate_failed_response_handler;

/// Replicate video generation model — implements `VideoModel`.
///
/// Aligned with Vercel AI SDK `ReplicateVideoModel`
/// (`reference/ai/packages/replicate/src/replicate-video-model.ts`).
///
/// Uses the predictions API: POST to create (`do_start`); Core drives the
/// status polling via `do_status`.
pub struct ReplicateVideoModel {
    model_id: String,
    config: EndpointConfig,
}

impl ReplicateVideoModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }
}

fn video_file_to_url(file: &VideoFile) -> Result<String, AiMuxError> {
    match file {
        VideoFile::Url { url, .. } => Ok(url.clone()),
        VideoFile::File { media_type, data } => Ok(format!(
            "data:{media_type};base64,{}",
            aimux_provider_utils::convert_to_base64(data)
        )),
    }
}

#[async_trait]
impl VideoModel for ReplicateVideoModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }
    fn model_id(&self) -> &str {
        &self.model_id
    }
    fn max_videos_per_call(&self) -> Option<u32> {
        Some(1)
    }

    async fn do_start(
        &self,
        options: &VideoCallOptions,
    ) -> Result<VideoOperationStart, AiMuxError> {
        let warnings: Vec<Warning> = Vec::new();

        let mut input = Map::new();
        if let Some(ref prompt) = options.prompt {
            input.insert("prompt".to_string(), json!(prompt));
        }
        if let Some(ref image) = options.image {
            input.insert("image".to_string(), json!(video_file_to_url(image)?));
        }
        if let Some(seed) = options.seed {
            input.insert("seed".to_string(), json!(seed));
        }

        let body = json!({
            "model": self.model_id,
            "input": Value::Object(input),
        });

        let exchange = self.config.exchange(options.headers.as_ref()).await?;

        // Submit prediction.
        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(exchange.url("/predictions"), options),
            body,
            aimux_provider_utils::create_json_response_handler(),
            replicate_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;
        let prediction: Value = resp.value;
        let prediction_id = prediction
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AiMuxError::invalid_response_data("Replicate prediction missing id"))?
            .to_string();

        Ok(VideoOperationStart {
            operation: json!({ "prediction_id": prediction_id }),
            warnings,
            provider_metadata: None,
            response: video_response(
                aimux_core::Timestamp::now(),
                &self.model_id,
                response_headers,
            ),
        })
    }

    async fn do_status(
        &self,
        operation: &Value,
        options: &VideoCallOptions,
    ) -> Result<VideoOperationStatus, AiMuxError> {
        let prediction_id = operation
            .get("prediction_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                AiMuxError::invalid_argument(
                    "replicate operation reference is missing prediction_id".to_string(),
                )
            })?;

        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let poll_url = exchange.url(&format!("/predictions/{prediction_id}"));

        let resp = aimux_provider_utils::get_from_api(
            exchange.request(poll_url.clone(), options),
            aimux_provider_utils::create_json_response_handler::<Value>(),
            replicate_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;
        let response_body = resp.raw_value.as_ref().map(ToString::to_string);
        let raw_body = resp.value;
        let status_str = raw_body
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        match status_str {
            "succeeded" => {}
            // A terminally failed prediction must be a non-retryable error,
            // not Pending, so the Core poll loop stops immediately.
            "failed" | "canceled" => {
                return Err(task_failed(
                    format!("Replicate prediction {status_str}"),
                    poll_url,
                    status_str.to_string(),
                    response_body,
                ));
            }
            // starting / processing / unknown — keep polling.
            _ => return Ok(VideoOperationStatus::Pending),
        }

        // Extract video from output.
        let videos: Vec<VideoData> =
            if let Some(url) = raw_body.get("output").and_then(|v| v.as_str()) {
                vec![VideoData::Url {
                    url: url.to_string(),
                    media_type: "video/mp4".to_string(),
                }]
            } else if let Some(arr) = raw_body.get("output").and_then(|v| v.as_array()) {
                arr.iter()
                    .filter_map(|v| {
                        v.as_str().map(|url| VideoData::Url {
                            url: url.to_string(),
                            media_type: "video/mp4".to_string(),
                        })
                    })
                    .collect()
            } else {
                vec![]
            };
        if videos.is_empty() {
            return Err(AiMuxError::invalid_response_data(
                "Replicate prediction succeeded without a usable output URL".to_string(),
            ));
        }

        Ok(VideoOperationStatus::Completed(VideoResult {
            videos,
            warnings: Vec::new(),
            provider_metadata: None,
            response: video_response(
                aimux_core::Timestamp::now(),
                &self.model_id,
                response_headers,
            ),
        }))
    }
}
