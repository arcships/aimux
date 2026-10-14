//! # aimux-provider-utils
//!
//! Shared utilities for provider implementations.
//!
//! Provides the pluggable [`Fetch`] transport, one-exchange HTTP helpers,
//! response handlers, API key and setting loading, [`Resolvable`] values and
//! layered header management, URL utilities and the streamed tool-call
//! tracker for the OpenAI chat-completions wire format — the Rust equivalents
//! of `@ai-sdk/provider-utils`. Operation retry and timeout live in
//! `aimux-core`.

mod aisdk_pins;
pub mod api_key;
pub mod decision_language_model;
pub mod download_blob;
pub mod extract_response_headers;
pub mod fetch;
pub mod generate_id;
pub mod get_from_api;
pub mod handle_fetch_error;
pub mod headers;
pub mod http;
pub mod logging;
pub mod media_type;
pub mod multipart;
pub mod post_to_api;
pub mod read_response_with_size_limit;
pub mod resolvable;
pub mod response_handler;
pub mod sigv4_fetch;
mod streaming_tool_call_argument_state;
mod streaming_tool_call_tracker;
pub mod transform_stream;
pub mod url;
/// WebSocket client for realtime provider APIs (RFC-0028). Empty unless the
/// `ws` feature is enabled.
#[cfg(feature = "ws")]
pub mod ws;

pub use aimux_core::download_guard::same_origin;
pub use api_key::{load_api_key, load_optional_setting, load_setting};
pub use decision_language_model::DecisionLanguageModel;
pub use download_blob::download_blob;
pub use fetch::{
    Fetch, FetchError, FetchFunction, FetchRequest, FetchResponse, PinnedFetch, ProxyConfig,
    RequestCredentials, ReqwestFetch, default_fetch, set_default_transport,
};
pub use generate_id::generate_id;
pub use get_from_api::get_from_api;
pub use headers::{
    HeaderMapOpt, HeadersFn, combine_headers, normalize_headers, with_user_agent_suffix,
};
pub use http::{ExchangeContext, HttpBody, HttpRequest, sleep_or_abort};
pub use logging::{init_logging, redact_error_context};
pub use media_type::{
    MAX_ID3_TAG_BYTES, MediaTypeData, detect_media_type, get_top_level_media_type,
    is_full_media_type, resolve_full_media_type,
};
pub use multipart::{MultipartForm, media_type_to_extension};
pub use post_to_api::{post_form_data_to_api, post_json_to_api, post_to_api};
pub use resolvable::Resolvable;
pub use response_handler::{
    ProviderErrorParts, ResponseHandler, ResponseHandlerInput, ResponseHandlerOutput,
    create_binary_response_handler, create_event_source_response_handler,
    create_json_error_response_handler, create_json_response_handler,
    create_status_code_error_response_handler, invalid_response_api_call, stream_error_api_call,
};
pub use sigv4_fetch::{AwsCredentials, SigV4Fetch, SigV4Signing};
// As `@ai-sdk/provider-utils` (`index.ts:115-119`): the tracker is exported
// from the crate root, its argument state stays internal.
pub use streaming_tool_call_tracker::{
    StreamingToolCallDelta, StreamingToolCallTracker, TrackerError, TypeValidation,
};
pub use transform_stream::{PipeThrough, TransformStreamController, Transformer, pipe_through};
pub use url::{validate_base_url, without_trailing_slash, without_trailing_slash_opt};
