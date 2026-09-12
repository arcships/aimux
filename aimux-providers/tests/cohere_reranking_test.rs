//! Rust translations of the Cohere Reranking model tests.
//!
//! Source: `reference/ai/packages/cohere/src/reranking/cohere-reranking-model.test.ts`
//! (12 test cases across "json documents" and "text documents" groups).

use std::collections::HashMap;

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::reranking_model::{RerankingCallOptions, RerankingDocuments, RerankingModel};
use aimux_providers::{CohereConfig, CohereProvider};

// -- helpers -----------------------------------------------------------------

fn rerank_response_body() -> Value {
    json!({
        "id": "b44fe75b-e3d3-489a-b61e-1a1aede3ef72",
        "meta": {
            "api_version": { "version": "2" },
            "billed_units": { "search_units": 1 }
        },
        "results": [
            { "index": 1, "relevance_score": 0.10183054 },
            { "index": 0, "relevance_score": 0.03762639 }
        ]
    })
}

fn provider(server: &MockServer) -> CohereProvider {
    let config = CohereConfig::new("test-api-key").with_base_url(server.uri());
    CohereProvider::new(config)
}

fn cohere_provider_options() -> HashMap<String, Value> {
    let mut po = HashMap::new();
    po.insert(
        "cohere".to_string(),
        json!({ "maxTokensPerDoc": 1000, "priority": 1 }),
    );
    po
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
        provider_options: Some(cohere_provider_options()),
        headers: None,
        max_retries: None,
        timeout: None,
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
        provider_options: Some(cohere_provider_options()),
        headers: None,
        max_retries: None,
        timeout: None,
    }
}

async fn mount_rerank_mock(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/rerank"))
        .respond_with(ResponseTemplate::new(200).set_body_json(rerank_response_body()))
        .mount(server)
        .await;
}

// -- json documents tests ----------------------------------------------------

#[tokio::test]
async fn json_docs_should_send_request_with_stringified_json_documents() {
    let server = MockServer::start().await;
    mount_rerank_mock(&server).await;

    let provider = provider(&server);
    let model = provider.reranking_model("rerank-english-v3.0");

    model
        .do_rerank(&json_docs_opts("rainy day", 2))
        .await
        .unwrap();

    let requests = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(
        body["documents"][0],
        json!("{\"example\":\"sunny day at the beach\"}")
    );
    assert_eq!(
        body["documents"][1],
        json!("{\"example\":\"rainy day in the city\"}")
    );
    assert_eq!(body["max_tokens_per_doc"], 1000);
    assert_eq!(body["model"], "rerank-english-v3.0");
    assert_eq!(body["priority"], 1);
    assert_eq!(body["query"], "rainy day");
    assert_eq!(body["top_n"], 2);
}

#[tokio::test]
async fn json_docs_should_send_request_with_correct_headers() {
    let server = MockServer::start().await;
    mount_rerank_mock(&server).await;

    let provider = provider(&server);
    let model = provider.reranking_model("rerank-english-v3.0");

    model
        .do_rerank(&json_docs_opts("rainy day", 2))
        .await
        .unwrap();

    let requests = server.received_requests().await.unwrap();
    let headers = &requests[0].headers;
    assert_eq!(
        headers.get("authorization").and_then(|v| v.to_str().ok()),
        Some("Bearer test-api-key")
    );
    assert_eq!(
        headers.get("content-type").and_then(|v| v.to_str().ok()),
        Some("application/json")
    );
}

