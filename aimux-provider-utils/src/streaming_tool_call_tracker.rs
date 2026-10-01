//! Streaming tool call tracker.
//!
//! Rust translation of `@ai-sdk/provider-utils`'s `StreamingToolCallTracker`
//! (`packages/provider-utils/src/streaming-tool-call-tracker.ts`).
//!
//! Tracks streaming tool call state across the deltas of an OpenAI-compatible
//! chat completion stream: accumulates `arguments` fragments, emits
//! [`StreamPart::ToolInputStart`] / [`StreamPart::ToolInputDelta`] /
//! [`StreamPart::ToolInputEnd`] / [`StreamPart::ToolCall`] parts, and
//! finalizes unfinished calls on [`StreamingToolCallTracker::flush`].
//!
//! Deltas are correlated to calls by wire `id`, `index` and function name
//! (see [`StreamingToolCallTracker`]'s resolution table), not by `index`
//! alone, so providers that reuse indices, repeat ids, drop ids on
//! continuations or send blank names are handled.
//!
//! Like the TS original, a call is *never* finalized before `flush`: a
//! parsable argument buffer can still be the prefix of a longer argument
//! string, so acting on it early would use truncated inputs (ai-sdk #13137).
//!
//! This is a tool for the OpenAI chat-completions wire format only. Protocols
//! whose streams carry explicit tool-call boundaries (Anthropic content
//! blocks, Google complete `functionCall` parts, Bedrock content blocks,
//! Cohere `tool-call-*` events, the Responses API's output items) do not need
//! it.

use std::collections::{HashMap, HashSet};

use serde_json::Value;
use thiserror::Error;

use aimux_core::error::AiMuxError;
use aimux_core::stream_part::StreamPart;
use aimux_core::types::ProviderMetadata;

use crate::streaming_tool_call_argument_state::{
    StreamingToolCallArgumentState, starts_with_structured_value,
};

/// Fallback id used when the id generator returns a blank string.
const FALLBACK_TOOL_CALL_ID: &str = "tool-call";

/// The `function` sub-object of a streaming tool call delta.
#[derive(Debug, Clone, Default)]
pub struct StreamingToolCallFunction {
    pub name: Option<String>,
    pub arguments: Option<String>,
}

/// A streaming tool call delta — the `tool_calls[i]` entry of an OpenAI-style
/// streaming chunk.
///
/// `arguments: null` (TS) maps to `None`; `arguments: ''` maps to `Some("")`.
#[derive(Debug, Clone, Default)]
pub struct StreamingToolCallDelta {
    pub index: Option<usize>,
    pub id: Option<String>,
    /// The `type` field. Named `r#type` because `type` is a reserved word.
    pub r#type: Option<String>,
    pub function: Option<StreamingToolCallFunction>,
    /// Provider-specific payload carried alongside the standard fields, read
    /// by the `extract_metadata` hook (e.g. a Google thought signature).
    pub extra: Value,
}

impl StreamingToolCallDelta {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn index(mut self, index: usize) -> Self {
        self.index = Some(index);
        self
    }

    #[must_use]
    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Set the `type` field (named `tool_type` because `type` is reserved).
    #[must_use]
    pub fn tool_type(mut self, t: impl Into<String>) -> Self {
        self.r#type = Some(t.into());
        self
    }

    #[must_use]
    pub fn function_name(mut self, name: impl Into<String>) -> Self {
        self.function.get_or_insert_with(Default::default).name = Some(name.into());
        self
    }

    /// Set the `function.arguments` fragment. Pass `""` for an explicit empty
    /// fragment; omit the call for `None` (TS `arguments: null`).
    #[must_use]
    pub fn arguments(mut self, args: impl Into<String>) -> Self {
        self.function.get_or_insert_with(Default::default).arguments = Some(args.into());
        self
    }

    #[must_use]
    pub fn extra(mut self, extra: Value) -> Self {
        self.extra = extra;
        self
    }
}

/// How to validate the `type` field on a new tool call delta.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TypeValidation {
    /// No validation (default).
    #[default]
    None,
    /// Error if `type` is present and not `"function"`.
    IfPresent,
    /// Error if `type` is not exactly `"function"`.
    Required,
}

