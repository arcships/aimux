//! Black Forest Labs image provider.
//!
//! Aligned with Vercel AI SDK `BlackForestLabsImageModel`
//! (`reference/ai/packages/black-forest-labs/src/black-forest-labs-image-model.ts`).
//!
//! Uses an async submit + poll pattern: POST to submit, then GET poll_url
//! until status is "Ready", then download the image.

use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::error::ApiCallError;
use aimux_core::image_model::{
    ImageCallOptions, ImageFile, ImageFileData, ImageModel, ImageOutputs, ImageResponse,
    ImageResult,
};
use aimux_core::retry;
use aimux_core::shared::Warning;
use aimux_provider_utils::{HttpRequest, load_api_key, sleep_or_abort, without_trailing_slash};

/// AI SDK's `isTrustedUrl` (black-forest-labs-api.ts): credentials may go to
/// the configured origin, or over HTTPS to `bfl.ai` and its subdomains — BFL
/// serves polling and asset URLs from regional clusters. Allowlisted hosts
/// still get full URL/DNS validation; this gates only the headers.
fn bfl_trusted_url(url: &str, base_url: &str) -> bool {
    if aimux_provider_utils::same_origin(url, base_url) {
        return true;
    }
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    parsed.scheme() == "https"
        && parsed
            .host_str()
            .is_some_and(|host| host == "bfl.ai" || host.ends_with(".bfl.ai"))
}

fn bfl_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
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
                data.get("message")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "Unknown Black Forest Labs error".to_string());
        aimux_provider_utils::ProviderErrorParts {
            message,
            provider_code: None,
        }
    })
}

const DEFAULT_POLL_INTERVAL_MS: u64 = 500;
const DEFAULT_POLL_TIMEOUT_MS: u64 = 60000;

/// Configuration for the Black Forest Labs provider.
#[derive(Debug, Clone)]
pub struct BlackForestLabsConfig {
    pub api_key: String,
    pub base_url: String,
    pub headers: Option<HashMap<String, String>>,
}

impl BlackForestLabsConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://api.bfl.ai".to_string(),
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
    /// Create from the `BFL_API_KEY` environment variable.
    ///
    /// # Errors
    ///
    /// Returns `AiMuxError::InvalidArgument` when the environment variable is not
    /// set.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let api_key = load_api_key(None, "BFL_API_KEY", "Black Forest Labs")?;
        Ok(Self::new(api_key))
    }
}

pub struct BlackForestLabsProvider {
    config: BlackForestLabsConfig,
}
impl BlackForestLabsProvider {
    #[must_use]
    pub fn new(config: BlackForestLabsConfig) -> Self {
        Self { config }
    }
    #[must_use]
    pub fn image(&self, model_id: &str) -> BlackForestLabsImageModel {
        BlackForestLabsImageModel::new(model_id.to_string(), self.config.clone())
    }
}

/// A Black Forest Labs image generation model.
pub struct BlackForestLabsImageModel {
    model_id: String,
    config: BlackForestLabsConfig,
}
impl BlackForestLabsImageModel {
    #[must_use]
    pub fn new(model_id: String, config: BlackForestLabsConfig) -> Self {
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

    fn submit_endpoint(&self) -> String {
        format!("{}/{}", self.config.base_url, self.model_id)
    }

    fn file_to_string(file: &ImageFile) -> Result<String, AiMuxError> {
        match file {
            ImageFile::Url { url } => Ok(url.clone()),
            ImageFile::File { data, .. } => match data {
                ImageFileData::Base64(s) => Ok(s.clone()),
                ImageFileData::Binary(b) => Ok(base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    b,
                )),
            },
        }
    }

    fn convert_size_to_aspect_ratio(size: &aimux_core::shared::Size) -> Option<String> {
        let (w, h) = (size.width(), size.height());
        if w == 0 || h == 0 {
            return None;
        }
        let g = gcd(w, h);
        Some(format!("{}:{}", w / g, h / g))
    }
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 { a } else { gcd(b, a % b) }
}

#[async_trait]
impl ImageModel for BlackForestLabsImageModel {
    fn provider(&self) -> &str {
        "blackForestLabs"
    }
    fn model_id(&self) -> &str {
        &self.model_id
    }
    fn max_images_per_call(&self) -> Option<u32> {
        Some(1)
    }

