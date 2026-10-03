//! Codex provider (RFC-0018) — OpenAI Codex models over the Responses API.
//!
//! [`create_codex`] takes [`CodexProviderSettings`], validates the base URL,
//! fixes the provider name and returns a [`CodexProvider`]. Credentials are not
//! read there; they are resolved in the request headers of every call, so a
//! host that refreshes the ChatGPT access token supplies an
//! [`Resolvable::AsyncFn`] or rebuilds the provider. The AI SDK has no Codex
//! package; the factory follows the shape of the OpenAI one.
//!
//! Codex models (`gpt-5.2-codex`, `gpt-5.1-codex`, `gpt-5.1-codex-mini`,
//! `gpt-5-codex`, …) are available **only** through the Responses API
//! (verified 2026-08-03, RFC-0018 §2 V1).
//!
//! Two access modes ([`CodexMode`], chosen in the settings):
//!
//! - **`ApiKey`** (default): `https://api.openai.com/v1/responses` — the
//!   official, documented, usage-based channel. Recommended for CI/automation.
//! - **`ChatGptAccount`**: `https://chatgpt.com/backend-api/codex/responses` —
//!   the ChatGPT-subscription channel (single personal account, per OpenAI
//!   ToU). Undocumented endpoint — **best-effort, no SLA**. Mirrors the
//!   official Codex client: **always streams** (`stream: true`, `store: false`).
//!
//! OAuth is *not* the library's job (RFC-0018 §3.2 responsibility split): the
//! integrator performs the device-code login, persists tokens, and orchestrates
//! refresh. The library exposes [`codex_refresh`] as a stateless protocol
//! helper and maps subscription-channel 401 responses to
//! [`AiMuxError::TokenExpired`] so the integrator can refresh and rebuild the
//! provider.
//!
//! Scope boundaries (RFC-0018): no account pools, no quota rotation, no
//! reselling — single account, single user.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use futures::StreamExt;
use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use aimux_core::embedding_model::EmbeddingModel;
use aimux_core::error::AiMuxError;
use aimux_core::image_model::ImageModel;
use aimux_core::language_model::LanguageModel;
use aimux_core::model_catalogue::RuntimeModel;
use aimux_core::options::CallOptions;
use aimux_core::provider::{Provider, ProviderDiscovery};
use aimux_core::result::{GenerateResult, StreamResult};
use aimux_core::types::Warning;

use aimux_provider_utils::{
    FetchFunction, HeaderMapOpt, HeadersFn, HttpRequest, Resolvable, validate_base_url,
};

use crate::openai::config::OpenAIModelConfig;
use crate::openai::responses::responses_convert::{
    build_responses_event_stream, build_responses_generate_result,
};
use crate::openai::responses::{
    OpenAIResponsesModel, ResponsesNamespace, build_responses_request_body,
};
pub use crate::shared::TransformRequestBody;
use crate::shared::{AuthScheme, Credential, credential_headers};

/// Default API-key base URL (official OpenAI Responses endpoint).
pub const CODEX_API_BASE_URL: &str = "https://api.openai.com/v1";
/// Default subscription base URL (undocumented ChatGPT backend endpoint). The
/// Responses path is appended, giving `https://chatgpt.com/backend-api/codex/responses`
/// (the endpoint the `chatgpt` cassettes record).
pub const CODEX_SUBSCRIPTION_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";
/// Environment variable the API-key mode reads when no key is given.
pub const CODEX_API_KEY_ENV_VAR: &str = "CODEX_API_KEY";
/// OAuth token endpoint used by [`codex_refresh`] (RFC-0018 §2 V3).
pub const CODEX_OAUTH_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";

const DEFAULT_NAME: &str = "codex";
const DEFAULT_ORIGINATOR: &str = "aimux";

