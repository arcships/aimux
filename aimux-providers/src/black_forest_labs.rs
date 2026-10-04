//! Black Forest Labs image provider.
//!
//! Aligned with Vercel AI SDK `BlackForestLabsImageModel`
//! (`reference/ai/packages/black-forest-labs/src/black-forest-labs-image-model.ts`).
//!
//! Uses an async submit + poll pattern: POST to submit, then GET poll_url
//! until status is "Ready", then download the image.
//!
//! [`create_black_forest_labs`] takes [`BlackForestLabsProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`BlackForestLabsProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `BFL_API_KEY`.
//! [`black_forest_labs()`] is the default instance; it reads nothing and cannot fail.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::error::ApiCallError;
use aimux_core::image_model::{
    ImageCallOptions, ImageFile, ImageFileData, ImageModel, ImageOutputs, ImageResponse,
    ImageResult,
};
use aimux_core::shared::Warning;
use aimux_provider_utils::HttpRequest;
use aimux_provider_utils::{FetchFunction, HeaderMapOpt, HeadersFn, Resolvable, validate_base_url};

use crate::shared::{
    Credential, EndpointConfig, POLL_INTERVAL_MILLIS_KEY, PollStep, is_poll_control_key,
    poll_interval_ms, poll_until, provider_headers, retry_download,
};

/// AI SDK's `isTrustedUrl` (black-forest-labs-api.ts): credentials may go to
/// the configured origin, or over HTTPS to `bfl.ai` and its subdomains — BFL
/// serves polling and asset URLs from regional clusters. Allowlisted hosts
/// still get full URL/DNS validation; this gates only the headers.
fn bfl_trusted_url(url: &str, base_url: &str) -> bool {
    if aimux_provider_utils::same_origin(url, base_url) {
        return true;
    }
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    parsed.scheme() == "https"
        && parsed
            .host_str()
            .is_some_and(|host| host == "bfl.ai" || host.ends_with(".bfl.ai"))
}

fn bfl_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    aimux_provider_utils::create_json_error_response_handler(|data| {
        let detail = data.get("detail");
        let message = detail
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                detail
                    .filter(|value| !value.is_null())
                    .map(Value::to_string)
            })
            .or_else(|| {
                data.get("message")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "Unknown Black Forest Labs error".to_string());
        aimux_provider_utils::ProviderErrorParts {
            message,
            provider_code: None,
        }
    })
}

/// Milliseconds between two polls of a generation
/// (`providerOptions.blackForestLabs.pollIntervalMillis` overrides it for one call).
const POLL_INTERVAL_MS: u64 = 500;
/// How many times a generation is polled before the call gives up (the sixty
/// seconds of the AI SDK at the default interval).
const MAX_POLL_ATTEMPTS: u32 = 120;

/// What a finished generation reports.
struct BflResult {
    image_url: String,
    seed: Option<Value>,
    start_time: Option<Value>,
    end_time: Option<Value>,
    duration: Option<Value>,
}

pub(crate) mod options;

const DEFAULT_BASE_URL: &str = "https://api.bfl.ai";
const API_KEY_ENV_VAR: &str = "BFL_API_KEY";
const DEFAULT_NAME: &str = "blackForestLabs";

/// Settings of [`create_black_forest_labs`].
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except `base_url` and `name`; `api_key` and `headers` are
/// evaluated on every request.
#[derive(Clone, Default)]
pub struct BlackForestLabsProviderSettings {
    /// Base URL for the API calls. Default `https://api.bfl.ai`; a
    /// trailing slash is removed.
    pub base_url: Option<String>,
    /// The API key. `None` loads `BFL_API_KEY` when a request is made and
    /// fails that request with `AiMuxError::LoadApiKey` if it is unset. An
    /// explicit value is used as given, `""` included: it never falls back to
    /// the environment. A [`Resolvable::Future`] is awaited once, an
    /// [`Resolvable::AsyncFn`] on every request.
    pub api_key: Option<Resolvable<String>>,
    /// Extra headers on every request. A `None` value removes the header,
    /// including the credential. Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The provider name, the prefix of the `provider()` strings (`"{name}.image"`).
    /// Default `"blackForestLabs"`. The providerOptions key stays `blackForestLabs`.
    pub name: Option<String>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
}

impl std::fmt::Debug for BlackForestLabsProviderSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BlackForestLabsProviderSettings")
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key)
            .field(
                "headers",
                &self.headers.as_ref().map(std::collections::HashMap::len),
            )
            .field("name", &self.name)
            .field("fetch", &self.fetch.is_some())
            .finish()
    }
}

