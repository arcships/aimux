//! Jina AI reranking provider tests.
//!
//! Each test spins up a `wiremock` mock server, configures a JSON response,
//! creates a `JinaAiRerankingModel` pointing at the mock, calls `do_rerank`,
//! and asserts on the request body / headers / result.
//!
//! Tests do not hit the public network and do not read real credentials.

use std::collections::HashMap;

use serde_json::{Value, json};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::Provider;
use aimux_core::error::AiMuxError;
use aimux_core::reranking_model::{RerankingCallOptions, RerankingDocuments, RerankingModel};
use aimux_providers::{JinaAiConfig, JinaAiProvider};

const API_KEY: &str = "test-jina-key";
const MODEL: &str = "jina-reranker-v2-base-multilingual";

// -- helpers -----------------------------------------------------------------

/// A realistic Jina `/v1/rerank` success response.
///
/// Results include `document` (an extra field the model ignores) to verify
/// that unknown-but-legal fields degrade safely without panicking.
fn rerank_response_body() -> Value {
    json!({
        "model": MODEL,
        "object": "list",
        "usage": {
            "total_tokens": 12,
            "prompt_tokens": 8,
            "completion_tokens": 0
        },
        "results": [
            {
                "index": 1,
                "relevance_score": 0.8783142566680908,
                "document": { "text": "rainy day in the city" }
            },
            {
                "index": 0,
                "relevance_score": 0.255859375,
                "document": { "text": "sunny day at the beach" }
            }
        ]
    })
}

fn provider(server: &MockServer) -> JinaAiProvider {
    let config = JinaAiConfig::new(API_KEY).with_base_url(server.uri());
    JinaAiProvider::new(config)
}

fn text_docs_opts(query: &str, top_n: u32) -> RerankingCallOptions {
    RerankingCallOptions {
        documents: RerankingDocuments::Text {
            values: vec![
                "sunny day at the beach".to_string(),
                "rainy day in the city".to_string(),
            ],
        },
        query: query.to_string(),
        top_n: Some(top_n),
        abort_signal: None,
        provider_options: None,
        headers: None,
    }
}

fn json_docs_opts(query: &str, top_n: u32) -> RerankingCallOptions {
    RerankingCallOptions {
        documents: RerankingDocuments::Object {
            values: vec![
                json!({ "example": "sunny day at the beach" }),
                json!({ "example": "rainy day in the city" }),
            ],
        },
        query: query.to_string(),
        top_n: Some(top_n),
        abort_signal: None,
        provider_options: None,
        headers: None,
    }
}

fn jina_provider_options(return_documents: bool) -> HashMap<String, Value> {
    let mut po = HashMap::new();
    po.insert(
        "jina".to_string(),
        json!({ "returnDocuments": return_documents }),
    );
    po
}

async fn mount_rerank_mock(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/v1/rerank"))
        .respond_with(ResponseTemplate::new(200).set_body_json(rerank_response_body()))
        .mount(server)
        .await;
}

// -- result mapping ----------------------------------------------------------

#[tokio::test]
async fn do_rerank_returns_correct_ranking() {
    let server = MockServer::start().await;
    mount_rerank_mock(&server).await;

    let provider = provider(&server);
    let model = provider.reranking_model(MODEL);

    let result = model
        .do_rerank(&text_docs_opts("rainy day", 2))
        .await
        .unwrap();

    assert_eq!(result.ranking.len(), 2);
    // index / relevance_score preserved from the response, in order.
    assert_eq!(result.ranking[0].index, 1);
    assert!((result.ranking[0].relevance_score - 0.8783142566680908).abs() < 1e-9);
    assert_eq!(result.ranking[1].index, 0);
    assert!((result.ranking[1].relevance_score - 0.255859375).abs() < 1e-9);
}

#[tokio::test]
async fn do_rerank_exposes_response_body_and_model() {
    let server = MockServer::start().await;
    mount_rerank_mock(&server).await;

    let provider = provider(&server);
    let model = provider.reranking_model(MODEL);

    let result = model
        .do_rerank(&text_docs_opts("rainy day", 2))
        .await
        .unwrap();

    let response = result.response.expect("response");
    assert_eq!(response.model_id.as_deref(), Some(MODEL));
    let body = response.body.expect("body");
    assert_eq!(body["model"], MODEL);
    assert_eq!(body["results"][0]["index"], 1);
    assert_eq!(body["results"][0]["relevance_score"], 0.8783142566680908);
    assert_eq!(body["usage"]["total_tokens"], 12);
}

// -- URL + auth header -------------------------------------------------------

