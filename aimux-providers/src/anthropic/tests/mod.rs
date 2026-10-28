//! Tests ported from `@ai-sdk/anthropic` 4.0.71, one file per upstream test file.

mod convert_to_prompt;
mod convert_usage;
mod evaluation;
mod files;
mod language_model;
mod language_model_stream;
mod message_lifecycle;
mod prepare_tools;
mod provider;
mod remaining_support;
#[allow(clippy::module_inception)]
mod sanitize_json_schema;
mod skills;
