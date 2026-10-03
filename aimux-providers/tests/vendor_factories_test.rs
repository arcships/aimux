//! The Azure / xAI / Mistral / Cohere / Hugging Face / Codex / Open Responses /
//! Voyage / ElevenLabs factories.
//!
//! Each package follows the shape of the AI SDK's `createXxx`: settings are
//! validated once, the credential is evaluated on every request (never when the
//! provider is created, never from a stale copy), headers layer provider ->
//! call with `None` removing, the transport is the one the settings name, and
//! the `provider()` strings are the AI SDK's. The per-package wire behavior is
//! covered by the package's own test files; this file covers the factory
//! contract across all nine, through a scripted `Fetch` transport.

#[path = "common/mock_fetch.rs"]
mod mock_fetch;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{Value, json};
use serial_test::serial;

use aimux_core::AiMuxError;
use aimux_core::content::ContentPart;
use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel};
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelPrompt, LanguageModelPromptMessage};
use aimux_core::message::Role;
use aimux_core::options::CallOptions;
use aimux_core::provider::{Provider, ProviderDiscovery};
use aimux_core::speech_model::SpeechModel;
use aimux_core::transcription_model::{
    AudioChunk, InputAudioFormat, TranscriptionModel, TranscriptionStreamOptions,
};
use aimux_provider_utils::ws::{WebSocketRequest, WsConnection, WsConnector};
use aimux_provider_utils::{HeaderMapOpt, Resolvable};
use aimux_providers::azure::{AzureOpenAIProviderSettings, azure, create_azure};
use aimux_providers::codex::{CodexMode, CodexProviderSettings, codex, create_codex};
use aimux_providers::cohere::{CohereProviderSettings, cohere, create_cohere};
use aimux_providers::elevenlabs::{ElevenLabsProviderSettings, create_elevenlabs, elevenlabs};
use aimux_providers::huggingface::{HuggingFaceProviderSettings, create_huggingface, huggingface};
use aimux_providers::mistral::{MistralProviderSettings, create_mistral, mistral};
use aimux_providers::open_responses::{OpenResponsesProviderSettings, create_open_responses};
use aimux_providers::voyage::{VoyageProviderSettings, create_voyage, voyage};
use aimux_providers::xai::{XAIProviderSettings, create_xai, xai};

use mock_fetch::{Canned, EnvVar, MockFetch};

// ── helpers ──────────────────────────────────────────────────────────────────

fn prompt(text: &str) -> LanguageModelPrompt {
    vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![ContentPart::text(text)],
        ..Default::default()
    }]
}

fn call() -> CallOptions {
    CallOptions::new(prompt("Hello"))
}

fn value(text: &str) -> Resolvable<String> {
    Resolvable::Value(text.to_string())
}

fn headers(pairs: &[(&str, Option<&str>)]) -> HeaderMapOpt {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_string(), value.map(str::to_string)))
        .collect()
}

/// A Responses API answer with one text output item.
fn responses_answer() -> Canned {
    Canned::json(&json!({
        "id": "resp_1",
        "object": "response",
        "created_at": 1_741_257_730,
        "status": "completed",
        "error": null,
        "incomplete_details": null,
        "model": "m",
        "output": [{
            "id": "msg_1",
            "type": "message",
            "status": "completed",
            "role": "assistant",
            "content": [{ "type": "output_text", "text": "answer", "annotations": [] }]
        }],
        "usage": { "input_tokens": 1, "output_tokens": 1 }
    }))
}

/// A chat-completions-shaped answer (Mistral).
fn chat_answer() -> Canned {
    Canned::json(&json!({
        "id": "chatcmpl-1",
        "object": "chat.completion",
        "created": 1_711_115_037,
        "model": "m",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": "answer" },
            "finish_reason": "stop"
        }],
        "usage": { "prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2 }
    }))
}

fn embedding_answer() -> Canned {
    Canned::json(&json!({
        "data": [{ "index": 0, "embedding": [0.5, 0.25] }],
        "embeddings": { "float": [[0.5, 0.25]] },
        "usage": { "prompt_tokens": 1 }
    }))
}

fn error_answer(status: u16) -> Canned {
    Canned {
        status,
        headers: vec![("content-type".into(), "application/json".into())],
        body: serde_json::to_vec(&json!({ "error": { "message": "boom" }, "message": "boom" }))
            .unwrap(),
    }
}

