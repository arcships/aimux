//! Black Forest Labs image generation model.

use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::ErrorCause;
use aimux_core::error::AiMuxError;
use aimux_core::image_model::{ImageCallOptions, ImageFile, ImageModel, ImageOutputs, ImageResult};
use aimux_core::shared::Warning;
use aimux_provider_utils::HttpRequest;

use crate::shared::{
    EndpointConfig, POLL_INTERVAL_MILLIS_KEY, PollStep, image_response, is_poll_control_key,
    namespaced_options, poll_interval_ms, poll_until, retry_download, size_to_aspect_ratio,
    task_failed, unsupported,
};

/// The providerOptions key, read whatever the provider is named.
pub(crate) const NAMESPACE: &str = "blackForestLabs";

/// Milliseconds between two polls of a generation
/// (`providerOptions.blackForestLabs.pollIntervalMillis` overrides it for one call).
const POLL_INTERVAL_MS: u64 = 500;
/// How many times a generation is polled before the call gives up (the sixty
/// seconds of the AI SDK at the default interval).
const MAX_POLL_ATTEMPTS: u32 = 120;

/// What a finished generation reports.
struct BflResult {
    image_url: String,
    seed: Option<Value>,
    start_time: Option<Value>,
    end_time: Option<Value>,
    duration: Option<Value>,
}

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

/// A Black Forest Labs image generation model.
pub struct BlackForestLabsImageModel {
    model_id: String,
    config: EndpointConfig,
}

impl BlackForestLabsImageModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }

    fn file_to_string(file: &ImageFile) -> Result<String, AiMuxError> {
        match file {
            ImageFile::Url { url } => Ok(url.clone()),
            ImageFile::File { data, .. } => {
                Ok(aimux_provider_utils::convert_to_base64(data).into_owned())
            }
        }
    }
}

#[async_trait]
impl ImageModel for BlackForestLabsImageModel {
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
        let mut warnings: Vec<Warning> = Vec::new();

        let final_aspect_ratio = if let Some(ar) = options.aspect_ratio {
            Some(ar.to_string())
        } else if let Some(size) = options.size {
            if let Some(ar) = size_to_aspect_ratio(&size) {
                warnings.push(unsupported("size", "Deriving aspect_ratio from size. Use the width and height provider options to specify dimensions for models that support them."));
                Some(ar)
            } else {
                None
            }
        } else {
            None
        };

        if options.size.is_some() && options.aspect_ratio.is_some() {
            warnings.push(unsupported("size", "Black Forest Labs ignores size when aspectRatio is provided. Use the width and height provider options to specify dimensions for models that support them"));
        }

