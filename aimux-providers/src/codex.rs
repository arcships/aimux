//! Codex provider (RFC-0018) — OpenAI Codex models over the Responses API.
//!
//! Codex models (`gpt-5.2-codex`, `gpt-5.1-codex`, `gpt-5.1-codex-mini`,
//! `gpt-5-codex`, …) are available **only** through the Responses API
//! (verified 2026-08-03, RFC-0018 §2 V1).
//!
//! Two access modes ([`CodexMode`]):
//!
//! - **ApiKey** (default): `https://api.openai.com/v1/responses` — the
//!   official, documented, usage-based channel. Recommended for CI/automation.
//! - **Subscription**: `https://chatgpt.com/backend-api/codex/responses` —
//!   the ChatGPT-subscription channel (single personal account, per OpenAI
//!   ToU). Undocumented endpoint — **best-effort, no SLA**. Mirrors the
//!   official Codex client: **always streams** (`stream: true`, `store: false`).
//!
//! OAuth is *not* the library's job (RFC-0018 §3.2 responsibility split): the
//! integrator performs the device-code login, persists tokens, and orchestrates
//! refresh. The library exposes [`codex_refresh`] as a stateless protocol
//! helper and maps subscription-channel 401 responses to
//! [`AiMuxError::TokenExpired`] so the integrator can refresh and retry.
//!
//! Scope boundaries (RFC-0018): no account pools, no quota rotation, no
//! reselling — single account, single user.

use std::collections::HashMap;

use async_trait::async_trait;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::options::CallOptions;
use aimux_core::provider::Provider;
use aimux_core::result::{GenerateResult, StreamResult};
use aimux_core::types::Warning;

use aimux_provider_utils::response::DEFAULT_ERROR_STRUCTURE;
use aimux_provider_utils::{
    HttpBody, HttpMethod, HttpRequest, RetryConfig, send, send_stream_timed,
};
use aimux_stream::SseStream;

use crate::openai::responses::responses_convert::{
    build_header_list, build_responses_event_stream, build_responses_generate_result,
};
use crate::openai::responses::{OpenAIResponsesModel, build_responses_request_body};
use crate::openai::{OpenAICompatProfile, OpenAIConfig};

/// Default API-key base URL (official OpenAI Responses endpoint).
pub const CODEX_API_BASE_URL: &str = "https://api.openai.com/v1";
/// Default subscription base URL (undocumented ChatGPT backend endpoint).
pub const CODEX_SUBSCRIPTION_BASE_URL: &str = "https://chatgpt.com/backend-api";
/// Environment variable read by [`CodexConfig::from_env`].
pub const CODEX_API_KEY_ENV_VAR: &str = "CODEX_API_KEY";
/// OAuth token endpoint used by [`codex_refresh`] (RFC-0018 §2 V3).
pub const CODEX_OAUTH_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";

/// Access mode for the Codex provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexMode {
    /// Official API-key channel (`api.openai.com`), usage-based.
    ApiKey,
    /// ChatGPT-subscription channel (`chatgpt.com/backend-api`), single
    /// personal account; always streams.
    Subscription,
}

/// Configuration for the Codex provider.
#[derive(Debug, Clone)]
pub struct CodexConfig {
    openai: OpenAIConfig,
    mode: CodexMode,
    /// Subscription mode: `ChatGPT-Account-Id` header value (optional).
    chatgpt_account_id: Option<String>,
    /// Subscription mode: `Originator` header value (client identifier;
    /// reference clients use e.g. `"opencode"` / `"codex_cli_rs"`).
    originator: String,
}

