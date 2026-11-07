//! The fal image model (`fal-image-model.ts`) and its options
//! (`fal-image-model-options.ts`, `fal-image-settings.ts`).

use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::image_model::{
    ImageCallOptions, ImageModel, ImageOutputs, ImageResponse, ImageResult,
};
use aimux_core::shared::Warning;
use aimux_provider_utils::HttpRequest;

use crate::shared::{
    EndpointConfig, POLL_INTERVAL_MS_KEY, is_poll_control_key, poll_interval_ms, retry_download,
};

use super::error::fal_failed_response_handler;
use super::options;

/// Milliseconds between two attempts to download a generated image
/// (`providerOptions.fal.pollIntervalMs` overrides it for one call).
const DOWNLOAD_RETRY_INTERVAL_MS: u64 = 1_000;

// ── Image model ──────────────────────────────────────────────────────────────

/// `falImageModelOptionsSchema` (fal-image-model-options.ts), checked
/// only: a loose object whose known keys, camelCase and their deprecated
/// snake_case forms, are `.nullish()` with the ranges and enums below. The
/// request is built from the raw namespace, as upstream's transform keeps
/// every unknown key.
#[derive(Debug, Deserialize)]
#[allow(
    dead_code,
    reason = "validated only; the body is built from the raw namespace"
)]
struct FalImageOptions {
    #[serde(default, rename = "imageUrl")]
    image_url: Option<String>,
    #[serde(default, rename = "maskUrl")]
    mask_url: Option<String>,
    #[serde(default, rename = "guidanceScale", deserialize_with = "guidance_scale")]
    guidance_scale: Option<f64>,
    #[serde(
        default,
        rename = "numInferenceSteps",
        deserialize_with = "num_inference_steps"
    )]
    num_inference_steps: Option<f64>,
    #[serde(default, rename = "enableSafetyChecker")]
    enable_safety_checker: Option<bool>,
    #[serde(default, rename = "outputFormat")]
    output_format: Option<FalOutputFormat>,
    #[serde(default, rename = "syncMode")]
    sync_mode: Option<bool>,
    #[serde(default)]
    strength: Option<f64>,
    #[serde(default)]
    acceleration: Option<FalAcceleration>,
    #[serde(
        default,
        rename = "safetyTolerance",
        deserialize_with = "safety_tolerance"
    )]
    safety_tolerance: Option<f64>,
    #[serde(default, rename = "useMultipleImages")]
    use_multiple_images: Option<bool>,
    #[serde(default, rename = "image_url")]
    image_url_snake: Option<String>,
    #[serde(default, rename = "mask_url")]
    mask_url_snake: Option<String>,
    #[serde(
        default,
        rename = "guidance_scale",
        deserialize_with = "guidance_scale"
    )]
    guidance_scale_snake: Option<f64>,
    #[serde(
        default,
        rename = "num_inference_steps",
        deserialize_with = "num_inference_steps"
    )]
    num_inference_steps_snake: Option<f64>,
    #[serde(default, rename = "enable_safety_checker")]
    enable_safety_checker_snake: Option<bool>,
    #[serde(default, rename = "output_format")]
    output_format_snake: Option<FalOutputFormat>,
    #[serde(default, rename = "sync_mode")]
    sync_mode_snake: Option<bool>,
    #[serde(
        default,
        rename = "safety_tolerance",
        deserialize_with = "safety_tolerance"
    )]
    safety_tolerance_snake: Option<f64>,
}

/// `outputFormat`: `z.enum(['jpeg', 'png'])`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum FalOutputFormat {
    Jpeg,
    Png,
}

/// `acceleration`: `z.enum(['none', 'regular', 'high'])`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum FalAcceleration {
    #[serde(rename = "none")]
    Off,
    Regular,
    High,
}

/// A nullish number within `min..=max`.
pub(super) fn number_in<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
    min: f64,
    max: f64,
) -> Result<Option<f64>, D::Error> {
    match Option::<f64>::deserialize(deserializer)? {
        Some(value) if !(min..=max).contains(&value) => Err(serde::de::Error::custom(format!(
            "expected a number from {min} to {max}"
        ))),
        value => Ok(value),
    }
}