/// The environment variables of the nine packages, cleared for a test and
/// restored when the guard set drops.
struct CleanEnv(#[allow(dead_code)] Vec<EnvVar>);

fn clean_env() -> CleanEnv {
    CleanEnv(
        [
            "AZURE_API_KEY",
            "AZURE_RESOURCE_NAME",
            "XAI_API_KEY",
            "MISTRAL_API_KEY",
            "COHERE_API_KEY",
            "HUGGINGFACE_API_KEY",
            "CODEX_API_KEY",
            "VOYAGE_API_KEY",
            "ELEVENLABS_API_KEY",
        ]
        .into_iter()
        .map(|name| EnvVar::set(name, None))
        .collect(),
    )
}

fn load_api_key_var(error: &AiMuxError) -> &str {
    match error {
        AiMuxError::LoadApiKey { env_var, .. } => env_var,
        other => panic!("expected LoadApiKey, got {other:?}"),
    }
}

// ── provider strings ─────────────────────────────────────────────────────────

#[test]
fn provider_strings_follow_the_ai_sdk() {
    let azure = create_azure(AzureOpenAIProviderSettings::default()).unwrap();
    assert_eq!(azure.chat("d").provider(), "azure.chat");
    assert_eq!(azure.responses("d").provider(), "azure.responses");
    assert_eq!(azure.embedding("d").provider(), "azure.embeddings");
    assert_eq!(
        aimux_core::image_model::ImageModel::provider(&azure.image("d")),
        "azure.image"
    );
    assert_eq!(azure.transcription("d").provider(), "azure.transcription");
    assert_eq!(azure.speech("d").provider(), "azure.speech");
    assert_eq!(
        azure.language_model("d").unwrap().provider(),
        "azure.responses"
    );

    let xai = create_xai(XAIProviderSettings::default()).unwrap();
    assert_eq!(xai.responses("m").provider(), "xai.responses");
    assert_eq!(xai.language_model("m").unwrap().provider(), "xai.responses");

    let mistral = create_mistral(MistralProviderSettings::default()).unwrap();
    assert_eq!(mistral.chat("m").provider(), "mistral.chat");
    assert_eq!(mistral.embedding("m").provider(), "mistral.embedding");

    let cohere = create_cohere(CohereProviderSettings::default()).unwrap();
    assert_eq!(cohere.chat("m").provider(), "cohere.chat");
    assert_eq!(cohere.embedding("m").provider(), "cohere.textEmbedding");
    assert_eq!(
        aimux_core::reranking_model::RerankingModel::provider(&cohere.reranking("m")),
        "cohere.reranking"
    );

    let hf = create_huggingface(HuggingFaceProviderSettings::default()).unwrap();
    assert_eq!(hf.responses("m").provider(), "huggingface.responses");
    assert_eq!(
        hf.language_model("m").unwrap().provider(),
        "huggingface.responses"
    );

    let codex = create_codex(CodexProviderSettings::default()).unwrap();
    assert_eq!(codex.responses("m").provider(), "codex.responses");

    let open_responses = create_open_responses(OpenResponsesProviderSettings::new(
        "my.proxy",
        "http://x/v1",
    ))
    .unwrap();
    assert_eq!(
        open_responses.responses("m").provider(),
        "my.proxy.responses"
    );

    let voyage = create_voyage(VoyageProviderSettings::default()).unwrap();
    assert_eq!(voyage.embedding("m").provider(), "voyage.embedding");
    assert_eq!(
        aimux_core::reranking_model::RerankingModel::provider(&voyage.reranking("m")),
        "voyage.reranking"
    );

    let eleven = create_elevenlabs(ElevenLabsProviderSettings::default()).unwrap();
    assert_eq!(eleven.speech("m").provider(), "elevenlabs.speech");
    assert_eq!(
        eleven.transcription("m").provider(),
        "elevenlabs.transcription"
    );
}

#[test]
fn name_setting_prefixes_every_provider_string() {
    let mistral = create_mistral(MistralProviderSettings {
        name: Some("proxy".into()),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(mistral.chat("m").provider(), "proxy.chat");
    assert_eq!(mistral.embedding("m").provider(), "proxy.embedding");

    let cohere = create_cohere(CohereProviderSettings {
        name: Some("proxy".into()),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(cohere.embedding("m").provider(), "proxy.textEmbedding");

    let xai = create_xai(XAIProviderSettings {
        name: Some("proxy".into()),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(xai.responses("m").provider(), "proxy.responses");

    let eleven = create_elevenlabs(ElevenLabsProviderSettings {
        name: Some("proxy".into()),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(eleven.speech("m").provider(), "proxy.speech");
}

#[test]
fn missing_models_are_no_such_model() {
    fn model_type<T: ?Sized>(result: Result<Arc<T>, AiMuxError>) -> String {
        match result {
            Err(AiMuxError::NoSuchModel { model_type, .. }) => model_type,
            Err(other) => panic!("expected NoSuchModel, got {other:?}"),
            Ok(_) => panic!("expected NoSuchModel"),
        }
    }
    let voyage = create_voyage(VoyageProviderSettings::default()).unwrap();
    assert_eq!(model_type(voyage.language_model("m")), "languageModel");
    let mistral = create_mistral(MistralProviderSettings::default()).unwrap();
    assert_eq!(model_type(mistral.image_model("m")), "imageModel");
    let hf = create_huggingface(HuggingFaceProviderSettings::default()).unwrap();
    assert_eq!(model_type(hf.embedding_model("m")), "embeddingModel");
    let xai = create_xai(XAIProviderSettings::default()).unwrap();
    assert_eq!(model_type(xai.embedding_model("m")), "embeddingModel");
    let eleven = create_elevenlabs(ElevenLabsProviderSettings::default()).unwrap();
    assert_eq!(model_type(eleven.language_model("m")), "languageModel");
    assert!(eleven.speech_model("m").is_some());
    assert!(eleven.transcription_model("m").is_some());
    assert!(mistral.speech_model("m").is_none());
}

// ── creation reads nothing, calls load the key ───────────────────────────────

#[test]
#[serial]
fn default_instances_read_no_environment_and_never_fail() {
    let _env = clean_env();
    let _ = (
        azure(),
        xai(),
        mistral(),
        cohere(),
        huggingface(),
        codex(),
        voyage(),
        elevenlabs(),
    );
    for result in [
        create_azure(AzureOpenAIProviderSettings::default()).map(|_| ()),
        create_xai(XAIProviderSettings::default()).map(|_| ()),
        create_mistral(MistralProviderSettings::default()).map(|_| ()),
        create_cohere(CohereProviderSettings::default()).map(|_| ()),
        create_huggingface(HuggingFaceProviderSettings::default()).map(|_| ()),
        create_codex(CodexProviderSettings::default()).map(|_| ()),
        create_voyage(VoyageProviderSettings::default()).map(|_| ()),
        create_elevenlabs(ElevenLabsProviderSettings::default()).map(|_| ()),
    ] {
        result.expect("creation never needs the environment");
    }
}

#[test]
fn invalid_base_urls_fail_when_the_provider_is_created() {
    let bad = Some("not a url".to_string());
    assert!(
        create_xai(XAIProviderSettings {
            base_url: bad.clone(),
            ..Default::default()
        })
        .is_err()
    );
    assert!(
        create_azure(AzureOpenAIProviderSettings {
            base_url: bad.clone(),
            ..Default::default()
        })
        .is_err()
    );
    assert!(
        create_codex(CodexProviderSettings {
            base_url: bad.clone(),
            ..Default::default()
        })
        .is_err()
    );
    assert!(create_open_responses(OpenResponsesProviderSettings::new("x", "not a url")).is_err());
    assert!(create_open_responses(OpenResponsesProviderSettings::new(" ", "http://x/v1")).is_err());
}

#[tokio::test]
#[serial]
async fn a_missing_key_fails_the_call_with_the_env_var_name() {
    let _env = clean_env();
    let mock = MockFetch::new(vec![]);

    let azure = create_azure(AzureOpenAIProviderSettings {
        resource_name: Some("res".into()),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap();
    let err = azure.chat("d").do_generate(&call()).await.unwrap_err();
    assert_eq!(load_api_key_var(&err), "AZURE_API_KEY");

    let xai = create_xai(XAIProviderSettings {
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap();
    let err = xai.responses("m").do_generate(&call()).await.unwrap_err();
    assert_eq!(load_api_key_var(&err), "XAI_API_KEY");

    let mistral = create_mistral(MistralProviderSettings {
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap();
    let err = mistral.chat("m").do_generate(&call()).await.unwrap_err();
    assert_eq!(load_api_key_var(&err), "MISTRAL_API_KEY");

    let cohere = create_cohere(CohereProviderSettings {
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap();
    let err = cohere.chat("m").do_generate(&call()).await.unwrap_err();
    assert_eq!(load_api_key_var(&err), "COHERE_API_KEY");

    let hf = create_huggingface(HuggingFaceProviderSettings {
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap();
    let err = hf.responses("m").do_generate(&call()).await.unwrap_err();
    assert_eq!(load_api_key_var(&err), "HUGGINGFACE_API_KEY");

    let codex = create_codex(CodexProviderSettings {
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap();
    let err = codex.responses("m").do_generate(&call()).await.unwrap_err();
    assert_eq!(load_api_key_var(&err), "CODEX_API_KEY");

    let voyage = create_voyage(VoyageProviderSettings {
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap();
    let err = voyage
        .embedding("m")
        .do_embed(&EmbeddingCallOptions::new("x"))
        .await
        .unwrap_err();
    assert_eq!(load_api_key_var(&err), "VOYAGE_API_KEY");

    let eleven = create_elevenlabs(ElevenLabsProviderSettings {
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap();
    let err = eleven
        .speech("m")
        .do_generate(&aimux_core::speech_model::SpeechCallOptions::new("hi"))
        .await
        .unwrap_err();
    assert_eq!(load_api_key_var(&err), "ELEVENLABS_API_KEY");

    // Nothing reached the transport.
    assert!(mock.seen().is_empty());
}

#[tokio::test]
#[serial]
async fn an_empty_key_is_sent_as_given_and_never_falls_back() {
    let _env = clean_env();
    let _decoy = EnvVar::set("MISTRAL_API_KEY", Some("from-env"));
    let mock = MockFetch::new(vec![chat_answer()]);
    let mistral = create_mistral(MistralProviderSettings {
        api_key: Some(value("")),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap();
    mistral.chat("m").do_generate(&call()).await.unwrap();
    assert_eq!(
        mock.seen()[0]
            .headers
            .get("authorization")
            .map(String::as_str),
        Some("Bearer ")
    );
}

#[tokio::test]
#[serial]
async fn the_key_is_evaluated_on_every_request() {
    let _env = clean_env();
    let mock = MockFetch::new(vec![chat_answer(), chat_answer()]);
    let counter = Arc::new(AtomicUsize::new(0));
    let mistral = create_mistral(MistralProviderSettings {
        api_key: Some(Resolvable::from_async_fn({
            let counter = counter.clone();
            move || {
                let n = counter.fetch_add(1, Ordering::SeqCst) + 1;
                async move { Ok(format!("key-{n}")) }
            }
        })),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap();
    let model = mistral.chat("m");
    model.do_generate(&call()).await.unwrap();
    model.do_generate(&call()).await.unwrap();
    let seen = mock.seen();
    assert_eq!(seen[0].headers["authorization"], "Bearer key-1");
    assert_eq!(seen[1].headers["authorization"], "Bearer key-2");
}

// ── headers, transform, transport, discovery ─────────────────────────────────

#[tokio::test]
#[serial]
async fn headers_layer_provider_then_call_and_none_removes() {
    let _env = clean_env();
    let mock = MockFetch::new(vec![chat_answer()]);
    let mistral = create_mistral(MistralProviderSettings {
        api_key: Some(value("k")),
        headers: Some(headers(&[
            ("X-Provider", Some("p")),
            ("X-Both", Some("provider")),
            ("Authorization", None),
        ])),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap();
    let mut options = call();
    options.headers = Some(
        [("x-both".to_string(), "call".to_string())]
            .into_iter()
            .collect(),
    );
    mistral.chat("m").do_generate(&options).await.unwrap();
    let seen = &mock.seen()[0];
    assert_eq!(seen.headers["x-provider"], "p");
    assert_eq!(seen.headers["x-both"], "call");
    assert!(!seen.headers.contains_key("authorization"));
}

#[tokio::test]
#[serial]
async fn transform_request_body_rewrites_the_body_that_is_sent_and_reported() {
    let _env = clean_env();
    let transform: aimux_providers::mistral::TransformRequestBody = Arc::new(|mut body: Value| {
        body["transformed"] = json!(true);
        body
    });
    let mock = MockFetch::new(vec![chat_answer()]);
    let mistral = create_mistral(MistralProviderSettings {
        api_key: Some(value("k")),
        transform_request_body: Some(transform),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap();
    let result = mistral.chat("m").do_generate(&call()).await.unwrap();
    assert_eq!(mock.seen()[0].json_body()["transformed"], json!(true));
    assert_eq!(result.request_body.unwrap()["transformed"], json!(true));
}

#[tokio::test]
#[serial]
async fn every_package_sends_through_the_injected_transport() {
    let _env = clean_env();
    let mock = MockFetch::new(vec![
        responses_answer(),
        chat_answer(),
        embedding_answer(),
        responses_answer(),
    ]);
    let xai = create_xai(XAIProviderSettings {
        api_key: Some(value("k")),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap();
    xai.responses("grok").do_generate(&call()).await.unwrap();

    let mistral = create_mistral(MistralProviderSettings {
        api_key: Some(value("k")),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap();
    mistral.chat("m").do_generate(&call()).await.unwrap();

    let voyage = create_voyage(VoyageProviderSettings {
        api_key: Some(value("k")),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap();
    voyage
        .embedding("voyage-3")
        .do_embed(&EmbeddingCallOptions::new("x"))
        .await
        .unwrap();

    let hf = create_huggingface(HuggingFaceProviderSettings {
        api_key: Some(value("k")),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap();
    hf.responses("m").do_generate(&call()).await.unwrap();

    let urls: Vec<String> = mock.seen().into_iter().map(|seen| seen.url).collect();
    assert_eq!(
        urls,
        [
            "https://api.x.ai/v1/responses",
            "https://api.mistral.ai/v1/chat/completions",
            "https://api.voyageai.com/v1/embeddings",
            "https://router.huggingface.co/v1/responses",
        ]
    );
}

#[tokio::test]
#[serial]
async fn discovery_is_one_exchange_with_no_retry() {
    let _env = clean_env();
    let mock = MockFetch::new(vec![error_answer(500), error_answer(500)]);
    let mistral = create_mistral(MistralProviderSettings {
        api_key: Some(value("k")),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap();
    assert!(mistral.list_models().await.is_err());
    assert_eq!(mock.seen().len(), 1);
    assert_eq!(mock.seen()[0].url, "https://api.mistral.ai/v1/models");

    let mock = MockFetch::new(vec![error_answer(500), error_answer(500)]);
    let cohere = create_cohere(CohereProviderSettings {
        api_key: Some(value("k")),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap();
    assert!(cohere.list_models().await.is_err());
    assert_eq!(mock.seen().len(), 1);

    let mock = MockFetch::new(vec![Canned::json(
        &json!({ "data": [{ "id": "grok-4", "owned_by": "xai" }] }),
    )]);
    let xai = create_xai(XAIProviderSettings {
        api_key: Some(value("k")),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap();
    let models = xai.list_models().await.unwrap();
    assert_eq!(models[0].id, "grok-4");
    assert_eq!(mock.seen()[0].url, "https://api.x.ai/v1/models");
}

#[test]
fn settings_debug_never_prints_secrets() {
    let settings = CodexProviderSettings {
        mode: CodexMode::ChatGptAccount {
            token: value("secret-token"),
            account_id: Some("acct".into()),
        },
        headers: Some(headers(&[("X-Secret", Some("secret-header"))])),
        ..Default::default()
    };
    let debug = format!("{settings:?}");
    assert!(!debug.contains("secret-token"), "{debug}");
    assert!(!debug.contains("secret-header"), "{debug}");

    let azure = AzureOpenAIProviderSettings {
        api_key: Some(value("secret-key")),
        ..Default::default()
    };
    assert!(!format!("{azure:?}").contains("secret-key"));
}

// ── Azure ────────────────────────────────────────────────────────────────────

type AzureChange = Box<dyn FnOnce(&mut AzureOpenAIProviderSettings)>;

fn azure_at(
    mock: &Arc<MockFetch>,
    change: impl FnOnce(&mut AzureOpenAIProviderSettings),
) -> aimux_providers::azure::AzureOpenAIProvider {
    let mut settings = AzureOpenAIProviderSettings {
        api_key: Some(value("azure-key")),
        fetch: Some(mock.transport()),
        ..Default::default()
    };
    change(&mut settings);
    create_azure(settings).unwrap()
}

#[tokio::test]
#[serial]
async fn azure_urls_follow_the_ai_sdk_rules() {
    let _env = clean_env();
    let cases: Vec<(AzureChange, &str)> = vec![
        (
            Box::new(|s| s.resource_name = Some("my-res".into())),
            "https://my-res.openai.azure.com/openai/v1/chat/completions?api-version=v1",
        ),
        (
            Box::new(|s| {
                s.resource_name = Some("my-res".into());
                s.use_deployment_based_urls = true;
                s.api_version = Some("2025-04-01-preview".into());
            }),
            "https://my-res.openai.azure.com/openai/deployments/gpt-4o/chat/completions?api-version=2025-04-01-preview",
        ),
        (
            Box::new(|s| s.base_url = Some("https://x.openai.azure.com/openai/v1/".into())),
            "https://x.openai.azure.com/openai/v1/chat/completions",
        ),
        (
            Box::new(|s| s.base_url = Some("https://gateway.example/azure".into())),
            "https://gateway.example/azure/chat/completions",
        ),
    ];
    for (change, expected) in cases {
        let mock = MockFetch::new(vec![chat_answer()]);
        let provider = azure_at(&mock, change);
        provider.chat("gpt-4o").do_generate(&call()).await.unwrap();
        let seen = mock.seen();
        assert_eq!(seen[0].url, expected);
        assert_eq!(seen[0].headers["api-key"], "azure-key");
        assert!(!seen[0].headers.contains_key("authorization"));
    }
}

#[tokio::test]
#[serial]
async fn azure_other_modalities_use_the_same_url_rules() {
    let _env = clean_env();
    let mock = MockFetch::new(vec![embedding_answer()]);
    let provider = azure_at(&mock, |s| s.resource_name = Some("my-res".into()));
    provider
        .embedding("emb")
        .do_embed(&EmbeddingCallOptions::new("x"))
        .await
        .unwrap();
    assert_eq!(
        mock.seen()[0].url,
        "https://my-res.openai.azure.com/openai/v1/embeddings?api-version=v1"
    );
}

#[tokio::test]
#[serial]
async fn azure_resource_name_is_read_from_the_environment_per_request() {
    let _env = clean_env();
    let mock = MockFetch::new(vec![chat_answer()]);
    let provider = azure_at(&mock, |_| {});
    let err = provider.chat("d").do_generate(&call()).await.unwrap_err();
    assert!(
        matches!(&err, AiMuxError::LoadSetting { env_var, .. } if env_var == "AZURE_RESOURCE_NAME"),
        "{err:?}"
    );
    let _resource = EnvVar::set("AZURE_RESOURCE_NAME", Some("from-env"));
    provider.chat("d").do_generate(&call()).await.unwrap();
    assert!(
        mock.seen()[0]
            .url
            .starts_with("https://from-env.openai.azure.com/")
    );
}

#[tokio::test]
#[serial]
async fn azure_resource_name_must_be_a_dns_label() {
    let _env = clean_env();
    let mock = MockFetch::new(vec![]);
    let provider = azure_at(&mock, |s| s.resource_name = Some("evil.example/x".into()));
    let err = provider.chat("d").do_generate(&call()).await.unwrap_err();
    assert!(matches!(err, AiMuxError::InvalidArgument(_)), "{err:?}");
    assert!(mock.seen().is_empty());
}

#[tokio::test]
#[serial]
async fn azure_token_provider_is_a_bearer_token_resolved_per_request() {
    let _env = clean_env();
    let mock = MockFetch::new(vec![chat_answer(), chat_answer(), chat_answer()]);
    let counter = Arc::new(AtomicUsize::new(0));
    let provider = azure_at(&mock, {
        let counter = counter.clone();
        move |s| {
            s.api_key = None;
            s.base_url = Some("https://gateway.example/azure".into());
            s.token_provider = Some(Resolvable::from_async_fn(move || {
                let n = counter.fetch_add(1, Ordering::SeqCst) + 1;
                async move { Ok(format!("entra-{n}")) }
            }));
        }
    });
    let model = provider.chat("d");
    model.do_generate(&call()).await.unwrap();
    model.do_generate(&call()).await.unwrap();
    let seen = mock.seen();
    assert_eq!(seen[0].headers["authorization"], "Bearer entra-1");
    assert_eq!(seen[1].headers["authorization"], "Bearer entra-2");
    assert!(!seen[0].headers.contains_key("api-key"));

    // A caller-set Authorization header wins and the token is not requested.
    let mock = MockFetch::new(vec![chat_answer()]);
    let counter = Arc::new(AtomicUsize::new(0));
    let provider = azure_at(&mock, {
        let counter = counter.clone();
        move |s| {
            s.api_key = None;
            s.base_url = Some("https://gateway.example/azure".into());
            s.headers = Some(headers(&[("Authorization", Some("Bearer mine"))]));
            s.token_provider = Some(Resolvable::from_async_fn(move || {
                counter.fetch_add(1, Ordering::SeqCst);
                async move { Ok("unused".to_string()) }
            }));
        }
    });
    provider.chat("d").do_generate(&call()).await.unwrap();
    assert_eq!(mock.seen()[0].headers["authorization"], "Bearer mine");
    assert_eq!(counter.load(Ordering::SeqCst), 0);
}

#[tokio::test]
#[serial]
async fn azure_responses_read_azure_options_then_openai_and_write_azure_metadata() {
    let _env = clean_env();
    let base = Some("https://gateway.example/azure".to_string());

    // `openai` options are the fallback when there are no `azure` options.
    let mock = MockFetch::new(vec![responses_answer()]);
    let provider = azure_at(&mock, |s| s.base_url = base.clone());
    let mut options = call();
    options.provider_options = Some(
        [("openai".to_string(), json!({ "user": "from-openai" }))]
            .into_iter()
            .collect(),
    );
    let result = provider.responses("d").do_generate(&options).await.unwrap();
    assert_eq!(mock.seen()[0].json_body()["user"], "from-openai");
    let metadata = result.provider_metadata.expect("metadata");
    assert_eq!(metadata["azure"]["responseId"], "resp_1");
    assert!(metadata.get("openai").is_none());

    // `azure` options win as a whole.
    let mock = MockFetch::new(vec![responses_answer()]);
    let provider = azure_at(&mock, |s| s.base_url = base.clone());
    let mut options = call();
    options.provider_options = Some(
        [
            ("openai".to_string(), json!({ "user": "from-openai" })),
            ("azure".to_string(), json!({ "user": "from-azure" })),
        ]
        .into_iter()
        .collect(),
    );
    provider.responses("d").do_generate(&options).await.unwrap();
    assert_eq!(mock.seen()[0].json_body()["user"], "from-azure");

    // An assistant item carrying `azure.itemId` is replayed as a reference.
    let mock = MockFetch::new(vec![responses_answer()]);
    let provider = azure_at(&mock, |s| s.base_url = base.clone());
    let history = vec![
        LanguageModelPromptMessage {
            role: Role::User,
            content: vec![ContentPart::text("Hi")],
            ..Default::default()
        },
        LanguageModelPromptMessage {
            role: Role::Assistant,
            content: vec![ContentPart::Text {
                text: "earlier".into(),
                provider_options: Some(json!({ "azure": { "itemId": "msg_azure" } })),
            }],
            ..Default::default()
        },
    ];
    provider
        .responses("d")
        .do_generate(&CallOptions::new(history))
        .await
        .unwrap();
    let input = &mock.seen()[0].json_body()["input"];
    assert!(
        input
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["type"] == "item_reference" && item["id"] == "msg_azure"),
        "{input}"
    );
}

#[tokio::test]
#[serial]
async fn azure_responses_treat_assistant_prefixed_data_as_a_file_id() {
    let _env = clean_env();
    let mock = MockFetch::new(vec![responses_answer()]);
    let provider = azure_at(&mock, |s| {
        s.base_url = Some("https://gateway.example/azure".into())
    });
    let message = LanguageModelPromptMessage {
        role: Role::User,
        content: vec![ContentPart::FileBase64 {
            data: "assistant-123".into(),
            media_type: "application/pdf".into(),
            filename: Some("a.pdf".into()),
            provider_options: None,
        }],
        ..Default::default()
    };
    provider
        .responses("d")
        .do_generate(&CallOptions::new(vec![message]))
        .await
        .unwrap();
    let body = mock.seen()[0].json_body();
    let part = &body["input"][0]["content"][0];
    assert_eq!(part["type"], "input_file");
    assert_eq!(part["file_id"], "assistant-123");
    assert!(part.get("file_data").is_none(), "{part}");
}

#[tokio::test]
#[serial]
async fn azure_discovery_lists_deployments_in_one_exchange() {
    let _env = clean_env();
    let mock = MockFetch::new(vec![Canned::json(
        &json!({ "data": [{ "id": "my-gpt4o", "model": "gpt-4o" }] }),
    )]);
    let provider = azure_at(&mock, |s| s.resource_name = Some("my-res".into()));
    let models = provider.list_models().await.unwrap();
    assert_eq!(models[0].id, "my-gpt4o");
    assert_eq!(
        mock.seen()[0].url,
        "https://my-res.openai.azure.com/openai/deployments?api-version=2024-10-21"
    );
    assert_eq!(mock.seen()[0].headers["api-key"], "azure-key");
}

// ── Codex ────────────────────────────────────────────────────────────────────

fn sse(events: &[Value]) -> Canned {
    let mut body = String::new();
    for event in events {
        body.push_str(&format!("data: {event}\n\n"));
    }
    body.push_str("data: [DONE]\n\n");
    Canned {
        status: 200,
        headers: vec![("content-type".into(), "text/event-stream".into())],
        body: body.into_bytes(),
    }
}

fn completed_event() -> Value {
    let Value::Object(mut response) =
        serde_json::from_slice::<Value>(&responses_answer().body).unwrap()
    else {
        unreachable!()
    };
    response.insert("model".into(), json!("gpt-5.2-codex"));
    json!({ "type": "response.completed", "response": response })
}

#[tokio::test]
#[serial]
async fn codex_chatgpt_account_streams_without_storing_and_resolves_the_token_per_request() {
    let _env = clean_env();
    let mock = MockFetch::new(vec![sse(&[completed_event()]), sse(&[completed_event()])]);
    let counter = Arc::new(AtomicUsize::new(0));
    let transform: aimux_providers::codex::TransformRequestBody = Arc::new(|mut body: Value| {
        // Runs after the package's own `store: false` rule.
        body["store_seen_by_transform"] = body["store"].clone();
        body
    });
    let provider = create_codex(CodexProviderSettings {
        mode: CodexMode::ChatGptAccount {
            token: Resolvable::from_async_fn({
                let counter = counter.clone();
                move || {
                    let n = counter.fetch_add(1, Ordering::SeqCst) + 1;
                    async move { Ok(format!("acct-token-{n}")) }
                }
            }),
            account_id: Some("acct_1".into()),
        },
        headers: Some(headers(&[("X-Extra", Some("e"))])),
        transform_request_body: Some(transform),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap();
    let model = provider.responses("gpt-5.2-codex");
    model.do_generate(&call()).await.unwrap();
    model.do_generate(&call()).await.unwrap();

    let seen = mock.seen();
    // The subscription endpoint is `/backend-api/codex/responses` (as the
    // `chatgpt` cassettes record), not `/backend-api/responses`.
    assert_eq!(
        seen[0].url,
        "https://chatgpt.com/backend-api/codex/responses"
    );
    assert_eq!(
        aimux_providers::CODEX_SUBSCRIPTION_BASE_URL,
        "https://chatgpt.com/backend-api/codex"
    );
    assert_eq!(seen[0].headers["authorization"], "Bearer acct-token-1");
    assert_eq!(seen[1].headers["authorization"], "Bearer acct-token-2");
    assert_eq!(seen[0].headers["originator"], "aimux");
    assert_eq!(seen[0].headers["chatgpt-account-id"], "acct_1");
    assert_eq!(seen[0].headers["x-extra"], "e");
    let body = seen[0].json_body();
    assert_eq!(body["stream"], json!(true));
    assert_eq!(body["store"], json!(false));
    assert_eq!(body["store_seen_by_transform"], json!(false));
}

#[tokio::test]
#[serial]
async fn codex_api_key_mode_defaults_to_the_openai_endpoint() {
    let _env = clean_env();
    let _key = EnvVar::set("CODEX_API_KEY", Some("env-codex-key"));
    let mock = MockFetch::new(vec![responses_answer()]);
    let provider = create_codex(CodexProviderSettings {
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap();
    provider
        .responses("gpt-5.2-codex")
        .do_generate(&call())
        .await
        .unwrap();
    let seen = &mock.seen()[0];
    assert_eq!(seen.url, "https://api.openai.com/v1/responses");
    assert_eq!(seen.headers["authorization"], "Bearer env-codex-key");
    assert!(!seen.headers.contains_key("originator"));
    assert!(seen.json_body().get("store").is_none());
}

#[tokio::test]
#[serial]
async fn codex_account_401_is_token_expired_and_not_retryable() {
    let _env = clean_env();
    let mock = MockFetch::new(vec![error_answer(401)]);
    let provider = create_codex(CodexProviderSettings {
        mode: CodexMode::ChatGptAccount {
            token: value("old"),
            account_id: None,
        },
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap();
    let err = provider
        .responses("m")
        .do_generate(&call())
        .await
        .unwrap_err();
    assert!(matches!(err, AiMuxError::TokenExpired(_)), "{err:?}");
    assert!(!err.is_retryable());
}

// ── Open Responses ───────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn open_responses_sends_no_credential_unless_configured() {
    let _env = clean_env();
    let mock = MockFetch::new(vec![responses_answer(), responses_answer()]);
    let local = create_open_responses(OpenResponsesProviderSettings {
        fetch: Some(mock.transport()),
        ..OpenResponsesProviderSettings::new("lmstudio", "http://localhost:1234/v1/")
    })
    .unwrap();
    local.responses("m").do_generate(&call()).await.unwrap();

    let counter = Arc::new(AtomicUsize::new(0));
    let keyed = create_open_responses(OpenResponsesProviderSettings {
        api_key: Some(value("sk")),
        headers: Some(Resolvable::from_async_fn({
            let counter = counter.clone();
            move || {
                let n = counter.fetch_add(1, Ordering::SeqCst) + 1;
                async move { Ok(headers(&[("X-N", Some(&n.to_string()))])) }
            }
        })),
        fetch: Some(mock.transport()),
        ..OpenResponsesProviderSettings::new("lmstudio", "http://localhost:1234/v1")
    })
    .unwrap();
    keyed.responses("m").do_generate(&call()).await.unwrap();

    let seen = mock.seen();
    assert_eq!(seen[0].url, "http://localhost:1234/v1/responses");
    assert!(!seen[0].headers.contains_key("authorization"));
    assert_eq!(seen[1].headers["authorization"], "Bearer sk");
    assert_eq!(seen[1].headers["x-n"], "1");
}

#[tokio::test]
#[serial]
async fn open_responses_reads_provider_options_under_the_first_name_segment() {
    let _env = clean_env();
    let mock = MockFetch::new(vec![responses_answer()]);
    let provider = create_open_responses(OpenResponsesProviderSettings {
        fetch: Some(mock.transport()),
        ..OpenResponsesProviderSettings::new("my.proxy", "http://localhost:1234/v1")
    })
    .unwrap();
    let mut options = call();
    options.provider_options = Some(
        [("my".to_string(), json!({ "reasoningSummary": "auto" }))]
            .into_iter()
            .collect(),
    );
    provider.responses("m").do_generate(&options).await.unwrap();
    assert_eq!(mock.seen()[0].json_body()["reasoning"]["summary"], "auto");
}

// ── ElevenLabs ───────────────────────────────────────────────────────────────

/// The URL and headers of a recorded WebSocket handshake.
type Handshake = (String, Vec<(String, String)>);

/// A connector that records the handshake request and refuses to connect.
#[derive(Default)]
struct RecordingConnector {
    seen: Mutex<Option<Handshake>>,
}

#[async_trait]
impl WsConnector for RecordingConnector {
    async fn connect(&self, request: &WebSocketRequest) -> Result<WsConnection, AiMuxError> {
        *self.seen.lock().unwrap() = Some((request.url.clone(), request.headers.clone()));
        Err(AiMuxError::InvalidArgument("connector refused".into()))
    }
}

#[tokio::test]
#[serial]
async fn elevenlabs_websocket_handshake_uses_resolved_headers_and_the_injected_connector() {
    let _env = clean_env();
    let connector = Arc::new(RecordingConnector::default());
    let counter = Arc::new(AtomicUsize::new(0));
    let provider = create_elevenlabs(ElevenLabsProviderSettings {
        api_key: Some(Resolvable::from_async_fn({
            let counter = counter.clone();
            move || {
                let n = counter.fetch_add(1, Ordering::SeqCst) + 1;
                async move { Ok(format!("el-{n}")) }
            }
        })),
        headers: Some(headers(&[("X-Trace", Some("t"))])),
        web_socket: Some(connector.clone()),
        ..Default::default()
    })
    .unwrap();
    let model = provider.transcription("scribe_v2_realtime");

    for expected_key in ["el-1", "el-2"] {
        let result = model
            .do_stream(TranscriptionStreamOptions {
                audio: Box::pin(futures::stream::iter(vec![AudioChunk::Binary(vec![0, 1])])),
                input_audio_format: InputAudioFormat {
                    format_type: "audio/pcm".into(),
                    rate: Some(16_000),
                },
                provider_options: None,
                abort_signal: None,
                headers: None,
                include_raw_chunks: false,
                timeout: None,
            })
            .await;
        let Err(err) = result else {
            panic!("connector refuses");
        };
        assert!(
            matches!(&err, AiMuxError::InvalidArgument(m) if m == "connector refused"),
            "{err:?}"
        );
        let (url, handshake) = connector.seen.lock().unwrap().clone().unwrap();
        assert!(
            url.starts_with("wss://api.elevenlabs.io/v1/speech-to-text/realtime?"),
            "{url}"
        );
        let header = |name: &str| {
            handshake
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(header("xi-api-key"), Some(expected_key));
        assert_eq!(header("x-trace"), Some("t"));
    }
}

#[tokio::test]
#[serial]
async fn elevenlabs_speech_sends_the_key_as_xi_api_key() {
    let _env = clean_env();
    let mock = MockFetch::new(vec![Canned {
        status: 200,
        headers: vec![("content-type".into(), "audio/mpeg".into())],
        body: vec![1, 2, 3],
    }]);
    let provider = create_elevenlabs(ElevenLabsProviderSettings {
        api_key: Some(value("el-key")),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap();
    provider
        .speech("eleven_multilingual_v2")
        .do_generate(&aimux_core::speech_model::SpeechCallOptions::new("hello"))
        .await
        .unwrap();
    let seen = &mock.seen()[0];
    assert!(
        seen.url
            .starts_with("https://api.elevenlabs.io/v1/text-to-speech/"),
        "{}",
        seen.url
    );
    assert_eq!(seen.headers["xi-api-key"], "el-key");
}
