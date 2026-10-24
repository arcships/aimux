//! Anthropic-AWS settings and environment, against
//! `@ai-sdk/anthropic-aws` 2.0.63 src/anthropic-aws-provider.ts.
//!
//! Every test changes process environment variables, so all of them are
//! serial.

#[path = "common/mock_fetch.rs"]
mod mock_fetch;

use std::sync::Arc;

use serde_json::json;
use serial_test::serial;

use aimux_core::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::LanguageModelMessage;
use aimux_core::options::CallOptions;
use aimux_provider_utils::{AwsCredentials, Resolvable};
use aimux_providers::{AnthropicAwsAuth, AnthropicAwsProviderSettings, create_anthropic_aws};
use mock_fetch::{Canned, EnvVar, MockFetch};

const VARIABLES: [&str; 6] = [
    "AWS_REGION",
    "ANTHROPIC_AWS_API_KEY",
    "ANTHROPIC_AWS_WORKSPACE_ID",
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
];

/// Clears every variable the provider reads; restored on drop.
fn clean_environment() -> Vec<EnvVar> {
    VARIABLES
        .iter()
        .map(|name| EnvVar::set(name, None))
        .collect()
}

fn message() -> Canned {
    Canned::json(&json!({
        "id": "msg_1", "type": "message", "role": "assistant", "model": "claude",
        "content": [{"type": "text", "text": "ok"}],
        "stop_reason": "end_turn", "stop_sequence": null,
        "usage": {"input_tokens": 1, "output_tokens": 1}
    }))
}

fn options() -> CallOptions {
    CallOptions::new(vec![LanguageModelMessage::user_text("hi")])
}

fn api_key() -> Option<AnthropicAwsAuth> {
    Some(AnthropicAwsAuth::ApiKey(Resolvable::Value("key".into())))
}

async fn generate(
    fetch: &Arc<MockFetch>,
    settings: AnthropicAwsProviderSettings,
) -> Result<(), AiMuxError> {
    create_anthropic_aws(AnthropicAwsProviderSettings {
        fetch: Some(fetch.transport()),
        ..settings
    })?
    .messages("claude")
    .do_generate(&options())
    .await
    .map(|_| ())
}

fn is_missing(error: &AiMuxError, variable: &str) -> bool {
    matches!(error, AiMuxError::LoadSetting { env_var, .. } if env_var == variable)
}

/// The region comes from the setting or `AWS_REGION` and has no default
/// (`getRegion`, anthropic-aws-provider.ts:212-227; `getBaseURL`, :229-231).
#[serial]
#[tokio::test]
async fn region_comes_from_the_setting_or_aws_region_with_no_default() {
    let _env = clean_environment();
    let settings = || AnthropicAwsProviderSettings {
        workspace_id: Some("ws".into()),
        auth: api_key(),
        ..Default::default()
    };

    let fetch = MockFetch::new(vec![]);
    let error = generate(&fetch, settings()).await.unwrap_err();
    assert!(is_missing(&error, "AWS_REGION"), "{error:?}");
    assert!(fetch.seen().is_empty());

    let _region = EnvVar::set("AWS_REGION", Some("eu-west-3"));
    let fetch = MockFetch::new(vec![message(), message()]);
    generate(&fetch, settings()).await.unwrap();
    generate(
        &fetch,
        AnthropicAwsProviderSettings {
            region: Some("ap-south-1".into()),
            ..settings()
        },
    )
    .await
    .unwrap();
    let seen = fetch.seen();
    assert_eq!(
        seen[0].url,
        "https://aws-external-anthropic.eu-west-3.api.aws/v1/messages"
    );
    assert_eq!(
        seen[1].url,
        "https://aws-external-anthropic.ap-south-1.api.aws/v1/messages"
    );

    let _region = EnvVar::set("AWS_REGION", Some("bad.region"));
    let fetch = MockFetch::new(vec![]);
    let error = generate(&fetch, settings()).await.unwrap_err();
    assert!(
        matches!(&error, AiMuxError::InvalidArgument { message, .. } if message.contains("Invalid AWS region")),
        "{error:?}"
    );
    assert!(fetch.seen().is_empty());
}

