//! Rust translations of the Amazon Bedrock Reranking model tests.
//!
//! Source: `reference/ai/packages/amazon-bedrock/src/reranking/amazon-bedrock-reranking-model.test.ts`
//! (13 test cases across "json documents" and "text documents" groups).

use std::collections::HashMap;

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::reranking_model::{RerankingCallOptions, RerankingDocuments, RerankingModel};
use aimux_providers::{BedrockProvider, BedrockProviderConfig};

// -- helpers -----------------------------------------------------------------

fn rerank_response_body() -> Value {
    json!({
        "results": [
            { "index": 0, "relevanceScore": 0.5110583305358887 },
            { "index": 5, "relevanceScore": 0.30241215229034424 }
        ]
    })
}

fn provider(server: &MockServer) -> BedrockProvider {
    let config = BedrockProviderConfig::with_bearer_token("test-auth", "us-west-2")
        .with_base_url(server.uri());
    BedrockProvider::new(config)
}

fn bedrock_provider_options() -> HashMap<String, Value> {
    let mut po = HashMap::new();
    po.insert(
        "bedrock".to_string(),
        json!({
            "nextToken": "test-token",
            "additionalModelRequestFields": { "test": "test-value" }
        }),
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
        provider_options: Some(bedrock_provider_options()),
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
        provider_options: Some(bedrock_provider_options()),
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
    let model = provider.reranking_model("cohere.rerank-v3-5:0");

    model
        .do_rerank(&json_docs_opts("rainy day", 2))
        .await
        .unwrap();

    let requests = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["nextToken"], "test-token");
    assert_eq!(body["queries"][0]["textQuery"]["text"], "rainy day");
    assert_eq!(body["queries"][0]["type"], "TEXT");
    assert_eq!(
        body["rerankingConfiguration"]["bedrockRerankingConfiguration"]["modelConfiguration"]["modelArn"],
        "arn:aws:bedrock:us-west-2::foundation-model/cohere.rerank-v3-5:0"
    );
    assert_eq!(
        body["rerankingConfiguration"]["bedrockRerankingConfiguration"]["modelConfiguration"]["additionalModelRequestFields"]
            ["test"],
        "test-value"
    );
    assert_eq!(
        body["rerankingConfiguration"]["bedrockRerankingConfiguration"]["numberOfResults"],
        2
    );
    assert_eq!(
        body["rerankingConfiguration"]["type"],
        "BEDROCK_RERANKING_MODEL"
    );
    // JSON document sources
    assert_eq!(body["sources"][0]["type"], "INLINE");
    assert_eq!(body["sources"][0]["inlineDocumentSource"]["type"], "JSON");
    assert_eq!(
        body["sources"][0]["inlineDocumentSource"]["jsonDocument"]["example"],
        "sunny day at the beach"
    );
    assert_eq!(
        body["sources"][1]["inlineDocumentSource"]["jsonDocument"]["example"],
        "rainy day in the city"
    );
}

#[tokio::test]
async fn json_docs_should_send_bedrock_reranking_configuration_wire_key() {
    let server = MockServer::start().await;
    mount_rerank_mock(&server).await;

    let provider = provider(&server);
    let model = provider.reranking_model("cohere.rerank-v3-5:0");

    model
        .do_rerank(&json_docs_opts("rainy day", 2))
        .await
        .unwrap();

    let requests = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    // The Bedrock API requires this exact member name.
    assert!(
        body["rerankingConfiguration"]
            .get("bedrockRerankingConfiguration")
            .is_some(),
        "rerankingConfiguration.bedrockRerankingConfiguration must be present"
    );
}

#[tokio::test]
async fn json_docs_should_send_request_with_correct_headers() {
    let server = MockServer::start().await;
    mount_rerank_mock(&server).await;

    let provider = provider(&server);
    let model = provider.reranking_model("cohere.rerank-v3-5:0");

    model
        .do_rerank(&json_docs_opts("rainy day", 2))
        .await
        .unwrap();

    let requests = server.received_requests().await.unwrap();
    let headers = &requests[0].headers;
    assert_eq!(
        headers.get("content-type").and_then(|v| v.to_str().ok()),
        Some("application/json")
    );
    assert_eq!(
        headers.get("authorization").and_then(|v| v.to_str().ok()),
        Some("Bearer test-auth")
    );
}

#[tokio::test]
async fn json_docs_should_return_result_without_warnings() {
    let server = MockServer::start().await;
    mount_rerank_mock(&server).await;

    let provider = provider(&server);
    let model = provider.reranking_model("cohere.rerank-v3-5:0");

    let result = model
        .do_rerank(&json_docs_opts("rainy day", 2))
        .await
        .unwrap();

    assert!(result.warnings.is_none());
}

