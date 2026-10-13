//! Shared transport and inline image handling for native decision adapters.
use std::collections::HashMap;

use aimux_core::AiMuxError;
use aimux_core::decision_model::{DecisionCallOptions, DecisionImage};
use aimux_core::shared::{FileBytes, SharedProviderMetadata};
use aimux_provider_utils::HttpRequest;
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
    /// Resolve the full endpoint through the native model's URL callback.
    pub(crate) fn url(&self, path: &str) -> Result<String, AiMuxError> {
        match self {
            Self::OpenAI(config) => config.url(path),
            Self::Compatible(config) => Ok(format!(
                "{}{}",
                config.base_url.resolve()?.trim_end_matches('/'),
                path
            )),
        }
    }

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
                config.with_transport(
                    HttpRequest::new(url, headers.list, options),
                    origin,
                    headers.fetch_user_agent,
                )
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
    // The image's bytes are a base64 string in the JSON and read as the AI
    // SDK reads base64 (`convertBase64ToUint8Array`); a string is sent on
    // verbatim, as `convertToBase64` passes a string through.
    let (base64, decoded_len) = match &image.data {
        FileBytes::Binary(bytes) => (aimux_core::wire::encode_base64(bytes), bytes.len()),
        FileBytes::Base64(data) => {
            let bytes = aimux_core::wire::decode_base64(data)
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
    Ok(SharedProviderMetadata::from([(provider.into(), object)]))
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use aimux_provider_utils::Resolvable;

    use super::*;

    #[test]
    fn native_decision_url_passes_endpoint_to_url_callback() {
        let mut config = OpenAIModelConfig::fixed(
            "openai".into(),
            "https://example.com/v1".into(),
            Resolvable::Value(HashMap::new()),
            None,
            None,
        );
        config.url = Arc::new(|path| {
            assert_eq!(path, "/decisions");
            Ok("https://example.com/decisions?version=1".into())
        });
        assert_eq!(
            DecisionConfig::OpenAI(config).url("/decisions").unwrap(),
            "https://example.com/decisions?version=1"
        );
    }

    /// Decision image bytes are the AI SDK's base64 JSON: binary is encoded as
    /// `btoa` does, a string is decoded as `atob` reads it (`provider-utils`
    /// `uint8-utils.ts:6-23`) and sent on verbatim.
    #[test]
    fn inline_image_reads_bytes_as_the_ai_sdk_does() {
        let image = |data| DecisionImage {
            data,
            media_type: "image/png".into(),
            detail: None,
        };
        let binary = inline_image(&image(FileBytes::Binary(vec![0, 1, 2, 250, 255]))).unwrap();
        assert_eq!(binary.base64, "AAEC+v8=");
        assert_eq!(binary.data_url, "data:image/png;base64,AAEC+v8=");
        let unpadded = inline_image(&image(FileBytes::Base64("AAEC+v8".into()))).unwrap();
        assert_eq!(
            (unpadded.base64.as_str(), unpadded.decoded_len),
            ("AAEC+v8", 5)
        );
        assert!(inline_image(&image(FileBytes::Base64("not base64!".into()))).is_err());
    }
}
