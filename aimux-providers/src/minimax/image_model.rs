//! MiniMax H3 image generation model (`/v1/images/generations`).

use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::image_model::{
    ImageCallOptions, ImageModel, ImageOutputs, ImageResult, ImageUsage,
};
use aimux_core::shared::Warning;
use aimux_provider_utils::HttpRequest;

use crate::shared::{EndpointConfig, image_response, retry_download, unsupported};

use super::options::{self, ImageOptions, NAMESPACE};
use super::video_model::{base_resp_error, minimax_failed_response_handler};
use super::{BaseResp, not_retryable, not_retryable_unless_rate_limited};

const MODELS: [&str; 2] = ["MiniMax-H3-Image", "MiniMax-H3-Image-Highspeed"];
const MAX_PROMPT_CHARS: usize = 32_000;
const AREAS: [&str; 3] = ["1K", "2K", "4K"];
const MIN_PIXELS: u64 = 655_360;
const MAX_PIXELS: u64 = 20_000_000;
const AUTO: &str = "auto";

/// MiniMax holds the generation request open for up to 600 s; the exchange
/// may take that long plus transport overhead before it is a dead
/// connection. A shorter local bound would fail a generation that MiniMax
/// still completes and bills.
const GENERATE_RESPONSE_TIMEOUT: Duration = Duration::from_secs(600 + 15);

/// Wait between two attempts to download a generated image.
const DOWNLOAD_RETRY_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Debug, Deserialize)]
struct GenerateResponse {
    #[serde(default)]
    data: Vec<GeneratedImage>,
    #[serde(default)]
    output_format: Option<String>,
    #[serde(default)]
    usage: Option<GenerateUsage>,
    #[serde(default)]
    request_id: Option<String>,
    #[serde(default)]
    base_resp: Option<BaseResp>,
}

#[derive(Debug, Deserialize)]
struct GeneratedImage {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    size: Option<String>,
    #[serde(default)]
    revised_prompt: Option<String>,
    #[serde(default)]
    error: Option<ImageError>,
}

#[derive(Debug, Deserialize)]
struct ImageError {
    #[serde(default)]
    code: Option<Value>,
    #[serde(default)]
    message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GenerateUsage {
    #[serde(default)]
    input_tokens: Option<u32>,
    #[serde(default)]
    output_tokens: Option<u32>,
    #[serde(default)]
    total_tokens: Option<u32>,
}

/// A MiniMax H3 image model — implements [`ImageModel`].
pub struct MiniMaxImageModel {
    model_id: String,
    config: EndpointConfig,
}

impl MiniMaxImageModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }
}

/// `X:Y` with X and Y from 1 to 300 and a ratio from 1:3 to 3:1.
fn check_aspect_ratio(value: &str) -> Result<(), AiMuxError> {
    let parsed = value.split_once(':').and_then(|(x, y)| {
        let (x, y) = (x.parse::<u64>().ok()?, y.parse::<u64>().ok()?);
        ((1..=300).contains(&x) && (1..=300).contains(&y)).then_some((x, y))
    });
    match parsed {
        Some((x, y)) if x <= 3 * y && y <= 3 * x => Ok(()),
        _ => Err(AiMuxError::invalid_argument(format!(
            "MiniMax image aspect ratio must be `auto` or X:Y (X, Y 1 to 300, 1:3 to 3:1), got `{value}`"
        ))),
    }
}

