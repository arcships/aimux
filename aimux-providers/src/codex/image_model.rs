//! Codex image models on the ChatGPT subscription channel (#526).
//!
//! The subscription channel (`chatgpt.com/backend-api/codex`) serves image
//! generation and editing against the account's Codex usage. Its wire shapes
//! differ from the public Image API, so these models do not reuse the
//! `openai` image model:
//!
//! - **`gpt-image-2`** — the direct image endpoints, both JSON:
//!   `POST {base}/images/generations` with `{model, prompt}`, and
//!   `POST {base}/images/edits` with `{model, prompt, images: [{image_url}]}`
//!   (a JSON body with `data:` URLs, not multipart). The response is the
//!   Image API's (`data[].b64_json`, `usage`, `size`, …).
//! - **`gpt-image-2.5-flare`, `gpt-image-2.5-sunburst` and their
//!   `-2026-09-08` snapshots** — the Responses `image_generation` tool:
//!   `POST {base}/responses` (streamed), the outer model only calls the tool.
//!   The image bytes arrive in the `image_generation_call` item of
//!   `response.output_item.done`; the final `response.completed` may carry an
//!   empty `output`, so the retained item is used and `response.output` is
//!   only the fallback.
//!
//! Only the fields the channel was probed with are sent. A failure is never
//! replayed: a timeout, a reset connection, a 5xx or a malformed body may all
//! come after the server accepted (and billed) the generation, so every
//! error these models return is non-retryable. A 401 is
//! [`AiMuxError::TokenExpired`], as for the language models.

use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::image_model::{
    ImageCallOptions, ImageFile, ImageFileData, ImageModel, ImageOutputs, ImageResponse,
    ImageResult,
};
use aimux_core::shared::{SharedProviderMetadata, Warning};
use aimux_provider_utils::{HttpRequest, MediaTypeData};

use crate::openai::internal::{
    OpenAIModelConfig, extract_image_metadata, extract_image_outputs, extract_image_usage,
    openai_failed_response_handler,
};

use super::responses_model::CodexModel;

/// The direct-endpoint model.
const GPT_IMAGE_2: &str = "gpt-image-2";

/// The models served through the Responses `image_generation` tool.
const RESPONSES_TOOL_MODELS: &[&str] = &[
    "gpt-image-2.5-flare",
    "gpt-image-2.5-flare-2026-09-08",
    "gpt-image-2.5-sunburst",
    "gpt-image-2.5-sunburst-2026-09-08",
];

/// The outer Responses model that calls the image tool.
const RESPONSES_OUTER_MODEL: &str = "gpt-5.6-luna";

/// The instructions that make the outer model call the tool with the prompt
/// unchanged.
const RESPONSES_INSTRUCTIONS: &str = "Call image_generation with the user's prompt exactly as \
written, preserving all wording, language, punctuation, and visual constraints.";

/// Hang guard of the direct (non-streaming) exchanges. A high-quality or
/// large generation legitimately holds the connection well past the 30 s
/// default; the Core operation `timeout` stays the caller's outer bound.
pub(super) const DIRECT_EXCHANGE_TIMEOUT: Duration = Duration::from_secs(600);

/// The providerOptions namespaces these models would read. They take no
/// options, so any key under them is refused rather than silently dropped.
const OPTION_NAMESPACES: &[&str] = &["codex", "openai"];

/// Bounds of a `gpt-image-2.5` `size` (OpenAI image size rules).
const SIZE_STEP: u32 = 16;
const SIZE_MAX_SIDE: u32 = 3840;
const SIZE_MAX_RATIO: u32 = 3;
const SIZE_MIN_PIXELS: u64 = 655_360;
const SIZE_MAX_PIXELS: u64 = 8_294_400;

/// How a model id reaches the channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Route {
    /// `POST /images/generations` and `POST /images/edits`.
    Direct,
    /// `POST /responses` with the `image_generation` tool.
    ResponsesTool,
}

