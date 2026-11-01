//! Rust translation of the fal image model tests.
//! Source: `reference/ai/packages/fal/src/fal-image-model.test.ts`

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::image_model::{ImageCallOptions, ImageModel, ImageOutputs};
use aimux_provider_utils::Resolvable;
use aimux_providers::{FalProviderSettings, create_fal};

const PROMPT: &str = "A cute baby sea otter";

async fn mock_fal(server: &MockServer, body: Value) {
    Mock::given(method("POST"))
        .and(path("/fal-ai/flux/schnell"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/image.png"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"fake-image-bytes"))
        .mount(server)
        .await;
}

fn fal_response(server_uri: &str) -> Value {
    json!({ "images": [{ "url": format!("{}/image.png", server_uri), "width": 1024, "height": 1024 }] })
}

fn options(prompt: &str) -> ImageCallOptions {
    ImageCallOptions::new(prompt.to_string())
}

#[tokio::test]
async fn should_extract_generated_images() {
    let server = MockServer::start().await;
    mock_fal(&server, fal_response(&server.uri())).await;
    let config = FalProviderSettings {
        api_key: Some(Resolvable::Value("test-key".to_string())),
        base_url: Some(server.uri().to_string()),
        ..Default::default()
    };
    let model = create_fal(config).unwrap().image("fal-ai/flux/schnell");
    let result = model.do_generate(&options(PROMPT)).await.unwrap();
    match result.images {
        ImageOutputs::Binary(imgs) => {
            assert_eq!(imgs.len(), 1);
            assert_eq!(imgs[0], b"fake-image-bytes");
        }
        _ => panic!("expected Binary"),
    }
}

#[tokio::test]
async fn should_pass_prompt_and_n() {
    let server = MockServer::start().await;
    mock_fal(&server, fal_response(&server.uri())).await;
    let config = FalProviderSettings {
        api_key: Some(Resolvable::Value("test-key".to_string())),
        base_url: Some(server.uri().to_string()),
        ..Default::default()
    };
    let model = create_fal(config).unwrap().image("fal-ai/flux/schnell");
    let mut opts = options(PROMPT);
    opts.n = 1;
    model.do_generate(&opts).await.unwrap();
    let reqs = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(body["prompt"], PROMPT);
    assert_eq!(body["num_images"], 1);
}

#[tokio::test]
async fn should_pass_size() {
    let server = MockServer::start().await;
    mock_fal(&server, fal_response(&server.uri())).await;
    let config = FalProviderSettings {
        api_key: Some(Resolvable::Value("test-key".to_string())),
        base_url: Some(server.uri().to_string()),
        ..Default::default()
    };
    let model = create_fal(config).unwrap().image("fal-ai/flux/schnell");
    let mut opts = options(PROMPT);
    opts.n = 1;
    opts.size = Some(aimux_core::shared::Size::new(1024, 1024));
    model.do_generate(&opts).await.unwrap();
    let reqs = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(body["image_size"]["width"], 1024);
    assert_eq!(body["image_size"]["height"], 1024);
}

#[tokio::test]
async fn should_pass_aspect_ratio() {
    let server = MockServer::start().await;
    mock_fal(&server, fal_response(&server.uri())).await;
    let config = FalProviderSettings {
        api_key: Some(Resolvable::Value("test-key".to_string())),
        base_url: Some(server.uri().to_string()),
        ..Default::default()
    };
    let model = create_fal(config).unwrap().image("fal-ai/flux/schnell");
    let mut opts = options(PROMPT);
    opts.n = 1;
    opts.aspect_ratio = Some(aimux_core::shared::AspectRatio::new(16, 9));
    model.do_generate(&opts).await.unwrap();
    let reqs = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(body["image_size"], "landscape_16_9");
}

#[tokio::test]
async fn should_pass_seed() {
    let server = MockServer::start().await;
    mock_fal(&server, fal_response(&server.uri())).await;
    let config = FalProviderSettings {
        api_key: Some(Resolvable::Value("test-key".to_string())),
        base_url: Some(server.uri().to_string()),
        ..Default::default()
    };
    let model = create_fal(config).unwrap().image("fal-ai/flux/schnell");
    let mut opts = options(PROMPT);
    opts.n = 1;
    opts.seed = Some(42);
    model.do_generate(&opts).await.unwrap();
    let reqs = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(body["seed"], 42);
}

#[tokio::test]
async fn should_pass_auth_headers() {
    let server = MockServer::start().await;
    mock_fal(&server, fal_response(&server.uri())).await;
    let config = FalProviderSettings {
        api_key: Some(Resolvable::Value("test-key".to_string())),
        base_url: Some(server.uri().to_string()),
        ..Default::default()
    };
    let model = create_fal(config).unwrap().image("fal-ai/flux/schnell");
    model.do_generate(&options(PROMPT)).await.unwrap();
    let reqs = server.received_requests().await.unwrap();
    assert_eq!(
        reqs[0]
            .headers
            .get("authorization")
            .unwrap()
            .to_str()
            .unwrap(),
        "Key test-key"
    );
}

#[tokio::test]
async fn should_forward_provider_options() {
    let server = MockServer::start().await;
    mock_fal(&server, fal_response(&server.uri())).await;
    let config = FalProviderSettings {
        api_key: Some(Resolvable::Value("test-key".to_string())),
        base_url: Some(server.uri().to_string()),
        ..Default::default()
    };
    let model = create_fal(config).unwrap().image("fal-ai/flux/schnell");
    let mut opts = options(PROMPT);
    opts.n = 1;
    opts.provider_options.insert(
        "fal".into(),
        serde_json::from_value(
            json!({ "numInferenceSteps": 30, "guidanceScale": 7.5, "outputFormat": "png" }),
        )
        .unwrap(),
    );
    model.do_generate(&opts).await.unwrap();
    let reqs = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(body["num_inference_steps"], 30);
    assert_eq!(body["guidance_scale"], 7.5);
    assert_eq!(body["output_format"], "png");
}

#[tokio::test]
async fn should_handle_single_image_response() {
    let server = MockServer::start().await;
    let body = json!({ "image": { "url": format!("{}/image.png", server.uri()) } });
    mock_fal(&server, body).await;
    let config = FalProviderSettings {
        api_key: Some(Resolvable::Value("test-key".to_string())),
        base_url: Some(server.uri().to_string()),
        ..Default::default()
    };
    let model = create_fal(config).unwrap().image("fal-ai/flux/schnell");
    let result = model.do_generate(&options(PROMPT)).await.unwrap();
    match result.images {
        ImageOutputs::Binary(imgs) => assert_eq!(imgs.len(), 1),
        _ => panic!("expected Binary"),
    }
}

/// `providerOptions.fal` is parsed against `falImageModelOptionsSchema`
/// (fal-image-model.ts doGenerate): a value outside the schema fails with
/// the upstream message before anything is sent.
#[tokio::test]
async fn should_reject_image_options_outside_the_schema() {
    let model = create_fal(FalProviderSettings {
        api_key: Some(Resolvable::Value("test-key".to_string())),
        base_url: Some("http://127.0.0.1:9".to_string()),
        ..Default::default()
    })
    .unwrap()
    .image("fal-ai/flux/schnell");
    for fal in [
        json!({ "guidanceScale": 25 }),
        json!({ "guidance_scale": 0.5 }),
        json!({ "numInferenceSteps": 51 }),
        json!({ "outputFormat": "webp" }),
        json!({ "acceleration": "max" }),
        json!({ "safetyTolerance": "7" }),
        json!({ "safety_tolerance": 0 }),
        json!({ "syncMode": "yes" }),
        json!({ "imageUrl": 1 }),
    ] {
        let mut call = options(PROMPT);
        call.provider_options = serde_json::from_value(json!({ "fal": fal })).unwrap();
        let error = model.do_generate(&call).await.unwrap_err();
        let aimux_core::error::AiMuxError::InvalidArgument { message, cause } = &error else {
            panic!("expected InvalidArgument for {fal}, got {error:?}");
        };
        assert_eq!(
            message, "Invalid argument for parameter providerOptions: invalid fal provider options",
            "{fal}"
        );
        assert!(cause.is_some(), "{fal}");
    }
}

/// Values inside the schema, `null`, and unknown keys pass (a loose object).
#[tokio::test]
async fn should_accept_image_options_inside_the_schema() {
    let server = MockServer::start().await;
    mock_fal(&server, fal_response(&server.uri())).await;
    let model = create_fal(FalProviderSettings {
        api_key: Some(Resolvable::Value("test-key".to_string())),
        base_url: Some(server.uri().to_string()),
        ..Default::default()
    })
    .unwrap()
    .image("fal-ai/flux/schnell");
    let mut call = options(PROMPT);
    call.provider_options = serde_json::from_value(json!({ "fal": {
        "guidanceScale": 7.5,
        "numInferenceSteps": 50,
        "outputFormat": "png",
        "acceleration": "none",
        "safetyTolerance": "2",
        "safety_tolerance": 6,
        "syncMode": null,
        "custom_param": { "nested": true },
    } }))
    .unwrap();
    model.do_generate(&call).await.unwrap();
}

fn flux_schnell(base_url: &str) -> aimux_providers::FalImageModel {
    create_fal(FalProviderSettings {
        api_key: Some(Resolvable::Value("test-key".to_string())),
        base_url: Some(base_url.to_string()),
        ..Default::default()
    })
    .unwrap()
    .image("fal-ai/flux/schnell")
}

async fn sent_body(server: &MockServer) -> Value {
    let requests = server.received_requests().await.unwrap();
    let post = requests
        .iter()
        .rev()
        .find(|request| request.method.as_str() == "POST")
        .unwrap();
    serde_json::from_slice(&post.body).unwrap()
}

/// fal-image-model.ts getArgs: `image_size` is `undefined`, so not sent,
/// without a size or with an aspect ratio `convertAspectRatioToSize` does
/// not map.
#[tokio::test]
async fn should_not_send_an_unset_image_size() {
    let server = MockServer::start().await;
    mock_fal(&server, fal_response(&server.uri())).await;
    let model = flux_schnell(&server.uri());

    model.do_generate(&options(PROMPT)).await.unwrap();
    assert_eq!(
        sent_body(&server).await,
        json!({ "prompt": PROMPT, "num_images": 1 })
    );

    let mut call = options(PROMPT);
    call.aspect_ratio = Some(serde_json::from_value(json!("2:1")).unwrap());
    model.do_generate(&call).await.unwrap();
    assert_eq!(
        sent_body(&server).await,
        json!({ "prompt": PROMPT, "num_images": 1 })
    );
}

fn with_fal(fal: Value) -> ImageCallOptions {
    let mut call = options(PROMPT);
    call.provider_options = serde_json::from_value(json!({ "fal": fal })).unwrap();
    call
}

/// fal-image-model.ts getArgs: the schema's transform maps a snake_case key
/// to its camelCase name, records it in `__deprecatedKeys` (one warning),
/// and drops known keys that are null. Unknown keys pass through as given.
#[tokio::test]
async fn should_rewrite_the_options_like_the_upstream_transform() {
    let server = MockServer::start().await;
    mock_fal(&server, fal_response(&server.uri())).await;
    let model = flux_schnell(&server.uri());

    // "should convert camelCase provider options to snake_case for API"
    let result = model
        .do_generate(&with_fal(json!({
            "imageUrl": "https://example.com/image.png",
            "guidanceScale": 7.5,
            "numInferenceSteps": 50,
            "enableSafetyChecker": false,
        })))
        .await
        .unwrap();
    assert_eq!(
        sent_body(&server).await,
        json!({
            "prompt": PROMPT,
            "num_images": 1,
            "image_url": "https://example.com/image.png",
            "guidance_scale": 7.5,
            "num_inference_steps": 50,
            "enable_safety_checker": false,
        })
    );
    assert!(result.warnings.is_empty(), "{:?}", result.warnings);

    // "should accept deprecated snake_case provider options with warning"
    let result = model
        .do_generate(&with_fal(json!({
            "image_url": "https://example.com/image.png",
            "guidance_scale": 7.5,
            "num_inference_steps": 50,
        })))
        .await
        .unwrap();
    assert_eq!(
        sent_body(&server).await,
        json!({
            "prompt": PROMPT,
            "num_images": 1,
            "image_url": "https://example.com/image.png",
            "guidance_scale": 7.5,
            "num_inference_steps": 50,
        })
    );
    let [aimux_core::shared::Warning::Other { message }] = result.warnings.as_slice() else {
        panic!("expected one warning, got {:?}", result.warnings);
    };
    assert!(message.contains("deprecated snake_case"), "{message}");
    assert!(
        message.contains("'image_url' (use 'imageUrl')"),
        "{message}"
    );
    assert!(
        message.contains("'guidance_scale' (use 'guidanceScale')"),
        "{message}"
    );
    assert!(
        message.contains("'num_inference_steps' (use 'numInferenceSteps')"),
        "{message}"
    );

    // A null known key is not sent; a null unknown key is.
    model
        .do_generate(&with_fal(json!({ "guidanceScale": null, "extra": null })))
        .await
        .unwrap();
    assert_eq!(
        sent_body(&server).await,
        json!({ "prompt": PROMPT, "num_images": 1, "extra": null })
    );

    // The snake_case value wins over its camelCase twin.
    model
        .do_generate(&with_fal(
            json!({ "guidanceScale": 7.5, "guidance_scale": 12 }),
        ))
        .await
        .unwrap();
    assert_eq!(
        sent_body(&server).await,
        json!({ "prompt": PROMPT, "num_images": 1, "guidance_scale": 12 })
    );
}

/// fal-image-model.ts `falFailedResponseHandler`: a FastAPI validation
/// error is reported as `loc.join('.'): msg` per entry, joined by newlines;
/// anything else as its `message` (upstream "should handle API errors").
#[tokio::test]
async fn should_report_a_validation_error_by_location() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/fal-ai/flux/schnell"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "detail": [
                { "loc": ["prompt"], "msg": "Invalid prompt", "type": "value_error" },
                { "loc": ["image_size", "width"], "msg": "must be positive", "type": "value_error" },
            ],
        })))
        .mount(&server)
        .await;
    let error = flux_schnell(&server.uri())
        .do_generate(&options(PROMPT))
        .await
        .unwrap_err();
    let aimux_core::error::AiMuxError::ApiCall(detail) = &error else {
        panic!("expected ApiCall, got {error:?}");
    };
    assert_eq!(
        detail.message,
        "prompt: Invalid prompt\nimage_size.width: must be positive"
    );
    assert_eq!(detail.status_code, Some(400));

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/fal-ai/flux/schnell"))
        .respond_with(
            ResponseTemplate::new(500).set_body_json(json!({ "message": "Unknown fal error" })),
        )
        .mount(&server)
        .await;
    let error = flux_schnell(&server.uri())
        .do_generate(&options(PROMPT))
        .await
        .unwrap_err();
    let aimux_core::error::AiMuxError::ApiCall(detail) = &error else {
        panic!("expected ApiCall, got {error:?}");
    };
    assert_eq!(detail.message, "Unknown fal error");
}

