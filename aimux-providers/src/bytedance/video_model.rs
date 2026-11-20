//! Seedance asynchronous task protocol.

use async_trait::async_trait;
use serde_json::{Value, json};

use aimux_core::video_model::{
    VideoCallOptions, VideoData, VideoFile, VideoFrameType, VideoModel, VideoOperationStart,
    VideoOperationStatus, VideoResult,
};
use aimux_core::{AiMuxError, shared::SharedProviderMetadata};

use super::options::{check_types, failed_response, http_url, image_url, no_replay, options};
use crate::shared::{EndpointConfig, task_failed, video_response};

const PATH: &str = "/contents/generations/tasks";
const ALLOWED: &[&str] = &[
    "resolution",
    "ratio",
    "duration",
    "frames",
    "framespersecond",
    "seed",
    "generate_audio",
    "return_last_frame",
    "service_tier",
    "execution_expires_after",
    "callback_url",
    "imageUrls",
    "videoUrls",
    "audioUrls",
];

/// A Seedance model with resumable, JSON-serializable task references.
pub struct ByteDanceVideoModel {
    model_id: String,
    config: EndpointConfig,
}

impl ByteDanceVideoModel {
    pub(crate) fn new(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }

    fn body(&self, call: &VideoCallOptions) -> Result<Value, AiMuxError> {
        if call.n != 1 {
            return Err(AiMuxError::invalid_argument(
                "Seedance generates one video per task",
            ));
        }
        let prompt = call.prompt.as_deref().filter(|v| !v.trim().is_empty());
        let mut body = options(&call.provider_options, ALLOWED)?;
        check_types(
            &body,
            &["resolution", "ratio", "service_tier", "callback_url"],
            &[
                "duration",
                "frames",
                "framespersecond",
                "seed",
                "execution_expires_after",
            ],
            &["generate_audio", "return_last_frame"],
            &[],
        )?;
        if let Some(url) = body.get("callback_url").and_then(Value::as_str) {
            http_url(url)?;
        }
        if call.resolution.is_some() {
            return Err(AiMuxError::invalid_argument(
                "Seedance uses providerOptions.bytedance.resolution (e.g. 720p), not pixel dimensions",
            ));
        }
        for (name, value) in [
            ("duration", call.duration.map(|v| json!(v))),
            ("framespersecond", call.fps.map(|v| json!(v))),
            ("seed", call.seed.map(|v| json!(v))),
            ("ratio", call.aspect_ratio.map(|v| json!(v.to_string()))),
            ("generate_audio", call.generate_audio.map(|v| json!(v))),
        ] {
            if let Some(value) = value {
                body.entry(name).or_insert(value);
            }
        }
        let mut content = prompt
            .map(|text| vec![json!({"type":"text", "text":text})])
            .unwrap_or_default();
        if let Some(image) = &call.image {
            content.push(image_content(image, "first_frame")?);
        }
        for frame in call.frame_images.iter().flatten() {
            let role = match frame.frame_type {
                VideoFrameType::FirstFrame => "first_frame",
                VideoFrameType::LastFrame => "last_frame",
            };
            if role == "first_frame" && call.image.is_some() {
                return Err(AiMuxError::invalid_argument(
                    "Seedance received two first frames",
                ));
            }
            content.push(image_content(&frame.image, role)?);
        }
        for image in call.input_references.iter().flatten() {
            match image {
                VideoFile::Url {
                    url,
                    media_type: Some(kind),
                } if kind.starts_with("video/") => {
                    http_url(url)?;
                    content.push(json!({"type":"video_url", "video_url":{"url":url}, "role":"reference_video"}));
                }
                _ => content.push(image_content(image, "reference_image")?),
            }
        }
        for (key, kind, role) in [
            ("imageUrls", "image", "reference_image"),
            ("videoUrls", "video", "reference_video"),
            ("audioUrls", "audio", "reference_audio"),
        ] {
            if let Some(urls) = body.remove(key) {
                let urls = urls.as_array().ok_or_else(|| {
                    AiMuxError::invalid_argument(format!("{key} must be an array of URLs"))
                })?;
                for url in urls {
                    let url = url.as_str().ok_or_else(|| {
                        AiMuxError::invalid_argument(format!("{key} must contain strings"))
                    })?;
                    http_url(url)?;
                    let key = format!("{kind}_url");
                    content.push(json!({"type":key, key:{"url":url}, "role":role}));
                }
            }
        }
        if content.is_empty() {
            return Err(AiMuxError::invalid_argument(
                "Seedance requires a prompt or media reference",
            ));
        }
        let first = content
            .iter()
            .filter(|item| item["role"] == "first_frame")
            .count();
        let last = content
            .iter()
            .filter(|item| item["role"] == "last_frame")
            .count();
        let references = content.iter().any(|item| {
            item["role"]
                .as_str()
                .is_some_and(|role| role.starts_with("reference_"))
        });
        if first > 1 || last > 1 || (last == 1 && first == 0) || ((first + last > 0) && references)
        {
            return Err(AiMuxError::invalid_argument(
                "Seedance requires distinct first/last frames and cannot combine frames with references",
            ));
        }
        body.insert("model".into(), json!(self.model_id));
        body.insert("content".into(), json!(content));
        Ok(Value::Object(body))
    }
}