    async fn do_generate(&self, options: &ImageCallOptions) -> Result<ImageResult, AiMuxError> {
        let mut warnings: Vec<Warning> = Vec::new();

        let final_aspect_ratio = if let Some(ar) = options.aspect_ratio {
            Some(ar.to_string())
        } else if let Some(size) = options.size {
            if let Some(ar) = Self::convert_size_to_aspect_ratio(&size) {
                warnings.push(Warning::Unsupported { feature: "size".into(), details: Some("Deriving aspect_ratio from size. Use the width and height provider options to specify dimensions for models that support them.".into()) });
                Some(ar)
            } else {
                None
            }
        } else {
            None
        };

        if options.size.is_some() && options.aspect_ratio.is_some() {
            warnings.push(Warning::Unsupported { feature: "size".into(), details: Some("Black Forest Labs ignores size when aspectRatio is provided. Use the width and height provider options to specify dimensions for models that support them".into()) });
        }

        let bfl_opts = options.provider_options.get("blackForestLabs");
        let (width_str, height_str) = options
            .size
            .map(|s| (s.width().to_string(), s.height().to_string()))
            .unwrap_or((String::new(), String::new()));

        // Build input images
        let input_images: Vec<String> = if let Some(ref files) = options.files {
            files
                .iter()
                .map(Self::file_to_string)
                .collect::<Result<_, _>>()?
        } else {
            Vec::new()
        };

        if input_images.len() > 10 {
            return Err(AiMuxError::InvalidArgument(
                "Black Forest Labs supports up to 10 input images.".into(),
            ));
        }

        let input_field = if self.model_id == "flux-pro-1.0-fill" {
            "image"
        } else {
            "input_image"
        };
        let mut input_images_obj = Map::new();
        for (i, img) in input_images.iter().enumerate() {
            let key = if i == 0 {
                input_field.to_string()
            } else {
                format!("{}_{}", input_field, i + 1)
            };
            input_images_obj.insert(key, json!(img));
        }

        let mask_value = if let Some(ref mask) = options.mask {
            Some(Self::file_to_string(mask)?)
        } else {
            None
        };

        let mut body = Map::new();
        if let Some(ref p) = options.prompt {
            body.insert("prompt".into(), json!(p));
        }
        if let Some(seed) = options.seed {
            body.insert("seed".into(), json!(seed));
        }
        if let Some(ref ar) = final_aspect_ratio {
            body.insert("aspect_ratio".into(), json!(ar));
        }
        // width/height from provider options or size
        let width = bfl_opts.and_then(|o| o.get("width")).cloned().or_else(|| {
            if options.size.is_some() {
                Some(json!(width_str))
            } else {
                None
            }
        });
        let height = bfl_opts.and_then(|o| o.get("height")).cloned().or_else(|| {
            if options.size.is_some() {
                Some(json!(height_str))
            } else {
                None
            }
        });
        if let Some(w) = width {
            body.insert("width".into(), w);
        }
        if let Some(h) = height {
            body.insert("height".into(), h);
        }

        // Forward BFL provider options
        if let Some(bfl) = bfl_opts.and_then(|v| v.as_object()) {
            let map: &[(&str, &str)] = &[
                ("steps", "steps"),
                ("guidance", "guidance"),
                ("imagePromptStrength", "image_prompt_strength"),
                ("imagePrompt", "image_prompt"),
                ("outputFormat", "output_format"),
                ("promptUpsampling", "prompt_upsampling"),
                ("raw", "raw"),
                ("safetyTolerance", "safety_tolerance"),
                ("webhookSecret", "webhook_secret"),
                ("webhookUrl", "webhook_url"),
            ];
            for (key, value) in bfl {
                if matches!(
                    key.as_str(),
                    "width" | "height" | "pollIntervalMillis" | "pollTimeoutMillis"
                ) {
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
        for (k, v) in &input_images_obj {
            body.insert(k.clone(), v.clone());
        }
        if let Some(m) = mask_value {
            body.insert("mask".into(), json!(m));
        }

        let headers = self.build_headers(options.headers.as_ref());
        let header_list: Vec<(String, String)> = headers.into_iter().collect();

        // Submit
        let resp = aimux_provider_utils::post_json_to_api(
            HttpRequest {
                url: self.submit_endpoint(),
                headers: header_list.clone(),

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
            bfl_failed_response_handler(),
        )
        .await?;
        let submit_body: Value = resp.value;

        let poll_url = submit_body
            .get("polling_url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                AiMuxError::InvalidResponseData("missing polling_url in BFL response".to_string())
            })?
            .to_string();
        let request_id = submit_body
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let retries = retry::prepare_retries(
            options.max_retries,
            self.retry_config(),
            options.abort_signal.clone(),
        );

        // Poll for result
        let poll_interval = bfl_opts
            .and_then(|o| o.get("pollIntervalMillis"))
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(DEFAULT_POLL_INTERVAL_MS);
        let poll_timeout = bfl_opts
            .and_then(|o| o.get("pollTimeoutMillis"))
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(DEFAULT_POLL_TIMEOUT_MS);
        let max_attempts = (poll_timeout / poll_interval.max(1)) as usize;

        let mut poll_url_with_id = url::Url::parse(&poll_url)
            .map_err(|e| AiMuxError::InvalidResponseData(format!("invalid BFL poll URL: {e}")))?;
        if !poll_url_with_id.query_pairs().any(|(k, _)| k == "id") {
            poll_url_with_id
                .query_pairs_mut()
                .append_pair("id", &request_id);
        }

        let mut image_url = None;
        let mut result_seed = None;
        let mut result_start_time = None;
        let mut result_end_time = None;
        let mut result_duration = None;

        // AI SDK gates headers per URL via isTrustedUrl; response-supplied
        // targets outside the BFL allowlist get none.
        let gated_headers = |url: &str| -> Vec<(String, String)> {
            if bfl_trusted_url(url, &self.config.base_url) {
                header_list.clone()
            } else {
                vec![]
            }
        };

        for _ in 0..max_attempts {
            // AI SDK polls polling_url with validateUrl: true and gates the
            // headers itself via isTrustedUrl (base_url origin or HTTPS
            // *.bfl.ai), so credentialed_origin is None here.
            let poll_url = poll_url_with_id.to_string();
            let pr = retries
                .retry(|| {
                    aimux_provider_utils::get_from_api(
                        HttpRequest {
                            url: poll_url.clone(),
                            headers: gated_headers(&poll_url),

                            abort_signal: options.abort_signal.clone(),
                            call_id: None,
                            recording_context: None,
                            response_timeout: None,
                            validate_url: true,
                            trusted_origin: Some(self.config.base_url.clone()),
                            // Headers are gated per URL by gated_headers above.
                            credentialed_origin: None,
                        },
                        aimux_provider_utils::create_json_response_handler::<Value>(),
                        bfl_failed_response_handler(),
                    )
                })
                .await?;
            let response_body = pr.raw_value.as_ref().map(ToString::to_string);
            let pv = pr.value;

            let poll_status = pv
                .get("status")
                .and_then(|v| v.as_str())
                .or_else(|| pv.get("state").and_then(|v| v.as_str()))
                .unwrap_or("");
            if poll_status == "Ready" {
                if let Some(result) = pv.get("result") {
                    image_url = result
                        .get("sample")
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    result_seed = result.get("seed").cloned();
                    result_start_time = result.get("start_time").cloned();
                    result_end_time = result.get("end_time").cloned();
                    result_duration = result.get("duration").cloned();
                }
                if image_url.is_none() {
                    return Err(AiMuxError::InvalidResponseData(
                        "BFL poll reported Ready without result.sample".to_string(),
                    ));
                }
                break;
            }
            if poll_status == "Error" || poll_status == "Failed" {
                return Err(AiMuxError::ApiCall(Box::new(ApiCallError {
                    status_code: Some(200),
                    provider_code: Some(poll_status.to_string()),
                    message: "Black Forest Labs generation failed.".into(),
                    response_body,
                    ..ApiCallError::new(
                        "Black Forest Labs generation failed.",
                        poll_url_with_id.to_string(),
                        serde_json::json!({}),
                    )
                })));
            }
            sleep_or_abort(
                std::time::Duration::from_millis(poll_interval),
                options.abort_signal.as_ref(),
            )
            .await?;
        }

        let image_url = image_url.ok_or_else(|| {
            AiMuxError::Timeout(format!(
                "blackForestLabs task {request_id} polling timed out after {poll_timeout}ms"
            ))
        })?;

        // Download image; result.sample is a URL from the poll response body.
        // AI SDK sends its headers to trusted BFL hosts on the download too,
        // gated by the same allowlist.
        let ir = retries
            .retry(|| {
                aimux_provider_utils::get_from_api(
                    HttpRequest {
                        url: image_url.clone(),
                        headers: gated_headers(&image_url),

                        abort_signal: options.abort_signal.clone(),
                        call_id: None,
                        recording_context: None,
                        response_timeout: None,
                        validate_url: true,
                        trusted_origin: Some(self.config.base_url.clone()),
                        // Headers are gated per URL by gated_headers above.
                        credentialed_origin: None,
                    },
                    aimux_provider_utils::create_binary_response_handler(),
                    bfl_failed_response_handler(),
                )
            })
            .await?;
        let image_bytes = ir.value.to_vec();
        let download_headers: HashMap<String, String> = HashMap::new();

        // Build provider metadata
        let mut metadata = HashMap::new();
        let mut bfl_meta = Map::new();
        let mut img_meta = Map::new();
        if let Some(s) = result_seed {
            img_meta.insert("seed".into(), s);
        }
        if let Some(s) = result_start_time {
            img_meta.insert("start_time".into(), s);
        }
        if let Some(e) = result_end_time {
            img_meta.insert("end_time".into(), e);
        }
        if let Some(d) = result_duration {
            img_meta.insert("duration".into(), d);
        }
        if let Some(c) = submit_body.get("cost") {
            img_meta.insert("cost".into(), c.clone());
        }
        if let Some(i) = submit_body.get("input_mp") {
            img_meta.insert("inputMegapixels".into(), i.clone());
        }
        if let Some(o) = submit_body.get("output_mp") {
            img_meta.insert("outputMegapixels".into(), o.clone());
        }
        bfl_meta.insert("images".into(), json!([Value::Object(img_meta)]));
        metadata.insert("blackForestLabs".into(), Value::Object(bfl_meta));

        Ok(ImageResult {
            images: ImageOutputs::Binary(vec![image_bytes]),
            warnings,
            provider_metadata: Some(metadata),
            response: ImageResponse {
                timestamp: Some(chrono::Utc::now().to_rfc3339()),
                model_id: Some(self.model_id.clone()),
                headers: Some(download_headers),
            },
            usage: None,
        })
    }
}