/// Create a Black Forest Labs provider.
///
/// # Errors
///
/// Returns `AiMuxError::InvalidArgument` when `base_url` is not an `http(s)`
/// URL with a host. That is the only way this fails: the key is loaded per
/// request, not here.
pub fn create_black_forest_labs(
    settings: BlackForestLabsProviderSettings,
) -> Result<BlackForestLabsProvider, AiMuxError> {
    let base_url = match settings.base_url.as_deref() {
        Some(url) => validate_base_url(url)?,
        None => DEFAULT_BASE_URL.to_string(),
    };
    Ok(BlackForestLabsProvider {
        name: settings.name.unwrap_or_else(|| DEFAULT_NAME.to_string()),
        base_url,
        headers: provider_headers(
            Credential::explicit_or_env(settings.api_key, API_KEY_ENV_VAR, "Black Forest Labs"),
            Vec::new(),
            settings.headers,
        ),
        fetch: settings.fetch,
    })
}

/// The default provider: `create_black_forest_labs` with default settings, created on
/// first use. Creating it reads nothing from the environment and cannot fail;
/// a missing key surfaces from the first request instead.
pub fn black_forest_labs() -> &'static BlackForestLabsProvider {
    static DEFAULT: OnceLock<BlackForestLabsProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_black_forest_labs(BlackForestLabsProviderSettings::default())
            .expect("default Black Forest Labs settings are always valid")
    })
}

/// A Black Forest Labs provider. Cheap to clone the models out of; it holds no HTTP
/// client.
pub struct BlackForestLabsProvider {
    name: String,
    base_url: String,
    headers: HeadersFn,
    fetch: Option<FetchFunction>,
}

impl BlackForestLabsProvider {
    fn model_config(&self, method: &str) -> EndpointConfig {
        EndpointConfig::fixed(
            format!("{}.{method}", self.name),
            self.base_url.clone(),
            self.headers.clone(),
            self.fetch.clone(),
            None,
        )
    }

    /// An image model (e.g. `"flux-pro-1.1"`); `provider()` is `"{name}.image"`.
    #[must_use]
    pub fn image(&self, model_id: &str) -> BlackForestLabsImageModel {
        BlackForestLabsImageModel::from_config(model_id.to_string(), self.model_config("image"))
    }
}

crate::impl_single_modality_provider!(BlackForestLabsProvider, image_model, |p, id| p.image(id));

pub struct BlackForestLabsImageModel {
    model_id: String,
    config: EndpointConfig,
}
impl BlackForestLabsImageModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }

    fn file_to_string(file: &ImageFile) -> Result<String, AiMuxError> {
        match file {
            ImageFile::Url { url } => Ok(url.clone()),
            ImageFile::File { data, .. } => match data {
                ImageFileData::Base64(s) => Ok(s.clone()),
                ImageFileData::Binary(b) => Ok(base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    b,
                )),
            },
        }
    }

    fn convert_size_to_aspect_ratio(size: &aimux_core::shared::Size) -> Option<String> {
        let (w, h) = (size.width(), size.height());
        if w == 0 || h == 0 {
            return None;
        }
        let g = gcd(w, h);
        Some(format!("{}:{}", w / g, h / g))
    }
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 { a } else { gcd(b, a % b) }
}

#[async_trait]
impl ImageModel for BlackForestLabsImageModel {
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
        let mut warnings: Vec<Warning> = Vec::new();

        let final_aspect_ratio = if let Some(ar) = options.aspect_ratio {
            Some(ar.to_string())
        } else if let Some(size) = options.size {
            if let Some(ar) = Self::convert_size_to_aspect_ratio(&size) {
                warnings.push(Warning::Unsupported { feature: "size".into(), details: Some("Deriving aspect_ratio from size. Use the width and height provider options to specify dimensions for models that support them.".into()) });
                Some(ar)
            } else {
                None
            }
        } else {
            None
        };

        if options.size.is_some() && options.aspect_ratio.is_some() {
            warnings.push(Warning::Unsupported { feature: "size".into(), details: Some("Black Forest Labs ignores size when aspectRatio is provided. Use the width and height provider options to specify dimensions for models that support them".into()) });
        }

        let bfl_opts = options::black_forest_labs_options(Some(&options.provider_options));
        let (width_str, height_str) = options
            .size
            .map(|s| (s.width().to_string(), s.height().to_string()))
            .unwrap_or((String::new(), String::new()));