impl Route {
    fn of(model_id: &str) -> Option<Self> {
        if model_id == GPT_IMAGE_2 {
            Some(Self::Direct)
        } else if RESPONSES_TOOL_MODELS.contains(&model_id) {
            Some(Self::ResponsesTool)
        } else {
            None
        }
    }
}

/// A Codex image model on the ChatGPT subscription channel.
#[derive(Clone)]
pub struct CodexImageModel {
    model_id: String,
    route: Route,
    config: OpenAIModelConfig,
}

impl CodexImageModel {
    /// The model for a subscription-channel image id, or `None` for an id the
    /// channel does not serve as an image model.
    pub(super) fn subscription(model_id: &str, config: OpenAIModelConfig) -> Option<Self> {
        Route::of(model_id).map(|route| Self {
            model_id: model_id.to_string(),
            route,
            config,
        })
    }

    /// Validate the call; returns the warnings for ignored settings.
    fn validate(&self, options: &ImageCallOptions) -> Result<Vec<Warning>, AiMuxError> {
        if options.n != 1 {
            return Err(AiMuxError::invalid_argument(format!(
                "{} generates one image per call; got n = {}",
                self.model_id, options.n
            )));
        }
        if options.prompt.as_deref().is_none_or(str::is_empty) {
            return Err(AiMuxError::invalid_argument(format!(
                "{} requires a prompt",
                self.model_id
            )));
        }
        let provider_name = self
            .config
            .provider
            .split_once('.')
            .map_or(self.config.provider.as_str(), |(name, _)| name);
        let namespaces = OPTION_NAMESPACES
            .iter()
            .copied()
            .chain((!OPTION_NAMESPACES.contains(&provider_name)).then_some(provider_name));
        for namespace in namespaces {
            if let Some(object) = options.provider_options.get(namespace)
                && !object.is_empty()
            {
                let keys: Vec<&str> = object.keys().map(String::as_str).collect();
                return Err(AiMuxError::invalid_argument(format!(
                    "{} takes no provider options; unsupported `{namespace}` option(s): {}",
                    self.model_id,
                    keys.join(", ")
                )));
            }
        }
        if options.mask.is_some() {
            return Err(AiMuxError::invalid_argument(format!(
                "{} does not support mask-based editing",
                self.model_id
            )));
        }
        if options.files.as_ref().is_some_and(Vec::is_empty) {
            return Err(AiMuxError::invalid_argument(format!(
                "{} image editing requires at least one input image",
                self.model_id
            )));
        }
        if let Some(size) = options.size {
            match self.route {
                Route::Direct => {
                    return Err(AiMuxError::invalid_argument(format!(
                        "{} does not support the `size` option",
                        self.model_id
                    )));
                }
                Route::ResponsesTool => validate_size(size.width(), size.height())?,
            }
        }

        let mut warnings = Vec::new();
        if options.aspect_ratio.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "aspectRatio".to_string(),
                details: Some("This model does not support aspect ratio.".to_string()),
            });
        }
        if options.seed.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "seed".to_string(),
                details: None,
            });
        }
        Ok(warnings)
    }

    /// The inline `data:` URLs of the input images, in order.
    async fn input_image_urls(
        &self,
        options: &ImageCallOptions,
    ) -> Result<Vec<String>, AiMuxError> {
        let mut urls = Vec::new();
        for file in options.files.iter().flatten() {
            urls.push(image_data_url(file, options).await?);
        }
        Ok(urls)
    }

    /// The direct exchange: JSON body, a long hang guard.
    pub(super) fn direct_request(
        &self,
        path: &str,
        headers: Vec<(String, String)>,
        options: &ImageCallOptions,
    ) -> Result<HttpRequest, AiMuxError> {
        let mut request = self
            .config
            .http_request(self.config.url(path)?, headers, options);
        request.response_timeout = Some(DIRECT_EXCHANGE_TIMEOUT);
        Ok(request)
    }

    async fn generate_direct(
        &self,
        options: &ImageCallOptions,
        warnings: Vec<Warning>,
    ) -> Result<ImageResult, AiMuxError> {
        let timestamp = aimux_core::Timestamp::now();
        let headers = self
            .config
            .request_headers(options.headers.as_ref())
            .await?;
        let mut body = Map::new();
        body.insert("model".into(), json!(self.model_id));
        body.insert("prompt".into(), json!(options.prompt));
        let path = if options.files.is_some() {
            let images: Vec<Value> = self
                .input_image_urls(options)
                .await?
                .into_iter()
                .map(|url| json!({ "image_url": url }))
                .collect();
            body.insert("images".into(), Value::Array(images));
            "/images/edits"
        } else {
            "/images/generations"
        };
        let body = self.config.transform_body(Value::Object(body));

        let response = aimux_provider_utils::post_json_to_api(
            self.direct_request(path, headers, options)?,
            body,
            aimux_provider_utils::create_json_response_handler::<Value>(),
            openai_failed_response_handler(),
        )
        .await
        .map_err(no_replay)?;

        let value = response.value;
        let images = extract_image_outputs(&value);
        if matches!(&images, ImageOutputs::Base64(images) if images.is_empty()) {
            return Err(AiMuxError::invalid_response_data(
                "image response contained no `data[].b64_json`",
            ));
        }
        Ok(ImageResult {
            images,
            warnings,
            provider_metadata: Some(extract_image_metadata(&value)),
            response: ImageResponse {
                timestamp: Some(timestamp),
                model_id: Some(self.model_id.clone()),
                headers: Some(response.response_headers),
            },
            usage: extract_image_usage(&value),
        })
    }

    async fn generate_with_responses_tool(
        &self,
        options: &ImageCallOptions,
        warnings: Vec<Warning>,
    ) -> Result<ImageResult, AiMuxError> {
        let timestamp = aimux_core::Timestamp::now();
        let mut headers = self
            .config
            .request_headers(options.headers.as_ref())
            .await?;
        if !headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("accept"))
        {
            headers.push(("accept".to_string(), "text/event-stream".to_string()));
        }

        let is_edit = options.files.is_some();
        let mut tool = Map::new();
        tool.insert("type".into(), json!("image_generation"));
        tool.insert("model".into(), json!(self.model_id));
        tool.insert(
            "action".into(),
            json!(if is_edit { "edit" } else { "generate" }),
        );
        if let Some(size) = options.size {
            tool.insert("size".into(), json!(size.to_string()));
        }
        tool.insert("output_format".into(), json!("png"));

        let mut content = vec![json!({ "type": "input_text", "text": options.prompt })];
        for url in self.input_image_urls(options).await? {
            content.push(json!({ "type": "input_image", "image_url": url }));
        }

        let body = self.config.transform_body(json!({
            "model": RESPONSES_OUTER_MODEL,
            "store": false,
            "stream": true,
            "instructions": RESPONSES_INSTRUCTIONS,
            "tool_choice": { "type": "image_generation" },
            "tools": [Value::Object(tool)],
            "input": [{ "type": "message", "role": "user", "content": content }],
        }));

        let endpoint = self.config.url("/responses")?;
        let response = aimux_provider_utils::post_json_to_api(
            self.config.http_request(endpoint.clone(), headers, options),
            body.clone(),
            aimux_provider_utils::create_event_source_response_handler::<Value>(),
            openai_failed_response_handler(),
        )
        .await
        .map_err(no_replay)?;
        let response_headers = response.response_headers;
        let mut events = response.value;

        // The first `image_generation_call` item that carried a result.
        let mut retained: Option<Value> = None;
        let mut frame_error: Option<AiMuxError> = None;
        let mut completed: Option<Value> = None;
        while let Some(event) = events.next().await {
            let data = match event {
                Ok(data) => data,
                Err(error) if error.is_recoverable_stream_error() => {
                    frame_error.get_or_insert(error);
                    continue;
                }
                Err(error) => return Err(no_replay(error)),
            };
            match data.get("type").and_then(Value::as_str).unwrap_or("") {
                "response.output_item.done" => {
                    let item = &data["item"];
                    if retained.is_none() && is_image_call_with_result(item) {
                        retained = Some(item.clone());
                    }
                }
                "response.completed" => {
                    completed = Some(data);
                    break;
                }
                "error" | "response.failed" | "response.incomplete" => {
                    return Err(stream_failure(&data, &endpoint, body, response_headers));
                }
                _ => {}
            }
        }

        let Some(completed) = completed else {
            return Err(no_replay(frame_error.unwrap_or_else(|| {
                AiMuxError::invalid_response_data("image stream ended without response.completed")
            })));
        };
        let response_object = &completed["response"];
        let status = response_object["status"].as_str().unwrap_or("");
        if status != "completed" {
            return Err(stream_failure(
                &completed,
                &endpoint,
                body,
                response_headers,
            ));
        }
        let item = retained
            .or_else(|| {
                response_object["output"]
                    .as_array()?
                    .iter()
                    .find(|item| item["type"] == "image_generation_call")
                    .cloned()
            })
            .ok_or_else(|| {
                AiMuxError::invalid_response_data(
                    "image response contained no image_generation_call item",
                )
            })?;
        let image = item["result"]
            .as_str()
            .filter(|r| !r.is_empty())
            .ok_or_else(|| {
                AiMuxError::invalid_response_data("image_generation_call item has no `result`")
            })?;

        Ok(ImageResult {
            images: ImageOutputs::Base64(vec![image.to_string()]),
            warnings,
            provider_metadata: Some(tool_call_metadata(&item, response_object)),
            response: ImageResponse {
                timestamp: Some(timestamp),
                model_id: Some(self.model_id.clone()),
                headers: Some(response_headers),
            },
            usage: None,
        })
    }
}

