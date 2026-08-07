//! Integration tests for `Provider::list_models` (RFC-0027).
//!
//! `list_models` returns `RuntimeModel` (provider's official data only).
//! Community catalogue (`get_model_specs`) is tested separately — the two are
//! deliberately decoupled.

mod common;

use std::path::Path;

use serde_json::Value;
use serial_test::serial;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::model_catalogue::RuntimeModel;
use aimux_core::provider::Provider;
use aimux_providers::catalogue;
use aimux_providers::openai::{OpenAIConfig, OpenAIProvider};
use aimux_providers::{ProviderOptions, provider_handle};

async fn mount_cassette_file(server: &MockServer, cassette_path: &Path) -> String {
    let text = std::fs::read_to_string(cassette_path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", cassette_path.display()));
    let v: Value = serde_json::from_str(&text).expect("cassette is JSON");
    let req = &v["request"];
    let m = req["method"].as_str().unwrap_or("GET").to_ascii_uppercase();
    let p = req["path"]
        .as_str()
        .expect("cassette has request.path")
        .to_string();
    let resp = &v["response"];
    let status = resp["status"].as_u64().unwrap_or(200) as u16;
    let body = resp["body"].as_str().unwrap_or("");
    let mut template = ResponseTemplate::new(status).set_body_bytes(body.as_bytes().to_vec());
    if let Some(headers) = resp["headers"].as_object() {
        for (name, val) in headers {
            if let Some(s) = val.as_str() {
                template = template.append_header(name.as_str(), s);
            }
        }
    }
    Mock::given(method(m.as_str()))
        .and(path(p.as_str()))
        .respond_with(template)
        .mount(server)
        .await;
    p
}

fn base_url_for(server_uri: &str, cassette_path_str: &str) -> String {
    let base = cassette_path_str
        .strip_suffix("/models")
        .unwrap_or(cassette_path_str);
    format!("{server_uri}{base}")
}

#[tokio::test]
#[serial]
async fn openai_provider_list_models() {
    let server = MockServer::start().await;
    let cassette = Path::new("tests/cassettes/openai/list_models_smoke.json");
    let recorded_path = mount_cassette_file(&server, cassette).await;
    let base_url = base_url_for(&server.uri(), &recorded_path);
    let config = OpenAIConfig::new("test-key").with_base_url(base_url);
    let provider = OpenAIProvider::new(config);
    let models: Vec<RuntimeModel> = provider.list_models().await.unwrap();
    assert!(models.iter().any(|m| m.id == "gpt-4o"));
    assert!(models.iter().all(|m| m.owned_by.is_some()));
}

#[tokio::test]
#[serial]
async fn deepseek_provider_list_models_via_handle() {
    let server = MockServer::start().await;
    let cassette = Path::new("tests/cassettes/deepseek/list_models_smoke.json");
    let recorded_path = mount_cassette_file(&server, cassette).await;
    let base_url = base_url_for(&server.uri(), &recorded_path);
    let opts = ProviderOptions {
        base_url: Some(base_url),
        ..Default::default()
    };
    let handle = provider_handle("deepseek", Some("test-key".into()), Some(opts)).unwrap();
    let models = handle.list_models().await.unwrap();
    assert_eq!(models.len(), 2);
    assert!(models.iter().any(|m| m.id == "deepseek-v4-flash"));
    assert!(models.iter().any(|m| m.id == "deepseek-v4-pro"));
}

#[tokio::test]
#[serial]
async fn provider_handle_then_language_model() {
    let server = MockServer::start().await;
    let cassette = Path::new("tests/cassettes/deepseek/list_models_smoke.json");
    let recorded_path = mount_cassette_file(&server, cassette).await;
    let base_url = base_url_for(&server.uri(), &recorded_path);
    let opts = ProviderOptions {
        base_url: Some(base_url),
        ..Default::default()
    };
    let handle = provider_handle("deepseek", Some("test-key".into()), Some(opts)).unwrap();
    let models = handle.list_models().await.unwrap();
    let first_id = models[0].id.clone();
    let _model = handle.language_model(&first_id).unwrap();
}

#[tokio::test]
#[serial]
async fn anthropic_list_models() {
    let server = MockServer::start().await;
    let cassette = Path::new("tests/cassettes/anthropic/list_models_smoke.json");
    let _ = mount_cassette_file(&server, cassette).await;
    let base_url = server.uri().trim_end_matches('/').to_string();
    use aimux_providers::anthropic::{AnthropicConfig, AnthropicProvider};
    let config = AnthropicConfig::new("test-key").with_base_url(&base_url);
    let provider = AnthropicProvider::new(config);
    let models: Vec<RuntimeModel> = provider.list_models().await.unwrap();
    assert!(!models.is_empty());
    assert!(models.iter().all(|m| !m.id.is_empty()));
}

#[tokio::test]
#[serial]
async fn google_list_models() {
    let server = MockServer::start().await;
    let cassette = Path::new("tests/cassettes/gemini/list_models_smoke.json");
    let recorded_path = mount_cassette_file(&server, cassette).await;
    let base_url = format!(
        "{}{}",
        server.uri(),
        recorded_path
            .strip_suffix("/models")
            .unwrap_or(&recorded_path)
    );
    use aimux_providers::google::{GoogleConfig, GoogleProvider};
    let config = GoogleConfig::new("test-key").with_base_url(&base_url);
    let provider = GoogleProvider::new(config);
    let models: Vec<RuntimeModel> = provider.list_models().await.unwrap();
    assert!(!models.is_empty());
    assert!(models.iter().all(|m| !m.id.starts_with("models/")));
}

#[tokio::test]
#[serial]
async fn ollama_list_models_via_delegate_macro() {
    let server = MockServer::start().await;
    let body = r#"{"data":[{"id":"llama3.2","object":"model","owned_by":"ollama"},{"id":"qwen3:4b","object":"model","owned_by":"ollama"}]}"#;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(
            ResponseTemplate::new(200)
                .append_header("content-type", "application/json")
                .set_body_bytes(body.as_bytes().to_vec()),
        )
        .mount(&server)
        .await;
    use aimux_providers::ollama::{OllamaConfig, OllamaProvider};
    let config = OllamaConfig::new("ollama").with_base_url(format!("{}/v1", server.uri()));
    let provider = OllamaProvider::new(config);
    let models: Vec<RuntimeModel> = provider.list_models().await.unwrap();
    assert_eq!(models.len(), 2);
    assert!(models.iter().any(|m| m.id == "llama3.2"));
}

