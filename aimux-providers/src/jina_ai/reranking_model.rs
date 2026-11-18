use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::reranking_model::{
    RerankingCallOptions, RerankingDocuments, RerankingModel, RerankingRank, RerankingResponse,
    RerankingResult,
};
use aimux_core::shared::SharedProviderOptions;
use aimux_core::types::Warning;

use crate::shared::{EndpointConfig, namespaced_options};

/// The providerOptions key, read whatever the provider is named.
pub(crate) const NAMESPACE: &str = "jina";

/// Jina AI error response structure: `{ "detail": "...", "code": "..." }`.
///
/// `detail` carries the human-readable message; `code` carries the
/// machine-readable error code (e.g. `AUTH_INVALID_API_KEY`). Both the
/// `ErrorResponse` (`detail` + optional `code`) and the FastAPI
/// `HTTPValidationError` (`detail`) shapes are covered, since both surface
/// the message under `detail`.
fn jina_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    aimux_provider_utils::create_json_error_response_handler(|data| {
        aimux_provider_utils::ProviderErrorParts {
            message: data
                .get("detail")
                .and_then(Value::as_str)
                .unwrap_or("Jina AI request failed")
                .to_string(),
            provider_code: data.get("code").and_then(Value::as_str).map(str::to_string),
        }
    })
}

/// Jina AI provider-specific reranking options.
#[derive(Debug, Clone, Default)]
struct JinaRerankingOptions {
    return_documents: Option<bool>,
}

fn parse_jina_reranking_options(
    provider_options: Option<&SharedProviderOptions>,
) -> JinaRerankingOptions {
    let mut opts = JinaRerankingOptions::default();
    if let Some(jina) = namespaced_options(provider_options, NAMESPACE)
        && let Some(v) = jina
            .get("returnDocuments")
            .and_then(serde_json::Value::as_bool)
    {
        opts.return_documents = Some(v);
    }
    opts
}

/// Convert call options into the Jina AI rerank request body (pure function).
///
/// Object documents are stringified (with a compatibility warning) since the
/// Jina rerank endpoint accepts a list of strings.
fn build_request_body(
    model_id: &str,
    options: &RerankingCallOptions,
    jina_options: &JinaRerankingOptions,
    warnings: &mut Vec<Warning>,
) -> Value {
    let documents: Vec<String> = match &options.documents {
        RerankingDocuments::Text { values } => values.clone(),
        RerankingDocuments::Object { values } => {
            warnings.push(Warning::Compatibility {
                feature: "object documents".to_string(),
                details: Some("Object documents are converted to strings.".to_string()),
            });
            values
                .iter()
                .map(std::string::ToString::to_string)
                .collect()
        }
    };

    let mut body = json!({
        "model": model_id,
        "query": options.query,
        "documents": documents,
        "top_n": options.top_n,
    });

    if let Some(return_documents) = jina_options.return_documents {
        body["return_documents"] = json!(return_documents);
    }

    body
}

/// The response from the Jina AI `/v1/rerank` endpoint.
///
/// Only the fields used by the trait are deserialized; extra fields returned
/// by the API (e.g. `object`, per-result `document`/`embedding`) are ignored
/// so unknown-but-legal values degrade safely.
#[derive(Debug, Deserialize)]
struct JinaRerankingResponse {
    #[serde(default)]
    model: Option<String>,
    results: Vec<JinaRerankingResult>,
}

#[derive(Debug, Deserialize)]
struct JinaRerankingResult {
    index: u32,
    relevance_score: f64,
}

/// A Jina AI reranking model.
pub struct JinaAiRerankingModel {
    model_id: String,
    config: EndpointConfig,
}

impl JinaAiRerankingModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }
}

#[async_trait]
impl RerankingModel for JinaAiRerankingModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_rerank(
        &self,
        options: &RerankingCallOptions,
    ) -> Result<RerankingResult, AiMuxError> {
        let jina_options = parse_jina_reranking_options(options.provider_options.as_ref());

        let mut warnings = Vec::new();
        let body = build_request_body(&self.model_id, options, &jina_options, &mut warnings);

        let exchange = self.config.exchange(options.headers.as_ref()).await?;

        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(exchange.url("/v1/rerank"), options),
            body,
            aimux_provider_utils::create_json_response_handler::<JinaRerankingResponse>(),
            jina_failed_response_handler(),
        )
        .await?;

        // Capture response headers.
        let response_headers = resp.response_headers;

        let raw_body = resp.raw_value.unwrap_or(Value::Null);
        let data = resp.value;

        let model = data.model;
        let ranking: Vec<RerankingRank> = data
            .results
            .into_iter()
            .map(|r| RerankingRank {
                index: r.index,
                relevance_score: r.relevance_score,
            })
            .collect();

        Ok(RerankingResult {
            ranking,
            provider_metadata: None,
            warnings: Some(warnings),
            response: Some(RerankingResponse {
                id: None,
                timestamp: None,
                model_id: model,
                headers: Some(response_headers),
                body: Some(raw_body),
            }),
        })
    }
}
