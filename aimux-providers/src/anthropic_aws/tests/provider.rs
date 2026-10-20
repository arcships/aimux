//! Port of `@ai-sdk/anthropic-aws` 2.0.63 `src/anthropic-aws-provider.test.ts`.
//!
//! Every test changes process environment variables, so all of them are
//! serial. `vi.mock('aws4fetch')` has no counterpart: SigV4 signing runs for
//! real, so its assertions check the shape of the signature instead of the
//! mocked `Credential=test` value. The clock is not injectable through the
//! provider settings, so `x-amz-date` is checked for format only.
//!
//! The older settings and environment ports (formerly
//! `tests/anthropic_aws_settings_test.rs`) follow their harness helpers at the
//! end, on the same [`MockFetch`].

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use aimux_core::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::LanguageModelMessage;
use aimux_core::options::CallOptions;
use aimux_core::provider::Provider;
use aimux_core::stream_part::StreamPart;
use aimux_provider_utils::{AwsCredentials, Resolvable};
use futures::StreamExt;
use regex::Regex;
use serde_json::json;
use serial_test::serial;

use crate::anthropic_aws::AnthropicAwsProvider;
use crate::test_support::mock_fetch::{Canned, EnvVar, MockFetch, Seen};
use crate::{AnthropicAwsAuth, AnthropicAwsProviderSettings, create_anthropic_aws};

/// `TEST_PROMPT`: one user message with the text `Hello`.
fn prompt() -> CallOptions {
    CallOptions::new(vec![LanguageModelMessage::user_text("Hello")])
}

const MODEL: &str = "claude-sonnet-4-6";

/// `createSuccessfulResponse()`.
fn successful_response() -> Canned {
    Canned::json(&json!({
        "type": "message",
        "id": "msg_123",
        "model": "claude-sonnet-4-6",
        "content": [{"type": "text", "text": "Hi"}],
        "stop_reason": null,
        "stop_sequence": null,
        "usage": {"input_tokens": 1, "output_tokens": 1},
    }))
}

/// `stubDefaultEnv()`: region `us-west-2`, a default workspace, and every
/// other variable the provider reads unset. Restored on drop.
fn stub_default_env() -> Vec<EnvVar> {
    vec![
        EnvVar::set("AWS_REGION", Some("us-west-2")),
        EnvVar::set("ANTHROPIC_AWS_WORKSPACE_ID", Some("wrkspc_default")),
        EnvVar::set("ANTHROPIC_AWS_API_KEY", None),
        EnvVar::set("AWS_ACCESS_KEY_ID", None),
        EnvVar::set("AWS_SECRET_ACCESS_KEY", None),
        EnvVar::set("AWS_SESSION_TOKEN", None),
    ]
}

fn api_key(key: &str) -> AnthropicAwsAuth {
    AnthropicAwsAuth::ApiKey(Resolvable::Value(key.to_string()))
}

/// A request header by its lower-cased name.
fn header<'a>(request: &'a Seen, name: &str) -> Option<&'a str> {
    request.headers.get(name).map(String::as_str)
}

/// `rejects.toThrow(/pattern/)`: the message matches the regex.
fn assert_message(error: &AiMuxError, pattern: &str) {
    let regex = Regex::new(pattern).expect("valid pattern");
    let message = error.to_string();
    assert!(
        regex.is_match(&message),
        "error `{message}` does not match /{pattern}/"
    );
}

/// The error of a `doGenerate` call that must fail.
async fn generate_error(model: Arc<dyn LanguageModel>) -> AiMuxError {
    model
        .do_generate(&prompt())
        .await
        .expect_err("doGenerate was expected to fail")
}

/// The error of a model or embedding/image model lookup that must fail.
fn lookup_error<T: ?Sized>(result: Result<Arc<T>, AiMuxError>) -> AiMuxError {
    match result {
        Ok(_) => panic!("the lookup was expected to fail"),
        Err(error) => error,
    }
}

// ── The harness of the older settings ports ───────────────────────────────────

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