        let bfl_opts = namespaced_options(Some(&options.provider_options), NAMESPACE);
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
            return Err(AiMuxError::invalid_argument(
                "Black Forest Labs supports up to 10 input images.",
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
        if let Some(bfl) = bfl_opts {
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
                if is_poll_control_key(key) || matches!(key.as_str(), "width" | "height") {
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

        let exchange = self.config.exchange(options.headers.as_ref()).await?;

        // Submit. This is the only request that creates a generation: nothing
        // below sends it again.
        let resp = aimux_provider_utils::post_json_to_api(
            exchange
                .request(exchange.url(&format!("/{}", self.model_id)), options)
                .generation_for(options),
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
                AiMuxError::invalid_response_data("missing polling_url in BFL response")
            })?
            .to_string();
        let request_id = submit_body
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let poll_interval = Duration::from_millis(poll_interval_ms(
            bfl_opts,
            POLL_INTERVAL_MILLIS_KEY,
            POLL_INTERVAL_MS,
        ));

        let mut poll_url_with_id =
            url::Url::parse(&poll_url).map_err(|e| AiMuxError::InvalidResponseData {
                message: format!("invalid BFL poll URL: {e}"),
                cause: Some(ErrorCause::source(&e)),
            })?;
        if !poll_url_with_id.query_pairs().any(|(k, _)| k == "id") {
            poll_url_with_id
                .query_pairs_mut()
                .append_pair("id", &request_id);
        }
        let poll_url = poll_url_with_id.to_string();

        // AI SDK gates headers per URL via isTrustedUrl; response-supplied
        // targets outside the BFL allowlist get none.
        let gated_request = |url: &str| -> HttpRequest {
            let mut request = exchange.request(url.to_string(), options);
            // Headers are gated per URL here, not by the credentialed origin.
            request.credentialed_origin = None;
            if !bfl_trusted_url(url, exchange.base_url()) {
                request.headers = Vec::new();
            }
            request.validate_url = true;
            request.trusted_origin = Some(exchange.base_url().to_string());
            request
        };

        // Poll for the result. AI SDK polls polling_url with validateUrl: true
        // and gates the headers itself via isTrustedUrl (base_url origin or
        // HTTPS *.bfl.ai).
        let ready = poll_until(
            &format!("blackForestLabs task {request_id}"),
            options.abort_signal.as_ref(),
            poll_interval,
            MAX_POLL_ATTEMPTS,
            || async {
                let pr = aimux_provider_utils::get_from_api(
                    gated_request(&poll_url),
                    aimux_provider_utils::create_json_response_handler::<Value>(),
                    bfl_failed_response_handler(),
                )
                .await?;
                let response_body = pr.raw_value.as_ref().map(ToString::to_string);
                let pv = pr.value;

                let poll_status = pv
                    .get("status")
                    .and_then(|v| v.as_str())
                    .or_else(|| pv.get("state").and_then(|v| v.as_str()))
                    .unwrap_or("");
                if poll_status == "Ready" {
                    let result = pv.get("result");
                    let sample = result
                        .and_then(|r| r.get("sample"))
                        .and_then(|v| v.as_str())
                        .map(String::from)
                        .ok_or_else(|| {
                            AiMuxError::invalid_response_data(
                                "BFL poll reported Ready without result.sample".to_string(),
                            )
                        })?;
                    return Ok(PollStep::Ready(BflResult {
                        image_url: sample,
                        seed: result.and_then(|r| r.get("seed")).cloned(),
                        start_time: result.and_then(|r| r.get("start_time")).cloned(),
                        end_time: result.and_then(|r| r.get("end_time")).cloned(),
                        duration: result.and_then(|r| r.get("duration")).cloned(),
                    }));
                }
                if poll_status == "Error" || poll_status == "Failed" {
                    return Err(task_failed(
                        "Black Forest Labs generation failed.",
                        poll_url.clone(),
                        poll_status.to_string(),
                        response_body,
                    ));
                }
                Ok(PollStep::Pending)
            },
        )
        .await?;
        let image_url = ready.image_url;
        let result_seed = ready.seed;
        let result_start_time = ready.start_time;
        let result_end_time = ready.end_time;
        let result_duration = ready.duration;

        // Download image; result.sample is a URL from the poll response body.
        // AI SDK sends its headers to trusted BFL hosts on the download too,
        // gated by the same allowlist.
        let ir = retry_download(options.abort_signal.as_ref(), poll_interval, || {
            aimux_provider_utils::get_from_api(
                gated_request(&image_url),
                aimux_provider_utils::create_binary_response_handler(),
                bfl_failed_response_handler(),
            )
        })
        .await?;
        let image_bytes = ir.value.to_vec();
        let download_headers: HashMap<String, String> = HashMap::new();

        // Build provider metadata
        let mut metadata = aimux_core::shared::NamespaceMap::new();
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
        metadata.insert(NAMESPACE.into(), bfl_meta);

        Ok(ImageResult {
            images: ImageOutputs::Binary(vec![image_bytes]),
            warnings,
            provider_metadata: Some(metadata),
            response: image_response(
                aimux_core::Timestamp::now(),
                &self.model_id,
                download_headers,
            ),
            usage: None,
        })
    }
}
