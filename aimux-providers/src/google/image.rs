//! Google image generation delegates to the Gemini language model.

use super::convert::{build_request_body_with_warnings, convert_usage, validate_call_options};
use super::options::{GOOGLE, google_metadata};
use super::types::GenerateContentResponse;
use crate::shared::EndpointConfig;
use aimux_core::error::AiMuxError;
use aimux_core::image_model::{
    ImageCallOptions, ImageFile, ImageFileData, ImageModel, ImageOutputs, ImageResponse,
    ImageResult, ImageUsage,
};
use aimux_core::language_model_message::{FilePart, LanguageModelMessage, TextPart, UserPart};
use aimux_core::options::CallOptions;
use aimux_core::shared::{FileBytes, FileData, Warning};
use aimux_core::tool::{ProviderTool, Tool};
use async_trait::async_trait;
use serde_json::json;

#[derive(Debug, Clone, Default)]
pub struct GoogleImageSettings {
    pub max_images_per_call: Option<u32>,
}

pub struct GoogleImageModel {
    model_id: String,
    settings: GoogleImageSettings,
    config: EndpointConfig,
}

impl GoogleImageModel {
    pub(crate) fn from_config(
        model_id: String,
        settings: GoogleImageSettings,
        config: EndpointConfig,
    ) -> Self {
        Self {
            model_id,
            settings,
            config,
        }
    }
}

