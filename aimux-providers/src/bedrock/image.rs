//! Amazon Bedrock image model — implements the `ImageModel` trait.
//!
//! Aligned with Vercel AI SDK `AmazonBedrockImageModel`
//! (`reference/ai/packages/amazon-bedrock/src/amazon-bedrock-image-model.ts`).
//!
//! Endpoint: `POST {base_url}/model/{model_id}/invoke`

use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::error::{AiMuxError, ApiCallError};
use aimux_core::image_model::{
    ImageCallOptions, ImageFile, ImageFileData, ImageModel, ImageOutputs, ImageResponse,
    ImageResult,
};
use aimux_core::shared::Warning;

use aimux_provider_utils::{HttpBody, HttpRequest};

use super::sigv4::sign_request;
use super::{BedrockAuth, BedrockConfig};

/// Returns the max images per call for the given model ID.
fn get_max_images_per_call(model_id: &str) -> u32 {
    match model_id {
        "amazon.titan-image-generator-v1" | "amazon.titan-image-generator-v2:0" => 5,
        "amazon.nova-canvas-v1:0" => 5,
        _ => 1,
    }
}

/// An Amazon Bedrock image generation model.
///
/// Does **not** hold an HTTP client — the `aimux-provider-utils` API helpers use the process-wide shared
/// `Client` internally (RFC-0009 §4.1).
pub struct BedrockImageModel {
    model_id: String,
    config: BedrockConfig,
}

impl BedrockImageModel {
    #[must_use]
    pub fn new(model_id: String, config: BedrockConfig) -> Self {
        Self { model_id, config }
    }

    fn endpoint(&self) -> String {
        format!("{}/model/{}/invoke", self.config.base_url, self.model_id)
    }

    fn build_headers(
        &self,
        body: &str,
        url: &str,
        extra: Option<&HashMap<String, String>>,
    ) -> Result<Vec<(String, String)>, AiMuxError> {
        let mut extra_headers: Vec<(String, String)> = Vec::new();
        if let Some(extra) = extra {
            for (k, v) in extra {
                extra_headers.push((k.clone(), v.clone()));
            }
        }
        match &self.config.auth {
            BedrockAuth::BearerToken(token) => {
                let mut headers = vec![("Authorization".into(), format!("Bearer {token}"))];
                headers.extend(extra_headers);
                Ok(headers)
            }
            BedrockAuth::SigV4(creds) => {
                let signed = sign_request(creds, "bedrock", "POST", url, body, &extra_headers);
                let mut headers: Vec<(String, String)> = Vec::new();
                for (k, v) in &signed.headers {
                    headers.push((k.clone(), v.clone()));
                }
                Ok(headers)
            }
        }
    }

    fn get_base64_data(file: &ImageFile) -> Result<String, AiMuxError> {
        match file {
            ImageFile::Url { .. } => Err(AiMuxError::InvalidArgument(
                "URL-based images are not supported for Amazon Bedrock image editing.".into(),
            )),
            ImageFile::File { data, .. } => match data {
                ImageFileData::Base64(s) => Ok(s.clone()),
                ImageFileData::Binary(b) => Ok(base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    b,
                )),
            },
        }
    }
}

#[async_trait]
impl ImageModel for BedrockImageModel {
    fn provider(&self) -> &str {
        "amazon-bedrock"
    }
    fn model_id(&self) -> &str {
        &self.model_id
    }
    fn retry_config(&self) -> aimux_core::retry::RetryConfig {
        self.config.retry_config
    }
    fn max_images_per_call(&self) -> Option<u32> {
        Some(get_max_images_per_call(&self.model_id))
    }