#[tokio::test]
#[serial]
async fn list_models_malformed_response() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"not json at all".to_vec()))
        .mount(&server)
        .await;
    let config = OpenAIConfig::new("test-key").with_base_url(format!("{}/v1", server.uri()));
    let provider = OpenAIProvider::new(config);
    let err = provider.list_models().await.unwrap_err();
    assert!(matches!(err, aimux_core::AiMuxError::Json(_)));
}

#[tokio::test]
#[serial]
async fn list_models_empty_data() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(br#"{"data":[]}"#.to_vec()))
        .mount(&server)
        .await;
    let config = OpenAIConfig::new("test-key").with_base_url(format!("{}/v1", server.uri()));
    let provider = OpenAIProvider::new(config);
    let models: Vec<RuntimeModel> = provider.list_models().await.unwrap();
    assert!(models.is_empty());
}

#[tokio::test]
#[serial]
async fn list_models_http_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(
            ResponseTemplate::new(500).set_body_bytes(br#"{"error":"internal"}"#.to_vec()),
        )
        .mount(&server)
        .await;
    let config = OpenAIConfig::new("test-key")
        .with_base_url(format!("{}/v1", server.uri()))
        .with_retry_config(aimux_provider_utils::RetryConfig {
            max_retries: 0,
            ..Default::default()
        });
    let provider = OpenAIProvider::new(config);
    let err = provider.list_models().await.unwrap_err();
    assert!(!err.to_string().is_empty());
}

#[test]
fn provider_handle_unknown_name() {
    match aimux_providers::provider_handle("no-such-provider", Some("k".into()), None) {
        Err(e) => assert!(matches!(e, aimux_core::AiMuxError::UnknownProvider(_))),
        Ok(_) => panic!("unknown provider should fail"),
    }
}

#[tokio::test]
async fn unsupported_provider_returns_unsupported() {
    struct StubProvider;
    impl Provider for StubProvider {
        fn name(&self) -> &str {
            "stub"
        }
        fn language_model(
            &self,
            _model_id: &str,
        ) -> Result<Box<dyn aimux_core::language_model::LanguageModel>, aimux_core::AiMuxError>
        {
            Err(aimux_core::AiMuxError::Unsupported("none".into()))
        }
    }
    let err = StubProvider.list_models().await.unwrap_err();
    assert!(matches!(err, aimux_core::AiMuxError::Unsupported(_)));
}

