//! Prodia image provider.
//!
//! Aligned with Vercel AI SDK `ProdiaImageModel`
//! (`reference/ai/packages/prodia/src/prodia-image-model.ts`).
//!
//! POST to `/job?price=true`, returns a multipart response with a JSON "job"
//! part and a binary "output" image part.
//!
//! [`create_prodia`] takes [`ProdiaProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`ProdiaProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `PRODIA_API_KEY`.
//! [`prodia()`] is the default instance; it reads nothing and cannot fail.

use std::collections::HashMap;
use std::sync::OnceLock;

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::error::{AiMuxError, ApiCallError};
use aimux_core::image_model::{
    ImageCallOptions, ImageModel, ImageOutputs, ImageResponse, ImageResult,
};
use aimux_core::shared::Warning;
use aimux_provider_utils::{FetchFunction, HeaderMapOpt, Resolvable, validate_base_url};

use crate::shared::{AuthScheme, Credential, EndpointConfig, ProviderHeaders};

fn prodia_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    aimux_provider_utils::create_json_error_response_handler(|data| {
        let detail = data.get("detail");
        let message = detail
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                detail
                    .filter(|value| !value.is_null())
                    .map(Value::to_string)
            })
            .or_else(|| {
                data.get("error")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .or_else(|| {
                data.get("message")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "Unknown Prodia error".to_string());
        aimux_provider_utils::ProviderErrorParts {
            message,
            provider_code: None,
        }
    })
}

pub(crate) mod options;

const DEFAULT_BASE_URL: &str = "https://prodia.com/api";
const API_KEY_ENV_VAR: &str = "PRODIA_API_KEY";
const DEFAULT_NAME: &str = "prodia";

/// Settings of [`create_prodia`].
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except `base_url` and `name`; `api_key` and `headers` are
/// evaluated on every request.
#[derive(Clone, Default)]
pub struct ProdiaProviderSettings {
    /// Base URL for the API calls. Default `https://prodia.com/api`; a
    /// trailing slash is removed.
    pub base_url: Option<String>,
    /// The API key. `None` loads `PRODIA_API_KEY` when a request is made and
    /// fails that request with `AiMuxError::LoadApiKey` if it is unset. An
    /// explicit value is used as given, `""` included: it never falls back to
    /// the environment. A [`Resolvable::Future`] is awaited once, an
    /// [`Resolvable::AsyncFn`] on every request.
    pub api_key: Option<Resolvable<String>>,
    /// Extra headers on every request. A `None` value removes the header,
    /// including the credential. Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The provider name, the prefix of the `provider()` strings (`"{name}.image"`, `"{name}.video"`).
    /// Default `"prodia"`. The providerOptions key stays `prodia`.
    pub name: Option<String>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
}

impl std::fmt::Debug for ProdiaProviderSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProdiaProviderSettings")
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key)
            .field(
                "headers",
                &self.headers.as_ref().map(std::collections::HashMap::len),
            )
            .field("name", &self.name)
            .field("fetch", &self.fetch.is_some())
            .finish()
    }
}

/// Create a Prodia provider.
///
/// # Errors
///
/// Returns `AiMuxError::InvalidArgument` when `base_url` is not an `http(s)`
/// URL with a host. That is the only way this fails: the key is loaded per
/// request, not here.
pub fn create_prodia(settings: ProdiaProviderSettings) -> Result<ProdiaProvider, AiMuxError> {
    let base_url = match settings.base_url.as_deref() {
        Some(url) => validate_base_url(url)?,
        None => DEFAULT_BASE_URL.to_string(),
    };
    Ok(ProdiaProvider {
        name: settings.name.unwrap_or_else(|| DEFAULT_NAME.to_string()),
        base_url,
        credential: Credential::explicit_or_env(settings.api_key, API_KEY_ENV_VAR, "Prodia"),
        user_headers: settings.headers,
        fetch: settings.fetch,
    })
}

/// The default provider: `create_prodia` with default settings, created on
/// first use. Creating it reads nothing from the environment and cannot fail;
/// a missing key surfaces from the first request instead.
pub fn prodia() -> &'static ProdiaProvider {
    static DEFAULT: OnceLock<ProdiaProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_prodia(ProdiaProviderSettings::default())
            .expect("default Prodia settings are always valid")
    })
}

/// A Prodia provider. Cheap to clone the models out of; it holds no HTTP
/// client.
pub struct ProdiaProvider {
    name: String,
    base_url: String,
    credential: Credential,
    user_headers: Option<HeaderMapOpt>,
    fetch: Option<FetchFunction>,
}

