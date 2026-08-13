//! Prodia image provider.
//!
//! Aligned with Vercel AI SDK `ProdiaImageModel`
//! (`reference/ai/packages/prodia/src/prodia-image-model.ts`).
//!
//! POST to `/job?price=true`, returns a multipart response with a JSON "job"
//! part and a binary "output" image part.

use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::error::{AiMuxError, ApiCallError};
use aimux_core::image_model::{
    ImageCallOptions, ImageModel, ImageOutputs, ImageResponse, ImageResult,
};
use aimux_core::shared::Warning;
use aimux_provider_utils::response::DEFAULT_ERROR_STRUCTURE;
use aimux_provider_utils::{
    HttpBody, HttpMethod, HttpRequest, RetryConfig, load_api_key, send, sleep_or_abort,
    without_trailing_slash,
};

/// Configuration for the Prodia provider.
#[derive(Debug, Clone)]
pub struct ProdiaConfig {
    pub api_key: String,
    pub base_url: String,
    pub headers: Option<HashMap<String, String>>,
}

impl ProdiaConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://prodia.com/api".to_string(),
            headers: None,
        }
    }
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = without_trailing_slash(&url.into());
        self
    }
    pub fn with_headers(mut self, headers: HashMap<String, String>) -> Self {
        self.headers = Some(headers);
        self
    }
    pub fn from_env() -> Result<Self, AiMuxError> {
        let api_key = load_api_key(None, "PRODIA_API_KEY", "Prodia")?;
        Ok(Self::new(api_key))
    }
}

pub struct ProdiaProvider {
    config: ProdiaConfig,
}
impl ProdiaProvider {
    pub fn new(config: ProdiaConfig) -> Self {
        Self { config }
    }
    pub fn image(&self, model_id: &str) -> ProdiaImageModel {
        ProdiaImageModel::new(model_id.to_string(), self.config.clone())
    }
    /// Create a video generation model instance.
    pub fn video(&self, model_id: &str) -> ProdiaVideoModel {
        ProdiaVideoModel::new(model_id.to_string(), self.config.clone())
    }
}

/// A Prodia image generation model.
pub struct ProdiaImageModel {
    model_id: String,
    config: ProdiaConfig,
}
impl ProdiaImageModel {
    pub fn new(model_id: String, config: ProdiaConfig) -> Self {
        Self { model_id, config }
    }

    fn build_headers(&self, extra: Option<&HashMap<String, String>>) -> HashMap<String, String> {
        let mut h = HashMap::new();
        h.insert("X-Prodia-Key".into(), self.config.api_key.clone());
        h.insert("Accept".into(), "multipart/form-data; image/png".into());
        if let Some(ref ch) = self.config.headers {
            for (k, v) in ch {
                h.insert(k.clone(), v.clone());
            }
        }
        if let Some(extra) = extra {
            for (k, v) in extra {
                h.insert(k.clone(), v.clone());
            }
        }
        h
    }

    fn endpoint(&self) -> String {
        format!("{}/job?price=true", self.config.base_url)
    }
}

/// Parse a multipart body into parts (name, content_type, body).
fn parse_multipart(body: &[u8], boundary: &str) -> Vec<(String, String, Vec<u8>)> {
    let delimiter = format!("--{}", boundary);
    let mut parts = Vec::new();
    let mut idx = 0;

    while let Some(start) = body[idx..]
        .windows(delimiter.len())
        .position(|w| w == delimiter.as_bytes())
    {
        let abs_start = idx + start + delimiter.len();
        if abs_start >= body.len() {
            break;
        }
        // Skip CRLF after boundary
        let content_start = if body[abs_start..].starts_with(b"\r\n") {
            abs_start + 2
        } else {
            abs_start
        };

        // Find next boundary
        let next_delimiter = format!("\r\n{}", delimiter);
        let end = body[content_start..]
            .windows(next_delimiter.len())
            .position(|w| w == next_delimiter.as_bytes())
            .map(|p| content_start + p)
            .unwrap_or(body.len());

        let part = &body[content_start..end];

        // Parse headers
        let header_end = part
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .unwrap_or(part.len());
        let header_str = String::from_utf8_lossy(&part[..header_end]);
        let body_start = if header_end + 4 <= part.len() {
            header_end + 4
        } else {
            part.len()
        };

        let mut name = String::new();
        let mut content_type = String::new();
        for line in header_str.lines() {
            if line.to_lowercase().starts_with("content-disposition:")
                && let Some(n) = line.find("name=\"")
                && let Some(end) = line[n + 6..].find('"')
            {
                name = line[n + 6..n + 6 + end].to_string();
            }
            if line.to_lowercase().starts_with("content-type:") {
                content_type = line[13..].trim().to_string();
            }
        }

        parts.push((name, content_type, part[body_start..].to_vec()));
        idx = end;
    }

    parts
}

