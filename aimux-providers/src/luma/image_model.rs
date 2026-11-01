//! The Luma image model (`luma-image-model.ts`, with the model ids and
//! reference types of `luma-image-settings.ts`).

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::image_model::{
    ImageCallOptions, ImageFile, ImageModel, ImageOutputs, ImageResponse, ImageResult,
};
use aimux_core::shared::Warning;
use aimux_provider_utils::HttpRequest;

use crate::shared::{
    EndpointConfig, POLL_INTERVAL_MILLIS_KEY, PollStep, is_poll_control_key, poll_interval_ms,
    poll_until, retry_download,
};

use super::options;

fn luma_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    aimux_provider_utils::create_json_error_response_handler(|data| {
        let message = data
            .get("detail")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(|item| item.get("msg"))
            .and_then(Value::as_str)
            .unwrap_or("Unknown Luma error")
            .to_string();
        aimux_provider_utils::ProviderErrorParts {
            message,
            provider_code: None,
        }
    })
}

/// `lumaGenerationResponseSchema` (luma-image-model.ts), the submit and poll
/// answers: what the implementation reads, checked.
#[derive(serde::Deserialize)]
struct LumaGenerationResponse {
    id: String,
    state: GenerationState,
    #[serde(default)]
    assets: Option<LumaAssets>,
}

/// `z.enum(['queued', 'dreaming', 'completed', 'failed'])`.
#[derive(serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum GenerationState {
    Queued,
    Dreaming,
    Completed,
    Failed,
}

#[derive(serde::Deserialize)]
struct LumaAssets {
    /// The URL of the generated image.
    image: String,
}

/// Milliseconds between two polls of a generation (`providerOptions.luma.pollIntervalMillis`
/// overrides it for one call).
const POLL_INTERVAL_MS: u64 = 500;
/// How many times a generation is polled before the call gives up.
const MAX_POLL_ATTEMPTS: u32 = 120;

/// A Luma image generation model.
pub struct LumaImageModel {
    model_id: String,
    config: EndpointConfig,
}

impl LumaImageModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }
}

