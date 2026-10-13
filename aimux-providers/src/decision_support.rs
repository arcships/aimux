//! Shared transport, provider options and metadata of the native decision
//! adapters (aimux extensions; the AI SDK's own decision models wrap a
//! language model, see `aimux_provider_utils::DecisionLanguageModel`).
use std::collections::HashMap;

use aimux_core::AiMuxError;
use aimux_core::decision_model::{DecisionCallOptions, DecisionQuestion};
use aimux_core::shared::{ResponseInfo, SharedProviderMetadata};
use aimux_provider_utils::HttpRequest;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// How a native decision provider produced its probabilities. This is
/// provenance, not a calibration guarantee (RFC-0037). Reported as
/// `providerMetadata.<provider>.probabilitySource`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionProbabilitySource {
    Native,
    LogitScoring,
    ModelEstimate,
}

impl DecisionProbabilitySource {
    /// The wire name (`"native"`, `"logit_scoring"`, `"model_estimate"`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::LogitScoring => "logit_scoring",
            Self::ModelEstimate => "model_estimate",
        }
    }
}

impl std::str::FromStr for DecisionProbabilitySource {
    type Err = AiMuxError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "native" => Ok(Self::Native),
            "logit_scoring" => Ok(Self::LogitScoring),
            "model_estimate" => Ok(Self::ModelEstimate),
            _ => Err(AiMuxError::InvalidArgument(format!(
                "unknown decision probability source: {value:?}"
            ))),
        }
    }
}

/// An inline image shared by all questions, placed before the textual
/// state: an entry of `providerOptions.<provider>.images`,
/// `{data, mediaType, detail?}` as the AI SDK's image parts name the media
/// type.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DecisionImage {
    /// Base64 image bytes.
    pub data: String,
    pub media_type: String,
    /// Provider image detail, for example OpenAI's `low`, `high`, `auto`,
    /// `original`.
    #[serde(default)]
    pub detail: Option<String>,
}

/// Take `images` out of a provider's options.
pub(crate) fn take_images(
    options: &mut Map<String, Value>,
) -> Result<Vec<DecisionImage>, AiMuxError> {
    match options.remove("images") {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(images) => serde_json::from_value(images)
            .map_err(|e| AiMuxError::InvalidArgument(format!("decision images: {e}"))),
    }
}

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
    // The image's bytes are a base64 string, read as the AI SDK reads
    // base64 (`convertBase64ToUint8Array`) and sent on verbatim.
    let decoded_len = aimux_core::wire::decode_base64(&image.data)
        .map_err(|e| AiMuxError::InvalidArgument(format!("decision image base64: {e}")))?
        .len();
    let base64 = image.data.clone();
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

/// What a native provider reports beyond the V4 result, under its provider
/// key: the probability source, and per answer what the wire adds (such as
/// `confidence`). The raw response stays in `response.body`.
pub(crate) fn provider_metadata(
    provider: &str,
    probability_source: DecisionProbabilitySource,
    answers: Map<String, Value>,
) -> SharedProviderMetadata {
    let mut metadata = Map::new();
    metadata.insert(
        "probabilitySource".into(),
        json!(probability_source.as_str()),
    );
    if !answers.is_empty() {
        metadata.insert("answers".into(), Value::Object(answers));
    }
    SharedProviderMetadata::from([(provider.into(), metadata)])
}

/// The V4 response information of a native exchange: the model the server
/// reports, its headers and its body.
pub(crate) fn response_info(
    model: Option<&str>,
    headers: HashMap<String, String>,
    raw: Value,
) -> ResponseInfo {
    ResponseInfo {
        model_id: model.filter(|m| !m.trim().is_empty()).map(str::to_owned),
        headers: Some(headers),
        body: Some(raw),
        ..Default::default()
    }
}

/// The number of candidates a question offers: its options, its levels, or
/// the two Boolean outcomes.
pub(crate) fn candidate_count(question: &DecisionQuestion) -> usize {
    match question {
        DecisionQuestion::Choice { criteria, .. } => criteria.len(),
        DecisionQuestion::Score { criteria, .. } => criteria.len(),
        DecisionQuestion::Boolean { .. } => 2,
    }
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

    /// Decision image bytes are a base64 string, read as `atob` reads it
    /// (`provider-utils` `uint8-utils.ts:6-23`) and sent on verbatim.
    #[test]
    fn inline_image_reads_base64_as_the_ai_sdk_does() {
        let image = |data: &str| DecisionImage {
            data: data.into(),
            media_type: "image/png".into(),
            detail: None,
        };
        let padded = inline_image(&image("AAEC+v8=")).unwrap();
        assert_eq!(padded.base64, "AAEC+v8=");
        assert_eq!(padded.data_url, "data:image/png;base64,AAEC+v8=");
        let unpadded = inline_image(&image("AAEC+v8")).unwrap();
        assert_eq!(
            (unpadded.base64.as_str(), unpadded.decoded_len),
            ("AAEC+v8", 5)
        );
        assert!(inline_image(&image("not base64!")).is_err());
    }
}
