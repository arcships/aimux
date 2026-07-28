//! Rust translations of the Voyage Reranking model tests.
//!
//! Source: `reference/ai/packages/voyage/src/reranking/voyage-reranking-model.test.ts`
//! (10 test cases across "object documents" and "text documents" groups).

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::reranking_model::{RerankingCallOptions, RerankingDocuments, RerankingModel};
use aimux_providers::{VoyageConfig, VoyageProvider};

// -- helpers -----------------------------------------------------------------

fn rerank_response_body() -> Value {
    json!({
        "data": [
            { "index": 1, "relevance_score": 0.5703125 },
            { "index": 0, "relevance_score": 0.255859375 }
        ],
        "model": "rerank-2.5",
        "object": "list",
        "usage": { "total_tokens": 12 }
    })
}

fn provider(server: &MockServer) -> VoyageProvider {
    let config = VoyageConfig::new("test-api-key").with_base_url(server.uri());
    VoyageProvider::new(config)
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

async fn mount_rerank_mock(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/rerank"))
        .respond_with(ResponseTemplate::new(200).set_body_json(rerank_response_body()))
        .mount(server)
        .await;
}

// -- object documents tests --------------------------------------------------

#[tokio::test]
async fn object_docs_should_send_request_with_stringified_json_documents() {
    let server = MockServer::start().await;
    mount_rerank_mock(&server).await;

    let provider = provider(&server);
    let model = provider.reranking_model("rerank-2.5");

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
    assert_eq!(body["model"], "rerank-2.5");
    assert_eq!(body["query"], "rainy day");
    assert_eq!(body["top_k"], 2);
}

#[tokio::test]
async fn object_docs_should_send_request_with_correct_headers() {
    let server = MockServer::start().await;
    mount_rerank_mock(&server).await;

    let provider = provider(&server);
    let model = provider.reranking_model("rerank-2.5");

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
async fn object_docs_should_return_result_with_warnings() {
    let server = MockServer::start().await;
    mount_rerank_mock(&server).await;

    let provider = provider(&server);
    let model = provider.reranking_model("rerank-2.5");

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
async fn object_docs_should_return_result_with_correct_ranking() {
    let server = MockServer::start().await;
    mount_rerank_mock(&server).await;

    let provider = provider(&server);
    let model = provider.reranking_model("rerank-2.5");

    let result = model
        .do_rerank(&json_docs_opts("rainy day", 2))
        .await
        .unwrap();

    assert_eq!(result.ranking.len(), 2);
    assert_eq!(result.ranking[0].index, 1);
    assert!((result.ranking[0].relevance_score - 0.5703125).abs() < 1e-8);
    assert_eq!(result.ranking[1].index, 0);
    assert!((result.ranking[1].relevance_score - 0.255859375).abs() < 1e-8);
}

#[tokio::test]
async fn object_docs_should_return_result_with_correct_response_body() {
    let server = MockServer::start().await;
    mount_rerank_mock(&server).await;

    let provider = provider(&server);
    let model = provider.reranking_model("rerank-2.5");

    let result = model
        .do_rerank(&json_docs_opts("rainy day", 2))
        .await
        .unwrap();

    let response = result.response.expect("response");
    let body = response.body.expect("body");
    assert_eq!(body["data"][0]["index"], 1);
    assert_eq!(body["data"][0]["relevance_score"], 0.5703125);
    assert_eq!(body["model"], "rerank-2.5");
    assert_eq!(body["object"], "list");
    assert_eq!(body["usage"]["total_tokens"], 12);
}

// -- text documents tests ----------------------------------------------------

#[tokio::test]
async fn text_docs_should_send_request_with_text_documents() {
    let server = MockServer::start().await;
    mount_rerank_mock(&server).await;

    let provider = provider(&server);
    let model = provider.reranking_model("rerank-2.5");

    model
        .do_rerank(&text_docs_opts("rainy day", 2))
        .await
        .unwrap();

    let requests = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["documents"][0], "sunny day at the beach");
    assert_eq!(body["documents"][1], "rainy day in the city");
    assert_eq!(body["model"], "rerank-2.5");
    assert_eq!(body["query"], "rainy day");
    assert_eq!(body["top_k"], 2);
}

#[tokio::test]
async fn text_docs_should_send_request_with_correct_headers() {
    let server = MockServer::start().await;
    mount_rerank_mock(&server).await;

    let provider = provider(&server);
    let model = provider.reranking_model("rerank-2.5");

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
    let model = provider.reranking_model("rerank-2.5");

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
    let model = provider.reranking_model("rerank-2.5");

    let result = model
        .do_rerank(&text_docs_opts("rainy day", 2))
        .await
        .unwrap();

    assert_eq!(result.ranking.len(), 2);
    assert_eq!(result.ranking[0].index, 1);
    assert!((result.ranking[0].relevance_score - 0.5703125).abs() < 1e-8);
}

#[tokio::test]
async fn text_docs_should_return_result_with_correct_response_body() {
    let server = MockServer::start().await;
    mount_rerank_mock(&server).await;

    let provider = provider(&server);
    let model = provider.reranking_model("rerank-2.5");

    let result = model
        .do_rerank(&text_docs_opts("rainy day", 2))
        .await
        .unwrap();

    let response = result.response.expect("response");
    let body = response.body.expect("body");
    assert_eq!(body["data"][0]["index"], 1);
    assert_eq!(body["data"][0]["relevance_score"], 0.5703125);
}
