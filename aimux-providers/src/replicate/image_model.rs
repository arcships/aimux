//! Replicate image generation model.

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::image_model::{ImageCallOptions, ImageModel, ImageOutputs, ImageResult};
use aimux_core::shared::Warning;
use aimux_provider_utils::HttpRequest;

use crate::shared::{
    EndpointConfig, POLL_INTERVAL_MS_KEY, image_response, is_poll_control_key, namespaced_options,
    poll_interval_ms, retry_download,
};

use super::provider::replicate_failed_response_handler;

/// Replicate's `prefer: wait` holds the connection for at most this long when
/// no explicit duration is given (their documented default and maximum).
const DEFAULT_WAIT_SECONDS: u64 = 60;

/// Headroom added on top of the wait window for connect/TLS/body transport
/// when sizing the create exchange's `response_timeout`.
const WAIT_TRANSPORT_MARGIN_SECONDS: u64 = 5;

/// Milliseconds between two attempts to download a finished prediction's output
/// (`providerOptions.replicate.pollIntervalMs` overrides it for one call).
const DOWNLOAD_RETRY_INTERVAL_MS: u64 = 1_000;

/// The providerOptions key, read whatever the provider is named.
pub(crate) const NAMESPACE: &str = "replicate";

const FLUX_2_PATTERN: &str = "black-forest-labs/flux-2-";
const MAX_FLUX_2_INPUT_IMAGES: u32 = 8;

/// A Replicate image generation model.
pub struct ReplicateImageModel {
    model_id: String,
    config: EndpointConfig,
}

impl ReplicateImageModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }

    fn is_flux2(&self) -> bool {
        self.model_id.starts_with(FLUX_2_PATTERN)
    }

    /// The path of the prediction endpoint of this model; a `:version` suffix
    /// of the model id is sent in the body, not in the path.
    fn endpoint_path(&self) -> String {
        let model = self
            .model_id
            .split_once(':')
            .map_or(self.model_id.as_str(), |(model, _version)| model);
        format!("/models/{model}/predictions")
    }
}

