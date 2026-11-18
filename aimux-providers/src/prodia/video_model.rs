//! Prodia video generation model.

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::shared::Warning;
use aimux_core::video_model::{
    VideoCallOptions, VideoData, VideoModel, VideoOperationStart, VideoOperationStatus, VideoResult,
};

use crate::shared::{EndpointConfig, task_failed, video_response};

use super::provider::prodia_failed_response_handler;

/// Prodia video generation model — implements `VideoModel`.
///
/// Aligned with Vercel AI SDK `ProdiaVideoModel`
/// (`reference/ai/packages/prodia/src/prodia-video-model.ts`).
pub struct ProdiaVideoModel {
    model_id: String,
    config: EndpointConfig,
}

impl ProdiaVideoModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }
}

#[async_trait]
impl VideoModel for ProdiaVideoModel {
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

        let mut config_obj = Map::new();
        if let Some(ref prompt) = options.prompt {
            config_obj.insert("prompt".to_string(), json!(prompt));
        }
        if let Some(seed) = options.seed {
            config_obj.insert("seed".to_string(), json!(seed));
        }

        let body = json!({"type": self.model_id, "config": Value::Object(config_obj)});

        let exchange = self.config.exchange(options.headers.as_ref()).await?;

        // Submit job.
        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(exchange.url("/job"), options),
            body,
            aimux_provider_utils::create_json_response_handler(),
            prodia_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;
        let job: Value = resp.value;
        let job_id = job
            .get("job")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                AiMuxError::invalid_response_data(
                    "Prodia job submission response missing job id".to_string(),
                )
            })?
            .to_string();

        Ok(VideoOperationStart {
            operation: json!({ "job_id": job_id }),
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
        let job_id = operation
            .get("job_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                AiMuxError::invalid_argument(
                    "prodia operation reference is missing job_id".to_string(),
                )
            })?;

        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let poll_url = exchange.url(&format!("/job/{job_id}"));

        let resp = aimux_provider_utils::get_from_api(
            exchange.request(poll_url.clone(), options),
            aimux_provider_utils::create_json_response_handler::<Value>(),
            prodia_failed_response_handler(),
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
            "done" => {}
            // A terminally failed job must be a non-retryable error, not
            // Pending, so the Core poll loop stops immediately.
            "failed" => {
                return Err(task_failed(
                    "Prodia video generation failed",
                    poll_url,
                    status_str.to_string(),
                    response_body,
                ));
            }
            // queued / running / unknown — keep polling.
            _ => return Ok(VideoOperationStatus::Pending),
        }

        // Extract video URL.
        let video_url = raw_body
            .get("videoUrl")
            .or_else(|| raw_body.get("video_url"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                AiMuxError::invalid_response_data("Prodia job done without a video URL")
            })?;
        let videos: Vec<VideoData> = vec![VideoData::Url {
            url: video_url.to_string(),
            media_type: "video/mp4".to_string(),
        }];

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