#[async_trait]
impl ImageModel for GoogleImageModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }
    fn model_id(&self) -> &str {
        &self.model_id
    }
    fn max_images_per_call(&self) -> Option<u32> {
        Some(self.settings.max_images_per_call.unwrap_or(1))
    }

    async fn do_generate(&self, options: &ImageCallOptions) -> Result<ImageResult, AiMuxError> {
        if !self.model_id.starts_with("gemini-") {
            return Err(AiMuxError::UnsupportedFunctionality("Google image models other than Gemini are no longer supported. Use a model ID that starts with `gemini-`.".into()));
        }
        if options.mask.is_some() {
            return Err(AiMuxError::UnsupportedFunctionality(
                "Gemini image models do not support mask-based image editing.".into(),
            ));
        }
        let warnings = if options.size.is_some() {
            vec![Warning::Unsupported {
                feature: "size".into(),
                details: Some(
                    "This model does not support the `size` option. Use `aspectRatio` instead."
                        .into(),
                ),
            }]
        } else {
            Vec::new()
        };
        let mut content = Vec::new();
        if let Some(prompt) = &options.prompt {
            content.push(UserPart::Text(TextPart {
                text: prompt.clone(),
                provider_options: None,
            }));
        }
        for file in options.files.iter().flatten() {
            let (data, media_type) = match file {
                ImageFile::Url { url } => (
                    FileData::Url {
                        url: url.clone(),
                        original_url: None,
                    },
                    "image/*".into(),
                ),
                ImageFile::File { media_type, data } => (
                    FileData::Data {
                        data: match data {
                            ImageFileData::Base64(data) => FileBytes::Base64(data.clone()),
                            ImageFileData::Binary(data) => FileBytes::Binary(data.clone()),
                        },
                    },
                    media_type.clone(),
                ),
            };
            content.push(UserPart::File(FilePart {
                data,
                media_type,
                filename: None,
                provider_options: None,
            }));
        }
        let mut call = CallOptions::new(vec![LanguageModelMessage::User {
            content,
            provider_options: None,
        }]);
        call.seed = options.seed;
        call.headers = options.headers.clone();
        call.abort_signal = options.abort_signal.clone();
        call.provider_options = Some(options.provider_options.clone());
        let google = call
            .provider_options
            .as_mut()
            .expect("set above")
            .entry(GOOGLE.into())
            .or_default();
        if let Some(search) = google.remove("googleSearch") {
            let invalid =
                || AiMuxError::InvalidArgument("Invalid Google image googleSearch option".into());
            let object = search.as_object().ok_or_else(invalid)?;
            if object.get("searchTypes").is_some_and(|types| {
                !types.as_object().is_some_and(|types| {
                    ["webSearch", "imageSearch"]
                        .iter()
                        .all(|key| types.get(*key).is_none_or(serde_json::Value::is_object))
                })
            }) || object.get("timeRangeFilter").is_some_and(|range| {
                !range.as_object().is_some_and(|range| {
                    ["startTime", "endTime"]
                        .iter()
                        .all(|key| range.get(*key).is_some_and(serde_json::Value::is_string))
                })
            }) {
                return Err(invalid());
            }
            call.tools = Some(vec![Tool::Provider(ProviderTool {
                id: "google.google_search".into(),
                name: "google_search".into(),
                args: object.clone(),
            })]);
        }
        google.insert("responseModalities".into(), json!(["IMAGE"]));
        let user_config = google.remove("imageConfig").filter(|v| !v.is_null());
        if options.aspect_ratio.is_some() || user_config.is_some() {
            let mut config = user_config
                .and_then(|v| v.as_object().cloned())
                .unwrap_or_default();
            if let Some(ratio) = options.aspect_ratio {
                config.insert("aspectRatio".into(), json!(ratio.to_string()));
            }
            google.insert("imageConfig".into(), serde_json::Value::Object(config));
        }
        validate_call_options(&call)?;
        let (body, _) = build_request_body_with_warnings(&self.model_id, &call)?;
        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let response = aimux_provider_utils::post_json_to_api(
            exchange.request(
                exchange.url(&format!("/models/{}:generateContent", self.model_id)),
                options,
            ),
            exchange.transform_body(body),
            aimux_provider_utils::create_json_response_handler::<GenerateContentResponse>(),
            super::google_failed_response_handler(),
        )
        .await?;
        let data = response.value;
        let candidate = data.candidates.into_iter().next().unwrap_or_default();
        let images: Vec<String> = candidate
            .content
            .as_ref()
            .and_then(|c| c.parts.as_ref())
            .into_iter()
            .flatten()
            .filter_map(|part| {
                if part.get("thought").and_then(serde_json::Value::as_bool) == Some(true) {
                    return None;
                }
                let inline = part.get("inlineData")?;
                inline
                    .get("mimeType")?
                    .as_str()?
                    .starts_with("image/")
                    .then(|| {
                        inline
                            .get("data")
                            .and_then(serde_json::Value::as_str)
                            .map(String::from)
                    })
                    .flatten()
            })
            .collect();
        let metadata = google_metadata(json!({
            "promptFeedback": data.prompt_feedback,
            "groundingMetadata": candidate.grounding_metadata,
            "urlContextMetadata": candidate.url_context_metadata,
            "safetyRatings": candidate.safety_ratings,
            "usageMetadata": data.usage_metadata,
            "finishMessage": candidate.finish_message,
            "serviceTier": data.usage_metadata.as_ref().and_then(|u| u.service_tier.as_ref()),
            "finishReason": candidate.finish_reason.or_else(|| data.prompt_feedback.as_ref().and_then(|f| f.get("blockReason")).and_then(serde_json::Value::as_str).filter(|reason| !reason.is_empty() && *reason != "BLOCK_REASON_UNSPECIFIED" && *reason != "BLOCKED_REASON_UNSPECIFIED").map(String::from)),
            "images": images.iter().map(|_| json!({})).collect::<Vec<_>>(),
        }));
        let usage = data
            .usage_metadata
            .as_ref()
            .map(convert_usage)
            .unwrap_or_default();
        let input = usage.input_tokens.total;
        let output = usage.output_tokens.total;
        Ok(ImageResult {
            images: ImageOutputs::Base64(images),
            warnings,
            provider_metadata: Some(metadata),
            response: ImageResponse {
                timestamp: Some(chrono::Utc::now().to_rfc3339()),
                model_id: Some(self.model_id.clone()),
                headers: Some(response.response_headers),
            },
            usage: Some(ImageUsage {
                input_tokens: input,
                output_tokens: output,
                total_tokens: Some(input.unwrap_or(0) + output.unwrap_or(0)),
            }),
        })
    }
}
