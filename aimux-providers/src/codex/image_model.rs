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
//!
//! Input images are decoded locally; the direct exchange carries a response
//! timeout and the streamed drain its own bound, so corrupt caller input and a
//! stalled stream fail locally instead of becoming an opaque error from the
//! billed endpoint.

use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use futures::{Stream, StreamExt};
use serde_json::{Map, Value, json};

use aimux_core::ErrorCause;
use aimux_core::error::{AiMuxError, ApiCallError, ApiCallErrorOrigin};
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

/// Hang guard of the streamed drain. `post_json_to_api` exempts streaming
/// handlers from the exchange timeout (their body outlives the call), so the
/// drain of a `gpt-image-2.5` exchange carries its own bound: a stalled SSE
/// must not keep `do_generate` waiting forever.
pub(super) const STREAMED_EXCHANGE_TIMEOUT: Duration = Duration::from_secs(600);

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
        if let ImageOutputs::Base64(images) = &images {
            if images.is_empty() {
                return Err(AiMuxError::invalid_response_data(
                    "image response contained no `data[].b64_json`",
                ));
            }
            for image in images {
                validate_image_result(image)?;
            }
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
        let events = response.value;

        // The drain owns its bound: `post_json_to_api` exempts streaming
        // handlers from the exchange timeout, so without this a stalled stream
        // would keep `do_generate` waiting forever.
        let ImageStream {
            retained,
            saw_image_call,
            completed,
        } = drain_image_stream(
            events,
            &endpoint,
            &body,
            &response_headers,
            STREAMED_EXCHANGE_TIMEOUT,
        )
        .await?;

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
        let output = response_object["output"]
            .as_array()
            .map_or(&[][..], Vec::as_slice);
        let item = retained
            .or_else(|| {
                output
                    .iter()
                    .find(|item| is_completed_image_call(item))
                    .cloned()
            })
            .ok_or_else(|| {
                if saw_image_call
                    || output
                        .iter()
                        .any(|item| item["type"] == "image_generation_call")
                {
                    AiMuxError::invalid_response_data(
                        "image_generation_call item did not complete with an image result",
                    )
                } else {
                    AiMuxError::invalid_response_data(
                        "image response contained no image_generation_call item",
                    )
                }
            })?;
        let image = item["result"]
            .as_str()
            .filter(|r| !r.is_empty())
            .ok_or_else(|| {
                AiMuxError::invalid_response_data("image_generation_call item has no `result`")
            })?;
        validate_image_result(image)?;

        Ok(ImageResult {
            images: ImageOutputs::Base64(vec![image.to_string()]),
            warnings,
            provider_metadata: Some(tool_call_metadata(&item, response_object)),
            response: ImageResponse {
                timestamp: Some(timestamp),
                model_id: Some(self.model_id.clone()),
                headers: Some(response_headers),
            },
            usage: extract_image_usage(response_object),
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

/// Reject empty or malformed image payloads without exposing their contents.
fn validate_image_result(image: &str) -> Result<(), AiMuxError> {
    match aimux_provider_utils::convert_base64_to_uint8_array(image) {
        Ok(bytes) if !bytes.is_empty() => Ok(()),
        _ => Err(AiMuxError::invalid_response_data(
            "image result must contain non-empty base64 data",
        )),
    }
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

fn is_completed_image_call(item: &Value) -> bool {
    item["type"] == "image_generation_call"
        && item["status"].as_str() == Some("completed")
        && item["result"]
            .as_str()
            .is_some_and(|result| !result.is_empty())
}

/// The terminal state of an image stream.
#[derive(Debug)]
struct ImageStream {
    /// The first `image_generation_call` item that completed with a result.
    retained: Option<Value>,
    /// Whether any `image_generation_call` item was produced at all.
    saw_image_call: bool,
    /// The `response.completed` event.
    completed: Value,
}

/// Drain the stream until `response.completed`, bounded by `timeout`.
///
/// A failure event ends the drain with its error; a stream that ends without
/// `response.completed` is invalid. The bound exists because
/// `post_json_to_api` exempts streaming handlers from the exchange timeout.
async fn drain_image_stream<S>(
    mut events: S,
    endpoint: &str,
    body: &Value,
    response_headers: &HashMap<String, String>,
    timeout: Duration,
) -> Result<ImageStream, AiMuxError>
where
    S: Stream<Item = Result<Value, AiMuxError>> + Unpin,
{
    let drain = async {
        let mut retained: Option<Value> = None;
        let mut saw_image_call = false;
        while let Some(event) = events.next().await {
            let data = event.map_err(no_replay)?;
            match data.get("type").and_then(Value::as_str).unwrap_or("") {
                "response.output_item.done" => {
                    let item = &data["item"];
                    if item["type"] == "image_generation_call" {
                        saw_image_call = true;
                        if retained.is_none() && is_completed_image_call(item) {
                            retained = Some(item.clone());
                        }
                    }
                }
                "response.completed" => {
                    return Ok(ImageStream {
                        retained,
                        saw_image_call,
                        completed: data,
                    });
                }
                "error" | "response.failed" | "response.incomplete" => {
                    return Err(stream_failure(
                        &data,
                        endpoint,
                        body.clone(),
                        response_headers.clone(),
                    ));
                }
                _ => {}
            }
        }
        Err(AiMuxError::invalid_response_data(
            "image stream ended without response.completed",
        ))
    };
    match tokio::time::timeout(timeout, drain).await {
        Ok(result) => result,
        Err(_) => Err(stream_timeout(timeout, endpoint, body)),
    }
}

/// The error of a stream that outlived its hang guard. Non-retryable like every
/// other failure these models return: the generation may already be billed.
fn stream_timeout(timeout: Duration, endpoint: &str, body: &Value) -> AiMuxError {
    AiMuxError::ApiCall(Box::new(ApiCallError {
        is_retryable: false,
        origin: ApiCallErrorOrigin::Local,
        ..ApiCallError::new(
            format!(
                "Request exceeded the {}s streamed exchange response timeout",
                timeout.as_secs()
            ),
            endpoint,
            aimux_provider_utils::logging::redact_error_context(body.clone()),
        )
    }))
}

/// The error of an in-band failure event (`error`, `response.failed`,
/// `response.incomplete`, or a `response.completed` whose status is not
/// `completed`). Never retryable.
fn stream_failure(
    event: &Value,
    endpoint: &str,
    body: Value,
    response_headers: HashMap<String, String>,
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
/// when only partial), a validated `data:` URL, and any other URL downloaded
/// as `downloadBlob` does (guarded, no provider headers) and inlined.
///
/// Every payload is decoded here, so empty or corrupt caller input fails with
/// `InvalidArgument` instead of reaching the billed endpoint as an opaque
/// error.
async fn image_data_url(
    file: &ImageFile,
    options: &ImageCallOptions,
) -> Result<String, AiMuxError> {
    match file {
        ImageFile::File { media_type, data } => {
            let bytes = decoded_image_data(data)?;
            let media_type = full_image_media_type(media_type, MediaTypeData::Bytes(&bytes))?;
            Ok(inline_image_data_url(&media_type, &bytes))
        }
        ImageFile::Url { url } => {
            if let Some((media_type, bytes)) = parse_image_data_url(url)? {
                return Ok(inline_image_data_url(&media_type, &bytes));
            }
            let response =
                aimux_provider_utils::download_blob(url.clone(), options.abort_signal.clone())
                    .await?;
            let header = response
                .response_headers
                .get("content-type")
                .map(|value| value.split(';').next().unwrap_or("").trim().to_string())
                .unwrap_or_default();
            let bytes = require_image_data(response.value.to_vec())?;
            let media_type = full_image_media_type(&header, MediaTypeData::Bytes(&bytes))?;
            Ok(inline_image_data_url(&media_type, &bytes))
        }
    }
}

/// Inline image bytes as a `data:` URL.
fn inline_image_data_url(media_type: &str, bytes: &[u8]) -> String {
    format!(
        "data:{media_type};base64,{}",
        aimux_provider_utils::convert_uint8_array_to_base64(bytes)
    )
}

/// Decode inline image data: a `Base64` payload is decoded rather than passed
/// through, so malformed text fails here instead of after the call is billed.
fn decoded_image_data(data: &ImageFileData) -> Result<Vec<u8>, AiMuxError> {
    let bytes = match data {
        ImageFileData::Binary(bytes) => bytes.clone(),
        ImageFileData::Base64(text) => aimux_provider_utils::convert_base64_to_uint8_array(text)
            .map_err(|error| AiMuxError::InvalidArgument {
                message: format!("invalid image base64: {error}"),
                cause: Some(ErrorCause::source(&error)),
            })?,
    };
    require_image_data(bytes)
}

/// Refuse empty image data: an empty `data:` URL would only fail at the
/// endpoint, after the request has been sent.
fn require_image_data(bytes: Vec<u8>) -> Result<Vec<u8>, AiMuxError> {
    if bytes.is_empty() {
        return Err(AiMuxError::invalid_argument("input image data is empty"));
    }
    Ok(bytes)
}

/// The media type and decoded payload of an inline `data:` URL, or `None` for
/// any other URL.
///
/// The scheme is matched case-insensitively. Media type parameters such as
/// `charset` are accepted, but the payload must still be base64-encoded (the
/// last parameter is `base64`); a non-image or partial media type, a payload
/// without the `;base64` marker, and malformed or empty data are refused
/// before the request is sent.
fn parse_image_data_url(url: &str) -> Result<Option<(String, Vec<u8>)>, AiMuxError> {
    let Some((scheme, rest)) = url.split_once(':') else {
        return Ok(None);
    };
    if !scheme.eq_ignore_ascii_case("data") {
        return Ok(None);
    }
    let invalid = |reason: &str| {
        Err(AiMuxError::invalid_argument(format!(
            "invalid image `data:` URL: {reason}"
        )))
    };
    let Some((head, payload)) = rest.split_once(',') else {
        return invalid("missing the `,` separator");
    };
    let mut parameters = head.split(';');
    let media_type = parameters.next().unwrap_or("");
    let image_media_type = media_type
        .split_once('/')
        .filter(|(top_level, subtype)| {
            top_level.eq_ignore_ascii_case("image") && !subtype.is_empty() && *subtype != "*"
        })
        .map(|_| media_type.to_ascii_lowercase());
    let Some(media_type) = image_media_type else {
        return invalid("the media type is not a full `image/<subtype>`");
    };
    // RFC 2397 puts `;base64` last, after any media type parameters
    // (`data:image/png;charset=utf-8;base64,...`).
    if !parameters
        .last()
        .is_some_and(|parameter| parameter.eq_ignore_ascii_case("base64"))
    {
        return invalid(
            "the payload is not base64-encoded: the media type must end with `;base64`",
        );
    }
    let bytes = aimux_provider_utils::convert_base64_to_uint8_array(payload).map_err(|error| {
        AiMuxError::InvalidArgument {
            message: format!("invalid image base64: {error}"),
            cause: Some(ErrorCause::source(&error)),
        }
    })?;
    Ok(Some((media_type, require_image_data(bytes)?)))
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

    // A stalled stream must not keep the call waiting:
    // `post_json_to_api` exempts streaming handlers from the exchange timeout,
    // so the drain carries its own bound.
    #[tokio::test]
    async fn stalled_stream_is_bounded_and_not_replayed() {
        let stalled = futures::stream::pending::<Result<Value, AiMuxError>>();
        let error = drain_image_stream(
            stalled,
            "https://chatgpt.com/backend-api/codex/responses",
            &json!({ "model": RESPONSES_OUTER_MODEL }),
            &HashMap::new(),
            Duration::from_millis(20),
        )
        .await
        .expect_err("a stalled stream must not hang");
        let AiMuxError::ApiCall(detail) = &error else {
            panic!("expected ApiCall, got {error:?}");
        };
        assert_eq!(detail.origin, ApiCallErrorOrigin::Local);
        assert!(!detail.is_retryable);
        assert!(
            detail
                .message
                .contains("streamed exchange response timeout"),
            "{}",
            detail.message
        );
        assert_eq!(
            detail.url,
            "https://chatgpt.com/backend-api/codex/responses"
        );
        assert!(STREAMED_EXCHANGE_TIMEOUT >= Duration::from_secs(300));
    }

    #[test]
    fn data_urls_are_parsed_only_for_image_payloads() {
        assert_eq!(
            parse_image_data_url("https://example.test/a.png").unwrap(),
            None
        );
        let png = Some(("image/png".to_string(), b"Hello".to_vec()));
        assert_eq!(
            parse_image_data_url("data:image/png;base64,SGVsbG8=").unwrap(),
            png
        );
        // Media type parameters are allowed before the `;base64` marker.
        assert_eq!(
            parse_image_data_url("data:image/png;charset=utf-8;base64,SGVsbG8=").unwrap(),
            png
        );
        // The scheme, media type and marker are case-insensitive; the rebuilt
        // data URL is normalized.
        assert_eq!(
            parse_image_data_url("DATA:IMAGE/PNG;BASE64,SGVsbG8=").unwrap(),
            png
        );
        assert_eq!(
            parse_image_data_url("DATA:IMAGE/PNG;CHARSET=UTF-8;BASE64,SGVsbG8=").unwrap(),
            png
        );
        for invalid in [
            "data:text/plain;base64,SGVsbG8=",
            "data:image;base64,SGVsbG8=",
            "data:image/*;base64,SGVsbG8=",
            "data:image/png,SGVsbG8=",
            "data:image/png;base64;charset=utf-8,SGVsbG8=",
            "data:image/png;base64,not base64!!",
            "data:image/png;base64,",
            "data:image/png",
        ] {
            let error = parse_image_data_url(invalid).expect_err(invalid);
            assert!(
                matches!(error, AiMuxError::InvalidArgument { .. }),
                "{invalid}: {error:?}"
            );
        }
        // The error names the actual reason: the base64 marker is missing, not
        // the payload being non-base64 text.
        let error = parse_image_data_url("data:image/png,SGVsbG8=").expect_err("no marker");
        assert!(
            error.to_string().contains("must end with `;base64`"),
            "{error}"
        );
    }
}