/// fal-image-model.ts doGenerate: `providerMetadata.fal.images` carries each
/// image's metadata with `nsfw` taken from `has_nsfw_concepts[index]` or
/// `nsfw_content_detected[index]`, and the top-level response metadata keeps
/// every other property (upstream "providerMetaData for lora" / "for lcm").
#[tokio::test]
async fn should_merge_the_nsfw_flags_into_the_image_metadata() {
    let server = MockServer::start().await;
    let image = json!({
        "url": format!("{}/image.png", server.uri()),
        "width": 1024,
        "height": 1024,
        "content_type": "image/png",
        "file_data": "<image file_data>",
        "file_size": 123,
        "file_name": "<image file_name>",
    });
    mock_fal(
        &server,
        json!({
            "images": [image],
            "prompt": "<prompt>",
            "seed": 123,
            "has_nsfw_concepts": [true],
            "debug_latents": {
                "url": "<debug_latents url>",
                "content_type": "<debug_latents content_type>",
                "file_name": "<debug_latents file_name>",
                "file_data": "<debug_latents file_data>",
                "file_size": 123,
            },
        }),
    )
    .await;
    let result = flux_schnell(&server.uri())
        .do_generate(&options(PROMPT))
        .await
        .unwrap();
    let fal = Value::Object(result.provider_metadata.unwrap()["fal"].clone());
    assert_eq!(
        fal,
        json!({
            "images": [{
                "width": 1024,
                "height": 1024,
                "contentType": "image/png",
                "fileName": "<image file_name>",
                "fileData": "<image file_data>",
                "fileSize": 123,
                "nsfw": true,
            }],
            "seed": 123,
            "debug_latents": {
                "url": "<debug_latents url>",
                "content_type": "<debug_latents content_type>",
                "file_name": "<debug_latents file_name>",
                "file_data": "<debug_latents file_data>",
                "file_size": 123,
            },
        })
    );

    // The other flag, without `has_nsfw_concepts`, and null metadata kept.
    let server = MockServer::start().await;
    mock_fal(
        &server,
        json!({
            "images": [{
                "url": format!("{}/image.png", server.uri()),
                "width": 1024,
                "height": 1024,
                "file_name": null,
                "file_size": null,
            }],
            "seed": 123,
            "num_inference_steps": 456,
            "nsfw_content_detected": [false],
        }),
    )
    .await;
    let result = flux_schnell(&server.uri())
        .do_generate(&options(PROMPT))
        .await
        .unwrap();
    let fal = Value::Object(result.provider_metadata.unwrap()["fal"].clone());
    assert_eq!(
        fal,
        json!({
            "images": [{
                "width": 1024,
                "height": 1024,
                "fileName": null,
                "fileSize": null,
                "nsfw": false,
            }],
            "seed": 123,
            "num_inference_steps": 456,
        })
    );
}