#[tokio::test]
async fn json_docs_should_return_result_with_warnings() {
    let server = MockServer::start().await;
    mount_rerank_mock(&server).await;

    let provider = provider(&server);
    let model = provider.reranking_model("rerank-english-v3.0");

    let result = model
        .do_rerank(&json_docs_opts("rainy day", 2))
        .await
        .unwrap();

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

#[tokio::test]
async fn json_docs_should_return_result_with_correct_ranking() {
    let server = MockServer::start().await;
    mount_rerank_mock(&server).await;

    let provider = provider(&server);
    let model = provider.reranking_model("rerank-english-v3.0");

    let result = model
        .do_rerank(&json_docs_opts("rainy day", 2))
        .await
        .unwrap();

    assert_eq!(result.ranking.len(), 2);
    assert_eq!(result.ranking[0].index, 1);
    assert!((result.ranking[0].relevance_score - 0.10183054).abs() < 1e-8);
    assert_eq!(result.ranking[1].index, 0);
    assert!((result.ranking[1].relevance_score - 0.03762639).abs() < 1e-8);
}

#[tokio::test]
async fn json_docs_should_not_return_provider_metadata() {
    let server = MockServer::start().await;
    mount_rerank_mock(&server).await;

    let provider = provider(&server);
    let model = provider.reranking_model("rerank-english-v3.0");

    let result = model
        .do_rerank(&json_docs_opts("rainy day", 2))
        .await
        .unwrap();

    assert!(result.provider_metadata.is_none());
}

#[tokio::test]
async fn json_docs_should_return_result_with_correct_response() {
    let server = MockServer::start().await;
    mount_rerank_mock(&server).await;

    let provider = provider(&server);
    let model = provider.reranking_model("rerank-english-v3.0");

    let result = model
        .do_rerank(&json_docs_opts("rainy day", 2))
        .await
        .unwrap();

    let response = result.response.expect("response");
    assert_eq!(
        response.id.as_deref(),
        Some("b44fe75b-e3d3-489a-b61e-1a1aede3ef72")
    );
    let body = response.body.expect("body");
    assert_eq!(body["results"][0]["index"], 1);
    assert_eq!(body["results"][0]["relevance_score"], 0.10183054);
}

// -- text documents tests ----------------------------------------------------

#[tokio::test]
async fn text_docs_should_send_request_with_text_documents() {
    let server = MockServer::start().await;
    mount_rerank_mock(&server).await;

    let provider = provider(&server);
    let model = provider.reranking_model("rerank-english-v3.0");

    model
        .do_rerank(&text_docs_opts("rainy day", 2))
        .await
        .unwrap();

    let requests = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["documents"][0], "sunny day at the beach");
    assert_eq!(body["documents"][1], "rainy day in the city");
    assert_eq!(body["model"], "rerank-english-v3.0");
    assert_eq!(body["query"], "rainy day");
    assert_eq!(body["top_n"], 2);
}

#[tokio::test]
async fn text_docs_should_send_request_with_correct_headers() {
    let server = MockServer::start().await;
    mount_rerank_mock(&server).await;

    let provider = provider(&server);
    let model = provider.reranking_model("rerank-english-v3.0");

    model
        .do_rerank(&text_docs_opts("rainy day", 2))
        .await
        .unwrap();

    let requests = server.received_requests().await.unwrap();
    let headers = &requests[0].headers;
    assert_eq!(
        headers.get("authorization").and_then(|v| v.to_str().ok()),
        Some("Bearer test-api-key")
    );
    assert_eq!(
        headers.get("content-type").and_then(|v| v.to_str().ok()),
        Some("application/json")
    );
}

#[tokio::test]
async fn text_docs_should_return_result_without_warnings() {
    let server = MockServer::start().await;
    mount_rerank_mock(&server).await;

    let provider = provider(&server);
    let model = provider.reranking_model("rerank-english-v3.0");

    let result = model
        .do_rerank(&text_docs_opts("rainy day", 2))
        .await
        .unwrap();

    let warnings = result.warnings.expect("warnings");
    assert!(warnings.is_empty());
}

#[tokio::test]
async fn text_docs_should_return_result_with_correct_ranking() {
    let server = MockServer::start().await;
    mount_rerank_mock(&server).await;

    let provider = provider(&server);
    let model = provider.reranking_model("rerank-english-v3.0");

    let result = model
        .do_rerank(&text_docs_opts("rainy day", 2))
        .await
        .unwrap();

    assert_eq!(result.ranking.len(), 2);
    assert_eq!(result.ranking[0].index, 1);
    assert!((result.ranking[0].relevance_score - 0.10183054).abs() < 1e-8);
}

#[tokio::test]
async fn text_docs_should_not_return_provider_metadata() {
    let server = MockServer::start().await;
    mount_rerank_mock(&server).await;

    let provider = provider(&server);
    let model = provider.reranking_model("rerank-english-v3.0");

    let result = model
        .do_rerank(&text_docs_opts("rainy day", 2))
        .await
        .unwrap();

    assert!(result.provider_metadata.is_none());
}

#[tokio::test]
async fn text_docs_should_return_result_with_correct_response() {
    let server = MockServer::start().await;
    mount_rerank_mock(&server).await;

    let provider = provider(&server);
    let model = provider.reranking_model("rerank-english-v3.0");

    let result = model
        .do_rerank(&text_docs_opts("rainy day", 2))
        .await
        .unwrap();

    let response = result.response.expect("response");
    assert_eq!(
        response.id.as_deref(),
        Some("b44fe75b-e3d3-489a-b61e-1a1aede3ef72")
    );
}