#[async_trait]
impl ImageModel for ReplicateImageModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }
    fn model_id(&self) -> &str {
        &self.model_id
    }
    fn max_images_per_call(&self) -> Option<u32> {
        if self.is_flux2() {
            Some(MAX_FLUX_2_INPUT_IMAGES)
        } else {
            Some(1)
        }
    }

    async fn do_generate(&self, options: &ImageCallOptions) -> Result<ImageResult, AiMuxError> {
        let mut warnings: Vec<Warning> = Vec::new();
        let (_model_id, version) = self
            .model_id
            .split_once(':')
            .unwrap_or((&self.model_id, ""));

        let replicate_opts = namespaced_options(Some(&options.provider_options), NAMESPACE);
        let max_wait = replicate_opts
            .and_then(|o| o.get("maxWaitTimeInSeconds"))
            .and_then(serde_json::Value::as_u64);

        // Build image inputs
        let mut image_inputs = Map::new();
        if let Some(ref files) = options.files
            && !files.is_empty()
        {
            if self.is_flux2() {
                for (i, file) in files
                    .iter()
                    .enumerate()
                    .take(MAX_FLUX_2_INPUT_IMAGES as usize)
                {
                    let key = if i == 0 {
                        "input_image".to_string()
                    } else {
                        format!("input_image_{}", i + 1)
                    };
                    image_inputs.insert(
                        key,
                        json!(aimux_provider_utils::convert_image_model_file_to_data_uri(
                            file
                        )),
                    );
                }
                if files.len() > MAX_FLUX_2_INPUT_IMAGES as usize {
                    warnings.push(Warning::Other { message: format!("Flux-2 models support up to {MAX_FLUX_2_INPUT_IMAGES} input images. Additional images are ignored.") });
                }
            } else {
                image_inputs.insert(
                    "image".into(),
                    json!(aimux_provider_utils::convert_image_model_file_to_data_uri(
                        &files[0]
                    )),
                );
                if files.len() > 1 {
                    warnings.push(Warning::Other { message: "This Replicate model only supports a single input image. Additional images are ignored.".into() });
                }
            }
        }

        // Handle mask
        let mask_input = if let Some(ref mask) = options.mask {
            if self.is_flux2() {
                warnings.push(Warning::Other {
                    message: "Flux-2 models do not support mask input. The mask will be ignored."
                        .into(),
                });
                None
            } else {
                Some(aimux_provider_utils::convert_image_model_file_to_data_uri(
                    mask,
                ))
            }
        } else {
            None
        };

        // Build input object
        let mut input = Map::new();
        if let Some(ref p) = options.prompt {
            input.insert("prompt".into(), json!(p));
        }
        if let Some(ar) = options.aspect_ratio {
            input.insert("aspect_ratio".into(), json!(ar.to_string()));
        }
        if let Some(s) = options.size {
            input.insert("size".into(), json!(s.to_string()));
        }
        if let Some(seed) = options.seed {
            input.insert("seed".into(), json!(seed));
        }
        input.insert("num_outputs".into(), json!(options.n));
        for (k, v) in &image_inputs {
            input.insert(k.clone(), v.clone());
        }
        if let Some(m) = mask_input {
            input.insert("mask".into(), json!(m));
        }

        // Forward replicate provider options (excluding maxWaitTimeInSeconds and
        // the pacing keys, which are read here, never sent)
        if let Some(ro) = replicate_opts {
            for (k, v) in ro {
                if k != "maxWaitTimeInSeconds" && !is_poll_control_key(k) {
                    input.insert(k.clone(), v.clone());
                }
            }
        }

        let mut body = Map::new();
        body.insert("input".into(), Value::Object(input));
        if !version.is_empty() {
            body.insert("version".into(), json!(version));
        }

        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let prefer = if let Some(mw) = max_wait {
            format!("wait={mw}")
        } else {
            "wait".to_string()
        };
        let mut request = exchange.request(exchange.url(&self.endpoint_path()), options);
        request
            .headers
            .retain(|(name, _)| !name.eq_ignore_ascii_case("prefer"));
        request.headers.push(("prefer".to_string(), prefer));
        // This exchange legitimately holds the connection for the whole wait
        // window; widen the hang guard past it so a slow-but-alive generation
        // is not misread as a dead transport (retryable -> re-create -> double
        // billing).
        request.response_timeout = Some(Duration::from_secs(
            max_wait.unwrap_or(DEFAULT_WAIT_SECONDS) + WAIT_TRANSPORT_MARGIN_SECONDS,
        ));
        let request = request.generation(options.timeout.as_ref());

        let resp = aimux_provider_utils::post_json_to_api(
            request,
            Value::Object(body),
            aimux_provider_utils::create_json_response_handler(),
            replicate_failed_response_handler(),
        )
        .await?;

        let rh = resp.response_headers;
        let rb: Value = resp.value;

        // Extract output (string or array of strings)
        let urls: Vec<String> = match &rb["output"] {
            Value::Array(arr) => arr
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect(),
            Value::String(s) => vec![s.clone()],
            _ => Vec::new(),
        };

        // Download images; output URLs come from the prediction response
        // body, so they go through the SSRF download guard. A transient failure
        // repeats the download, never the prediction.
        let mut downloaded: Vec<Vec<u8>> = Vec::new();
        for url in &urls {
            let ir = retry_download(
                options.abort_signal.as_ref(),
                Duration::from_millis(poll_interval_ms(
                    replicate_opts,
                    POLL_INTERVAL_MS_KEY,
                    DOWNLOAD_RETRY_INTERVAL_MS,
                )),
                || {
                    aimux_provider_utils::get_from_api(
                        HttpRequest {
                            url: url.clone(),
                            abort_signal: options.abort_signal.clone(),
                            validate_url: true,
                            trusted_origin: Some(exchange.base_url().to_string()),
                            credentialed_origin: Some(exchange.base_url().to_string()),
                            ..Default::default()
                        },
                        aimux_provider_utils::create_binary_response_handler(),
                        replicate_failed_response_handler(),
                    )
                },
            )
            .await?;
            downloaded.push(ir.value.to_vec());
        }

        Ok(ImageResult {
            images: ImageOutputs::Binary(downloaded),
            warnings,
            provider_metadata: None,
            response: image_response(aimux_core::Timestamp::now(), &self.model_id, rh),
            usage: None,
        })
    }
}
