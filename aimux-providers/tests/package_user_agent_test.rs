//! The full user-agent each pinned package sends, as literals.
//!
//! Upstream a request carries the package part, added by the provider's
//! `getHeaders` (`withUserAgentSuffix(.., "ai-sdk-{package}/{VERSION}")`),
//! then the provider-utils part, added by `postToApi` with the `VERSION` of
//! the provider-utils that package imports (provider-utils
//! post-to-api.ts:100-104). npm installs each package's exact
//! `@ai-sdk/provider-utils` dependency for it, so the second part follows the
//! package's own package.json: 5.0.51 for the packages pinned on the
//! provider 4.0.19 line, 5.0.53 for those on 4.0.21. aimux leaves out the
//! runtime fragment (`node.js/…`), as for every provider.
//!
//! The versions are literals on purpose: a test that reads the pins could not
//! catch the constants and the pins drifting together, and none of them may
//! follow the `@ai-sdk/provider-utils` entry of VERSIONS.json.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::json;

use aimux_core::language_model_message::LanguageModelMessage;
use aimux_core::options::CallOptions;
use aimux_core::provider::Provider;
use aimux_provider_utils::{FetchFunction, Resolvable};

#[path = "common/mock_fetch.rs"]
mod mock_fetch;

use mock_fetch::{Canned, RouteFetch};

fn options() -> CallOptions {
    CallOptions::new(vec![LanguageModelMessage::user_text("Hello")])
}

/// Every request answered with an empty JSON object; only the request
/// matters here.
fn transport() -> (Arc<RouteFetch>, FetchFunction) {
    let fetch = RouteFetch::new(|_| Canned::json(&json!({})));
    let transport = fetch.transport();
    (fetch, transport)
}

/// The user-agent of the one request `provider`'s `model_id` sends.
async fn sent_user_agent(provider: &dyn Provider, model_id: &str, fetch: &RouteFetch) -> String {
    sent_user_agent_with(provider, model_id, fetch, options()).await
}

async fn sent_user_agent_with(
    provider: &dyn Provider,
    model_id: &str,
    fetch: &RouteFetch,
    options: CallOptions,
) -> String {
    let model = provider.language_model(model_id).unwrap();
    // The empty response does not parse; the request has gone out by then.
    let _ = model.do_generate(&options).await;
    let seen = fetch.seen();
    assert_eq!(seen.len(), 1, "{model_id}: one request");
    seen[0].headers["user-agent"].clone()
}

