//! AssemblyAI transcription (STT) provider.
//!
//! Aligned with Vercel AI SDK `createAssemblyAI` / `AssemblyAITranscriptionModel`
//! (`reference/ai/packages/assemblyai/src/assemblyai-transcription-model.ts`).
//!
//! AssemblyAI uses a three-step async pattern:
//! 1. POST `/v2/upload` with raw audio bytes → returns `upload_url`
//! 2. POST `/v2/transcript` with JSON body (audio_url + options) → returns transcript `id`
//! 3. GET `/v2/transcript/{id}` polling until status is `completed`
//!
//! [`create_assemblyai`] takes [`AssemblyAIProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`AssemblyAIProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `ASSEMBLYAI_API_KEY`.
//! [`assemblyai()`] is the default instance; it reads nothing and cannot fail.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::error::ApiCallError;
use aimux_core::shared::{SharedProviderMetadata, Warning};
use aimux_core::transcription_model::{
    AudioInput, TranscriptionCallOptions, TranscriptionModel, TranscriptionRequest,
    TranscriptionResponse, TranscriptionResult, TranscriptionSegment,
};
use aimux_provider_utils::HttpBody;
use aimux_provider_utils::{FetchFunction, HeaderMapOpt, Resolvable, validate_base_url};

use crate::shared::{
    AuthScheme, Credential, EndpointConfig, POLL_INTERVAL_MS_KEY, PollStep, ProviderHeaders,
    is_poll_control_key, poll_interval_ms, poll_until,
};

/// AssemblyAI errors: `{"error": "..."}` where `error` is a plain string
/// (e.g. `"Authentication error, API token missing/invalid."`). No machine
/// code is documented.
fn assemblyai_error_parts(data: &Value) -> aimux_provider_utils::ProviderErrorParts {
    let error = data.get("error");
    let message = error
        .and_then(Value::as_str)
        .or_else(|| {
            error
                .and_then(|value| value.get("message"))
                .and_then(Value::as_str)
        })
        .or_else(|| data.get("message").and_then(Value::as_str))
        .unwrap_or("AssemblyAI request failed")
        .to_string();
    aimux_provider_utils::ProviderErrorParts {
        message,
        provider_code: error
            .and_then(|value| value.get("code"))
            .and_then(|value| match value {
                Value::String(s) => Some(s.clone()),
                Value::Number(n) => Some(n.to_string()),
                _ => None,
            }),
    }
}

fn assemblyai_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    aimux_provider_utils::create_json_error_response_handler(assemblyai_error_parts)
}

// ── Config ──────────────────────────────────────────────────────────────────

/// Milliseconds between two polls of a transcript
/// (`providerOptions.assemblyai.pollIntervalMs` overrides it for one call).
const POLL_INTERVAL_MS: u64 = 100;
/// How many times a transcript is polled before the call gives up (ten minutes
/// at the default interval).
const MAX_POLL_ATTEMPTS: u32 = 6_000;

pub(crate) mod options;

const DEFAULT_BASE_URL: &str = "https://api.assemblyai.com";
const API_KEY_ENV_VAR: &str = "ASSEMBLYAI_API_KEY";
const DEFAULT_NAME: &str = "assemblyai";

/// Settings of [`create_assemblyai`].
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except `base_url` and `name`; `api_key` and `headers` are
/// evaluated on every request.
#[derive(Clone, Default)]
pub struct AssemblyAIProviderSettings {
    /// Base URL for the API calls. Default `https://api.assemblyai.com`; a
    /// trailing slash is removed.
    pub base_url: Option<String>,
    /// The API key. `None` loads `ASSEMBLYAI_API_KEY` when a request is made and
    /// fails that request with `AiMuxError::LoadApiKey` if it is unset. An
    /// explicit value is used as given, `""` included: it never falls back to
    /// the environment. A [`Resolvable::Future`] is awaited once, an
    /// [`Resolvable::AsyncFn`] on every request.
    pub api_key: Option<Resolvable<String>>,
    /// Extra headers on every request. A `None` value removes the header,
    /// including the credential. Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The provider name, the prefix of the `provider()` strings (`"{name}.transcription"`).
    /// Default `"assemblyai"`. The providerOptions key stays `assemblyai`.
    pub name: Option<String>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
}

