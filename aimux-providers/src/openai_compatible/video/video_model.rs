//! Multipart creation and asynchronous status for compatible `/videos` gateways.

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::AiMuxError;
use aimux_core::video_model::{
    VideoCallOptions, VideoData, VideoFile, VideoFileData, VideoFrameType, VideoModel,
    VideoOperationStart, VideoOperationStatus, VideoResponse, VideoResult,
};
use aimux_provider_utils::{HttpBody, HttpRequest, MultipartForm};

use super::super::CompatModelConfig;
use super::super::utils::to_camel_case;

/// An asynchronous video model served by an OpenAI-compatible gateway.
pub struct OpenAICompatibleVideoModel {
    model_id: String,
    config: CompatModelConfig,
}

impl OpenAICompatibleVideoModel {
    pub(crate) fn from_config(model_id: String, config: CompatModelConfig) -> Self {
        Self { model_id, config }
    }

    async fn form(&self, options: &VideoCallOptions) -> Result<MultipartForm, AiMuxError> {
        if options.n != 1 {
            return Err(AiMuxError::invalid_argument(
                "compatible video requests require n = 1",
            ));
        }
        let prompt = options
            .prompt
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| AiMuxError::invalid_argument("compatible video requires a prompt"))?;
        let name = self.config.provider_options_name();
        let camel = to_camel_case(name);
        let mut fields = Map::new();
        for namespace in ["openaiCompatible", name, camel.as_str()] {
            if let Some(values) = options.provider_options.get(namespace) {
                fields.extend(values.iter().map(|(k, v)| (k.clone(), v.clone())));
            }
        }
        // A reference is always a file part; a textual provider option with
        // this name would produce an ambiguous multipart request.
        if fields.contains_key("input_reference") {
            return Err(AiMuxError::invalid_argument(
                "use image, frameImages or inputReferences for input_reference files",
            ));
        }
        fields.insert("model".into(), json!(self.model_id));
        fields.insert("prompt".into(), json!(prompt));
        if let Some(duration) = options.duration {
            fields.insert("seconds".into(), json!(duration));
        }
        if let Some(size) = options.resolution {
            fields.insert("size".into(), json!(size.to_string()));
        }
        if let Some(ratio) = options.aspect_ratio {
            fields.insert("aspect_ratio".into(), json!(ratio.to_string()));
        }
        if let Some(fps) = options.fps {
            fields.insert("fps".into(), json!(fps));
        }
        if let Some(seed) = options.seed {
            fields.insert("seed".into(), json!(seed));
        }
        if let Some(audio) = options.generate_audio {
            fields.insert("generate_audio".into(), json!(audio));
        }
        let mut form = MultipartForm::new();
        for (key, value) in fields {
            let text = match value {
                Value::String(s) => s,
                other => other.to_string(),
            };
            form.text(&key, &text)?;
        }
        let frames = options.frame_images.as_deref().unwrap_or_default();
        let first_count = frames
            .iter()
            .filter(|f| f.frame_type == VideoFrameType::FirstFrame)
            .count();
        let last_count = frames
            .iter()
            .filter(|f| f.frame_type == VideoFrameType::LastFrame)
            .count();
        if first_count > 1 || last_count > 1 || (options.image.is_some() && first_count > 0) {
            return Err(AiMuxError::invalid_argument(
                "duplicate first or last video frame",
            ));
        }
        let mut files: Vec<&VideoFile> = options.image.iter().collect();
        for role in [VideoFrameType::FirstFrame, VideoFrameType::LastFrame] {
            files.extend(
                frames
                    .iter()
                    .filter(|f| f.frame_type == role)
                    .map(|f| &f.image),
            );
        }
        files.extend(options.input_references.iter().flatten());
        for (index, file) in files.into_iter().enumerate() {
            let (bytes, media_type) = file_bytes(file, options).await?;
            form.file(
                "input_reference",
                &format!("reference-{index}"),
                &media_type,
                &bytes,
            )?;
        }
        Ok(form)
    }

    async fn completed_video(
        &self,
        value: &Value,
        id: &str,
        options: &VideoCallOptions,
    ) -> Result<VideoData, AiMuxError> {
        let (content_url, base) = self
            .config
            .url(&format!("/videos/{}/content", encode_id(id)))?;
        let target = match value.get("url") {
            None | Some(Value::Null) => content_url,
            Some(Value::String(raw)) => match resolve_asset_url(&base, raw)? {
                AssetUrl::Public(url) => {
                    return Ok(VideoData::Url {
                        url,
                        media_type: "video/mp4".into(),
                    });
                }
                AssetUrl::Authenticated(url) => url,
            },
            Some(_) => {
                return Err(AiMuxError::invalid_response_data(
                    "video url must be a string",
                ));
            }
        };
        let headers = self
            .config
            .request_headers(options.headers.as_ref())
            .await?;
        let mut request = self.config.with_transport(
            HttpRequest::new(target, headers.list, options),
            base.clone(),
            headers.fetch_user_agent,
        );
        request.validate_url = true;
        request.trusted_origin = Some(base);
        let response = aimux_provider_utils::get_from_api(
            request,
            aimux_provider_utils::create_binary_response_handler(),
            aimux_provider_utils::create_status_code_error_response_handler(),
        )
        .await?;
        let media_type = response
            .response_headers
            .get("content-type")
            .map(|s| s.split(';').next().unwrap_or("video/mp4").to_string())
            .unwrap_or_else(|| "video/mp4".into());
        Ok(VideoData::Binary {
            data: response.value.to_vec(),
            media_type,
        })
    }
}

