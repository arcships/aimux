//! The full `SignedHeaders` of every `SigV4Fetch` caller, as literals.
//!
//! AI SDK counterparts, run with their own locked dependencies (no npm
//! overrides; aws4fetch 1.0.20) against a capturing mock fetch:
//!
//! | caller | TS package | SignedHeaders (no session token) |
//! |---|---|---|
//! | bedrock chat / stream / embedding / image / reranking | `@ai-sdk/amazon-bedrock@5.0.100` | `host;x-amz-date` |
//! | anthropic_aws | `@ai-sdk/anthropic-aws@2.0.63` | `anthropic-version;anthropic-workspace-id;host;x-amz-date` |
//!
//! With a session token each adds `;x-amz-security-token`; none sends
//! `x-amz-content-sha256`. Amazon Polly has no AI SDK counterpart: its test
//! locks what aimux signs today (every header, plus `x-amz-content-sha256`).

use wiremock::matchers::any;
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel};
use aimux_core::image_model::{ImageCallOptions, ImageModel};
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelMessage, TextPart, UserPart};
use aimux_core::options::CallOptions;
use aimux_core::reranking_model::{RerankingCallOptions, RerankingDocuments, RerankingModel};
use aimux_core::speech_model::{SpeechCallOptions, SpeechModel};
use aimux_provider_utils::{AwsCredentials, Resolvable};
use aimux_providers::aws_polly::{AwsPollyProviderSettings, create_aws_polly};
use aimux_providers::{
    AmazonBedrockProviderSettings, AnthropicAwsAuth, AnthropicAwsProviderSettings,
    create_amazon_bedrock, create_anthropic_aws,
};

struct Sent {
    signed_headers: String,
    content_sha256: bool,
}

/// Every request fails with 400; only the first request is inspected.
async fn server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(400).set_body_string("{}"))
        .mount(&server)
        .await;
    server
}

async fn sent(server: &MockServer) -> Sent {
    let requests = server.received_requests().await.unwrap();
    let request = &requests[0];
    let authorization = request.headers["authorization"].to_str().unwrap();
    let signed_headers = authorization
        .split("SignedHeaders=")
        .nth(1)
        .and_then(|rest| rest.split(',').next())
        .unwrap_or_else(|| panic!("not SigV4: {authorization}"))
        .to_string();
    Sent {
        signed_headers,
        content_sha256: request.headers.contains_key("x-amz-content-sha256"),
    }
}

fn prompt() -> CallOptions {
    let mut options = CallOptions::new(vec![LanguageModelMessage::User {
        content: vec![UserPart::Text(TextPart {
            text: "hi".into(),
            provider_options: None,
        })],
        provider_options: None,
    }]);
    options.max_retries = Some(0);
    options
}

fn bedrock(server: &MockServer, token: bool) -> aimux_providers::bedrock::AmazonBedrockProvider {
    assert!(std::env::var_os("AWS_BEARER_TOKEN_BEDROCK").is_none());
    create_amazon_bedrock(AmazonBedrockProviderSettings {
        base_url: Some(server.uri()),
        region: Some("us-east-1".into()),
        access_key_id: Some("AKIDEXAMPLE".into()),
        secret_access_key: Some("secret".into()),
        session_token: token.then(|| "TOKEN".into()),
        ..Default::default()
    })
    .unwrap()
}

const BEDROCK: &str = "host;x-amz-date";
const BEDROCK_TOKEN: &str = "host;x-amz-date;x-amz-security-token";

async fn bedrock_case(
    token: bool,
    call: impl AsyncFnOnce(&aimux_providers::bedrock::AmazonBedrockProvider),
) {
    let server = server().await;
    call(&bedrock(&server, token)).await;
    let sent = sent(&server).await;
    assert_eq!(
        sent.signed_headers,
        if token { BEDROCK_TOKEN } else { BEDROCK }
    );
    assert!(!sent.content_sha256);
}

async fn chat(p: &aimux_providers::bedrock::AmazonBedrockProvider) {
    let _ = p.chat("amazon.nova-pro-v1:0").do_generate(&prompt()).await;
}
async fn stream(p: &aimux_providers::bedrock::AmazonBedrockProvider) {
    let _ = p.chat("amazon.nova-pro-v1:0").do_stream(&prompt()).await;
}
async fn embedding(p: &aimux_providers::bedrock::AmazonBedrockProvider) {
    let mut options = EmbeddingCallOptions {
        values: vec!["x".into()],
        abort_signal: None,
        provider_options: None,
        headers: None,
        max_retries: None,
        timeout: None,
    };
    options.max_retries = Some(0);
    let _ = p
        .embedding("amazon.titan-embed-text-v2:0")
        .do_embed(&options)
        .await;
}
async fn image(p: &aimux_providers::bedrock::AmazonBedrockProvider) {
    let mut options = ImageCallOptions::new("x".to_string());
    options.max_retries = Some(0);
    let _ = p
        .image("amazon.nova-canvas-v1:0")
        .do_generate(&options)
        .await;
}
async fn reranking(p: &aimux_providers::bedrock::AmazonBedrockProvider) {
    let mut options = RerankingCallOptions::new(
        "q",
        RerankingDocuments::Text {
            values: vec!["a".into()],
        },
    );
    options.max_retries = Some(0);
    let _ = p.reranking("amazon.rerank-v1:0").do_rerank(&options).await;
}