/// Errors raised while processing a tool call delta.
///
/// The TS tracker throws `InvalidResponseDataError`; these convert into
/// [`AiMuxError::InvalidResponseData`].
#[derive(Debug, Error, PartialEq, Eq)]
pub enum TrackerError {
    #[error("Expected 'function.name' to be a string.")]
    MissingFunctionName,
    #[error("Expected 'function' type.")]
    InvalidType,
    #[error("Failed to create a unique tool call ID.")]
    IdExhausted,
}

impl From<TrackerError> for AiMuxError {
    fn from(error: TrackerError) -> Self {
        AiMuxError::InvalidResponseData(error.to_string())
    }
}

struct TrackedToolCall {
    id: String,
    index: Option<usize>,
    sequence: usize,
    function_name: String,
    arguments: String,
    argument_state: StreamingToolCallArgumentState,
    has_finished: bool,
    metadata: Option<Value>,
}

enum ToolCallResolution {
    Existing(usize),
    New,
    Ambiguous,
}

type GenerateIdFn = Box<dyn Fn() -> String + Send + Sync>;
type ExtractMetadataFn = Box<dyn Fn(&StreamingToolCallDelta) -> Option<Value> + Send + Sync>;
type BuildMetadataFn = Box<dyn Fn(Option<&Value>) -> Option<ProviderMetadata> + Send + Sync>;

/// Tracks streaming tool call state across multiple deltas.
///
/// [`process_delta`](Self::process_delta) and [`flush`](Self::flush) return
/// the [`StreamPart`]s to forward downstream (the TS tracker enqueues them on
/// a controller instead).
///
/// # Correlation
///
/// | ID evidence | index/name evidence | start evidence | resolution |
/// | --- | --- | --- | --- |
/// | known | matching | any | matching call, new call, or ambiguity |
/// | known | conflicting | named | new call |
/// | unseen | matching | structured start | new call |
/// | unseen | matching | continuation | matching call or ambiguity |
/// | absent | matching | any | matching call, new call, or ambiguity |
/// | absent | absent | named | new call |
/// | absent | absent | unnamed | sole unfinished call, new call, or ambiguity |
///
/// An *ambiguous* delta is dropped.
pub struct StreamingToolCallTracker {
    tool_calls: Vec<TrackedToolCall>,
    tool_calls_by_id: HashMap<String, HashSet<usize>>,
    tool_calls_by_index: HashMap<usize, HashSet<usize>>,
    used_tool_call_ids: HashSet<String>,
    next_generated_id_suffixes: HashMap<String, usize>,
    generate_id: GenerateIdFn,
    type_validation: TypeValidation,
    extract_metadata: Option<ExtractMetadataFn>,
    build_provider_metadata: Option<BuildMetadataFn>,
}

impl Default for StreamingToolCallTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamingToolCallTracker {
    /// Create a tracker with no metadata handling and default settings.
    ///
    /// The default id generator returns a blank-free constant, so ids that
    /// need generating become `tool-call`, `tool-call-1`, `tool-call-2`, …
    #[must_use]
    pub fn new() -> Self {
        Self {
            tool_calls: Vec::new(),
            tool_calls_by_id: HashMap::new(),
            tool_calls_by_index: HashMap::new(),
            used_tool_call_ids: HashSet::new(),
            next_generated_id_suffixes: HashMap::new(),
            generate_id: Box::new(|| FALLBACK_TOOL_CALL_ID.to_string()),
            type_validation: TypeValidation::None,
            extract_metadata: None,
            build_provider_metadata: None,
        }
    }

