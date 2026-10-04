//! The OpenAI-compatible image model (`{name}.image`).
//!
//! The Rust form of `OpenAICompatibleImageModel`:
//!
//! - generation: `POST {base_url}/images/generations`, JSON body with
//!   `response_format: "b64_json"`;
//! - editing (`files` given): `POST {base_url}/images/edits`, multipart form.
//!
//! The provider options of the `openaiCompatible` namespace, then the
//! provider's own, are forwarded to the endpoint as given.

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::image_model::{
    ImageCallOptions, ImageFile, ImageFileData, ImageModel, ImageOutputs, ImageResponse,
    ImageResult,
};
use aimux_core::shared::Warning;
use aimux_provider_utils::{HttpBody, MultipartForm};

use super::config::CompatModelConfig;
use super::convert::to_camel_case;

/// An OpenAI-compatible image generation / editing model.
pub struct OpenAICompatibleImageModel {
    model_id: String,
    config: CompatModelConfig,
}

impl OpenAICompatibleImageModel {
    pub(crate) fn from_config(model_id: String, config: CompatModelConfig) -> Self {
        Self { model_id, config }
    }

    /// The provider options that go to the endpoint: generic namespace, then
    /// the provider's own, then its camelCase form (later wins).
    fn forwarded_options(&self, options: &ImageCallOptions) -> Map<String, Value> {
        let name = self.config.provider_options_name();
        let camel = to_camel_case(name);
        let mut out = Map::new();
        for key in ["openaiCompatible", name, camel.as_str()] {
            if let Some(object) = options.provider_options.get(key) {
                out.extend(object.iter().map(|(k, v)| (k.clone(), v.clone())));
            }
        }
        out
    }
}

fn file_bytes(file: &ImageFile) -> Result<(Vec<u8>, String), AiMuxError> {
    match file {
        ImageFile::File { media_type, data } => {
            let bytes = match data {
                ImageFileData::Binary(bytes) => bytes.clone(),
                ImageFileData::Base64(b64) => {
                    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64)
                        .map_err(|e| AiMuxError::InvalidArgument(format!("invalid base64: {e}")))?
                }
            };
            Ok((bytes, media_type.clone()))
        }
        ImageFile::Url { .. } => Err(AiMuxError::InvalidArgument(
            "image edit files must be inline data, not URLs".to_string(),
        )),
    }
}

fn form_value(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

#[async_trait]
impl ImageModel for OpenAICompatibleImageModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn max_images_per_call(&self) -> Option<u32> {
        Some(10)
    }

    async fn do_generate(&self, options: &ImageCallOptions) -> Result<ImageResult, AiMuxError> {
        let mut warnings = Vec::new();
        if options.aspect_ratio.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "aspectRatio".to_string(),
                details: Some(
                    "This model does not support aspect ratio. Use `size` instead.".to_string(),
                ),
            });
        }
        if options.seed.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "seed".to_string(),
                details: None,
            });
        }

        let timestamp = chrono::Utc::now().to_rfc3339();
        let headers = self
            .config
            .request_headers(options.headers.as_ref())
            .await?;
        let forwarded = self.forwarded_options(options);

        let (value, response_headers): (Value, _) = if let Some(files) = &options.files {
            let mut form = MultipartForm::new();
            form.text("model", &self.model_id)?;
            if let Some(prompt) = &options.prompt {
                form.text("prompt", prompt)?;
            }
            form.text("n", &options.n.to_string())?;
            if let Some(size) = options.size {
                form.text("size", &size.to_string())?;
            }
            for (name, value) in &forwarded {
                form.text(name, &form_value(value))?;
            }
            let field = if files.len() == 1 { "image" } else { "image[]" };
            for file in files {
                let (bytes, media_type) = file_bytes(file)?;
                form.file(field, "image", &media_type, &bytes)?;
            }
            if let Some(mask) = &options.mask {
                let (bytes, media_type) = file_bytes(mask)?;
                form.file("mask", "mask", &media_type, &bytes)?;
            }
            let (bytes, content_type) = form.finish();
            let resp = aimux_provider_utils::post_to_api(
                self.config
                    .http_request("/images/edits", headers, options)?,
                HttpBody::Bytes(bytes, content_type),
                aimux_provider_utils::create_json_response_handler(),
                self.config.failed_response_handler(),
            )
            .await?;
            (resp.value, resp.response_headers)
        } else {
            let mut body = Map::new();
            body.insert("model".into(), json!(self.model_id));
            if let Some(prompt) = &options.prompt {
                body.insert("prompt".into(), json!(prompt));
            }
            body.insert("n".into(), json!(options.n));
            if let Some(size) = options.size {
                body.insert("size".into(), json!(size.to_string()));
            }
            body.extend(forwarded);
            body.insert("response_format".into(), json!("b64_json"));
            let body = self.config.transform_body(Value::Object(body));
            let resp = aimux_provider_utils::post_json_to_api(
                self.config
                    .http_request("/images/generations", headers, options)?,
                body,
                aimux_provider_utils::create_json_response_handler(),
                self.config.failed_response_handler(),
            )
            .await?;
            (resp.value, resp.response_headers)
        };

        let images: Vec<String> = value
            .get("data")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.get("b64_json").and_then(Value::as_str))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();

        Ok(ImageResult {
            images: ImageOutputs::Base64(images),
            warnings,
            provider_metadata: None,
            response: ImageResponse {
                timestamp: Some(timestamp),
                model_id: Some(self.model_id.clone()),
                headers: Some(response_headers),
            },
            usage: None,
        })
    }
}