/// Access mode of the Codex provider, with the credential of that channel.
#[derive(Debug, Clone)]
pub enum CodexMode {
    /// Official API-key channel (`api.openai.com`), usage-based. `None` loads
    /// `CODEX_API_KEY` when a request is made and fails that request with
    /// `AiMuxError::LoadApiKey` if it is unset; an explicit key is used as
    /// given, `""` included.
    ApiKey(Option<Resolvable<String>>),
    /// ChatGPT-subscription channel (`chatgpt.com/backend-api`), single
    /// personal account; always streams.
    ///
    /// `token` is the access token minted by the integrator's OAuth
    /// device-code flow: the library never performs login, persists tokens, or
    /// refreshes automatically (RFC-0018 §3.2). A [`Resolvable::AsyncFn`] is
    /// evaluated on every request.
    ChatGptAccount {
        /// The account access token, sent as `Authorization: Bearer`.
        token: Resolvable<String>,
        /// The `ChatGPT-Account-Id` header value (optional).
        account_id: Option<String>,
    },
}

impl Default for CodexMode {
    /// The API-key channel, key from `CODEX_API_KEY`.
    fn default() -> Self {
        Self::ApiKey(None)
    }
}

/// Settings of [`create_codex`].
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except `base_url` and `name`; the credential and `headers` are
/// evaluated on every request.
#[derive(Clone, Default)]
pub struct CodexProviderSettings {
    /// The access mode and its credential. Default: API key from
    /// `CODEX_API_KEY`.
    pub mode: CodexMode,
    /// Base URL for the API calls. Default [`CODEX_API_BASE_URL`] for
    /// `ApiKey` and [`CODEX_SUBSCRIPTION_BASE_URL`] for `ChatGptAccount`; a
    /// trailing slash is removed.
    pub base_url: Option<String>,
    /// Extra headers on every request. A `None` value removes the header,
    /// including `Authorization`. They win over the channel headers
    /// (`Originator`, `ChatGPT-Account-Id`); per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The provider name, the prefix of `provider()` (`"{name}.responses"`).
    /// Default `"codex"`.
    pub name: Option<String>,
    /// `ChatGptAccount` mode: the `Originator` header value (client
    /// identifier; reference clients use e.g. `"opencode"` / `"codex_cli_rs"`).
    /// Default `"aimux"`.
    pub originator: Option<String>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
    /// Rewrites every JSON request body once, after it is serialized (and
    /// after the `store: false` rule of the subscription channel) and before it
    /// is sent.
    pub transform_request_body: Option<TransformRequestBody>,
}

impl std::fmt::Debug for CodexProviderSettings {
    /// Never prints the credential or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodexProviderSettings")
            .field("mode", &self.mode)
            .field("base_url", &self.base_url)
            .field(
                "headers",
                &self.headers.as_ref().map(std::collections::HashMap::len),
            )
            .field("name", &self.name)
            .field("originator", &self.originator)
            .field("fetch", &self.fetch.is_some())
            .field(
                "transform_request_body",
                &self.transform_request_body.is_some(),
            )
            .finish()
    }
}

/// Which channel a provider talks to; the model rules differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Channel {
    ApiKey,
    ChatGptAccount,
}

/// Create a Codex provider.
///
/// # Errors
///
/// Returns `AiMuxError::InvalidArgument` when `base_url` is not an `http(s)`
/// URL with a host. That is the only way this fails: credentials are resolved
/// per request, not here.
pub fn create_codex(settings: CodexProviderSettings) -> Result<CodexProvider, AiMuxError> {
    let (channel, default_base_url, credential, fixed) = match settings.mode {
        CodexMode::ApiKey(key) => (
            Channel::ApiKey,
            CODEX_API_BASE_URL,
            Credential::explicit_or_env(key, CODEX_API_KEY_ENV_VAR, "Codex"),
            Vec::new(),
        ),
        CodexMode::ChatGptAccount { token, account_id } => {
            let mut fixed = vec![(
                "Originator".to_string(),
                settings
                    .originator
                    .unwrap_or_else(|| DEFAULT_ORIGINATOR.to_string()),
            )];
            if let Some(id) = account_id {
                fixed.push(("ChatGPT-Account-Id".to_string(), id));
            }
            (
                Channel::ChatGptAccount,
                CODEX_SUBSCRIPTION_BASE_URL,
                Credential::Explicit(token),
                fixed,
            )
        }
    };
    let base_url = match settings.base_url.as_deref() {
        Some(url) => validate_base_url(url)?,
        None => default_base_url.to_string(),
    };
    Ok(CodexProvider {
        name: settings.name.unwrap_or_else(|| DEFAULT_NAME.to_string()),
        channel,
        base_url,
        headers: credential_headers(credential, AuthScheme::Bearer, fixed, settings.headers),
        fetch: settings.fetch,
        transform_request_body: settings.transform_request_body,
    })
}

