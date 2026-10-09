//! Streaming tool call tracker for the OpenAI chat-completions wire format.
//!
//! Port of `@ai-sdk/provider-utils`'s `streaming-tool-call-tracker.ts`. It
//! accumulates `tool_calls[i]` deltas, emits [`StreamPart::ToolInputStart`] /
//! [`StreamPart::ToolInputDelta`] as they arrive, and emits
//! [`StreamPart::ToolInputEnd`] / [`StreamPart::ToolCall`] for every call in
//! [`StreamingToolCallTracker::finish`]. A call is never finalized earlier: a
//! parsable argument buffer can still be the prefix of a longer argument
//! string (ai-sdk #13137).
//!
//! Deltas are correlated to calls by wire `id`, `index` and function name,
//! not by `index` alone, so providers that reuse indices, repeat ids, drop
//! ids on continuations or send blank names are handled:
//!
//! | ID evidence | index/name evidence | start evidence | resolution |
//! | --- | --- | --- | --- |
//! | known | matching | any | matching call, new call, or ambiguity |
//! | known | conflicting | named | new call |
//! | unseen | matching | structured start | new call |
//! | unseen | matching | continuation | matching call or ambiguity |
//! | absent | matching | any | matching call, new call, or ambiguity |
//! | absent | absent | named | new call |
//! | absent | absent | unnamed | sole call, new call, or ambiguity |
//!
//! An ambiguous delta is dropped.
//!
//! Protocols whose streams carry explicit tool-call boundaries (Anthropic,
//! Google, Bedrock, Cohere, the Responses API) do not need this.

use serde_json::Value;
use thiserror::Error;

use aimux_core::error::AiMuxError;
use aimux_core::stream_part::StreamPart;
use aimux_core::types::ProviderMetadata;

use crate::streaming_tool_call_argument_state::{
    StreamingToolCallArgumentState, starts_with_structured_value,
};

/// Id used when the id generator returns a blank string; also the output of
/// the default generator.
const FALLBACK_TOOL_CALL_ID: &str = "tool-call";

/// One `tool_calls[i]` entry of a streaming chunk, borrowed from the
/// provider's own wire type.
///
/// The upstream `function: { name, arguments }` object is flattened; a
/// missing `function` is the same as both fields being `None`.
#[derive(Debug, Clone, Default)]
pub struct StreamingToolCallDelta<'a> {
    pub index: Option<usize>,
    pub id: Option<&'a str>,
    pub r#type: Option<&'a str>,
    pub name: Option<&'a str>,
    /// `None` is the wire `null` / absent; `Some("")` is an empty fragment.
    pub arguments: Option<&'a str>,
    /// Provider metadata for the `ToolCall` part (the upstream
    /// `extractMetadata` hook). Read only from the delta that starts a call.
    pub provider_metadata: Option<ProviderMetadata>,
}

/// How to validate the `type` field on the delta that starts a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TypeValidation {
    /// No validation.
    #[default]
    None,
    /// Reject a `type` that is present and not `"function"`.
    IfPresent,
    /// Reject a `type` that is not exactly `"function"`.
    Required,
}

/// A delta the tracker cannot accept; the upstream
/// `InvalidResponseDataError`.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum TrackerError {
    #[error("Expected 'function.name' to be a string.")]
    MissingFunctionName,
    #[error("Expected 'function' type.")]
    InvalidType,
}

impl From<TrackerError> for AiMuxError {
    fn from(error: TrackerError) -> Self {
        AiMuxError::InvalidResponseData(error.to_string())
    }
}

struct ToolCall {
    id: String,
    name: String,
    arguments: String,
    argument_state: StreamingToolCallArgumentState,
    /// Index of the starting delta; orders [`StreamingToolCallTracker::finish`].
    index: Option<usize>,
    /// Every wire id and index seen for this call.
    wire_ids: Vec<String>,
    indices: Vec<usize>,
    provider_metadata: Option<ProviderMetadata>,
}

enum Resolution {
    Existing(usize),
    New,
    Ambiguous,
}

/// Tracks the tool calls of one streamed response. See the module docs.
pub struct StreamingToolCallTracker {
    calls: Vec<ToolCall>,
    generate_id: Box<dyn Fn() -> String + Send + Sync>,
    type_validation: TypeValidation,
}

