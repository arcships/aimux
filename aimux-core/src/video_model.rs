//! The `VideoModel` trait — the provider-facing interface for video generation.
//!
//! Aligned with Vercel AI SDK `VideoModelV4`
//! (`reference/ai/packages/provider/src/video-model/v4/`).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use std::time::{Duration, Instant};

use crate::error::AiMuxError;
use crate::shared::{
    AspectRatio, SharedHeaders, SharedProviderMetadata, SharedProviderOptions, Size, Warning,
};
use crate::{AbortSignal, retry, timeout};

/// A video or image file used for video editing or image-to-video generation.
///
/// Aligned with V4 `VideoModelV4File`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum VideoFile {
    /// Inline file data (base64 or binary) with an explicit media type.
    File {
        /// IANA media type, e.g. `"video/mp4"` or `"image/png"`.
        media_type: String,
        /// File data as a base64 string or binary bytes.
        data: VideoFileData,
    },
    /// A URL pointing to the file.
    Url {
        /// The URL of the video or image file.
        url: String,
        /// The media type of the referenced file, when known.
        media_type: Option<String>,
    },
}

/// File payload for a [`VideoFile::File`]: base64 string or raw bytes.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum VideoFileData {
    /// Base64-encoded string.
    Base64(String),
    /// Raw binary bytes.
    Binary(Vec<u8>),
}

/// The role a frame image plays in video generation.
///
/// Aligned with V4 `VideoModelV4FrameType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum VideoFrameType {
    /// The starting frame the model animates from.
    FirstFrame,
    /// The ending frame the model animates towards.
    LastFrame,
}

/// A role-tagged image input for image-to-video and first-last-frame generation.
///
/// Aligned with V4 `VideoModelV4FrameImage`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct VideoFrameImage {
    /// The image file used for this frame.
    pub image: VideoFile,
    /// Which frame this image represents.
    pub frame_type: VideoFrameType,
}

/// Generated video data: a URL, base64-encoded string, or binary data.
///
/// Aligned with V4 `VideoModelV4VideoData`. Most providers return URLs due to
/// large file sizes.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum VideoData {
    /// Video available as a URL (most common).
    Url { url: String, media_type: String },
    /// Video as a base64-encoded string.
    Base64 { data: String, media_type: String },
    /// Video as binary data.
    Binary { data: Vec<u8>, media_type: String },
}

/// Options passed to [`VideoModel::do_start`] and [`VideoModel::do_status`].
///
/// Aligned with V4 `VideoModelV4CallOptions`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct VideoCallOptions {
    /// Text prompt for the video generation. `None` when not required.
    pub prompt: Option<String>,

    /// Number of videos to generate. Default `1`; most models only support
    /// `n = 1` due to computational cost.
    // serde default: typed binding structs omit unset fields, and a missing
    // `n` must mean 1, not a hard deserialization failure at the FFI boundary.
    #[serde(default = "default_video_n")]
    pub n: u32,

    /// Aspect ratio, in `{width}:{height}` format (e.g. `"16:9"`).
    pub aspect_ratio: Option<AspectRatio>,

    /// Resolution, in `{width}x{height}` format (e.g. `"1280x720"`).
    pub resolution: Option<Size>,

    /// Duration of the video in seconds. Typically 3–10 seconds.
    pub duration: Option<u32>,

    /// Frames per second. Common values: 24, 30, 60.
    pub fps: Option<u32>,

    /// Seed for deterministic generation. `None` uses a random seed.
    #[ts(type = "number | null")]
    pub seed: Option<u64>,

    /// Input image for image-to-video generation (the starting frame).
    pub image: Option<VideoFile>,

    /// Role-tagged image inputs for first-last-frame generation.
    pub frame_images: Option<Vec<VideoFrameImage>>,

    /// Reference inputs for reference-to-video generation (images or videos).
    pub input_references: Option<Vec<VideoFile>>,

    /// Whether the model should generate audio alongside the video.
    pub generate_audio: Option<bool>,

    /// Additional provider-specific options, keyed by provider name.
    #[serde(default)]
    pub provider_options: SharedProviderOptions,

    /// Abort signal for cancelling the operation.
    #[serde(skip)]
    #[ts(skip)]
    pub abort_signal: Option<AbortSignal>,

    /// Per-call retry override. `None` uses the model default.
    pub max_retries: Option<u32>,

    /// Per-call poll pacing override for the start/status flow. Unset fields
    /// fall back to the model's [`VideoModel::poll_config`].
    pub poll: Option<VideoPollOptions>,

    /// Per-call operation timeout.
    pub timeout: Option<crate::options::TimeoutConfiguration>,

    /// Additional HTTP headers to send with the request.
    pub headers: Option<SharedHeaders>,
}

