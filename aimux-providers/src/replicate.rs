//! Replicate image provider.
//!
//! Aligned with Vercel AI SDK `ReplicateImageModel`
//! (`reference/ai/packages/replicate/src/replicate-image-model.ts`).

use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::error::{AiMuxError, ApiCallError};
use aimux_core::image_model::{
    ImageCallOptions, ImageFile, ImageFileData, ImageModel, ImageOutputs, ImageResponse,
    ImageResult,
};
use aimux_core::retry;
use aimux_core::shared::Warning;
use aimux_provider_utils::{HttpRequest, load_api_key, without_trailing_slash};

/// Replicate's `prefer: wait` holds the connection for at most this long when
/// no explicit duration is given (their documented default and maximum).
const DEFAULT_WAIT_SECONDS: u64 = 60;

/// Headroom added on top of the wait window for connect/TLS/body transport
/// when sizing the create exchange's `response_timeout`.
const WAIT_TRANSPORT_MARGIN_SECONDS: u64 = 5;

fn replicate_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    aimux_provider_utils::create_json_error_response_handler(|data| {
        aimux_provider_utils::ProviderErrorParts {
            message: data
                .get("detail")
                .or_else(|| data.get("error"))
                .and_then(Value::as_str)
                .unwrap_or("Unknown Replicate error")
                .to_string(),
            provider_code: None,
        }
    })
}

/// Configuration for the Replicate provider.
#[derive(Debug, Clone)]
pub struct ReplicateConfig {
    pub api_key: String,
    pub base_url: String,
    pub headers: Option<HashMap<String, String>>,
}

impl ReplicateConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://api.replicate.com/v1".to_string(),
            headers: None,
        }
    }

    #[must_use]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = without_trailing_slash(&url.into());
        self
    }

    #[must_use]
    pub fn with_headers(mut self, headers: HashMap<String, String>) -> Self {
        self.headers = Some(headers);
        self
    }

    /// Create from the `REPLICATE_API_TOKEN` environment variable.
    ///
    /// # Errors
    ///
    /// Returns `AiMuxError::InvalidArgument` when the environment variable is not
    /// set.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let api_key = load_api_key(None, "REPLICATE_API_TOKEN", "Replicate")?;
        Ok(Self::new(api_key))
    }
}

/// Replicate provider — creates `ReplicateImageModel` instances.
pub struct ReplicateProvider {
    config: ReplicateConfig,
}

impl ReplicateProvider {
    #[must_use]
    pub fn new(config: ReplicateConfig) -> Self {
        Self { config }
    }

    #[must_use]
    pub fn image(&self, model_id: &str) -> ReplicateImageModel {
        ReplicateImageModel::new(model_id.to_string(), self.config.clone())
    }

    /// Create a video generation model instance for the given model name
    /// (e.g. `"wan-lab/wan-2.1-t2v-14b"`).
    #[must_use]
    pub fn video(&self, model_id: &str) -> ReplicateVideoModel {
        ReplicateVideoModel::new(model_id.to_string(), self.config.clone())
    }
}

/// Flux-2 model pattern for max input images.
const FLUX_2_PATTERN: &str = "black-forest-labs/flux-2-";
const MAX_FLUX_2_INPUT_IMAGES: u32 = 8;

/// A Replicate image generation model.
pub struct ReplicateImageModel {
    model_id: String,
    config: ReplicateConfig,
}

impl ReplicateImageModel {
    #[must_use]
    pub fn new(model_id: String, config: ReplicateConfig) -> Self {
        Self { model_id, config }
    }

    fn is_flux2(&self) -> bool {
        self.model_id.starts_with(FLUX_2_PATTERN)
    }

    fn build_headers(&self, extra: Option<&HashMap<String, String>>) -> HashMap<String, String> {
        let mut h = HashMap::new();
        h.insert(
            "Authorization".into(),
            format!("Bearer {}", self.config.api_key),
        );
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

    fn file_to_data_uri(file: &ImageFile) -> Result<String, AiMuxError> {
        match file {
            ImageFile::Url { url } => Ok(url.clone()),
            ImageFile::File { media_type, data } => {
                let b64 = match data {
                    ImageFileData::Base64(s) => s.clone(),
                    ImageFileData::Binary(b) => {
                        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, b)
                    }
                };
                Ok(format!("data:{media_type};base64,{b64}"))
            }
        }
    }

    fn endpoint(&self) -> String {
        if let Some((model_id, _version)) = self.model_id.split_once(':') {
            format!("{}/models/{}/predictions", self.config.base_url, model_id)
        } else {
            format!(
                "{}/models/{}/predictions",
                self.config.base_url, self.model_id
            )
        }
    }
}