#[async_trait]
impl ImageModel for LumaImageModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }
    fn model_id(&self) -> &str {
        &self.model_id
    }
    fn max_images_per_call(&self) -> Option<u32> {
        Some(1)
    }
    /// `photon-1` and `photon-flash-1` take input files; other models are
    /// unknown (luma-image-model.ts `supportsFileInputs`).
    fn supports_file_inputs(&self) -> Option<bool> {
        matches!(self.model_id.as_str(), "photon-1" | "photon-flash-1").then_some(true)
    }
    /// No mask on the models that take files; unknown on the others.
    fn supports_mask_inputs(&self) -> Option<bool> {
        (self.supports_file_inputs() == Some(true)).then_some(false)
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

        options::validate(Some(&options.provider_options))?;
        let luma_opts = options::luma_options(Some(&options.provider_options));

        // Extract non-request options
        let poll_interval = Duration::from_millis(poll_interval_ms(
            luma_opts,
            POLL_INTERVAL_MILLIS_KEY,
            POLL_INTERVAL_MS,
        ));
        let reference_type = luma_opts
            .and_then(|o| o.get("referenceType"))
            .and_then(|v| v.as_str())
            .unwrap_or("image");
        let image_configs = luma_opts
            .and_then(|o| o.get("images"))
            .and_then(|v| v.as_array())
            .cloned();

        // Build editing options from files (luma-image-model.ts
        // getEditingOptions): the mask is refused first, files or not.
        if options.mask.is_some() {
            return Err(AiMuxError::invalid_argument(
                "Luma AI does not support mask-based image editing. Use the prompt to describe the changes you want to make, along with `prompt.images` containing the source image URL.",
            ));
        }
        let mut editing_opts = Map::new();
        if let Some(ref files) = options.files
            && !files.is_empty()
        {
            // Validate all files are URL-based
            for file in files {
                if !matches!(file, ImageFile::Url { .. }) {
                    return Err(AiMuxError::invalid_argument(
                        "Luma AI only supports URL-based images. Please provide image URLs using `prompt.images` with publicly accessible URLs. Base64 and Uint8Array data are not supported.",
                    ));
                }
            }

            let default_weight = match reference_type {
                "style" => 0.8,
                "modify_image" => 1.0,
                _ => 0.85, // image; character sends no weight
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
                "character" => {
                    // luma-image-model.ts getEditingOptions: the images grouped by
                    // `images[i].id` (default `identity0`), at most 4 per identity.
                    let mut identities: Map<String, Value> = Map::new();
                    for (i, f) in files.iter().enumerate() {
                        let url = if let ImageFile::Url { url } = f {
                            url.clone()
                        } else {
                            String::new()
                        };
                        let identity = image_configs
                            .as_ref()
                            .and_then(|c| c.get(i))
                            .and_then(|c| c.get("id"))
                            .and_then(Value::as_str)
                            .unwrap_or("identity0");
                        let images = identities
                            .entry(identity.to_string())
                            .or_insert_with(|| json!({ "images": [] }));
                        if let Some(list) = images["images"].as_array_mut() {
                            list.push(json!(url));
                        }
                    }
                    for (identity, images) in &identities {
                        let count = images["images"].as_array().map_or(0, Vec::len);
                        if count > 4 {
                            return Err(AiMuxError::invalid_argument(format!(
                                "Luma AI character supports up to 4 images per identity. Identity '{identity}' has {count} images."
                            )));
                        }
                    }
                    editing_opts.insert("character".into(), Value::Object(identities));
                }
                "modify_image" => {
                    if files.len() > 1 {
                        return Err(AiMuxError::invalid_argument(format!(
                            "Luma AI modify_image only supports a single input image. You provided {} images.",
                            files.len()
                        )));
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
                        return Err(AiMuxError::invalid_argument(format!(
                            "Luma AI image supports up to 4 reference images. You provided {} images.",
                            files.len()
                        )));
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
        if let Some(luma) = luma_opts {
            for (k, v) in luma {
                if is_poll_control_key(k) || matches!(k.as_str(), "referenceType" | "images") {
                    continue;
                }
                body.insert(k.clone(), v.clone());
            }
        }

        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let generations_url = |generation_id: Option<&str>| match generation_id {
            Some(id) => exchange.url(&format!("/dream-machine/v1/generations/{id}")),
            None => exchange.url("/dream-machine/v1/generations/image"),
        };

        // Submit. This is the only request that creates a generation: nothing
        // below sends it again.
        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(generations_url(None), options),
            Value::Object(body),
            aimux_provider_utils::create_json_response_handler::<LumaGenerationResponse>(),
            luma_failed_response_handler(),
        )
        .await?;
        let rh = resp.response_headers;
        let generation_id = resp.value.id;

        // Poll for completion.
        let poll_url = generations_url(Some(&generation_id));
        let image_url = poll_until(
            &format!("luma generation {generation_id}"),
            options.abort_signal.as_ref(),
            poll_interval,
            MAX_POLL_ATTEMPTS,
            || async {
                let pr = aimux_provider_utils::get_from_api(
                    exchange.request(poll_url.clone(), options),
                    aimux_provider_utils::create_json_response_handler::<LumaGenerationResponse>(),
                    luma_failed_response_handler(),
                )
                .await?;
                // luma-image-model.ts pollForImageUrl
                match pr.value.state {
                    GenerationState::Completed => {
                        return pr
                            .value
                            .assets
                            .map(|assets| PollStep::Ready(assets.image))
                            .ok_or_else(|| {
                                AiMuxError::invalid_response_data(
                                    "Image generation completed but no image was found.",
                                )
                            });
                    }
                    GenerationState::Failed => {
                        return Err(AiMuxError::invalid_response_data(
                            "Image generation failed.",
                        ));
                    }
                    GenerationState::Queued | GenerationState::Dreaming => {}
                }
                Ok(PollStep::Pending)
            },
        )
        .await?;

        // Download image; assets.image is a URL from the poll response body,
        // so it goes through the SSRF download guard.
        let ir = retry_download(options.abort_signal.as_ref(), poll_interval, || {
            aimux_provider_utils::get_from_api(
                HttpRequest {
                    url: image_url.clone(),
                    abort_signal: options.abort_signal.clone(),
                    validate_url: true,
                    trusted_origin: Some(exchange.base_url().to_string()),
                    credentialed_origin: Some(exchange.base_url().to_string()),
                    ..Default::default()
                },
                aimux_provider_utils::create_binary_response_handler(),
                aimux_provider_utils::create_status_code_error_response_handler(),
            )
        })
        .await?;
        let image_bytes = ir.value.to_vec();

        Ok(ImageResult {
            images: ImageOutputs::Binary(vec![image_bytes]),
            warnings,
            provider_metadata: None,
            response: ImageResponse {
                timestamp: Some(aimux_core::Timestamp::now()),
                model_id: Some(self.model_id.clone()),
                headers: Some(rh),
            },
            usage: None,
        })
    }
}