fn default_video_n() -> u32 {
    1
}

impl VideoCallOptions {
    /// Create options with a prompt and `n = 1`, all other fields defaulted.
    pub fn new(prompt: impl Into<String>) -> Self {
        Self {
            prompt: Some(prompt.into()),
            n: 1,
            aspect_ratio: None,
            resolution: None,
            duration: None,
            fps: None,
            seed: None,
            image: None,
            frame_images: None,
            input_references: None,
            generate_audio: None,
            provider_options: SharedProviderOptions::new(),
            abort_signal: None,
            max_retries: None,
            poll: None,
            timeout: None,
            headers: None,
        }
    }
}

/// Per-call poll pacing for video generation (AI SDK `generateVideo` `poll`).
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct VideoPollOptions {
    /// Delay between consecutive status checks, in milliseconds.
    // `number`, not the `bigint` ts-rs infers from u64: the JS bindings pass
    // options through `JSON.stringify`, which throws on BigInt.
    #[ts(type = "number | null")]
    pub interval_ms: Option<u64>,
    /// Maximum total time to wait for completion, in milliseconds.
    #[ts(type = "number | null")]
    pub timeout_ms: Option<u64>,
}

/// The final result of a video generation operation.
///
/// Aligned with V4 `VideoModelV4Result`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct VideoResult {
    /// Generated videos as URLs, base64 strings, or binary data.
    pub videos: Vec<VideoData>,

    /// Warnings for the call, e.g. unsupported features.
    pub warnings: Vec<Warning>,

    /// Additional provider-specific metadata, keyed by provider name.
    pub provider_metadata: Option<SharedProviderMetadata>,

    /// Response information for telemetry and debugging.
    pub response: VideoResponse,
}

/// Response information for a video generation call.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct VideoResponse {
    /// Timestamp for the start of the generated response (ISO 8601 string).
    pub timestamp: Option<String>,
    /// The ID of the model that was used to generate the response.
    pub model_id: Option<String>,
    /// Response headers.
    pub headers: Option<SharedHeaders>,
}

/// Result of starting an asynchronous video generation via
/// [`VideoModel::do_start`].
///
/// Aligned with V4 `VideoModelV4OperationStartResult`.
#[derive(Debug, Clone)]
pub struct VideoOperationStart {
    /// JSON-serializable opaque reference passed to [`VideoModel::do_status`]
    /// to check the status of the generation (e.g. a task ID or poll URL).
    pub operation: serde_json::Value,

    /// Warnings for the start call, e.g. unsupported features.
    pub warnings: Vec<Warning>,

    /// Additional provider-specific metadata from the start call.
    pub provider_metadata: Option<SharedProviderMetadata>,

    /// Response information for telemetry and debugging.
    pub response: VideoResponse,
}

/// Status of an asynchronous video generation, from [`VideoModel::do_status`].
///
/// Aligned with V4 `VideoModelV4OperationStatusResult`; its `error` arm maps
/// to `Err(AiMuxError)` (a terminally failed task should be non-retryable).
#[derive(Debug, Clone)]
pub enum VideoOperationStatus {
    /// The generation is still in progress; poll again later.
    Pending,
    /// The generation is complete.
    Completed(VideoResult),
}

/// Pacing for the Core-owned status poll loop.
#[derive(Debug, Clone, Copy)]
pub struct VideoPollConfig {
    /// Delay between consecutive `do_status` calls.
    pub interval: Duration,
    /// Maximum total time to wait for the generation to complete.
    pub timeout: Duration,
}

impl Default for VideoPollConfig {
    fn default() -> Self {
        // AI SDK generate-video defaults (intervalMs: 5000, timeoutMs: 600_000).
        Self {
            interval: Duration::from_secs(5),
            timeout: Duration::from_secs(600),
        }
    }
}

