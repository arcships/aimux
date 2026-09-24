//! OpenAI Responses API language model.
//!
//! Implements the [`LanguageModel`] trait against the `/v1/responses`
//! endpoint. The Responses API uses a different request/response format from
//! chat completions: requests carry an `input` array (not `messages`), and
//! streaming events are typed as `response.output_text.delta`,
//! `response.function_call_arguments.delta`, etc.
//!
//! Mirrors the TS `OpenAIResponsesLanguageModel`. The core paths implemented
//! here are:
//! - **Request building**: input array, instructions, store,
//!   previous_response_id, reasoning (effort/summary), response_format
//!   (json_schema/json_object) — see [`convert::build_responses_request_body`].
//! - **Non-streaming**: text, function-call, and reasoning output items -- see
//!   [`OpenAIResponsesModel::do_generate`].
//! - **Streaming**: the `response.created -> output_item.added ->
//!   output_text.delta -> output_text.done -> output_item.done ->
//!   response.completed` main path, plus `function_call_arguments.delta/done`
//!   and `reasoning_summary_text.delta` -- see [`OpenAIResponsesModel::do_stream`].
//!
//! The non-streaming output parser and the streaming SSE event reducer are
//! shared with the Azure OpenAI Responses provider via
//! [`responses_convert`] (RFC-0012 §3.5).
//!
//! Provider configuration follows the chat-completions model
//! (`super::OpenAIModel`): `OpenAIConfig.headers` / `org_id` / `project` and
//! `OpenAIConfig.body_overrides` (RFC-0017) are applied, per-call
//! `CallOptions.headers` / `body_overrides` win, and provider options are read
//! from the config-derived namespace returned by
//! [`OpenAIResponsesModel::provider_options_name`] (`"azure"` / `"openai"`,
//! with the AI SDK's `"openai"` fallback).

pub mod convert;
pub mod responses_convert;
pub mod types;

pub use convert::{
    ResponsesInputResult, ResponsesRequestBodyResult, ResponsesRequestConfig,
    build_responses_request_body, build_responses_request_body_with_config,
    convert_responses_usage, convert_to_responses_input, map_responses_finish_reason,
    prepare_responses_tools,
};

use std::collections::HashMap;

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::Value;

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateResult, StreamResult};

use aimux_provider_utils::HttpRequest;

use super::OpenAIConfig;
use responses_convert::build_header_list;

/// An OpenAI Responses API language model.
///
/// Does **not** hold an HTTP client — the `aimux-provider-utils` API helpers use
/// the process-wide shared `Client` internally (RFC-0009 §4.1).
///
/// Created via `OpenAIResponsesProvider` or
/// directly with [`OpenAIResponsesModel::new`].
pub struct OpenAIResponsesModel {
    model_id: String,
    config: OpenAIConfig,
}

impl OpenAIResponsesModel {
    #[must_use]
    pub fn new(model_id: String, config: OpenAIConfig) -> Self {
        Self { model_id, config }
    }

    /// Build the request headers: provider-level config headers
    /// (`OpenAIConfig.headers` / organization / project), then per-call
    /// `CallOptions.headers` (which win).
    ///
    /// Shares [`super::model::build_auth_headers`] with the chat path, so both
    /// wire formats send identical auth/config headers (issue #166 B6: the
    /// Responses model used to drop `config.headers` and `config.project`).
    pub(crate) fn build_headers(
        &self,
        extra: Option<&HashMap<String, String>>,
    ) -> HashMap<String, String> {
        let mut headers = super::model::build_auth_headers(&self.config);
        if let Some(extra) = extra {
            for (k, v) in extra {
                headers.insert(k.clone(), v.clone());
            }
        }
        headers
    }

    fn endpoint(&self) -> String {
        format!("{}/responses", self.config.base_url)
    }

    /// The provider-options namespace: `"azure"` when the provider string
    /// contains `"azure"`, otherwise `"openai"`. Mirrors the TS
    /// `providerOptionsName` in `openai-responses-language-model.ts`.
    ///
    /// Used for **both** directions: the request builder reads
    /// `provider_options[<name>]` (falling back to `"openai"` when the
    /// provider's own namespace is absent, as the TS `parseProviderOptions`
    /// fallback does) and response provider-metadata is emitted under the same
    /// key. Aimux extension: `config.provider` is the registry name, so any
    /// non-Azure registry provider (e.g. `"codex"`, `"openrouter"`) resolves to
    /// `"openai"` — the same namespace the AI SDK uses for those endpoints.
    fn provider_options_name(&self) -> &str {
        if self.config.provider.contains("azure") {
            "azure"
        } else {
            "openai"
        }
    }

    /// The provider-level request configuration: the provider-options
    /// namespace (see [`Self::provider_options_name`]) and
    /// `OpenAIConfig.body_overrides` (RFC-0017), applied to the built body
    /// before per-call overrides.
    fn request_config(&self) -> ResponsesRequestConfig<'_> {
        ResponsesRequestConfig {
            provider_options_name: self.provider_options_name(),
            body_overrides: self.config.body_overrides.as_ref(),
        }
    }
}

