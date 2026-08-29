//! Google Vertex AI video generation model — implements `VideoModel`.
//!
//! Aligned with Vercel AI SDK `GoogleVertexVideoModel`
//! (`reference/ai/packages/google-vertex/src/google-vertex-video-model.ts`).
//!
//! Uses the Long Running Operations API:
//! 1. POST `{base_url}/models/{model}:predictLongRunning` → returns operation name
//! 2. GET operation — polled by Core via `do_status` until `done: true`
//! 3. Return video URL(s)

use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::error::{AiMuxError, ApiCallError};
use aimux_core::video_model::{
    VideoCallOptions, VideoData, VideoModel, VideoOperationStart, VideoOperationStatus,
    VideoResponse, VideoResult,
};

use aimux_provider_utils::{HttpRequest, RetryConfig};

use super::VertexAuth;

/// A Google Vertex AI video generation model.
///
/// Does **not** hold an HTTP client — the `aimux-provider-utils` API helpers use the process-wide shared
/// `Client` internally (RFC-0009 §4.1).
pub struct VertexVideoModel {
    model_id: String,
    project: String,
    location: String,
    auth: VertexAuth,
    base_url: String,
    retry_config: RetryConfig,
}

impl VertexVideoModel {
    #[must_use]
    pub fn new(
        model_id: String,
        project: String,
        location: String,
        auth: VertexAuth,
        base_url: String,
    ) -> Self {
        Self {
            model_id,
            project,
            location,
            auth,
            base_url,
            retry_config: RetryConfig::default(),
        }
    }

    pub(crate) fn with_retry_config(mut self, retry_config: RetryConfig) -> Self {
        self.retry_config = retry_config;
        self
    }

    fn build_headers(&self, extra: Option<&HashMap<String, String>>) -> HashMap<String, String> {
        let mut h = HashMap::new();
        match &self.auth {
            VertexAuth::BearerToken(token) => {
                h.insert("Authorization".to_string(), format!("Bearer {token}"));
            }
            VertexAuth::ApiKey(key) => {
                h.insert("x-goog-api-key".to_string(), key.clone());
            }
        }
        if let Some(extra) = extra {
            for (k, v) in extra {
                h.insert(k.clone(), v.clone());
            }
        }
        h
    }

    fn predict_url(&self) -> String {
        if self.base_url.starts_with("http://127.0.0.1")
            || self.base_url.starts_with("http://localhost")
        {
            return format!(
                "{}/models/{}:predictLongRunning",
                self.base_url, self.model_id
            );
        }
        format!(
            "https://{}-aiplatform.googleapis.com/v1beta1/projects/{}/locations/{}/publishers/google/models/{}:predictLongRunning",
            self.location, self.project, self.location, self.model_id
        )
    }

    fn operation_url(&self, name: &str) -> String {
        if self.base_url.starts_with("http://127.0.0.1")
            || self.base_url.starts_with("http://localhost")
        {
            return format!("{}/{}", self.base_url, name);
        }
        format!(
            "https://{}-aiplatform.googleapis.com/v1beta1/{}",
            self.location, name
        )
    }
}

#[async_trait]
impl VideoModel for VertexVideoModel {
    fn provider(&self) -> &str {
        "google.vertex"
    }
    fn model_id(&self) -> &str {
        &self.model_id
    }
    fn retry_config(&self) -> aimux_core::retry::RetryConfig {
        self.retry_config
    }
    fn max_videos_per_call(&self) -> Option<u32> {
        Some(1)
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

        let headers = self.build_headers(options.headers.as_ref());
        let header_list: Vec<(String, String)> = headers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        let resp = aimux_provider_utils::post_json_to_api(
            HttpRequest {
                url: self.predict_url(),
                headers: header_list.clone(),

                abort_signal: options.abort_signal.clone(),
                call_id: None,
                recording_context: None,
                response_timeout: None,
                validate_url: false,
                trusted_origin: None,
                credentialed_origin: None,
            },
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
            .ok_or_else(|| {
                AiMuxError::InvalidResponseData(
                    "Vertex video prediction missing operation name".to_string(),
                )
            })?
            .to_string();

        Ok(VideoOperationStart {
            operation: json!({ "operation_name": operation_name }),
            warnings: Vec::new(),
            provider_metadata: None,
            response: VideoResponse {
                timestamp: Some(chrono::Utc::now().to_rfc3339()),
                model_id: Some(self.model_id.clone()),
                headers: response_headers,
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

        let headers = self.build_headers(options.headers.as_ref());
        let header_list: Vec<(String, String)> = headers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        let poll_url = self.operation_url(operation_name);
        let resp = aimux_provider_utils::get_from_api(
            HttpRequest {
                url: poll_url.clone(),
                headers: header_list,

                abort_signal: options.abort_signal.clone(),
                call_id: None,
                recording_context: None,
                response_timeout: None,
                validate_url: false,
                trusted_origin: None,
                credentialed_origin: None,
            },
            aimux_provider_utils::create_json_response_handler::<Value>(),
            crate::google::google_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers.unwrap_or_default();
        let response_body = resp.raw_value.as_ref().map(ToString::to_string);
        let raw_body: Value = resp.value;
        // Check the in-band error first: a terminal response may carry both
        // done:true and an error object (provider-declared failure).
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
        if raw_body.get("done").and_then(serde_json::Value::as_bool) != Some(true) {
            return Ok(VideoOperationStatus::Pending);
        }

        let videos: Vec<VideoData> = raw_body
            .get("response")
            .and_then(|r| r.get("videos"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| {
                        v.get("gcsUri")
                            .or_else(|| v.get("url"))
                            .and_then(|u| u.as_str())
                            .map(|url| VideoData::Url {
                                url: url.to_string(),
                                media_type: "video/mp4".to_string(),
                            })
                    })
                    .collect()
            })
            .unwrap_or_default();

        if videos.is_empty() {
            return Err(AiMuxError::InvalidResponseData(
                "Vertex video operation completed without any video output".to_string(),
            ));
        }

        Ok(VideoOperationStatus::Completed(VideoResult {
            videos,
            warnings: Vec::new(),
            provider_metadata: None,
            response: VideoResponse {
                timestamp: Some(chrono::Utc::now().to_rfc3339()),
                model_id: Some(self.model_id.clone()),
                headers: Some(response_headers),
            },
        }))
    }
}