        // Build input images
        let input_images: Vec<String> = if let Some(ref files) = options.files {
            files
                .iter()
                .map(Self::file_to_string)
                .collect::<Result<_, _>>()?
        } else {
            Vec::new()
        };

        if input_images.len() > 10 {
            return Err(AiMuxError::InvalidArgument(
                "Black Forest Labs supports up to 10 input images.".into(),
            ));
        }

        let input_field = if self.model_id == "flux-pro-1.0-fill" {
            "image"
        } else {
            "input_image"
        };
        let mut input_images_obj = Map::new();
        for (i, img) in input_images.iter().enumerate() {
            let key = if i == 0 {
                input_field.to_string()
            } else {
                format!("{}_{}", input_field, i + 1)
            };
            input_images_obj.insert(key, json!(img));
        }

        let mask_value = if let Some(ref mask) = options.mask {
            Some(Self::file_to_string(mask)?)
        } else {
            None
        };

        let mut body = Map::new();
        if let Some(ref p) = options.prompt {
            body.insert("prompt".into(), json!(p));
        }
        if let Some(seed) = options.seed {
            body.insert("seed".into(), json!(seed));
        }
        if let Some(ref ar) = final_aspect_ratio {
            body.insert("aspect_ratio".into(), json!(ar));
        }
        // width/height from provider options or size
        let width = bfl_opts.and_then(|o| o.get("width")).cloned().or_else(|| {
            if options.size.is_some() {
                Some(json!(width_str))
            } else {
                None
            }
        });
        let height = bfl_opts.and_then(|o| o.get("height")).cloned().or_else(|| {
            if options.size.is_some() {
                Some(json!(height_str))
            } else {
                None
            }
        });
        if let Some(w) = width {
            body.insert("width".into(), w);
        }
        if let Some(h) = height {
            body.insert("height".into(), h);
        }

        // Forward BFL provider options
        if let Some(bfl) = bfl_opts {
            let map: &[(&str, &str)] = &[
                ("steps", "steps"),
                ("guidance", "guidance"),
                ("imagePromptStrength", "image_prompt_strength"),
                ("imagePrompt", "image_prompt"),
                ("outputFormat", "output_format"),
                ("promptUpsampling", "prompt_upsampling"),
                ("raw", "raw"),
                ("safetyTolerance", "safety_tolerance"),
                ("webhookSecret", "webhook_secret"),
                ("webhookUrl", "webhook_url"),
            ];
            for (key, value) in bfl {
                if is_poll_control_key(key) || matches!(key.as_str(), "width" | "height") {
                    continue;
                }
                let ak = map
                    .iter()
                    .find(|(k, _)| *k == key)
                    .map(|(_, v)| v.to_string())
                    .unwrap_or_else(|| key.clone());
                body.insert(ak, value.clone());
            }
        }
        for (k, v) in &input_images_obj {
            body.insert(k.clone(), v.clone());
        }
        if let Some(m) = mask_value {
            body.insert("mask".into(), json!(m));
        }

        let exchange = self.config.exchange(options.headers.as_ref()).await?;

