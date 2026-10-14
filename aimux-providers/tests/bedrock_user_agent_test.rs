//! The full user-agents the Amazon Bedrock family sends, as literals.
//!
//! `@ai-sdk/amazon-bedrock@5.0.100` (which pins `@ai-sdk/provider-utils@5.0.51`),
//! run against a capturing mock fetch, sends for Converse (API key and
//! SigV4), Anthropic-on-Bedrock and Mantle alike:
//!
//! `ai-sdk-amazon-bedrock/5.0.100 ai-sdk-provider-utils/5.0.51 node.js/v20.19.2 ai-sdk-amazon-bedrock/5.0.100 node.js/v20.19.2`
//!
//! `getHeaders` adds the first part (amazon-bedrock-provider.ts:294,
//! bedrock-mantle-provider.ts:237), `postToApi` the provider-utils part, and
//! the transport the second package part (amazon-bedrock-sigv4-fetch.ts:133 for an
//! API key, :46 for SigV4). aimux leaves
//! out the runtime fragments (`node.js/…`), as for every provider.
//!
//! The versions are literals on purpose: a test that reads the pin could not
//! catch the constant and `VERSIONS.json` drifting together.

use std::collections::HashMap;

use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::embedding_model::EmbeddingCallOptions;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelMessage, TextPart, UserPart};
use aimux_core::options::CallOptions;
use aimux_core::provider::Provider;
use aimux_provider_utils::{AwsCredentials, Resolvable};
use aimux_providers::{
    AmazonBedrockProviderSettings, AnthropicAwsAuth, AnthropicAwsProviderSettings, PresetSettings,
    create_amazon_bedrock, create_anthropic_aws, create_provider,
};

/// What `@ai-sdk/amazon-bedrock@5.0.100` sends, runtime fragments removed.
const BEDROCK_UA: &str =
    "ai-sdk-amazon-bedrock/5.0.100 ai-sdk-provider-utils/5.0.51 ai-sdk-amazon-bedrock/5.0.100";

async fn sent_user_agent(server: &MockServer) -> String {
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    requests[0]
        .headers
        .get("user-agent")
        .expect("user-agent")
        .to_str()
        .unwrap()
        .to_string()
}

async fn sent_authorization(server: &MockServer) -> Option<String> {
    let requests = server.received_requests().await.unwrap();
    requests[0]
        .headers
        .get("authorization")
        .map(|value| value.to_str().unwrap().to_string())
}

fn embed_options() -> EmbeddingCallOptions {
    EmbeddingCallOptions {
        values: vec!["x".to_string()],
        abort_signal: None,
        provider_options: None,
        headers: None,
        max_retries: None,
        timeout: None,
    }
}

async fn titan_server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/model/amazon.titan-embed-text-v2%3A0/invoke"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "embedding": [0.1], "inputTextTokenCount": 1 })),
        )
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn bedrock_api_key_user_agent_matches_the_ai_sdk() {
    let server = titan_server().await;
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
        .do_embed(&embed_options())
        .await
        .unwrap();
    assert_eq!(
        sent_authorization(&server).await.as_deref(),
        Some("Bearer test-auth")
    );
    assert_eq!(sent_user_agent(&server).await, BEDROCK_UA);
}

#[tokio::test]
async fn bedrock_sigv4_user_agent_matches_the_ai_sdk() {
    assert!(
        std::env::var_os("AWS_BEARER_TOKEN_BEDROCK").is_none(),
        "this test needs the SigV4 path"
    );
    let server = titan_server().await;
    let provider = create_amazon_bedrock(AmazonBedrockProviderSettings {
        base_url: Some(server.uri()),
        api_key: None,
        region: Some("us-east-1".to_string()),
        access_key_id: Some("AKIDEXAMPLE".to_string()),
        secret_access_key: Some("secret".to_string()),
        ..Default::default()
    })
    .unwrap();
    provider
        .embedding_model("amazon.titan-embed-text-v2:0")
        .unwrap()
        .do_embed(&embed_options())
        .await
        .unwrap();
    let authorization = sent_authorization(&server).await.expect("signed");
    assert!(
        authorization.starts_with("AWS4-HMAC-SHA256 "),
        "{authorization}"
    );
    // Like aws4fetch in @ai-sdk/amazon-bedrock@5.0.100 (captured
    // `SignedHeaders=host;x-amz-date`): the user-agent is sent, not signed.
    assert!(
        authorization.contains(" SignedHeaders=host;x-amz-date, "),
        "{authorization}"
    );
    let requests = server.received_requests().await.unwrap();
    assert!(!requests[0].headers.contains_key("x-amz-content-sha256"));
    assert_eq!(sent_user_agent(&server).await, BEDROCK_UA);
}

