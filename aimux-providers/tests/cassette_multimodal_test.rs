//! Multimodal cassette replay tests.
//!
//! Tests embedding, files, transcription, and image modalities using
//! real cassette data. Each cassette is mounted individually on a
//! fresh wiremock server.

mod common;

use std::fs;

use serde_json::Value;
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel};
use aimux_core::files_model::{Files, UploadFileCallOptions, UploadFileData};
use aimux_core::image_model::{ImageCallOptions, ImageModel};
use aimux_core::shared::FileBytes;
use aimux_core::transcription_model::{AudioInput, TranscriptionCallOptions, TranscriptionModel};
use aimux_providers::openai::{OpenAIConfig, OpenAIProvider};

const CASSETTE_DIR: &str = "tests/cassettes";

/// Load a single cassette by provider + filename.
fn load_cassette(provider: &str, filename: &str) -> Option<Value> {
    let path = format!("{CASSETTE_DIR}/{provider}/{filename}");
    let content = fs::read_to_string(&path).ok()?;
    serde_json::from_str(&content).ok()
}

/// Mount a single cassette on a fresh mock server.
/// Returns (server, base_url_for_v1).
async fn mount_single(cass: &Value) -> (MockServer, String) {
    let server = MockServer::start().await;
    let req_path = cass["request"]["path"].as_str().unwrap_or("/");
    let resp_status = cass["response"]["status"].as_u64().unwrap_or(200) as u16;
    let resp_body = cass["response"]["body"].as_str().unwrap_or("").to_string();

    let headers: Vec<(String, String)> = cass["response"]["headers"]
        .as_object()
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();

    let mock = Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path(req_path))
        .respond_with(move |_req: &Request| {
            let mut t = ResponseTemplate::new(resp_status);
            for (k, v) in &headers {
                t = t.insert_header(k, v);
            }
            t.set_body_string(resp_body.clone())
        });
    mock.mount(&server).await;

    let base_url = format!("{}/v1", server.uri());
    (server, base_url)
}

// ── Embedding cassettes ─────────────────────────────────────────────────────

#[tokio::test]
async fn cassette_openai_embedding_query() {
    let cass = load_cassette("openai", "TestOpenAI.test_query.json").expect("cassette should load");
    let (server, base_url) = mount_single(&cass).await;

    let provider = OpenAIProvider::new(OpenAIConfig::new("test-key").with_base_url(base_url));
    let model = provider.embedding_model("text-embedding-3-small");

    let opts = EmbeddingCallOptions::new("hello");
    let result = model.do_embed(&opts).await.expect("embed should succeed");

    // OpenAI cassette vectors are stored as strings (base64-encoded floats),
    // so provider may return empty Vec or string-as-float parse failures.
    // Hard assert: provider must return at least the embeddings array structure.
    assert!(
        !result.embeddings.is_empty(),
        "should have at least one embedding vector"
    );
}

#[tokio::test]
async fn cassette_openai_embedding_documents() {
    let cass =
        load_cassette("openai", "TestOpenAI.test_documents.json").expect("cassette should load");
    let (server, base_url) = mount_single(&cass).await;

    let provider = OpenAIProvider::new(OpenAIConfig::new("test-key").with_base_url(base_url));
    let model = provider.embedding_model("text-embedding-3-small");

    let opts = EmbeddingCallOptions {
        values: vec!["doc1".into(), "doc2".into()],
        abort_signal: None,
        provider_options: None,
        headers: None,
    };
    let result = model.do_embed(&opts).await.expect("embed should succeed");

    assert!(
        !result.embeddings.is_empty(),
        "should have at least one embedding vector"
    );
}

#[tokio::test]
async fn cassette_openai_embedding_error() {
    let cass =
        load_cassette("openai", "TestOpenAI.test_embed_error.json").expect("cassette should load");
    let (server, base_url) = mount_single(&cass).await;

    let provider = OpenAIProvider::new(OpenAIConfig::new("test-key").with_base_url(base_url));
    let model = provider.embedding_model("nonexistent");

    let opts = EmbeddingCallOptions::new("hello");
    let result = model.do_embed(&opts).await;

    assert!(result.is_err(), "should fail for nonexistent model");
}

// ── Cohere embedding cassettes ──────────────────────────────────────────────

#[tokio::test]
async fn cassette_cohere_embedding_query() {
    let cass = load_cassette("cohere", "TestCohere.test_query.json").expect("cassette should load");

    let server = MockServer::start().await;
    let req_path = cass["request"]["path"].as_str().unwrap_or("/");
    let resp_status = cass["response"]["status"].as_u64().unwrap_or(200) as u16;
    let resp_body = cass["response"]["body"].as_str().unwrap_or("").to_string();
    let headers: Vec<(String, String)> = cass["response"]["headers"]
        .as_object()
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();

    Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path(req_path))
        .respond_with(move |_req: &Request| {
            let mut t = ResponseTemplate::new(resp_status);
            for (k, v) in &headers {
                t = t.insert_header(k, v);
            }
            t.set_body_string(resp_body.clone())
        })
        .mount(&server)
        .await;

    // Cohere base_url: the cassette path is /v2/embed, so base_url = server/v2
    let base_url = format!("{}/v2", server.uri());

    use aimux_providers::cohere::{CohereConfig, CohereProvider};
    let provider = CohereProvider::new(CohereConfig::new("test-key").with_base_url(base_url));
    let model = provider.embedding_model("embed-english-v3.0");

    let opts = EmbeddingCallOptions::new("hello");
    let result = model.do_embed(&opts).await;

    match result {
        Ok(r) => assert!(!r.embeddings.is_empty(), "should have embeddings"),
        Err(e) => panic!("cohere embed failed: {e}"),
    }
}

