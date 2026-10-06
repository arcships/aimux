//! The providerOptions and providerMetadata namespace of Azure OpenAI.
//!
//! The AI SDK's Azure package reuses the OpenAI models. Its Responses model
//! reads `providerOptions.azure` (and, when a call gave no `azure` options,
//! `providerOptions.openai`, the functional fallback this port keeps) and
//! writes response metadata under `azure`. Transcription reads `azure` for API
//! routing and Speech options and `openai` for OpenAI transcription options;
//! the other models read `openai` only.

use crate::openai::responses::ResponsesNamespace;

/// The Azure providerOptions / providerMetadata key.
pub(crate) const NAMESPACE: &str = "azure";

/// What the Responses model reads and writes: `azure` first, then `openai`;
/// metadata under `azure`.
pub(crate) const RESPONSES: ResponsesNamespace =
    ResponsesNamespace::new(&[NAMESPACE, "openai"], NAMESPACE);

/// Chat uses the same option schema as the OpenAI package.
pub(crate) const CHAT_OPTIONS: crate::openai::options::ChatOptionsParser =
    crate::openai::options::parse_chat_options;