#[async_trait]
impl LanguageModel for OpenAIResponsesModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn retry_config(&self) -> aimux_core::retry::RetryConfig {
        self.config.retry_config
    }

    fn config_snapshot(&self) -> aimux_core::recording::ProviderRecord {
        super::config_snapshot_from_config(&self.config.provider, &self.model_id, &self.config)
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        let headers = self.build_headers(options.headers.as_ref());
        let request_result = build_responses_request_body_with_config(
            &self.model_id,
            options,
            false,
            self.request_config(),
        );
        let body = request_result.body;
        let provider_key = self.provider_options_name().to_string();

        let endpoint = self.endpoint();
        let resp = aimux_provider_utils::post_json_to_api(
            HttpRequest::new(endpoint.clone(), build_header_list(&headers), options),
            body.clone(),
            aimux_provider_utils::create_json_response_handler::<Value>(),
            super::openai_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;
        let raw_body = resp
            .raw_value
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default();
        let data = resp.value;

        responses_convert::build_responses_generate_result(
            &data,
            &raw_body,
            request_result.warnings,
            provider_key,
            endpoint,
            body,
            response_headers,
        )
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let headers = self.build_headers(options.headers.as_ref());
        let request_result = build_responses_request_body_with_config(
            &self.model_id,
            options,
            true,
            self.request_config(),
        );
        let body = request_result.body;
        let warnings = request_result.warnings;
        let provider_key = self.provider_options_name().to_string();

        // Whether the effective body stores the response. Used to decide when
        // reasoning summary parts are concluded; derived from the body (not
        // from provider options) so overrides cannot diverge from the request.
        let store_flag = responses_convert::store_requested(&body);

        let endpoint = self.endpoint();
        let resp = aimux_provider_utils::post_json_to_api(
            HttpRequest::new(endpoint.clone(), build_header_list(&headers), options),
            body.clone(),
            aimux_provider_utils::create_event_source_response_handler::<Value>(),
            super::openai_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;

        let mut sse_stream = resp.value;
        let first_event = match sse_stream.next().await {
            Some(Err(error @ AiMuxError::ApiCall(_))) => return Err(error),
            first_event => first_event,
        };
        let stream = responses_convert::build_responses_event_stream(
            first_event,
            sse_stream,
            provider_key,
            warnings,
            store_flag,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn model(config: OpenAIConfig) -> OpenAIResponsesModel {
        OpenAIResponsesModel::new("gpt-4o".to_string(), config)
    }

    fn headers(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    /// `OpenAIConfig.headers` and `project` reach the request headers, and the
    /// auth headers the Responses model already sent are unchanged (chat-path
    /// `build_auth_headers` parity).
    #[test]
    fn build_headers_applies_project_and_config_headers() {
        let config = OpenAIConfig::new("test-key")
            .with_org_id("org_123")
            .with_project("proj_123")
            .with_headers(headers(&[("x-provider-header", "provider-value")]));

        let built = model(config).build_headers(None);

        assert_eq!(
            built.get("Authorization").map(String::as_str),
            Some("Bearer test-key")
        );
        assert_eq!(
            built.get("OpenAI-Organization").map(String::as_str),
            Some("org_123")
        );
        assert_eq!(
            built.get("OpenAI-Project").map(String::as_str),
            Some("proj_123")
        );
        assert_eq!(
            built.get("x-provider-header").map(String::as_str),
            Some("provider-value")
        );
    }

    /// Per-call headers override provider-level ones without dropping the rest.
    #[test]
    fn build_headers_per_call_wins() {
        let config = OpenAIConfig::new("test-key")
            .with_headers(headers(&[("x-scope", "provider"), ("x-kept", "yes")]));

        let built = model(config).build_headers(Some(&headers(&[("x-scope", "call")])));

        assert_eq!(built.get("x-scope").map(String::as_str), Some("call"));
        assert_eq!(built.get("x-kept").map(String::as_str), Some("yes"));
    }

    /// The provider-options namespace follows the configured provider name.
    #[test]
    fn provider_options_name_follows_config_provider() {
        assert_eq!(
            model(OpenAIConfig::new("k")).provider_options_name(),
            "openai"
        );
        assert_eq!(
            model(OpenAIConfig::new("k").with_provider("codex")).provider_options_name(),
            "openai"
        );
        assert_eq!(
            model(OpenAIConfig::new("k").with_provider("azure")).provider_options_name(),
            "azure"
        );
        assert_eq!(
            model(OpenAIConfig::new("k").with_provider("azure_openai")).provider_options_name(),
            "azure"
        );
    }

    /// The streaming `store` flag is derived from the effective request body:
    /// explicit `true` stores, an omitted `store` keeps the historical
    /// "not explicitly stored" behavior.
    #[test]
    fn store_requested_reads_effective_body() {
        assert!(responses_convert::store_requested(
            &serde_json::json!({ "store": true })
        ));
        assert!(!responses_convert::store_requested(
            &serde_json::json!({ "store": false })
        ));
        assert!(!responses_convert::store_requested(&serde_json::json!({})));
        assert!(!responses_convert::store_requested(
            &serde_json::json!({ "store": null })
        ));
        assert!(!responses_convert::store_requested(
            &serde_json::json!({ "store": "true" })
        ));
    }

    /// The model's request config carries the config-derived namespace and the
    /// provider-level body overrides.
    #[test]
    fn request_config_carries_provider_settings() {
        let config = model(
            OpenAIConfig::new("k")
                .with_provider("azure")
                .with_body_overrides(serde_json::json!({ "store": false })),
        );
        let request_config = config.request_config();
        assert_eq!(request_config.provider_options_name, "azure");
        assert_eq!(
            request_config.body_overrides,
            Some(&serde_json::json!({ "store": false }))
        );

        assert!(
            model(OpenAIConfig::new("k"))
                .request_config()
                .body_overrides
                .is_none()
        );
    }
}