#[tokio::test]
async fn bedrock_user_agent_follows_a_caller_user_agent() {
    let server = titan_server().await;
    let provider = create_amazon_bedrock(AmazonBedrockProviderSettings {
        base_url: Some(server.uri()),
        api_key: Some("test-auth".to_string().into()),
        region: Some("us-east-1".to_string()),
        headers: Some(HashMap::from([(
            "User-Agent".to_string(),
            Some("app/1".to_string()),
        )])),
        ..Default::default()
    })
    .unwrap();
    provider
        .embedding_model("amazon.titan-embed-text-v2:0")
        .unwrap()
        .do_embed(&embed_options())
        .await
        .unwrap();
    assert_eq!(
        sent_user_agent(&server).await,
        format!("app/1 {BEDROCK_UA}")
    );
}

fn chat_options() -> CallOptions {
    CallOptions::new(vec![LanguageModelMessage::User {
        content: vec![UserPart::Text(TextPart {
            text: "hi".into(),
            provider_options: None,
        })],
        provider_options: None,
    }])
}

#[tokio::test]
async fn bedrock_mantle_preset_user_agent_matches_the_ai_sdk() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "c", "object": "chat.completion", "created": 0, "model": "m",
            "choices": [{ "index": 0, "finish_reason": "stop",
                "message": { "role": "assistant", "content": "ok" } }],
        })))
        .mount(&server)
        .await;
    let provider = create_provider(
        "bedrock_mantle",
        PresetSettings {
            api_key: Some(Resolvable::Value("k".to_string())),
            base_url: Some(server.uri()),
            ..Default::default()
        },
    )
    .unwrap();
    provider
        .language_model("openai.gpt-oss-120b")
        .unwrap()
        .do_generate(&chat_options())
        .await
        .unwrap();
    assert_eq!(
        sent_authorization(&server).await.as_deref(),
        Some("Bearer k")
    );
    assert_eq!(sent_user_agent(&server).await, BEDROCK_UA);
}

/// Anthropic-AWS (Claude Platform on AWS, `aws-external-anthropic`) is not
/// Bedrock. `@ai-sdk/anthropic-aws@2.0.63`, run against a capturing mock
/// fetch, sends (API key and SigV4 alike)
/// `ai-sdk-provider-utils/5.0.53 node.js/v20.19.2 ai-sdk-anthropic-aws/2.0.63 node.js/v20.19.2`,
/// a caller user-agent first; aimux leaves out the runtime fragments.
const ANTHROPIC_AWS_UA: &str = "ai-sdk-provider-utils/5.0.53 ai-sdk-anthropic-aws/2.0.63";

async fn anthropic_aws_user_agent(auth: AnthropicAwsAuth) -> (String, Option<String>) {
    anthropic_aws_user_agent_with(auth, None).await
}

async fn anthropic_aws_user_agent_with(
    auth: AnthropicAwsAuth,
    headers: Option<HashMap<String, Option<String>>>,
) -> (String, Option<String>) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "msg", "type": "message", "role": "assistant", "model": "m",
            "content": [{ "type": "text", "text": "ok" }],
            "stop_reason": "end_turn", "stop_sequence": null,
            "usage": { "input_tokens": 1, "output_tokens": 1 },
        })))
        .mount(&server)
        .await;
    let provider = create_anthropic_aws(AnthropicAwsProviderSettings {
        base_url: Some(server.uri()),
        region: Some("us-east-1".to_string()),
        auth: Some(auth),
        workspace_id: Some("ws".to_string()),
        headers,
        ..Default::default()
    })
    .unwrap();
    provider
        .messages("claude-sonnet-4-5")
        .do_generate(&chat_options())
        .await
        .unwrap();
    (
        sent_user_agent(&server).await,
        sent_authorization(&server).await,
    )
}

#[tokio::test]
async fn anthropic_aws_api_key_user_agent() {
    let (ua, _) =
        anthropic_aws_user_agent(AnthropicAwsAuth::ApiKey(Resolvable::Value("k".into()))).await;
    assert_eq!(ua, ANTHROPIC_AWS_UA);
}