#[async_trait]
impl ImageModel for CodexImageModel {
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

    async fn do_generate(&self, options: &ImageCallOptions) -> Result<ImageResult, AiMuxError> {
        let warnings = self.validate(options)?;
        match self.route {
            Route::Direct => self.generate_direct(options, warnings).await,
            Route::ResponsesTool => self.generate_with_responses_tool(options, warnings).await,
        }
    }
}

/// `auto` is the default and is expressed by omitting `size`; an explicit
/// size must satisfy the OpenAI image size rules.
fn validate_size(width: u32, height: u32) -> Result<(), AiMuxError> {
    let invalid = |reason: &str| {
        Err(AiMuxError::invalid_argument(format!(
            "invalid size `{width}x{height}`: {reason}"
        )))
    };
    if !width.is_multiple_of(SIZE_STEP) || !height.is_multiple_of(SIZE_STEP) {
        return invalid("both sides must be multiples of 16");
    }
    if width > SIZE_MAX_SIDE || height > SIZE_MAX_SIDE {
        return invalid("neither side may exceed 3840");
    }
    let (long, short) = (width.max(height), width.min(height));
    if long > short * SIZE_MAX_RATIO {
        return invalid("the long side may be at most 3 times the short side");
    }
    let pixels = u64::from(width) * u64::from(height);
    if !(SIZE_MIN_PIXELS..=SIZE_MAX_PIXELS).contains(&pixels) {
        return invalid("total pixels must be between 655,360 and 8,294,400");
    }
    Ok(())
}