/// The default provider: `create_codex` with default settings (API key from
/// `CODEX_API_KEY`), created on first use. Creating it reads nothing from the
/// environment and cannot fail; a missing key surfaces from the first request
/// instead.
pub fn codex() -> &'static CodexProvider {
    static DEFAULT: OnceLock<CodexProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_codex(CodexProviderSettings::default())
            .expect("default Codex settings are always valid")
    })
}

/// A Codex provider. Cheap to clone the models out of; it holds no HTTP
/// client.
pub struct CodexProvider {
    name: String,
    channel: Channel,
    base_url: String,
    headers: HeadersFn,
    fetch: Option<FetchFunction>,
    transform_request_body: Option<TransformRequestBody>,
}

impl CodexProvider {
    fn model_config(&self, method: &str) -> OpenAIModelConfig {
        OpenAIModelConfig::fixed(
            format!("{}.{method}", self.name),
            self.base_url.clone(),
            self.headers.clone(),
            self.fetch.clone(),
            self.transform_request_body.clone(),
        )
    }

    /// A Codex model over the Responses API (e.g. `"gpt-5.2-codex"`);
    /// `provider()` is `"{name}.responses"`.
    #[must_use]
    pub fn responses(&self, model_id: &str) -> CodexModel {
        CodexModel {
            model_id: model_id.to_string(),
            channel: self.channel,
            config: self.model_config("responses"),
        }
    }

    /// The provider as a function: the default language model for an id. The
    /// same model as [`responses`](Self::responses) and
    /// [`language_model`](Provider::language_model).
    #[must_use]
    pub fn call(&self, model_id: &str) -> Arc<dyn LanguageModel> {
        Arc::new(self.responses(model_id))
    }
}

impl Provider for CodexProvider {
    fn language_model(&self, model_id: &str) -> Result<Arc<dyn LanguageModel>, AiMuxError> {
        Ok(self.call(model_id))
    }

    fn embedding_model(&self, model_id: &str) -> Result<Arc<dyn EmbeddingModel>, AiMuxError> {
        Err(AiMuxError::no_such_model(model_id, "embeddingModel"))
    }

    fn image_model(&self, model_id: &str) -> Result<Arc<dyn ImageModel>, AiMuxError> {
        Err(AiMuxError::no_such_model(model_id, "imageModel"))
    }
}

impl ProviderDiscovery for CodexProvider {
    /// `GET {base_url}/models` (OpenAI-compatible): one exchange, no retry.
    fn list_models(&self) -> BoxFuture<'_, Result<Vec<RuntimeModel>, AiMuxError>> {
        let config = self.model_config("models");
        Box::pin(async move { crate::openai::model::list_models_once(&config).await })
    }
}

/// A Codex language model over the Responses API.
#[derive(Clone)]
pub struct CodexModel {
    model_id: String,
    channel: Channel,
    config: OpenAIModelConfig,
}

impl CodexModel {
    /// The shared Responses model, for the API-key channel.
    fn inner(&self) -> OpenAIResponsesModel {
        OpenAIResponsesModel::from_config(self.model_id.clone(), self.config.clone())
    }

    /// Build the request body; the subscription channel always streams and
    /// never stores (mirrors the official Codex client). That rule is part of
    /// this package's request construction, not a body override.
    fn subscription_body(&self, options: &CallOptions) -> (Value, Vec<Warning>) {
        let mut result = build_responses_request_body(&self.model_id, options, true);
        result.body["store"] = Value::Bool(false);
        (self.config.transform_body(result.body), result.warnings)
    }