#[test]
fn catalogue_normalize_and_lookup() {
    assert_eq!(
        catalogue::normalize_provider_name("fireworks-ai"),
        "fireworks"
    );
    assert_eq!(
        catalogue::normalize_provider_name("google-vertex"),
        "vertex"
    );
    let anya2a = serde_json::json!({"updated_at":"1","providers":{"fireworks-ai":{"models":[{"id":"fw-1","type":"chat","tool_call":true}]}}});
    let cat = catalogue::parse_anya2a_all(&anya2a).unwrap();
    assert!(cat.lookup("fireworks", "fw-1").is_some());
    assert!(cat.lookup("fireworks-ai", "fw-1").is_none());
}

#[test]
fn parse_anya2a_missing_providers_key() {
    let err = catalogue::parse_anya2a_all(&serde_json::json!({"updated_at":"0"})).unwrap_err();
    assert!(matches!(err, aimux_core::AiMuxError::Json(_)));
}

#[test]
fn normalize_multi_target_aliases() {
    assert_eq!(
        catalogue::normalize_provider_name("google-vertex-anthropic"),
        "vertex"
    );
    assert_eq!(
        catalogue::normalize_provider_name("xiaomi-token-plan-ams"),
        "xiaomi"
    );
    assert_eq!(
        catalogue::normalize_provider_name("zhipuai-coding-plan"),
        "zhipu_coding_plan"
    );
    assert_eq!(
        catalogue::normalize_provider_name("siliconflow-com"),
        "siliconflow"
    );
}

#[tokio::test]
#[serial]
async fn get_model_specs_success() {
    let server = MockServer::start().await;
    let body = r#"{"updated_at":"12345","providers":{"deepseek":{"models":[{"id":"deepseek-chat","type":"chat","tool_call":true}]}}}"#;
    Mock::given(method("GET"))
        .and(path("/all.json"))
        .respond_with(
            ResponseTemplate::new(200)
                .append_header("content-type", "application/json")
                .set_body_bytes(body.as_bytes().to_vec()),
        )
        .mount(&server)
        .await;
    let cat = aimux_providers::get_model_specs(Some(&format!("{}/all.json", server.uri())))
        .await
        .unwrap();
    assert_eq!(cat.updated_at, 12345);
    assert!(cat.lookup("deepseek", "deepseek-chat").is_some());
}

#[tokio::test]
#[serial]
async fn get_model_specs_http_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/all.json"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    let err = aimux_providers::get_model_specs(Some(&format!("{}/all.json", server.uri())))
        .await
        .unwrap_err();
    assert!(!err.to_string().is_empty());
}

#[test]
fn model_spec_serde_roundtrip() {
    use aimux_core::model_catalogue::*;
    let spec = ModelSpec {
        display_name: Some("GPT-4o".into()),
        r#type: ModelType::Chat,
        limits: ModelLimits {
            context: Some(128000),
            output: Some(16384),
            input: None,
        },
        modalities: ModelModalities {
            input: vec![Modality::Text, Modality::Image],
            output: vec![Modality::Text],
        },
        capabilities: ModelCapabilities {
            tool_call: true,
            structured_output: true,
            temperature: true,
            attachment: true,
        },
        reasoning: Some(ReasoningSpec {
            supported: true,
            default_enabled: false,
            mode: Some(ReasoningMode::Effort),
            effort_default: Some("high".into()),
            effort_options: vec!["low".into(), "high".into()],
            budget_min: None,
            interleaved: true,
            visibility: Some(ReasoningVisibility::Summary),
        }),
        cost: Some(ModelCost {
            input: Some(2.5),
            output: Some(10.0),
            cache_read: Some(1.25),
            cache_write: None,
        }),
        source: CatalogueSource::Anya2a,
        provider: Some("openai".into()),
        raw: None,
    };
    let json = serde_json::to_string(&spec).unwrap();
    assert_eq!(spec, serde_json::from_str::<ModelSpec>(&json).unwrap());
}

#[test]
fn modality_other_fallback() {
    assert_eq!(
        serde_json::from_str::<aimux_core::model_catalogue::Modality>(r#""quantum""#).unwrap(),
        aimux_core::model_catalogue::Modality::Other
    );
}

#[test]
fn model_type_image_edit_alias() {
    assert_eq!(
        serde_json::from_str::<aimux_core::model_catalogue::ModelType>(r#""image-edit""#).unwrap(),
        aimux_core::model_catalogue::ModelType::ImageEdit
    );
}
