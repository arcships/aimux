//! Jina AI provider — rerank modality only.
//!
//! Implements the `RerankingModel` trait against the Jina AI rerank API
//! (`POST https://api.jina.ai/v1/rerank`).
//!
//! Jina AI is a modality-specific provider: it exposes a native rerank
//! protocol (query + documents → relevance scores) and does not support
//! language models.

use std::collections::HashMap;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;
use aimux_core::reranking_model::{
    RerankingCallOptions, RerankingDocuments, RerankingModel, RerankingRank, RerankingResponse,
    RerankingResult,
};
use aimux_core::types::Warning;
use aimux_provider_utils::response::ErrorStructure;
use aimux_provider_utils::{
    HttpBody, HttpMethod, HttpRequest, RetryConfig, load_api_key, send, without_trailing_slash,
};

/// Jina AI error response structure: `{ "detail": "...", "code": "..." }`.
///
/// `detail` carries the human-readable message; `code` carries the
/// machine-readable error code (e.g. `AUTH_INVALID_API_KEY`). Both the
/// `ErrorResponse` (`detail` + optional `code`) and the FastAPI
/// `HTTPValidationError` (`detail`) shapes are covered, since both surface
/// the message under `detail`.
const JINA_ERROR_STRUCTURE: ErrorStructure = ErrorStructure {
    message_path: &["detail"],
    type_path: &["code"],
};

/// Configuration for the Jina AI provider.
#[derive(Debug, Clone)]
pub struct JinaAiConfig {
    pub api_key: String,
    pub base_url: String,
    pub headers: Option<HashMap<String, String>>,
}

impl JinaAiConfig {
    /// Create from an API key (uses the default Jina AI base URL).
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://api.jina.ai".to_string(),
            headers: None,
        }
    }

    /// Use a custom base URL.
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = without_trailing_slash(&url.into());
        self
    }

    /// Attach extra default headers sent with every request.
    pub fn with_headers(mut self, headers: HashMap<String, String>) -> Self {
        self.headers = Some(headers);
        self
    }

    /// Create from the `JINA_AI_API_KEY` environment variable.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let api_key = load_api_key(None, "JINA_AI_API_KEY", "Jina AI")?;
        Ok(Self::new(api_key))
    }
}

/// Jina AI provider — creates `JinaAiRerankingModel` instances.
///
/// Jina AI is a rerank-only provider; it does not support language models.
pub struct JinaAiProvider {
    config: JinaAiConfig,
}

impl JinaAiProvider {
    pub fn new(config: JinaAiConfig) -> Self {
        Self { config }
    }

    /// Create a reranking model instance for the given model name (e.g.
    /// `"jina-reranker-v2-base-multilingual"` or `"jina-reranker-v3"`).
    pub fn reranking_model(&self, model_id: &str) -> JinaAiRerankingModel {
        JinaAiRerankingModel::new(model_id.to_string(), self.config.clone())
    }
}

impl Provider for JinaAiProvider {
    fn name(&self) -> &str {
        "jina_ai"
    }

    fn language_model(&self, _model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Err(AiMuxError::Unsupported(
            "jina_ai does not support language models. Use reranking_model() instead.".to_string(),
        ))
    }
}

/// Jina AI provider-specific reranking options.
#[derive(Debug, Clone, Default)]
struct JinaRerankingOptions {
    return_documents: Option<bool>,
}

fn parse_jina_reranking_options(
    provider_options: Option<&HashMap<String, Value>>,
) -> JinaRerankingOptions {
    let mut opts = JinaRerankingOptions::default();
    if let Some(po) = provider_options
        && let Some(jina) = po.get("jina")
        && let Some(v) = jina.get("returnDocuments").and_then(|v| v.as_bool())
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
            values.iter().map(|v| v.to_string()).collect()
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
    config: JinaAiConfig,
}

impl JinaAiRerankingModel {
    pub fn new(model_id: String, config: JinaAiConfig) -> Self {
        Self { model_id, config }
    }

    fn build_headers(&self, extra: Option<&HashMap<String, String>>) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        headers.insert(
            "Authorization".to_string(),
            format!("Bearer {}", self.config.api_key),
        );
        if let Some(ref ch) = self.config.headers {
            for (k, v) in ch {
                headers.insert(k.clone(), v.clone());
            }
        }
        if let Some(extra) = extra {
            for (k, v) in extra {
                headers.insert(k.clone(), v.clone());
            }
        }
        headers
    }

    fn endpoint(&self) -> String {
        format!("{}/v1/rerank", self.config.base_url)
    }
}

#[async_trait]
impl RerankingModel for JinaAiRerankingModel {
    fn provider(&self) -> &str {
        "jina_ai"
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

        let headers: Vec<(String, String)> = self
            .build_headers(options.headers.as_ref())
            .into_iter()
            .collect();

        let resp = send(
            HttpRequest {
                method: HttpMethod::Post,
                url: self.endpoint(),
                headers,
                body: HttpBody::Json(body),

                abort_signal: options.abort_signal.clone(),
            },
            RetryConfig::default(),
            &JINA_ERROR_STRUCTURE,
        )
        .await?;

        // Capture response headers.
        let response_headers = resp.headers;

        let raw_body: Value =
            serde_json::from_slice(&resp.body).map_err(|e| AiMuxError::Json(e.to_string()))?;

        let data: JinaRerankingResponse =
            serde_json::from_value(raw_body.clone()).map_err(|e| {
                AiMuxError::Provider(format!("failed to parse reranking response: {e}"))
            })?;

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
