//! Rust translation of the Luma image model tests.
//! Source: `reference/ai/packages/luma/src/luma-image-model.test.ts`

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::image_model::{ImageCallOptions, ImageFile, ImageModel, ImageOutputs};
use aimux_provider_utils::Resolvable;
use aimux_providers::{LumaProviderSettings, create_luma};

const PROMPT: &str = "A beautiful sunset";

async fn mock_luma(server: &MockServer) {
    let img_url = format!("{}/image.png", server.uri());
    Mock::given(method("POST"))
        .and(path("/dream-machine/v1/generations/image"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "id": "gen-123", "state": "queued" })),
        )
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/dream-machine/v1/generations/gen-123"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({ "id": "gen-123", "state": "completed", "assets": { "image": img_url } }),
        ))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/image.png"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"fake-luma-image"))
        .mount(server)
        .await;
}

fn options(prompt: &str) -> ImageCallOptions {
    ImageCallOptions::new(prompt.to_string())
}

#[tokio::test]
async fn should_extract_generated_image() {
    let server = MockServer::start().await;
    mock_luma(&server).await;
    let config = LumaProviderSettings {
        api_key: Some(Resolvable::Value("test-key".to_string())),
        base_url: Some(server.uri().to_string()),
        ..Default::default()
    };
    let model = create_luma(config).unwrap().image("ray2");
    let result = model.do_generate(&options(PROMPT)).await.unwrap();
    match result.images {
        ImageOutputs::Binary(imgs) => {
            assert_eq!(imgs.len(), 1);
            assert_eq!(imgs[0], b"fake-luma-image");
        }
        _ => panic!("expected Binary"),
    }
}

#[tokio::test]
async fn should_pass_prompt_and_model() {
    let server = MockServer::start().await;
    mock_luma(&server).await;
    let config = LumaProviderSettings {
        api_key: Some(Resolvable::Value("test-key".to_string())),
        base_url: Some(server.uri().to_string()),
        ..Default::default()
    };
    let model = create_luma(config).unwrap().image("ray2");
    model.do_generate(&options(PROMPT)).await.unwrap();
    let reqs = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(body["prompt"], PROMPT);
    assert_eq!(body["model"], "ray2");
}

#[tokio::test]
async fn should_pass_aspect_ratio() {
    let server = MockServer::start().await;
    mock_luma(&server).await;
    let config = LumaProviderSettings {
        api_key: Some(Resolvable::Value("test-key".to_string())),
        base_url: Some(server.uri().to_string()),
        ..Default::default()
    };
    let model = create_luma(config).unwrap().image("ray2");
    let mut opts = options(PROMPT);
    opts.aspect_ratio = Some(aimux_core::shared::AspectRatio::new(16, 9));
    model.do_generate(&opts).await.unwrap();
    let reqs = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(body["aspect_ratio"], "16:9");
}

#[tokio::test]
async fn should_warn_for_seed() {
    let server = MockServer::start().await;
    mock_luma(&server).await;
    let config = LumaProviderSettings {
        api_key: Some(Resolvable::Value("test-key".to_string())),
        base_url: Some(server.uri().to_string()),
        ..Default::default()
    };
    let model = create_luma(config).unwrap().image("ray2");
    let mut opts = options(PROMPT);
    opts.seed = Some(42);
    let result = model.do_generate(&opts).await.unwrap();
    assert!(result.warnings.iter().any(|w| matches!(w, aimux_core::types::Warning::Unsupported { feature, .. } if feature == "seed")));
}

#[tokio::test]
async fn should_warn_for_size() {
    let server = MockServer::start().await;
    mock_luma(&server).await;
    let config = LumaProviderSettings {
        api_key: Some(Resolvable::Value("test-key".to_string())),
        base_url: Some(server.uri().to_string()),
        ..Default::default()
    };
    let model = create_luma(config).unwrap().image("ray2");
    let mut opts = options(PROMPT);
    opts.size = Some(aimux_core::shared::Size::new(1024, 1024));
    let result = model.do_generate(&opts).await.unwrap();
    assert!(result.warnings.iter().any(|w| matches!(w, aimux_core::types::Warning::Unsupported { feature, .. } if feature == "size")));
}

