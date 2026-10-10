//! Hume speech (TTS) provider.
//!
//! Aligned with Vercel AI SDK `createHume`
//! (`reference/ai/packages/hume/src/hume-provider.ts`) and `HumeSpeechModel`
//! (`reference/ai/packages/hume/src/hume-speech-model.ts`).
//!
//! Endpoint: `POST https://api.hume.ai/v0/tts/file`
//!
//! Authentication: `X-Hume-Api-Key` header.
//!
//! The Hume TTS API accepts `utterances` (an array of `{ text, voice, speed,
//! description }` objects) and `format` (with a `type` field) in the request
//! body, and returns raw binary audio.
//!
//! `language` is not supported and emits a warning. The model ID is always an
//! empty string (Hume does not use model IDs for TTS).
//!
//! [`create_hume`] takes [`HumeProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`HumeProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `HUME_API_KEY`.
//! [`hume()`] is the default instance; it reads nothing and cannot fail.

use std::sync::OnceLock;

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::shared::{SharedProviderOptions, Warning};
use aimux_core::speech_model::{
    AudioData, SpeechCallOptions, SpeechModel, SpeechRequest, SpeechResponse, SpeechResult,
};
use aimux_provider_utils::{FetchFunction, HeaderMapOpt, HeadersFn, Resolvable, validate_base_url};

use crate::shared::{AuthScheme, Credential, EndpointConfig, credential_headers};

/// Hume errors carry a top-level `message` with a `code` such as `"E0101"`
/// (https://dev.hume.ai/docs/resources/errors); the nested `{error:
/// {message, code}}` shape matches the AI SDK's Hume error schema.
fn hume_error_parts(data: &Value) -> aimux_provider_utils::ProviderErrorParts {
    let error = data.get("error");
    let message = data
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| {
            error
                .and_then(|value| value.get("message"))
                .and_then(Value::as_str)
        })
        .or_else(|| error.and_then(Value::as_str))
        .unwrap_or("Hume request failed")
        .to_string();
    aimux_provider_utils::ProviderErrorParts {
        message,
        provider_code: data
            .get("code")
            .or_else(|| error.and_then(|value| value.get("code")))
            .and_then(|value| match value {
                Value::String(s) => Some(s.clone()),
                Value::Number(n) => Some(n.to_string()),
                _ => None,
            }),
    }
}

fn hume_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    aimux_provider_utils::create_json_error_response_handler(hume_error_parts)
}

// ── Config ───────────────────────────────────────────────────────────────────

pub(crate) mod options;

const DEFAULT_BASE_URL: &str = "https://api.hume.ai";
const API_KEY_ENV_VAR: &str = "HUME_API_KEY";
const DEFAULT_NAME: &str = "hume";

/// Settings of [`create_hume`].
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except `base_url` and `name`; `api_key` and `headers` are
/// evaluated on every request.
#[derive(Clone, Default)]
pub struct HumeProviderSettings {
    /// Base URL for the API calls. Default `https://api.hume.ai`; a
    /// trailing slash is removed.
    pub base_url: Option<String>,
    /// The API key. `None` loads `HUME_API_KEY` when a request is made and
    /// fails that request with `AiMuxError::LoadApiKey` if it is unset. An
    /// explicit value is used as given, `""` included: it never falls back to
    /// the environment. A [`Resolvable::Future`] is awaited once, an
    /// [`Resolvable::AsyncFn`] on every request.
    pub api_key: Option<Resolvable<String>>,
    /// Extra headers on every request. A `None` value removes the header,
    /// including the credential. Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The provider name, the prefix of the `provider()` strings (`"{name}.speech"`).
    /// Default `"hume"`. The providerOptions key stays `hume`.
    pub name: Option<String>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
}

impl std::fmt::Debug for HumeProviderSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HumeProviderSettings")
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