impl std::fmt::Debug for AssemblyAIProviderSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AssemblyAIProviderSettings")
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key)
            .field(
                "headers",
                &self.headers.as_ref().map(std::collections::HashMap::len),
            )
            .field("name", &self.name)
            .field("fetch", &self.fetch.is_some())
            .finish()
    }
}

/// Create a AssemblyAI provider.
///
/// # Errors
///
/// Returns `AiMuxError::InvalidArgument` when `base_url` is not an `http(s)`
/// URL with a host. That is the only way this fails: the key is loaded per
/// request, not here.
pub fn create_assemblyai(
    settings: AssemblyAIProviderSettings,
) -> Result<AssemblyAIProvider, AiMuxError> {
    let base_url = match settings.base_url.as_deref() {
        Some(url) => validate_base_url(url)?,
        None => DEFAULT_BASE_URL.to_string(),
    };
    Ok(AssemblyAIProvider {
        name: settings.name.unwrap_or_else(|| DEFAULT_NAME.to_string()),
        base_url,
        headers: ProviderHeaders::new(
            Credential::explicit_or_env(settings.api_key, API_KEY_ENV_VAR, "AssemblyAI"),
            AuthScheme::Header("Authorization"),
            Vec::new(),
            settings.headers,
        ),
        fetch: settings.fetch,
    })
}

/// The default provider: `create_assemblyai` with default settings, created on
/// first use. Creating it reads nothing from the environment and cannot fail;
/// a missing key surfaces from the first request instead.
pub fn assemblyai() -> &'static AssemblyAIProvider {
    static DEFAULT: OnceLock<AssemblyAIProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_assemblyai(AssemblyAIProviderSettings::default())
            .expect("default AssemblyAI settings are always valid")
    })
}

/// A AssemblyAI provider. Cheap to clone the models out of; it holds no HTTP
/// client.
pub struct AssemblyAIProvider {
    name: String,
    base_url: String,
    headers: ProviderHeaders,
    fetch: Option<FetchFunction>,
}

impl AssemblyAIProvider {
    fn model_config(&self, method: &str) -> EndpointConfig {
        EndpointConfig::fixed(
            format!("{}.{method}", self.name),
            self.base_url.clone(),
            self.headers.clone(),
            self.fetch.clone(),
        )
    }

    /// A transcription (STT) model (e.g. `"universal-3-5-pro"`); `provider()` is `"{name}.transcription"`.
    #[must_use]
    pub fn transcription(&self, model_id: &str) -> AssemblyAITranscriptionModel {
        AssemblyAITranscriptionModel::from_config(
            model_id.to_string(),
            self.model_config("transcription"),
        )
    }
}

crate::impl_single_modality_provider!(AssemblyAIProvider, transcription_model, |p, id| p
    .transcription(id));

// ── Response schema ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct AssemblyAIUploadResponse {
    upload_url: String,
}

#[derive(Debug, Deserialize)]
struct AssemblyAISubmitResponse {
    id: String,
}

#[derive(Debug, Deserialize)]
struct AssemblyAIWord {
    text: String,
    start: f64,
    end: f64,
}