fn key_auth() -> Option<AnthropicAwsAuth> {
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

mod create_anthropic_aws {
    use super::*;

    mod base_url_configuration {
        use super::*;

        /// upstream: @ai-sdk/anthropic-aws src/anthropic-aws-provider.test.ts › createAnthropicAws › baseURL configuration › uses the default Claude Platform on AWS base URL with region templating
        #[serial]
        #[tokio::test]
        async fn uses_the_default_claude_platform_on_aws_base_url_with_region_templating() {
            let _env = stub_default_env();
            let fetch = MockFetch::new(vec![successful_response()]);
            let provider = create_anthropic_aws(AnthropicAwsProviderSettings {
                region: Some("us-east-1".into()),
                workspace_id: Some("wrkspc_test".into()),
                auth: Some(api_key("test-api-key")),
                fetch: Some(fetch.transport()),
                ..Default::default()
            })
            .unwrap();

            provider.call(MODEL).do_generate(&prompt()).await.unwrap();

            let seen = fetch.seen();
            assert_eq!(seen.len(), 1);
            assert_eq!(
                seen[0].url,
                "https://aws-external-anthropic.us-east-1.api.aws/v1/messages"
            );
        }

        /// upstream: @ai-sdk/anthropic-aws src/anthropic-aws-provider.test.ts › createAnthropicAws › baseURL configuration › reads AWS_REGION from the environment when region option is omitted
        #[serial]
        #[tokio::test]
        async fn reads_aws_region_from_the_environment_when_region_option_is_omitted() {
            let _env = stub_default_env();
            let _region = EnvVar::set("AWS_REGION", Some("eu-west-1"));
            let fetch = MockFetch::new(vec![successful_response()]);
            let provider = create_anthropic_aws(AnthropicAwsProviderSettings {
                workspace_id: Some("wrkspc_test".into()),
                auth: Some(api_key("test-api-key")),
                fetch: Some(fetch.transport()),
                ..Default::default()
            })
            .unwrap();

            provider.call(MODEL).do_generate(&prompt()).await.unwrap();

            assert_eq!(
                fetch.seen()[0].url,
                "https://aws-external-anthropic.eu-west-1.api.aws/v1/messages"
            );
        }

        /// upstream: @ai-sdk/anthropic-aws src/anthropic-aws-provider.test.ts › createAnthropicAws › baseURL configuration › prefers the baseURL option over the default template
        #[serial]
        #[tokio::test]
        async fn prefers_the_base_url_option_over_the_default_template() {
            let _env = stub_default_env();
            let fetch = MockFetch::new(vec![successful_response()]);
            let provider = create_anthropic_aws(AnthropicAwsProviderSettings {
                region: Some("us-west-2".into()),
                workspace_id: Some("wrkspc_test".into()),
                auth: Some(api_key("test-api-key")),
                base_url: Some("https://proxy.example.com/v1/".into()),
                fetch: Some(fetch.transport()),
                ..Default::default()
            })
            .unwrap();

            provider.call(MODEL).do_generate(&prompt()).await.unwrap();

            assert_eq!(fetch.seen()[0].url, "https://proxy.example.com/v1/messages");
        }
    }
}

mod anthropic_aws_provider_authentication {
    use super::*;

    mod api_key_option {
        use super::*;

        /// upstream: @ai-sdk/anthropic-aws src/anthropic-aws-provider.test.ts › anthropicAws provider - authentication › apiKey option › sends x-api-key header when apiKey is provided
        #[serial]
        #[tokio::test]
        async fn sends_x_api_key_header_when_api_key_is_provided() {
            let _env = stub_default_env();
            let fetch = MockFetch::new(vec![successful_response()]);
            let provider = create_anthropic_aws(AnthropicAwsProviderSettings {
                region: Some("us-west-2".into()),
                workspace_id: Some("wrkspc_test".into()),
                auth: Some(api_key("sk-aws-platform-key")),
                fetch: Some(fetch.transport()),
                ..Default::default()
            })
            .unwrap();

            provider.call(MODEL).do_generate(&prompt()).await.unwrap();

            let request = fetch.seen().remove(0);
            assert_eq!(header(&request, "x-api-key"), Some("sk-aws-platform-key"));
            assert_eq!(header(&request, "authorization"), None);
        }

        /// upstream: @ai-sdk/anthropic-aws src/anthropic-aws-provider.test.ts › anthropicAws provider - authentication › apiKey option › reads apiKey from ANTHROPIC_AWS_API_KEY when option is omitted
        #[serial]
        #[tokio::test]
        async fn reads_api_key_from_anthropic_aws_api_key_when_option_is_omitted() {
            let _env = stub_default_env();
            let _key = EnvVar::set("ANTHROPIC_AWS_API_KEY", Some("sk-from-env"));
            let fetch = MockFetch::new(vec![successful_response()]);
            let provider = create_anthropic_aws(AnthropicAwsProviderSettings {
                region: Some("us-west-2".into()),
                workspace_id: Some("wrkspc_test".into()),
                fetch: Some(fetch.transport()),
                ..Default::default()
            })
            .unwrap();

            provider.call(MODEL).do_generate(&prompt()).await.unwrap();

            let request = fetch.seen().remove(0);
            assert_eq!(header(&request, "x-api-key"), Some("sk-from-env"));
        }
    }

    mod sig_v4_path {
        use super::*;

        /// upstream: @ai-sdk/anthropic-aws src/anthropic-aws-provider.test.ts › anthropicAws provider - authentication › SigV4 path › signs requests with SigV4 when apiKey is not provided
        #[serial]
        #[tokio::test]
        async fn signs_requests_with_sig_v4_when_api_key_is_not_provided() {
            let _env = stub_default_env();
            let fetch = MockFetch::new(vec![successful_response()]);
            let provider = create_anthropic_aws(AnthropicAwsProviderSettings {
                region: Some("us-west-2".into()),
                workspace_id: Some("wrkspc_test".into()),
                auth: Some(AnthropicAwsAuth::SigV4(Resolvable::Value(AwsCredentials {
                    access_key_id: "akid".into(),
                    secret_access_key: "secret".into(),
                    session_token: None,
                    region: "us-west-2".into(),
                }))),
                fetch: Some(fetch.transport()),
                ..Default::default()
            })
            .unwrap();

            provider.call(MODEL).do_generate(&prompt()).await.unwrap();

            let request = fetch.seen().remove(0);
            let authorization = header(&request, "authorization").expect("the request is signed");
            assert!(
                authorization.starts_with("AWS4-HMAC-SHA256 Credential=akid/"),
                "{authorization}"
            );
            assert!(
                authorization.contains("/us-west-2/aws-external-anthropic/aws4_request"),
                "{authorization}"
            );
            let amz_date = header(&request, "x-amz-date").expect("the request is signed");
            assert!(
                Regex::new(r"^\d{8}T\d{6}Z$").unwrap().is_match(amz_date),
                "{amz_date}"
            );
            assert_eq!(header(&request, "x-api-key"), None);
        }

        /// upstream: @ai-sdk/anthropic-aws src/anthropic-aws-provider.test.ts › anthropicAws provider - authentication › SigV4 path › honors a credentialProvider for dynamic SigV4 credentials
        #[serial]
        #[tokio::test]
        async fn honors_a_credential_provider_for_dynamic_sig_v4_credentials() {
            let _env = stub_default_env();
            let calls = Arc::new(AtomicUsize::new(0));
            let counter = calls.clone();
            let credentials = Resolvable::from_async_fn(move || {
                let counter = counter.clone();
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    Ok(AwsCredentials {
                        access_key_id: "dynamic-akid".into(),
                        secret_access_key: "dynamic-secret".into(),
                        session_token: Some("dynamic-session".into()),
                        region: "us-west-2".into(),
                    })
                }
            });
            let fetch = MockFetch::new(vec![successful_response()]);
            let provider = create_anthropic_aws(AnthropicAwsProviderSettings {
                region: Some("us-west-2".into()),
                workspace_id: Some("wrkspc_test".into()),
                auth: Some(AnthropicAwsAuth::SigV4(credentials)),
                fetch: Some(fetch.transport()),
                ..Default::default()
            })
            .unwrap();

            provider.call(MODEL).do_generate(&prompt()).await.unwrap();

            assert!(
                calls.load(Ordering::SeqCst) > 0,
                "credential provider not called"
            );
        }

        /// upstream: @ai-sdk/anthropic-aws src/anthropic-aws-provider.test.ts › anthropicAws provider - authentication › SigV4 path › throws a guided error when SigV4 credentials are missing
        #[serial]
        #[tokio::test]
        async fn throws_a_guided_error_when_sig_v4_credentials_are_missing() {
            let _env = stub_default_env();
            let fetch = MockFetch::new(vec![]);
            let provider = create_anthropic_aws(AnthropicAwsProviderSettings {
                region: Some("us-west-2".into()),
                workspace_id: Some("wrkspc_test".into()),
                fetch: Some(fetch.transport()),
                ..Default::default()
            })
            .unwrap();

            let error = generate_error(provider.call(MODEL)).await;

            assert_message(&error, "AWS SigV4 authentication requires AWS credentials");
        }

        /// upstream: @ai-sdk/anthropic-aws src/anthropic-aws-provider.test.ts › anthropicAws provider - authentication › SigV4 path › wraps credentialProvider rejections with a guided message
        #[serial]
        #[tokio::test]
        async fn wraps_credential_provider_rejections_with_a_guided_message() {
            let _env = stub_default_env();
            let credentials = Resolvable::from_async_fn(|| async {
                Err::<AwsCredentials, _>(AiMuxError::Other("STS denied".into()))
            });
            let fetch = MockFetch::new(vec![]);
            let provider = create_anthropic_aws(AnthropicAwsProviderSettings {
                region: Some("us-west-2".into()),
                workspace_id: Some("wrkspc_test".into()),
                auth: Some(AnthropicAwsAuth::SigV4(credentials)),
                fetch: Some(fetch.transport()),
                ..Default::default()
            })
            .unwrap();

            let error = generate_error(provider.call(MODEL)).await;

            assert_message(&error, "AWS credential provider failed: STS denied");
        }
    }
}

