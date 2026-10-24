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
    assert!(
        error
            .to_string()
            .starts_with("AWS SigV4 authentication requires AWS credentials."),
        "{error:?}"
    );
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

/// The model's provider name is `anthropic-aws.messages` unless the settings
/// name it (anthropic-aws-provider.ts:246).
#[serial]
#[test]
fn the_default_provider_name_is_anthropic_aws_messages() {
    let provider = create_anthropic_aws(AnthropicAwsProviderSettings::default()).unwrap();
    assert_eq!(
        provider.messages("claude").provider(),
        "anthropic-aws.messages"
    );
    let named = create_anthropic_aws(AnthropicAwsProviderSettings {
        name: Some("my-aws".into()),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(named.messages("claude").provider(), "my-aws");
}

/// The Messages API fetches http(s) image and PDF URLs itself, so the model
/// reports them as supported (anthropic-aws-provider.ts:251-254).
#[serial]
#[tokio::test]
async fn models_support_http_image_and_pdf_urls() {
    let provider = create_anthropic_aws(AnthropicAwsProviderSettings::default()).unwrap();
    let urls = provider.messages("claude").supported_urls().await;
    let mut kinds: Vec<_> = urls.0.keys().map(String::as_str).collect();
    kinds.sort_unstable();
    assert_eq!(kinds, ["application/pdf", "image/*"]);
    assert!(urls.is_supported("https://example.com/a.png", "image/png"));
    assert!(urls.is_supported("http://example.com/a.pdf", "application/pdf"));
    assert!(!urls.is_supported("ftp://example.com/a.png", "image/png"));
}

/// A missing SigV4 key fails with upstream's guidance, the setting error
/// kept as the original error; nothing is sent
/// (anthropic-aws-provider.ts:183-209).
#[serial]
#[tokio::test]
async fn a_missing_sigv4_key_fails_with_guidance() {
    let _env = clean_environment();
    let settings = || AnthropicAwsProviderSettings {
        region: Some("us-west-2".into()),
        workspace_id: Some("ws".into()),
        ..Default::default()
    };

    let fetch = MockFetch::new(vec![]);
    let error = generate(&fetch, settings()).await.unwrap_err();
    let AiMuxError::Other { message, .. } = &error else {
        panic!("a missing access key is the guidance error: {error:?}");
    };
    assert!(
        message.starts_with(
            "AWS SigV4 authentication requires AWS credentials. Please provide either:\n"
        ),
        "{message}"
    );
    assert!(message.contains("\nOriginal error: "), "{message}");
    assert!(message.contains("AWS_ACCESS_KEY_ID"), "{message}");

    let _key_id = EnvVar::set("AWS_ACCESS_KEY_ID", Some("AKIDENV"));
    let error = generate(&fetch, settings()).await.unwrap_err();
    let AiMuxError::Other { message, .. } = &error else {
        panic!("a missing secret key is the guidance error: {error:?}");
    };
    assert!(
        message.starts_with(
            "AWS SigV4 authentication requires both AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY. \
             Please ensure both credentials are provided.\nOriginal error: "
        ),
        "{message}"
    );
    assert!(
        message.contains("AWS_SECRET_ACCESS_KEY` environment"),
        "{message}"
    );
    assert!(fetch.seen().is_empty());
}

/// A failing credential producer is reported with upstream's guidance and
/// nothing is sent (anthropic-aws-provider.ts:145-160).
#[serial]
#[tokio::test]
async fn a_failing_credential_producer_is_reported_with_guidance() {
    let _env = clean_environment();
    let fetch = MockFetch::new(vec![]);
    let error = generate(
        &fetch,
        AnthropicAwsProviderSettings {
            region: Some("us-west-2".into()),
            workspace_id: Some("ws".into()),
            auth: Some(AnthropicAwsAuth::SigV4(Resolvable::from_async_fn(
                || async { Err(AiMuxError::other("STS denied")) },
            ))),
            ..Default::default()
        },
    )
    .await
    .unwrap_err();
    assert!(
        error.to_string().contains(
            "AWS credential provider failed: STS denied. Please ensure your credential \
             provider returns valid AWS credentials with access_key_id and secret_access_key."
        ),
        "{error}"
    );
    assert!(fetch.seen().is_empty());
}

/// With no `fetch` setting each request uses the process default transport
/// current when it is made, not the one at creation: upstream reads
/// `globalThis.fetch` per call (anthropic-aws-fetch.ts:36, :124). The
/// default is pointed at a closed port while the provider is created, then at
/// a plain-HTTP proxy that answers the request.
#[serial]
#[tokio::test]
async fn the_default_transport_is_looked_up_per_request() {
    use aimux_provider_utils::{ProxyConfig, ReqwestFetch, set_default_transport};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    struct Restore;
    impl Drop for Restore {
        fn drop(&mut self) {
            set_default_transport(ReqwestFetch::default());
        }
    }
    let proxy = |port: u16| ProxyConfig {
        http_url: Some(format!("http://127.0.0.1:{port}")),
        ..ProxyConfig::default()
    };

    let _env = clean_environment();
    let _restore = Restore;
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    set_default_transport(ReqwestFetch::new(proxy(
        closed.local_addr().unwrap().port(),
    )));
    drop(closed);
    let provider = create_anthropic_aws(AnthropicAwsProviderSettings {
        base_url: Some("http://example.com/v1".into()),
        workspace_id: Some("ws".into()),
        auth: api_key(),
        ..Default::default()
    })
    .unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    set_default_transport(ReqwestFetch::new(proxy(
        listener.local_addr().unwrap().port(),
    )));
    let served = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut head = Vec::new();
        let mut chunk = [0u8; 4096];
        while !head.windows(4).any(|window| window == b"\r\n\r\n") {
            let n = stream.read(&mut chunk).await.unwrap();
            head.extend_from_slice(&chunk[..n]);
        }
        let body = json!({
            "id": "msg_1", "type": "message", "role": "assistant", "model": "claude",
            "content": [{"type": "text", "text": "ok"}],
            "stop_reason": "end_turn", "stop_sequence": null,
            "usage": {"input_tokens": 1, "output_tokens": 1}
        })
        .to_string();
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\
             connection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).await.unwrap();
        String::from_utf8_lossy(&head)
            .lines()
            .next()
            .unwrap()
            .to_string()
    });

    provider
        .messages("claude")
        .do_generate(&options())
        .await
        .expect("the request goes through the current default transport");
    assert_eq!(
        served.await.unwrap(),
        "POST http://example.com/v1/messages HTTP/1.1"
    );
}
