//! Google Vertex AI image model — implements the `ImageModel` trait.
//!
//! Aligned with Vercel AI SDK `GoogleVertexImageModel`
//! (`reference/ai/packages/google-vertex/src/google-vertex-image-model.ts`).
//!
//! Uses the Gemini language model endpoint for image generation.

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::google::convert::build_vertex_request_body;
use crate::google::google_failed_response_handler;
use crate::google::options::{GOOGLE_VERTEX, Namespace};
use crate::shared::EndpointConfig;
use aimux_core::language_model_message::{FilePart, LanguageModelMessage, TextPart, UserPart};
use aimux_core::options::CallOptions;
use aimux_core::shared::provider_namespace;

use aimux_core::error::AiMuxError;
use aimux_core::image_model::{
    ImageCallOptions, ImageFile, ImageFileData, ImageModel, ImageOutputs, ImageResponse,
    ImageResult, ImageUsage,
};
use aimux_core::shared::{FileBytes, FileData, Warning};

fn is_gemini_model(model_id: &str) -> bool {
    model_id.starts_with("gemini-")
}

/// A Google Vertex AI image generation model.
///
/// Does **not** hold an HTTP client — the `aimux-provider-utils` API helpers use the process-wide shared
/// `Client` internally (RFC-0009 §4.1).
pub struct VertexImageModel {
    model_id: String,
    config: EndpointConfig,
}

impl VertexImageModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }

    fn generate_content_path(&self) -> String {
        let mp = if self.model_id.contains('/') {
            self.model_id.clone()
        } else {
            format!("models/{}", self.model_id)
        };
        format!("/{mp}:generateContent")
    }

    async fn do_generate_gemini(
        &self,
        options: &ImageCallOptions,
    ) -> Result<ImageResult, AiMuxError> {
        let mut warnings = Vec::new();
        if options.mask.is_some() {
            return Err(AiMuxError::UnsupportedFunctionality(
                "Gemini image models do not support mask-based image editing.".into(),
            ));
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

        let mut parts = Vec::new();
        if let Some(ref p) = options.prompt {
            parts.push(UserPart::Text(TextPart {
                text: p.clone(),
                provider_options: None,
            }));
        }
        if let Some(ref files) = options.files {
            for file in files {
                match file {
                    ImageFile::Url { url } => {
                        let file = FilePart {
                            data: FileData::Url {
                                url: url::Url::parse(url)
                                    .map_err(|error| {
                                        AiMuxError::InvalidArgument(error.to_string())
                                    })?
                                    .to_string(),
                                original_url: None,
                            },
                            media_type: "image/*".into(),
                            filename: None,
                            provider_options: None,
                        };
                        aimux_provider_utils::resolve_full_media_type(&file)?;
                        parts.push(UserPart::File(file));
                    }
                    ImageFile::File { media_type, data } => {
                        parts.push(UserPart::File(FilePart {
                            data: FileData::Data {
                                data: match data {
                                    ImageFileData::Base64(data) => FileBytes::Base64(data.clone()),
                                    ImageFileData::Binary(data) => FileBytes::Binary(data.clone()),
                                },
                            },
                            media_type: media_type.clone(),
                            filename: None,
                            provider_options: None,
                        }));
                    }
                }
            }
        }

        let mut inner_options = Namespace::Vertex
            .write_keys()
            .iter()
            .find_map(|key| options.provider_options.get(*key))
            .cloned()
            .unwrap_or_default();
        inner_options.insert("responseModalities".into(), json!(["IMAGE"]));
        let mut image_config = inner_options
            .get("imageConfig")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        if let Some(ar) = options.aspect_ratio {
            image_config.insert("aspectRatio".into(), json!(ar.to_string()));
        }
        if options.aspect_ratio.is_some() || inner_options.contains_key("imageConfig") {
            inner_options.insert("imageConfig".into(), Value::Object(image_config));
        }
        let mut call_options = CallOptions::new(vec![LanguageModelMessage::User {
            content: parts,
            provider_options: None,
        }]);
        call_options.seed = options.seed;
        call_options.provider_options = Some(
            provider_namespace(GOOGLE_VERTEX, Value::Object(inner_options))
                .expect("provider metadata must be an object"),
        );
        let body = build_vertex_request_body(&self.model_id, &call_options)?;
        let exchange = self.config.exchange(options.headers.as_ref()).await?;

        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(exchange.url(&self.generate_content_path()), options),
            body,
            aimux_provider_utils::create_json_response_handler(),
            google_failed_response_handler(),
        )
        .await?;
        let rh = resp.response_headers;
        let rb: Value = resp.value;

        let mut images: Vec<String> = Vec::new();
        if let Some(c) = rb
            .get("candidates")
            .and_then(|c| c.as_array())
            .and_then(|c| c.first())
            && let Some(parts) = c
                .get("content")
                .and_then(|c| c.get("parts"))
                .and_then(|p| p.as_array())
        {
            for p in parts {
                if p.get("thought").and_then(Value::as_bool) != Some(true)
                    && let Some(id) = p.get("inlineData")
                    && let Some(mt) = id.get("mimeType").and_then(|m| m.as_str())
                    && mt.starts_with("image/")
                    && let Some(d) = id.get("data").and_then(|d| d.as_str())
                {
                    images.push(d.to_string());
                }
            }
        }

        let usage = Some(
            if let Some(u) = rb.get("usageMetadata").filter(|u| !u.is_null()) {
                let count = |name| u.get(name).and_then(Value::as_u64).unwrap_or(0) as u32;
                let input_tokens = count("promptTokenCount") + count("toolUsePromptTokenCount");
                let output_tokens = count("candidatesTokenCount") + count("thoughtsTokenCount");
                ImageUsage {
                    input_tokens: Some(input_tokens),
                    output_tokens: Some(output_tokens),
                    total_tokens: Some(input_tokens + output_tokens),
                }
            } else {
                ImageUsage {
                    total_tokens: Some(0),
                    ..Default::default()
                }
            },
        );

        let payload = json!({ "images": images.iter().map(|_| json!({})).collect::<Vec<_>>() });
        let metadata = Namespace::Vertex.metadata(payload);

        Ok(ImageResult {
            images: ImageOutputs::Base64(images),
            warnings,
            provider_metadata: Some(metadata),
            response: ImageResponse {
                timestamp: Some(chrono::Utc::now().to_rfc3339()),
                model_id: Some(self.model_id.clone()),
                headers: Some(rh),
            },
            usage,
        })
    }
}

#[async_trait]
impl ImageModel for VertexImageModel {
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
        if is_gemini_model(&self.model_id) {
            self.do_generate_gemini(options).await
        } else {
            Err(AiMuxError::UnsupportedFunctionality(
                "Google image models other than Gemini are no longer supported. Use a model ID that starts with `gemini-`.".into(),
            ))
        }
    }
}
