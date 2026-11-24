//! Multipart creation and asynchronous status for compatible `/videos` gateways.

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::AiMuxError;
use aimux_core::video_model::{
    VideoCallOptions, VideoData, VideoFile, VideoFileData, VideoFrameType, VideoModel,
    VideoOperationStart, VideoOperationStatus, VideoResponse, VideoResult,
};
use aimux_provider_utils::{HttpBody, HttpRequest, MultipartForm};

use super::super::CompatModelConfig;
use super::super::provider::RequestHeaders;
use super::super::utils::to_camel_case;
use crate::shared::{retry_download, task_failed};

/// How long a failed result download waits before its next attempt.
const DOWNLOAD_RETRY_INTERVAL: Duration = Duration::from_millis(500);

/// The exchange guard of the two exchanges that move a whole media file: the
/// multipart creation (reference files) and the authenticated content
/// download. The 30s default is a hang guard for JSON exchanges; a transfer of
/// tens of megabytes can legitimately outlive it, and a creation read as timed
/// out counts as an accepted, billable task whose result is lost.
const MEDIA_TRANSFER_TIMEOUT: Duration = Duration::from_secs(600);

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
        // one of these names would produce an ambiguous multipart request.
        if fields.contains_key("input_reference") || fields.contains_key("input_references") {
            return Err(AiMuxError::invalid_argument(
                "use image, frameImages or inputReferences for reference file parts",
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
        // vLLM-Omni's `_parse_video_form` reads a singular `input_reference`
        // and an ordered `input_references` list; repeated scalar parts
        // collapse to the last file, which loses the first frame of a
        // first/last-frame pair. One file therefore goes in the singular
        // field, two or more in the repeated plural one.
        let field = if files.len() == 1 {
            "input_reference"
        } else {
            "input_references"
        };
        for (index, file) in files.into_iter().enumerate() {
            let (bytes, media_type) = file_bytes(file, options).await?;
            form.file(field, &format!("reference-{index}"), &media_type, &bytes)?;
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
        let (target, attach_credentials) = match value.get("url") {
            None | Some(Value::Null) => (content_url, true),
            Some(Value::String(raw)) => match resolve_asset_url(&self.config, &base, raw)? {
                AssetUrl::Public(url) => {
                    return Ok(VideoData::Url {
                        url,
                        media_type: "video/mp4".into(),
                    });
                }
                AssetUrl::Gateway(url) => (url, true),
                AssetUrl::Complete(url) => (url, false),
            },
            Some(_) => {
                return Err(AiMuxError::invalid_response_data(
                    "video url must be a string",
                ));
            }
        };
        let headers = if attach_credentials {
            self.config
                .request_headers(options.headers.as_ref())
                .await?
        } else {
            RequestHeaders {
                list: Vec::new(),
                fetch_user_agent: None,
            }
        };
        let request = self.content_request(target, base, headers, options, attach_credentials);
        // The task is already billed, so a transient download failure is
        // retried here; only an exhausted download becomes a non-retryable
        // error that never resubmits the creation.
        let response = retry_download(
            options.abort_signal.as_ref(),
            DOWNLOAD_RETRY_INTERVAL,
            || {
                aimux_provider_utils::get_from_api(
                    request.clone(),
                    aimux_provider_utils::create_binary_response_handler(),
                    aimux_provider_utils::create_status_code_error_response_handler(),
                )
            },
        )
        .await
        .map_err(no_replay)?;
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

    /// The multipart creation exchange. It carries the reference files, so it
    /// gets the media-transfer guard rather than the 30s default.
    fn creation_request(
        &self,
        headers: RequestHeaders,
        options: &VideoCallOptions,
    ) -> Result<HttpRequest, AiMuxError> {
        let mut request = self.config.http_request("/videos", headers, options)?;
        request.response_timeout = Some(MEDIA_TRANSFER_TIMEOUT);
        Ok(request)
    }

    /// The authenticated content download. A video is transferred whole in one
    /// exchange, so it gets the same guard as the creation upload; the SSRF
    /// guard and the credentialed-origin gate stay on, as for every download.
    fn content_request(
        &self,
        target: String,
        base: String,
        headers: RequestHeaders,
        options: &VideoCallOptions,
        attach_credentials: bool,
    ) -> HttpRequest {
        let trusted_origin = base.clone();
        let mut request = self.config.with_transport(
            HttpRequest::new(target, headers.list, options),
            base,
            headers.fetch_user_agent,
        );
        if !attach_credentials {
            // A complete target carries its own authorisation in the URL the
            // gateway supplied; the provider's credential must not be added as
            // a second scheme.
            request.credentialed_origin = None;
        }
        request.validate_url = true;
        request.trusted_origin = Some(trusted_origin);
        request.response_timeout = Some(MEDIA_TRANSFER_TIMEOUT);
        request
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
        let request = self.creation_request(headers, options)?;
        let creation_url = request.url.clone();
        let response = aimux_provider_utils::post_to_api(
            request,
            HttpBody::Bytes(bytes, content_type),
            aimux_provider_utils::create_json_response_handler::<Value>(),
            self.config.failed_response_handler(),
        )
        .await
        .map_err(not_retryable_unless_rate_limited)?;
        if let Some(status) = response.value.get("status").and_then(Value::as_str) {
            check_status(status, &response.value, &creation_url)?;
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
        let request =
            self.config
                .http_request(&format!("/videos/{}", encode_id(id)), headers, options)?;
        let status_url = request.url.clone();
        let response = aimux_provider_utils::get_from_api(
            request,
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
        if !check_status(status, &response.value, &status_url)? {
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

fn check_status(status: &str, value: &Value, url: &str) -> Result<bool, AiMuxError> {
    match status {
        "queued" | "pending" | "processing" | "in_progress" | "running" => Ok(false),
        "completed" | "succeeded" => Ok(true),
        "failed" | "cancelled" | "canceled" | "expired" | "error" => {
            let message = match value.get("error") {
                Some(Value::String(message)) => message.clone(),
                Some(error) if !error.is_null() => error.to_string(),
                _ => status.to_string(),
            };
            // The terminal status reaches Core as the provider code, so a
            // caller can tell an ended task from an accepted one; the task is
            // terminal, so the error is never retried.
            Err(task_failed(
                format!("video generation failed: {message}"),
                url,
                status,
                Some(value.to_string()),
            ))
        }
        _ => {
            // A name this list does not know may be a gateway alias for an
            // in-progress task; keep polling, bounded by the poll timeout, as
            // the other asynchronous video models do.
            Ok(false)
        }
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

/// Clear the retryability of a create, except for a rate limit.
///
/// A create the gateway refused for any other reason queued no task, so a
/// replay could bill twice. A rate-limited create (HTTP 429) did not run, and
/// replaying it is safe, so it stays retryable.
fn not_retryable_unless_rate_limited(error: AiMuxError) -> AiMuxError {
    match error {
        AiMuxError::ApiCall(mut detail) => {
            if detail.status_code != Some(429) {
                detail.is_retryable = false;
            }
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
    /// Another origin: Core downloads it without provider credentials.
    Public(String),
    /// A target on the gateway's own API: the configured query parameters the
    /// URL lacks are appended and the configured credentials are sent.
    Gateway(String),
    /// A complete target the response supplied with a query of its own: sent
    /// exactly as received, because that query may be the only thing that
    /// authorises it and a second scheme next to it would conflict.
    Complete(String),
}

/// Classify a completed video's `url`. An HTTP(S) URL on another origin stays
/// public for Core's uncredentialed downloader. On the provider's own origin,
/// the form of the reference decides whether the gateway's parameters and
/// credentials are added. A relative reference addresses the gateway's API, so
/// it is resolved against the base as a directory (and must stay on that
/// origin) and gets them. An absolute URL is a complete target: the gateway
/// named the whole address, and a query it carries may be a signature over
/// every parameter and its encoding, so the target is sent as received and
/// only a queryless one is treated as an asset route that needs the configured
/// authentication. A relative reference that carries its own authentication is
/// the same complete target, resolved.
fn resolve_asset_url(
    config: &CompatModelConfig,
    base: &str,
    raw: &str,
) -> Result<AssetUrl, AiMuxError> {
    if raw.trim().is_empty() || raw != raw.trim() || raw.chars().any(char::is_control) {
        return Err(AiMuxError::invalid_response_data("invalid video asset URL"));
    }
    let base = url::Url::parse(&format!("{}/", base.trim_end_matches('/')))
        .map_err(|_| AiMuxError::invalid_response_data("invalid video base URL"))?;
    let (url, relative) = match url::Url::parse(raw) {
        Ok(url) => (url, false),
        Err(url::ParseError::RelativeUrlWithoutBase) => {
            let url = base.join(raw).map_err(|_| {
                AiMuxError::invalid_response_data("invalid relative video asset URL")
            })?;
            (url, true)
        }
        Err(_) => return Err(AiMuxError::invalid_response_data("invalid video asset URL")),
    };
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(AiMuxError::invalid_response_data(
            "video asset requires an HTTP(S) URL without userinfo",
        ));
    }
    if url.origin() != base.origin() {
        if relative {
            return Err(AiMuxError::invalid_response_data(
                "relative video asset URL must stay on the provider origin",
            ));
        }
        return Ok(AssetUrl::Public(url.to_string()));
    }
    if relative {
        // A relative reference addresses the gateway's API, so it is requested
        // with the configured parameters and credentials. The one exception is
        // a reference whose own query authenticates it: that is resolved and
        // otherwise sent as received.
        if has_self_authenticating_query(&url) {
            return Ok(AssetUrl::Complete(url.to_string()));
        }
        return Ok(AssetUrl::Gateway(config.with_configured_query(&url)));
    }
    if url.query().is_some() {
        // The gateway supplied a complete target and chose its query, so it is
        // kept byte for byte: appending a configured parameter or re-encoding
        // the pairs would change a canonical query a signature covers, and a
        // provider credential next to the target's own is a second scheme.
        return Ok(AssetUrl::Complete(raw.to_owned()));
    }
    Ok(AssetUrl::Gateway(config.with_configured_query(&url)))
}

/// Whether the URL's own query authenticates it. Only a relative reference
/// needs the question answered: an absolute target's query is never touched.
/// The names cover the presigned link schemes of the object stores and CDNs
/// gateways commonly put in front of their media — SigV4 `X-Amz-*`, Cloud
/// Storage `X-Goog-*`, Azure SAS `sig`, CloudFront
/// `Signature`/`Policy`/`Key-Pair-Id` — and the plain signature and token
/// names compatible relays use, including Akamai `hdnts`, `hmac`, `jwt`,
/// `credential` and `auth`. The list supplements the structural rule: a
/// signature scheme outside it that appears in an absolute URL is preserved
/// without being recognized, and a new name is added here only for the
/// relative form, which is decided by form and then exempted by name.
fn has_self_authenticating_query(url: &url::Url) -> bool {
    url.query_pairs().any(|(name, _)| {
        let normalized: String = name
            .chars()
            .filter(|c| *c != '-' && *c != '_')
            .flat_map(char::to_lowercase)
            .collect();
        normalized == "sig"
            || normalized.ends_with("signature")
            || normalized == "awsaccesskeyid"
            || normalized == "policy"
            || normalized == "keypairid"
            || normalized == "token"
            || normalized == "accesstoken"
            || normalized == "authtoken"
            || normalized == "apikey"
            || normalized == "hdnts"
            || normalized == "hmac"
            || normalized == "jwt"
            || normalized == "credential"
            || normalized == "credentials"
            || normalized == "auth"
            || normalized.starts_with("xamz")
            || normalized.starts_with("xgoog")
    })
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use aimux_provider_utils::{Credential, ProviderHeaders};

    use super::super::super::ChatSettings;
    use super::super::super::provider::BaseUrl;
    use super::*;

    const BASE_URL: &str = "https://gateway.example/v1";

    /// A model over a gateway that authenticates with an `api-version` query
    /// parameter, the shape that loses `query_params` on asset resolution.
    fn model() -> OpenAICompatibleVideoModel {
        OpenAICompatibleVideoModel::from_config(
            "model".into(),
            CompatModelConfig {
                provider: "gateway.video".into(),
                base_url: BaseUrl::Fixed(BASE_URL.into()),
                query_params: Some(Arc::new(vec![("api-version".into(), "2024-01".into())])),
                headers: ProviderHeaders::bearer(Credential::None, Vec::new(), None),
                fetch_headers: None,
                fetch: None,
                body_rewrite: None,
                transform_request_body: None,
                chat: ChatSettings::default(),
            },
        )
    }

    fn headers() -> RequestHeaders {
        RequestHeaders {
            list: Vec::new(),
            fetch_user_agent: None,
        }
    }

    /// Both exchanges move a whole media file, so both declare the media
    /// guard instead of the 30s default; a creation timed out at 30s counts
    /// as an accepted, billable task whose result is lost.
    #[test]
    fn media_transfers_declare_the_wide_exchange_guard() {
        let model = model();
        let options = VideoCallOptions::new("cat");
        let creation = model.creation_request(headers(), &options).unwrap();
        assert_eq!(
            creation.url,
            "https://gateway.example/v1/videos?api-version=2024-01"
        );
        assert_eq!(creation.response_timeout, Some(MEDIA_TRANSFER_TIMEOUT));
        assert!(MEDIA_TRANSFER_TIMEOUT > Duration::from_secs(30));

        let download = model.content_request(
            "https://gateway.example/v1/videos/task/content?api-version=2024-01".into(),
            BASE_URL.into(),
            headers(),
            &options,
            true,
        );
        assert_eq!(download.response_timeout, Some(MEDIA_TRANSFER_TIMEOUT));
        assert!(download.validate_url);
        assert_eq!(download.trusted_origin.as_deref(), Some(BASE_URL));
        assert_eq!(download.credentialed_origin.as_deref(), Some(BASE_URL));

        let signed = model.content_request(
            "https://gateway.example/files/movie.mp4?X-Amz-Signature=abc".into(),
            BASE_URL.into(),
            headers(),
            &options,
            false,
        );
        assert_eq!(signed.response_timeout, Some(MEDIA_TRANSFER_TIMEOUT));
        assert!(signed.validate_url);
        assert_eq!(signed.trusted_origin.as_deref(), Some(BASE_URL));
        assert_eq!(signed.credentialed_origin, None);
    }

    /// An absolute asset URL without a query is a route on the gateway's own
    /// API, so the configured query parameters are appended and the configured
    /// credentials are sent.
    #[test]
    fn queryless_same_origin_asset_keeps_the_configured_query() {
        let AssetUrl::Gateway(url) = resolve_asset_url(
            &model().config,
            BASE_URL,
            "https://gateway.example/files/movie.mp4",
        )
        .unwrap() else {
            panic!("a queryless same-origin asset must stay credentialed")
        };
        assert_eq!(
            url,
            "https://gateway.example/files/movie.mp4?api-version=2024-01"
        );
    }

    /// A complete target the response supplied is sent as received: its query
    /// may be a signature or a token that covers every parameter and its
    /// encoding, so the configured parameters are not appended, a value is not
    /// replaced and the pairs are not re-encoded. The rule is the form of the
    /// URL, not the name of its parameters, so an unlisted signature scheme is
    /// preserved the same way.
    #[test]
    fn absolute_asset_with_its_own_query_is_sent_as_received() {
        let model = model();
        for raw in [
            // Akamai token authentication: no name the supplement knows.
            "https://gateway.example/files/movie.mp4?hdnts=exp=1700000000~hmac=abc",
            // A plain query the gateway chose, with a value it chose.
            "https://gateway.example/files/movie.mp4?v=2&api-version=2023-01",
            // Percent-encoded and form-encoded bytes stay exactly as received.
            "https://gateway.example/files/movie.mp4?a=exp%3D1%7Ehmac%3Dabc&b=x+y",
        ] {
            let AssetUrl::Complete(url) = resolve_asset_url(&model.config, BASE_URL, raw).unwrap()
            else {
                panic!("{raw} must be sent as received")
            };
            assert_eq!(url, raw);
        }
    }

    /// A relative reference is resolved as received: the query bytes the
    /// gateway wrote keep their order and encoding, and only the configured
    /// parameters the reference lacks are appended.
    #[test]
    fn relative_asset_keeps_the_received_query_bytes() {
        let AssetUrl::Gateway(url) = resolve_asset_url(
            &model().config,
            BASE_URL,
            "assets/movie.mp4?Expires=1&meta=exp%3D1%7Ehmac%3Dabc&a=b+c",
        )
        .unwrap() else {
            panic!("an unsigned relative asset must get the configured query")
        };
        assert_eq!(
            url,
            "https://gateway.example/v1/assets/movie.mp4?Expires=1&meta=exp%3D1%7Ehmac%3Dabc&a=b+c&api-version=2024-01"
        );
    }

    /// A relative reference whose own query authenticates it is a presigned
    /// target: it is only resolved against the base, with no configured
    /// parameters appended and no re-encoding.
    #[test]
    fn self_authenticating_relative_query_is_kept_exactly() {
        let model = model();
        for raw in [
            "assets/movie.mp4?X-Amz-Signature=abc%20def&X-Amz-Expires=900",
            "assets/movie.mp4?hdnts=exp=1700000000~hmac=abc",
            "assets/movie.mp4?hmac=abc&expires=1",
            "assets/movie.mp4?jwt=abc.def.ghi",
            "assets/movie.mp4?credential=abc",
            "assets/movie.mp4?auth=abc",
        ] {
            let AssetUrl::Complete(url) = resolve_asset_url(&model.config, BASE_URL, raw).unwrap()
            else {
                panic!("{raw} must stay as received")
            };
            assert_eq!(url, format!("https://gateway.example/v1/{raw}"), "{raw}");
        }
    }

    /// The signature and token parameter names of the common presigning
    /// schemes mark a relative reference as self-authenticating, in any
    /// casing; a reference whose parameters do not authenticate it keeps the
    /// configured query.
    #[test]
    fn presigning_parameter_names_are_recognized() {
        let model = model();
        for name in [
            "Signature",
            "signature",
            "sig",
            "X-Amz-Signature",
            "x-amz-signature",
            "X-Goog-Signature",
            "Policy",
            "Key-Pair-Id",
            "token",
            "access_token",
            "hdnts",
            "hmac",
            "jwt",
            "credential",
            "auth",
        ] {
            let raw = format!("assets/movie.mp4?{name}=v");
            let AssetUrl::Complete(url) = resolve_asset_url(&model.config, BASE_URL, &raw).unwrap()
            else {
                panic!("{name} must mark the reference as self-authenticating")
            };
            assert_eq!(url, format!("https://gateway.example/v1/{raw}"));
        }
        let AssetUrl::Gateway(url) =
            resolve_asset_url(&model.config, BASE_URL, "assets/movie.mp4?v=2").unwrap()
        else {
            panic!("a cache buster must keep the configured query")
        };
        assert_eq!(
            url,
            "https://gateway.example/v1/assets/movie.mp4?v=2&api-version=2024-01"
        );
    }

    /// A foreign absolute URL stays public: Core downloads it without the
    /// provider's credentials, and a signed query is kept for that download.
    #[test]
    fn foreign_absolute_asset_is_public() {
        let model = model();
        let resolved =
            resolve_asset_url(&model.config, BASE_URL, "https://cdn.example/movie.mp4").unwrap();
        let AssetUrl::Public(url) = resolved else {
            panic!("foreign asset must not be credentialed")
        };
        assert_eq!(url, "https://cdn.example/movie.mp4");

        let signed = "https://cdn.example/movie.mp4?X-Amz-Signature=abc&X-Amz-Expires=900";
        let AssetUrl::Public(url) = resolve_asset_url(&model.config, BASE_URL, signed).unwrap()
        else {
            panic!("foreign asset must not be credentialed")
        };
        assert_eq!(url, signed);
    }

    /// A relative URL resolves against the base as a directory and cannot
    /// leave the provider's origin.
    #[test]
    fn relative_asset_stays_on_the_provider_origin() {
        let model = model();
        let AssetUrl::Gateway(url) =
            resolve_asset_url(&model.config, BASE_URL, "assets/movie.mp4").unwrap()
        else {
            panic!("relative asset must stay credentialed")
        };
        assert_eq!(
            url,
            "https://gateway.example/v1/assets/movie.mp4?api-version=2024-01"
        );
        assert!(resolve_asset_url(&model.config, BASE_URL, "//cdn.example/movie.mp4").is_err());
    }

    /// An unknown status is a gateway alias this list does not know, not a
    /// terminal failure.
    #[test]
    fn unknown_status_keeps_polling() {
        assert!(!check_status("gateway_alias", &Value::Null, BASE_URL).unwrap());
        assert!(!check_status("queued", &Value::Null, BASE_URL).unwrap());
        assert!(check_status("completed", &Value::Null, BASE_URL).unwrap());
        assert!(check_status("failed", &Value::Null, BASE_URL).is_err());
    }

    /// Only a rate limit keeps a refused create retryable; every other status
    /// is terminal, and a transport timeout has no status at all.
    #[test]
    fn only_rate_limited_creates_stay_retryable() {
        for (status, retryable) in [(Some(429), true), (Some(500), false), (None, false)] {
            let error = not_retryable_unless_rate_limited(AiMuxError::ApiCall(Box::new(
                aimux_core::ApiCallError {
                    status_code: status,
                    is_retryable: true,
                    ..aimux_core::ApiCallError::new("failed", BASE_URL, Value::Null)
                },
            )));
            assert_eq!(error.is_retryable(), retryable, "{status:?}");
        }
    }
}