/// fal-image-model.ts `supportsFileInputs` / `supportsMaskInputs`: a known
/// model advertises whether it takes files and a mask, an unknown one says
/// nothing (upstream "capabilities").
#[test]
fn should_advertise_file_and_mask_capabilities() {
    let provider = create_fal(FalProviderSettings {
        api_key: Some(Resolvable::Value("test-key".to_string())),
        ..Default::default()
    })
    .unwrap();
    for (model_id, file_inputs, mask_inputs) in [
        ("fal-ai/flux/dev/image-to-image", Some(true), Some(false)),
        ("fal-ai/flux-2/edit", Some(true), Some(false)),
        ("fal-ai/flux-pro/kontext", Some(true), Some(false)),
        ("fal-ai/flux-pro/kontext/max", Some(true), Some(false)),
        ("fal-ai/flux-general/inpainting", Some(true), Some(true)),
        ("fal-ai/recraft/v3/text-to-image", Some(false), Some(false)),
        ("custom/image-model", None, None),
        ("custom/image-to-image", None, None),
        ("custom/inpainting", None, None),
        ("custom/text-to-image", None, None),
        ("fal-ai/custom/image-to-image", None, None),
        ("fal-ai/custom/inpainting", None, None),
        ("fal-ai/custom/text-to-image", None, None),
    ] {
        let model = provider.image(model_id);
        assert_eq!(model.supports_file_inputs(), file_inputs, "{model_id}");
        assert_eq!(model.supports_mask_inputs(), mask_inputs, "{model_id}");
    }
}