#[tokio::test]
async fn should_pass_auth_headers() {
    let server = MockServer::start().await;
    mock_luma(&server).await;
    let config = LumaProviderSettings {
        api_key: Some(Resolvable::Value("test-key".to_string())),
        base_url: Some(server.uri().to_string()),
        ..Default::default()
    };
    let model = create_luma(config).unwrap().image("ray2");
    model.do_generate(&options(PROMPT)).await.unwrap();
    let reqs = server.received_requests().await.unwrap();
    assert_eq!(
        reqs[0]
            .headers
            .get("authorization")
            .unwrap()
            .to_str()
            .unwrap(),
        "Bearer test-key"
    );
}

#[tokio::test]
async fn should_support_image_editing_with_urls() {
    let server = MockServer::start().await;
    mock_luma(&server).await;
    let config = LumaProviderSettings {
        api_key: Some(Resolvable::Value("test-key".to_string())),
        base_url: Some(server.uri().to_string()),
        ..Default::default()
    };
    let model = create_luma(config).unwrap().image("ray2");
    let mut opts = options("Edit this");
    opts.n = 1;
    opts.files = Some(vec![ImageFile::Url {
        url: "https://example.com/source.png".into(),
    }]);
    model.do_generate(&opts).await.unwrap();
    let reqs = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert!(body.get("image").is_some());
    let images = body["image"].as_array().unwrap();
    assert_eq!(images.len(), 1);
    assert_eq!(images[0]["url"], "https://example.com/source.png");
}

#[tokio::test]
async fn should_throw_for_mask() {
    let server = MockServer::start().await;
    let config = LumaProviderSettings {
        api_key: Some(Resolvable::Value("test-key".to_string())),
        base_url: Some(server.uri().to_string()),
        ..Default::default()
    };
    let model = create_luma(config).unwrap().image("ray2");
    let mut opts = options("Edit");
    opts.n = 1;
    opts.mask = Some(ImageFile::Url {
        url: "https://example.com/mask.png".into(),
    });
    assert!(model.do_generate(&opts).await.is_err());
}

fn url_file(url: &str) -> ImageFile {
    ImageFile::Url {
        url: url.to_string(),
    }
}

/// `referenceType: 'character'` sends the images grouped by identity:
/// `images[i].id`, default `identity0` (luma-image-model.ts getEditingOptions).
#[tokio::test]
async fn should_send_character_references_grouped_by_identity() {
    for (images, expected) in [
        (
            json!(null),
            json!({ "identity0": { "images": ["https://example.com/a.jpg", "https://example.com/b.jpg"] } }),
        ),
        (
            json!([{ "id": "identity0" }, { "id": "identity1" }]),
            json!({
                "identity0": { "images": ["https://example.com/a.jpg"] },
                "identity1": { "images": ["https://example.com/b.jpg"] },
            }),
        ),
    ] {
        let server = MockServer::start().await;
        mock_luma(&server).await;
        let config = LumaProviderSettings {
            api_key: Some(Resolvable::Value("test-key".to_string())),
            base_url: Some(server.uri().to_string()),
            ..Default::default()
        };
        let model = create_luma(config).unwrap().image("photon-1");
        let mut opts = options(PROMPT);
        opts.files = Some(vec![
            url_file("https://example.com/a.jpg"),
            url_file("https://example.com/b.jpg"),
        ]);
        let mut luma = json!({ "referenceType": "character" });
        if !images.is_null() {
            luma["images"] = images;
        }
        opts.provider_options = serde_json::from_value(json!({ "luma": luma })).unwrap();
        model.do_generate(&opts).await.unwrap();
        let reqs = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
        assert_eq!(body["character"], expected);
        assert!(body.get("image").is_none(), "{body}");
        assert!(body.get("images").is_none(), "{body}");
        assert!(body.get("referenceType").is_none(), "{body}");
    }
}