/// Create a Hume provider.
///
/// # Errors
///
/// Returns `AiMuxError::InvalidArgument` when `base_url` is not an `http(s)`
/// URL with a host. That is the only way this fails: the key is loaded per
/// request, not here.
pub fn create_hume(settings: HumeProviderSettings) -> Result<HumeProvider, AiMuxError> {
    let base_url = match settings.base_url.as_deref() {
        Some(url) => validate_base_url(url)?,
        None => DEFAULT_BASE_URL.to_string(),
    };
    Ok(HumeProvider {
        name: settings.name.unwrap_or_else(|| DEFAULT_NAME.to_string()),
        base_url,
        headers: credential_headers(
            Credential::explicit_or_env(settings.api_key, API_KEY_ENV_VAR, "Hume"),
            AuthScheme::Header("X-Hume-Api-Key"),
            Vec::new(),
            settings.headers,
        ),
        fetch: settings.fetch,
    })
}

/// The default provider: `create_hume` with default settings, created on
/// first use. Creating it reads nothing from the environment and cannot fail;
/// a missing key surfaces from the first request instead.
pub fn hume() -> &'static HumeProvider {
    static DEFAULT: OnceLock<HumeProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_hume(HumeProviderSettings::default())
            .expect("default Hume settings are always valid")
    })
}

/// A Hume provider. Cheap to clone the models out of; it holds no HTTP
/// client.
pub struct HumeProvider {
    name: String,
    base_url: String,
    headers: HeadersFn,
    fetch: Option<FetchFunction>,
}

impl HumeProvider {
    fn model_config(&self, method: &str) -> EndpointConfig {
        EndpointConfig::fixed(
            format!("{}.{method}", self.name),
            self.base_url.clone(),
            self.headers.clone(),
            self.fetch.clone(),
        )
    }

    /// The speech (TTS) model. Hume does not use model ids for TTS, so the model id is always an empty string; `provider()` is `"{name}.speech"`.
    #[must_use]
    pub fn speech(&self) -> HumeSpeechModel {
        HumeSpeechModel::from_config(self.model_config("speech"))
    }
}

crate::impl_single_modality_provider!(HumeProvider, speech_model, |p, _id| p.speech());

// ── Speech model ─────────────────────────────────────────────────────────────

/// The default voice ID used when no voice is provided.
const DEFAULT_VOICE_ID: &str = "d8ab67c6-953d-4bd8-9370-8fa53a0f1453";

/// The output formats accepted by the Hume TTS API.
const SUPPORTED_OUTPUT_FORMATS: &[&str] = &["mp3", "pcm", "wav"];

/// A Hume speech (TTS) model.
pub struct HumeSpeechModel {
    /// Hume does not use model IDs for TTS; this is always an empty string.
    model_id: String,
    config: EndpointConfig,
}

impl HumeSpeechModel {
    pub(crate) fn from_config(config: EndpointConfig) -> Self {
        Self {
            model_id: String::new(),
            config,
        }
    }
}

#[async_trait]
impl SpeechModel for HumeSpeechModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_generate(&self, options: &SpeechCallOptions) -> Result<SpeechResult, AiMuxError> {
        let (body, warnings) = build_request(options)?;

        let exchange = self.config.exchange(options.headers.as_ref()).await?;

        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(exchange.url("/v0/tts/file"), options),
            Value::Object(body.clone()),
            aimux_provider_utils::create_binary_response_handler(),
            hume_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;

        let audio_bytes = resp.value.to_vec();

        let timestamp = chrono::Utc::now().to_rfc3339();

        Ok(SpeechResult {
            audio: AudioData::Binary(audio_bytes),
            warnings,
            request: Some(SpeechRequest {
                body: Some(Value::Object(body)),
            }),
            response: SpeechResponse {
                timestamp: Some(timestamp),
                model_id: Some(self.model_id.clone()),
                headers: Some(response_headers),
                body: None,
            },
            provider_metadata: None,
        })
    }
}

// ── Request builder ──────────────────────────────────────────────────────────

