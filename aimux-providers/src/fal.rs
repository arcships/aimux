//! Fal transcription (STT) provider.
//!
//! Aligned with Vercel AI SDK `createFal` / `FalTranscriptionModel`
//! (`reference/ai/packages/fal/src/fal-transcription-model.ts`).
//!
//! Fal uses an async queue pattern: POST to submit, then GET to poll until
//! the result is ready. The audio is sent as a base64 data URL.

use std::collections::HashMap;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::retry;
use aimux_core::shared::Warning;
use aimux_core::transcription_model::{
    AudioInput, TranscriptionCallOptions, TranscriptionModel, TranscriptionRequest,
    TranscriptionResponse, TranscriptionResult, TranscriptionSegment,
};
use aimux_provider_utils::{HttpRequest, load_api_key, sleep_or_abort, without_trailing_slash};

/// fal errors are FastAPI-style: `{"detail": "..."}` or
/// `{"detail": [{"loc": [...], "msg": "...", "type": "..."}]}` where `type`
/// is the machine code (https://fal.ai/docs/model-apis/errors).
fn fal_error_parts(data: &Value) -> aimux_provider_utils::ProviderErrorParts {
    let detail = data.get("detail");
    let first_detail = detail
        .and_then(Value::as_array)
        .and_then(|items| items.first());
    let message = detail
        .and_then(Value::as_str)
        .or_else(|| {
            first_detail
                .and_then(|item| item.get("msg"))
                .and_then(Value::as_str)
        })
        .or_else(|| {
            data.get("error")
                .and_then(|value| value.get("message"))
                .and_then(Value::as_str)
        })
        .or_else(|| data.get("error").and_then(Value::as_str))
        .or_else(|| data.get("message").and_then(Value::as_str))
        .unwrap_or("Fal request failed")
        .to_string();
    aimux_provider_utils::ProviderErrorParts {
        message,
        provider_code: first_detail
            .and_then(|item| item.get("type"))
            .and_then(Value::as_str)
            .map(str::to_owned),
    }
}

fn fal_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    aimux_provider_utils::create_json_error_response_handler(fal_error_parts)
}

// ── Config ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct FalConfig {
    pub api_key: String,
    pub base_url: String,
    pub headers: Option<HashMap<String, String>>,
}

impl FalConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://queue.fal.run".to_string(),
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

    /// Create from the `FAL_KEY` environment variable.
    ///
    /// # Errors
    ///
    /// Returns `AiMuxError::InvalidArgument` when the environment variable is not
    /// set.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let api_key = load_api_key(None, "FAL_KEY", "Fal")?;
        Ok(Self::new(api_key))
    }
}

pub struct FalProvider {
    config: FalConfig,
}

impl FalProvider {
    #[must_use]
    pub fn new(config: FalConfig) -> Self {
        Self { config }
    }

    #[must_use]
    pub fn transcription(&self, model_id: &str) -> FalTranscriptionModel {
        FalTranscriptionModel::new(model_id.to_string(), self.config.clone())
    }

    /// Create a video generation model instance for the given model name
    /// (e.g. `"fal-ai/kling-video"`).
    #[must_use]
    pub fn video(&self, model_id: &str) -> FalVideoModel {
        FalVideoModel::new(model_id.to_string(), self.config.clone())
    }

    /// Create an image generation model instance for the given model name
    /// (e.g. `"fal-ai/flux/schnell"`).
    #[must_use]
    pub fn image(&self, model_id: &str) -> FalImageModel {
        FalImageModel::new(model_id.to_string(), self.config.clone())
    }
}

// ── Response schema ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct FalJobResponse {
    request_id: String,
}

#[derive(Debug, Deserialize)]
struct FalChunk {
    text: String,
    #[serde(default)]
    timestamp: Option<Vec<f64>>,
}

#[derive(Debug, Deserialize)]
struct FalTranscriptionResponse {
    text: String,
    #[serde(default)]
    chunks: Option<Vec<FalChunk>>,
    #[serde(default, rename = "inferred_languages")]
    inferred_languages: Option<Vec<String>>,
}

// ── Model ───────────────────────────────────────────────────────────────────

fn audio_input_to_base64(audio: &AudioInput) -> Result<String, AiMuxError> {
    match audio {
        AudioInput::Base64(s) => Ok(s.clone()),
        AudioInput::Binary(bytes) => Ok(base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            bytes,
        )),
    }
}

pub struct FalTranscriptionModel {
    model_id: String,
    config: FalConfig,
}