#[tokio::test]
async fn do_rerank_hits_correct_url_with_auth_header() {
    let server = MockServer::start().await;
    // The mock only matches POST /v1/rerank with the correct bearer header,
    // so a wrong URL or missing auth would fail to match.
    Mock::given(method("POST"))
        .and(path("/v1/rerank"))
        .and(header("authorization", format!("Bearer {API_KEY}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(rerank_response_body()))
        .mount(&server)
        .await;

    let provider = provider(&server);
    let model = provider.reranking_model(MODEL);

    model
        .do_rerank(&text_docs_opts("rainy day", 2))
        .await
        .unwrap();

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    // URL path correctness.
    assert_eq!(
        requests[0].url.path(),
        "/v1/rerank",
        "expected path /v1/rerank, got {}",
        requests[0].url
    );
    let headers = &requests[0].headers;
    assert_eq!(
        headers.get("authorization").and_then(|v| v.to_str().ok()),
        Some("Bearer test-jina-key")
    );
    assert_eq!(
        headers.get("content-type").and_then(|v| v.to_str().ok()),
        Some("application/json")
    );
}

// -- request body ------------------------------------------------------------

#[tokio::test]
async fn do_rerank_sends_correct_request_body() {
    let server = MockServer::start().await;
    mount_rerank_mock(&server).await;

    let provider = provider(&server);
    let model = provider.reranking_model(MODEL);

    model
        .do_rerank(&text_docs_opts("rainy day", 2))
        .await
        .unwrap();

    let requests = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["model"], MODEL);
    assert_eq!(body["query"], "rainy day");
    assert_eq!(body["documents"][0], "sunny day at the beach");
    assert_eq!(body["documents"][1], "rainy day in the city");
    assert_eq!(body["top_n"], 2);
    // return_documents is only sent when explicitly configured.
    assert!(body.get("return_documents").is_none());
}

#[tokio::test]
async fn do_rerank_forwards_return_documents_provider_option() {
    let server = MockServer::start().await;
    mount_rerank_mock(&server).await;

    let provider = provider(&server);
    let model = provider.reranking_model(MODEL);

    let mut opts = text_docs_opts("rainy day", 2);
    opts.provider_options = Some(jina_provider_options(false));
    model.do_rerank(&opts).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["return_documents"], false);
}

#[tokio::test]
async fn object_documents_are_stringified_with_warning() {
    let server = MockServer::start().await;
    mount_rerank_mock(&server).await;

    let provider = provider(&server);
    let model = provider.reranking_model(MODEL);

    let result = model
        .do_rerank(&json_docs_opts("rainy day", 2))
        .await
        .unwrap();

    // Documents sent as stringified JSON.
    let requests = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(
        body["documents"][0],
        json!("{\"example\":\"sunny day at the beach\"}")
    );

    // A compatibility warning is emitted.
    let warnings = result.warnings.expect("warnings");
    assert_eq!(warnings.len(), 1);
    match &warnings[0] {
        aimux_core::types::Warning::Compatibility { feature, details } => {
            assert_eq!(feature, "object documents");
            assert_eq!(
                details.as_deref(),
                Some("Object documents are converted to strings.")
            );
        }
        other => panic!("expected Compatibility warning, got {other:?}"),
    }
}

// -- error mapping -----------------------------------------------------------

#[tokio::test]
async fn status_401_maps_to_auth_error() {
    let server = MockServer::start().await;
    // Jina vendor-specific error shape: { detail, code }.
    Mock::given(method("POST"))
        .and(path("/v1/rerank"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "detail": "Invalid API key. Verify your API key at https://jina.ai/api-dashboard/key-manager or generate a new one.",
            "code": "AUTH_INVALID_API_KEY"
        })))
        .mount(&server)
        .await;

    let provider = provider(&server);
    let model = provider.reranking_model(MODEL);

    let result = model.do_rerank(&text_docs_opts("rainy day", 2)).await;
    assert!(
        matches!(
            result,
            Err(AiMuxError::Auth(ref m)) if m ==
                "Invalid API key. Verify your API key at https://jina.ai/api-dashboard/key-manager or generate a new one."
        ),
        "expected Auth error, got {result:?}"
    );
}

// -- Provider trait ----------------------------------------------------------

#[tokio::test]
async fn provider_name_is_jina_ai() {
    let config = JinaAiConfig::new(API_KEY);
    let provider = JinaAiProvider::new(config);
    assert_eq!(provider.name(), "jina_ai");
}

#[test]
fn language_model_returns_unsupported_error() {
    let config = JinaAiConfig::new(API_KEY);
    let provider = JinaAiProvider::new(config);
    match provider.language_model(MODEL) {
        Err(AiMuxError::Unsupported(msg)) => {
            assert!(
                msg.contains("provider 'jina_ai' does not provide language models"),
                "unexpected message: {msg}"
            );
        }
        _ => panic!("expected Unsupported error, got success or another error variant"),
    }
}
