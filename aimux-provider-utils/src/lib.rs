//! # aimux-provider-utils
//!
//! Shared utilities for provider implementations.
//!
//! Provides HTTP helpers, API key loading, header management, URL utilities,
//! and retry logic — the Rust equivalents of `@ai-sdk/provider-utils`.

pub mod api_key;
pub mod headers;
pub mod multipart;
pub mod response;
pub mod retry;
pub mod url;

pub use api_key::load_api_key;
pub use headers::with_user_agent_suffix;
pub use multipart::{MultipartForm, media_type_to_extension};
pub use response::{DEFAULT_ERROR_STRUCTURE, ErrorStructure, parse_provider_error};
pub use retry::{
    RetryConfig, get_retry_delay_ms, parse_retry_after, retry_with_exponential_backoff,
    retry_with_exponential_backoff_respecting_retry_headers,
};
pub use url::{validate_base_url, without_trailing_slash, without_trailing_slash_opt};
