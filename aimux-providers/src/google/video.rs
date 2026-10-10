//! Google video generation model — implements `VideoModel`.
//!
//! Aligned with Vercel AI SDK `GoogleVideoModel`
//! (`reference/ai/packages/google/src/google-video-model.ts`).
//!
//! Uses Google's Long Running Operations API:
//! 1. POST `{base_url}/models/{model}:predictLongRunning` → returns operation name
//! 2. GET `{base_url}/{operation_name}` — polled by Core via `do_status` until `done: true`
//! 3. Return video URL(s) from operation result

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::error::{AiMuxError, ApiCallError};
use aimux_core::video_model::{
    VideoCallOptions, VideoData, VideoModel, VideoOperationStart, VideoOperationStatus,
    VideoResponse, VideoResult,
};

use crate::shared::EndpointConfig;

/// A Google video generation model.
///
/// Does **not** hold an HTTP client — the `aimux-provider-utils` API helpers use the process-wide shared
/// `Client` internally (RFC-0009 §4.1).
pub struct GoogleVideoModel {
    model_id: String,
    config: EndpointConfig,
}

impl GoogleVideoModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }
}

#[async_trait]
impl VideoModel for GoogleVideoModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }
    fn model_id(&self) -> &str {
        &self.model_id
    }
    fn max_videos_per_call(&self) -> Option<u32> {
        Some(4)
    }

    async fn do_start(
        &self,
        options: &VideoCallOptions,
    ) -> Result<VideoOperationStart, AiMuxError> {
        use aimux_core::video_model::VideoFrameType;
        let mut warnings = Vec::new();
        let google = super::options::google_options(Some(&options.provider_options));
        if let Some(google) = google {
            for (key, value) in google.iter().filter(|(_, v)| !v.is_null()) {
                let valid = match key.as_str() {
                    "pollIntervalMs" | "pollTimeoutMs" => value.as_f64().is_some_and(|v| v > 0.0),
                    "personGeneration" => matches!(
                        value.as_str(),
                        Some("dont_allow" | "allow_adult" | "allow_all")
                    ),
                    "negativePrompt" => value.is_string(),
                    "referenceImages" => value.as_array().is_some_and(|images| {
                        images.iter().all(|image| {
                            image.as_object().is_some_and(|image| {
                                ["bytesBase64Encoded", "gcsUri"].iter().all(|key| {
                                    image.get(*key).is_none_or(|v| v.is_null() || v.is_string())
                                })
                            })
                        })
                    }),
                    _ => true,
                };
                if !valid {
                    return Err(AiMuxError::InvalidArgument(format!(
                        "Invalid Google video option: {key}"
                    )));
                }
            }
        }
        let mut instance = Map::new();
        if let Some(prompt) = &options.prompt {
            instance.insert("prompt".into(), json!(prompt));
        }
        let frames = options.frame_images.as_deref().unwrap_or_default();
        let first = frames
            .iter()
            .find(|f| f.frame_type == VideoFrameType::FirstFrame)
            .map(|f| &f.image)
            .or(options.image.as_ref());
        if let Some(image) = first.and_then(|f| convert_image(f, &mut warnings)) {
            instance.insert("image".into(), image);
        }
        if let Some(image) = frames
            .iter()
            .find(|f| f.frame_type == VideoFrameType::LastFrame)
            .and_then(|f| convert_image(&f.image, &mut warnings))
        {
            instance.insert("lastFrame".into(), image);
        }
        if frames.is_empty()
            && options
                .input_references
                .as_ref()
                .is_some_and(|r| !r.is_empty())
        {
            let references: Vec<Value> = options
                .input_references
                .iter()
                .flatten()
                .filter_map(|file| {
                    convert_image(file, &mut warnings)
                        .map(|image| json!({"image": image, "referenceType": "asset"}))
                })
                .collect();
            instance.insert("referenceImages".into(), json!(references));
        } else if let Some(references) = google
            .and_then(|g| g.get("referenceImages"))
            .and_then(Value::as_array)
        {
            instance.insert("referenceImages".into(), json!(references.iter().map(|image| {
                if let Some(bytes) = image.get("bytesBase64Encoded").and_then(Value::as_str).filter(|s| !s.is_empty()) { json!({"image": {"bytesBase64Encoded": bytes, "mimeType": "image/png"}, "referenceType": "asset"}) }
                else if let Some(uri) = image.get("gcsUri").and_then(Value::as_str).filter(|s| !s.is_empty()) { json!({"image": {"gcsUri": uri, "mimeType": "image/png"}, "referenceType": "asset"}) }
                else { image.clone() }
            }).collect::<Vec<_>>()));
        }
        let instances = vec![Value::Object(instance)];
        let mut parameters = Map::new();
        parameters.insert("sampleCount".into(), json!(options.n));
        if let Some(ar) = options.aspect_ratio {
            parameters.insert("aspectRatio".into(), json!(ar.to_string()));
        }
        if let Some(resolution) = options.resolution {
            let resolution = resolution.to_string();
            parameters.insert(
                "resolution".into(),
                json!(match resolution.as_str() {
                    "1280x720" => "720p",
                    "1920x1080" => "1080p",
                    "3840x2160" => "4k",
                    _ => &resolution,
                }),
            );
        }
        if let Some(seed) = options.seed.filter(|v| *v != 0) {
            parameters.insert("seed".into(), json!(seed));
        }
        if let Some(duration) = options.duration.filter(|v| *v != 0) {
            parameters.insert("durationSeconds".into(), json!(duration));
        }
        if let Some(google) = google {
            for (key, value) in google {
                if !matches!(
                    key.as_str(),
                    "pollIntervalMs" | "pollTimeoutMs" | "referenceImages"
                ) && !(matches!(key.as_str(), "personGeneration" | "negativePrompt")
                    && value.is_null())
                {
                    parameters.insert(key.clone(), value.clone());
                }
            }
        }

        let body = json!({
            "instances": instances,
            "parameters": Value::Object(parameters),
        });

        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let url = exchange.url(&format!("/models/{}:predictLongRunning", self.model_id));

        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(url, options),
            body,
            aimux_provider_utils::create_json_response_handler(),
            super::google_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;
        let predict_response: Value = resp.value;
        let operation_name = predict_response
            .get("name")
            .and_then(|v| v.as_str())
            .filter(|name| !name.is_empty())
            .ok_or_else(|| {
                AiMuxError::InvalidResponseData("No operation name returned from API".to_string())
            })?
            .to_string();

        Ok(VideoOperationStart {
            operation: json!({ "operationName": operation_name }),
            warnings,
            provider_metadata: None,
            response: VideoResponse {
                timestamp: Some(chrono::Utc::now().to_rfc3339()),
                model_id: Some(self.model_id.clone()),
                headers: Some(response_headers),
            },
        })
    }

    async fn do_status(
        &self,
        operation: &Value,
        options: &VideoCallOptions,
    ) -> Result<VideoOperationStatus, AiMuxError> {
        let operation_name = operation
            .get("operationName")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                AiMuxError::InvalidArgument(
                    "google operation reference is missing operationName".to_string(),
                )
            })?;

        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let poll_url = exchange.url(&format!("/{operation_name}"));
        let resp = aimux_provider_utils::get_from_api(
            exchange.request(poll_url.clone(), options),
            aimux_provider_utils::create_json_response_handler::<Value>(),
            super::google_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;
        let response_body = resp.raw_value.as_ref().map(ToString::to_string);
        let raw_body: Value = resp.value;
        if raw_body.get("done").and_then(serde_json::Value::as_bool) != Some(true) {
            return Ok(VideoOperationStatus::Pending);
        }

        // Check the in-band error first: a terminal response may carry both
        // done:true and an error object (provider-declared failure).
        if let Some(err) = raw_body.get("error").filter(|v| !v.is_null()) {
            let msg = err
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown error");
            return Err(AiMuxError::ApiCall(Box::new(ApiCallError {
                status_code: Some(200),
                provider_code: err
                    .get("status")
                    .and_then(|v| v.as_str())
                    .map(std::string::ToString::to_string),
                response_body,
                ..ApiCallError::new(msg, poll_url, serde_json::json!({}))
            })));
        }
        let outputs = raw_body
            .get("response")
            .and_then(|r| r.get("generateVideoResponse"))
            .and_then(|r| r.get("generatedSamples"))
            .and_then(Value::as_array)
            .filter(|outputs| !outputs.is_empty())
            .ok_or_else(|| {
                AiMuxError::InvalidResponseData(format!(
                    "No videos in response. Response: {raw_body}"
                ))
            })?;
        let endpoint = (self.config.endpoint)().await?;
        let api_key = endpoint
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("x-goog-api-key"))
            .and_then(|(_, value)| value.as_deref());
        let mut video_metadata = Vec::new();
        let videos: Vec<VideoData> = outputs
            .iter()
            .filter_map(|sample| {
                let uri = sample
                    .get("video")?
                    .get("uri")?
                    .as_str()
                    .filter(|uri| !uri.is_empty())?;
                video_metadata.push(json!({"uri": uri}));
                Some(VideoData::Url {
                    url: authenticated_video_url(uri, &endpoint.base_url, api_key),
                    media_type: "video/mp4".to_string(),
                })
            })
            .collect();
        if videos.is_empty() {
            return Err(AiMuxError::InvalidResponseData(
                "No valid videos in response".to_string(),
            ));
        }

        Ok(VideoOperationStatus::Completed(VideoResult {
            videos,
            warnings: Vec::new(),
            provider_metadata: Some(super::options::google_metadata(
                json!({"videos": video_metadata}),
            )),
            response: VideoResponse {
                timestamp: Some(chrono::Utc::now().to_rfc3339()),
                model_id: Some(self.model_id.clone()),
                headers: Some(response_headers),
            },
        }))
    }
}