#[async_trait]
impl ImageModel for ReplicateImageModel {
    fn provider(&self) -> &str {
        "replicate"
    }
    fn model_id(&self) -> &str {
        &self.model_id
    }
    fn max_images_per_call(&self) -> Option<u32> {
        if self.is_flux2() {
            Some(MAX_FLUX_2_INPUT_IMAGES)
        } else {
            Some(1)
        }
    }

    async fn do_generate(&self, options: &ImageCallOptions) -> Result<ImageResult, AiMuxError> {
        let mut warnings: Vec<Warning> = Vec::new();
        let (_model_id, version) = self
            .model_id
            .split_once(':')
            .unwrap_or((&self.model_id, ""));

        let replicate_opts = options.provider_options.get("replicate");
        let max_wait = replicate_opts
            .and_then(|o| o.get("maxWaitTimeInSeconds"))
            .and_then(serde_json::Value::as_u64);

        // Build image inputs
        let mut image_inputs = Map::new();
        if let Some(ref files) = options.files
            && !files.is_empty()
        {
            if self.is_flux2() {
                for (i, file) in files
                    .iter()
                    .enumerate()
                    .take(MAX_FLUX_2_INPUT_IMAGES as usize)
                {
                    let key = if i == 0 {
                        "input_image".to_string()
                    } else {
                        format!("input_image_{}", i + 1)
                    };
                    image_inputs.insert(key, json!(Self::file_to_data_uri(file)?));
                }
                if files.len() > MAX_FLUX_2_INPUT_IMAGES as usize {
                    warnings.push(Warning::Other { message: format!("Flux-2 models support up to {MAX_FLUX_2_INPUT_IMAGES} input images. Additional images are ignored.") });
                }
            } else {
                image_inputs.insert("image".into(), json!(Self::file_to_data_uri(&files[0])?));
                if files.len() > 1 {
                    warnings.push(Warning::Other { message: "This Replicate model only supports a single input image. Additional images are ignored.".into() });
                }
            }
        }

        // Handle mask
        let mask_input = if let Some(ref mask) = options.mask {
            if self.is_flux2() {
                warnings.push(Warning::Other {
                    message: "Flux-2 models do not support mask input. The mask will be ignored."
                        .into(),
                });
                None
            } else {
                Some(Self::file_to_data_uri(mask)?)
            }
        } else {
            None
        };

        // Build input object
        let mut input = Map::new();
        if let Some(ref p) = options.prompt {
            input.insert("prompt".into(), json!(p));
        }
        if let Some(ar) = options.aspect_ratio {
            input.insert("aspect_ratio".into(), json!(ar.to_string()));
        }
        if let Some(s) = options.size {
            input.insert("size".into(), json!(s.to_string()));
        }
        if let Some(seed) = options.seed {
            input.insert("seed".into(), json!(seed));
        }
        input.insert("num_outputs".into(), json!(options.n));
        for (k, v) in &image_inputs {
            input.insert(k.clone(), v.clone());
        }
        if let Some(m) = mask_input {
            input.insert("mask".into(), json!(m));
        }

        // Forward replicate provider options (excluding maxWaitTimeInSeconds)
        if let Some(ro) = replicate_opts.and_then(|v| v.as_object()) {
            for (k, v) in ro {
                if k != "maxWaitTimeInSeconds" {
                    input.insert(k.clone(), v.clone());
                }
            }
        }

        let mut body = Map::new();
        body.insert("input".into(), Value::Object(input));
        if !version.is_empty() {
            body.insert("version".into(), json!(version));
        }

        // Build headers with prefer
        let mut headers = self.build_headers(options.headers.as_ref());
        let prefer = if let Some(mw) = max_wait {
            format!("wait={mw}")
        } else {
            "wait".to_string()
        };
        headers.insert("prefer".into(), prefer);

        let header_list: Vec<(String, String)> = headers.into_iter().collect();

        let resp = aimux_provider_utils::post_json_to_api(
            HttpRequest {
                url: self.endpoint(),
                headers: header_list,

                abort_signal: options.abort_signal.clone(),
                call_id: None,
                recording_context: None,
                // This exchange legitimately holds the connection for the
                // whole wait window; widen the hang guard past it so a
                // slow-but-alive generation is not misread as a dead
                // transport (retryable → re-create → double billing).
                response_timeout: Some(Duration::from_secs(
                    max_wait.unwrap_or(DEFAULT_WAIT_SECONDS) + WAIT_TRANSPORT_MARGIN_SECONDS,
                )),
                validate_url: false,
                trusted_origin: None,
                credentialed_origin: None,
            },
            Value::Object(body),
            aimux_provider_utils::create_json_response_handler(),
            replicate_failed_response_handler(),
        )
        .await?;

        let rh = resp.response_headers.unwrap_or_default();
        let rb: Value = resp.value;
        let retries = retry::prepare_retries(
            options.max_retries,
            self.retry_config(),
            options.abort_signal.clone(),
        );

        // Extract output (string or array of strings)
        let urls: Vec<String> = match &rb["output"] {
            Value::Array(arr) => arr
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect(),
            Value::String(s) => vec![s.clone()],
            _ => Vec::new(),
        };

        // Download images; output URLs come from the prediction response
        // body, so they go through the SSRF download guard.
        let mut downloaded: Vec<Vec<u8>> = Vec::new();
        for url in &urls {
            let ir = retries
                .retry(|| {
                    aimux_provider_utils::get_from_api(
                        HttpRequest {
                            url: url.clone(),
                            headers: vec![],

                            abort_signal: options.abort_signal.clone(),
                            call_id: None,
                            recording_context: None,
                            response_timeout: None,
                            validate_url: true,
                            trusted_origin: Some(self.config.base_url.clone()),
                            credentialed_origin: Some(self.config.base_url.clone()),
                        },
                        aimux_provider_utils::create_binary_response_handler(),
                        replicate_failed_response_handler(),
                    )
                })
                .await?;
            downloaded.push(ir.value.to_vec());
        }

        Ok(ImageResult {
            images: ImageOutputs::Binary(downloaded),
            warnings,
            provider_metadata: None,
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
    VideoCallOptions, VideoData, VideoFile, VideoFileData, VideoModel, VideoOperationStart,
    VideoOperationStatus, VideoResponse, VideoResult,
};

/// Replicate video generation model — implements `VideoModel`.
///
/// Aligned with Vercel AI SDK `ReplicateVideoModel`
/// (`reference/ai/packages/replicate/src/replicate-video-model.ts`).
///
/// Uses the predictions API: POST to create (`do_start`); Core drives the
/// status polling via `do_status`.
pub struct ReplicateVideoModel {
    model_id: String,
    config: ReplicateConfig,
}

impl ReplicateVideoModel {
    #[must_use]
    pub fn new(model_id: String, config: ReplicateConfig) -> Self {
        Self { model_id, config }
    }

    fn build_headers(&self, extra: Option<&HashMap<String, String>>) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        headers.insert(
            "Authorization".to_string(),
            format!("Token {}", self.config.api_key),
        );
        if let Some(ref ch) = self.config.headers {
            for (k, v) in ch {
                headers.insert(k.clone(), v.clone());
            }
        }
        if let Some(extra) = extra {
            for (k, v) in extra {
                headers.insert(k.clone(), v.clone());
            }
        }
        headers
    }
}

fn video_file_to_url(file: &VideoFile) -> Result<String, AiMuxError> {
    match file {
        VideoFile::Url { url, .. } => Ok(url.clone()),
        VideoFile::File { media_type, data } => match data {
            VideoFileData::Base64(s) => Ok(format!("data:{media_type};base64,{s}")),
            VideoFileData::Binary(bytes) => {
                let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes);
                Ok(format!("data:{media_type};base64,{b64}"))
            }
        },
    }
}

#[async_trait]
impl VideoModel for ReplicateVideoModel {
    fn provider(&self) -> &str {
        "replicate"
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

        let headers = self.build_headers(options.headers.as_ref());
        let header_list: Vec<(String, String)> = headers.into_iter().collect();

        // Submit prediction.
        let resp = aimux_provider_utils::post_json_to_api(
            HttpRequest {
                url: format!("{}/predictions", self.config.base_url),
                headers: header_list,

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
            replicate_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;
        let prediction: Value = resp.value;
        let prediction_id = prediction
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                AiMuxError::InvalidResponseData("Replicate prediction missing id".to_string())
            })?
            .to_string();

        Ok(VideoOperationStart {
            operation: json!({ "prediction_id": prediction_id }),
            warnings,
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
        let prediction_id = operation
            .get("prediction_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                AiMuxError::InvalidArgument(
                    "replicate operation reference is missing prediction_id".to_string(),
                )
            })?;

        let headers = self.build_headers(options.headers.as_ref());
        let header_list: Vec<(String, String)> = headers.into_iter().collect();
        let poll_url = format!("{}/predictions/{}", self.config.base_url, prediction_id);

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
            replicate_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers.unwrap_or_default();
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
                return Err(AiMuxError::ApiCall(Box::new(ApiCallError {
                    status_code: Some(200),
                    provider_code: Some(status_str.to_string()),
                    response_body,
                    ..ApiCallError::new(
                        format!("Replicate prediction {status_str}"),
                        poll_url,
                        serde_json::json!({}),
                    )
                })));
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
            return Err(AiMuxError::InvalidResponseData(
                "Replicate prediction succeeded without a usable output URL".to_string(),
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