impl Default for StreamingToolCallTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamingToolCallTracker {
    /// A tracker without `type` validation whose generated ids are
    /// `tool-call`, `tool-call-1`, `tool-call-2`, …
    #[must_use]
    pub fn new() -> Self {
        Self {
            calls: Vec::new(),
            generate_id: Box::new(|| FALLBACK_TOOL_CALL_ID.to_string()),
            type_validation: TypeValidation::None,
        }
    }

    /// Set the generator for calls without a usable wire id. Blank or
    /// repeated outputs are turned into unique ids.
    #[must_use]
    pub fn with_generate_id(mut self, f: impl Fn() -> String + Send + Sync + 'static) -> Self {
        self.generate_id = Box::new(f);
        self
    }

    #[must_use]
    pub fn with_type_validation(mut self, validation: TypeValidation) -> Self {
        self.type_validation = validation;
        self
    }

    /// Process one delta, appending the parts it produces to `out`.
    ///
    /// # Errors
    ///
    /// A delta that starts a call without a function name, or whose `type`
    /// fails validation.
    pub fn process(
        &mut self,
        delta: StreamingToolCallDelta<'_>,
        out: &mut Vec<StreamPart>,
    ) -> Result<(), TrackerError> {
        let StreamingToolCallDelta {
            index,
            id,
            r#type,
            name: wire_name,
            arguments,
            provider_metadata,
        } = delta;
        let wire_id = non_blank(id);
        let name = non_blank(wire_name);
        let explicit_start = name.is_some() && starts_with_structured_value(arguments);

        let call = match self.resolve(wire_id, index, name, explicit_start) {
            Resolution::Ambiguous => return Ok(()),
            // Blank names cannot start a usable call, but some providers
            // repeat a blank name on continuations; those were correlated
            // above, so only an unmatched blank-name delta is ignored.
            Resolution::New if wire_name.is_some() && name.is_none() => return Ok(()),
            Resolution::New => {
                self.validate_type(r#type)?;
                let name = name.ok_or(TrackerError::MissingFunctionName)?;
                self.start(
                    wire_id,
                    index,
                    name,
                    arguments.unwrap_or(""),
                    provider_metadata,
                    out,
                )
            }
            Resolution::Existing(call) => {
                let tool_call = &mut self.calls[call];
                if let Some(wire_id) = wire_id
                    && !tool_call.wire_ids.iter().any(|w| w == wire_id)
                {
                    tool_call.wire_ids.push(wire_id.to_string());
                }
                if let Some(arguments) = arguments {
                    tool_call.argument_state.append(arguments);
                    tool_call.arguments.push_str(arguments);
                    out.push(StreamPart::ToolInputDelta {
                        id: tool_call.id.clone(),
                        delta: arguments.to_string(),
                        provider_metadata: None,
                    });
                }
                call
            }
        };

        let indices = &mut self.calls[call].indices;
        if let Some(index) = index
            && !indices.contains(&index)
        {
            indices.push(index);
        }
        Ok(())
    }

    /// Finalize every call, appending `ToolInputEnd` and `ToolCall` parts to
    /// `out`. Calls are ordered by index when every call has one, otherwise
    /// by arrival.
    pub fn finish(mut self, out: &mut Vec<StreamPart>) {
        if self.calls.iter().all(|c| c.index.is_some()) {
            self.calls.sort_by_key(|c| c.index); // stable: ties keep arrival order
        }
        for call in self.calls {
            out.push(StreamPart::ToolInputEnd {
                id: call.id.clone(),
                provider_metadata: None,
            });
            out.push(StreamPart::ToolCall {
                tool_call_id: call.id,
                tool_name: call.name,
                // Raw argument text; Core parses it.
                input: Value::String(call.arguments),
                provider_executed: None,
                dynamic: None,
                thought_signature: None,
                invalid: None,
                error: None,
                provider_metadata: call.provider_metadata,
            });
        }
    }

    /// Apply the correlation table in the module docs.
    fn resolve(
        &self,
        wire_id: Option<&str>,
        index: Option<usize>,
        name: Option<&str>,
        explicit_start: bool,
    ) -> Resolution {
        let at_index = |c: &ToolCall| index.is_some_and(|i| c.indices.contains(&i));
        let named = |c: &ToolCall| name.is_none_or(|n| c.name == n);
        let index_known = self.calls.iter().any(at_index);

        let Some(wire_id) = wire_id else {
            if index_known {
                // Repeated names are valid on continuations; a different name
                // at the same index is a new call from a provider that reuses
                // indices across parallel calls.
                return self.matching(|c| at_index(c) && named(c), explicit_start);
            }
            if name.is_some() {
                return Resolution::New;
            }
            return self.matching(|_| true, false);
        };

        let has_id = |c: &ToolCall| c.wire_ids.iter().any(|w| w == wire_id);
        if !self.calls.iter().any(has_id) {
            // A previously unseen id plus a named structured start is
            // stronger evidence of a distinct call than a reused index/name;
            // ids may still change on plain continuations.
            if explicit_start {
                return Resolution::New;
            }
            return self.matching(|c| at_index(c) && named(c), false);
        }

        if index.is_none() {
            return match name {
                Some(name) => self.matching(|c| has_id(c) && c.name == name, explicit_start),
                None => self.matching(has_id, false),
            };
        }

        let resolution = self.matching(|c| has_id(c) && at_index(c) && named(c), explicit_start);
        if !matches!(resolution, Resolution::New) {
            return resolution;
        }
        if name.is_some() {
            // A named delta at a distinct index starts a new call even when
            // its id and name repeat: providers may reuse ids across
            // parallel calls.
            return Resolution::New;
        }
        if index_known {
            // Conflicting labels on a continuation cannot be resolved safely.
            return Resolution::Ambiguous;
        }
        self.matching(has_id, false)
    }

    /// Resolve to the single call matching `predicate`. With an explicit
    /// structured start, a call whose structured arguments are already
    /// complete cannot continue, so it is not a candidate.
    fn matching(&self, predicate: impl Fn(&ToolCall) -> bool, explicit_start: bool) -> Resolution {
        let mut candidates = self.calls.iter().enumerate().filter(|(_, c)| {
            predicate(c) && !(explicit_start && c.argument_state.has_complete_structured_value())
        });
        match (candidates.next(), candidates.next()) {
            (None, _) => Resolution::New,
            (Some((call, _)), None) => Resolution::Existing(call),
            (Some(_), Some(_)) => Resolution::Ambiguous,
        }
    }

    fn validate_type(&self, r#type: Option<&str>) -> Result<(), TrackerError> {
        let valid = match self.type_validation {
            TypeValidation::None => true,
            TypeValidation::IfPresent => r#type.is_none_or(|t| t == "function"),
            TypeValidation::Required => r#type == Some("function"),
        };
        if valid {
            Ok(())
        } else {
            Err(TrackerError::InvalidType)
        }
    }

    fn start(
        &mut self,
        wire_id: Option<&str>,
        index: Option<usize>,
        name: &str,
        arguments: &str,
        provider_metadata: Option<ProviderMetadata>,
        out: &mut Vec<StreamPart>,
    ) -> usize {
        let id = self.unique_id(wire_id);
        out.push(StreamPart::ToolInputStart {
            id: id.clone(),
            tool_name: name.to_string(),
            provider_executed: None,
            dynamic: None,
            title: None,
            provider_metadata: None,
        });
        if !arguments.is_empty() {
            out.push(StreamPart::ToolInputDelta {
                id: id.clone(),
                delta: arguments.to_string(),
                provider_metadata: None,
            });
        }
        self.calls.push(ToolCall {
            id,
            name: name.to_string(),
            arguments: arguments.to_string(),
            argument_state: StreamingToolCallArgumentState::new(arguments),
            index,
            wire_ids: wire_id.map(str::to_string).into_iter().collect(),
            indices: Vec::new(),
            provider_metadata,
        });
        self.calls.len() - 1
    }

    /// The wire id if unused, else the generated id, else the generated id
    /// with the first free `-N` suffix.
    fn unique_id(&self, wire_id: Option<&str>) -> String {
        let used = |id: &str| self.calls.iter().any(|c| c.id == id);
        if let Some(wire_id) = wire_id
            && !used(wire_id)
        {
            return wire_id.to_string();
        }
        let generated = (self.generate_id)();
        let generated = non_blank(Some(&generated)).unwrap_or(FALLBACK_TOOL_CALL_ID);
        if !used(generated) {
            return generated.to_string();
        }
        // Ids are never released, so the first free suffix equals what the
        // upstream resume-from-last-suffix search returns.
        (1..)
            .map(|suffix| format!("{generated}-{suffix}"))
            .find(|id| !used(id))
            .expect("finitely many ids are in use")
    }
}

fn non_blank(value: Option<&str>) -> Option<&str> {
    value.filter(|v| !v.trim().is_empty())
}
