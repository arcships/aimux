//! Luma image provider.
//!
//! Aligned with Vercel AI SDK `LumaImageModel`
//! (`reference/ai/packages/luma/src/luma-image-model.ts`).
//!
//! Uses an async submit + poll pattern: POST to create generation, then GET
//! poll until state is "completed", then download the image.

use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::error::ApiCallError;
use aimux_core::image_model::{
    ImageCallOptions, ImageFile, ImageModel, ImageOutputs, ImageResponse, ImageResult,
};
use aimux_core::shared::Warning;
use aimux_provider_utils::response::DEFAULT_ERROR_STRUCTURE;
use aimux_provider_utils::{
    HttpBody, HttpMethod, HttpRequest, RetryConfig, load_api_key, send, sleep_or_abort,
    without_trailing_slash,
};

const DEFAULT_POLL_INTERVAL_MS: u64 = 500;
const DEFAULT_MAX_POLL_ATTEMPTS: u64 = 120;

/// Configuration for the Luma provider.
#[derive(Debug, Clone)]
pub struct LumaConfig {
    pub api_key: String,
    pub base_url: String,
    pub headers: Option<HashMap<String, String>>,
}

impl LumaConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://api.lumalabs.ai".to_string(),
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
    /// Create from the `LUMA_API_KEY` environment variable.
    ///
    /// # Errors
    ///
    /// Returns `AiMuxError::InvalidArgument` when the environment variable is not
    /// set.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let api_key = load_api_key(None, "LUMA_API_KEY", "Luma")?;
        Ok(Self::new(api_key))
    }
}

pub struct LumaProvider {
    config: LumaConfig,
}
impl LumaProvider {
    #[must_use]
    pub fn new(config: LumaConfig) -> Self {
        Self { config }
    }
    #[must_use]
    pub fn image(&self, model_id: &str) -> LumaImageModel {
        LumaImageModel::new(model_id.to_string(), self.config.clone())
    }
}

/// A Luma image generation model.
pub struct LumaImageModel {
    model_id: String,
    config: LumaConfig,
}
impl LumaImageModel {
    #[must_use]
    pub fn new(model_id: String, config: LumaConfig) -> Self {
        Self { model_id, config }
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

    fn generations_url(&self, generation_id: Option<&str>) -> String {
        match generation_id {
            Some(id) => format!(
                "{}/dream-machine/v1/generations/{}",
                self.config.base_url, id
            ),
            None => format!(
                "{}/dream-machine/v1/generations/image",
                self.config.base_url
            ),
        }
    }
}

#[async_trait]
impl ImageModel for LumaImageModel {
    fn provider(&self) -> &str {
        "luma"
    }
    fn model_id(&self) -> &str {
        &self.model_id
    }
    fn max_images_per_call(&self) -> Option<u32> {
        Some(1)
    }