/// Make an error non-retryable and map the subscription 401.
fn no_replay(error: AiMuxError) -> AiMuxError {
    match CodexModel::map_subscription_401(error) {
        AiMuxError::ApiCall(mut detail) => {
            detail.is_retryable = false;
            AiMuxError::ApiCall(detail)
        }
        other => other,
    }
}

fn is_image_call_with_result(item: &Value) -> bool {
    item["type"] == "image_generation_call"
        && item["result"]
            .as_str()
            .is_some_and(|result| !result.is_empty())
}

/// The error of an in-band failure event (`error`, `response.failed`,
/// `response.incomplete`, or a `response.completed` whose status is not
/// `completed`). Never retryable.
fn stream_failure(
    event: &Value,
    endpoint: &str,
    body: Value,
    response_headers: std::collections::HashMap<String, String>,
) -> AiMuxError {
    let error = event
        .get("error")
        .filter(|error| !error.is_null())
        .or_else(|| {
            event["response"]
                .get("error")
                .filter(|error| !error.is_null())
        });
    let event_type = event["type"].as_str().unwrap_or("error");
    let message = error
        .and_then(|error| error["message"].as_str())
        .map(str::to_string)
        .or_else(|| {
            event["response"]["incomplete_details"]["reason"]
                .as_str()
                .map(|reason| format!("image response incomplete: {reason}"))
        })
        .unwrap_or_else(|| match event["response"]["status"].as_str() {
            Some(status) if event_type == "response.completed" => {
                format!("image response finished with status `{status}`")
            }
            _ => format!("image generation failed ({event_type})"),
        });
    let provider_code = error
        .and_then(|error| error.get("code").or_else(|| error.get("type")))
        .and_then(Value::as_str)
        .map(str::to_string);
    aimux_provider_utils::create_provider_stream_error(
        aimux_provider_utils::ProviderStreamError {
            message,
            provider_code,
            status_code: Some(200),
            is_retryable: Some(false),
            data: event.clone(),
        },
        event,
        endpoint.to_string(),
        body,
        response_headers,
    )
}

