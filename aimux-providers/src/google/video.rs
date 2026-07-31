//! Google video generation model — implements `VideoModel`.
//!
//! Aligned with Vercel AI SDK `GoogleVideoModel`
//! (`reference/ai/packages/google/src/google-video-model.ts`).
//!
//! Uses Google's Long Running Operations API:
//! 1. POST `{base_url}/models/{model}:predictLongRunning` → returns operation name
//! 2. GET `{base_url}/{operation_name}` to poll until `done: true`
//! 3. Return video URL(s) from operation result

use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::shared::Warning;
use aimux_core::video_model::{
    VideoCallOptions, VideoData, VideoModel, VideoResponse, VideoResult,
};

use aimux_provider_utils::response::ErrorStructure;
use aimux_provider_utils::{HttpBody, HttpMethod, HttpRequest, RetryConfig, send};

use super::GoogleConfig;

const GOOGLE_ERROR_STRUCTURE: ErrorStructure = ErrorStructure {
    message_path: &["error", "message"],
    type_path: &["error", "status"],
};

/// A Google video generation model.
///
/// Does **not** hold an HTTP client — `http::send` uses the process-wide shared
/// `Client` internally (RFC-0009 §4.1).
pub struct GoogleVideoModel {
    model_id: String,
    config: GoogleConfig,
}

impl GoogleVideoModel {
    pub fn new(model_id: String, config: GoogleConfig) -> Self {
        Self { model_id, config }
    }

    fn build_headers(&self, extra: Option<&HashMap<String, String>>) -> HashMap<String, String> {
        let mut h = HashMap::new();
        h.insert("x-goog-api-key".to_string(), self.config.api_key.clone());
        if let Some(extra) = extra {
            for (k, v) in extra {
                h.insert(k.clone(), v.clone());
            }
        }
        h
    }

    fn predict_url(&self) -> String {
        format!(
            "{}/models/{}:predictLongRunning",
            self.config.base_url, self.model_id
        )
    }

    fn operation_url(&self, name: &str) -> String {
        format!("{}/{}", self.config.base_url, name)
    }
}

#[async_trait]
impl VideoModel for GoogleVideoModel {
    fn provider(&self) -> &str {
        "google"
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
        if let Some(fps) = options.fps {
            parameters.insert("fps".to_string(), json!(fps));
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

        let url = self.predict_url();

        let resp = send(
            HttpRequest {
                method: HttpMethod::Post,
                url,
                headers: header_list.clone(),
                body: HttpBody::Json(body),

                abort_signal: options.abort_signal.clone(),
            },
            RetryConfig::default(),
            &GOOGLE_ERROR_STRUCTURE,
        )
        .await?;

        let predict_response: Value =
            serde_json::from_slice(&resp.body).map_err(|e| AiMuxError::Json(e.to_string()))?;
        let operation_name = predict_response
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                AiMuxError::Provider("Google video prediction missing operation name".to_string())
            })?
            .to_string();

        // Poll for completion.
        let mut raw_body: Value;
        let mut response_headers: HashMap<String, String>;
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;

            let poll_url = self.operation_url(&operation_name);
            let resp = send(
                HttpRequest {
                    method: HttpMethod::Get,
                    url: poll_url,
                    headers: header_list.clone(),
                    body: HttpBody::Empty,

                    abort_signal: options.abort_signal.clone(),
                },
                RetryConfig::default(),
                &GOOGLE_ERROR_STRUCTURE,
            )
            .await?;

            response_headers = resp.headers;
            raw_body = serde_json::from_slice(&resp.body).unwrap_or(Value::Null);
            if raw_body.get("done").and_then(|v| v.as_bool()) == Some(true) {
                break;
            }
            if let Some(err) = raw_body.get("error") {
                let msg = err
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown error");
                return Err(AiMuxError::Provider(msg.to_string()));
            }
        }

        // Extract videos from response.
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
