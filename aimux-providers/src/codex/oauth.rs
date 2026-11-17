//! Codex OAuth token types and stateless refresh helpers (RFC-0018).

use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_provider_utils::HttpRequest;

use super::CodexTokens;
use crate::openai::internal::openai_failed_response_handler;

/// OAuth token endpoint used by [`codex_refresh`] (RFC-0018 §2 V3).
pub const CODEX_OAUTH_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";

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
        openai_failed_response_handler(),
    )
    .await?;

    let data: Value = resp.value;
    let access_token = data
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            AiMuxError::invalid_response_data("oauth token response missing access_token")
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

#[cfg(test)]
mod tests {
    use super::CodexTokens;

    #[test]
    fn tokens_debug_redacts_secrets() {
        let tokens = CodexTokens {
            access_token: "at-secret-value".to_string(),
            refresh_token: Some("rt-secret-value".to_string()),
            expires_in_secs: Some(3600),
        };
        let printed = format!("{tokens:?}");
        assert!(!printed.contains("at-secret-value"));
        assert!(!printed.contains("rt-secret-value"));
        assert!(printed.contains("3600"));
    }
}