#[async_trait]
impl VideoModel for OpenAICompatibleVideoModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }
    fn model_id(&self) -> &str {
        &self.model_id
    }
    fn max_videos_per_call(&self) -> Option<u32> {
        Some(1)
    }

    async fn do_start(
        &self,
        options: &VideoCallOptions,
    ) -> Result<VideoOperationStart, AiMuxError> {
        let form = self.form(options).await?;
        let headers = self
            .config
            .request_headers(options.headers.as_ref())
            .await?;
        let (bytes, content_type) = form.finish();
        let response = aimux_provider_utils::post_to_api(
            self.config.http_request("/videos", headers, options)?,
            HttpBody::Bytes(bytes, content_type),
            aimux_provider_utils::create_json_response_handler::<Value>(),
            self.config.failed_response_handler(),
        )
        .await
        .map_err(no_replay)?;
        if let Some(status) = response.value.get("status").and_then(Value::as_str) {
            check_status(status, &response.value)?;
        }
        let id = response
            .value
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.trim().is_empty())
            .ok_or_else(|| AiMuxError::invalid_response_data("video creation returned no id"))?;
        Ok(VideoOperationStart {
            operation: json!({"id": id}),
            warnings: Vec::new(),
            provider_metadata: None,
            response: VideoResponse {
                timestamp: Some(aimux_core::Timestamp::now()),
                model_id: Some(self.model_id.clone()),
                headers: Some(response.response_headers),
            },
        })
    }

    async fn do_status(
        &self,
        operation: &Value,
        options: &VideoCallOptions,
    ) -> Result<VideoOperationStatus, AiMuxError> {
        let id = operation
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.trim().is_empty())
            .ok_or_else(|| AiMuxError::invalid_argument("video operation requires an id"))?;
        let headers = self
            .config
            .request_headers(options.headers.as_ref())
            .await?;
        let response = aimux_provider_utils::get_from_api(
            self.config
                .http_request(&format!("/videos/{}", encode_id(id)), headers, options)?,
            aimux_provider_utils::create_json_response_handler::<Value>(),
            self.config.failed_response_handler(),
        )
        .await?;
        if let Some(returned) = response.value.get("id")
            && returned.as_str() != Some(id)
        {
            return Err(AiMuxError::invalid_response_data(
                "video task id does not match the operation",
            ));
        }
        let status = response
            .value
            .get("status")
            .and_then(Value::as_str)
            .ok_or_else(|| AiMuxError::invalid_response_data("video task has no status"))?;
        if !check_status(status, &response.value)? {
            return Ok(VideoOperationStatus::Pending);
        }
        let video = self.completed_video(&response.value, id, options).await?;
        Ok(VideoOperationStatus::Completed(VideoResult {
            videos: vec![video],
            warnings: Vec::new(),
            provider_metadata: None,
            response: VideoResponse {
                timestamp: Some(aimux_core::Timestamp::now()),
                model_id: Some(self.model_id.clone()),
                headers: Some(response.response_headers),
            },
        }))
    }
}