    async fn do_generate(&self, options: &ImageCallOptions) -> Result<ImageResult, AiMuxError> {
        let mut warnings: Vec<Warning> = Vec::new();

        if options.seed.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "seed".into(),
                details: Some("This model does not support the `seed` option.".into()),
            });
        }
        if options.size.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "size".into(),
                details: Some(
                    "This model does not support the `size` option. Use `aspectRatio` instead."
                        .into(),
                ),
            });
        }

        let luma_opts = options.provider_options.get("luma");

        // Extract non-request options
        let poll_interval = luma_opts
            .and_then(|o| o.get("pollIntervalMillis"))
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(DEFAULT_POLL_INTERVAL_MS);
        let max_poll_attempts = luma_opts
            .and_then(|o| o.get("maxPollAttempts"))
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(DEFAULT_MAX_POLL_ATTEMPTS);
        let reference_type = luma_opts
            .and_then(|o| o.get("referenceType"))
            .and_then(|v| v.as_str())
            .unwrap_or("image");
        let image_configs = luma_opts
            .and_then(|o| o.get("images"))
            .and_then(|v| v.as_array())
            .cloned();

        // Build editing options from files
        let mut editing_opts = Map::new();
        if let Some(ref files) = options.files
            && !files.is_empty()
        {
            // Validate all files are URL-based
            for file in files {
                if !matches!(file, ImageFile::Url { .. }) {
                    return Err(AiMuxError::InvalidArgument(
                        "Luma AI only supports URL-based images.".into(),
                    ));
                }
            }

            let default_weight = match reference_type {
                "style" => 0.8,
                "modify_image" => 1.0,
                _ => 0.85, // image, character
            };

            match reference_type {
                "style" => {
                    let arr: Vec<Value> = files
                        .iter()
                        .enumerate()
                        .map(|(i, f)| {
                            let url = if let ImageFile::Url { url } = f {
                                url.clone()
                            } else {
                                String::new()
                            };
                            let weight = image_configs
                                .as_ref()
                                .and_then(|c| c.get(i))
                                .and_then(|c| c.get("weight"))
                                .and_then(serde_json::Value::as_f64)
                                .unwrap_or(default_weight);
                            json!({ "url": url, "weight": weight })
                        })
                        .collect();
                    editing_opts.insert("style".into(), json!(arr));
                }
                "modify_image" => {
                    if files.len() > 1 {
                        return Err(AiMuxError::InvalidArgument(
                            "Luma AI modify_image only supports a single input image.".into(),
                        ));
                    }
                    let url = if let ImageFile::Url { url } = &files[0] {
                        url.clone()
                    } else {
                        String::new()
                    };
                    let weight = image_configs
                        .as_ref()
                        .and_then(|c| c.first())
                        .and_then(|c| c.get("weight"))
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(default_weight);
                    editing_opts.insert(
                        "modify_image".into(),
                        json!({ "url": url, "weight": weight }),
                    );
                }
                _ => {
                    // image (default)
                    if files.len() > 4 {
                        return Err(AiMuxError::InvalidArgument(
                            "Luma AI image supports up to 4 reference images.".into(),
                        ));
                    }
                    let arr: Vec<Value> = files
                        .iter()
                        .enumerate()
                        .map(|(i, f)| {
                            let url = if let ImageFile::Url { url } = f {
                                url.clone()
                            } else {
                                String::new()
                            };
                            let weight = image_configs
                                .as_ref()
                                .and_then(|c| c.get(i))
                                .and_then(|c| c.get("weight"))
                                .and_then(serde_json::Value::as_f64)
                                .unwrap_or(default_weight);
                            json!({ "url": url, "weight": weight })
                        })
                        .collect();
                    editing_opts.insert("image".into(), json!(arr));
                }
            }
        }

        if options.mask.is_some() {
            return Err(AiMuxError::InvalidArgument(
                "Luma AI does not support mask-based image editing.".into(),
            ));
        }

        // Build request body
        let mut body = Map::new();
        if let Some(ref p) = options.prompt {
            body.insert("prompt".into(), json!(p));
        }
        if let Some(ar) = options.aspect_ratio {
            body.insert("aspect_ratio".into(), json!(ar.to_string()));
        }
        body.insert("model".into(), json!(self.model_id));
        for (k, v) in &editing_opts {
            body.insert(k.clone(), v.clone());
        }

        // Forward luma provider options (excluding non-request options)
        if let Some(luma) = luma_opts.and_then(|v| v.as_object()) {
            for (k, v) in luma {
                if matches!(
                    k.as_str(),
                    "pollIntervalMillis" | "maxPollAttempts" | "referenceType" | "images"
                ) {
                    continue;
                }
                body.insert(k.clone(), v.clone());
            }
        }

        let headers = self.build_headers(options.headers.as_ref());
        let header_list: Vec<(String, String)> = headers.into_iter().collect();

        // Submit
        let resp = send(
            HttpRequest {
                method: HttpMethod::Post,
                url: self.generations_url(None),
                headers: header_list.clone(),
                body: HttpBody::Json(Value::Object(body)),

                abort_signal: options.abort_signal.clone(),
                call_id: None,
                recording_context: None,
            },
            RetryConfig::default(),
            &DEFAULT_ERROR_STRUCTURE,
        )
        .await?;
        let rh = resp.headers;
        let submit_body: Value = serde_json::from_slice(&resp.body)?;

        let generation_id = submit_body
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                AiMuxError::InvalidResponseData("missing id in Luma response".to_string())
            })?
            .to_string();

        // Poll for completion
        let mut image_url = None;
        for _ in 0..max_poll_attempts {
            let pr = send(
                HttpRequest {
                    method: HttpMethod::Get,
                    url: self.generations_url(Some(&generation_id)),
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
            let pv: Value = serde_json::from_slice(&pr.body)?;

            let state = pv.get("state").and_then(|v| v.as_str()).unwrap_or("");
            if state == "completed" {
                image_url = pv
                    .get("assets")
                    .and_then(|a| a.get("image"))
                    .and_then(|v| v.as_str())
                    .map(String::from);
                if image_url.is_none() {
                    return Err(AiMuxError::InvalidResponseData(format!(
                        "Luma generation {generation_id} completed without assets.image"
                    )));
                }
                break;
            }
            if state == "failed" {
                return Err(AiMuxError::ApiCall(ApiCallError {
                    status_code: Some(pr.status),
                    provider_code: Some(state.to_string()),
                    message: "Image generation failed.".into(),
                    response_body: Some(String::from_utf8_lossy(&pr.body).into_owned()),
                    ..Default::default()
                }));
            }
            sleep_or_abort(
                std::time::Duration::from_millis(poll_interval),
                options.abort_signal.as_ref(),
            )
            .await?;
        }

        let image_url = image_url.ok_or_else(|| {
            AiMuxError::Timeout(format!(
                "luma generation {generation_id} polling timed out after {max_poll_attempts} attempts ({}ms)",
                max_poll_attempts * poll_interval
            ))
        })?;

        // Download image
        let ir = send(
            HttpRequest {
                method: HttpMethod::Get,
                url: image_url,
                headers: vec![],
                body: HttpBody::Empty,

                abort_signal: options.abort_signal.clone(),
                call_id: None,
                recording_context: None,
            },
            RetryConfig::default(),
            &DEFAULT_ERROR_STRUCTURE,
        )
        .await?;
        let image_bytes = ir.body.to_vec();

        Ok(ImageResult {
            images: ImageOutputs::Binary(vec![image_bytes]),
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