/// Build the Hume TTS request body and collect warnings.
///
/// Mirrors the TS `HumeSpeechModel.getArgs`:
/// - `voice` defaults to `"d8ab67c6-953d-4bd8-9370-8fa53a0f1453"`.
/// - `outputFormat` defaults to `"mp3"`; unsupported formats emit a warning.
/// - `speed` and `instructions` (as `description`) are placed inside the
///   utterance.
/// - `language` is not supported and emits a warning.
/// - Provider options (`hume` key) support a `context` field with either a
///   `generationId` or a list of `utterances`.
fn build_request(
    options: &SpeechCallOptions,
) -> Result<(Map<String, Value>, Vec<Warning>), AiMuxError> {
    let mut warnings = Vec::new();

    let voice = options.voice.as_deref().unwrap_or(DEFAULT_VOICE_ID);
    let output_format = options.output_format.as_deref().unwrap_or("mp3");

    // Build the utterance.
    let mut utterance = Map::new();
    utterance.insert("text".to_string(), json!(options.text));
    if let Some(speed) = options.speed {
        utterance.insert("speed".to_string(), json!(speed));
    }
    if let Some(ref instructions) = options.instructions {
        utterance.insert("description".to_string(), json!(instructions));
    }
    utterance.insert(
        "voice".to_string(),
        json!({ "id": voice, "provider": "HUME_AI" }),
    );

    let mut body = Map::new();
    body.insert("utterances".to_string(), json!([Value::Object(utterance)]));

    // Format.
    let mut format = Map::new();
    format.insert("type".to_string(), json!("mp3"));
    if SUPPORTED_OUTPUT_FORMATS.contains(&output_format) {
        format.insert("type".to_string(), json!(output_format));
    } else {
        warnings.push(Warning::Unsupported {
            feature: "outputFormat".to_string(),
            details: Some(format!(
                "Unsupported output format: {output_format}. Using mp3 instead."
            )),
        });
    }
    body.insert("format".to_string(), Value::Object(format));

    // Add provider-specific options (context).
    let hume_options = parse_hume_provider_options(options.provider_options.as_ref());
    if let Some(ref opts) = hume_options
        && let Some(ref context) = opts.context
    {
        body.insert("context".to_string(), json!(context));
    }

    if let Some(ref language) = options.language {
        warnings.push(Warning::Unsupported {
            feature: "language".to_string(),
            details: Some(format!(
                "Hume speech models do not support language selection. Language parameter \"{language}\" was ignored."
            )),
        });
    }

    Ok((body, warnings))
}

// ── Provider options parsing ─────────────────────────────────────────────────

/// Parsed `hume` speech provider options.
#[derive(Debug, Default)]
struct HumeSpeechProviderOptions {
    /// Context for the speech synthesis request — either a `generation_id` or
    /// a list of `utterances`. Stored as a pre-built JSON value ready for the
    /// request body.
    context: Option<Value>,
}

/// Extract Hume-specific speech options from the shared provider options.
fn parse_hume_provider_options(
    options: Option<&SharedProviderOptions>,
) -> Option<HumeSpeechProviderOptions> {
    let opts = options::hume_options(options)?;
    let context = opts.get("context").and_then(|c| c.as_object())?;

    // The context can be either { generationId: "..." } or { utterances: [...] }.
    // We map it to the API shape: { generation_id: "..." } or { utterances: [...] }.
    let resolved_context =
        if let Some(gen_id) = context.get("generationId").and_then(|v| v.as_str()) {
            json!({ "generation_id": gen_id })
        } else {
            let utterances = context.get("utterances").and_then(|v| v.as_array())?;
            let mapped: Vec<Value> = utterances
                .iter()
                .filter_map(|u| u.as_object())
                .map(|u| {
                    let mut m = Map::new();
                    if let Some(t) = u.get("text").and_then(|v| v.as_str()) {
                        m.insert("text".to_string(), json!(t));
                    }
                    if let Some(d) = u.get("description").and_then(|v| v.as_str()) {
                        m.insert("description".to_string(), json!(d));
                    }
                    if let Some(s) = u.get("speed").and_then(serde_json::Value::as_f64) {
                        m.insert("speed".to_string(), json!(s));
                    }
                    if let Some(ts) = u.get("trailingSilence").and_then(serde_json::Value::as_f64) {
                        m.insert("trailing_silence".to_string(), json!(ts));
                    }
                    if let Some(v) = u.get("voice") {
                        m.insert("voice".to_string(), v.clone());
                    }
                    Value::Object(m)
                })
                .collect();
            json!({ "utterances": mapped })
        };

    Some(HumeSpeechProviderOptions {
        context: Some(resolved_context),
    })
}