#[derive(Debug, Deserialize)]
struct AssemblyAITranscriptResponse {
    #[allow(dead_code)]
    id: String,
    status: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default, rename = "language_code")]
    language_code: Option<String>,
    #[serde(default)]
    words: Option<Vec<AssemblyAIWord>>,
    #[serde(default)]
    utterances: Option<Vec<Value>>,
    #[serde(default, rename = "sentiment_analysis_results")]
    sentiment_analysis_results: Option<Vec<Value>>,
    #[serde(default, rename = "entities")]
    entities: Option<Vec<Value>>,
    #[serde(default, rename = "content_safety_labels")]
    content_safety_labels: Option<Value>,
    #[serde(default, rename = "iab_categories_result")]
    iab_categories_result: Option<Value>,
    #[serde(default, rename = "auto_highlights_result")]
    auto_highlights_result: Option<Value>,
    #[serde(default, rename = "audio_duration")]
    audio_duration: Option<f64>,
    #[serde(default)]
    error: Option<String>,
}

// ── Model ───────────────────────────────────────────────────────────────────

fn audio_input_to_bytes(audio: &AudioInput) -> Result<Vec<u8>, AiMuxError> {
    match audio {
        AudioInput::Binary(bytes) => Ok(bytes.clone()),
        AudioInput::Base64(b64) => {
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64)
                .map_err(|e| AiMuxError::InvalidArgument(format!("invalid base64: {e}")))
        }
    }
}

pub struct AssemblyAITranscriptionModel {
    model_id: String,
    config: EndpointConfig,
}

impl AssemblyAITranscriptionModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }
}