    /// Map a subscription-channel authentication failure to
    /// [`AiMuxError::TokenExpired`] so the integrator can refresh the access
    /// token and rebuild the provider (RFC-0018 §3.2). The only credential on
    /// the subscription channel is the account token, so any 401 from the
    /// endpoint (read from the `status_code` field) means the token is
    /// invalid/expired.
    fn map_subscription_401(error: AiMuxError) -> AiMuxError {
        match error {
            AiMuxError::ApiCall(d) if d.status_code == Some(401) => {
                AiMuxError::TokenExpired(d.message)
            }
            other => other,
        }
    }

    /// Subscription `do_generate`: the endpoint only accepts streaming, so
    /// request with `stream: true` and assemble the final result from the
    /// terminal `response.completed` / `response.incomplete` event (the
    /// official client always streams; non-streaming requests error).
    async fn subscription_generate(
        &self,
        options: &CallOptions,
    ) -> Result<GenerateResult, AiMuxError> {
        let headers = self
            .config
            .request_headers(options.headers.as_ref())
            .await?;
        let (body, warnings) = self.subscription_body(options);

        let endpoint = self.config.url("/responses")?;
        let resp = aimux_provider_utils::post_json_to_api(
            self.config.http_request(endpoint.clone(), headers, options),
            body.clone(),
            aimux_provider_utils::create_event_source_response_handler::<Value>(),
            crate::openai::openai_failed_response_handler(),
        )
        .await
        .map_err(Self::map_subscription_401)?;
        let response_headers = resp.response_headers;
        let mut sse = resp.value;
        // (parsed response object, raw event payload) of the terminal event.
        let mut completed: Option<(Value, String)> = None;
        let mut failure: Option<AiMuxError> = None;

        while let Some(event) = sse.next().await {
            match event {
                Ok(data) => {
                    // The endpoint emits `data:`-only SSE frames — the event
                    // type lives in the JSON payload, not the SSE event field.
                    let etype = data.get("type").and_then(|t| t.as_str()).unwrap_or("");
                    match etype {
                        "response.completed" | "response.incomplete" => {
                            // The completed event nests the full response
                            // object under `response` — the generate-result
                            // parser expects the response object itself.
                            let raw = data.to_string();
                            let obj = data.get("response").cloned().unwrap_or(data);
                            completed = Some((obj, raw));
                            break;
                        }
                        "response.failed" | "error" => {
                            let err_obj = data
                                .get("error")
                                .or_else(|| data.get("response").and_then(|r| r.get("error")));
                            let message = err_obj
                                .and_then(|e| e.get("message"))
                                .and_then(|m| m.as_str())
                                .unwrap_or("subscription response failed");
                            // Provider-declared in-band failure: keep the
                            // observed 2xx envelope status (§2.2). The shared
                            // helper redacts the raw request context.
                            failure = Some(aimux_provider_utils::stream_error_api_call(
                                message,
                                err_obj
                                    .and_then(|e| e.get("code").or_else(|| e.get("type")))
                                    .and_then(|c| c.as_str())
                                    .map(std::string::ToString::to_string),
                                Some(200),
                                &data,
                                endpoint.clone(),
                                body.clone(),
                                response_headers.clone(),
                            ));
                            break;
                        }
                        _ => {}
                    }
                }
                Err(e) if e.is_recoverable_stream_error() => {
                    // A malformed event does not invalidate later SSE frames.
                    // Retain the first parse error in case the stream ends
                    // without a terminal response object.
                    failure.get_or_insert(e);
                }
                Err(e) => {
                    failure = Some(e);
                    break;
                }
            }
        }

        match completed {
            Some((data, raw)) => build_responses_generate_result(
                &data,
                &raw,
                warnings,
                ResponsesNamespace::OPENAI.write_key().to_string(),
                endpoint,
                body,
                response_headers,
            ),
            None => Err(failure.unwrap_or_else(|| {
                AiMuxError::InvalidResponseData(
                    "subscription stream ended without response.completed".to_string(),
                )
            })),
        }
    }

    /// Subscription `do_stream`: always streams with `store: false`.
    async fn subscription_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let headers = self
            .config
            .request_headers(options.headers.as_ref())
            .await?;
        let (body, warnings) = self.subscription_body(options);