        // Submit. This is the only request that creates a generation: nothing
        // below sends it again.
        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(exchange.url(&format!("/{}", self.model_id)), options),
            Value::Object(body),
            aimux_provider_utils::create_json_response_handler(),
            bfl_failed_response_handler(),
        )
        .await?;
        let submit_body: Value = resp.value;

        let poll_url = submit_body
            .get("polling_url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                AiMuxError::InvalidResponseData("missing polling_url in BFL response".to_string())
            })?
            .to_string();
        let request_id = submit_body
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let poll_interval = Duration::from_millis(poll_interval_ms(
            bfl_opts,
            POLL_INTERVAL_MILLIS_KEY,
            POLL_INTERVAL_MS,
        ));

        let mut poll_url_with_id = url::Url::parse(&poll_url)
            .map_err(|e| AiMuxError::InvalidResponseData(format!("invalid BFL poll URL: {e}")))?;
        if !poll_url_with_id.query_pairs().any(|(k, _)| k == "id") {
            poll_url_with_id
                .query_pairs_mut()
                .append_pair("id", &request_id);
        }
        let poll_url = poll_url_with_id.to_string();

        // AI SDK gates headers per URL via isTrustedUrl; response-supplied
        // targets outside the BFL allowlist get none.
        let gated_request = |url: &str| -> HttpRequest {
            let mut request = exchange.request(url.to_string(), options);
            // Headers are gated per URL here, not by the credentialed origin.
            request.credentialed_origin = None;
            if !bfl_trusted_url(url, exchange.base_url()) {
                request.headers = Vec::new();
            }
            request.validate_url = true;
            request.trusted_origin = Some(exchange.base_url().to_string());
            request
        };

        // Poll for the result. AI SDK polls polling_url with validateUrl: true
        // and gates the headers itself via isTrustedUrl (base_url origin or
        // HTTPS *.bfl.ai).
        let ready = poll_until(
            &format!("blackForestLabs task {request_id}"),
            options.abort_signal.as_ref(),
            poll_interval,
            MAX_POLL_ATTEMPTS,
            || async {
                let pr = aimux_provider_utils::get_from_api(
                    gated_request(&poll_url),
                    aimux_provider_utils::create_json_response_handler::<Value>(),
                    bfl_failed_response_handler(),
                )
                .await?;
                let response_body = pr.raw_value.as_ref().map(ToString::to_string);
                let pv = pr.value;

                let poll_status = pv
                    .get("status")
                    .and_then(|v| v.as_str())
                    .or_else(|| pv.get("state").and_then(|v| v.as_str()))
                    .unwrap_or("");
                if poll_status == "Ready" {
                    let result = pv.get("result");
                    let sample = result
                        .and_then(|r| r.get("sample"))
                        .and_then(|v| v.as_str())
                        .map(String::from)
                        .ok_or_else(|| {
                            AiMuxError::InvalidResponseData(
                                "BFL poll reported Ready without result.sample".to_string(),
                            )
                        })?;
                    return Ok(PollStep::Ready(BflResult {
                        image_url: sample,
                        seed: result.and_then(|r| r.get("seed")).cloned(),
                        start_time: result.and_then(|r| r.get("start_time")).cloned(),
                        end_time: result.and_then(|r| r.get("end_time")).cloned(),
                        duration: result.and_then(|r| r.get("duration")).cloned(),
                    }));
                }
                if poll_status == "Error" || poll_status == "Failed" {
                    return Err(AiMuxError::ApiCall(Box::new(ApiCallError {
                        status_code: Some(200),
                        provider_code: Some(poll_status.to_string()),
                        message: "Black Forest Labs generation failed.".into(),
                        response_body,
                        ..ApiCallError::new(
                            "Black Forest Labs generation failed.",
                            poll_url.clone(),
                            serde_json::json!({}),
                        )
                    })));
                }
                Ok(PollStep::Pending)
            },
        )
        .await?;
        let image_url = ready.image_url;
        let result_seed = ready.seed;
        let result_start_time = ready.start_time;
        let result_end_time = ready.end_time;
        let result_duration = ready.duration;

        // Download image; result.sample is a URL from the poll response body.
        // AI SDK sends its headers to trusted BFL hosts on the download too,
        // gated by the same allowlist.
        let ir = retry_download(options.abort_signal.as_ref(), poll_interval, || {
            aimux_provider_utils::get_from_api(
                gated_request(&image_url),
                aimux_provider_utils::create_binary_response_handler(),
                bfl_failed_response_handler(),
            )
        })
        .await?;
        let image_bytes = ir.value.to_vec();
        let download_headers: HashMap<String, String> = HashMap::new();

        // Build provider metadata
        let mut metadata = HashMap::new();
        let mut bfl_meta = Map::new();
        let mut img_meta = Map::new();
        if let Some(s) = result_seed {
            img_meta.insert("seed".into(), s);
        }
        if let Some(s) = result_start_time {
            img_meta.insert("start_time".into(), s);
        }
        if let Some(e) = result_end_time {
            img_meta.insert("end_time".into(), e);
        }
        if let Some(d) = result_duration {
            img_meta.insert("duration".into(), d);
        }
        if let Some(c) = submit_body.get("cost") {
            img_meta.insert("cost".into(), c.clone());
        }
        if let Some(i) = submit_body.get("input_mp") {
            img_meta.insert("inputMegapixels".into(), i.clone());
        }
        if let Some(o) = submit_body.get("output_mp") {
            img_meta.insert("outputMegapixels".into(), o.clone());
        }
        bfl_meta.insert("images".into(), json!([Value::Object(img_meta)]));
        metadata.insert(options::NAMESPACE.into(), bfl_meta);

        Ok(ImageResult {
            images: ImageOutputs::Binary(vec![image_bytes]),
            warnings,
            provider_metadata: Some(metadata),
            response: ImageResponse {
                timestamp: Some(chrono::Utc::now().to_rfc3339()),
                model_id: Some(self.model_id.clone()),
                headers: Some(download_headers),
            },
            usage: None,
        })
    }
}
