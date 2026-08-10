//! Azure OpenAI Responses API language model.
//!
//! Implements the [`LanguageModel`] trait against the Azure OpenAI
//! `/responses` endpoint. Azure speaks the same Responses API wire format as
//! OpenAI (requests carry an `input` array, streaming events are typed as
//! `response.output_text.delta`, etc.) but differs in:
//!
//! 1. **URL construction** — Azure addresses a *deployment* and pins the API
//!    version via a `?api-version=` query parameter. The v1 form is
//!    `{base}/v1/responses?api-version={v}`; the deployment-based form is
//!    `{base}/deployments/{deployment}/responses?api-version={v}`.
//! 2. **Authentication** — either an `api-key` header (API key) or a `Bearer`
//!    token supplied by a [`TokenProvider`] (Azure AD / Microsoft Entra ID).
//!    The two are mutually exclusive.
//! 3. **File ID prefix** — Azure file IDs use the `assistant-` prefix. When a
//!    file content part's data starts with `assistant-`, it is passed through
//!    as a `file_id` instead of being base64-encoded.
//!
//! The request-body building logic is shared with the OpenAI Responses
//! provider via [`crate::openai::responses::convert`]; the non-streaming
//! output parsing and the streaming SSE event reducer are shared via
//! [`crate::openai::responses::responses_convert`] (RFC-0012 §3.5).
//!
//! Mirrors the TS `createResponsesModel` factory in
//! `azure-openai-provider.ts`.

use std::collections::HashMap;

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateResult, StreamResult};

use aimux_provider_utils::response::DEFAULT_ERROR_STRUCTURE;
use aimux_provider_utils::{
    HttpBody, HttpMethod, HttpRequest, send_stream_timed, send_timed, with_user_agent_suffix,
    without_trailing_slash,
};
use aimux_stream::SseStream;

use crate::azure::{AzureAuth, AzureConfig};
use crate::openai::responses::convert::build_responses_request_body;
use crate::openai::responses::responses_convert::{
    build_header_list, build_responses_event_stream, build_responses_generate_result,
};

/// The file-ID prefix Azure uses for uploaded files.
const AZURE_FILE_ID_PREFIX: &str = "assistant-";

/// An Azure OpenAI Responses API language model (addressed by deployment name).
///
/// Created via [`AzureProvider::responses_model`](super::AzureProvider::responses_model).
///
/// This mirrors the TS `OpenAIResponsesLanguageModel` configured with
/// `provider: 'azure.responses'` and `fileIdPrefixes: ['assistant-']`.
pub struct AzureResponsesModel {
    deployment: String,
    config: AzureConfig,
}

impl AzureResponsesModel {
    pub fn new(deployment: String, config: AzureConfig) -> Self {
        Self { deployment, config }
    }

    /// Build the Responses endpoint URL for this deployment.
    ///
    /// v1 form (default): `{base}/v1/responses?api-version={v}`
    ///
    /// Deployment-based: `{base}/deployments/{deployment}/responses?api-version={v}`
    ///
    /// Custom gateway (non-Azure baseURL, v1 form): `{base}/responses` — the
    /// gateway owns its own versioning, so `api-version` is omitted.
    fn endpoint(&self) -> String {
        let prefix = match &self.config.base_url {
            Some(url) => without_trailing_slash(url),
            None => format!(
                "https://{}.openai.azure.com/openai",
                self.config
                    .resource_name
                    .as_deref()
                    .expect("resource_name or base_url required (validated at construction)")
            ),
        };

        let path = if self.config.use_deployment_based_urls {
            format!("/deployments/{}/responses", self.deployment)
        } else {
            "/v1/responses".to_string()
        };

        // Only append api-version for the Azure OpenAI endpoint or the
        // deployment-based form. A custom (non-Azure) gateway baseURL with the
        // v1 form owns its own versioning.
        let is_azure = prefix.contains(".openai.azure.com");
        if is_azure || self.config.use_deployment_based_urls {
            format!("{}{}?api-version={}", prefix, path, self.config.api_version)
        } else {
            format!("{}{}", prefix, path)
        }
    }