    async fn do_generate(&self, options: &ImageCallOptions) -> Result<ImageResult, AiMuxError> {
        let mut warnings = Vec::new();
        let (width, height) = options
            .size
            .map(|s| (Some(s.width()), Some(s.height())))
            .unwrap_or((None, None));
        let has_files = options.files.as_ref().is_some_and(|f| !f.is_empty());

        // Parse provider options (amazonBedrock or bedrock)
        let ab_opts = options
            .provider_options
            .get("amazonBedrock")
            .or_else(|| options.provider_options.get("bedrock"));

        // Build image generation config
        let mut image_gen_config = Map::new();
        if let Some(w) = width {
            image_gen_config.insert("width".into(), json!(w));
        }
        if let Some(h) = height {
            image_gen_config.insert("height".into(), json!(h));
        }
        if let Some(seed) = options.seed {
            image_gen_config.insert("seed".into(), json!(seed));
        }
        if options.n > 0 {
            image_gen_config.insert("numberOfImages".into(), json!(options.n));
        }
        if let Some(v) = ab_opts.and_then(|o| o.get("quality")) {
            image_gen_config.insert("quality".into(), v.clone());
        }
        if let Some(v) = ab_opts.and_then(|o| o.get("cfgScale")) {
            image_gen_config.insert("cfgScale".into(), v.clone());
        }

        let mut args = Map::new();

        if has_files {
            let files = options.files.as_ref().unwrap();
            let has_mask = options.mask.is_some();
            let has_mask_prompt = ab_opts.and_then(|o| o.get("maskPrompt")).is_some();
            let task_type = ab_opts
                .and_then(|o| o.get("taskType"))
                .and_then(|v| v.as_str())
                .unwrap_or(if has_mask || has_mask_prompt {
                    "INPAINTING"
                } else {
                    "IMAGE_VARIATION"
                });

            let source_image_b64 = Self::get_base64_data(&files[0])?;

            match task_type {
                "INPAINTING" => {
                    let mut params = Map::new();
                    params.insert("image".into(), json!(source_image_b64));
                    if let Some(ref p) = options.prompt {
                        params.insert("text".into(), json!(p));
                    }
                    if let Some(v) = ab_opts.and_then(|o| o.get("negativeText")) {
                        params.insert("negativeText".into(), v.clone());
                    }
                    if has_mask {
                        if let Some(ref mask) = options.mask {
                            params.insert("maskImage".into(), json!(Self::get_base64_data(mask)?));
                        }
                    } else if let Some(v) = ab_opts.and_then(|o| o.get("maskPrompt")) {
                        params.insert("maskPrompt".into(), v.clone());
                    }
                    args.insert("taskType".into(), json!("INPAINTING"));
                    args.insert("inPaintingParams".into(), Value::Object(params));
                    args.insert(
                        "imageGenerationConfig".into(),
                        Value::Object(image_gen_config),
                    );
                }
                "OUTPAINTING" => {
                    let mut params = Map::new();
                    params.insert("image".into(), json!(source_image_b64));
                    if let Some(ref p) = options.prompt {
                        params.insert("text".into(), json!(p));
                    }
                    if let Some(v) = ab_opts.and_then(|o| o.get("negativeText")) {
                        params.insert("negativeText".into(), v.clone());
                    }
                    if let Some(v) = ab_opts.and_then(|o| o.get("outPaintingMode")) {
                        params.insert("outPaintingMode".into(), v.clone());
                    }
                    if has_mask {
                        if let Some(ref mask) = options.mask {
                            params.insert("maskImage".into(), json!(Self::get_base64_data(mask)?));
                        }
                    } else if let Some(v) = ab_opts.and_then(|o| o.get("maskPrompt")) {
                        params.insert("maskPrompt".into(), v.clone());
                    }
                    args.insert("taskType".into(), json!("OUTPAINTING"));
                    args.insert("outPaintingParams".into(), Value::Object(params));
                    args.insert(
                        "imageGenerationConfig".into(),
                        Value::Object(image_gen_config),
                    );
                }
                "BACKGROUND_REMOVAL" => {
                    args.insert("taskType".into(), json!("BACKGROUND_REMOVAL"));
                    args.insert(
                        "backgroundRemovalParams".into(),
                        json!({ "image": source_image_b64 }),
                    );
                }
                "IMAGE_VARIATION" => {
                    let images: Vec<String> = files
                        .iter()
                        .map(Self::get_base64_data)
                        .collect::<Result<_, _>>()?;
                    let mut params = Map::new();
                    params.insert("images".into(), json!(images));
                    if let Some(ref p) = options.prompt {
                        params.insert("text".into(), json!(p));
                    }
                    if let Some(v) = ab_opts.and_then(|o| o.get("negativeText")) {
                        params.insert("negativeText".into(), v.clone());
                    }
                    if let Some(v) = ab_opts.and_then(|o| o.get("similarityStrength")) {
                        params.insert("similarityStrength".into(), v.clone());
                    }
                    args.insert("taskType".into(), json!("IMAGE_VARIATION"));
                    args.insert("imageVariationParams".into(), Value::Object(params));
                    args.insert(
                        "imageGenerationConfig".into(),
                        Value::Object(image_gen_config),
                    );
                }
                _ => {
                    return Err(AiMuxError::InvalidArgument(format!(
                        "Unsupported task type: {task_type}"
                    )));
                }
            }
        } else {
            // Standard text-to-image
            let mut params = Map::new();
            if let Some(ref p) = options.prompt {
                params.insert("text".into(), json!(p));
            }
            if let Some(v) = ab_opts.and_then(|o| o.get("negativeText")) {
                params.insert("negativeText".into(), v.clone());
            }
            if let Some(v) = ab_opts.and_then(|o| o.get("style")) {
                params.insert("style".into(), v.clone());
            }
            args.insert("taskType".into(), json!("TEXT_IMAGE"));
            args.insert("textToImageParams".into(), Value::Object(params));
            args.insert(
                "imageGenerationConfig".into(),
                Value::Object(image_gen_config),
            );
        }

        if options.aspect_ratio.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "aspectRatio".into(),
                details: Some(
                    "This model does not support aspect ratio. Use `size` instead.".into(),
                ),
            });
        }

        let body_str = serde_json::to_string(&Value::Object(args))
            .map_err(|e| AiMuxError::JsonParse(e.to_string()))?;
        let url = self.endpoint();
        let headers = self.build_headers(&body_str, &url, options.headers.as_ref())?;

        let resp = aimux_provider_utils::post_to_api(
            HttpRequest {
                url: url.clone(),
                headers,

                abort_signal: options.abort_signal.clone(),
                call_id: None,
                recording_context: None,
                ..Default::default()
            },
            HttpBody::Bytes(body_str.into_bytes(), "application/json".to_string()),
            aimux_provider_utils::create_json_response_handler::<Value>(),
            super::bedrock_failed_response_handler(),
        )
        .await?;
        let rh = resp.response_headers;
        let response_body = resp.raw_value.as_ref().map(ToString::to_string);
        let rb = resp.value;

        // Handle moderated/blocked requests
        if let Some(s) = rb.get("status").and_then(|v| v.as_str())
            && s == "Request Moderated"
        {
            let reasons = rb
                .get("details")
                .and_then(|d| d.get("Moderation Reasons"))
                .and_then(|v| v.as_array());
            let reasons_str = reasons
                .map(|r| {
                    r.iter()
                        .filter_map(|v| v.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_else(|| "Unknown".into());
            return Err(AiMuxError::ApiCall(Box::new(ApiCallError {
                status_code: Some(200),
                provider_code: Some(s.to_string()),
                response_body,
                ..ApiCallError::new(
                    format!("Amazon Bedrock request was moderated: {reasons_str}"),
                    url,
                    serde_json::json!({}),
                )
            })));
        }

        let images: Vec<String> = rb
            .get("images")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        if images.is_empty() {
            return Err(AiMuxError::InvalidResponseData(
                "Amazon Bedrock returned no images.".into(),
            ));
        }

        Ok(ImageResult {
            images: ImageOutputs::Base64(images),
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
