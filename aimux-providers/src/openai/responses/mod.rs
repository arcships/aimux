//! `responses/`: the OpenAI Responses API language model.

pub(super) mod convert_responses_usage;
pub(super) mod convert_to_responses_input;
pub(super) mod map_responses_finish_reason;
pub(super) mod responses_api;
pub(super) mod responses_language_model;
pub(super) mod responses_language_model_options;
pub(super) mod responses_prepare_tools;
pub(super) mod tool_args;

pub use convert_responses_usage::{convert_responses_usage, parse_usage};
pub use convert_to_responses_input::{ResponsesInputResult, convert_to_responses_input};
pub use map_responses_finish_reason::map_responses_finish_reason;
pub use responses_api::{
    ResponsesInputTokensDetails, ResponsesOutputTokensDetails, ResponsesUsage,
};
pub use responses_language_model::{
    OpenAIResponsesModel, ResponsesRequestBodyResult, build_responses_request_body,
    build_responses_request_body_for,
};
pub use responses_language_model_options::ResponsesNamespace;
pub(crate) use responses_language_model_options::ResponsesProfile;
pub use responses_prepare_tools::{PreparedResponsesTools, prepare_responses_tools};

/// The Responses output parser and event stream Codex reuses.
pub(crate) mod responses_convert {
    pub(crate) use super::responses_language_model::{
        build_responses_event_stream_with_raw, build_responses_generate_result,
    };
}