#[async_trait]
impl TranscriptionModel for AssemblyAITranscriptionModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_generate(
        &self,
        options: &TranscriptionCallOptions,
    ) -> Result<TranscriptionResult, AiMuxError> {
        let mut warnings: Vec<Warning> = Vec::new();

        let audio_bytes = audio_input_to_bytes(&options.audio)?;

        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let aai_options = options::assemblyai_options(options.provider_options.as_ref());

        // Step 1: Upload audio. One attempt: an upload is never replayed
        // implicitly.
        let resp = aimux_provider_utils::post_to_api(
            exchange.request(exchange.url("/v2/upload"), options),
            HttpBody::Bytes(audio_bytes, "application/octet-stream".to_string()),
            aimux_provider_utils::create_json_response_handler(),
            assemblyai_failed_response_handler(),
        )
        .await?;

        let upload: AssemblyAIUploadResponse = resp.value;

        // Step 2: Submit transcript request.
        let mut body = Map::new();

        // Model selection.
        if self.model_id == "best" {
            body.insert("speech_model".to_string(), json!(self.model_id));
            warnings.push(Warning::Unsupported {
                feature: "model 'best'".to_string(),
                details: Some(
                    "The 'best' model is a legacy AssemblyAI model. Use 'universal-3-5-pro' instead."
                        .to_string(),
                ),
            });
        } else {
            body.insert("speech_models".to_string(), json!([self.model_id]));
        }

        body.insert("audio_url".to_string(), json!(upload.upload_url));

        // Parse and forward provider options, except the poll interval, which
        // is read here.
        if let Some(obj) = aai_options {
            for (k, v) in obj {
                if !is_poll_control_key(k) {
                    body.insert(k.clone(), v.clone());
                }
            }
        }

        // This creates the transcript and is sent exactly once; only the status
        // poll below repeats.
        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(exchange.url("/v2/transcript"), options),
            Value::Object(body),
            aimux_provider_utils::create_json_response_handler(),
            assemblyai_failed_response_handler(),
        )
        .await?;

        let submit: AssemblyAISubmitResponse = resp.value;

        // Step 3: Poll for completion.
        let poll_url = exchange.url(&format!("/v2/transcript/{}", submit.id));
        let resp = poll_until(
            &format!("assemblyai transcript {}", submit.id),
            options.abort_signal.as_ref(),
            Duration::from_millis(poll_interval_ms(
                aai_options,
                POLL_INTERVAL_MS_KEY,
                POLL_INTERVAL_MS,
            )),
            MAX_POLL_ATTEMPTS,
            || async {
                let resp = aimux_provider_utils::get_from_api(
                    exchange.request(poll_url.clone(), options),
                    aimux_provider_utils::create_json_response_handler::<
                        AssemblyAITranscriptResponse,
                    >(),
                    assemblyai_failed_response_handler(),
                )
                .await?;
                match resp.value.status.clone().as_str() {
                    "completed" => Ok(PollStep::Ready(resp)),
                    "error" => Err(AiMuxError::ApiCall(Box::new(ApiCallError {
                        status_code: Some(200),
                        provider_code: Some("error".to_string()),
                        response_body: Some(
                            resp.raw_value.clone().unwrap_or(Value::Null).to_string(),
                        ),
                        ..ApiCallError::new(
                            format!(
                                "Transcription failed: {}",
                                resp.value
                                    .error
                                    .clone()
                                    .unwrap_or_else(|| "Unknown error".to_string())
                            ),
                            poll_url.clone(),
                            serde_json::json!({}),
                        )
                    }))),
                    _ => Ok(PollStep::Pending),
                }
            },
        )
        .await?;

        let response_headers = resp.response_headers;
        let raw_body = resp.raw_value.unwrap_or(Value::Null);
        let parsed = resp.value;

        // Build segments from words (timestamps are in milliseconds).
        let segments: Vec<TranscriptionSegment> = parsed
            .words
            .as_ref()
            .map(|words| {
                words
                    .iter()
                    .map(|w| TranscriptionSegment {
                        text: w.text.clone(),
                        start_second: w.start / 1000.0,
                        end_second: w.end / 1000.0,
                    })
                    .collect()
            })
            .unwrap_or_default();

        let language = parsed.language_code.clone();
        let duration_in_seconds = parsed.audio_duration.or_else(|| {
            parsed
                .words
                .as_ref()
                .and_then(|w| w.last())
                .map(|w| w.end / 1000.0)
        });

        let text = parsed.text.unwrap_or_default();

        // Build provider metadata from extra fields.
        let mut provider_metadata: Option<SharedProviderMetadata> = None;
        let mut md = HashMap::new();
        let mut aai_meta = Map::new();
        if parsed.utterances.is_some() {
            aai_meta.insert(
                "utterances".to_string(),
                raw_body.get("utterances").cloned().unwrap_or(Value::Null),
            );
        }
        if parsed.sentiment_analysis_results.is_some() {
            aai_meta.insert(
                "sentimentAnalysisResults".to_string(),
                raw_body
                    .get("sentiment_analysis_results")
                    .cloned()
                    .unwrap_or(Value::Null),
            );
        }
        if parsed.entities.is_some() {
            aai_meta.insert(
                "entities".to_string(),
                raw_body.get("entities").cloned().unwrap_or(Value::Null),
            );
        }
        if parsed.content_safety_labels.is_some() {
            aai_meta.insert(
                "contentSafetyLabels".to_string(),
                raw_body
                    .get("content_safety_labels")
                    .cloned()
                    .unwrap_or(Value::Null),
            );
        }
        if parsed.iab_categories_result.is_some() {
            aai_meta.insert(
                "iabCategoriesResult".to_string(),
                raw_body
                    .get("iab_categories_result")
                    .cloned()
                    .unwrap_or(Value::Null),
            );
        }
        if parsed.auto_highlights_result.is_some() {
            aai_meta.insert(
                "autoHighlightsResult".to_string(),
                raw_body
                    .get("auto_highlights_result")
                    .cloned()
                    .unwrap_or(Value::Null),
            );
        }
        if !aai_meta.is_empty() {
            md.insert(options::NAMESPACE.to_string(), aai_meta);
            provider_metadata = Some(md);
        }

        let timestamp = chrono::Utc::now().to_rfc3339();

        Ok(TranscriptionResult {
            text,
            segments,
            language,
            duration_in_seconds,
            warnings,
            request: Some(TranscriptionRequest { body: None }),
            response: TranscriptionResponse {
                timestamp: Some(timestamp),
                model_id: Some(self.model_id.clone()),
                headers: Some(response_headers),
                body: Some(raw_body),
            },
            provider_metadata,
        })
    }
}
