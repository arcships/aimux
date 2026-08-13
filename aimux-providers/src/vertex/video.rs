//! Google Vertex AI video generation model — implements `VideoModel`.
//!
//! Aligned with Vercel AI SDK `GoogleVertexVideoModel`
//! (`reference/ai/packages/google-vertex/src/google-vertex-video-model.ts`).
//!
//! Uses the Long Running Operations API:
//! 1. POST `{base_url}/models/{model}:predictLongRunning` → returns operation name
//! 2. GET operation to poll until `done: true`
//! 3. Return video URL(s)

use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::error::{AiMuxError, ApiCallError};
use aimux_core::shared::Warning;
use aimux_core::video_model::{
    VideoCallOptions, VideoData, VideoModel, VideoResponse, VideoResult,
};

use aimux_provider_utils::response::ErrorStructure;
use aimux_provider_utils::{HttpBody, HttpMethod, HttpRequest, RetryConfig, send, sleep_or_abort};

use super::VertexAuth;

const GOOGLE_ERROR_STRUCTURE: ErrorStructure = ErrorStructure {
    message_path: &["error", "message"],
    type_path: &["error", "status"],
};

/// A Google Vertex AI video generation model.
///
/// Does **not** hold an HTTP client — `http::send` uses the process-wide shared
/// `Client` internally (RFC-0009 §4.1).
pub struct VertexVideoModel {
    model_id: String,
    project: String,
    location: String,
    auth: VertexAuth,
    base_url: String,
}

impl VertexVideoModel {
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
        }
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
    fn max_videos_per_call(&self) -> Option<u32> {
        Some(1)
    }

    async fn do_generate(&self, options: &VideoCallOptions) -> Result<VideoResult, AiMuxError> {
        let warnings: Vec<Warning> = Vec::new();

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

        let resp = send(
            HttpRequest {
                method: HttpMethod::Post,
                url: self.predict_url(),
                headers: header_list.clone(),
                body: HttpBody::Json(body),

                abort_signal: options.abort_signal.clone(),
                call_id: None,
                recording_context: None,
            },
            RetryConfig::default(),
            &GOOGLE_ERROR_STRUCTURE,
        )
        .await?;

        let predict_response: Value = serde_json::from_slice(&resp.body)?;
        let operation_name = predict_response
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                AiMuxError::InvalidResponseData(
                    "Vertex video prediction missing operation name".to_string(),
                )
            })?
            .to_string();

        // Poll for completion.
        let mut raw_body: Value;
        let mut response_headers: HashMap<String, String>;
        loop {
            sleep_or_abort(
                std::time::Duration::from_millis(100),
                options.abort_signal.as_ref(),
            )
            .await?;

            let resp = send(
                HttpRequest {
                    method: HttpMethod::Get,
                    url: self.operation_url(&operation_name),
                    headers: header_list.clone(),
                    body: HttpBody::Empty,

                    abort_signal: options.abort_signal.clone(),
                    call_id: None,
                    recording_context: None,
                },
                RetryConfig::default(),
                &GOOGLE_ERROR_STRUCTURE,
            )
            .await?;

            response_headers = resp.headers;
            raw_body = serde_json::from_slice(&resp.body)?;
            // Check the in-band error first: a terminal response may carry both
            // done:true and an error object (provider-declared failure).
            if let Some(err) = raw_body.get("error") {
                let msg = err
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown error");
                return Err(AiMuxError::ApiCall(ApiCallError {
                    status_code: Some(resp.status),
                    provider_code: err
                        .get("status")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    message: msg.to_string(),
                    response_body: Some(String::from_utf8_lossy(&resp.body).into_owned()),
                    ..Default::default()
                }));
            }
            if raw_body.get("done").and_then(|v| v.as_bool()) == Some(true) {
                break;
            }
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

        Ok(VideoResult {
            videos,
            warnings,
            provider_metadata: None,
            response: VideoResponse {
                timestamp: Some(chrono::Utc::now().to_rfc3339()),
                model_id: Some(self.model_id.clone()),
                headers: Some(response_headers),
            },
        })
    }
}