/// The request body, and the warnings for the call options MiniMax does not
/// take.
fn build_body(
    model_id: &str,
    options: &ImageCallOptions,
) -> Result<(Value, Vec<Warning>), AiMuxError> {
    if !MODELS.contains(&model_id) {
        return Err(AiMuxError::invalid_argument(format!(
            "MiniMax image model `{model_id}` is not supported; supported models: {}",
            MODELS.join(", ")
        )));
    }
    if options.files.as_ref().is_some_and(|f| !f.is_empty()) || options.mask.is_some() {
        return Err(AiMuxError::UnsupportedFunctionality(
            "MiniMax H3 image models do not edit images: input files and masks are not supported"
                .to_string(),
        ));
    }
    if options.n != 1 {
        return Err(AiMuxError::invalid_argument(format!(
            "MiniMax H3 image models generate one image per call, got n = {}",
            options.n
        )));
    }
    let opts: ImageOptions = options::parse(&options.provider_options)?;
    let mut warnings = Vec::new();

    let prompt = options.prompt.as_deref().unwrap_or_default();
    let prompt_chars = prompt.trim().chars().count();
    if prompt_chars == 0 || prompt_chars > MAX_PROMPT_CHARS {
        return Err(AiMuxError::invalid_argument(format!(
            "MiniMax image prompt must be 1 to {MAX_PROMPT_CHARS} characters, got {prompt_chars}"
        )));
    }

    let size = options.size.map(|size| {
        let (w, h) = (u64::from(size.width()), u64::from(size.height()));
        (size, w, h)
    });
    if let Some((size, w, h)) = size {
        let pixels = w * h;
        if !(MIN_PIXELS..=MAX_PIXELS).contains(&pixels) || w > 3 * h || h > 3 * w {
            return Err(AiMuxError::invalid_argument(format!(
                "MiniMax image size must cover 655,360 to 20,000,000 pixels with an aspect ratio of 1:3 to 3:1, got {size}"
            )));
        }
    }
    let aspect_ratio = opts
        .aspect_ratio
        .clone()
        .or_else(|| options.aspect_ratio.map(|ar| ar.to_string()));
    if let Some(ratio) = aspect_ratio.as_deref()
        && ratio != AUTO
    {
        check_aspect_ratio(ratio)?;
    }
    if let Some(area) = opts.area.as_deref()
        && !AREAS.contains(&area)
    {
        return Err(AiMuxError::invalid_argument(format!(
            "MiniMax image area must be one of {}, got `{area}`",
            AREAS.join(", ")
        )));
    }
    if size.is_some() && (aspect_ratio.is_some() || opts.area.is_some()) {
        return Err(AiMuxError::invalid_argument(
            "MiniMax image size cannot be combined with an aspect ratio or an area",
        ));
    }
    if opts.disable_prompt_enhanced == Some(true)
        && size.is_none()
        && aspect_ratio.as_deref().is_none_or(|ratio| ratio == AUTO)
    {
        return Err(AiMuxError::invalid_argument(
            "MiniMax disablePromptEnhanced needs an explicit size or an X:Y aspect ratio",
        ));
    }
    if let Some(compression) = opts.output_compression
        && !(0..=100).contains(&compression)
    {
        return Err(AiMuxError::invalid_argument(format!(
            "MiniMax outputCompression must be 0 to 100, got {compression}"
        )));
    }
    if options.seed.is_some() {
        warnings.push(unsupported(
            "seed",
            "MiniMax H3 image models do not take a seed.",
        ));
    }

    let mut body = Map::new();
    body.insert("model".into(), json!(model_id));
    body.insert("prompt".into(), json!(prompt));
    body.insert("n".into(), json!(1));
    body.insert("response_format".into(), json!("url"));
    body.insert("stream".into(), json!(false));
    if let Some((size, ..)) = size {
        body.insert("size".into(), json!(size.to_string()));
    }
    if let Some(ratio) = aspect_ratio {
        body.insert("aspect_ratio".into(), json!(ratio));
    }
    if let Some(area) = opts.area {
        body.insert("area".into(), json!(area));
    }
    if let Some(disable) = opts.disable_prompt_enhanced {
        body.insert("disable_prompt_enhanced".into(), json!(disable));
    }
    if let Some(compression) = opts.output_compression {
        body.insert("output_compression".into(), json!(compression));
    }
    if let Some(watermark) = opts.aigc_watermark {
        body.insert("aigc_watermark".into(), json!(watermark));
    }
    Ok((Value::Object(body), warnings))
}

