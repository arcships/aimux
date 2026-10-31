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
pub mod convert_image_model_file_to_data_uri;
pub mod convert_inline_file_data_to_uint8_array;
pub mod create_language_model_response_metadata;
pub mod create_provider_stream_error;
pub mod create_tool_name_mapping;
pub mod decision_language_model;
pub mod download_blob;
pub mod extract_response_headers;
pub mod fetch;
pub mod generate_id;
pub mod get_from_api;
pub mod handle_fetch_error;
pub mod headers;
pub mod http;
pub mod inject_json_instruction;
pub mod is_valid_hostname_part;
pub mod logging;
pub mod map_reasoning_to_provider;
pub mod media_type;
pub mod multipart;
pub mod parse_provider_options;
pub mod post_to_api;
pub mod provider_headers;
pub mod read_response_with_size_limit;
pub mod resolvable;
pub mod resolve_provider_reference;
pub mod response_handler;
pub mod sigv4_fetch;
pub mod strip_file_extension;
#[cfg(test)]
mod tests;
pub mod tool_result_to_content;
pub mod transform_stream;
pub mod uint8_utils;
pub mod url;
pub mod user_agent_fetch;
/// WebSocket client for realtime provider APIs (RFC-0028). Empty unless the
/// `ws` feature is enabled.
#[cfg(feature = "ws")]
pub mod ws;

pub use aimux_core::download_guard::same_origin;
pub use api_key::{base_url_setting, load_api_key, load_optional_setting, load_setting};
pub use convert_image_model_file_to_data_uri::convert_image_model_file_to_data_uri;
pub use convert_inline_file_data_to_uint8_array::convert_inline_file_data_to_uint8_array;
pub use create_language_model_response_metadata::create_language_model_response_metadata;
pub use create_provider_stream_error::{ProviderStreamError, create_provider_stream_error};
pub use create_tool_name_mapping::{ToolNameMapping, create_tool_name_mapping};
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
pub use inject_json_instruction::{inject_json_instruction, inject_json_instruction_into_messages};
pub use is_valid_hostname_part::is_valid_hostname_part;
pub use logging::{init_logging, redact_error_context};
pub use map_reasoning_to_provider::{
    DEFAULT_MIN_REASONING_BUDGET, DEFAULT_REASONING_BUDGET_PERCENTAGES,
    map_reasoning_to_provider_budget, map_reasoning_to_provider_effort,
};
pub use media_type::{
    MAX_ID3_TAG_BYTES, MediaTypeData, detect_media_type, get_top_level_media_type,
    is_full_media_type, resolve_full_media_type,
};
pub use multipart::{MultipartForm, media_type_to_extension};
pub use parse_provider_options::{optional_field, parse_provider_options};
pub use post_to_api::{post_form_data_to_api, post_json_to_api, post_to_api};
pub use provider_headers::{AuthScheme, Credential, ProviderHeaders};
pub use resolvable::Resolvable;
pub use resolve_provider_reference::resolve_provider_reference;
pub use response_handler::{
    ProviderErrorParts, ResponseHandler, ResponseHandlerInput, ResponseHandlerOutput,
    create_binary_response_handler, create_event_source_response_handler,
    create_json_error_response_handler, create_json_response_handler,
    create_status_code_error_response_handler, invalid_response_api_call,
};
pub use sigv4_fetch::{AwsCredentials, SigV4Fetch, SigV4Signing};
pub use strip_file_extension::strip_file_extension;
pub use tool_result_to_content::{tool_result_content_value, tool_result_to_content};
pub use transform_stream::{PipeThrough, TransformStreamController, Transformer, pipe_through};
pub use uint8_utils::{
    Base64OrBytes, InvalidBase64Error, convert_base64_to_uint8_array, convert_to_base64,
    convert_uint8_array_to_base64,
};
pub use url::{validate_base_url, without_trailing_slash};
pub use user_agent_fetch::{UserAgentFetch, transport_user_agent};
