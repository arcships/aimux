//! Codex provider (RFC-0018) — OpenAI Codex models over the Responses API.
//!
//! [`create_codex`] takes [`CodexProviderSettings`], validates the base URL,
//! fixes the provider name and returns a [`CodexProvider`]. Credentials are not
//! read there; they are resolved in the request headers of every call, so a
//! host that refreshes the ChatGPT access token supplies an
//! [`Resolvable::AsyncFn`](aimux_provider_utils::Resolvable::AsyncFn) or
//! rebuilds the provider. The AI SDK has no Codex package; the factory follows
//! the shape of the OpenAI one.
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
//! [`AiMuxError::TokenExpired`](aimux_core::error::AiMuxError::TokenExpired)
//! so the integrator can refresh and rebuild the provider.
//!
//! Scope boundaries (RFC-0018): no account pools, no quota rotation, no
//! reselling — single account, single user.

mod oauth;
mod provider;
mod responses_model;

pub use crate::shared::TransformRequestBody;
pub use oauth::{CODEX_OAUTH_TOKEN_URL, CodexTokens, codex_refresh, codex_refresh_at};
pub use provider::{
    CODEX_API_BASE_URL, CODEX_API_KEY_ENV_VAR, CODEX_SUBSCRIPTION_BASE_URL, CodexMode,
    CodexProvider, CodexProviderSettings, codex, create_codex,
};
pub use responses_model::CodexModel;