fn convert_image(
    file: &aimux_core::video_model::VideoFile,
    warnings: &mut Vec<aimux_core::shared::Warning>,
) -> Option<Value> {
    use aimux_core::video_model::{VideoFile, VideoFileData};
    match file {
        VideoFile::Url { url, .. } if url.starts_with("gs://") => {
            Some(json!({"gcsUri": url, "mimeType": "image/png"}))
        }
        VideoFile::Url { .. } => {
            warnings.push(aimux_core::shared::Warning::Unsupported { feature: "URL-based image input".into(), details: Some("Google Generative AI video models require base64-encoded images or GCS URIs. URL will be ignored.".into()) });
            None
        }
        VideoFile::File { data, media_type } => {
            let data = match data {
                VideoFileData::Base64(data) => data.clone(),
                VideoFileData::Binary(data) => {
                    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, data)
                }
            };
            Some(
                json!({"bytesBase64Encoded": data, "mimeType": if media_type.is_empty() { "image/png" } else { media_type }}),
            )
        }
    }
}

fn authenticated_video_url(uri: &str, base_url: &str, key: Option<&str>) -> String {
    let same_origin = url::Url::parse(uri)
        .ok()
        .zip(url::Url::parse(base_url).ok())
        .is_some_and(|(uri, base)| uri.origin() == base.origin());
    if let Some(key) = key.filter(|_| same_origin) {
        format!(
            "{uri}{}key={key}",
            if uri.contains('?') { "&" } else { "?" }
        )
    } else {
        uri.into()
    }
}
