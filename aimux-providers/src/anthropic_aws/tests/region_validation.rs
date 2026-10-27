//! Port of `@ai-sdk/anthropic-aws` 2.0.63 `src/region-validation.test.ts`.
//!
//! The upstream `describe` is `${name} region validation` with `name` =
//! `anthropic-aws`; the marker keeps the template text, the module name is
//! the resolved text. Every test changes process environment variables, so
//! all of them are serial.
//!
//! The upstream `argument: 'region'` property has no counterpart: aimux's
//! `InvalidArgument` carries a message only, which names the region setting.

use std::sync::Arc;
use std::sync::Mutex;

use async_trait::async_trait;
use serial_test::serial;

use crate::test_support::mock_fetch::{EnvVar, MockFetch};
use crate::{AnthropicAwsAuth, AnthropicAwsProviderSettings, create_anthropic_aws};
use aimux_core::AiMuxError;
use aimux_core::language_model_message::LanguageModelMessage;
use aimux_core::options::CallOptions;
use aimux_provider_utils::{
    Fetch, FetchError, FetchFunction, FetchRequest, FetchResponse, Resolvable,
};

/// The variables the upstream `beforeEach` stubs (the endpoint overrides) and
/// every other variable the provider reads, so only the test's own settings
/// reach the provider. Restored on drop.
fn clean_environment() -> Vec<EnvVar> {
    [
        "AWS_ENDPOINT_URL",
        "AWS_ENDPOINT_URL_BEDROCK_RUNTIME",
        "AWS_ENDPOINT_URL_BEDROCK_AGENT_RUNTIME",
        "AWS_REGION",
        "ANTHROPIC_AWS_API_KEY",
        "ANTHROPIC_AWS_WORKSPACE_ID",
        "AWS_ACCESS_KEY_ID",
        "AWS_SECRET_ACCESS_KEY",
        "AWS_SESSION_TOKEN",
    ]
    .into_iter()
    .map(|name| EnvVar::set(name, None))
    .collect()
}

/// `fetch = vi.fn(async () => { throw new Error('captured') })`: records the
/// URL of every request and fails it.
#[derive(Default)]
struct CapturingFetch {
    urls: Mutex<Vec<String>>,
}

impl CapturingFetch {
    fn transport(self: &Arc<Self>) -> FetchFunction {
        self.clone()
    }

    fn urls(&self) -> Vec<String> {
        self.urls.lock().unwrap().clone()
    }
}

#[async_trait]
impl Fetch for CapturingFetch {
    async fn fetch(&self, request: FetchRequest) -> Result<FetchResponse, FetchError> {
        self.urls.lock().unwrap().push(request.url.to_string());
        Err(FetchError::Other("captured".into()))
    }
}

/// `create({ ... })('test-model').doGenerate({ prompt })`. The upstream test
/// wraps the synchronous `create` in a promise chain, so a rejection from it
/// and one from `doGenerate` both reach the caller: here both are one error.
async fn generate(
    settings: AnthropicAwsProviderSettings,
    messages: Vec<LanguageModelMessage>,
) -> Result<(), AiMuxError> {
    let provider = create_anthropic_aws(settings)?;
    let model = provider.call("test-model");
    let result = model.do_generate(&CallOptions::new(messages)).await;
    result.map(|_| ())
}

/// The error of a call that must be rejected.
fn rejection(result: Result<(), AiMuxError>, what: &str) -> AiMuxError {
    match result {
        Ok(()) => panic!("{what}"),
        Err(error) => error,
    }
}

/// `toMatchObject({ name: 'AI_InvalidArgumentError' })`, on the wire form.
fn wire_name(error: &AiMuxError) -> String {
    serde_json::to_value(error).unwrap()["name"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

fn hello() -> Vec<LanguageModelMessage> {
    vec![LanguageModelMessage::user_text("Hello")]
}

mod anthropic_aws_region_validation {
    use super::*;

    /// upstream: @ai-sdk/anthropic-aws src/region-validation.test.ts › ${…} › rejects %j before fetching
    #[serial]
    #[tokio::test]
    async fn rejects_j_before_fetching() {
        let _env = clean_environment();
        for region in [
            "evil.example.com/#",
            "user@internal:8080/#",
            "us-east-1/../..",
            "us east 1",
            "",
            "region\n",
        ] {
            let fetch = MockFetch::new(vec![]);
            let error = rejection(
                generate(
                    AnthropicAwsProviderSettings {
                        region: Some(region.into()),
                        workspace_id: Some("test-workspace".into()),
                        auth: Some(AnthropicAwsAuth::ApiKey(Resolvable::Value(
                            "test-key".into(),
                        ))),
                        fetch: Some(fetch.transport()),
                        ..Default::default()
                    },
                    hello(),
                )
                .await,
                &format!("region {region:?} was accepted"),
            );

            assert!(
                matches!(&error, AiMuxError::InvalidArgument { message, .. } if message.contains("Invalid AWS region")),
                "region {region:?}: {error:?}"
            );
            assert_eq!(wire_name(&error), "AI_InvalidArgumentError");
            assert!(
                fetch.seen().is_empty(),
                "region {region:?} reached the transport"
            );
        }
    }

    /// upstream: @ai-sdk/anthropic-aws src/region-validation.test.ts › ${…} › uses the environment region
    #[serial]
    #[tokio::test]
    async fn uses_the_environment_region() {
        let _env = clean_environment();
        let _region = EnvVar::set("AWS_REGION", Some("user@internal/#"));
        let fetch = MockFetch::new(vec![]);
        let error = rejection(
            generate(
                AnthropicAwsProviderSettings {
                    workspace_id: Some("test-workspace".into()),
                    auth: Some(AnthropicAwsAuth::ApiKey(Resolvable::Value(
                        "test-key".into(),
                    ))),
                    fetch: Some(fetch.transport()),
                    ..Default::default()
                },
                vec![],
            )
            .await,
            "the environment region was accepted",
        );

        assert!(
            matches!(&error, AiMuxError::InvalidArgument { message, .. } if message.contains("Invalid AWS region")),
            "{error:?}"
        );
        assert!(fetch.seen().is_empty());
    }

    /// upstream: @ai-sdk/anthropic-aws src/region-validation.test.ts › ${…} › sends to a custom endpoint regardless of unused region %j
    #[serial]
    #[tokio::test]
    async fn sends_to_a_custom_endpoint_regardless_of_unused_region_j() {
        let _env = clean_environment();
        for region in ["us-east-1", "not a region"] {
            let fetch = Arc::new(CapturingFetch::default());
            let error = rejection(
                generate(
                    AnthropicAwsProviderSettings {
                        region: Some(region.into()),
                        workspace_id: Some("test-workspace".into()),
                        auth: Some(AnthropicAwsAuth::ApiKey(Resolvable::Value(
                            "test-key".into(),
                        ))),
                        base_url: Some("https://proxy.example/v1".into()),
                        fetch: Some(fetch.transport()),
                        ..Default::default()
                    },
                    vec![],
                )
                .await,
                &format!("region {region:?}: the request was not rejected"),
            );

            assert!(
                error.to_string().contains("captured"),
                "region {region:?}: {error}"
            );
            let urls = fetch.urls();
            let first = urls.first().cloned().unwrap_or_default();
            assert!(
                first.contains("https://proxy.example/v1"),
                "region {region:?}: request went to {first:?}"
            );
        }
    }
}