#[async_trait]
impl ImageModel for ProdiaImageModel {
    fn provider(&self) -> &str {
        "prodia"
    }
    fn model_id(&self) -> &str {
        &self.model_id
    }
    fn max_images_per_call(&self) -> Option<u32> {
        Some(1)
    }

    async fn do_generate(&self, options: &ImageCallOptions) -> Result<ImageResult, AiMuxError> {
        let warnings: Vec<Warning> = Vec::new();

        let prodia_opts = options.provider_options.get("prodia");

        // Build job config
        let mut job_config = Map::new();
        if let Some(ref p) = options.prompt {
            job_config.insert("prompt".into(), json!(p));
        }

        // width/height
        if let Some(w) = prodia_opts.and_then(|o| o.get("width")) {
            job_config.insert("width".into(), w.clone());
        } else if let Some(s) = options.size {
            job_config.insert("width".into(), json!(s.width()));
        }
        if let Some(h) = prodia_opts.and_then(|o| o.get("height")) {
            job_config.insert("height".into(), h.clone());
        } else if let Some(s) = options.size {
            job_config.insert("height".into(), json!(s.height()));
        }

        if let Some(seed) = options.seed {
            job_config.insert("seed".into(), json!(seed));
        }
        if let Some(v) = prodia_opts.and_then(|o| o.get("steps")) {
            job_config.insert("steps".into(), v.clone());
        }
        if let Some(v) = prodia_opts.and_then(|o| o.get("stylePreset")) {
            job_config.insert("style_preset".into(), v.clone());
        }
        if let Some(v) = prodia_opts.and_then(|o| o.get("loras")) {
            job_config.insert("loras".into(), v.clone());
        }
        if let Some(v) = prodia_opts.and_then(|o| o.get("progressive")) {
            job_config.insert("progressive".into(), v.clone());
        }

        let body = json!({ "type": self.model_id, "config": job_config });

        let headers = self.build_headers(options.headers.as_ref());
        let header_list: Vec<(String, String)> = headers.into_iter().collect();

        let resp = send(
            HttpRequest {
                method: HttpMethod::Post,
                url: self.endpoint(),
                headers: header_list,
                body: HttpBody::Json(body),

                abort_signal: options.abort_signal.clone(),
                call_id: None,
                recording_context: None,
            },
            RetryConfig::default(),
            &DEFAULT_ERROR_STRUCTURE,
        )
        .await?;

        let rh = resp.headers;
        let content_type = rh.get("content-type").cloned().unwrap_or_default();

        // Extract boundary
        let boundary = content_type
            .split(';')
            .find_map(|p| {
                let p = p.trim();
                p.strip_prefix("boundary=").map(|s| s.trim().to_string())
            })
            .ok_or_else(|| {
                AiMuxError::InvalidResponseData(format!(
                    "Prodia response missing multipart boundary: {content_type}"
                ))
            })?;

        let body_bytes = resp.body.to_vec();

        // Parse multipart
        let parts = parse_multipart(&body_bytes, &boundary);

        let mut job_result: Option<Value> = None;
        let mut image_bytes: Option<Vec<u8>> = None;

        for (name, part_content_type, part_body) in parts {
            if name == "job" || name.contains("job") {
                let json_str = String::from_utf8_lossy(&part_body);
                job_result = serde_json::from_str(&json_str).ok();
            } else if name == "output"
                || name.contains("output")
                || part_content_type.starts_with("image/")
            {
                image_bytes = Some(part_body);
            }
        }

        let image_bytes = image_bytes.ok_or_else(|| {
            AiMuxError::InvalidResponseData("Prodia multipart response missing output image".into())
        })?;
        let job_result = job_result.unwrap_or(Value::Null);

        // Build provider metadata
        let mut metadata = HashMap::new();
        let mut prodia_meta = Map::new();
        prodia_meta.insert("images".into(), json!([job_result]));
        metadata.insert("prodia".into(), Value::Object(prodia_meta));

        Ok(ImageResult {
            images: ImageOutputs::Binary(vec![image_bytes]),
            warnings,
            provider_metadata: Some(metadata),
            response: ImageResponse {
                timestamp: Some(chrono::Utc::now().to_rfc3339()),
                model_id: Some(self.model_id.clone()),
                headers: Some(rh),
            },
            usage: None,
        })
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Video model
// ════════════════════════════════════════════════════════════════════════════

use aimux_core::video_model::{
    VideoCallOptions, VideoData, VideoModel, VideoResponse, VideoResult,
};

/// Prodia video generation model — implements `VideoModel`.
///
/// Aligned with Vercel AI SDK `ProdiaVideoModel`
/// (`reference/ai/packages/prodia/src/prodia-video-model.ts`).
pub struct ProdiaVideoModel {
    model_id: String,
    config: ProdiaConfig,
}

impl ProdiaVideoModel {
    pub fn new(model_id: String, config: ProdiaConfig) -> Self {
        Self { model_id, config }
    }

    fn build_headers(&self, extra: Option<&HashMap<String, String>>) -> HashMap<String, String> {
        let mut h = HashMap::new();
        h.insert("X-Prodia-Key".to_string(), self.config.api_key.clone());
        if let Some(ref ch) = self.config.headers {
            for (k, v) in ch {
                h.insert(k.clone(), v.clone());
            }
        }
        if let Some(extra) = extra {
            for (k, v) in extra {
                h.insert(k.clone(), v.clone());
            }
        }
        h
    }
}

#[async_trait]
impl VideoModel for ProdiaVideoModel {
    fn provider(&self) -> &str {
        "prodia"
    }
    fn model_id(&self) -> &str {
        &self.model_id
    }
    fn max_videos_per_call(&self) -> Option<u32> {
        Some(1)
    }

    async fn do_generate(&self, options: &VideoCallOptions) -> Result<VideoResult, AiMuxError> {
        let warnings: Vec<Warning> = Vec::new();

        let mut config_obj = Map::new();
        if let Some(ref prompt) = options.prompt {
            config_obj.insert("prompt".to_string(), json!(prompt));
        }
        if let Some(seed) = options.seed {
            config_obj.insert("seed".to_string(), json!(seed));
        }

        let body = json!({"type": self.model_id, "config": Value::Object(config_obj)});

        let headers = self.build_headers(options.headers.as_ref());
        let header_list: Vec<(String, String)> = headers.into_iter().collect();

        // Submit job.
        let resp = send(
            HttpRequest {
                method: HttpMethod::Post,
                url: format!("{}/job", self.config.base_url),
                headers: header_list.clone(),
                body: HttpBody::Json(body),

                abort_signal: options.abort_signal.clone(),
                call_id: None,
                recording_context: None,
            },
            RetryConfig::default(),
            &DEFAULT_ERROR_STRUCTURE,
        )
        .await?;

        let job: Value = serde_json::from_slice(&resp.body)?;
        let job_id = job
            .get("job")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                AiMuxError::InvalidResponseData(
                    "Prodia job submission response missing job id".to_string(),
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
                    url: format!("{}/job/{}", self.config.base_url, job_id),
                    headers: header_list.clone(),
                    body: HttpBody::Empty,

                    abort_signal: options.abort_signal.clone(),
                    call_id: None,
                    recording_context: None,
                },
                RetryConfig::default(),
                &DEFAULT_ERROR_STRUCTURE,
            )
            .await?;

            response_headers = resp.headers;
            raw_body = serde_json::from_slice(&resp.body)?;
            let status_str = raw_body
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if status_str == "done" || status_str == "failed" {
                if status_str == "failed" {
                    return Err(AiMuxError::ApiCall(ApiCallError {
                        status_code: Some(resp.status),
                        provider_code: Some(status_str.to_string()),
                        message: "Prodia video generation failed".to_string(),
                        response_body: Some(String::from_utf8_lossy(&resp.body).into_owned()),
                        ..Default::default()
                    }));
                }
                break;
            }
        }

        // Extract video URL.
        let video_url = raw_body
            .get("videoUrl")
            .or_else(|| raw_body.get("video_url"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                AiMuxError::InvalidResponseData("Prodia job done without a video URL".to_string())
            })?;
        let videos: Vec<VideoData> = vec![VideoData::Url {
            url: video_url.to_string(),
            media_type: "video/mp4".to_string(),
        }];

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