    /// Build the request headers: auth + provider extra headers + per-request
    /// headers, plus the `ai-sdk/azure/…` user-agent suffix.
    async fn build_headers(
        &self,
        extra: Option<&HashMap<String, String>>,
    ) -> Result<HashMap<String, String>, AiMuxError> {
        let mut headers = HashMap::new();

        match &self.config.auth {
            Some(AzureAuth::ApiKey(key)) => {
                headers.insert("api-key".to_string(), key.clone());
            }
            Some(AzureAuth::TokenProvider(tp)) => {
                let token = tp.get_token().await?;
                headers.insert("Authorization".to_string(), format!("Bearer {}", token));
            }
            None => {
                return Err(AiMuxError::Auth(
                    "Azure OpenAI has no authentication configured. \
                     Provide an `api_key` or a `token_provider`."
                        .to_string(),
                ));
            }
        }

        // Provider-level extra headers.
        for (k, v) in &self.config.extra_headers {
            headers.insert(k.clone(), v.clone());
        }
        // Per-request headers override provider headers.
        if let Some(extra) = extra {
            for (k, v) in extra {
                headers.insert(k.clone(), v.clone());
            }
        }

        with_user_agent_suffix(&mut headers, "azure");
        Ok(headers)
    }

    /// Post-process the request body to apply the Azure `assistant-` file-ID
    /// prefix: when a file content part's base64 data starts with `assistant-`,
    /// emit a `file_id` field instead of `image_url`/`file_data`.
    ///
    /// Mirrors the TS `fileIdPrefixes: ['assistant-']` option.
    fn apply_file_id_prefixes(body: &mut Value) {
        if let Some(input) = body.get_mut("input").and_then(|v| v.as_array_mut()) {
            for msg in input.iter_mut() {
                if let Some(content) = msg.get_mut("content").and_then(|v| v.as_array_mut()) {
                    for part in content.iter_mut() {
                        apply_prefix_to_part(part);
                    }
                }
            }
        }
    }
}

/// Check if a content part's `image_url` or `file_data` contains an
/// `assistant-` prefixed data URL and, if so, replace it with a `file_id`.
///
/// For `input_file` parts whose media type is an image, the type is also
/// changed to `input_image` — mirroring the TS behaviour where image files
/// are sent as `input_image` with a `file_id`.
fn apply_prefix_to_part(part: &mut Value) {
    // input_image: { type: "input_image", image_url: "data:<mime>;base64,<data>" }
    if part.get("type").and_then(|v| v.as_str()) == Some("input_image") {
        let file_id = part
            .get("image_url")
            .and_then(|v| v.as_str())
            .and_then(extract_base64_data)
            .filter(|data| data.starts_with(AZURE_FILE_ID_PREFIX))
            .map(|s| s.to_string());
        if let Some(file_id) = file_id
            && let Some(obj) = part.as_object_mut()
        {
            obj.remove("image_url");
            obj.insert("file_id".to_string(), json!(file_id));
        }
    }

    // input_file: { type: "input_file", file_data: "data:<mime>;base64,<data>" }
    if part.get("type").and_then(|v| v.as_str()) == Some("input_file") {
        let file_data = part
            .get("file_data")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        if let Some(file_data) = file_data {
            let media_type = extract_media_type(&file_data).unwrap_or("");
            let data = extract_base64_data(&file_data);
            if let Some(data) = data
                && data.starts_with(AZURE_FILE_ID_PREFIX)
                && let Some(obj) = part.as_object_mut()
            {
                obj.remove("file_data");
                obj.remove("filename");
                obj.insert("file_id".to_string(), json!(data));
                // If the media type is an image, switch to input_image
                // (mirrors TS behaviour).
                if media_type.starts_with("image/") {
                    obj.insert("type".to_string(), json!("input_image"));
                }
            }
        }
    }
}

/// Extract the MIME type from a `data:<mime>;base64,<payload>` URL.
fn extract_media_type(data_url: &str) -> Option<&str> {
    let prefix = data_url.strip_prefix("data:")?;
    let end = prefix.find(";base64,")?;
    Some(&prefix[..end])
}

/// Extract the base64 payload from a `data:<mime>;base64,<payload>` URL.
/// Returns `None` if the URL doesn't match the expected format.
fn extract_base64_data(data_url: &str) -> Option<&str> {
    let marker = ";base64,";
    data_url.split_once(marker).map(|(_, rest)| rest)
}

/// The provider-metadata key: always `"azure"` for the Azure responses model.
fn provider_key() -> &'static str {
    "azure"
}

#[async_trait]
impl LanguageModel for AzureResponsesModel {
    fn provider(&self) -> &str {
        "azure.responses"
    }

