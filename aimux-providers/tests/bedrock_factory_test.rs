//! `create_amazon_bedrock`: authentication, region and endpoint resolution.
//!
//! `@ai-sdk/amazon-bedrock` has no recorded fixtures, so these tests pin
//! `amazon-bedrock-provider.ts` directly, through an injected [`Fetch`] (the
//! factory's `fetch` setting) that records each request exactly as the
//! signing decorator forwards it.
//!
//! Authentication is decided per request: an API key (the `api_key` setting,
//! else `AWS_BEARER_TOKEN_BEDROCK`) sends `Authorization: Bearer` and signs
//! nothing; otherwise the `SigV4Fetch` decorator signs the final method, URL,
//! headers and body bytes. Creating a provider reads no environment variable
//! and cannot fail for a missing region, key or credential: those fail the
//! call with `LoadSetting`, naming the variable and never its value.

#[path = "common/mock_fetch.rs"]
mod mock_fetch;

use std::sync::Arc;

use serde_json::json;
use serial_test::serial;
use sha2::{Digest, Sha256};

use aimux_core::AiMuxError;
use aimux_core::content::ContentPart;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::LanguageModelPromptMessage;
use aimux_core::message::Role;
use aimux_core::options::CallOptions;
use aimux_provider_utils::{HeaderMapOpt, Resolvable};
use aimux_providers::bedrock::{
    AmazonBedrockProvider, AmazonBedrockProviderSettings, create_amazon_bedrock,
};

use mock_fetch::{Canned, EnvVar, MockFetch, Seen};

const MODEL: &str = "anthropic.claude-3-5-sonnet-20240620-v1:0";

const AWS_VARS: [&str; 7] = [
    "AWS_BEARER_TOKEN_BEDROCK",
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
    "AWS_REGION",
    "AWS_ENDPOINT_URL_BEDROCK_RUNTIME",
    "AWS_ENDPOINT_URL_BEDROCK_AGENT_RUNTIME",
];

/// Remove every AWS variable Bedrock consults for the length of a test.
fn clean_env() -> Vec<EnvVar> {
    AWS_VARS
        .iter()
        .map(|name| EnvVar::set(name, None))
        .collect()
}

fn user_prompt(text: &str) -> CallOptions {
    CallOptions::new(vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![ContentPart::text(text)],
        ..Default::default()
    }])
}

fn converse_response() -> Canned {
    Canned::json(&json!({
        "output": { "message": { "role": "assistant", "content": [{ "text": "ok" }] } },
        "stopReason": "end_turn",
        "usage": { "inputTokens": 1, "outputTokens": 1, "totalTokens": 2 }
    }))
}

fn sigv4(mock: &Arc<MockFetch>) -> AmazonBedrockProviderSettings {
    AmazonBedrockProviderSettings {
        region: Some("us-east-1".to_string()),
        access_key_id: Some("AKIAIOSFODNN7EXAMPLE".to_string()),
        secret_access_key: Some("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_string()),
        fetch: Some(mock.transport()),
        ..Default::default()
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn bedrock(settings: AmazonBedrockProviderSettings) -> AmazonBedrockProvider {
    create_amazon_bedrock(settings).expect("valid settings")
}

/// The request was signed over exactly its own body bytes.
fn assert_signed(seen: &Seen, region: &str) {
    let authorization = &seen.headers["authorization"];
    assert!(
        authorization.starts_with("AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/"),
        "{authorization}"
    );
    assert!(
        authorization.contains(&format!("/{region}/bedrock/aws4_request")),
        "{authorization}"
    );
    assert_eq!(
        seen.headers["x-amz-content-sha256"],
        sha256_hex(&seen.body),
        "the signature covers the final body bytes"
    );
    assert!(seen.headers.contains_key("x-amz-date"));
}

// ── identity ─────────────────────────────────────────────────────────────────

// ── namespaces ───────────────────────────────────────────────────────────────

// ── API key path ─────────────────────────────────────────────────────────────

#[serial]
#[tokio::test]
async fn an_api_key_sends_a_bearer_token_and_signs_nothing() {
    let _env = clean_env();
    let mock = MockFetch::new(vec![converse_response()]);
    let model = bedrock(AmazonBedrockProviderSettings {
        api_key: Some(Resolvable::Value("bedrock-key".to_string())),
        region: Some("us-west-2".to_string()),
        // Credentials that must not be used while a key is available.
        access_key_id: Some("AKIAIOSFODNN7EXAMPLE".to_string()),
        secret_access_key: Some("secret".to_string()),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .chat(MODEL);

    model.do_generate(&user_prompt("hi")).await.unwrap();

    let seen = mock.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "POST");
    assert_eq!(
        seen[0].url,
        "https://bedrock-runtime.us-west-2.amazonaws.com/model/anthropic.claude-3-5-sonnet-20240620-v1:0/converse"
    );
    assert_eq!(seen[0].headers["authorization"], "Bearer bedrock-key");
    assert!(
        !seen[0]
            .headers
            .keys()
            .any(|name| name.starts_with("x-amz-")),
        "nothing is signed: {:?}",
        seen[0].headers
    );
}

// ── SigV4 path ───────────────────────────────────────────────────────────────

#[serial]
#[tokio::test]
async fn sigv4_signs_the_final_bytes_of_the_request() {
    let _env = clean_env();
    let mock = MockFetch::new(vec![converse_response()]);
    let mut headers = HeaderMapOpt::new();
    headers.insert("X-Team".to_string(), Some("blue".to_string()));
    bedrock(AmazonBedrockProviderSettings {
        headers: Some(headers),
        ..sigv4(&mock)
    })
    .chat(MODEL)
    .do_generate(&user_prompt("hi"))
    .await
    .unwrap();

    let seen = mock.seen();
    assert_signed(&seen[0], "us-east-1");
    let authorization = &seen[0].headers["authorization"];
    // Every header that was on the request when it was signed is signed.
    assert!(authorization.contains("x-team"), "{authorization}");
    assert_eq!(seen[0].headers["x-team"], "blue");
    assert!(!seen[0].headers.contains_key("x-amz-security-token"));
}

// ── regions and endpoints ────────────────────────────────────────────────────

#[serial]
#[tokio::test]
async fn an_invalid_region_fails_before_sending_a_request() {
    let _env = clean_env();
    let mock = MockFetch::new(vec![converse_response()]);
    let model = bedrock(AmazonBedrockProviderSettings {
        region: Some("us-east-1.evil.example/#".to_string()),
        api_key: Some(Resolvable::Value("k".to_string())),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .chat(MODEL);

    let result = model.do_generate(&user_prompt("hi")).await;
    assert!(matches!(result, Err(AiMuxError::InvalidArgument(_))));
    assert!(mock.seen().is_empty(), "nothing was sent");
}

// ── discovery ────────────────────────────────────────────────────────────────
