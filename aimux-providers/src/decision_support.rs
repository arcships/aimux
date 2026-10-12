//! Shared transport and inline image handling for native decision adapters.
use std::collections::HashMap;

use aimux_core::AiMuxError;
use aimux_core::decision_model::{DecisionCallOptions, DecisionImage};
use aimux_core::shared::{FileBytes, SharedProviderMetadata};
use aimux_provider_utils::HttpRequest;
use base64::Engine;
use serde_json::Value;

use crate::openai::config::OpenAIModelConfig;
use crate::openai_compatible::config::CompatModelConfig;

/// The connection a decision adapter borrows from the provider that made it:
/// native OpenAI, or an OpenAI-compatible preset (a local runtime,
/// Cloudflare). Headers are resolved per request, like every other model of
/// that provider.
#[derive(Clone)]
pub(crate) enum DecisionConfig {
    OpenAI(OpenAIModelConfig),
    Compatible(CompatModelConfig),
}

impl DecisionConfig {
    /// The provider's base URL; adapters append their own endpoint paths.
    pub(crate) fn base_url(&self) -> Result<String, AiMuxError> {
        let base = match self {
            Self::OpenAI(config) => config.url("")?,
            Self::Compatible(config) => config.base_url.resolve()?,
        };
        Ok(base.trim_end_matches('/').to_string())
    }

    async fn request(
        &self,
        url: &str,
        options: &DecisionCallOptions,
    ) -> Result<HttpRequest, AiMuxError> {
        Ok(match self {
            Self::OpenAI(config) => {
                let headers = config.request_headers(options.headers.as_ref()).await?;
                config.http_request(url.to_string(), headers, options)
            }
            Self::Compatible(config) => {
                let headers = config.request_headers(options.headers.as_ref()).await?;
                let origin = config.base_url.resolve()?;
                config.with_transport(HttpRequest::new(url, headers, options), origin)
            }
        })
    }
}

pub(crate) struct InlineImage {
    pub data_url: String,
    pub base64: String,
    pub decoded_len: usize,
}

pub(crate) fn inline_image(image: &DecisionImage) -> Result<InlineImage, AiMuxError> {
    if !image.media_type.starts_with("image/") || image.media_type.contains([';', ',']) {
        return Err(AiMuxError::InvalidArgument(
            "decision image requires an image MIME type".into(),
        ));
    }
    let engine = base64::engine::general_purpose::STANDARD;
    let (base64, decoded_len) = match &image.data {
        FileBytes::Binary(bytes) => (engine.encode(bytes), bytes.len()),
        FileBytes::Base64(data) => {
            let bytes = engine
                .decode(data)
                .map_err(|e| AiMuxError::InvalidArgument(format!("decision image base64: {e}")))?;
            (data.clone(), bytes.len())
        }
    };
    if decoded_len == 0 {
        return Err(AiMuxError::InvalidArgument(
            "decision image is empty".into(),
        ));
    }
    Ok(InlineImage {
        data_url: format!("data:{};base64,{base64}", image.media_type),
        base64,
        decoded_len,
    })
}

pub(crate) async fn post<T>(
    config: &DecisionConfig,
    options: &DecisionCallOptions,
    url: &str,
    body: Value,
    convert: impl FnOnce(Value, HashMap<String, String>) -> Result<T, AiMuxError>,
) -> Result<T, AiMuxError> {
    let response = aimux_provider_utils::post_json_to_api(
        config.request(url, options).await?,
        body.clone(),
        aimux_provider_utils::create_json_response_handler::<Value>(),
        aimux_provider_utils::create_json_error_response_handler(|data| {
            let error = data
                .get("error")
                .or_else(|| data.get("detail"))
                .or_else(|| {
                    data.get("errors")
                        .and_then(Value::as_array)
                        .and_then(|v| v.first())
                })
                .unwrap_or(data);
            aimux_provider_utils::ProviderErrorParts {
                message: error
                    .as_str()
                    .or_else(|| error.get("message").and_then(Value::as_str))
                    .unwrap_or_default()
                    .into(),
                provider_code: error
                    .get("code")
                    .or_else(|| error.get("type"))
                    .and_then(|code| match code {
                        Value::String(s) => Some(s.clone()),
                        Value::Number(n) => Some(n.to_string()),
                        _ => None,
                    }),
            }
        }),
    )
    .await?;
    let raw = response.value;
    let headers = response.response_headers;
    convert(raw.clone(), headers.clone()).map_err(|error| {
        aimux_provider_utils::invalid_response_api_call(
            error.to_string(),
            200,
            url,
            body,
            raw,
            headers,
        )
    })
}

pub(crate) fn invalid(message: impl Into<String>) -> AiMuxError {
    AiMuxError::InvalidResponseData(message.into())
}

/// Provider metadata keeps the whole response object under the provider key.
pub(crate) fn response_metadata(
    provider: &str,
    raw: &Value,
) -> Result<SharedProviderMetadata, AiMuxError> {
    let object = raw.as_object().cloned().ok_or_else(|| {
        AiMuxError::InvalidResponseData(format!("{provider} response must be an object"))
    })?;
    Ok(HashMap::from([(provider.into(), object)]))
}

pub(crate) fn options(
    request: &DecisionCallOptions,
    provider: &str,
) -> Result<serde_json::Map<String, Value>, AiMuxError> {
    match request
        .provider_options
        .as_ref()
        .and_then(|options| options.get(provider))
    {
        None => Ok(serde_json::Map::new()),
        Some(options) => Ok(options.clone()),
    }
}