impl FalTranscriptionModel {
    #[must_use]
    pub fn new(model_id: String, config: FalConfig) -> Self {
        Self { model_id, config }
    }

    fn build_headers(&self, extra: Option<&HashMap<String, String>>) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        headers.insert(
            "Authorization".to_string(),
            format!("Key {}", self.config.api_key),
        );
        if let Some(ref config_headers) = self.config.headers {
            for (k, v) in config_headers {
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

    fn submit_url(&self) -> String {
        format!("{}/fal-ai/{}", self.config.base_url, self.model_id)
    }

    fn poll_url(&self, request_id: &str) -> String {
        format!(
            "{}/fal-ai/{}/requests/{}",
            self.config.base_url, self.model_id, request_id
        )
    }
}

#[async_trait]
impl TranscriptionModel for FalTranscriptionModel {
    fn provider(&self) -> &str {
        "fal"
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_generate(
        &self,
        options: &TranscriptionCallOptions,
    ) -> Result<TranscriptionResult, AiMuxError> {
        let warnings: Vec<Warning> = Vec::new();

        let base64_audio = audio_input_to_base64(&options.audio)?;
        let audio_url = format!("data:{};base64,{}", options.media_type, base64_audio);

        let mut body = Map::new();
        body.insert("task".to_string(), json!("transcribe"));
        body.insert("diarize".to_string(), json!(true));
        body.insert("chunk_level".to_string(), json!("word"));
        body.insert("audio_url".to_string(), json!(audio_url));

        // Parse provider options.
        if let Some(ref po) = options.provider_options
            && let Some(fal) = po.get("fal")
        {
            if let Some(v) = fal.get("language") {
                body.insert("language".to_string(), v.clone());
            }
            if let Some(v) = fal.get("version") {
                body.insert("version".to_string(), v.clone());
            }
            if let Some(v) = fal.get("batchSize") {
                body.insert("batch_size".to_string(), v.clone());
            }
            if let Some(v) = fal.get("numSpeakers") {
                body.insert("num_speakers".to_string(), v.clone());
            }
            if let Some(v) = fal.get("diarize").and_then(serde_json::Value::as_bool) {
                body.insert("diarize".to_string(), json!(v));
            }
            if let Some(v) = fal.get("chunkLevel") {
                body.insert("chunk_level".to_string(), v.clone());
            }
        }

        let headers = self.build_headers(options.headers.as_ref());

        // Submit job.
        let resp = aimux_provider_utils::post_json_to_api(
            HttpRequest {
                url: self.submit_url(),
                headers: headers
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),

                abort_signal: options.abort_signal.clone(),
                call_id: None,
                recording_context: None,
                response_timeout: None,
                validate_url: false,
                trusted_origin: None,
                credentialed_origin: None,
            },
            Value::Object(body),
            aimux_provider_utils::create_json_response_handler(),
            fal_failed_response_handler(),
        )
        .await?;

        let job: FalJobResponse = resp.value;
        let retries = retry::prepare_retries(
            options.max_retries,
            self.retry_config(),
            options.abort_signal.clone(),
        );

        // Poll for result.
        let raw_body: Value;
        let parsed: FalTranscriptionResponse;
        let response_headers: HashMap<String, String>;
        loop {
            // Fal returns 400/404 while a queued request is still registering.
            // Normalize that state inside the retry attempt so a preceding 5xx
            // cannot wrap the pending response in RetryError.
            let resp = retries
                .retry(|| {
                    let request = aimux_provider_utils::get_from_api(
                        HttpRequest {
                            url: self.poll_url(&job.request_id),
                            headers: headers
                                .iter()
                                .map(|(k, v)| (k.clone(), v.clone()))
                                .collect(),

                            abort_signal: options.abort_signal.clone(),
                            call_id: None,
                            recording_context: None,
                            response_timeout: None,
                            validate_url: false,
                            trusted_origin: None,
                            credentialed_origin: None,
                        },
                        aimux_provider_utils::create_json_response_handler::<
                            FalTranscriptionResponse,
                        >(),
                        fal_failed_response_handler(),
                    );
                    async move {
                        match request.await {
                            Ok(response) => Ok(Some(response)),
                            Err(AiMuxError::ApiCall(detail))
                                if matches!(detail.status_code, Some(400 | 404)) =>
                            {
                                Ok(None)
                            }
                            Err(error) => Err(error),
                        }
                    }
                })
                .await?;
            let Some(resp) = resp else {
                sleep_or_abort(
                    std::time::Duration::from_millis(100),
                    options.abort_signal.as_ref(),
                )
                .await?;
                continue;
            };

            response_headers = resp.response_headers.unwrap_or_default();
            raw_body = resp.raw_value.unwrap_or(Value::Null);
            parsed = resp.value;
            break;
        }

        let segments: Vec<TranscriptionSegment> = parsed
            .chunks
            .as_ref()
            .map(|chunks| {
                chunks
                    .iter()
                    .map(|c| TranscriptionSegment {
                        text: c.text.clone(),
                        start_second: c
                            .timestamp
                            .as_ref()
                            .and_then(|t| t.first().copied())
                            .unwrap_or(0.0),
                        end_second: c
                            .timestamp
                            .as_ref()
                            .and_then(|t| t.get(1).copied())
                            .unwrap_or(0.0),
                    })
                    .collect()
            })
            .unwrap_or_default();

        let language = parsed
            .inferred_languages
            .as_ref()
            .and_then(|l| l.first().cloned());

        let duration_in_seconds = parsed
            .chunks
            .as_ref()
            .and_then(|c| c.last())
            .and_then(|c| c.timestamp.as_ref())
            .and_then(|t| t.get(1).copied());

        let timestamp = chrono::Utc::now().to_rfc3339();

        Ok(TranscriptionResult {
            text: parsed.text,
            segments,
            language,
            duration_in_seconds,
            warnings,
            request: Some(TranscriptionRequest { body: None }),
            response: TranscriptionResponse {
                timestamp: Some(timestamp),
                model_id: Some(self.model_id.clone()),
                headers: Some(response_headers),
                body: Some(raw_body),
            },
            provider_metadata: None,
        })
    }
}

// ── Image model ──────────────────────────────────────────────────────────────

use aimux_core::image_model::{
    ImageCallOptions, ImageFile, ImageFileData, ImageModel, ImageOutputs, ImageResponse,
    ImageResult,
};

/// An fal.ai image generation model.
pub struct FalImageModel {
    model_id: String,
    config: FalConfig,
}

impl FalImageModel {
    #[must_use]
    pub fn new(model_id: String, config: FalConfig) -> Self {
        Self { model_id, config }
    }

    fn build_headers(&self, extra: Option<&HashMap<String, String>>) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        headers.insert(
            "Authorization".to_string(),
            format!("Key {}", self.config.api_key),
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

    fn endpoint(&self) -> String {
        format!("{}/{}", self.config.base_url, self.model_id)
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

    fn convert_aspect_ratio_to_size(ar: &aimux_core::shared::AspectRatio) -> Value {
        match (ar.width(), ar.height()) {
            (1, 1) => json!("square_hd"),
            (16, 9) => json!("landscape_16_9"),
            (9, 16) => json!("portrait_16_9"),
            (4, 3) => json!("landscape_4_3"),
            (3, 4) => json!("portrait_4_3"),
            (16, 10) => json!({ "width": 1280, "height": 800 }),
            (10, 16) => json!({ "width": 800, "height": 1280 }),
            (21, 9) => json!({ "width": 2560, "height": 1080 }),
            (9, 21) => json!({ "width": 1080, "height": 2560 }),
            _ => json!(null),
        }
    }
}

#[async_trait]
impl ImageModel for FalImageModel {
    fn provider(&self) -> &str {
        "fal"
    }
    fn model_id(&self) -> &str {
        &self.model_id
    }
    fn max_images_per_call(&self) -> Option<u32> {
        Some(1)
    }

    async fn do_generate(&self, options: &ImageCallOptions) -> Result<ImageResult, AiMuxError> {
        let mut warnings: Vec<Warning> = Vec::new();
        let image_size = if let Some(s) = options.size {
            json!({ "width": s.width(), "height": s.height() })
        } else if let Some(ar) = options.aspect_ratio {
            Self::convert_aspect_ratio_to_size(&ar)
        } else {
            json!(null)
        };

        let mut body = Map::new();
        if let Some(ref p) = options.prompt {
            body.insert("prompt".into(), json!(p));
        }
        if let Some(seed) = options.seed {
            body.insert("seed".into(), json!(seed));
        }
        body.insert("image_size".into(), image_size);
        body.insert("num_images".into(), json!(options.n));

        let fal_opts = options.provider_options.get("fal");

        if let Some(ref files) = options.files
            && !files.is_empty()
        {
            let multi = fal_opts
                .and_then(|o| o.get("useMultipleImages"))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            if multi {
                let uris: Result<Vec<String>, _> =
                    files.iter().map(Self::file_to_data_uri).collect();
                body.insert("image_urls".into(), json!(uris?));
            } else {
                body.insert(
                    "image_url".into(),
                    json!(Self::file_to_data_uri(&files[0])?),
                );
                if files.len() > 1 {
                    warnings.push(Warning::Other {
                        message:
                            "Multiple input images provided but useMultipleImages is not enabled."
                                .into(),
                    });
                }
            }
        }
        if let Some(ref mask) = options.mask {
            body.insert("mask_url".into(), json!(Self::file_to_data_uri(mask)?));
        }

        if let Some(fal) = fal_opts.and_then(|v| v.as_object()) {
            let map: &[(&str, &str)] = &[
                ("imageUrl", "image_url"),
                ("maskUrl", "mask_url"),
                ("guidanceScale", "guidance_scale"),
                ("numInferenceSteps", "num_inference_steps"),
                ("enableSafetyChecker", "enable_safety_checker"),
                ("outputFormat", "output_format"),
                ("syncMode", "sync_mode"),
                ("safetyTolerance", "safety_tolerance"),
            ];
            for (key, value) in fal {
                if key == "__deprecatedKeys" || key == "useMultipleImages" {
                    continue;
                }
                let ak = map
                    .iter()
                    .find(|(k, _)| *k == key)
                    .map(|(_, v)| v.to_string())
                    .unwrap_or_else(|| key.clone());
                body.insert(ak, value.clone());
            }
        }

        let headers = self.build_headers(options.headers.as_ref());

        let resp = aimux_provider_utils::post_json_to_api(
            HttpRequest {
                url: self.endpoint(),
                headers: headers
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),

                abort_signal: options.abort_signal.clone(),
                call_id: None,
                recording_context: None,
                response_timeout: None,
                validate_url: false,
                trusted_origin: None,
                credentialed_origin: None,
            },
            Value::Object(body),
            aimux_provider_utils::create_json_response_handler(),
            fal_failed_response_handler(),
        )
        .await?;

        let rh = resp.response_headers.unwrap_or_default();
        let rb: Value = resp.value;
        let retries = retry::prepare_retries(
            options.max_retries,
            self.retry_config(),
            options.abort_signal.clone(),
        );

        let target_images: Vec<Value> = if let Some(i) = rb.get("images").and_then(|v| v.as_array())
        {
            i.clone()
        } else if let Some(i) = rb.get("image") {
            vec![i.clone()]
        } else {
            vec![]
        };

        let mut downloaded: Vec<Vec<u8>> = Vec::new();
        for img in &target_images {
            if let Some(url) = img.get("url").and_then(|v| v.as_str()) {
                // images[].url comes from the queue result response body, so
                // it goes through the SSRF download guard.
                let ir = retries
                    .retry(|| {
                        aimux_provider_utils::get_from_api(
                            HttpRequest {
                                url: url.to_string(),
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
                            fal_failed_response_handler(),
                        )
                    })
                    .await?;
                downloaded.push(ir.value.to_vec());
            }
        }

        let mut metadata = HashMap::new();
        let mut fm = Map::new();
        let im: Vec<Value> = target_images
            .iter()
            .map(|img| {
                let mut e = Map::new();
                if let Some(o) = img.as_object() {
                    for (k, v) in o {
                        if !matches!(
                            k.as_str(),
                            "url" | "content_type" | "file_name" | "file_data" | "file_size"
                        ) {
                            e.insert(k.clone(), v.clone());
                        }
                    }
                }
                if let Some(v) = img.get("content_type") {
                    e.insert("contentType".into(), v.clone());
                }
                if let Some(v) = img.get("file_name") {
                    e.insert("fileName".into(), v.clone());
                }
                if let Some(v) = img.get("file_data") {
                    e.insert("fileData".into(), v.clone());
                }
                if let Some(v) = img.get("file_size") {
                    e.insert("fileSize".into(), v.clone());
                }
                Value::Object(e)
            })
            .collect();
        fm.insert("images".into(), json!(im));
        if let Some(o) = rb.as_object() {
            for (k, v) in o {
                if !matches!(
                    k.as_str(),
                    "images" | "image" | "prompt" | "has_nsfw_concepts" | "nsfw_content_detected"
                ) {
                    fm.insert(k.clone(), v.clone());
                }
            }
        }
        metadata.insert("fal".into(), Value::Object(fm));

        Ok(ImageResult {
            images: ImageOutputs::Binary(downloaded),
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
    VideoCallOptions, VideoData, VideoFile, VideoFileData, VideoModel, VideoOperationStart,
    VideoOperationStatus, VideoResponse, VideoResult,
};

/// Fal video generation model — implements `VideoModel`.
///
/// Aligned with Vercel AI SDK `FalVideoModel`
/// (`reference/ai/packages/fal/src/fal-video-model.ts`).
///
/// Uses the same async queue pattern as the transcription model; the status
/// polling is driven by Core via `do_status`.
pub struct FalVideoModel {
    model_id: String,
    config: FalConfig,
}

impl FalVideoModel {
    #[must_use]
    pub fn new(model_id: String, config: FalConfig) -> Self {
        Self { model_id, config }
    }

    fn normalized_model_id(&self) -> String {
        self.model_id
            .strip_prefix("fal-ai/")
            .or_else(|| self.model_id.strip_prefix("fal/"))
            .unwrap_or(&self.model_id)
            .to_string()
    }

    fn submit_url(&self) -> String {
        format!(
            "{}/fal-ai/{}",
            self.config.base_url,
            self.normalized_model_id()
        )
    }

    fn poll_url(&self, request_id: &str) -> String {
        format!(
            "{}/fal-ai/{}/requests/{}",
            self.config.base_url,
            self.normalized_model_id(),
            request_id
        )
    }

    fn build_headers(&self, extra: Option<&HashMap<String, String>>) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        headers.insert(
            "Authorization".to_string(),
            format!("Key {}", self.config.api_key),
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

fn video_file_to_data_uri(file: &VideoFile) -> Result<String, AiMuxError> {
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
impl VideoModel for FalVideoModel {
    fn provider(&self) -> &str {
        "fal"
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
        let mut body = Map::new();

        if let Some(ref prompt) = options.prompt {
            body.insert("prompt".to_string(), json!(prompt));
        }
        if let Some(ref image) = options.image {
            body.insert(
                "image_url".to_string(),
                json!(video_file_to_data_uri(image)?),
            );
        }
        if let Some(ar) = options.aspect_ratio {
            body.insert("aspect_ratio".to_string(), json!(ar.to_string()));
        }
        if let Some(duration) = options.duration {
            body.insert("duration".to_string(), json!(format!("{}s", duration)));
        }
        if let Some(seed) = options.seed {
            body.insert("seed".to_string(), json!(seed));
        }

        let headers = self.build_headers(options.headers.as_ref());

        // Submit.
        let resp = aimux_provider_utils::post_json_to_api(
            HttpRequest {
                url: self.submit_url(),
                headers: headers
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),

                abort_signal: options.abort_signal.clone(),
                call_id: None,
                recording_context: None,
                response_timeout: None,
                validate_url: false,
                trusted_origin: None,
                credentialed_origin: None,
            },
            Value::Object(body),
            aimux_provider_utils::create_json_response_handler(),
            fal_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;
        let job: FalJobResponse = resp.value;

        Ok(VideoOperationStart {
            operation: json!({ "request_id": job.request_id }),
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
        let request_id = operation
            .get("request_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                AiMuxError::InvalidArgument(
                    "fal operation reference is missing request_id".to_string(),
                )
            })?;

        let headers = self.build_headers(options.headers.as_ref());

        let resp = aimux_provider_utils::get_from_api(
            HttpRequest {
                url: self.poll_url(request_id),
                headers: headers
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),

                abort_signal: options.abort_signal.clone(),
                call_id: None,
                recording_context: None,
                response_timeout: None,
                validate_url: false,
                trusted_origin: None,
                credentialed_origin: None,
            },
            aimux_provider_utils::create_json_response_handler::<Value>(),
            fal_failed_response_handler(),
        )
        .await;

        // Fal returns 400/404 while a queued request is still registering or
        // running; that is Pending, not a failure.
        let resp = match resp {
            Ok(response) => response,
            Err(AiMuxError::ApiCall(detail)) if matches!(detail.status_code, Some(400 | 404)) => {
                return Ok(VideoOperationStatus::Pending);
            }
            Err(error) => return Err(error),
        };

        let response_headers = resp.response_headers.unwrap_or_default();
        let raw_body = resp.value;

        // Extract video URL from response.
        let videos: Vec<VideoData> = if let Some(video) = raw_body
            .get("video")
            .and_then(|v| v.get("url"))
            .and_then(|v| v.as_str())
        {
            vec![VideoData::Url {
                url: video.to_string(),
                media_type: "video/mp4".to_string(),
            }]
        } else if let Some(url) = raw_body.get("url").and_then(|v| v.as_str()) {
            vec![VideoData::Url {
                url: url.to_string(),
                media_type: "video/mp4".to_string(),
            }]
        } else {
            return Err(AiMuxError::InvalidResponseData(
                "Fal video result missing video URL".to_string(),
            ));
        };

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