/// The unified video generation model trait (provider-facing).
///
/// Aligned with V4 `VideoModelV4`'s asynchronous `doStart`/`doStatus` flow.
/// Every current provider API is task-based (create, then poll), so the
/// synchronous `doGenerate` arm is not offered: the polling loop lives in
/// [`generate_video`], and each phase is retried independently against the
/// same operation reference.
#[async_trait]
pub trait VideoModel: Send + Sync {
    /// Specification version (always `"v4"`).
    fn specification_version(&self) -> &'static str {
        "v4"
    }

    /// Provider name, e.g. `"fal"`.
    fn provider(&self) -> &str;

    /// Provider-specific model ID, e.g. `"kling-video"`.
    fn model_id(&self) -> &str;

    fn retry_config(&self) -> retry::RetryConfig {
        retry::RetryConfig::default()
    }

    /// Poll pacing for this model. Defaults to the AI SDK values; providers
    /// with configurable polling should surface their configuration here.
    fn poll_config(&self) -> VideoPollConfig {
        VideoPollConfig::default()
    }

    /// Limit of how many videos can be generated in a single API call.
    ///
    /// `None` means no fixed limit. Most video models only support `1`.
    fn max_videos_per_call(&self) -> Option<u32>;

    /// Start an asynchronous video generation and return an opaque operation
    /// reference for [`Self::do_status`].
    ///
    /// Naming: the `do_` prefix prevents accidental direct usage by users.
    async fn do_start(&self, options: &VideoCallOptions)
    -> Result<VideoOperationStart, AiMuxError>;

    /// Check the status of a generation started with [`Self::do_start`].
    ///
    /// A task that failed server-side should be reported as a non-retryable
    /// `Err`, not `Pending`, so the poll loop stops immediately.
    async fn do_status(
        &self,
        operation: &serde_json::Value,
        options: &VideoCallOptions,
    ) -> Result<VideoOperationStatus, AiMuxError>;
}

/// User-facing video generation with Core-owned retry, polling, batching, and
/// timeout.
///
/// Orchestration (AI SDK `generateVideo`, mirroring `generateImage`'s
/// batching): `options.n` is split into `VideoModel::max_videos_per_call`
/// sized batches, and every batch runs a full, independent `do_start`/poll
/// cycle **concurrently** (the AI SDK batches images/videos via
/// `Promise.all`, not sequentially) — each with its own idempotency key, so a
/// retried batch never collides with another batch's replay. Within a batch,
/// `do_start` is retried as one unit, then `do_status` is polled — each poll
/// retried independently — against that batch's operation reference, so a
/// transient poll failure never re-creates the billed task. Batch results are
/// flattened back in batch order; provider metadata across batches is
/// deep-merged the same way start/completion metadata is within a batch (see
/// [`merge_provider_metadata`]).
///
/// # Errors
///
/// Returns `InvalidArgument` when `options.n == 0` (checked before any
/// network call), or the first batch's provider failure, retry exhaustion,
/// poll or operation timeout, or caller abort — Rust drops the other
/// in-flight batches' futures on the first error, unlike `Promise.all`,
/// which lets sibling settles run to completion; their partial network
/// effects (e.g. an already-started but abandoned batch) are the same
/// either way, since nothing here reconnects to or cancels a provider job.
pub async fn generate_video(
    model: &dyn VideoModel,
    options: VideoCallOptions,
) -> Result<VideoResult, AiMuxError> {
    let timeout = timeout::OperationTimeout::new(options.timeout.unwrap_or_default())?;
    let abort_signal = options.abort_signal.clone();
    timeout::run(
        start_and_poll(model, options),
        abort_signal.as_ref(),
        timeout,
    )
    .await
}