mod anthropic_aws_provider_workspace_id {
    use super::*;

    /// upstream: @ai-sdk/anthropic-aws src/anthropic-aws-provider.test.ts › anthropicAws provider - workspaceId › sends the anthropic-version header on every request
    #[serial]
    #[tokio::test]
    async fn sends_the_anthropic_version_header_on_every_request() {
        let _env = stub_default_env();
        let fetch = MockFetch::new(vec![successful_response()]);
        let provider = create_anthropic_aws(AnthropicAwsProviderSettings {
            region: Some("us-west-2".into()),
            workspace_id: Some("wrkspc_test".into()),
            auth: Some(api_key("test-api-key")),
            fetch: Some(fetch.transport()),
            ..Default::default()
        })
        .unwrap();

        provider.call(MODEL).do_generate(&prompt()).await.unwrap();

        let request = fetch.seen().remove(0);
        assert_eq!(header(&request, "anthropic-version"), Some("2023-06-01"));
    }

    /// upstream: @ai-sdk/anthropic-aws src/anthropic-aws-provider.test.ts › anthropicAws provider - workspaceId › sends the anthropic-workspace-id header on every request
    #[serial]
    #[tokio::test]
    async fn sends_the_anthropic_workspace_id_header_on_every_request() {
        let _env = stub_default_env();
        let fetch = MockFetch::new(vec![successful_response()]);
        let provider = create_anthropic_aws(AnthropicAwsProviderSettings {
            region: Some("us-west-2".into()),
            workspace_id: Some("wrkspc_unique".into()),
            auth: Some(api_key("test-api-key")),
            fetch: Some(fetch.transport()),
            ..Default::default()
        })
        .unwrap();

        provider.call(MODEL).do_generate(&prompt()).await.unwrap();

        let request = fetch.seen().remove(0);
        assert_eq!(
            header(&request, "anthropic-workspace-id"),
            Some("wrkspc_unique")
        );
    }