/// The workspace id is required, from the setting or
/// `ANTHROPIC_AWS_WORKSPACE_ID` (`getHeaders`, anthropic-aws-provider.ts:233-242).
#[serial]
#[tokio::test]
async fn workspace_id_comes_from_the_setting_or_the_environment() {
    let _env = clean_environment();
    let settings = |workspace_id: Option<&str>| AnthropicAwsProviderSettings {
        region: Some("us-east-1".into()),
        workspace_id: workspace_id.map(String::from),
        auth: api_key(),
        ..Default::default()
    };

    let fetch = MockFetch::new(vec![]);
    let error = generate(&fetch, settings(None)).await.unwrap_err();
    assert!(
        is_missing(&error, "ANTHROPIC_AWS_WORKSPACE_ID"),
        "{error:?}"
    );
    assert!(fetch.seen().is_empty());

    let _workspace = EnvVar::set("ANTHROPIC_AWS_WORKSPACE_ID", Some("ws-env"));
    let fetch = MockFetch::new(vec![message(), message()]);
    generate(&fetch, settings(None)).await.unwrap();
    generate(&fetch, settings(Some("ws-setting")))
        .await
        .unwrap();
    let seen = fetch.seen();
    assert_eq!(seen[0].headers["anthropic-workspace-id"], "ws-env");
    assert_eq!(seen[1].headers["anthropic-workspace-id"], "ws-setting");
}

/// With no `auth`, `ANTHROPIC_AWS_API_KEY` selects `x-api-key`; without it the
/// request is signed with the `AWS_*` credentials
/// (anthropic-aws-provider.ts:130-210).
#[serial]
#[tokio::test]
async fn default_auth_is_the_api_key_variable_else_sigv4_from_the_environment() {
    let _env = clean_environment();
    let settings = || AnthropicAwsProviderSettings {
        region: Some("us-west-2".into()),
        workspace_id: Some("ws".into()),
        ..Default::default()
    };

    let fetch = MockFetch::new(vec![]);
    let error = generate(&fetch, settings()).await.unwrap_err();
    assert!(is_missing(&error, "AWS_ACCESS_KEY_ID"), "{error:?}");
    assert!(fetch.seen().is_empty());

    let _key_id = EnvVar::set("AWS_ACCESS_KEY_ID", Some("AKIDENV"));
    let _secret = EnvVar::set("AWS_SECRET_ACCESS_KEY", Some("secret"));
    let _token = EnvVar::set("AWS_SESSION_TOKEN", Some("token"));
    let fetch = MockFetch::new(vec![message()]);
    generate(&fetch, settings()).await.unwrap();
    let signed = &fetch.seen()[0];
    assert!(!signed.headers.contains_key("x-api-key"));
    assert!(
        signed.headers["authorization"].starts_with("AWS4-HMAC-SHA256 Credential=AKIDENV/"),
        "{}",
        signed.headers["authorization"]
    );
    assert!(
        signed.headers["authorization"].contains("/us-west-2/aws-external-anthropic/aws4_request")
    );
    assert_eq!(signed.headers["x-amz-security-token"], "token");

    let _api_key = EnvVar::set("ANTHROPIC_AWS_API_KEY", Some("env-key"));
    let fetch = MockFetch::new(vec![message()]);
    generate(&fetch, settings()).await.unwrap();
    let keyed = &fetch.seen()[0];
    assert_eq!(keyed.headers["x-api-key"], "env-key");
    assert!(!keyed.headers.contains_key("authorization"));
}

/// The region of the setting or `AWS_REGION` signs, over the region the
/// credentials carry: upstream spreads the loaded region over the credential
/// provider's result (anthropic-aws-provider.ts:145-150).
#[serial]
#[tokio::test]
async fn the_region_setting_signs_over_the_credentials_region() {
    let _env = clean_environment();
    let fetch = MockFetch::new(vec![message()]);
    generate(
        &fetch,
        AnthropicAwsProviderSettings {
            region: Some("eu-central-1".into()),
            workspace_id: Some("ws".into()),
            auth: Some(AnthropicAwsAuth::SigV4(Resolvable::Value(AwsCredentials {
                access_key_id: "AKID".into(),
                secret_access_key: "secret".into(),
                session_token: None,
                region: "us-east-1".into(),
            }))),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let seen = &fetch.seen()[0];
    assert_eq!(
        seen.url,
        "https://aws-external-anthropic.eu-central-1.api.aws/v1/messages"
    );
    assert!(
        seen.headers["authorization"].contains("/eu-central-1/aws-external-anthropic/aws4_request")
    );
}