/// `guidanceScale`: `z.number().min(1).max(20)`.
fn guidance_scale<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<f64>, D::Error> {
    number_in(deserializer, 1.0, 20.0)
}

/// `numInferenceSteps`: `z.number().min(1).max(50)`.
fn num_inference_steps<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<f64>, D::Error> {
    number_in(deserializer, 1.0, 50.0)
}

/// `safetyTolerance`: `z.enum(['1', ..., '6']).or(z.number().min(1).max(6))`.
fn safety_tolerance<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<f64>, D::Error> {
    match Option::<Value>::deserialize(deserializer)? {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(level)) if ["1", "2", "3", "4", "5", "6"].contains(&level.as_str()) => {
            Ok(level.parse().ok())
        }
        Some(Value::Number(level))
            if level
                .as_f64()
                .is_some_and(|level| (1.0..=6.0).contains(&level)) =>
        {
            Ok(level.as_f64())
        }
        Some(_) => Err(serde::de::Error::custom(
            "expected '1' to '6' or a number from 1 to 6",
        )),
    }
}

/// The image options with a deprecated snake_case form, in the order the
/// schema's transform maps them: the camelCase key and the body key, which
/// is the snake_case one.
const FAL_IMAGE_RENAMED_KEYS: [(&str, &str); 8] = [
    ("imageUrl", "image_url"),
    ("maskUrl", "mask_url"),
    ("guidanceScale", "guidance_scale"),
    ("numInferenceSteps", "num_inference_steps"),
    ("enableSafetyChecker", "enable_safety_checker"),
    ("outputFormat", "output_format"),
    ("syncMode", "sync_mode"),
    ("safetyTolerance", "safety_tolerance"),
];

/// Models with an inpainting endpoint, which take a mask
/// (fal-image-model.ts `supportsMaskInputs`).
const FAL_MASK_MODELS: [&str; 2] = [
    "fal-ai/flux-general/inpainting",
    "fal-ai/flux-lora/inpainting",
];

/// Image-to-image models, which take an input file (fal-image-model.ts
/// `supportsFileInputs`).
const FAL_FILE_MODELS: [&str; 10] = [
    "fal-ai/flux-2/edit",
    "fal-ai/flux-pro/kontext",
    "fal-ai/flux-pro/kontext/max",
    "fal-ai/flux-general/image-to-image",
    "fal-ai/flux-general/inpainting",
    "fal-ai/flux-lora/image-to-image",
    "fal-ai/flux-lora/inpainting",
    "fal-ai/flux/dev/image-to-image",
    "fal-ai/flux/krea/image-to-image",
    "fal-ai/recraft/v3/image-to-image",
];

/// Text-to-image models that do not take an input file
/// (fal-image-model.ts `supportsFileInputs`).
const FAL_TEXT_ONLY_MODELS: [&str; 9] = [
    "bria/text-to-image/3.2",
    "fal-ai/bria/text-to-image/base",
    "fal-ai/bria/text-to-image/fast",
    "fal-ai/bria/text-to-image/hd",
    "fal-ai/bytedance/dreamina/v3.1/text-to-image",
    "fal-ai/flux-kontext-lora/text-to-image",
    "fal-ai/recraft/v3/text-to-image",
    "fal-ai/wan/v2.2-5b/text-to-image",
    "fal-ai/wan/v2.2-a14b/text-to-image",
];

/// An fal.ai image generation model.
pub struct FalImageModel {
    model_id: String,
    config: EndpointConfig,
}

impl FalImageModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }

    fn endpoint_path(&self) -> String {
        format!("/{}", self.model_id)
    }

    fn convert_aspect_ratio_to_size(ar: &aimux_core::shared::AspectRatio) -> Option<Value> {
        Some(match (ar.width(), ar.height()) {
            (1, 1) => json!("square_hd"),
            (16, 9) => json!("landscape_16_9"),
            (9, 16) => json!("portrait_16_9"),
            (4, 3) => json!("landscape_4_3"),
            (3, 4) => json!("portrait_4_3"),
            (16, 10) => json!({ "width": 1280, "height": 800 }),
            (10, 16) => json!({ "width": 800, "height": 1280 }),
            (21, 9) => json!({ "width": 2560, "height": 1080 }),
            (9, 21) => json!({ "width": 1080, "height": 2560 }),
            _ => return None,
        })
    }
}

