//! The Bedrock user-agent carries `ai-sdk-amazon-bedrock/<pinned>` twice,
//! like `@ai-sdk/amazon-bedrock@5.0.100`: `getHeaders` appends it
//! (amazon-bedrock-provider.ts:294) and the API-key / SigV4 fetch appends it
//! again (amazon-bedrock-sigv4-fetch.ts:44, :131). Both must be the pinned
//! version, not aimux's crate version.

#[path = "../../fixtures/aisdk/pinned_versions.rs"]
mod pinned_versions;

use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::embedding_model::EmbeddingCallOptions;
use aimux_core::provider::Provider;
use aimux_providers::{AmazonBedrockProviderSettings, create_amazon_bedrock};

#[tokio::test]
async fn both_bedrock_user_agent_parts_carry_the_pinned_version() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/model/amazon.titan-embed-text-v2%3A0/invoke"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "embedding": [0.1], "inputTextTokenCount": 1 })),
        )
        .mount(&server)
        .await;
    let provider = create_amazon_bedrock(AmazonBedrockProviderSettings {
        base_url: Some(server.uri()),
        api_key: Some("test-auth".to_string().into()),
        region: Some("us-east-1".to_string()),
        ..Default::default()
    })
    .unwrap();
    provider
        .embedding_model("amazon.titan-embed-text-v2:0")
        .unwrap()
        .do_embed(&EmbeddingCallOptions {
            values: vec!["x".to_string()],
            abort_signal: None,
            provider_options: None,
            headers: None,
            max_retries: None,
            timeout: None,
        })
        .await
        .unwrap();

    let requests = server.received_requests().await.unwrap();
    let ua = requests[0]
        .headers
        .get("user-agent")
        .unwrap()
        .to_str()
        .unwrap();
    let expected = format!(
        "ai-sdk-amazon-bedrock/{}",
        pinned_versions::pinned("@ai-sdk/amazon-bedrock")
    );
    let parts: Vec<&str> = ua
        .split(' ')
        .filter(|part| part.starts_with("ai-sdk-amazon-bedrock/"))
        .collect();
    assert_eq!(parts, [expected.as_str(), expected.as_str()], "{ua}");
}
