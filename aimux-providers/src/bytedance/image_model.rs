//! Seedream generation and editing share the JSON images endpoint.

use super::options::{
    check_types, failed_response, http_url, image_url, no_replay,
    not_retryable_unless_rate_limited, options,
};
use crate::shared::{EndpointConfig, image_response, retry_download};
use aimux_core::{
    AiMuxError,
    image_model::{ImageCallOptions, ImageFile, ImageModel, ImageOutputs, ImageResult, ImageUsage},
    shared::{SharedProviderMetadata, Warning},
};
use aimux_provider_utils::HttpRequest;
use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{Value, json};
use std::time::Duration;

const ALLOWED: &[&str] = &[
    "watermark",
    "background",
    "sequential_image_generation",
    "sequential_image_generation_options",
    "optimize_prompt_options",
    "guidance_scale",
    "stream",
    "response_format",
    "size",
];

/// How long a failed asset download waits before its next attempt.
const DOWNLOAD_RETRY_INTERVAL_MS: u64 = 1_000;

/// A Seedream image model supporting JSON reference images and streamed results.
pub struct ByteDanceImageModel {
    model_id: String,
    config: EndpointConfig,
}
impl ByteDanceImageModel {
    pub(crate) fn new(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }

    fn body(&self, call: &ImageCallOptions) -> Result<Value, AiMuxError> {
        if call.n != 1 {
            return Err(AiMuxError::invalid_argument(
                "Seedream accepts one generation per call; use sequential_image_generation for groups",
            ));
        }
        if call.mask.is_some() {
            return Err(AiMuxError::invalid_argument(
                "Seedream does not support masks",
            ));
        }
        let prompt = call
            .prompt
            .as_deref()
            .filter(|v| !v.trim().is_empty())
            .ok_or_else(|| AiMuxError::invalid_argument("Seedream requires a prompt"))?;
        let mut body = options(&call.provider_options, ALLOWED)?;
        check_types(
            &body,
            &[
                "background",
                "sequential_image_generation",
                "response_format",
                "size",
            ],
            &[],
            &["watermark", "stream"],
            &[
                "sequential_image_generation_options",
                "optimize_prompt_options",
            ],
        )?;
        if body
            .get("guidance_scale")
            .is_some_and(|value| !value.is_number())
        {
            return Err(AiMuxError::invalid_argument(
                "guidance_scale must be a number",
            ));
        }
        if let Some(format) = body.get("response_format").and_then(Value::as_str)
            && !matches!(format, "url" | "b64_json")
        {
            return Err(AiMuxError::invalid_argument(
                "response_format must be url or b64_json",
            ));
        }
        if let Some(mode) = body
            .get("sequential_image_generation")
            .and_then(Value::as_str)
            && !matches!(mode, "auto" | "disabled")
        {
            return Err(AiMuxError::invalid_argument(
                "sequential_image_generation must be auto or disabled",
            ));
        }
        if let Some(size) = call.size {
            if body.contains_key("size") {
                return Err(AiMuxError::invalid_argument("size was specified twice"));
            }
            body.insert("size".into(), json!(size.to_string()));
        }
        if let Some(seed) = call.seed {
            body.insert("seed".into(), json!(seed));
        }
        // `response_format` is left to Ark's `url` default: a sequential 4K
        // group's base64 can exceed the JSON response budget.
        body.entry("stream").or_insert(json!(false));
        body.insert("model".into(), json!(self.model_id));
        body.insert("prompt".into(), json!(prompt));
        if let Some(files) = &call.files {
            if files.is_empty() {
                return Err(AiMuxError::invalid_argument(
                    "Seedream editing requires at least one reference image",
                ));
            }
            let mut images = Vec::new();
            for file in files {
                let url = match file {
                    ImageFile::Url { url } => url.clone(),
                    ImageFile::File { media_type, data } => {
                        if !media_type.starts_with("image/") {
                            return Err(AiMuxError::invalid_argument(
                                "Seedream reference files must be images",
                            ));
                        }
                        format!(
                            "data:{media_type};base64,{}",
                            aimux_provider_utils::convert_to_base64(data)
                        )
                    }
                };
                image_url(&url)?;
                images.push(url);
            }
            body.insert(
                "image".into(),
                if images.len() == 1 {
                    json!(images[0])
                } else {
                    json!(images)
                },
            );
        }
        Ok(Value::Object(body))
    }
}