    /// upstream: @ai-sdk/anthropic-aws src/anthropic-aws-provider.test.ts › anthropicAws provider - workspaceId › reads workspaceId from ANTHROPIC_AWS_WORKSPACE_ID when option is omitted
    #[serial]
    #[tokio::test]
    async fn reads_workspace_id_from_anthropic_aws_workspace_id_when_option_is_omitted() {
        let _env = stub_default_env();
        let _workspace = EnvVar::set("ANTHROPIC_AWS_WORKSPACE_ID", Some("wrkspc_from_env"));
        let fetch = MockFetch::new(vec![successful_response()]);
        let provider = create_anthropic_aws(AnthropicAwsProviderSettings {
            region: Some("us-west-2".into()),
            auth: Some(api_key("test-api-key")),
            fetch: Some(fetch.transport()),
            ..Default::default()
        })
        .unwrap();

        provider.call(MODEL).do_generate(&prompt()).await.unwrap();

        let request = fetch.seen().remove(0);
        assert_eq!(
            header(&request, "anthropic-workspace-id"),
            Some("wrkspc_from_env")
        );
    }

    /// upstream: @ai-sdk/anthropic-aws src/anthropic-aws-provider.test.ts › anthropicAws provider - workspaceId › throws when workspaceId is not resolvable at request time
    #[serial]
    #[tokio::test]
    async fn throws_when_workspace_id_is_not_resolvable_at_request_time() {
        let _env = stub_default_env();
        let _workspace = EnvVar::set("ANTHROPIC_AWS_WORKSPACE_ID", None);
        let fetch = MockFetch::new(vec![successful_response()]);
        let provider = create_anthropic_aws(AnthropicAwsProviderSettings {
            region: Some("us-west-2".into()),
            auth: Some(api_key("test-api-key")),
            fetch: Some(fetch.transport()),
            ..Default::default()
        })
        .unwrap();

        let error = generate_error(provider.call(MODEL)).await;

        assert_message(&error, "(?i)workspace");
    }
}