fn check_status(status: &str, value: &Value) -> Result<bool, AiMuxError> {
    match status {
        "queued" | "pending" | "processing" | "in_progress" | "running" => Ok(false),
        "completed" | "succeeded" => Ok(true),
        "failed" | "cancelled" | "canceled" | "expired" | "error" => {
            let message = match value.get("error") {
                Some(Value::String(message)) => message.clone(),
                Some(error) if !error.is_null() => error.to_string(),
                _ => status.to_string(),
            };
            Err(AiMuxError::invalid_response_data(format!(
                "video generation failed: {message}"
            )))
        }
        _ => Err(AiMuxError::invalid_response_data(format!(
            "unknown video status: {status}"
        ))),
    }
}

fn no_replay(error: AiMuxError) -> AiMuxError {
    match error {
        AiMuxError::ApiCall(mut detail) => {
            detail.is_retryable = false;
            AiMuxError::ApiCall(detail)
        }
        other => other,
    }
}

fn encode_id(id: &str) -> String {
    match id {
        "." => "%252E".into(),
        ".." => "%252E%252E".into(),
        _ => percent_encoding::utf8_percent_encode(id, percent_encoding::NON_ALPHANUMERIC)
            .to_string(),
    }
}

enum AssetUrl {
    Public(String),
    Authenticated(String),
}

fn resolve_asset_url(base: &str, raw: &str) -> Result<AssetUrl, AiMuxError> {
    if raw.trim().is_empty() || raw != raw.trim() || raw.chars().any(char::is_control) {
        return Err(AiMuxError::invalid_response_data("invalid video asset URL"));
    }
    let base = url::Url::parse(&format!("{}/", base.trim_end_matches('/')))
        .map_err(|_| AiMuxError::invalid_response_data("invalid video base URL"))?;
    match url::Url::parse(raw) {
        Ok(url) => {
            if !matches!(url.scheme(), "http" | "https")
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
            {
                return Err(AiMuxError::invalid_response_data(
                    "video asset requires an HTTP(S) URL without userinfo",
                ));
            }
            Ok(AssetUrl::Public(url.to_string()))
        }
        Err(url::ParseError::RelativeUrlWithoutBase) => {
            let url = base.join(raw).map_err(|_| {
                AiMuxError::invalid_response_data("invalid relative video asset URL")
            })?;
            if url.origin() != base.origin()
                || !url.username().is_empty()
                || url.password().is_some()
            {
                return Err(AiMuxError::invalid_response_data(
                    "relative video asset URL must stay on the provider origin",
                ));
            }
            Ok(AssetUrl::Authenticated(url.to_string()))
        }
        Err(_) => Err(AiMuxError::invalid_response_data("invalid video asset URL")),
    }
}

async fn file_bytes(
    file: &VideoFile,
    options: &VideoCallOptions,
) -> Result<(Vec<u8>, String), AiMuxError> {
    match file {
        VideoFile::File { media_type, data } => {
            let bytes = match data {
                VideoFileData::Binary(bytes) => bytes.clone(),
                VideoFileData::Base64(text) => {
                    aimux_provider_utils::convert_base64_to_uint8_array(text).map_err(|_| {
                        AiMuxError::invalid_argument("invalid base64 video reference")
                    })?
                }
            };
            Ok((bytes, media_type.clone()))
        }
        VideoFile::Url { url, media_type } => {
            let response =
                aimux_provider_utils::download_blob(url.clone(), options.abort_signal.clone())
                    .await?;
            let media_type = media_type
                .clone()
                .or_else(|| response.response_headers.get("content-type").cloned())
                .unwrap_or_else(|| "application/octet-stream".into());
            Ok((response.value.to_vec(), media_type))
        }
    }
}
