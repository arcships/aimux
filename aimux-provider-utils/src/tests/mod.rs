//! Ported upstream `@ai-sdk/provider-utils` tests, one file per upstream
//! `.test.ts` (RFC-0044 L6).

mod convert_image_model_file_to_data_uri;
mod convert_inline_file_data_to_uint8_array;
mod create_provider_stream_error;
mod create_tool_name_mapping;
mod map_reasoning_to_provider;
mod resolve_provider_reference;
mod strip_file_extension;
mod tool_result_to_content;
mod uint8_utils;