/// `{ "openai": { "images": [ {...} ] } }`, the shape the OpenAI image model
/// reports, from the tool call item.
fn tool_call_metadata(item: &Value, response: &Value) -> SharedProviderMetadata {
    let mut entry = Map::new();
    for (wire, key) in [
        ("revised_prompt", "revisedPrompt"),
        ("size", "size"),
        ("quality", "quality"),
        ("background", "background"),
        ("output_format", "outputFormat"),
    ] {
        if let Some(value) = item.get(wire).and_then(Value::as_str) {
            entry.insert(key.to_string(), json!(value));
        }
    }
    if let Some(created) = response.get("created_at").and_then(Value::as_u64) {
        entry.insert("created".to_string(), json!(created));
    }
    let mut openai = Map::new();
    openai.insert("images".to_string(), json!([Value::Object(entry)]));
    if let Some(id) = response.get("id").and_then(Value::as_str) {
        openai.insert("responseId".to_string(), json!(id));
    }
    let mut metadata = SharedProviderMetadata::new();
    metadata.insert("openai".to_string(), openai);
    metadata
}

/// An input image as a `data:` URL: inline data with its media type (sniffed
/// when only partial), a `data:` URL as given, and any other URL downloaded
/// as `downloadBlob` does (guarded, no provider headers) and inlined.
async fn image_data_url(
    file: &ImageFile,
    options: &ImageCallOptions,
) -> Result<String, AiMuxError> {
    match file {
        ImageFile::File { media_type, data } => {
            let sniff = match data {
                ImageFileData::Base64(text) => MediaTypeData::Base64(text),
                ImageFileData::Binary(bytes) => MediaTypeData::Bytes(bytes),
            };
            let media_type = full_image_media_type(media_type, sniff)?;
            Ok(format!(
                "data:{media_type};base64,{}",
                aimux_provider_utils::convert_to_base64(data)
            ))
        }
        ImageFile::Url { url } if url.starts_with("data:") => Ok(url.clone()),
        ImageFile::Url { url } => {
            let response =
                aimux_provider_utils::download_blob(url.clone(), options.abort_signal.clone())
                    .await?;
            let header = response
                .response_headers
                .get("content-type")
                .map(|value| value.split(';').next().unwrap_or("").trim().to_string())
                .unwrap_or_default();
            let media_type = full_image_media_type(&header, MediaTypeData::Bytes(&response.value))?;
            Ok(format!(
                "data:{media_type};base64,{}",
                aimux_provider_utils::convert_uint8_array_to_base64(&response.value)
            ))
        }
    }
}