mod anthropic_aws_provider_headers {
    use super::*;

    /// upstream: @ai-sdk/anthropic-aws src/anthropic-aws-provider.test.ts › anthropicAws provider - headers › merges custom headers with the workspace-id header
    #[serial]
    #[tokio::test]
    async fn merges_custom_headers_with_the_workspace_id_header() {
        let _env = stub_default_env();
        let fetch = MockFetch::new(vec![successful_response()]);
        let provider = create_anthropic_aws(AnthropicAwsProviderSettings {
            region: Some("us-west-2".into()),
            workspace_id: Some("wrkspc_test".into()),
            auth: Some(api_key("test-api-key")),
            headers: Some(
                [("x-custom".to_string(), Some("value".to_string()))]
                    .into_iter()
                    .collect(),
            ),
            fetch: Some(fetch.transport()),
            ..Default::default()
        })
        .unwrap();

        provider.call(MODEL).do_generate(&prompt()).await.unwrap();

        let request = fetch.seen().remove(0);
        assert_eq!(
            header(&request, "anthropic-workspace-id"),
            Some("wrkspc_test")
        );
        assert_eq!(header(&request, "x-custom"), Some("value"));
    }
}

mod anthropic_aws_provider_supported_urls {
    use super::*;

    /// upstream: @ai-sdk/anthropic-aws src/anthropic-aws-provider.test.ts › anthropicAws provider - supportedUrls › should support image/* URLs
    #[serial]
    #[tokio::test]
    async fn should_support_image_urls() {
        let _env = stub_default_env();
        let provider = create_anthropic_aws(AnthropicAwsProviderSettings {
            region: Some("us-west-2".into()),
            workspace_id: Some("wrkspc_test".into()),
            auth: Some(api_key("test-api-key")),
            ..Default::default()
        })
        .unwrap();

        let model = provider.call(MODEL);
        let supported_urls = model.supported_urls().await;

        let patterns = supported_urls
            .0
            .get("image/*")
            .expect("image/* URLs are supported");
        assert!(patterns[0].is_match("https://example.com/image.png"));
    }