/// groq-provider.ts:78-88; `@ai-sdk/groq@4.0.52` depends on provider-utils 5.0.51.
#[tokio::test]
async fn groq_names_its_own_provider_utils() {
    let (fetch, transport) = transport();
    let provider =
        aimux_providers::groq::create_groq(aimux_providers::groq::GroqProviderSettings {
            api_key: Some("k".into()),
            fetch: Some(transport),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        sent_user_agent(&provider, "llama-3.3-70b-versatile", &fetch).await,
        "ai-sdk-groq/4.0.52 ai-sdk-provider-utils/5.0.51"
    );
}

/// A per-call `user-agent` replaces the provider's, package part included
/// (`combineHeaders`), but `postToApi` still appends the package's own
/// provider-utils part (post-to-api.ts:100-104).
#[tokio::test]
async fn a_per_call_user_agent_keeps_the_package_provider_utils() {
    let (fetch, transport) = transport();
    let provider =
        aimux_providers::groq::create_groq(aimux_providers::groq::GroqProviderSettings {
            api_key: Some("k".into()),
            fetch: Some(transport),
            ..Default::default()
        })
        .unwrap();
    let mut options = options();
    options.headers = Some(HashMap::from([("user-agent".into(), "app/1".into())]));
    assert_eq!(
        sent_user_agent_with(&provider, "llama-3.3-70b-versatile", &fetch, options).await,
        "app/1 ai-sdk-provider-utils/5.0.51"
    );
}

/// deepseek-provider.ts:76-85; `@ai-sdk/deepseek@3.0.56` depends on 5.0.51.
#[tokio::test]
async fn deepseek_names_its_own_provider_utils() {
    let (fetch, transport) = transport();
    let provider = aimux_providers::deepseek::create_deepseek(
        aimux_providers::deepseek::DeepSeekProviderSettings {
            api_key: Some("k".into()),
            fetch: Some(transport),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        sent_user_agent(&provider, "deepseek-chat", &fetch).await,
        "ai-sdk-deepseek/3.0.56 ai-sdk-provider-utils/5.0.51"
    );
}

/// mistral-provider.ts:118-127; `@ai-sdk/mistral@4.0.54` depends on 5.0.51.
#[tokio::test]
async fn mistral_names_its_own_provider_utils() {
    let (fetch, transport) = transport();
    let provider = aimux_providers::create_mistral(aimux_providers::MistralProviderSettings {
        api_key: Some("k".into()),
        fetch: Some(transport),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(
        sent_user_agent(&provider, "mistral-small-latest", &fetch).await,
        "ai-sdk-mistral/4.0.54 ai-sdk-provider-utils/5.0.51"
    );
}

/// cohere-provider.ts:102-111; `@ai-sdk/cohere@4.0.52` depends on 5.0.51.
#[tokio::test]
async fn cohere_names_its_own_provider_utils() {
    let (fetch, transport) = transport();
    let provider = aimux_providers::create_cohere(aimux_providers::CohereProviderSettings {
        api_key: Some("k".into()),
        fetch: Some(transport),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(
        sent_user_agent(&provider, "command-a-03-2025", &fetch).await,
        "ai-sdk-cohere/4.0.52 ai-sdk-provider-utils/5.0.51"
    );
}

/// xai-provider.ts:149-158; `@ai-sdk/xai@5.0.12` depends on 5.0.51.
#[tokio::test]
async fn xai_names_its_own_provider_utils() {
    let (fetch, transport) = transport();
    let provider = aimux_providers::create_xai(aimux_providers::XAIProviderSettings {
        api_key: Some("k".into()),
        fetch: Some(transport),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(
        sent_user_agent(&provider, "grok-4", &fetch).await,
        "ai-sdk-xai/5.0.12 ai-sdk-provider-utils/5.0.51"
    );
}

/// azure-openai-provider.ts:239-244; `@ai-sdk/azure@4.0.84` depends on 5.0.51
/// (and runs on its own `@ai-sdk/openai@4.0.80`, which does too).
#[tokio::test]
async fn azure_names_its_own_provider_utils() {
    let (fetch, transport) = transport();
    let provider = aimux_providers::create_azure(aimux_providers::AzureOpenAIProviderSettings {
        resource_name: Some("test-resource".into()),
        api_key: Some(Resolvable::Value("k".into())),
        fetch: Some(transport),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(
        sent_user_agent(&provider, "test-deployment", &fetch).await,
        "ai-sdk-azure/4.0.84 ai-sdk-provider-utils/5.0.51"
    );
}

/// google-vertex-provider-base.ts:270-276; `@ai-sdk/google-vertex@5.0.98`
/// and its nested `@ai-sdk/google@4.0.85` depend on 5.0.51.
#[tokio::test]
async fn vertex_gemini_names_its_own_provider_utils() {
    let (fetch, transport) = transport();
    let provider = aimux_providers::create_google_vertex(aimux_providers::VertexProviderSettings {
        api_key: Some(Resolvable::Value("k".into())),
        fetch: Some(transport),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(
        sent_user_agent(&provider, "gemini-2.5-flash", &fetch).await,
        "ai-sdk-google-vertex/5.0.98 ai-sdk-provider-utils/5.0.51"
    );
}

/// The Anthropic sub-provider sends the caller's headers as they are
/// (google-vertex-anthropic-provider.ts:206-210) through the nested
/// `@ai-sdk/anthropic@4.0.68`, which depends on 5.0.51.
#[tokio::test]
async fn vertex_anthropic_names_the_nested_provider_utils() {
    let (fetch, transport) = transport();
    let provider = aimux_providers::create_google_vertex_anthropic(
        aimux_providers::VertexAnthropicProviderSettings {
            project: Some("test-project".into()),
            location: Some("us-east5".into()),
            fetch: Some(transport),
            generate_auth_token: Some(Resolvable::Value(Some("test-token".into()))),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        sent_user_agent(&provider, "claude-sonnet-4-5@20250929", &fetch).await,
        "ai-sdk-provider-utils/5.0.51"
    );
}

fn partner_settings(transport: FetchFunction) -> aimux_providers::VertexPartnerProviderSettings {
    aimux_providers::VertexPartnerProviderSettings {
        project: Some("test-project".into()),
        fetch: Some(transport),
        generate_auth_token: Some(Resolvable::Value(Some("test-token".into()))),
        ..Default::default()
    }
}

/// MaaS calls `createOpenAICompatible` from the `@ai-sdk/openai-compatible`
/// that google-vertex pins, 3.0.59 (google-vertex-maas-provider.ts:111-118),
/// which depends on 5.0.51.
#[tokio::test]
async fn vertex_maas_names_the_nested_openai_compatible() {
    let (fetch, transport) = transport();
    let provider = aimux_providers::create_google_vertex_maas(partner_settings(transport)).unwrap();
    assert_eq!(
        sent_user_agent(&provider, "meta/llama-3.3-70b-instruct-maas", &fetch).await,
        "ai-sdk-openai-compatible/3.0.59 ai-sdk-provider-utils/5.0.51"
    );
}

/// The xAI sub-provider builds on the same nested openai-compatible
/// (google-vertex-xai-provider.ts:170-184).
#[tokio::test]
async fn vertex_xai_names_the_nested_openai_compatible() {
    let (fetch, transport) = transport();
    let provider = aimux_providers::create_google_vertex_xai(partner_settings(transport)).unwrap();
    assert_eq!(
        sent_user_agent(&provider, "xai/grok-4.1-fast-reasoning", &fetch).await,
        "ai-sdk-openai-compatible/3.0.59 ai-sdk-provider-utils/5.0.51"
    );
}

/// openai-provider.ts:220; `@ai-sdk/openai@4.0.83` depends on 5.0.53.
#[tokio::test]
async fn openai_names_its_own_provider_utils() {
    let (fetch, transport) = transport();
    let provider = aimux_providers::create_openai(aimux_providers::OpenAIProviderSettings {
        api_key: Some("k".into()),
        fetch: Some(transport),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(
        sent_user_agent(&provider, "gpt-5", &fetch).await,
        "ai-sdk-openai/4.0.83 ai-sdk-provider-utils/5.0.53"
    );
}

/// anthropic-provider.ts:161; `@ai-sdk/anthropic@4.0.71` depends on 5.0.53.
#[tokio::test]
async fn anthropic_names_its_own_provider_utils() {
    let (fetch, transport) = transport();
    let provider = aimux_providers::create_anthropic(aimux_providers::AnthropicProviderSettings {
        api_key: Some("k".into()),
        fetch: Some(transport),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(
        sent_user_agent(&provider, "claude-sonnet-4-5", &fetch).await,
        "ai-sdk-anthropic/4.0.71 ai-sdk-provider-utils/5.0.53"
    );
}

/// google-provider.ts:260; `@ai-sdk/google@4.0.87` depends on 5.0.53.
#[tokio::test]
async fn google_names_its_own_provider_utils() {
    let (fetch, transport) = transport();
    let provider = aimux_providers::create_google(aimux_providers::GoogleProviderSettings {
        api_key: Some("k".into()),
        fetch: Some(transport),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(
        sent_user_agent(&provider, "gemini-2.5-flash", &fetch).await,
        "ai-sdk-google/4.0.87 ai-sdk-provider-utils/5.0.53"
    );
}

/// openai-compatible-provider.ts:160; `@ai-sdk/openai-compatible@3.0.62`
/// depends on 5.0.53.
#[tokio::test]
async fn openai_compatible_names_its_own_provider_utils() {
    let (fetch, transport) = transport();
    let provider = aimux_providers::openai_compatible::create_openai_compatible(
        aimux_providers::openai_compatible::OpenAICompatibleProviderSettings {
            name: "custom".into(),
            base_url: "https://api.example.com/v1".into(),
            api_key: Some("k".into()),
            fetch: Some(transport),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        sent_user_agent(&provider, "model", &fetch).await,
        "ai-sdk-openai-compatible/3.0.62 ai-sdk-provider-utils/5.0.53"
    );
}

fn open_responses(
    transport: FetchFunction,
    user_agent_suffix: Option<&str>,
) -> aimux_providers::OpenResponsesProvider {
    let mut settings =
        aimux_providers::OpenResponsesProviderSettings::new("lmstudio", "http://localhost:1234/v1");
    settings.fetch = Some(transport);
    settings.user_agent_suffix = user_agent_suffix.map(str::to_owned);
    aimux_providers::create_open_responses(settings).unwrap()
}

/// open-responses-provider.ts:114-127 adds `ai-sdk-open-responses/VERSION`;
/// `@ai-sdk/open-responses@2.0.58` depends on provider-utils 5.0.53.
#[tokio::test]
async fn open_responses_names_its_own_provider_utils() {
    let (fetch, transport) = transport();
    let provider = open_responses(transport, None);
    assert_eq!(
        sent_user_agent(&provider, "model", &fetch).await,
        "ai-sdk-open-responses/2.0.58 ai-sdk-provider-utils/5.0.53"
    );
}

/// open-responses-provider.ts:126: `userAgentSuffix` replaces the package
/// part; the provider-utils part still follows it.
#[tokio::test]
async fn open_responses_user_agent_suffix_replaces_the_package_part() {
    let (fetch, transport) = transport();
    let provider = open_responses(transport, Some("my-app/1.0"));
    assert_eq!(
        sent_user_agent(&provider, "model", &fetch).await,
        "my-app/1.0 ai-sdk-provider-utils/5.0.53"
    );
}

/// huggingface-provider.ts:68-75 adds no package part; `@ai-sdk/huggingface@2.0.62`
/// depends on provider-utils 5.0.53.
#[tokio::test]
async fn huggingface_sends_only_its_provider_utils() {
    let (fetch, transport) = transport();
    let provider =
        aimux_providers::create_huggingface(aimux_providers::HuggingFaceProviderSettings {
            api_key: Some("k".to_string().into()),
            fetch: Some(transport),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        sent_user_agent(&provider, "model", &fetch).await,
        "ai-sdk-provider-utils/5.0.53"
    );
}