/// A full `image/<subtype>` media type, sniffed from the data when the given
/// one is partial or not an image type.
fn full_image_media_type(media_type: &str, data: MediaTypeData<'_>) -> Result<String, AiMuxError> {
    if media_type.starts_with("image/") && aimux_provider_utils::is_full_media_type(media_type) {
        return Ok(media_type.to_string());
    }
    aimux_provider_utils::detect_media_type(data, Some("image"))?
        .map(str::to_string)
        .ok_or_else(|| {
            AiMuxError::invalid_argument(format!(
                "input image of media type `{media_type}` has no detectable image subtype"
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_cover_exactly_the_subscription_image_ids() {
        assert_eq!(Route::of("gpt-image-2"), Some(Route::Direct));
        for id in RESPONSES_TOOL_MODELS {
            assert_eq!(Route::of(id), Some(Route::ResponsesTool), "{id}");
        }
        for id in [
            "gpt-image-1",
            "gpt-image-2-2026-01-01",
            "dall-e-3",
            "gpt-5.2-codex",
        ] {
            assert_eq!(Route::of(id), None, "{id}");
        }
    }

    #[test]
    fn size_rules() {
        for (w, h) in [
            (1024, 1024),
            (1536, 1024),
            (3840, 2160),
            (1024, 3072),
            (2880, 2880),
        ] {
            assert!(validate_size(w, h).is_ok(), "{w}x{h}");
        }
        for (w, h) in [
            (1000, 1000), // not a multiple of 16
            (4096, 2048), // side over 3840
            (3840, 1264), // ratio over 3
            (800, 800),   // under 655,360 pixels
            (3840, 2176), // over 8,294,400 pixels
        ] {
            assert!(validate_size(w, h).is_err(), "{w}x{h}");
        }
    }

    // The direct exchanges carry their own hang guard, longer than the 30 s
    // default, so a slow generation is not cut off (and then replayed).
    #[test]
    fn direct_exchange_has_a_long_response_timeout() {
        let config = OpenAIModelConfig::fixed(
            "codex.image".into(),
            "https://chatgpt.com/backend-api/codex".into(),
            aimux_provider_utils::ProviderHeaders::new(
                aimux_provider_utils::Credential::Explicit(
                    aimux_provider_utils::Resolvable::Value("t".into()),
                ),
                aimux_provider_utils::AuthScheme::Bearer,
                Vec::new(),
                None,
            ),
            None,
            None,
        );
        let model = CodexImageModel::subscription(GPT_IMAGE_2, config).unwrap();
        let request = model
            .direct_request(
                "/images/generations",
                Vec::new(),
                &ImageCallOptions::new("x"),
            )
            .unwrap();
        assert_eq!(request.response_timeout, Some(DIRECT_EXCHANGE_TIMEOUT));
        assert!(DIRECT_EXCHANGE_TIMEOUT >= Duration::from_secs(300));
    }

    #[test]
    fn no_replay_clears_retryability_and_maps_401() {
        let retryable = AiMuxError::ApiCall(Box::new(aimux_core::ApiCallError {
            status_code: Some(500),
            is_retryable: true,
            ..aimux_core::ApiCallError::new("boom", "u", Value::Null)
        }));
        assert!(!no_replay(retryable).is_retryable());
        let unauthorized = AiMuxError::ApiCall(Box::new(aimux_core::ApiCallError {
            status_code: Some(401),
            ..aimux_core::ApiCallError::new("expired", "u", Value::Null)
        }));
        assert!(matches!(
            no_replay(unauthorized),
            AiMuxError::TokenExpired(_)
        ));
    }
}