#[tokio::test]
async fn anthropic_aws_user_agent_follows_a_caller_user_agent() {
    let (ua, _) = anthropic_aws_user_agent_with(
        AnthropicAwsAuth::ApiKey(Resolvable::Value("k".into())),
        Some(HashMap::from([(
            "User-Agent".to_string(),
            Some("app/1".to_string()),
        )])),
    )
    .await;
    assert_eq!(ua, format!("app/1 {ANTHROPIC_AWS_UA}"));
}

#[tokio::test]
async fn anthropic_aws_sigv4_user_agent() {
    let (ua, authorization) =
        anthropic_aws_user_agent(AnthropicAwsAuth::SigV4(Resolvable::Value(AwsCredentials {
            access_key_id: "AKIDEXAMPLE".into(),
            secret_access_key: "secret".into(),
            session_token: None,
            region: "us-east-1".into(),
        })))
        .await;
    let authorization = authorization.unwrap();
    // @ai-sdk/anthropic-aws@2.0.63 (aws4fetch) signs exactly these.
    assert!(
        authorization
            .contains(" SignedHeaders=anthropic-version;anthropic-workspace-id;host;x-amz-date, "),
        "{authorization}"
    );
    assert_eq!(ua, ANTHROPIC_AWS_UA);
}

fn bedrock_api_key_provider(
    server: &MockServer,
) -> aimux_providers::bedrock::AmazonBedrockProvider {
    create_amazon_bedrock(AmazonBedrockProviderSettings {
        base_url: Some(server.uri()),
        api_key: Some("test-auth".to_string().into()),
        region: Some("us-east-1".to_string()),
        ..Default::default()
    })
    .unwrap()
}

#[tokio::test]
async fn bedrock_chat_user_agent_matches_the_ai_sdk() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/model/amazon.nova-pro-v1%3A0/converse"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "output": { "message": { "role": "assistant", "content": [{ "text": "ok" }] } },
            "stopReason": "end_turn",
            "usage": { "inputTokens": 1, "outputTokens": 1, "totalTokens": 2 }
        })))
        .mount(&server)
        .await;
    bedrock_api_key_provider(&server)
        .chat("amazon.nova-pro-v1:0")
        .do_generate(&chat_options())
        .await
        .unwrap();
    assert_eq!(sent_user_agent(&server).await, BEDROCK_UA);
}

#[tokio::test]
async fn bedrock_stream_user_agent_matches_the_ai_sdk() {
    use futures::StreamExt;
    let server = MockServer::start().await;
    let body = aimux_providers::bedrock::event_stream::encode_messages(&[
        ("event", "messageStart", r#"{"role":"assistant"}"#),
        ("event", "messageStop", r#"{"stopReason":"end_turn"}"#),
    ]);
    Mock::given(method("POST"))
        .and(path("/model/amazon.nova-pro-v1%3A0/converse-stream"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/vnd.amazon.eventstream")
                .set_body_bytes(body),
        )
        .mount(&server)
        .await;
    let result = bedrock_api_key_provider(&server)
        .chat("amazon.nova-pro-v1:0")
        .do_stream(&chat_options())
        .await
        .unwrap();
    let _parts: Vec<_> = result.stream.collect().await;
    assert_eq!(sent_user_agent(&server).await, BEDROCK_UA);
}

#[tokio::test]
async fn bedrock_mantle_preset_user_agent_follows_a_caller_user_agent() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "c", "object": "chat.completion", "created": 0, "model": "m",
            "choices": [{ "index": 0, "finish_reason": "stop",
                "message": { "role": "assistant", "content": "ok" } }],
        })))
        .mount(&server)
        .await;
    let provider = create_provider(
        "bedrock_mantle",
        PresetSettings {
            api_key: Some(Resolvable::Value("k".to_string())),
            base_url: Some(server.uri()),
            headers: Some(HashMap::from([(
                "User-Agent".to_string(),
                Some("app/1".to_string()),
            )])),
            ..Default::default()
        },
    )
    .unwrap();
    provider
        .language_model("openai.gpt-oss-120b")
        .unwrap()
        .do_generate(&chat_options())
        .await
        .unwrap();
    assert_eq!(
        sent_user_agent(&server).await,
        format!("app/1 {BEDROCK_UA}")
    );
}