/// At most 4 images per identity.
#[tokio::test]
async fn should_reject_more_than_4_character_images_per_identity() {
    let model = create_luma(LumaProviderSettings {
        api_key: Some(Resolvable::Value("test-key".to_string())),
        base_url: Some("http://127.0.0.1:9".to_string()),
        ..Default::default()
    })
    .unwrap()
    .image("photon-1");
    let mut opts = options(PROMPT);
    opts.files = Some(
        (1..=5)
            .map(|i| url_file(&format!("https://example.com/{i}.jpg")))
            .collect(),
    );
    opts.provider_options =
        serde_json::from_value(json!({ "luma": { "referenceType": "character" } })).unwrap();
    let error = model.do_generate(&opts).await.unwrap_err();
    assert_eq!(
        error.to_string(),
        aimux_core::error::AiMuxError::invalid_argument(
            "Luma AI character supports up to 4 images per identity. Identity 'identity0' has 5 images."
        )
        .to_string()
    );
}

/// A generation that ends `failed`, or `completed` without an image, fails
/// with `InvalidResponseDataError` (luma-image-model.ts pollForImageUrl), not
/// an API call error.
#[tokio::test]
async fn should_fail_with_invalid_response_data_for_a_failed_or_imageless_generation() {
    for (poll, message) in [
        (
            json!({ "id": "gen-123", "state": "failed", "failure_reason": "Generation failed" }),
            "Image generation failed.",
        ),
        (
            json!({ "id": "gen-123", "state": "completed", "assets": null }),
            "Image generation completed but no image was found.",
        ),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/dream-machine/v1/generations/image"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "id": "gen-123", "state": "queued" })),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/dream-machine/v1/generations/gen-123"))
            .respond_with(ResponseTemplate::new(200).set_body_json(poll))
            .mount(&server)
            .await;
        let model = create_luma(LumaProviderSettings {
            api_key: Some(Resolvable::Value("test-key".to_string())),
            base_url: Some(server.uri().to_string()),
            ..Default::default()
        })
        .unwrap()
        .image("photon-1");
        let error = model.do_generate(&options(PROMPT)).await.unwrap_err();
        assert!(
            matches!(
                &error,
                aimux_core::error::AiMuxError::InvalidResponseData { message: m, .. } if m == message
            ),
            "{error:?}"
        );
    }
}

/// The mask is refused before the files are looked at, so a file Luma could
/// not take either still fails as a mask (luma-image-model.ts
/// getEditingOptions); the messages are upstream's.
#[tokio::test]
async fn should_refuse_a_mask_before_the_files() {
    let model = create_luma(LumaProviderSettings {
        api_key: Some(Resolvable::Value("test-key".to_string())),
        base_url: Some("http://127.0.0.1:9".to_string()),
        ..Default::default()
    })
    .unwrap()
    .image("photon-1");
    let mut opts = options(PROMPT);
    opts.files = Some(vec![ImageFile::File {
        media_type: "image/png".into(),
        data: aimux_core::image_model::ImageFileData::Binary(vec![137, 80, 78, 71]),
    }]);
    opts.mask = Some(url_file("https://example.com/mask.png"));
    let error = model.do_generate(&opts).await.unwrap_err();
    assert_eq!(
        error.to_string(),
        aimux_core::error::AiMuxError::invalid_argument(
            "Luma AI does not support mask-based image editing. Use the prompt to describe the changes you want to make, along with `prompt.images` containing the source image URL."
        )
        .to_string()
    );

    opts.mask = None;
    let error = model.do_generate(&opts).await.unwrap_err();
    assert_eq!(
        error.to_string(),
        aimux_core::error::AiMuxError::invalid_argument(
            "Luma AI only supports URL-based images. Please provide image URLs using `prompt.images` with publicly accessible URLs. Base64 and Uint8Array data are not supported."
        )
        .to_string()
    );

    opts.files = Some(
        (1..=5)
            .map(|i| url_file(&format!("https://example.com/{i}.jpg")))
            .collect(),
    );
    let error = model.do_generate(&opts).await.unwrap_err();
    assert_eq!(
        error.to_string(),
        aimux_core::error::AiMuxError::invalid_argument(
            "Luma AI image supports up to 4 reference images. You provided 5 images."
        )
        .to_string()
    );

    opts.files.as_mut().unwrap().truncate(2);
    opts.provider_options =
        serde_json::from_value(json!({ "luma": { "referenceType": "modify_image" } })).unwrap();
    let error = model.do_generate(&opts).await.unwrap_err();
    assert_eq!(
        error.to_string(),
        aimux_core::error::AiMuxError::invalid_argument(
            "Luma AI modify_image only supports a single input image. You provided 2 images."
        )
        .to_string()
    );
}