async fn start_and_poll(
    model: &dyn VideoModel,
    options: VideoCallOptions,
) -> Result<VideoResult, AiMuxError> {
    if options.n == 0 {
        return Err(AiMuxError::InvalidArgument(
            "video generation `n` must be at least 1".to_string(),
        ));
    }

    let abort_signal = options.abort_signal.clone();
    let retries = retry::prepare_retries(
        options.max_retries,
        model.retry_config(),
        abort_signal.clone(),
    );

    let mut poll = model.poll_config();
    if let Some(overrides) = options.poll {
        if let Some(ms) = overrides.interval_ms {
            poll.interval = Duration::from_millis(ms);
        }
        if let Some(ms) = overrides.timeout_ms {
            poll.timeout = Duration::from_millis(ms);
        }
    }

    let max_per_call = model.max_videos_per_call().unwrap_or(options.n).max(1);
    let batch_counts = batch_video_counts(options.n, max_per_call);

    let batches = batch_counts.into_iter().map(|n| {
        let mut batch_options = options.clone();
        batch_options.n = n;
        start_and_poll_one_batch(model, batch_options, &retries, poll, abort_signal.clone())
    });
    let results = futures::future::try_join_all(batches).await?;

    let mut videos = Vec::new();
    let mut warnings = Vec::new();
    let mut provider_metadata = None;
    let mut response = None;
    for result in results {
        videos.extend(result.videos);
        warnings.extend(result.warnings);
        provider_metadata = merge_provider_metadata(provider_metadata, result.provider_metadata);
        response.get_or_insert(result.response);
    }

    Ok(VideoResult {
        videos,
        warnings,
        provider_metadata,
        // `batch_counts` is never empty (`n >= 1` was checked above), so at
        // least one batch ran and `response` is always `Some` here; the first
        // batch's response represents the call for telemetry purposes.
        response: response.unwrap_or_default(),
    })
}

/// Split `n` into `max_per_call`-sized batches (AI SDK `generateVideo` /
/// `generateImage`): every batch but the last is full-sized; the last batch
/// takes the remainder, or a full batch if `n` divides evenly.
fn batch_video_counts(n: u32, max_per_call: u32) -> Vec<u32> {
    let max_per_call = max_per_call.max(1);
    let call_count = n.div_ceil(max_per_call);
    (0..call_count)
        .map(|i| {
            if i + 1 < call_count {
                max_per_call
            } else {
                let remainder = n % max_per_call;
                if remainder == 0 {
                    max_per_call
                } else {
                    remainder
                }
            }
        })
        .collect()
}

/// Run one `do_start` + poll cycle for a single provider-sized batch.
///
/// Each batch mints its own idempotency key (AI SDK batching semantics): a
/// batch, not the whole `n`-video request, is the unit of idempotent replay.
async fn start_and_poll_one_batch(
    model: &dyn VideoModel,
    options: VideoCallOptions,
    retries: &retry::PreparedRetries,
    poll: VideoPollConfig,
    abort_signal: Option<AbortSignal>,
) -> Result<VideoResult, AiMuxError> {
    // `do_start` is billable: mint one idempotency key per logical start,
    // OUTSIDE the retry closure, so providers that honor this header can
    // deduplicate a replay where the first attempt succeeded but its response
    // was lost. A caller-supplied key wins (AI SDK generate-video parity).
    let mut start_options = options.clone();
    let headers = start_options.headers.get_or_insert_with(SharedHeaders::new);
    if !headers
        .keys()
        .any(|k| k.eq_ignore_ascii_case("idempotency-key"))
    {
        headers.insert(
            "idempotency-key".to_string(),
            format!("aimux_vid_{:016x}", rand::random::<u64>()),
        );
    }

    let start = retries.retry(|| model.do_start(&start_options)).await?;

    let poll_started = Instant::now();
    loop {
        let elapsed = poll_started.elapsed();
        if elapsed >= poll.timeout {
            return Err(AiMuxError::Timeout(format!(
                "Video generation timed out after {:?}.",
                poll.timeout
            )));
        }
        retry::delay(
            poll.interval.min(poll.timeout - elapsed),
            abort_signal.as_ref(),
        )
        .await?;

        if poll_started.elapsed() >= poll.timeout {
            return Err(AiMuxError::Timeout(format!(
                "Video generation timed out after {:?}.",
                poll.timeout
            )));
        }

        // Matching the AI SDK, the poll budget paces the loop between status
        // checks; a hung status GET is already bounded by the per-exchange
        // response guard in provider-utils, and the retry count is finite.
        let status = retries
            .retry(|| model.do_status(&start.operation, &options))
            .await?;
        match status {
            VideoOperationStatus::Pending => {}
            VideoOperationStatus::Completed(mut result) => {
                // Start-call warnings/metadata precede the completion's own.
                let mut warnings = start.warnings;
                warnings.append(&mut result.warnings);
                result.warnings = warnings;
                result.provider_metadata =
                    merge_provider_metadata(start.provider_metadata, result.provider_metadata);
                return Ok(result);
            }
        }
    }
}