#[tokio::test]
async fn json_docs_should_return_result_with_correct_ranking() {
    let server = MockServer::start().await;
    mount_rerank_mock(&server).await;

    let provider = provider(&server);
    let model = provider.reranking_model("cohere.rerank-v3-5:0");

    let result = model
        .do_rerank(&json_docs_opts("rainy day", 2))
        .await
        .unwrap();

    assert_eq!(result.ranking.len(), 2);
    assert_eq!(result.ranking[0].index, 0);
    assert!((result.ranking[0].relevance_score - 0.5110583305358887).abs() < 1e-8);
    assert_eq!(result.ranking[1].index, 5);
    assert!((result.ranking[1].relevance_score - 0.30241215229034424).abs() < 1e-8);
}

#[tokio::test]
async fn json_docs_should_not_return_provider_metadata() {
    let server = MockServer::start().await;
    mount_rerank_mock(&server).await;

    let provider = provider(&server);
    let model = provider.reranking_model("cohere.rerank-v3-5:0");

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
    let model = provider.reranking_model("cohere.rerank-v3-5:0");

    let result = model
        .do_rerank(&json_docs_opts("rainy day", 2))
        .await
        .unwrap();

    let response = result.response.expect("response");
    let body = response.body.expect("body");
    assert_eq!(body["results"][0]["index"], 0);
    assert_eq!(body["results"][0]["relevanceScore"], 0.5110583305358887);
    assert_eq!(body["results"][1]["index"], 5);
    assert_eq!(body["results"][1]["relevanceScore"], 0.30241215229034424);
}

// -- text documents tests ----------------------------------------------------

#[tokio::test]
async fn text_docs_should_send_request_with_text_documents() {
    let server = MockServer::start().await;
    mount_rerank_mock(&server).await;

    let provider = provider(&server);
    let model = provider.reranking_model("cohere.rerank-v3-5:0");

    model
        .do_rerank(&text_docs_opts("rainy day", 2))
        .await
        .unwrap();

    let requests = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["nextToken"], "test-token");
    assert_eq!(body["queries"][0]["textQuery"]["text"], "rainy day");
    assert_eq!(
        body["rerankingConfiguration"]["bedrockRerankingConfiguration"]["modelConfiguration"]["modelArn"],
        "arn:aws:bedrock:us-west-2::foundation-model/cohere.rerank-v3-5:0"
    );
    assert_eq!(
        body["rerankingConfiguration"]["bedrockRerankingConfiguration"]["numberOfResults"],
        2
    );
    // Text document sources
    assert_eq!(body["sources"][0]["type"], "INLINE");
    assert_eq!(body["sources"][0]["inlineDocumentSource"]["type"], "TEXT");
    assert_eq!(
        body["sources"][0]["inlineDocumentSource"]["textDocument"]["text"],
        "sunny day at the beach"
    );
    assert_eq!(
        body["sources"][1]["inlineDocumentSource"]["textDocument"]["text"],
        "rainy day in the city"
    );
}

#[tokio::test]
async fn text_docs_should_send_request_with_correct_headers() {
    let server = MockServer::start().await;
    mount_rerank_mock(&server).await;

    let provider = provider(&server);
    let model = provider.reranking_model("cohere.rerank-v3-5:0");

    model
        .do_rerank(&text_docs_opts("rainy day", 2))
        .await
        .unwrap();

    let requests = server.received_requests().await.unwrap();
    let headers = &requests[0].headers;
    assert_eq!(
        headers.get("content-type").and_then(|v| v.to_str().ok()),
        Some("application/json")
    );
    assert_eq!(
        headers.get("authorization").and_then(|v| v.to_str().ok()),
        Some("Bearer test-auth")
    );
}

#[tokio::test]
async fn text_docs_should_return_result_without_warnings() {
    let server = MockServer::start().await;
    mount_rerank_mock(&server).await;

    let provider = provider(&server);
    let model = provider.reranking_model("cohere.rerank-v3-5:0");

    let result = model
        .do_rerank(&text_docs_opts("rainy day", 2))
        .await
        .unwrap();

    assert!(result.warnings.is_none());
}

#[tokio::test]
async fn text_docs_should_return_result_with_correct_ranking() {
    let server = MockServer::start().await;
    mount_rerank_mock(&server).await;

    let provider = provider(&server);
    let model = provider.reranking_model("cohere.rerank-v3-5:0");

    let result = model
        .do_rerank(&text_docs_opts("rainy day", 2))
        .await
        .unwrap();

    assert_eq!(result.ranking.len(), 2);
    assert_eq!(result.ranking[0].index, 0);
    assert!((result.ranking[0].relevance_score - 0.5110583305358887).abs() < 1e-8);
}

#[tokio::test]
async fn text_docs_should_not_return_provider_metadata() {
    let server = MockServer::start().await;
    mount_rerank_mock(&server).await;

    let provider = provider(&server);
    let model = provider.reranking_model("cohere.rerank-v3-5:0");

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
    let model = provider.reranking_model("cohere.rerank-v3-5:0");

    let result = model
        .do_rerank(&text_docs_opts("rainy day", 2))
        .await
        .unwrap();

    let response = result.response.expect("response");
    let body = response.body.expect("body");
    assert_eq!(body["results"][0]["index"], 0);
    assert_eq!(body["results"][0]["relevanceScore"], 0.5110583305358887);
}