// ── Files cassettes (OpenAI) ────────────────────────────────────────────────

#[tokio::test]
async fn cassette_openai_files_upload() {
    // Find a files upload cassette
    let dir = format!("{CASSETTE_DIR}/openai");
    let mut found = None;
    for entry in fs::read_dir(&dir).unwrap() {
        let f = entry.unwrap().path();
        if f.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let content = fs::read_to_string(&f).unwrap();
        let cass: Value = serde_json::from_str(&content).unwrap();
        if cass["request"]["path"].as_str() == Some("/v1/files")
            && cass["request"]["method"].as_str() == Some("POST")
        {
            found = Some((f.file_name().unwrap().to_string_lossy().to_string(), cass));
            break;
        }
    }

    let (filename, cass) = found.expect("should find an OpenAI files upload cassette");
    eprintln!("Using cassette: openai/{filename}");

    let server = MockServer::start().await;
    let req_path = cass["request"]["path"].as_str().unwrap_or("/");
    let resp_status = cass["response"]["status"].as_u64().unwrap_or(200) as u16;
    let resp_body = cass["response"]["body"].as_str().unwrap_or("").to_string();

    Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path(req_path))
        .respond_with(move |_req: &Request| {
            ResponseTemplate::new(resp_status).set_body_string(resp_body.clone())
        })
        .mount(&server)
        .await;

    let base_url = format!("{}/v1", server.uri());
    let provider = OpenAIProvider::new(OpenAIConfig::new("test-key").with_base_url(base_url));
    let files = provider.files();

    let opts = UploadFileCallOptions::new(
        UploadFileData::Data {
            data: FileBytes::Base64("dGVzdA==".into()),
        },
        "application/pdf",
    );

    let result = files.upload_file(&opts).await;

    match result {
        Ok(r) => {
            assert!(
                !r.provider_reference.is_empty(),
                "should have provider reference (file ID)"
            );
        }
        Err(e) => panic!("files upload failed: {e}"),
    }
}

// ── Image cassette (xAI) ────────────────────────────────────────────────────

#[tokio::test]
async fn cassette_xai_image_generation() {
    let cass = load_cassette("xai", "image_generation_smoke.json").expect("cassette should load");

    let server = MockServer::start().await;
    let req_path = cass["request"]["path"].as_str().unwrap_or("/");
    let resp_status = cass["response"]["status"].as_u64().unwrap_or(200) as u16;
    let resp_body = cass["response"]["body"].as_str().unwrap_or("").to_string();

    Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path(req_path))
        .respond_with(move |_req: &Request| {
            ResponseTemplate::new(resp_status).set_body_string(resp_body.clone())
        })
        .mount(&server)
        .await;

    // xAI is OpenAI-compatible — use OpenAI provider with xAI base_url
    let base_url = format!("{}/v1", server.uri());
    let provider = OpenAIProvider::new(
        OpenAIConfig::new("test-key")
            .with_base_url(base_url)
            .with_provider("xai"),
    );
    let model = provider.image("grok-2-image");

    use aimux_core::image_model::ImageCallOptions;
    let opts = ImageCallOptions {
        prompt: Some("A cute baby sea otter".into()),
        n: 1,
        size: None,
        aspect_ratio: None,
        seed: None,
        files: None,
        mask: None,
        provider_options: std::collections::HashMap::new(),
        abort_signal: None,
        headers: None,
    };

    let result = model
        .do_generate(&opts)
        .await
        .expect("image generate should succeed");

    match &result.images {
        aimux_core::image_model::ImageOutputs::Base64(v) => {
            assert!(!v.is_empty(), "should have at least one base64 image");
            assert!(!v[0].is_empty(), "image data should be non-empty");
        }
        aimux_core::image_model::ImageOutputs::Binary(v) => {
            assert!(!v.is_empty(), "should have at least one binary image");
            assert!(!v[0].is_empty(), "image data should be non-empty");
        }
    }
}

// ── Transcription cassette (OpenRouter) ─────────────────────────────────────

#[tokio::test]
async fn cassette_transcription() {
    let cass =
        load_cassette("openrouter", "transcription_smoke.json").expect("cassette should load");

    let server = MockServer::start().await;
    let req_path = cass["request"]["path"].as_str().unwrap_or("/");
    let resp_status = cass["response"]["status"].as_u64().unwrap_or(200) as u16;
    let resp_body = cass["response"]["body"].as_str().unwrap_or("").to_string();

    Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path(req_path))
        .respond_with(move |_req: &Request| {
            ResponseTemplate::new(resp_status).set_body_string(resp_body.clone())
        })
        .mount(&server)
        .await;

    // OpenRouter is OpenAI-compatible
    let base_url = format!("{}/api/v1", server.uri());
    let provider = OpenAIProvider::new(OpenAIConfig::new("test-key").with_base_url(base_url));
    let model = provider.transcription("whisper-1");

    use aimux_core::transcription_model::{AudioInput, TranscriptionCallOptions};
    let opts =
        TranscriptionCallOptions::new(AudioInput::Base64("dGVzdCBhdWRpbw==".into()), "audio/mp3");

    let result = model
        .do_generate(&opts)
        .await
        .expect("transcription should succeed");
    assert!(
        !r_text_empty(&result),
        "transcription text should be non-empty"
    );
    assert!(
        result.text.contains("sun"),
        "expected 'sun' in transcription, got: {}",
        result.text
    );
}

fn r_text_empty(r: &aimux_core::transcription_model::TranscriptionResult) -> bool {
    r.text.is_empty()
}