/// Merge two phases' provider metadata one level deep.
///
/// A plain `HashMap::entry().or_insert()` at the provider-key level drops
/// every field the other phase set once both phases report the *same*
/// provider key — e.g. a start call's `job_id` disappearing once the
/// completion call also reports `x_provider` metadata. Instead, when both
/// sides have an object for the same provider key, their fields are unioned;
/// on a field collision the later (`completion`/`b`) value wins. When a
/// provider key appears on only one side, or either side's value for it is
/// not a JSON object, that side's value is used as-is — there is nothing to
/// union.
fn merge_provider_metadata(
    a: Option<SharedProviderMetadata>,
    b: Option<SharedProviderMetadata>,
) -> Option<SharedProviderMetadata> {
    let Some(mut merged) = a else { return b };
    let Some(b) = b else { return Some(merged) };
    for (provider, b_value) in b {
        match merged.get_mut(&provider) {
            Some(a_value) => match (a_value.as_object_mut(), b_value.as_object()) {
                (Some(a_obj), Some(b_obj)) => {
                    for (field, value) in b_obj {
                        a_obj.insert(field.clone(), value.clone());
                    }
                }
                _ => *a_value = b_value,
            },
            None => {
                merged.insert(provider, b_value);
            }
        }
    }
    Some(merged)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;
    use crate::error::ApiCallError;

    /// `do_status` behavior per call index, cycled through in order.
    enum StatusStep {
        RetryableFailure,
        Pending,
        Complete,
    }

    struct ScriptedVideoModel {
        starts: AtomicU32,
        statuses: AtomicU32,
        script: Vec<StatusStep>,
        start_failures: u32,
        start_idempotency_keys: Mutex<Vec<Option<String>>>,
        status_idempotency_keys: Mutex<Vec<Option<String>>>,
    }

    impl ScriptedVideoModel {
        fn new(script: Vec<StatusStep>) -> Self {
            Self {
                starts: AtomicU32::new(0),
                statuses: AtomicU32::new(0),
                script,
                start_failures: 0,
                start_idempotency_keys: Mutex::new(Vec::new()),
                status_idempotency_keys: Mutex::new(Vec::new()),
            }
        }

        fn with_start_failures(mut self, failures: u32) -> Self {
            self.start_failures = failures;
            self
        }
    }

    fn idempotency_key(options: &VideoCallOptions) -> Option<String> {
        options.headers.as_ref().and_then(|headers| {
            headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("idempotency-key"))
                .map(|(_, value)| value.clone())
        })
    }

    #[async_trait]
    impl VideoModel for ScriptedVideoModel {
        fn provider(&self) -> &str {
            "test"
        }

        fn model_id(&self) -> &str {
            "scripted"
        }

        fn retry_config(&self) -> crate::retry::RetryConfig {
            crate::retry::RetryConfig {
                initial_delay: Duration::from_millis(1),
                ..crate::retry::RetryConfig::default()
            }
        }

        fn max_videos_per_call(&self) -> Option<u32> {
            Some(1)
        }

        async fn do_start(
            &self,
            options: &VideoCallOptions,
        ) -> Result<VideoOperationStart, AiMuxError> {
            let attempt = self.starts.fetch_add(1, Ordering::SeqCst);
            let key = idempotency_key(options);
            self.start_idempotency_keys
                .lock()
                .expect("start key capture lock should not be poisoned")
                .push(key.clone());
            // The Core-minted idempotency key must reach the start request.
            assert!(
                key.is_some(),
                "do_start should receive an idempotency-key header"
            );
            if attempt < self.start_failures {
                return Err(AiMuxError::ApiCall(Box::new(ApiCallError {
                    status_code: Some(503),
                    is_retryable: true,
                    ..ApiCallError::new(
                        "start unavailable",
                        "https://test/start",
                        serde_json::json!({}),
                    )
                })));
            }
            Ok(VideoOperationStart {
                operation: serde_json::json!({ "task_id": "t-1" }),
                warnings: vec![Warning::Other {
                    message: "from start".to_string(),
                }],
                provider_metadata: None,
                response: VideoResponse::default(),
            })
        }

        async fn do_status(
            &self,
            operation: &serde_json::Value,
            options: &VideoCallOptions,
        ) -> Result<VideoOperationStatus, AiMuxError> {
            assert_eq!(operation["task_id"], "t-1");
            self.status_idempotency_keys
                .lock()
                .expect("status key capture lock should not be poisoned")
                .push(idempotency_key(options));
            let i = self.statuses.fetch_add(1, Ordering::SeqCst) as usize;
            match self.script.get(i).unwrap_or(&StatusStep::Complete) {
                StatusStep::RetryableFailure => Err(AiMuxError::ApiCall(Box::new(ApiCallError {
                    status_code: Some(429),
                    is_retryable: true,
                    ..ApiCallError::new("rate limited", "https://test", serde_json::json!({}))
                }))),
                StatusStep::Pending => Ok(VideoOperationStatus::Pending),
                StatusStep::Complete => Ok(VideoOperationStatus::Completed(VideoResult {
                    videos: vec![VideoData::Url {
                        url: "https://cdn/video.mp4".to_string(),
                        media_type: "video/mp4".to_string(),
                    }],
                    warnings: vec![Warning::Other {
                        message: "from status".to_string(),
                    }],
                    provider_metadata: None,
                    response: VideoResponse::default(),
                })),
            }
        }
    }

    fn fast_poll_options() -> VideoCallOptions {
        let mut options = VideoCallOptions::new("a cat");
        options.poll = Some(VideoPollOptions {
            interval_ms: Some(1),
            timeout_ms: Some(2_000),
        });
        options
    }

    /// The core guarantee of the do_start/do_status split: a transient poll
    /// failure retries the status check against the same operation and never
    /// re-creates the billed task.
    #[tokio::test]
    async fn transient_status_failure_never_restarts_the_task() {
        let model = ScriptedVideoModel::new(vec![
            StatusStep::Pending,
            StatusStep::RetryableFailure,
            StatusStep::Complete,
        ]);
        let result = generate_video(&model, fast_poll_options()).await.unwrap();
        assert_eq!(model.starts.load(Ordering::SeqCst), 1);
        assert_eq!(model.statuses.load(Ordering::SeqCst), 3);
        assert!(
            model
                .status_idempotency_keys
                .lock()
                .unwrap()
                .iter()
                .all(Option::is_none),
            "the Core-minted start key must not be forwarded to status calls"
        );
        assert_eq!(result.videos.len(), 1);
    }

    #[tokio::test]
    async fn start_retry_reuses_one_idempotency_key() {
        let model = ScriptedVideoModel::new(vec![StatusStep::Complete]).with_start_failures(1);
        let mut options = fast_poll_options();
        options.max_retries = Some(1);

        let result = generate_video(&model, options).await.unwrap();

        assert_eq!(model.starts.load(Ordering::SeqCst), 2);
        assert_eq!(result.videos.len(), 1);
        let keys = model
            .start_idempotency_keys
            .lock()
            .expect("start key capture lock should not be poisoned");
        assert_eq!(keys.len(), 2);
        assert!(keys[0].is_some());
        assert_eq!(keys[0], keys[1]);
        assert_eq!(
            model
                .status_idempotency_keys
                .lock()
                .expect("status key capture lock should not be poisoned")
                .as_slice(),
            &[None]
        );
    }

    #[tokio::test]
    async fn caller_idempotency_key_is_preserved_for_start_and_status() {
        let model = ScriptedVideoModel::new(vec![StatusStep::Complete]);
        let mut options = fast_poll_options();
        options
            .headers
            .get_or_insert_with(SharedHeaders::new)
            .insert("Idempotency-Key".to_string(), "caller-key".to_string());

        generate_video(&model, options).await.unwrap();

        assert_eq!(
            model
                .start_idempotency_keys
                .lock()
                .expect("start key capture lock should not be poisoned")
                .as_slice(),
            &[Some("caller-key".to_string())]
        );
        assert_eq!(
            model
                .status_idempotency_keys
                .lock()
                .expect("status key capture lock should not be poisoned")
                .as_slice(),
            &[Some("caller-key".to_string())]
        );
    }

    #[tokio::test]
    async fn start_warnings_precede_completion_warnings() {
        let model = ScriptedVideoModel::new(vec![StatusStep::Complete]);
        let result = generate_video(&model, fast_poll_options()).await.unwrap();
        let messages: Vec<String> = result.warnings.iter().map(|w| format!("{w:?}")).collect();
        assert!(messages[0].contains("from start"), "{messages:?}");
        assert!(messages[1].contains("from status"), "{messages:?}");
    }

    #[test]
    fn merge_provider_metadata_unions_same_provider_key_across_phases() {
        let start = SharedProviderMetadata::from([(
            "fal".to_string(),
            serde_json::json!({ "job_id": "job-1", "region": "us-east" }),
        )]);
        let completion = SharedProviderMetadata::from([(
            "fal".to_string(),
            serde_json::json!({ "region": "eu-west", "seed": 42 }),
        )]);

        let merged = merge_provider_metadata(Some(start), Some(completion))
            .expect("both phases reported metadata");

        // `job_id` only came from the start phase and must survive the merge
        // instead of being dropped by an `entry().or_insert()` collision.
        assert_eq!(
            merged["fal"],
            serde_json::json!({ "job_id": "job-1", "region": "eu-west", "seed": 42 }),
            "expected start-only fields preserved and completion to win on collision"
        );
    }

    #[tokio::test]
    async fn poll_timeout_fails_a_generation_that_never_completes() {
        let model = ScriptedVideoModel::new(vec![]);
        // Script that always reports Pending.
        struct AlwaysPending(ScriptedVideoModel);
        #[async_trait]
        impl VideoModel for AlwaysPending {
            fn provider(&self) -> &str {
                "test"
            }
            fn model_id(&self) -> &str {
                "pending"
            }
            fn max_videos_per_call(&self) -> Option<u32> {
                Some(1)
            }
            async fn do_start(
                &self,
                options: &VideoCallOptions,
            ) -> Result<VideoOperationStart, AiMuxError> {
                self.0.do_start(options).await
            }
            async fn do_status(
                &self,
                _operation: &serde_json::Value,
                _options: &VideoCallOptions,
            ) -> Result<VideoOperationStatus, AiMuxError> {
                Ok(VideoOperationStatus::Pending)
            }
        }
        let model = AlwaysPending(model);
        let mut options = VideoCallOptions::new("a cat");
        options.poll = Some(VideoPollOptions {
            interval_ms: Some(1),
            timeout_ms: Some(20),
        });
        let error = generate_video(&model, options).await.unwrap_err();
        assert!(
            matches!(error, AiMuxError::Timeout(_)),
            "expected Timeout, got {error:?}"
        );
    }

    #[tokio::test]
    async fn poll_timeout_reached_during_delay_skips_status_call() {
        let model = ScriptedVideoModel::new(vec![StatusStep::Complete]);
        let mut options = VideoCallOptions::new("a cat");
        options.poll = Some(VideoPollOptions {
            interval_ms: Some(100),
            timeout_ms: Some(10),
        });

        let error = generate_video(&model, options).await.unwrap_err();

        assert!(matches!(error, AiMuxError::Timeout(_)));
        assert_eq!(model.starts.load(Ordering::SeqCst), 1);
        assert_eq!(model.statuses.load(Ordering::SeqCst), 0);
        assert!(
            model
                .status_idempotency_keys
                .lock()
                .expect("status key capture lock should not be poisoned")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn n_zero_is_rejected_before_any_network_call() {
        let model = ScriptedVideoModel::new(vec![StatusStep::Complete]);
        let mut options = fast_poll_options();
        options.n = 0;

        let error = generate_video(&model, options).await.unwrap_err();

        assert!(
            matches!(error, AiMuxError::InvalidArgument(_)),
            "expected InvalidArgument, got {error:?}"
        );
        assert_eq!(
            model.starts.load(Ordering::SeqCst),
            0,
            "n=0 must not start a billed request"
        );
    }

    /// A model whose `do_start` is retriable per-batch (a small delay proves
    /// batches overlap rather than running one after another) and whose
    /// `do_status` completes immediately with one video + one provider
    /// metadata field unique to the batch index.
    struct BatchedVideoModel {
        starts: AtomicU32,
        start_idempotency_keys: Mutex<Vec<Option<String>>>,
        start_delay: Duration,
        max_per_call: Option<u32>,
    }

    #[async_trait]
    impl VideoModel for BatchedVideoModel {
        fn provider(&self) -> &str {
            "test"
        }
        fn model_id(&self) -> &str {
            "batched"
        }
        fn max_videos_per_call(&self) -> Option<u32> {
            self.max_per_call
        }
        async fn do_start(
            &self,
            options: &VideoCallOptions,
        ) -> Result<VideoOperationStart, AiMuxError> {
            let index = self.starts.fetch_add(1, Ordering::SeqCst);
            self.start_idempotency_keys
                .lock()
                .expect("start key capture lock should not be poisoned")
                .push(idempotency_key(options));
            tokio::time::sleep(self.start_delay).await;
            Ok(VideoOperationStart {
                operation: serde_json::json!({ "task_id": format!("t-{index}") }),
                warnings: vec![Warning::Other {
                    message: format!("batch {index} start"),
                }],
                provider_metadata: Some(SharedProviderMetadata::from([(
                    "test".to_string(),
                    serde_json::json!({ format!("batch{index}"): index }),
                )])),
                response: VideoResponse::default(),
            })
        }
        async fn do_status(
            &self,
            operation: &serde_json::Value,
            _options: &VideoCallOptions,
        ) -> Result<VideoOperationStatus, AiMuxError> {
            let task_id = operation["task_id"].as_str().unwrap().to_string();
            Ok(VideoOperationStatus::Completed(VideoResult {
                videos: vec![VideoData::Url {
                    url: format!("https://cdn/{task_id}.mp4"),
                    media_type: "video/mp4".to_string(),
                }],
                warnings: Vec::new(),
                provider_metadata: None,
                response: VideoResponse::default(),
            }))
        }
    }

    #[tokio::test]
    async fn n_above_max_per_call_splits_into_batches_and_aggregates() {
        let model = BatchedVideoModel {
            starts: AtomicU32::new(0),
            start_idempotency_keys: Mutex::new(Vec::new()),
            start_delay: Duration::from_millis(1),
            max_per_call: Some(1),
        };
        let mut options = fast_poll_options();
        options.n = 3;

        let result = generate_video(&model, options).await.unwrap();

        assert_eq!(
            model.starts.load(Ordering::SeqCst),
            3,
            "n=3 with max_videos_per_call=1 must run three do_start batches"
        );
        assert_eq!(result.videos.len(), 3);
        assert_eq!(result.warnings.len(), 3, "warnings from every batch kept");

        let keys = model
            .start_idempotency_keys
            .lock()
            .expect("start key capture lock should not be poisoned");
        assert_eq!(keys.len(), 3);
        assert!(keys.iter().all(Option::is_some));
        let unique: std::collections::HashSet<_> = keys.iter().collect();
        assert_eq!(
            unique.len(),
            3,
            "each batch must mint its own idempotency key: {keys:?}"
        );

        // Provider metadata across batches is deep-merged (§ merge_provider_metadata),
        // not `entry().or_insert()`-dropped: all three batch-specific fields survive
        // under the shared "test" provider key.
        let metadata = result
            .provider_metadata
            .expect("provider metadata aggregated");
        let test_meta = metadata.get("test").expect("test provider metadata");
        for i in 0..3 {
            assert!(
                test_meta.get(format!("batch{i}")).is_some(),
                "batch {i}'s metadata field missing from aggregate: {test_meta:?}"
            );
        }
    }

    #[tokio::test]
    async fn batches_run_concurrently_not_sequentially() {
        let model = BatchedVideoModel {
            starts: AtomicU32::new(0),
            start_idempotency_keys: Mutex::new(Vec::new()),
            start_delay: Duration::from_millis(80),
            max_per_call: Some(1),
        };
        let mut options = fast_poll_options();
        options.n = 3;

        let started = Instant::now();
        generate_video(&model, options).await.unwrap();
        let elapsed = started.elapsed();

        // Sequential batches would take >= 3 * 80ms = 240ms; concurrent
        // batches take roughly one delay (~80ms) plus scheduling overhead.
        assert!(
            elapsed < Duration::from_millis(200),
            "batches should overlap (AI SDK generateVideo/generateImage runs \
             them concurrently via Promise.all), took {elapsed:?}"
        );
    }
}
