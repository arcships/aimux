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
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};
use serial_test::serial;
use sha2::{Digest, Sha256};

use aimux_core::AiMuxError;
use aimux_core::content::ContentPart;
use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel};
use aimux_core::image_model::{ImageCallOptions, ImageModel};
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::LanguageModelPromptMessage;
use aimux_core::message::Role;
use aimux_core::options::CallOptions;
use aimux_core::provider::{Provider, ProviderDiscovery};
use aimux_core::reranking_model::{RerankingCallOptions, RerankingDocuments, RerankingModel};
use aimux_provider_utils::{AwsCredentials, HeaderMapOpt, Resolvable};
use aimux_providers::bedrock::{
    AmazonBedrockProvider, AmazonBedrockProviderSettings, amazon_bedrock, create_amazon_bedrock,
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

#[test]
fn every_modality_reports_amazon_bedrock() {
    let provider = bedrock(AmazonBedrockProviderSettings::default());
    assert_eq!(provider.chat(MODEL).provider(), "amazon-bedrock");
    assert_eq!(provider.call(MODEL).provider(), "amazon-bedrock");
    assert_eq!(
        provider.language_model(MODEL).unwrap().provider(),
        "amazon-bedrock"
    );
    assert_eq!(provider.embedding("m").provider(), "amazon-bedrock");
    assert_eq!(provider.image("m").provider(), "amazon-bedrock");
    assert_eq!(provider.reranking("m").provider(), "amazon-bedrock");
}

#[test]
fn the_provider_offers_language_embedding_image_and_reranking() {
    let provider = bedrock(AmazonBedrockProviderSettings::default());
    assert!(provider.language_model("m").is_ok());
    assert!(provider.embedding_model("m").is_ok());
    assert!(provider.image_model("m").is_ok());
    assert!(provider.reranking_model("m").unwrap().is_ok());
    assert!(provider.speech_model("s").is_none());
    assert!(provider.video_model("v").is_none());
    assert!(Provider::files(&provider).is_none());
}

#[serial]
#[test]
fn the_default_instance_reads_nothing_and_cannot_fail() {
    let _env = clean_env();
    assert!(std::ptr::eq(amazon_bedrock(), amazon_bedrock()));
    assert_eq!(amazon_bedrock().chat("m").provider(), "amazon-bedrock");
}

#[test]
fn a_bad_base_url_fails_the_factory() {
    let result = create_amazon_bedrock(AmazonBedrockProviderSettings {
        base_url: Some("not a url".to_string()),
        ..Default::default()
    });
    assert!(matches!(result, Err(AiMuxError::InvalidArgument(_))));
}

// ── namespaces ───────────────────────────────────────────────────────────────

#[serial]
#[tokio::test]
async fn only_the_amazon_bedrock_key_is_read_and_written() {
    let _env = clean_env();
    let mock = MockFetch::new(vec![
        Canned::json(&json!({
            "output": { "message": { "role": "assistant", "content": [
                { "reasoningContent": { "reasoningText": { "text": "t", "signature": "sig" } } },
                { "text": "ok" }
            ] } },
            "stopReason": "end_turn",
            "usage": { "inputTokens": 1, "outputTokens": 1, "totalTokens": 2 }
        })),
        converse_response(),
    ]);
    let model = bedrock(AmazonBedrockProviderSettings {
        api_key: Some(Resolvable::Value("k".to_string())),
        region: Some("us-east-1".to_string()),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .chat(MODEL);

    // Written: canonical key only.
    let result = model.do_generate(&user_prompt("hi")).await.unwrap();
    let reasoning = result
        .content
        .iter()
        .find_map(|part| match part {
            aimux_core::result::GenerateContent::Reasoning {
                provider_metadata, ..
            } => provider_metadata.clone(),
            _ => None,
        })
        .expect("reasoning with a signature");
    assert_eq!(reasoning["amazonBedrock"]["signature"], "sig");
    assert!(reasoning.get("bedrock").is_none());

    // Read: the historical `bedrock` alias is ignored.
    let mut legacy = user_prompt("hi");
    legacy.provider_options = Some(
        [(
            "bedrock".to_string(),
            json!({ "additionalModelRequestFields": { "legacy": true } }),
        )]
        .into_iter()
        .collect(),
    );
    model.do_generate(&legacy).await.unwrap();
    let body = mock.seen()[1].json_body();
    assert!(body.get("additionalModelRequestFields").is_none(), "{body}");
}

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

#[serial]
#[tokio::test]
async fn the_bearer_token_environment_variable_is_read_per_request() {
    let _env = clean_env();
    let _key = EnvVar::set("AWS_BEARER_TOKEN_BEDROCK", Some("  env-bearer  "));
    let mock = MockFetch::new(vec![converse_response(), converse_response()]);
    let model = bedrock(AmazonBedrockProviderSettings {
        region: Some("us-east-1".to_string()),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .chat(MODEL);

    model.do_generate(&user_prompt("one")).await.unwrap();
    unsafe { std::env::set_var("AWS_BEARER_TOKEN_BEDROCK", "rotated") };
    model.do_generate(&user_prompt("two")).await.unwrap();

    let seen = mock.seen();
    assert_eq!(
        seen[0].headers["authorization"], "Bearer env-bearer",
        "trimmed"
    );
    assert_eq!(seen[1].headers["authorization"], "Bearer rotated");
}

#[serial]
#[tokio::test]
async fn an_explicit_key_wins_over_the_environment_and_blank_keys_fall_to_sigv4() {
    let _env = clean_env();
    let _key = EnvVar::set("AWS_BEARER_TOKEN_BEDROCK", Some("env-bearer"));
    let mock = MockFetch::new(vec![converse_response(), converse_response()]);
    bedrock(AmazonBedrockProviderSettings {
        api_key: Some(Resolvable::Value("explicit".to_string())),
        ..sigv4(&mock)
    })
    .chat(MODEL)
    .do_generate(&user_prompt("hi"))
    .await
    .unwrap();
    assert_eq!(mock.seen()[0].headers["authorization"], "Bearer explicit");

    // A whitespace-only key is no key: the request is signed instead.
    bedrock(AmazonBedrockProviderSettings {
        api_key: Some(Resolvable::Value("   ".to_string())),
        ..sigv4(&mock)
    })
    .chat(MODEL)
    .do_generate(&user_prompt("hi"))
    .await
    .unwrap();
    assert_signed(&mock.seen()[1], "us-east-1");
}

#[serial]
#[tokio::test]
async fn embeddings_images_and_reranking_use_the_same_authentication() {
    let _env = clean_env();
    let mock = MockFetch::new(vec![
        Canned::json(&json!({ "embedding": [0.5], "inputTextTokenCount": 2 })),
        Canned::json(&json!({ "images": ["AAAA"] })),
        Canned::json(&json!({ "results": [{ "index": 0, "relevanceScore": 0.5 }] })),
    ]);
    let provider = bedrock(AmazonBedrockProviderSettings {
        api_key: Some(Resolvable::Value("k".to_string())),
        region: Some("eu-west-1".to_string()),
        fetch: Some(mock.transport()),
        ..Default::default()
    });
    provider
        .embedding("amazon.titan-embed-text-v2:0")
        .do_embed(&EmbeddingCallOptions {
            values: vec!["a".to_string()],
            abort_signal: None,
            max_retries: None,
            timeout: None,
            provider_options: None,
            headers: None,
        })
        .await
        .unwrap();
    provider
        .image("amazon.titan-image-generator-v1")
        .do_generate(&ImageCallOptions::new("a cat".to_string()))
        .await
        .unwrap();
    provider
        .reranking("cohere.rerank-v3-5:0")
        .do_rerank(&RerankingCallOptions {
            documents: RerankingDocuments::Text {
                values: vec!["a".to_string()],
            },
            query: "q".to_string(),
            top_n: Some(1),
            abort_signal: None,
            provider_options: None,
            headers: None,
            max_retries: None,
            timeout: None,
        })
        .await
        .unwrap();

    let seen = mock.seen();
    assert_eq!(
        seen[0].url,
        "https://bedrock-runtime.eu-west-1.amazonaws.com/model/amazon.titan-embed-text-v2:0/invoke"
    );
    assert_eq!(
        seen[1].url,
        "https://bedrock-runtime.eu-west-1.amazonaws.com/model/amazon.titan-image-generator-v1/invoke"
    );
    assert_eq!(
        seen[2].url,
        "https://bedrock-agent-runtime.eu-west-1.amazonaws.com/rerank"
    );
    for request in &seen {
        assert_eq!(request.headers["authorization"], "Bearer k");
    }
    // The model ARN carries the region too.
    assert_eq!(
        seen[2].json_body()["rerankingConfiguration"]["bedrockRerankingConfiguration"]["modelConfiguration"]
            ["modelArn"],
        "arn:aws:bedrock:eu-west-1::foundation-model/cohere.rerank-v3-5:0"
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

#[serial]
#[tokio::test]
async fn the_provider_level_body_rewrite_happens_before_signing() {
    let _env = clean_env();
    let mock = MockFetch::new(vec![converse_response()]);
    bedrock(AmazonBedrockProviderSettings {
        transform_request_body: Some(Arc::new(|mut body: Value| {
            body["requestMetadata"] = json!({ "team": "blue" });
            body
        })),
        ..sigv4(&mock)
    })
    .chat(MODEL)
    .do_generate(&user_prompt("hi"))
    .await
    .unwrap();
    let seen = mock.seen();
    assert_eq!(
        seen[0].json_body()["requestMetadata"],
        json!({ "team": "blue" })
    );
    assert_signed(&seen[0], "us-east-1");
}

#[serial]
#[tokio::test]
async fn a_session_token_is_signed_and_sent() {
    let _env = clean_env();
    // With both keys explicit, only the setting is used, never the variable.
    let _token = EnvVar::set("AWS_SESSION_TOKEN", Some("env-token"));
    let mock = MockFetch::new(vec![converse_response(), converse_response()]);
    bedrock(AmazonBedrockProviderSettings {
        session_token: Some("explicit-token".to_string()),
        ..sigv4(&mock)
    })
    .chat(MODEL)
    .do_generate(&user_prompt("hi"))
    .await
    .unwrap();
    let seen = mock.seen();
    assert_eq!(seen[0].headers["x-amz-security-token"], "explicit-token");
    assert!(seen[0].headers["authorization"].contains("x-amz-security-token"));

    // Without a setting, explicit keys get no token from the environment.
    bedrock(sigv4(&mock))
        .chat(MODEL)
        .do_generate(&user_prompt("hi"))
        .await
        .unwrap();
    assert!(!mock.seen()[1].headers.contains_key("x-amz-security-token"));
}

#[serial]
#[tokio::test]
async fn environment_credentials_and_region_are_read_per_request() {
    let _env = clean_env();
    let _key = EnvVar::set("AWS_ACCESS_KEY_ID", Some("AKIAIOSFODNN7EXAMPLE"));
    let _secret = EnvVar::set("AWS_SECRET_ACCESS_KEY", Some("env-secret"));
    let _region = EnvVar::set("AWS_REGION", Some("eu-west-1"));
    let _token = EnvVar::set("AWS_SESSION_TOKEN", Some("env-token"));
    let mock = MockFetch::new(vec![converse_response(), converse_response()]);
    let model = bedrock(AmazonBedrockProviderSettings {
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .chat(MODEL);

    model.do_generate(&user_prompt("one")).await.unwrap();
    unsafe { std::env::set_var("AWS_REGION", "ap-southeast-2") };
    model.do_generate(&user_prompt("two")).await.unwrap();

    let seen = mock.seen();
    assert!(
        seen[0]
            .url
            .starts_with("https://bedrock-runtime.eu-west-1.amazonaws.com/")
    );
    assert_signed(&seen[0], "eu-west-1");
    assert_eq!(seen[0].headers["x-amz-security-token"], "env-token");
    assert!(
        seen[1]
            .url
            .starts_with("https://bedrock-runtime.ap-southeast-2.amazonaws.com/")
    );
    assert_signed(&seen[1], "ap-southeast-2");
}

#[serial]
#[tokio::test]
async fn a_credential_provider_is_resolved_on_every_request() {
    let _env = clean_env();
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let mock = MockFetch::new(vec![converse_response(), converse_response()]);
    let model = bedrock(AmazonBedrockProviderSettings {
        region: Some("us-east-1".to_string()),
        // Ignored in favour of the provider.
        access_key_id: Some("static-key".to_string()),
        secret_access_key: Some("static-secret".to_string()),
        credential_provider: Some(Resolvable::from_async_fn(move || {
            let counted = counted.clone();
            async move {
                counted.fetch_add(1, Ordering::SeqCst);
                Ok(AwsCredentials {
                    access_key_id: "AKIAIOSFODNN7EXAMPLE".to_string(),
                    secret_access_key: "rotating".to_string(),
                    session_token: Some("sts".to_string()),
                    region: String::new(),
                })
            }
        })),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .chat(MODEL);
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "creation evaluates nothing"
    );

    model.do_generate(&user_prompt("one")).await.unwrap();
    model.do_generate(&user_prompt("two")).await.unwrap();

    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let seen = mock.seen();
    assert_signed(&seen[0], "us-east-1");
    assert_eq!(seen[0].headers["x-amz-security-token"], "sts");
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

#[serial]
#[tokio::test]
async fn the_runtime_endpoint_environment_variable_and_base_url_override_the_host() {
    let _env = clean_env();
    let _endpoint = EnvVar::set(
        "AWS_ENDPOINT_URL_BEDROCK_RUNTIME",
        Some("https://vpce.example/"),
    );
    let mock = MockFetch::new(vec![converse_response(), converse_response()]);

    // The endpoint variable replaces the regional host (no region needed).
    bedrock(AmazonBedrockProviderSettings {
        api_key: Some(Resolvable::Value("k".to_string())),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .chat(MODEL)
    .do_generate(&user_prompt("hi"))
    .await
    .unwrap();

    // An explicit base URL wins over the variable.
    bedrock(AmazonBedrockProviderSettings {
        api_key: Some(Resolvable::Value("k".to_string())),
        base_url: Some("https://gateway.example/bedrock/".to_string()),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .chat(MODEL)
    .do_generate(&user_prompt("hi"))
    .await
    .unwrap();

    let seen = mock.seen();
    assert_eq!(
        seen[0].url,
        "https://vpce.example/model/anthropic.claude-3-5-sonnet-20240620-v1:0/converse"
    );
    assert_eq!(
        seen[1].url,
        "https://gateway.example/bedrock/model/anthropic.claude-3-5-sonnet-20240620-v1:0/converse"
    );
}

#[serial]
#[tokio::test]
async fn missing_settings_fail_the_call_with_load_setting_naming_the_variable() {
    let _env = clean_env();
    let mock = MockFetch::new(vec![converse_response()]);

    // No region anywhere.
    let provider = bedrock(AmazonBedrockProviderSettings {
        fetch: Some(mock.transport()),
        ..Default::default()
    });
    let model = provider.chat(MODEL);
    match model.do_generate(&user_prompt("hi")).await.unwrap_err() {
        AiMuxError::LoadSetting { env_var, name } => {
            assert_eq!(env_var, "AWS_REGION");
            assert_eq!(name, "region");
        }
        other => panic!("expected LoadSetting, got {other:?}"),
    }

    // Region but no access key: with no API key either, signing needs one.
    let model = bedrock(AmazonBedrockProviderSettings {
        region: Some("us-east-1".to_string()),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .chat(MODEL);
    match model.do_generate(&user_prompt("hi")).await.unwrap_err() {
        AiMuxError::LoadSetting { env_var, name } => {
            assert_eq!(env_var, "AWS_ACCESS_KEY_ID");
            assert_eq!(name, "access_key_id");
        }
        other => panic!("expected LoadSetting, got {other:?}"),
    }

    // An access key but no secret.
    let model = bedrock(AmazonBedrockProviderSettings {
        region: Some("us-east-1".to_string()),
        access_key_id: Some("AKIAIOSFODNN7EXAMPLE".to_string()),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .chat(MODEL);
    let error = model.do_generate(&user_prompt("hi")).await.unwrap_err();
    match &error {
        AiMuxError::LoadSetting { env_var, name } => {
            assert_eq!(env_var, "AWS_SECRET_ACCESS_KEY");
            assert_eq!(name, "secret_access_key");
            assert!(!error.to_string().contains("AKIAIOSFODNN7EXAMPLE"));
        }
        other => panic!("expected LoadSetting, got {other:?}"),
    }
    assert!(mock.seen().is_empty(), "nothing was sent");
}

// ── discovery ────────────────────────────────────────────────────────────────

#[serial]
#[tokio::test]
async fn list_models_is_one_signed_exchange_through_the_same_transport() {
    let _env = clean_env();
    let mock = MockFetch::new(vec![Canned::json(&json!({
        "modelSummaries": [
            { "modelId": "amazon.nova-lite-v1:0", "modelName": "Nova Lite" },
            { "modelId": "amazon.nova-pro-v1:0" }
        ]
    }))]);
    let models = bedrock(sigv4(&mock)).list_models().await.unwrap();
    assert_eq!(
        models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        ["amazon.nova-lite-v1:0", "amazon.nova-pro-v1:0"]
    );
    let seen = mock.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "GET");
    // The control-plane host (`bedrock.`), not the runtime's and not the
    // `*.api.amazonaws.com` style.
    assert_eq!(
        seen[0].url,
        "https://bedrock.us-east-1.amazonaws.com/foundation-models"
    );
    assert_signed(&seen[0], "us-east-1");
}