#[async_trait]
impl ImageModel for FalImageModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }
    fn model_id(&self) -> &str {
        &self.model_id
    }
    fn max_images_per_call(&self) -> Option<u32> {
        Some(1)
    }

    /// Whether the model takes an input file; `None` for an unknown model.
    fn supports_file_inputs(&self) -> Option<bool> {
        if FAL_FILE_MODELS.contains(&self.model_id.as_str()) {
            Some(true)
        } else if FAL_TEXT_ONLY_MODELS.contains(&self.model_id.as_str()) {
            Some(false)
        } else {
            None
        }
    }

    /// Whether the model takes a mask: the inpainting models do, a model
    /// known not to take files does not, an unknown one says nothing.
    fn supports_mask_inputs(&self) -> Option<bool> {
        if FAL_MASK_MODELS.contains(&self.model_id.as_str()) {
            Some(true)
        } else if self.supports_file_inputs().is_none() {
            None
        } else {
            Some(false)
        }
    }

    async fn do_generate(&self, options: &ImageCallOptions) -> Result<ImageResult, AiMuxError> {
        let mut warnings: Vec<Warning> = Vec::new();
        // `image_size` is undefined, so not sent, without a size or for an
        // aspect ratio `convertAspectRatioToSize` does not map.
        let image_size = if let Some(s) = options.size {
            Some(json!({ "width": s.width(), "height": s.height() }))
        } else {
            options
                .aspect_ratio
                .and_then(|ar| Self::convert_aspect_ratio_to_size(&ar))
        };

        let mut body = Map::new();
        if let Some(ref p) = options.prompt {
            body.insert("prompt".into(), json!(p));
        }
        if let Some(seed) = options.seed {
            body.insert("seed".into(), json!(seed));
        }
        if let Some(image_size) = image_size {
            body.insert("image_size".into(), image_size);
        }
        body.insert("num_images".into(), json!(options.n));

        // fal-image-model.ts doGenerate: parseProviderOptions with
        // falImageModelOptionsSchema before the body is built.
        aimux_provider_utils::parse_provider_options::<FalImageOptions>(
            options::NAMESPACE,
            Some(&options.provider_options),
        )?;
        let fal_opts = options::fal_options(Some(&options.provider_options));

        if let Some(ref files) = options.files
            && !files.is_empty()
        {
            let multi = fal_opts
                .and_then(|o| o.get("useMultipleImages"))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            if multi {
                let uris: Vec<String> = files
                    .iter()
                    .map(aimux_provider_utils::convert_image_model_file_to_data_uri)
                    .collect();
                body.insert("image_urls".into(), json!(uris));
            } else {
                body.insert(
                    "image_url".into(),
                    json!(aimux_provider_utils::convert_image_model_file_to_data_uri(
                        &files[0]
                    )),
                );
                if files.len() > 1 {
                    warnings.push(Warning::Other {
                        message: "Multiple input images provided but useMultipleImages is not enabled. Only the first image will be used. Set providerOptions.fal.useMultipleImages to true for models that support multiple images (e.g., fal-ai/flux-2/edit)."
                            .into(),
                    });
                }
            }
        }
        if let Some(ref mask) = options.mask {
            body.insert(
                "mask_url".into(),
                json!(aimux_provider_utils::convert_image_model_file_to_data_uri(
                    mask
                )),
            );
        }

        if let Some(fal) = fal_opts {
            // The schema's transform (fal-image-model-options.ts): a
            // non-null snake_case key wins over its camelCase twin and is
            // reported as deprecated; a null known key is dropped. Every
            // other key passes through, `useMultipleImages` excepted.
            let set = |key: &str| fal.get(key).filter(|value| !value.is_null());
            let mut deprecated_keys = Vec::new();
            for (camel_key, snake_key) in FAL_IMAGE_RENAMED_KEYS {
                if let Some(value) = set(snake_key) {
                    deprecated_keys.push(snake_key);
                    body.insert(snake_key.to_string(), value.clone());
                } else if let Some(value) = set(camel_key) {
                    body.insert(snake_key.to_string(), value.clone());
                }
            }
            for key in ["strength", "acceleration"] {
                if let Some(value) = set(key) {
                    body.insert(key.to_string(), value.clone());
                }
            }
            for (key, value) in fal {
                let known = key == "strength"
                    || key == "acceleration"
                    || key == "useMultipleImages"
                    || FAL_IMAGE_RENAMED_KEYS
                        .iter()
                        .any(|(camel_key, snake_key)| key == camel_key || key == snake_key);
                if !known && key != "__deprecatedKeys" && !is_poll_control_key(key) {
                    body.insert(key.clone(), value.clone());
                }
            }
            if !deprecated_keys.is_empty() {
                let keys = deprecated_keys
                    .iter()
                    .map(|snake_key| {
                        let camel_key = FAL_IMAGE_RENAMED_KEYS
                            .iter()
                            .find(|(_, snake)| snake == snake_key)
                            .map_or(*snake_key, |(camel, _)| *camel);
                        format!("'{snake_key}' (use '{camel_key}')")
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                warnings.push(Warning::Other {
                    message: format!(
                        "The following provider options use deprecated snake_case and will be removed in @ai-sdk/fal v2.0. Please use camelCase instead: {keys}"
                    ),
                });
            }
        }

        let exchange = self.config.exchange(options.headers.as_ref()).await?;

        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(exchange.url(&self.endpoint_path()), options),
            Value::Object(body),
            aimux_provider_utils::create_json_response_handler(),
            fal_failed_response_handler(),
        )
        .await?;

        let rh = resp.response_headers;
        let rb: Value = resp.value;
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
                let ir = retry_download(
                    options.abort_signal.as_ref(),
                    Duration::from_millis(poll_interval_ms(
                        fal_opts,
                        POLL_INTERVAL_MS_KEY,
                        DOWNLOAD_RETRY_INTERVAL_MS,
                    )),
                    || {
                        aimux_provider_utils::get_from_api(
                            HttpRequest {
                                url: url.to_string(),
                                abort_signal: options.abort_signal.clone(),
                                validate_url: true,
                                trusted_origin: Some(exchange.base_url().to_string()),
                                credentialed_origin: Some(exchange.base_url().to_string()),
                                ..Default::default()
                            },
                            aimux_provider_utils::create_binary_response_handler(),
                            fal_failed_response_handler(),
                        )
                    },
                )
                .await?;
                downloaded.push(ir.value.to_vec());
            }
        }

        let mut metadata = aimux_core::shared::NamespaceMap::new();
        let mut fm = Map::new();
        // `nsfw` of image `index`: `has_nsfw_concepts?.[index] ??
        // nsfw_content_detected?.[index]`, left out when neither has it.
        let nsfw_flag = |key: &str, index: usize| {
            rb.get(key)
                .and_then(|flags| flags.get(index))
                .filter(|flag| !flag.is_null())
                .cloned()
        };
        let im: Vec<Value> = target_images
            .iter()
            .enumerate()
            .map(|(index, img)| {
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
                if let Some(nsfw) = nsfw_flag("has_nsfw_concepts", index)
                    .or_else(|| nsfw_flag("nsfw_content_detected", index))
                {
                    e.insert("nsfw".into(), nsfw);
                }
                Value::Object(e)
            })
            .collect();
        fm.insert("images".into(), json!(im));
        if let Some(o) = rb.as_object() {
            for (k, v) in o {
                if !matches!(
                    k.as_str(),
                    "images" | "prompt" | "has_nsfw_concepts" | "nsfw_content_detected"
                ) {
                    fm.insert(k.clone(), v.clone());
                }
            }
        }
        metadata.insert(options::NAMESPACE.into(), fm);

        Ok(ImageResult {
            images: ImageOutputs::Binary(downloaded),
            warnings,
            provider_metadata: Some(metadata),
            response: ImageResponse {
                timestamp: Some(aimux_core::Timestamp::now()),
                model_id: Some(self.model_id.clone()),
                headers: Some(rh),
            },
            usage: None,
        })
    }
}
