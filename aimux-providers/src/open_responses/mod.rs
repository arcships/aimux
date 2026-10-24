//! Open Responses provider - a generic Responses API wrapper.
//!
//! Works with any OpenAI Responses-compatible API endpoint (LM Studio,
//! OpenAI, etc.). Unlike the OpenAI Chat Completions provider, this speaks
//! the Responses API wire format (`/v1/responses`).
//!
//! Translation of `reference/ai/packages/open-responses/src/responses/`.

mod provider;
mod responses;

pub use provider::{OpenResponsesProvider, OpenResponsesProviderSettings, create_open_responses};
pub use responses::{
    OpenResponsesModel, convert_to_open_responses_input, map_open_responses_finish_reason,
};

#[cfg(test)]
mod tests;