impl ProdiaProvider {
    fn model_config(&self, method: &str) -> EndpointConfig {
        EndpointConfig::fixed(
            format!("{}.{method}", self.name),
            self.base_url.clone(),
            match method {
                "image" => ProviderHeaders::new(
                    self.credential.clone(),
                    AuthScheme::Header("X-Prodia-Key"),
                    vec![(
                        "Accept".to_string(),
                        "multipart/form-data; image/png".to_string(),
                    )],
                    self.user_headers.clone(),
                ),
                _ => ProviderHeaders::new(
                    self.credential.clone(),
                    AuthScheme::Header("X-Prodia-Key"),
                    Vec::new(),
                    self.user_headers.clone(),
                ),
            },
            self.fetch.clone(),
        )
    }

    /// An image model (e.g. `"inference.flux-fast.schnell.txt2img.v2"`); `provider()` is `"{name}.image"`.
    #[must_use]
    pub fn image(&self, model_id: &str) -> ProdiaImageModel {
        ProdiaImageModel::from_config(model_id.to_string(), self.model_config("image"))
    }

    /// A video model; `provider()` is `"{name}.video"`.
    #[must_use]
    pub fn video(&self, model_id: &str) -> ProdiaVideoModel {
        ProdiaVideoModel::from_config(model_id.to_string(), self.model_config("video"))
    }
}

impl ::aimux_core::Provider for ProdiaProvider {
    fn language_model(
        &self,
        model_id: &str,
    ) -> Result<::std::sync::Arc<dyn ::aimux_core::LanguageModel>, AiMuxError> {
        Err(AiMuxError::no_such_model(model_id, "languageModel"))
    }

    fn embedding_model(
        &self,
        model_id: &str,
    ) -> Result<::std::sync::Arc<dyn ::aimux_core::EmbeddingModel>, AiMuxError> {
        Err(AiMuxError::no_such_model(model_id, "embeddingModel"))
    }

    fn image_model(
        &self,
        model_id: &str,
    ) -> Result<::std::sync::Arc<dyn ::aimux_core::ImageModel>, AiMuxError> {
        Ok(::std::sync::Arc::new(self.image(model_id)))
    }

    fn video_model(
        &self,
        model_id: &str,
    ) -> Option<Result<::std::sync::Arc<dyn ::aimux_core::VideoModel>, AiMuxError>> {
        Some(Ok(::std::sync::Arc::new(self.video(model_id))))
    }
}

pub struct ProdiaImageModel {
    model_id: String,
    config: EndpointConfig,
}

impl ProdiaImageModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }
}

/// Parse a multipart body into parts (name, content_type, body).
fn parse_multipart(body: &[u8], boundary: &str) -> Vec<(String, String, Vec<u8>)> {
    let delimiter = format!("--{boundary}");
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
        let next_delimiter = format!("\r\n{delimiter}");
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
        &self.config.provider
    }
    fn model_id(&self) -> &str {
        &self.model_id
    }
    fn max_images_per_call(&self) -> Option<u32> {
        Some(1)
    }

    async fn do_generate(&self, options: &ImageCallOptions) -> Result<ImageResult, AiMuxError> {
        let warnings: Vec<Warning> = Vec::new();

        let prodia_opts = options::prodia_options(Some(&options.provider_options));

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

        let exchange = self.config.exchange(options.headers.as_ref()).await?;

        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(exchange.url("/job?price=true"), options),
            body,
            aimux_provider_utils::create_binary_response_handler(),
            prodia_failed_response_handler(),
        )
        .await?;

        let rh = resp.response_headers;
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

        let body_bytes = resp.value.to_vec();

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
        metadata.insert(options::NAMESPACE.into(), prodia_meta);

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
    VideoCallOptions, VideoData, VideoModel, VideoOperationStart, VideoOperationStatus,
    VideoResponse, VideoResult,
};

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
                AiMuxError::InvalidResponseData(
                    "Prodia job submission response missing job id".to_string(),
                )
            })?
            .to_string();

        Ok(VideoOperationStart {
            operation: json!({ "job_id": job_id }),
            warnings,
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
        let job_id = operation
            .get("job_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                AiMuxError::InvalidArgument(
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
                return Err(AiMuxError::ApiCall(Box::new(ApiCallError {
                    status_code: Some(200),
                    provider_code: Some(status_str.to_string()),
                    response_body,
                    ..ApiCallError::new(
                        "Prodia video generation failed",
                        poll_url,
                        serde_json::json!({}),
                    )
                })));
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
                AiMuxError::InvalidResponseData("Prodia job done without a video URL".to_string())
            })?;
        let videos: Vec<VideoData> = vec![VideoData::Url {
            url: video_url.to_string(),
            media_type: "video/mp4".to_string(),
        }];

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