#[tokio::test]
async fn bedrock_chat_signed_headers() {
    bedrock_case(false, chat).await;
    bedrock_case(true, chat).await;
}
#[tokio::test]
async fn bedrock_stream_signed_headers() {
    bedrock_case(false, stream).await;
    bedrock_case(true, stream).await;
}
#[tokio::test]
async fn bedrock_embedding_signed_headers() {
    bedrock_case(false, embedding).await;
    bedrock_case(true, embedding).await;
}
#[tokio::test]
async fn bedrock_image_signed_headers() {
    bedrock_case(false, image).await;
    bedrock_case(true, image).await;
}
#[tokio::test]
async fn bedrock_reranking_signed_headers() {
    bedrock_case(false, reranking).await;
    bedrock_case(true, reranking).await;
}

async fn anthropic_aws_case(token: bool) -> Sent {
    anthropic_aws_case_with(token, Some("ws")).await
}

async fn anthropic_aws_case_with(token: bool, workspace_id: Option<&str>) -> Sent {
    let server = server().await;
    let provider = create_anthropic_aws(AnthropicAwsProviderSettings {
        base_url: Some(server.uri()),
        region: Some("us-east-1".into()),
        workspace_id: workspace_id.map(String::from),
        auth: Some(AnthropicAwsAuth::SigV4(Resolvable::Value(AwsCredentials {
            access_key_id: "AKIDEXAMPLE".into(),
            secret_access_key: "secret".into(),
            session_token: token.then(|| "TOKEN".into()),
            region: "us-east-1".into(),
        }))),
        ..Default::default()
    })
    .unwrap();
    let _ = provider
        .messages("claude-sonnet-4-5")
        .do_generate(&prompt())
        .await;
    sent(&server).await
}

#[tokio::test]
async fn anthropic_aws_signed_headers() {
    let sent = anthropic_aws_case(false).await;
    assert_eq!(
        sent.signed_headers,
        "anthropic-version;anthropic-workspace-id;host;x-amz-date"
    );
    assert!(!sent.content_sha256);
    let sent = anthropic_aws_case(true).await;
    assert_eq!(
        sent.signed_headers,
        "anthropic-version;anthropic-workspace-id;host;x-amz-date;x-amz-security-token"
    );
    assert!(!sent.content_sha256);
}

async fn aws_polly_case(token: bool) -> Sent {
    let server = server().await;
    let provider = create_aws_polly(AwsPollyProviderSettings {
        base_url: Some(server.uri()),
        region: Some("us-east-1".into()),
        access_key_id: Some("AKIDEXAMPLE".into()),
        secret_access_key: Some("secret".into()),
        session_token: token.then(|| "TOKEN".into()),
        ..Default::default()
    })
    .unwrap();
    let mut options = SpeechCallOptions::new("hi".to_string());
    options.max_retries = Some(0);
    let _ = provider.speech("standard").do_generate(&options).await;
    sent(&server).await
}

/// No AI SDK counterpart, session token: the `AllHeaders` set plus
/// `x-amz-security-token`.
#[tokio::test]
async fn aws_polly_signed_headers_with_session_token_are_locked() {
    let sent = aws_polly_case(true).await;
    assert_eq!(
        sent.signed_headers,
        "content-type;host;user-agent;x-amz-content-sha256;x-amz-date;x-amz-security-token"
    );
    assert!(sent.content_sha256);
}

/// No AI SDK counterpart: locks aimux's current signing.
#[tokio::test]
async fn aws_polly_signed_headers_are_locked() {
    let sent = aws_polly_case(false).await;
    assert_eq!(
        sent.signed_headers,
        "content-type;host;user-agent;x-amz-content-sha256;x-amz-date"
    );
    assert!(sent.content_sha256);
}

/// No workspace id. `@ai-sdk/anthropic-aws@2.0.63` sends no request at all
/// here: `getHeaders` throws `AI_LoadSettingError` (workspace ID missing,
/// anthropic-aws-provider.ts:235-240, `ANTHROPIC_AWS_WORKSPACE_ID` as the
/// fallback), so there is no TS SignedHeaders to match. This locks what
/// aimux, which treats the workspace id as optional, signs today.
#[tokio::test]
async fn anthropic_aws_signed_headers_without_workspace_id() {
    let sent = anthropic_aws_case_with(false, None).await;
    assert_eq!(sent.signed_headers, "anthropic-version;host;x-amz-date");
    assert!(!sent.content_sha256);
}