        let endpoint = self.config.url("/responses")?;
        let resp = aimux_provider_utils::post_json_to_api(
            self.config.http_request(endpoint.clone(), headers, options),
            body.clone(),
            aimux_provider_utils::create_event_source_response_handler::<Value>(),
            crate::openai::openai_failed_response_handler(),
        )
        .await
        .map_err(Self::map_subscription_401)?;

        let response_headers = resp.response_headers;
        let mut sse_stream = resp.value;
        let first_event = match sse_stream.next().await {
            Some(Err(error @ AiMuxError::ApiCall(_))) => return Err(error),
            first_event => first_event,
        };
        let stream = build_responses_event_stream(
            first_event,
            sse_stream,
            ResponsesNamespace::OPENAI.write_key().to_string(),
            warnings,
            false,
            endpoint,
            body.clone(),
            response_headers.clone(),
        )?;

        Ok(StreamResult {
            stream,
            request_body: Some(body),
            response_headers: Some(response_headers),
        })
    }
}

#[async_trait]
impl LanguageModel for CodexModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        match self.channel {
            Channel::ApiKey => self.inner().do_generate(options).await,
            Channel::ChatGptAccount => self.subscription_generate(options).await,
        }
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        match self.channel {
            Channel::ApiKey => self.inner().do_stream(options).await,
            Channel::ChatGptAccount => self.subscription_stream(options).await,
        }
    }
}

/// Tokens returned by the Codex OAuth token endpoint (RFC-0018 §2 V3).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexTokens {
    /// Access token to pass as the `ChatGptAccount` token of [`CodexProviderSettings`].
    pub access_token: String,
    /// Rotated refresh token — **single use**; the caller must persist the
    /// new value (the library never persists credentials).
    pub refresh_token: Option<String>,
    /// Access-token lifetime in seconds, if the endpoint reports it.
    pub expires_in_secs: Option<u64>,
}

/// Stateless OAuth token-refresh helper for the Codex subscription channel
/// (RFC-0018 §3.2). Performs one `POST auth.openai.com/oauth/token` call and
/// returns the refreshed tokens — **no persistence, no orchestration**; the
/// integrator owns storage and the 401 → refresh → retry loop.
///
/// Retries are disabled on purpose: refresh tokens rotate on first use, so
/// retrying a refresh whose first response was lost would burn the rotation
/// (`refresh_token_reused`).
///
/// # Errors
///
/// Returns `ApiCall` for HTTP/transport failures of the token endpoint,
/// `JsonParse` for a malformed body, and `InvalidResponseData` when the
/// response has no `access_token`.
pub async fn codex_refresh(
    refresh_token: &str,
    client_id: &str,
) -> Result<CodexTokens, AiMuxError> {
    codex_refresh_at(refresh_token, client_id, CODEX_OAUTH_TOKEN_URL).await
}

/// [`codex_refresh`] with an explicit token endpoint (tests / self-hosted
/// identity providers).
///
/// # Errors
///
/// Returns `ApiCall` for HTTP/transport failures of the token endpoint,
/// `JsonParse` for a malformed body, and `InvalidResponseData` when the
/// response has no `access_token`.
pub async fn codex_refresh_at(
    refresh_token: &str,
    client_id: &str,
    token_url: &str,
) -> Result<CodexTokens, AiMuxError> {
    let body = json!({
        "grant_type": "refresh_token",
        "refresh_token": refresh_token,
        "client_id": client_id,
    });

    let resp = aimux_provider_utils::post_json_to_api(
        HttpRequest {
            url: token_url.to_string(),
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            abort_signal: None,
            call_id: None,
            recording_context: None,
            ..Default::default()
        },
        body,
        aimux_provider_utils::create_json_response_handler(),
        crate::openai::openai_failed_response_handler(),
    )
    .await?;

    let data: Value = resp.value;
    let access_token = data
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            AiMuxError::InvalidResponseData("oauth token response missing access_token".to_string())
        })?;

    Ok(CodexTokens {
        access_token: access_token.to_string(),
        refresh_token: data
            .get("refresh_token")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        expires_in_secs: data.get("expires_in").and_then(serde_json::Value::as_u64),
    })
}