impl CodexConfig {
    /// API-key mode with the official base URL.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            openai: OpenAIConfig::new(api_key)
                .with_base_url(CODEX_API_BASE_URL)
                .with_provider("codex")
                .with_profile(OpenAICompatProfile::full()),
            mode: CodexMode::ApiKey,
            chatgpt_account_id: None,
            originator: "aimux".to_string(),
        }
    }

    /// Subscription mode with the ChatGPT backend base URL.
    ///
    /// `account_token` is the access token minted by the integrator's OAuth
    /// device-code flow — the library never performs login, persists tokens,
    /// or refreshes automatically (RFC-0018 §3.2).
    pub fn subscription(account_token: impl Into<String>) -> Self {
        Self {
            openai: OpenAIConfig::new(account_token)
                .with_base_url(CODEX_SUBSCRIPTION_BASE_URL)
                .with_provider("codex")
                .with_profile(OpenAICompatProfile::full()),
            mode: CodexMode::Subscription,
            chatgpt_account_id: None,
            originator: "aimux".to_string(),
        }
    }

    /// API-key mode reading `CODEX_API_KEY` from the environment.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let key = aimux_provider_utils::load_api_key(None, CODEX_API_KEY_ENV_VAR, "Codex")?;
        Ok(Self::new(key))
    }

    /// Override the base URL (tests / self-hosted endpoints).
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.openai = self.openai.with_base_url(url);
        self
    }

    /// Subscription mode: set the `ChatGPT-Account-Id` header.
    pub fn with_chatgpt_account_id(mut self, id: impl Into<String>) -> Self {
        self.chatgpt_account_id = Some(id.into());
        self
    }

    /// Subscription mode: override the `Originator` header (default `"aimux"`).
    pub fn with_originator(mut self, originator: impl Into<String>) -> Self {
        self.originator = originator.into();
        self
    }

    /// Override the retry configuration.
    pub fn with_retry_config(mut self, config: RetryConfig) -> Self {
        self.openai = self.openai.with_retry_config(config);
        self
    }

    /// Extra headers merged into every request (user-supplied values win over
    /// the subscription-mode defaults).
    pub fn with_headers(mut self, headers: HashMap<String, String>) -> Self {
        self.openai = self.openai.with_headers(headers);
        self
    }
}

/// Codex provider — creates [`CodexModel`] instances.
pub struct CodexProvider {
    config: CodexConfig,
}

impl CodexProvider {
    pub fn new(config: CodexConfig) -> Self {
        Self { config }
    }

    /// Create a model instance for the given Codex model id
    /// (e.g. `"gpt-5.2-codex"`).
    pub fn model(&self, model_id: &str) -> CodexModel {
        CodexModel {
            model_id: model_id.to_string(),
            config: self.config.clone(),
        }
    }
}

impl Provider for CodexProvider {
    fn name(&self) -> &str {
        "codex"
    }

    fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Ok(Box::new(self.model(model_id)))
    }
}

/// A Codex language model over the Responses API.
#[derive(Debug, Clone)]
pub struct CodexModel {
    model_id: String,
    config: CodexConfig,
}

impl CodexModel {
    fn inner(&self) -> OpenAIResponsesModel {
        OpenAIResponsesModel::new(self.model_id.clone(), self.config.openai.clone())
    }

    fn endpoint(&self) -> String {
        format!("{}/responses", self.config.openai.base_url)
    }

    /// Subscription mode: fold the channel headers (`Originator`,
    /// `ChatGPT-Account-Id`) into the per-call options. User-supplied header
    /// values win (`entry().or_insert`).
    fn subscription_options(&self, options: &CallOptions) -> CallOptions {
        let mut opts = options.clone();
        let mut headers = opts.headers.take().unwrap_or_default();
        headers
            .entry("Originator".to_string())
            .or_insert_with(|| self.config.originator.clone());
        if let Some(id) = &self.config.chatgpt_account_id {
            headers
                .entry("ChatGPT-Account-Id".to_string())
                .or_insert_with(|| id.clone());
        }
        opts.headers = Some(headers);
        opts
    }

    /// Build the request body; the subscription channel always streams and
    /// never stores (mirrors the official Codex client).
    fn body(
        &self,
        options: &CallOptions,
        stream: bool,
    ) -> Result<(Value, Vec<Warning>), AiMuxError> {
        let mut result = build_responses_request_body(&self.model_id, options, stream);
        if self.config.mode == CodexMode::Subscription {
            result.body["store"] = Value::Bool(false);
        }
        Ok((result.body, result.warnings))
    }