fn image_content(file: &VideoFile, role: &str) -> Result<Value, AiMuxError> {
    let url = match file {
        VideoFile::Url { url, media_type } => {
            if media_type
                .as_deref()
                .is_some_and(|kind| !kind.starts_with("image/"))
            {
                return Err(AiMuxError::invalid_argument(
                    "Seedance image inputs must use an image media type; use videoUrls/audioUrls for other references",
                ));
            }
            url.clone()
        }
        VideoFile::File { media_type, data } => {
            if !media_type.starts_with("image/") {
                return Err(AiMuxError::invalid_argument(
                    "Seedance inline references must be images",
                ));
            }
            format!(
                "data:{media_type};base64,{}",
                aimux_provider_utils::convert_to_base64(data)
            )
        }
    };
    image_url(&url)?;
    Ok(json!({"type":"image_url", "image_url":{"url":url}, "role":role}))
}

#[async_trait]
impl VideoModel for ByteDanceVideoModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }
    fn model_id(&self) -> &str {
        &self.model_id
    }
    fn max_videos_per_call(&self) -> Option<u32> {
        Some(1)
    }

    async fn do_start(&self, call: &VideoCallOptions) -> Result<VideoOperationStart, AiMuxError> {
        let body = self.body(call)?;
        let exchange = self.config.exchange(call.headers.as_ref()).await?;
        let response = aimux_provider_utils::post_json_to_api(
            exchange.request(exchange.url(PATH), call),
            exchange.transform_body(body),
            aimux_provider_utils::create_json_response_handler::<Value>(),
            failed_response(),
        )
        .await
        .map_err(no_replay)?;
        let id = response
            .value
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| AiMuxError::invalid_response_data("Seedance task response has no id"))?;
        Ok(VideoOperationStart {
            operation: json!({"id":id}),
            warnings: Vec::new(),
            provider_metadata: None,
            response: video_response(
                aimux_core::Timestamp::now(),
                &self.model_id,
                response.response_headers,
            ),
        })
    }

    async fn do_status(
        &self,
        operation: &Value,
        call: &VideoCallOptions,
    ) -> Result<VideoOperationStatus, AiMuxError> {
        let id = operation
            .get("id")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| AiMuxError::invalid_argument("Seedance operation has no id"))?;
        let exchange = self.config.exchange(call.headers.as_ref()).await?;
        let mut url = url::Url::parse(&exchange.url(PATH))
            .map_err(|_| AiMuxError::invalid_argument("invalid Seedance task URL"))?;
        url.path_segments_mut()
            .map_err(|()| AiMuxError::invalid_argument("invalid Seedance task URL"))?
            .push(id);
        let url = url.to_string();
        let response = aimux_provider_utils::get_from_api(
            exchange.request(url.clone(), call),
            aimux_provider_utils::create_json_response_handler::<Value>(),
            failed_response(),
        )
        .await?;
        let value = response.value;
        if value
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|returned| returned != id)
        {
            return Err(AiMuxError::invalid_response_data(
                "Seedance returned a different task id",
            ));
        }
        match value.get("status").and_then(Value::as_str) {
            Some("queued" | "running") => Ok(VideoOperationStatus::Pending),
            Some("succeeded") => {
                let video_url = value["content"]["video_url"].as_str().ok_or_else(|| {
                    AiMuxError::invalid_response_data("Seedance succeeded without a video URL")
                })?;
                http_url(video_url).map_err(|_| {
                    AiMuxError::invalid_response_data("Seedance returned a non-HTTP video URL")
                })?;
                let mut metadata = SharedProviderMetadata::new();
                let mut detail = serde_json::Map::new();
                detail.insert("taskId".into(), json!(id));
                if let Some(last) = value["content"].get("last_frame_url") {
                    detail.insert("lastFrameUrl".into(), last.clone());
                }
                metadata.insert("bytedance".into(), detail);
                Ok(VideoOperationStatus::Completed(VideoResult {
                    videos: vec![VideoData::Url {
                        url: video_url.to_owned(),
                        media_type: "video/mp4".into(),
                    }],
                    warnings: Vec::new(),
                    provider_metadata: Some(metadata),
                    response: video_response(
                        aimux_core::Timestamp::now(),
                        &self.model_id,
                        response.response_headers,
                    ),
                }))
            }
            Some(status @ ("failed" | "cancelled" | "expired")) => {
                let mut error = task_failed(
                    value["error"]["message"].as_str().unwrap_or(status),
                    url,
                    status,
                    Some(value.to_string()),
                );
                // Retry the status query for expired tasks; never resubmit generation.
                if status == "expired"
                    && let AiMuxError::ApiCall(detail) = &mut error
                {
                    detail.is_retryable = true;
                }
                Err(error)
            }
            _ => Err(AiMuxError::invalid_response_data(
                "unknown Seedance task status",
            )),
        }
    }
}