    /// upstream: @ai-sdk/anthropic-aws src/anthropic-aws-provider.test.ts › anthropicAws provider - supportedUrls › should support application/pdf URLs
    #[serial]
    #[tokio::test]
    async fn should_support_application_pdf_urls() {
        let _env = stub_default_env();
        let provider = create_anthropic_aws(AnthropicAwsProviderSettings {
            region: Some("us-west-2".into()),
            workspace_id: Some("wrkspc_test".into()),
            auth: Some(api_key("test-api-key")),
            ..Default::default()
        })
        .unwrap();

        let model = provider.call(MODEL);
        let supported_urls = model.supported_urls().await;

        let patterns = supported_urls
            .0
            .get("application/pdf")
            .expect("application/pdf URLs are supported");
        assert!(patterns[0].is_match("https://arxiv.org/pdf/2401.00001"));
    }
}

mod anthropic_aws_provider_model_identity {
    use super::*;

    fn provider() -> AnthropicAwsProvider {
        create_anthropic_aws(AnthropicAwsProviderSettings {
            region: Some("us-west-2".into()),
            workspace_id: Some("wrkspc_test".into()),
            auth: Some(api_key("test-api-key")),
            ..Default::default()
        })
        .unwrap()
    }

    /// upstream: @ai-sdk/anthropic-aws src/anthropic-aws-provider.test.ts › anthropicAws provider - model identity › sets the provider name to anthropic-aws.messages on the model
    #[serial]
    #[test]
    fn sets_the_provider_name_to_anthropic_aws_messages_on_the_model() {
        let _env = stub_default_env();

        let model = provider().call(MODEL);

        assert_eq!(model.provider(), "anthropic-aws.messages");
    }

    /// upstream: @ai-sdk/anthropic-aws src/anthropic-aws-provider.test.ts › anthropicAws provider - model identity › throws NoSuchModelError when embeddingModel is invoked
    ///
    /// Upstream matches the message `No such embeddingModel: …`; aimux-core formats
    /// `NoSuchModel` as `no such model: …`, a core difference ported with the
    /// core tests, so the variant and its `model_type` are asserted instead.
    #[serial]
    #[test]
    fn throws_no_such_model_error_when_embedding_model_is_invoked() {
        let _env = stub_default_env();

        let error = lookup_error(provider().embedding_model("any-model-id"));

        assert!(
            matches!(&error, AiMuxError::NoSuchModel { model_type, .. } if model_type == "embeddingModel"),
            "{error:?}"
        );
    }

    /// upstream: @ai-sdk/anthropic-aws src/anthropic-aws-provider.test.ts › anthropicAws provider - model identity › throws NoSuchModelError when imageModel is invoked
    ///
    /// Upstream matches the message `No such imageModel: …`; aimux-core formats
    /// `NoSuchModel` as `no such model: …`, a core difference ported with the
    /// core tests, so the variant and its `model_type` are asserted instead.
    #[serial]
    #[test]
    fn throws_no_such_model_error_when_image_model_is_invoked() {
        let _env = stub_default_env();

        let error = lookup_error(provider().image_model("any-model-id"));

        assert!(
            matches!(&error, AiMuxError::NoSuchModel { model_type, .. } if model_type == "imageModel"),
            "{error:?}"
        );
    }
}

mod anthropic_aws_provider_auth_precedence {
    use super::*;

