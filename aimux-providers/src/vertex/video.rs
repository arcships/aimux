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
use reqwest::Client;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::shared::Warning;
use aimux_core::video_model::{
    VideoCallOptions, VideoData, VideoModel, VideoResponse, VideoResult,
};

use aimux_provider_utils::response::{ErrorStructure, parse_provider_error};

use super::VertexAuth;

const GOOGLE_ERROR_STRUCTURE: ErrorStructure = ErrorStructure {
    message_path: &["error", "message"],
    type_path: &["error", "status"],
};

pub struct VertexVideoModel {
    model_id: String,
    project: String,
    location: String,
    auth: VertexAuth,
    base_url: String,
    client: Client,
}

impl VertexVideoModel {
    pub fn new(
        model_id: String,
        project: String,
        location: String,
        auth: VertexAuth,
        base_url: String,
        client: Client,
    ) -> Self {
        Self {
            model_id,
            project,
            location,
            auth,
            base_url,
            client,
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
        let header_map: reqwest::header::HeaderMap = headers
            .iter()
            .filter_map(|(k, v)| {
                reqwest::header::HeaderName::try_from(k)
                    .ok()
                    .zip(reqwest::header::HeaderValue::try_from(v).ok())
            })
            .collect();

        let resp = self
            .client
            .post(self.predict_url())
            .header("Content-Type", "application/json")
            .headers(header_map.clone())
            .json(&body)
            .send()
            .await
            .map_err(|e| AiMuxError::Http(e.to_string()))?;

        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(parse_provider_error(
                status.as_u16(),
                &text,
                &GOOGLE_ERROR_STRUCTURE,
            ));
        }

        let predict_response: Value = serde_json::from_str(&text).map_err(AiMuxError::Json)?;
        let operation_name = predict_response
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                AiMuxError::Provider("Vertex video prediction missing operation name".to_string())
            })?
            .to_string();

        // Poll for completion.
        let mut raw_body: Value;
        let mut response_headers: HashMap<String, String>;
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;

            let resp = self
                .client
                .get(self.operation_url(&operation_name))
                .headers(header_map.clone())
                .send()
                .await
                .map_err(|e| AiMuxError::Http(e.to_string()))?;

            let status = resp.status();
            response_headers = resp
                .headers()
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
                .collect();
            let text = resp.text().await.unwrap_or_default();
            if !status.is_success() {
                return Err(parse_provider_error(
                    status.as_u16(),
                    &text,
                    &GOOGLE_ERROR_STRUCTURE,
                ));
            }

            raw_body = serde_json::from_str(&text).unwrap_or(Value::Null);
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