#[async_trait]
impl ImageModel for ByteDanceImageModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }
    fn model_id(&self) -> &str {
        &self.model_id
    }
    fn max_images_per_call(&self) -> Option<u32> {
        Some(1)
    }
    fn supports_file_inputs(&self) -> Option<bool> {
        Some(true)
    }
    fn supports_mask_inputs(&self) -> Option<bool> {
        Some(false)
    }

    async fn do_generate(&self, call: &ImageCallOptions) -> Result<ImageResult, AiMuxError> {
        let body = self.body(call)?;
        let streaming = body["stream"] == true;
        let exchange = self.config.exchange(call.headers.as_ref()).await?;
        let mut request = exchange.request(exchange.url("/images/generations"), call);
        request.response_timeout = Some(Duration::from_secs(600));
        let body = exchange.transform_body(body);
        let (value, headers) = if streaming {
            request
                .headers
                .push(("accept".into(), "text/event-stream".into()));
            let response = aimux_provider_utils::post_json_to_api(
                request,
                body,
                aimux_provider_utils::create_event_source_response_handler::<Value>(),
                failed_response(),
            )
            .await
            .map_err(not_retryable_unless_rate_limited)?;
            let mut stream = response.value;
            let mut images = Vec::new();
            let mut completed = None;
            while let Some(event) = stream.next().await {
                let event = event.map_err(no_replay)?;
                if event.get("error").is_some_and(|value| !value.is_null()) {
                    return Err(AiMuxError::invalid_response_data(
                        event["error"]["message"]
                            .as_str()
                            .unwrap_or("Seedream streaming generation failed"),
                    ));
                }
                match event["type"].as_str() {
                    Some("image_generation.partial_succeeded") => images.push(event),
                    Some("image_generation.completed") => {
                        completed = Some(event);
                        break;
                    }
                    Some("image_generation.partial_failed" | "error") => {
                        return Err(AiMuxError::invalid_response_data(
                            event["error"]["message"]
                                .as_str()
                                .unwrap_or("Seedream streaming generation failed"),
                        ));
                    }
                    _ => {}
                }
            }
            let completed = completed.ok_or_else(|| {
                AiMuxError::invalid_response_data("Seedream stream ended without completion")
            })?;
            (
                json!({"data":images,"usage":completed["usage"]}),
                response.response_headers,
            )
        } else {
            let response = aimux_provider_utils::post_json_to_api(
                request,
                body,
                aimux_provider_utils::create_json_response_handler::<Value>(),
                failed_response(),
            )
            .await
            .map_err(not_retryable_unless_rate_limited)?;
            (response.value, response.response_headers)
        };
        let data = value["data"]
            .as_array()
            .filter(|v| !v.is_empty())
            .ok_or_else(|| AiMuxError::invalid_response_data("Seedream returned no images"))?;
        let mut images = Vec::new();
        for item in data {
            if item.get("error").is_some_and(|v| !v.is_null()) {
                return Err(AiMuxError::invalid_response_data(
                    item["error"]["message"]
                        .as_str()
                        .unwrap_or("Seedream image generation failed"),
                ));
            }
            let bytes = if let Some(base64) = item["b64_json"].as_str() {
                aimux_provider_utils::convert_base64_to_uint8_array(base64).map_err(|_| {
                    AiMuxError::invalid_response_data("Seedream returned invalid base64")
                })?
            } else if let Some(url) = item["url"].as_str() {
                http_url(url).map_err(|_| {
                    AiMuxError::invalid_response_data("Seedream returned a non-HTTP image URL")
                })?;
                // The generation is already billed, so a transient download
                // failure is retried here; only an exhausted download becomes
                // a non-retryable error that never replays the generation.
                retry_download(
                    call.abort_signal.as_ref(),
                    Duration::from_millis(DOWNLOAD_RETRY_INTERVAL_MS),
                    || {
                        aimux_provider_utils::get_from_api(
                            HttpRequest {
                                url: url.to_owned(),
                                abort_signal: call.abort_signal.clone(),
                                fetch: self.config.fetch.clone(),
                                validate_url: true,
                                trusted_origin: Some(exchange.base_url().to_owned()),
                                ..Default::default()
                            },
                            aimux_provider_utils::create_binary_response_handler(),
                            aimux_provider_utils::create_status_code_error_response_handler(),
                        )
                    },
                )
                .await
                .map_err(no_replay)?
                .value
                .to_vec()
            } else {
                return Err(AiMuxError::invalid_response_data(
                    "Seedream image has neither url nor b64_json",
                ));
            };
            if bytes.is_empty() {
                return Err(AiMuxError::invalid_response_data(
                    "Seedream returned an empty image",
                ));
            }
            images.push(bytes);
        }
        let mut metadata = SharedProviderMetadata::new();
        let details = data
            .iter()
            .map(|item| {
                let mut item = item.clone();
                if let Some(object) = item.as_object_mut() {
                    object.remove("b64_json");
                    object.remove("url");
                }
                item
            })
            .collect::<Vec<_>>();
        metadata.insert(
            "bytedance".into(),
            serde_json::Map::from_iter([("images".into(), json!(details))]),
        );
        let warnings = if call.aspect_ratio.is_some() {
            vec![Warning::Unsupported {
                feature: "aspectRatio".into(),
                details: Some("Set Seedream size instead.".into()),
            }]
        } else {
            Vec::new()
        };
        Ok(ImageResult {
            images: ImageOutputs::Binary(images),
            warnings,
            provider_metadata: Some(metadata),
            response: image_response(aimux_core::Timestamp::now(), &self.model_id, headers),
            usage: value
                .get("usage")
                .filter(|v| v.is_object())
                .map(|usage| ImageUsage {
                    input_tokens: usage["input_tokens"]
                        .as_u64()
                        .and_then(|v| v.try_into().ok()),
                    output_tokens: usage["output_tokens"]
                        .as_u64()
                        .and_then(|v| v.try_into().ok()),
                    total_tokens: usage["total_tokens"]
                        .as_u64()
                        .and_then(|v| v.try_into().ok()),
                }),
        })
    }
}
