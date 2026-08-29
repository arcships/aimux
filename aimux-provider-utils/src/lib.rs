//! # aimux-provider-utils
//!
//! Shared utilities for provider implementations.
//!
//! Provides one-exchange HTTP helpers, response handlers, API key loading,
//! header management, and URL utilities — the Rust equivalents of
//! `@ai-sdk/provider-utils`. Operation retry and timeout live in `aimux-core`.

pub mod api_key;
mod download_guard;
pub mod extract_response_headers;
pub mod get_from_api;
pub mod handle_fetch_error;
pub mod headers;
pub mod http;
pub mod logging;
pub mod multipart;
pub mod post_to_api;
pub mod read_response_with_size_limit;
pub mod response_handler;
pub mod retry;
pub mod url;
/// WebSocket client for realtime provider APIs (RFC-0028). Empty unless the
/// `ws` feature is enabled.
#[cfg(feature = "ws")]
pub mod ws;

pub use api_key::load_api_key;
pub use download_guard::same_origin;
pub use get_from_api::get_from_api;
pub use headers::with_user_agent_suffix;
pub use http::{HttpBody, HttpRequest, ProxyConfig, init_proxy, shared_client, sleep_or_abort};
pub use logging::{body_logging_enabled, init_logging, redact_body, redact_error_context};
pub use multipart::{MultipartForm, media_type_to_extension};
pub use post_to_api::{post_form_data_to_api, post_json_to_api, post_to_api};
pub use response_handler::{
    ProviderErrorParts, ResponseHandler, ResponseHandlerInput, ResponseHandlerOutput,
    create_binary_response_handler, create_event_source_response_handler,
    create_json_error_response_handler, create_json_response_handler,
    create_status_code_error_response_handler, stream_error_api_call,
};
pub use retry::RetryConfig;
pub use url::{validate_base_url, without_trailing_slash, without_trailing_slash_opt};
