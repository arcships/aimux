//! The xAI Responses API (`src/responses/` of `@ai-sdk/xai`).

mod convert_responses_usage;
mod convert_to_responses_input;
mod map_responses_finish_reason;
mod responses_api;
mod responses_language_model;
mod responses_language_model_options;
mod responses_prepare_tools;

pub use convert_responses_usage::convert_xai_responses_usage;
pub use convert_to_responses_input::{convert_to_xai_responses_input, resolve_provider_reference};
pub use map_responses_finish_reason::map_xai_responses_finish_reason;
pub use responses_api::{
    XaiResponsesInputTokensDetails, XaiResponsesOutputTokensDetails, XaiResponsesResponse,
    XaiResponsesUsage, event_type,
};
pub use responses_language_model::{
    ResponsesRequestBodyResult, WEB_SEARCH_SUB_TOOLS, X_SEARCH_SUB_TOOLS, XaiResponsesModel,
    build_responses_request_body, get_tool_input, resolve_tool_name,
};
pub use responses_prepare_tools::{PreparedResponsesTools, prepare_responses_tools};