    /// Set the id generator (the TS `generateId` option). Blank or repeated
    /// outputs are turned into usable unique ids.
    #[must_use]
    pub fn with_generate_id<F: Fn() -> String + Send + Sync + 'static>(mut self, f: F) -> Self {
        self.generate_id = Box::new(f);
        self
    }

    /// Set the `type` validation mode (the TS `typeValidation` option).
    #[must_use]
    pub fn with_type_validation(mut self, v: TypeValidation) -> Self {
        self.type_validation = v;
        self
    }

    /// Set the metadata extractor (the TS `extractMetadata` option). Called
    /// once when a new tool call is detected; the value is kept with the call
    /// and handed to the provider-metadata builder when the call finalizes.
    #[must_use]
    pub fn with_extract_metadata<
        F: Fn(&StreamingToolCallDelta) -> Option<Value> + Send + Sync + 'static,
    >(
        mut self,
        f: F,
    ) -> Self {
        self.extract_metadata = Some(Box::new(f));
        self
    }

    /// Set the provider-metadata builder (the TS
    /// `buildToolCallProviderMetadata` option). If it returns `None`, the
    /// `ToolCall` part carries no `provider_metadata`.
    #[must_use]
    pub fn with_build_provider_metadata<
        F: Fn(Option<&Value>) -> Option<ProviderMetadata> + Send + Sync + 'static,
    >(
        mut self,
        f: F,
    ) -> Self {
        self.build_provider_metadata = Some(Box::new(f));
        self
    }

    /// Process a tool call delta from a streaming chunk and return the parts
    /// it produces (possibly none).
    ///
    /// # Errors
    ///
    /// Returns [`TrackerError::InvalidType`] when `type` validation fails,
    /// [`TrackerError::MissingFunctionName`] when a new call has no
    /// (non-null) function name, and [`TrackerError::IdExhausted`] if no
    /// unique id can be produced.
    pub fn process_delta(
        &mut self,
        delta: &StreamingToolCallDelta,
    ) -> Result<Vec<StreamPart>, TrackerError> {
        let wire_name = delta.function.as_ref().and_then(|f| f.name.as_deref());
        let has_blank_name = wire_name.is_some_and(|n| n.trim().is_empty());
        let wire_id = non_blank(delta.id.as_deref());
        let name = non_blank(wire_name);
        let index = delta.index;
        let arguments = delta.function.as_ref().and_then(|f| f.arguments.as_deref());

        let resolution = self.resolve_tool_call(
            wire_id,
            index,
            name,
            name.is_some() && starts_with_structured_value(arguments),
        );

        let mut parts = Vec::new();
        let call = match resolution {
            ToolCallResolution::Ambiguous => return Ok(parts),
            ToolCallResolution::New => {
                // Blank names cannot start a usable call, but some providers
                // repeat a blank name on continuations. Those were correlated
                // above; only an unmatched blank-name delta is ignored.
                if has_blank_name {
                    return Ok(parts);
                }
                self.process_new_tool_call(delta, wire_id, index, name, &mut parts)?
            }
            ToolCallResolution::Existing(call) => {
                if let Some(wire_id) = wire_id {
                    self.associate_wire_id(call, wire_id);
                }
                self.process_existing_tool_call(call, arguments, &mut parts);
                call
            }
        };

        if let Some(index) = index {
            self.tool_calls_by_index
                .entry(index)
                .or_default()
                .insert(call);
        }
        Ok(parts)
    }

    /// Finalize any unfinished tool calls and return the closing parts. Call
    /// once when the stream ends.
    pub fn flush(&mut self) -> Vec<StreamPart> {
        // Index order is only reliable when every call has an index; for
        // mixed streams keep insertion order.
        let mut order: Vec<usize> = (0..self.tool_calls.len()).collect();
        if self.tool_calls.iter().all(|c| c.index.is_some()) {
            order.sort_by_key(|&i| (self.tool_calls[i].index, self.tool_calls[i].sequence));
        }

        let mut parts = Vec::new();
        for call in order {
            if !self.tool_calls[call].has_finished {
                self.finish_tool_call(call, &mut parts);
            }
        }
        parts
    }

    fn resolve_tool_call(
        &self,
        wire_id: Option<&str>,
        index: Option<usize>,
        name: Option<&str>,
        has_explicit_call_start: bool,
    ) -> ToolCallResolution {
        let indexed = index.and_then(|i| self.tool_calls_by_index.get(&i));
        let matching_indexed = self.filter_by_name(indexed, name);

        if let Some(wire_id) = wire_id {
            if let Some(with_id) = self.tool_calls_by_id.get(wire_id) {
                if index.is_some() {
                    let matching: Vec<usize> = matching_indexed
                        .iter()
                        .copied()
                        .filter(|c| with_id.contains(c))
                        .collect();
                    let resolved = self.resolve_matching(matching, has_explicit_call_start);
                    if !matches!(resolved, ToolCallResolution::New) {
                        return resolved;
                    }

                    // A named delta with a distinct index starts a new call
                    // even when its wire id and name repeat: providers may
                    // reuse ids across parallel calls.
                    if name.is_some() {
                        return ToolCallResolution::New;
                    }

                    // Conflicting labels on a continuation cannot be
                    // resolved safely.
                    if indexed.is_some() {
                        return ToolCallResolution::Ambiguous;
                    }

                    return self.resolve_matching(sorted(with_id), false);
                }

                if let Some(name) = name {
                    let matching: Vec<usize> = sorted(with_id)
                        .into_iter()
                        .filter(|&c| self.tool_calls[c].function_name == name)
                        .collect();
                    return self.resolve_matching(matching, has_explicit_call_start);
                }

                return self.resolve_matching(sorted(with_id), false);
            }

            if !matching_indexed.is_empty() {
                // A previously unseen id plus a named structured argument
                // start is stronger evidence of a distinct call than a reused
                // index/name; ids may still change on plain continuations.
                return if has_explicit_call_start {
                    ToolCallResolution::New
                } else {
                    self.resolve_matching(matching_indexed, false)
                };
            }

            return ToolCallResolution::New;
        }

        if indexed.is_some() {
            // Repeated names are valid on continuations; a different name at
            // the same index means a new call from a provider that reuses
            // indices across parallel calls.
            return self.resolve_matching(matching_indexed, has_explicit_call_start);
        }

        if name.is_some() {
            return ToolCallResolution::New;
        }

        let unfinished: Vec<usize> = (0..self.tool_calls.len())
            .filter(|&c| !self.tool_calls[c].has_finished)
            .collect();
        match unfinished.len() {
            0 => ToolCallResolution::New,
            1 => ToolCallResolution::Existing(unfinished[0]),
            _ => ToolCallResolution::Ambiguous,
        }
    }

    fn filter_by_name(&self, calls: Option<&HashSet<usize>>, name: Option<&str>) -> Vec<usize> {
        calls.map_or_else(Vec::new, |calls| {
            sorted(calls)
                .into_iter()
                .filter(|&c| name.is_none_or(|n| self.tool_calls[c].function_name == n))
                .collect()
        })
    }

    fn resolve_matching(
        &self,
        calls: Vec<usize>,
        has_explicit_call_start: bool,
    ) -> ToolCallResolution {
        if calls.is_empty() {
            return ToolCallResolution::New;
        }

        if !has_explicit_call_start {
            return if calls.len() == 1 {
                ToolCallResolution::Existing(calls[0])
            } else {
                ToolCallResolution::Ambiguous
            };
        }

        // A repeated name can occur on continuations. A fresh structured
        // argument prefix signals another call only once the matching call
        // has completed its own structured payload.
        let continuable: Vec<usize> = calls
            .into_iter()
            .filter(|&c| {
                !self.tool_calls[c]
                    .argument_state
                    .has_complete_structured_value()
            })
            .collect();
        match continuable.len() {
            0 => ToolCallResolution::New,
            1 => ToolCallResolution::Existing(continuable[0]),
            _ => ToolCallResolution::Ambiguous,
        }
    }

    fn process_new_tool_call(
        &mut self,
        delta: &StreamingToolCallDelta,
        wire_id: Option<&str>,
        index: Option<usize>,
        name: Option<&str>,
        parts: &mut Vec<StreamPart>,
    ) -> Result<usize, TrackerError> {
        match self.type_validation {
            TypeValidation::Required => {
                if delta.r#type.as_deref() != Some("function") {
                    return Err(TrackerError::InvalidType);
                }
            }
            TypeValidation::IfPresent => {
                if delta.r#type.as_deref().is_some_and(|t| t != "function") {
                    return Err(TrackerError::InvalidType);
                }
            }
            TypeValidation::None => {}
        }

        let name = name.ok_or(TrackerError::MissingFunctionName)?;
        let id = self.create_tool_call_id(wire_id)?;

        parts.push(StreamPart::ToolInputStart {
            id: id.clone(),
            tool_name: name.to_string(),
            provider_executed: None,
            dynamic: None,
            title: None,
            provider_metadata: None,
        });

        let metadata = self
            .extract_metadata
            .as_ref()
            .and_then(|extract| extract(delta));

        let initial_arguments = delta
            .function
            .as_ref()
            .and_then(|f| f.arguments.clone())
            .unwrap_or_default();

        let call = self.tool_calls.len();
        self.tool_calls.push(TrackedToolCall {
            id: id.clone(),
            index,
            sequence: call,
            function_name: name.to_string(),
            argument_state: StreamingToolCallArgumentState::new(&initial_arguments),
            arguments: initial_arguments.clone(),
            has_finished: false,
            metadata,
        });
        if let Some(wire_id) = wire_id {
            self.associate_wire_id(call, wire_id);
        }

        if !initial_arguments.is_empty() {
            parts.push(StreamPart::ToolInputDelta {
                id,
                delta: initial_arguments,
                provider_metadata: None,
            });
        }

        // Tool calls must not finalize before the stream ends (ai-sdk
        // #13137); finalization happens in `flush`.
        Ok(call)
    }

    fn process_existing_tool_call(
        &mut self,
        call: usize,
        arguments: Option<&str>,
        parts: &mut Vec<StreamPart>,
    ) {
        let tool_call = &mut self.tool_calls[call];
        if tool_call.has_finished {
            return;
        }
        if let Some(arguments) = arguments {
            tool_call.argument_state.append(arguments);
            tool_call.arguments.push_str(arguments);
            parts.push(StreamPart::ToolInputDelta {
                id: tool_call.id.clone(),
                delta: arguments.to_string(),
                provider_metadata: None,
            });
        }
    }

    fn associate_wire_id(&mut self, call: usize, wire_id: &str) {
        self.tool_calls_by_id
            .entry(wire_id.to_string())
            .or_default()
            .insert(call);
    }

    fn create_tool_call_id(&mut self, wire_id: Option<&str>) -> Result<String, TrackerError> {
        if let Some(wire_id) = wire_id
            && !self.used_tool_call_ids.contains(wire_id)
        {
            self.used_tool_call_ids.insert(wire_id.to_string());
            return Ok(wire_id.to_string());
        }

        let generated = non_blank(Some(&(self.generate_id)()))
            .unwrap_or(FALLBACK_TOOL_CALL_ID)
            .to_string();

        if !self.used_tool_call_ids.contains(&generated) {
            self.used_tool_call_ids.insert(generated.clone());
            return Ok(generated);
        }

        // Resume after the last suffix checked for this generated value so
        // deterministic generators stay bounded without rescanning occupied
        // suffixes.
        let initial_suffix = self
            .next_generated_id_suffixes
            .get(&generated)
            .copied()
            .unwrap_or(1);
        let maximum_suffix = initial_suffix + self.used_tool_call_ids.len();
        for suffix in initial_suffix..=maximum_suffix {
            let suffixed = format!("{generated}-{suffix}");
            if !self.used_tool_call_ids.contains(&suffixed) {
                self.used_tool_call_ids.insert(suffixed.clone());
                self.next_generated_id_suffixes
                    .insert(generated, suffix + 1);
                return Ok(suffixed);
            }
        }

        // Unreachable by the pigeonhole principle; guards the invariant
        // instead of looping without bound.
        Err(TrackerError::IdExhausted)
    }

    fn finish_tool_call(&mut self, call: usize, parts: &mut Vec<StreamPart>) {
        let tool_call = &mut self.tool_calls[call];
        tool_call.has_finished = true;

        parts.push(StreamPart::ToolInputEnd {
            id: tool_call.id.clone(),
            provider_metadata: None,
        });

        let provider_metadata = self
            .build_provider_metadata
            .as_ref()
            .and_then(|build| build(tool_call.metadata.as_ref()));

        parts.push(StreamPart::ToolCall {
            tool_call_id: tool_call.id.clone(),
            tool_name: tool_call.function_name.clone(),
            // Raw argument text; Core parses it after `stream_text`.
            input: Value::String(tool_call.arguments.clone()),
            provider_executed: None,
            dynamic: None,
            thought_signature: None,
            invalid: None,
            error: None,
            provider_metadata,
        });
    }
}

fn non_blank(value: Option<&str>) -> Option<&str> {
    value.filter(|v| !v.trim().is_empty())
}

fn sorted(calls: &HashSet<usize>) -> Vec<usize> {
    let mut calls: Vec<usize> = calls.iter().copied().collect();
    calls.sort_unstable();
    calls
}