    /// upstream: @ai-sdk/anthropic-aws src/anthropic-aws-provider.test.ts › anthropicAws provider - auth precedence › prefers the API-key path when both apiKey and AWS SigV4 creds are present
    #[serial]
    #[tokio::test]
    async fn prefers_the_api_key_path_when_both_api_key_and_aws_sig_v4_creds_are_present() {
        let _env = stub_default_env();
        let _key_id = EnvVar::set("AWS_ACCESS_KEY_ID", Some("should-be-ignored"));
        let _secret = EnvVar::set("AWS_SECRET_ACCESS_KEY", Some("should-be-ignored"));
        let fetch = MockFetch::new(vec![successful_response()]);
        let provider = create_anthropic_aws(AnthropicAwsProviderSettings {
            region: Some("us-west-2".into()),
            workspace_id: Some("wrkspc_test".into()),
            auth: Some(api_key("sk-aws-platform-key")),
            fetch: Some(fetch.transport()),
            ..Default::default()
        })
        .unwrap();

        provider.call(MODEL).do_generate(&prompt()).await.unwrap();

        let request = fetch.seen().remove(0);
        assert_eq!(header(&request, "x-api-key"), Some("sk-aws-platform-key"));
        assert_eq!(header(&request, "authorization"), None);
        assert_eq!(header(&request, "x-amz-date"), None);
    }
}

mod anthropic_aws_provider_streaming {
    use super::*;

    /// The upstream SSE body: its lines joined with `\n`, the last one empty.
    const SSE_LINES: &[&str] = &[
        "event: message_start",
        r#"data: {"type":"message_start","message":{"id":"msg_1","model":"claude-sonnet-4-6","role":"assistant","content":[],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1,"output_tokens":0}}}"#,
        "",
        "event: content_block_start",
        r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
        "",
        "event: content_block_delta",
        r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hi"}}"#,
        "",
        "event: content_block_stop",
        r#"data: {"type":"content_block_stop","index":0}"#,
        "",
        "event: message_delta",
        r#"data: {"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":1}}"#,
        "",
        "event: message_stop",
        r#"data: {"type":"message_stop"}"#,
        "",
    ];

    /// upstream: @ai-sdk/anthropic-aws src/anthropic-aws-provider.test.ts › anthropicAws provider - streaming › forwards doStream through the SigV4 / api-key fetch wrapper and yields stream events
    #[serial]
    #[tokio::test]
    async fn forwards_do_stream_through_the_fetch_wrapper_and_yields_stream_events() {
        let _env = stub_default_env();
        let fetch = MockFetch::new(vec![Canned {
            status: 200,
            headers: vec![("content-type".into(), "text/event-stream".into())],
            body: SSE_LINES.join("\n").into_bytes(),
        }]);
        let provider = create_anthropic_aws(AnthropicAwsProviderSettings {
            region: Some("us-west-2".into()),
            workspace_id: Some("wrkspc_test".into()),
            auth: Some(api_key("test-api-key")),
            fetch: Some(fetch.transport()),
            ..Default::default()
        })
        .unwrap();

        let stream = provider
            .call(MODEL)
            .do_stream(&prompt())
            .await
            .unwrap()
            .stream;
        let parts: Vec<StreamPart> = stream.map(|part| part.unwrap()).collect().await;

        assert!(!parts.is_empty());
        assert!(
            parts
                .iter()
                .any(|part| matches!(part, StreamPart::TextDelta { .. })),
            "no text-delta in {parts:?}"
        );

        // The same workspace and version headers as doGenerate, and the key.
        let request = fetch.seen().remove(0);
        assert_eq!(header(&request, "anthropic-version"), Some("2023-06-01"));
        assert_eq!(
            header(&request, "anthropic-workspace-id"),
            Some("wrkspc_test")
        );
        assert_eq!(header(&request, "x-api-key"), Some("test-api-key"));
    }
}

/// The region comes from the setting or `AWS_REGION` and has no default
/// (`getRegion`, anthropic-aws-provider.ts:212-227; `getBaseURL`, :229-231).
#[serial]
#[tokio::test]
async fn region_comes_from_the_setting_or_aws_region_with_no_default() {
    let _env = clean_environment();
    let settings = || AnthropicAwsProviderSettings {
        workspace_id: Some("ws".into()),
        auth: key_auth(),
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
        matches!(&error, AiMuxError::InvalidArgument(message) if message.contains("Invalid AWS region")),
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
        auth: key_auth(),
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
    let AiMuxError::Other(message) = &error else {
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
    let AiMuxError::Other(message) = &error else {
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
                || async { Err(AiMuxError::Other("STS denied".into())) },
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
        auth: key_auth(),
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