    fn model_id(&self) -> &str {
        &self.deployment
    }

    fn config_snapshot(&self) -> aimux_core::recording::ProviderRecord {
        use aimux_core::recording::ProviderRecord;
        ProviderRecord {
            provider: self.provider().to_string(),
            model_id: self.model_id().to_string(),
            base_url: self.config.base_url.clone(),
            // Azure 不记录明文 key/token;来源按构造方式标注(env/explicit)。
            api_key_source: self
                .config
                .api_key_source
                .clone()
                .unwrap_or_else(|| "explicit".to_string()),
            profile: None,
            provider_options: Some(serde_json::json!({
                "resource_name": self.config.resource_name,
                "api_version": self.config.api_version,
                "use_deployment_based_urls": self.config.use_deployment_based_urls,
            })),
        }
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        let headers = self.build_headers(options.headers.as_ref()).await?;
        let request_result = build_responses_request_body(&self.deployment, options, false);
        let mut body = request_result.body;

        // Apply Azure assistant- file ID prefix passthrough.
        Self::apply_file_id_prefixes(&mut body);

        let provider_key = provider_key().to_string();
        let retry_config = crate::openai::model::resolve_retry_config(
            &self.config.retry_config,
            options.max_retries,
        );

        let resp = send_timed(
            HttpRequest {
                method: HttpMethod::Post,
                url: self.endpoint(),
                headers: build_header_list(&headers),
                body: HttpBody::Json(body.clone()),

                abort_signal: options.abort_signal.clone(),
                call_id: options.call_id.clone(),
                recording_context: options.recording_context.clone(),
            },
            retry_config,
            &DEFAULT_ERROR_STRUCTURE,
            options.timeout.map(Into::into),
        )
        .await?;

        let response_headers = resp.headers;

        let data: Value =
            serde_json::from_slice(&resp.body).map_err(|e| AiMuxError::Json(e.to_string()))?;

        build_responses_generate_result(
            &data,
            request_result.warnings,
            provider_key,
            body,
            response_headers,
        )
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let headers = self.build_headers(options.headers.as_ref()).await?;
        let request_result = build_responses_request_body(&self.deployment, options, true);
        let mut body = request_result.body;
        let warnings = request_result.warnings;
        let provider_key = provider_key().to_string();

        // Apply Azure assistant- file ID prefix passthrough.
        Self::apply_file_id_prefixes(&mut body);

        // The `store` request option (None by default). Used to decide when
        // reasoning summary parts are concluded.
        let store_flag = options
            .provider_options
            .as_ref()
            .and_then(|m| m.get("openai"))
            .and_then(|o| o.get("store"))
            .and_then(|v| v.as_bool())
            == Some(true);

        let retry_config = crate::openai::model::resolve_retry_config(
            &self.config.retry_config,
            options.max_retries,
        );

        let resp = send_stream_timed(
            HttpRequest {
                method: HttpMethod::Post,
                url: self.endpoint(),
                headers: build_header_list(&headers),
                body: HttpBody::Json(body.clone()),

                abort_signal: options.abort_signal.clone(),
                call_id: options.call_id.clone(),
                recording_context: options.recording_context.clone(),
            },
            retry_config,
            &DEFAULT_ERROR_STRUCTURE,
            options.timeout.map(Into::into),
        )
        .await?;

        let response_headers = resp.headers;

        let mut sse_stream = SseStream::new(resp.body);
        let first_event = sse_stream.next().await;
        let stream = build_responses_event_stream(
            first_event,
            sse_stream,
            provider_key,
            warnings,
            store_flag,
        )?;

        Ok(StreamResult {
            stream,
            request_body: Some(body),
            response_headers: Some(response_headers),
        })
    }
}

// ── Unit tests ───────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_snapshot_matches_config() {
        let m = AzureResponsesModel::new(
            "gpt-4o".to_string(),
            AzureConfig::new()
                .with_base_url("https://gateway.example.com/openai")
                .with_api_version("2025-04-01-preview")
                .use_v1_urls()
                .with_api_key("k"),
        );
        let snap = m.config_snapshot();
        assert_eq!(snap.provider, "azure.responses");
        assert_eq!(snap.model_id, "gpt-4o");
        assert_eq!(
            snap.base_url.as_deref(),
            Some("https://gateway.example.com/openai")
        );
        assert_eq!(snap.api_key_source, "explicit");
    }
}