#[async_trait]
impl ImageModel for MiniMaxImageModel {
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
        Some(false)
    }

    fn supports_mask_inputs(&self) -> Option<bool> {
        Some(false)
    }

    /// Generate one image and download it.
    ///
    /// MiniMax bills a generation even when the client gives up on it, and a
    /// repeated request generates and bills again. So no failure of this call
    /// is retryable except an HTTP 429 on the generation request: not a
    /// transport failure or timeout, not a 5xx, and not a failed download
    /// (the download itself is attempted a few times, without the key).
    async fn do_generate(&self, options: &ImageCallOptions) -> Result<ImageResult, AiMuxError> {
        let (body, warnings) = build_body(&self.model_id, options)?;
        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let url = exchange.url("/v1/images/generations");
        let mut request = exchange.request(url.clone(), options);
        request.response_timeout = Some(GENERATE_RESPONSE_TIMEOUT);
        let resp = aimux_provider_utils::post_json_to_api(
            request,
            body,
            aimux_provider_utils::create_json_response_handler::<GenerateResponse>(),
            minimax_failed_response_handler(),
        )
        .await
        .map_err(not_retryable_unless_rate_limited)?;

        let headers = resp.response_headers;
        let response_body = resp.raw_value.as_ref().map(ToString::to_string);
        let generated = resp.value;
        if let Some((code, message)) = generated.base_resp.as_ref().and_then(BaseResp::failure) {
            return Err(base_resp_error(code, message, url, response_body, false));
        }
        let image = generated.data.into_iter().next().ok_or_else(|| {
            AiMuxError::invalid_response_data("MiniMax image generation returned no image")
        })?;
        if let Some(error) = image.error {
            let code = error
                .code
                .as_ref()
                .and_then(crate::shared::json_error_code)
                .unwrap_or_else(|| "image_error".to_string());
            return Err(crate::shared::task_failed(
                format!(
                    "MiniMax image generation failed: {}",
                    error.message.as_deref().unwrap_or("no image was generated")
                ),
                url,
                code,
                response_body,
            ));
        }
        let image_url = image
            .url
            .filter(|u| url::Url::parse(u).is_ok_and(|p| matches!(p.scheme(), "http" | "https")))
            .ok_or_else(|| {
                AiMuxError::invalid_response_data(
                    "MiniMax image generation returned no http(s) image URL",
                )
            })?;

        // The URL comes from the response body: it goes through the SSRF
        // guard, and no provider header (the key) is sent with it.
        let downloaded = retry_download(
            options.abort_signal.as_ref(),
            DOWNLOAD_RETRY_INTERVAL,
            || {
                aimux_provider_utils::get_from_api(
                    HttpRequest {
                        url: image_url.clone(),
                        abort_signal: options.abort_signal.clone(),
                        validate_url: true,
                        trusted_origin: Some(exchange.base_url().to_string()),
                        ..Default::default()
                    },
                    aimux_provider_utils::create_binary_response_handler(),
                    aimux_provider_utils::create_status_code_error_response_handler(),
                )
            },
        )
        .await
        .map_err(not_retryable)?;

        let mut metadata = Map::new();
        for (key, value) in [
            ("outputFormat", generated.output_format.map(Value::from)),
            ("size", image.size.map(Value::from)),
            ("revisedPrompt", image.revised_prompt.map(Value::from)),
            ("requestId", generated.request_id.map(Value::from)),
        ] {
            if let Some(value) = value {
                metadata.insert(key.into(), value);
            }
        }
        let mut provider_metadata = aimux_core::shared::SharedProviderMetadata::new();
        provider_metadata.insert(NAMESPACE.to_string(), metadata);

        Ok(ImageResult {
            images: ImageOutputs::Binary(vec![downloaded.value.to_vec()]),
            warnings,
            provider_metadata: Some(provider_metadata),
            response: image_response(aimux_core::Timestamp::now(), &self.model_id, headers),
            usage: generated.usage.map(|usage| ImageUsage {
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
                total_tokens: usage.total_tokens,
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aimux_core::image_model::ImageFile;

    fn with_minimax(mut options: ImageCallOptions, value: Value) -> ImageCallOptions {
        let Value::Object(obj) = value else { panic!() };
        options.provider_options.insert(NAMESPACE.into(), obj);
        options
    }

    fn err(options: &ImageCallOptions) -> String {
        build_body("MiniMax-H3-Image", options)
            .unwrap_err()
            .to_string()
    }

    #[test]
    fn a_plain_call_sends_one_url_image_without_streaming() {
        let (body, warnings) = build_body(
            "MiniMax-H3-Image-Highspeed",
            &ImageCallOptions::new("a cat"),
        )
        .unwrap();
        assert_eq!(
            body,
            json!({
                "model": "MiniMax-H3-Image-Highspeed",
                "prompt": "a cat",
                "n": 1,
                "response_format": "url",
                "stream": false,
            })
        );
        assert!(warnings.is_empty());
        assert!(
            build_body("image-01", &ImageCallOptions::new("x"))
                .unwrap_err()
                .to_string()
                .contains("not supported")
        );
    }

    #[test]
    fn size_limits_are_checked() {
        let mut o = ImageCallOptions::new("x");
        o.size = Some("2720x1536".parse().unwrap());
        assert_eq!(
            build_body("MiniMax-H3-Image", &o).unwrap().0["size"],
            "2720x1536"
        );
        o.size = Some("512x512".parse().unwrap());
        assert!(err(&o).contains("655,360"));
        o.size = Some("3100x1000".parse().unwrap());
        assert!(err(&o).contains("1:3 to 3:1"));
        o.size = Some("1024x640".parse().unwrap());
        assert!(build_body("MiniMax-H3-Image", &o).is_ok());
    }

    #[test]
    fn aspect_ratio_and_area_exclude_an_explicit_size() {
        let o = with_minimax(
            ImageCallOptions::new("x"),
            json!({"aspectRatio": "21:9", "area": "2K"}),
        );
        let (body, _) = build_body("MiniMax-H3-Image", &o).unwrap();
        assert_eq!(body["aspect_ratio"], "21:9");
        assert_eq!(body["area"], "2K");

        let mut o2 = ImageCallOptions::new("x");
        o2.aspect_ratio = Some("16:9".parse().unwrap());
        assert_eq!(
            build_body("MiniMax-H3-Image", &o2).unwrap().0["aspect_ratio"],
            "16:9"
        );
        o2.size = Some("2720x1536".parse().unwrap());
        assert!(err(&o2).contains("cannot be combined"));

        let mut o3 = with_minimax(ImageCallOptions::new("x"), json!({"area": "4K"}));
        o3.size = Some("2720x1536".parse().unwrap());
        assert!(err(&o3).contains("cannot be combined"));

        for bad in ["4:1", "301:300", "0:1", "16x9"] {
            let o = with_minimax(ImageCallOptions::new("x"), json!({"aspectRatio": bad}));
            assert!(err(&o).contains("X:Y"), "{bad}");
        }
        let o = with_minimax(ImageCallOptions::new("x"), json!({"area": "8K"}));
        assert!(err(&o).contains("1K, 2K, 4K"));
    }

    #[test]
    fn disabling_prompt_enhancement_needs_an_explicit_shape() {
        let o = with_minimax(
            ImageCallOptions::new("x"),
            json!({"disablePromptEnhanced": true}),
        );
        assert!(err(&o).contains("explicit size"));
        let o = with_minimax(
            ImageCallOptions::new("x"),
            json!({"disablePromptEnhanced": true, "aspectRatio": "auto"}),
        );
        assert!(err(&o).contains("explicit size"));
        let o = with_minimax(
            ImageCallOptions::new("x"),
            json!({"disablePromptEnhanced": true, "aspectRatio": "3:4"}),
        );
        assert!(build_body("MiniMax-H3-Image", &o).is_ok());
        let mut o = with_minimax(
            ImageCallOptions::new("x"),
            json!({"disablePromptEnhanced": true}),
        );
        o.size = Some("1024x1024".parse().unwrap());
        assert!(build_body("MiniMax-H3-Image", &o).is_ok());
    }

    #[test]
    fn other_options_are_checked() {
        let o = with_minimax(
            ImageCallOptions::new("x"),
            json!({"outputCompression": 101}),
        );
        assert!(err(&o).contains("0 to 100"));
        let o = with_minimax(ImageCallOptions::new("x"), json!({"quality": "hd"}));
        assert!(err(&o).contains("invalid minimax provider options"));
        assert!(err(&ImageCallOptions::new(" ")).contains("1 to 32000"));
        assert!(err(&ImageCallOptions::new("a".repeat(32_001))).contains("got 32001"));

        let mut o = ImageCallOptions::new("x");
        o.n = 2;
        assert!(err(&o).contains("one image per call"));
        let mut o = ImageCallOptions::new("x");
        o.files = Some(vec![ImageFile::Url {
            url: "https://example.com/a.png".into(),
        }]);
        assert!(matches!(
            build_body("MiniMax-H3-Image", &o),
            Err(AiMuxError::UnsupportedFunctionality(_))
        ));
        let mut o = ImageCallOptions::new("x");
        o.seed = Some(7);
        assert_eq!(build_body("MiniMax-H3-Image", &o).unwrap().1.len(), 1);
    }
}