    /// Map a subscription-channel authentication failure to
    /// [`AiMuxError::TokenExpired`] so the integrator can refresh the access
    /// token and retry (RFC-0018 §3.2). The only credential on the
    /// subscription channel is the account token, so any `Auth` error (401
    /// from the endpoint) means the token is invalid/expired.
    fn map_subscription_401(&self, error: AiMuxError) -> AiMuxError {
        match error {
            AiMuxError::Auth(message) => AiMuxError::TokenExpired(message),
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
        let opts = self.subscription_options(options);
        let headers = self.inner().build_headers(opts.headers.as_ref());
        let (body, warnings) = self.body(&opts, true)?;

        let resp = send_stream_timed(
            HttpRequest {
                method: HttpMethod::Post,
                url: self.endpoint(),
                headers: build_header_list(&headers),
                body: HttpBody::Json(body.clone()),
                abort_signal: opts.abort_signal.clone(),
            },
            self.config.openai.retry_config,
            &DEFAULT_ERROR_STRUCTURE,
            opts.timeout.map(Into::into),
        )
        .await
        .map_err(|e| self.map_subscription_401(e))?;

        let response_headers = resp.headers;
        let mut sse = SseStream::new(resp.body);
        let mut completed: Option<Value> = None;
        let mut failure: Option<AiMuxError> = None;

        while let Some(event) = sse.next().await {
            match event {
                Ok(ev) => {
                    let data: Value = match serde_json::from_str(&ev.data) {
                        Ok(v) => v,
                        Err(e) => {
                            failure = Some(AiMuxError::Json(e.to_string()));
                            break;
                        }
                    };
                    // The endpoint emits `data:`-only SSE frames — the event
                    // type lives in the JSON payload, not the SSE event field.
                    let etype = data.get("type").and_then(|t| t.as_str()).unwrap_or("");
                    match etype {
                        "response.completed" | "response.incomplete" => {
                            // The completed event nests the full response
                            // object under `response` — the generate-result
                            // parser expects the response object itself.
                            completed = Some(data.get("response").cloned().unwrap_or(data));
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
                            failure = Some(AiMuxError::Provider(message.to_string()));
                            break;
                        }
                        _ => {}
                    }
                }
                Err(e) => {
                    failure = Some(AiMuxError::Stream(e.to_string()));
                    break;
                }
            }
        }

        match completed {
            Some(data) => build_responses_generate_result(
                &data,
                warnings,
                "openai".to_string(),
                body,
                response_headers,
            ),
            None => Err(failure.unwrap_or_else(|| {
                AiMuxError::Stream(
                    "subscription stream ended without response.completed".to_string(),
                )
            })),
        }
    }

    /// Subscription `do_stream`: always streams with `store: false`.
    async fn subscription_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let opts = self.subscription_options(options);
        let headers = self.inner().build_headers(opts.headers.as_ref());
        let (body, warnings) = self.body(&opts, true)?;

        let resp = send_stream_timed(
            HttpRequest {
                method: HttpMethod::Post,
                url: self.endpoint(),
                headers: build_header_list(&headers),
                body: HttpBody::Json(body.clone()),
                abort_signal: opts.abort_signal.clone(),
            },
            self.config.openai.retry_config,
            &DEFAULT_ERROR_STRUCTURE,
            opts.timeout.map(Into::into),
        )
        .await
        .map_err(|e| self.map_subscription_401(e))?;

        let response_headers = resp.headers;
        let mut sse_stream = SseStream::new(resp.body);
        let first_event = sse_stream.next().await;
        let stream = build_responses_event_stream(
            first_event,
            sse_stream,
            "openai".to_string(),
            warnings,
            false,
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
        "codex"
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        match self.config.mode {
            CodexMode::ApiKey => self.inner().do_generate(options).await,
            CodexMode::Subscription => self.subscription_generate(options).await,
        }
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        match self.config.mode {
            CodexMode::ApiKey => self.inner().do_stream(options).await,
            CodexMode::Subscription => self.subscription_stream(options).await,
        }
    }
}

/// Tokens returned by the Codex OAuth token endpoint (RFC-0018 §2 V3).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexTokens {
    /// Access token to pass as `CodexConfig::subscription(..)`.
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
pub async fn codex_refresh(
    refresh_token: &str,
    client_id: &str,
) -> Result<CodexTokens, AiMuxError> {
    codex_refresh_at(refresh_token, client_id, CODEX_OAUTH_TOKEN_URL).await
}

/// [`codex_refresh`] with an explicit token endpoint (tests / self-hosted
/// identity providers).
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

    let resp = send(
        HttpRequest {
            method: HttpMethod::Post,
            url: token_url.to_string(),
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            body: HttpBody::Json(body),
            abort_signal: None,
        },
        RetryConfig {
            max_retries: 0,
            ..RetryConfig::default()
        },
        &DEFAULT_ERROR_STRUCTURE,
    )
    .await?;

    let data: Value =
        serde_json::from_slice(&resp.body).map_err(|e| AiMuxError::Json(e.to_string()))?;
    let access_token = data
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            AiMuxError::Provider("oauth token response missing access_token".to_string())
        })?;

    Ok(CodexTokens {
        access_token: access_token.to_string(),
        refresh_token: data
            .get("refresh_token")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        expires_in_secs: data.get("expires_in").and_then(|v| v.as_u64()),
    })
}
