//! Streaming tool call tracker.
//!
//! Rust translation of `@ai-sdk/provider-utils`'s `StreamingToolCallTracker`
//! (`packages/provider-utils/src/streaming-tool-call-tracker.ts`).
//!
//! Tracks streaming tool call state across multiple deltas from an
//! OpenAI-compatible chat completion stream: accumulates `arguments` string
//! fragments by `index`, emits `tool-input-start` / `tool-input-delta` /
//! `tool-input-end` / `tool-call` events, and finalizes any unfinished tool
//! calls on [`StreamingToolCallTracker::flush`].
//!
//! Like the TS original, a tool call is *never* finalized before `flush` — a
//! parsable argument buffer can still be the prefix of a longer argument
//! string, so acting on it early would use truncated inputs (ai-sdk #13137).

use serde_json::Value;
use thiserror::Error;

/// Default upper bound accepted for a tool call `index`.
const DEFAULT_MAX_INDEX: usize = 1024;

/// The `function` sub-object of a streaming tool call delta.
#[derive(Debug, Clone, Default)]
pub struct StreamingToolCallFunction {
    pub name: Option<String>,
    pub arguments: Option<String>,
}

/// A streaming tool call delta — the `tool_calls[i]` entry of an OpenAI-style
/// streaming chunk.
///
/// Use the builder methods ([`StreamingToolCallDelta::index`], etc.) to
/// construct one ergonomically. `arguments: null` (TS) maps to `None`; an
/// empty-string `arguments: ''` maps to `Some("")`.
#[derive(Debug, Clone, Default)]
pub struct StreamingToolCallDelta {
    pub index: Option<usize>,
    pub id: Option<String>,
    /// The `type` field. Named `r#type` because `type` is a reserved word.
    pub r#type: Option<String>,
    pub function: Option<StreamingToolCallFunction>,
    /// Provider-specific payload carried alongside the standard fields. Used by
    /// `extract_metadata` to pull out provider metadata (e.g. a Google thought
    /// signature). Defaults to [`Value::Null`].
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
    /// fragment; omit the call entirely for `None` (TS `arguments: null`).
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

/// The stream parts emitted by [`StreamingToolCallTracker`].
///
/// Mirrors the subset of `LanguageModelV4StreamPart` the TS tracker enqueues.
/// Note that [`ToolCallStreamPart::ToolCall::input`] is the raw accumulated
/// argument *string* — the tracker does not parse it (matching the TS
/// behavior).
#[derive(Debug, Clone, PartialEq)]
pub enum ToolCallStreamPart<M = ()> {
    /// Start of a tool call's input streaming.
    ToolInputStart { id: String, tool_name: String },
    /// A delta of tool call input (a partial argument fragment).
    ToolInputDelta { id: String, delta: String },
    /// End of a tool call's input streaming.
    ToolInputEnd { id: String },
    /// A complete, finalized tool call.
    ToolCall {
        tool_call_id: String,
        tool_name: String,
        input: String,
        provider_metadata: Option<M>,
    },
}

/// How to validate the `type` field on a new tool call delta.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TypeValidation {
    /// No validation (default).
    #[default]
    None,
    /// Throw if `type` is present and not `"function"`.
    IfPresent,
    /// Throw if `type` is not exactly `"function"`.
    Required,
}

/// Errors raised while processing a tool call delta.
#[derive(Debug, Error, PartialEq)]
pub enum TrackerError {
    #[error("Expected 'id' to be a string.")]
    MissingId,
    #[error("Expected 'function.name' to be a string.")]
    MissingFunctionName,
    #[error("Expected 'function' type.")]
    InvalidType,
    #[error("Tool call index out of range")]
    IndexOutOfRange,
}

struct TrackedToolCall<M> {
    id: String,
    function_name: String,
    arguments: String,
    has_finished: bool,
    metadata: Option<M>,
}

/// Extract provider metadata from a delta (the TS `extractMetadata` option).
type ExtractMetadataFn<M> = Box<dyn Fn(&StreamingToolCallDelta) -> Option<M>>;
/// Build the `providerMetadata` for a finalized tool call (the TS
/// `buildToolCallProviderMetadata` option).
type BuildMetadataFn<M> = Box<dyn Fn(Option<&M>) -> Option<M>>;

/// Tracks streaming tool call state across multiple deltas from an
/// OpenAI-compatible chat completion stream.
///
/// Emitted [`ToolCallStreamPart`]s accumulate in an internal buffer; inspect
/// them with [`parts`](Self::parts) and reset between checks with
/// [`clear_parts`](Self::clear_parts) (the TS test's `parts.length = 0`).
///
/// The type parameter `M` is the provider-metadata type. Use `()` (the
/// default) when no metadata handling is needed, or `serde_json::Value` (or
/// any `SharedV4ProviderMetadata`-like type) together with
/// [`with_extract_metadata`](Self::with_extract_metadata) /
/// [`with_build_provider_metadata`](Self::with_build_provider_metadata).
pub struct StreamingToolCallTracker<M = ()> {
    tool_calls: Vec<Option<TrackedToolCall<M>>>,
    parts: Vec<ToolCallStreamPart<M>>,
    // The TS `generateId` option: fallback for `toolCall.id ?? generateId()`
    // when an incoming tool-call delta omits its id.
    generate_id: Box<dyn Fn() -> String>,
    type_validation: TypeValidation,
    /// Upper bound accepted for a tool call `index`. Guards against a remote
    /// index resizing `tool_calls` to a huge vector. Defaults to
    /// [`DEFAULT_MAX_INDEX`] (1024).
    max_index: usize,
    extract_metadata: Option<ExtractMetadataFn<M>>,
    build_provider_metadata: Option<BuildMetadataFn<M>>,
}

impl Default for StreamingToolCallTracker<()> {
    fn default() -> Self {
        Self::new()
    }
}

impl<M> StreamingToolCallTracker<M> {
    /// Create a new tracker with no metadata handling and default settings.
    #[must_use]
    pub fn new() -> Self {
        Self {
            tool_calls: Vec::new(),
            parts: Vec::new(),
            generate_id: Box::new(|| "generated".to_string()),
            type_validation: TypeValidation::None,
            max_index: DEFAULT_MAX_INDEX,
            extract_metadata: None,
            build_provider_metadata: None,
        }
    }

    /// Set a custom id generator (the TS `generateId` option).
    #[must_use]
    pub fn with_generate_id<F: Fn() -> String + 'static>(mut self, f: F) -> Self {
        self.generate_id = Box::new(f);
        self
    }

    /// Set the `type` validation mode (the TS `typeValidation` option).
    #[must_use]
    pub fn with_type_validation(mut self, v: TypeValidation) -> Self {
        self.type_validation = v;
        self
    }

    /// Set the maximum accepted tool call `index` (defaults to 1024). A delta
    /// whose resolved `index` exceeds `max_index` returns
    /// [`TrackerError::IndexOutOfRange`] instead of resizing `tool_calls` to
    /// `index + 1` slots.
    #[must_use]
    pub fn with_max_index(mut self, max_index: usize) -> Self {
        self.max_index = max_index;
        self
    }

    /// Set the metadata extractor (the TS `extractMetadata` option). Called
    /// once when a new tool call is detected; the returned metadata is stored
    /// on the tool call and passed to the builder at finalization.
    #[must_use]
    pub fn with_extract_metadata<F: Fn(&StreamingToolCallDelta) -> Option<M> + 'static>(
        mut self,
        f: F,
    ) -> Self {
        self.extract_metadata = Some(Box::new(f));
        self
    }

    /// Set the provider-metadata builder (the TS `buildToolCallProviderMetadata`
    /// option). Receives the metadata previously extracted; if `None` is
    /// returned, no `provider_metadata` is included in the `tool-call` event.
    #[must_use]
    pub fn with_build_provider_metadata<F: Fn(Option<&M>) -> Option<M> + 'static>(
        mut self,
        f: F,
    ) -> Self {
        self.build_provider_metadata = Some(Box::new(f));
        self
    }

    /// Process a tool call delta from a streaming response chunk. Emits events
    /// into the internal buffer.
    ///
    /// # Errors
    ///
    /// Returns `TrackerError::IndexOutOfRange` when the delta's tool index
    /// exceeds the configured maximum.
    pub fn process_delta(&mut self, delta: &StreamingToolCallDelta) -> Result<(), TrackerError> {
        let index = delta.index.unwrap_or(self.tool_calls.len());
        // Guard against a remote `index` resizing `tool_calls` to a huge vector.
        if index > self.max_index {
            return Err(TrackerError::IndexOutOfRange);
        }
        let is_new = self
            .tool_calls
            .get(index)
            .map(std::option::Option::is_none)
            .unwrap_or(true);
        if is_new {
            self.process_new_tool_call(index, delta)?;
        } else {
            self.process_existing_tool_call(index, delta);
        }
        Ok(())
    }

    /// Finalize any unfinished tool calls. Should be called during the
    /// stream's flush to ensure all tool calls are properly completed. Emits
    /// `tool-input-end` and `tool-call` for each unfinished tool call.
    pub fn flush(&mut self) {
        for i in 0..self.tool_calls.len() {
            let needs_finish = self
                .tool_calls
                .get(i)
                .and_then(|o| o.as_ref())
                .is_some_and(|tc| !tc.has_finished);
            if needs_finish {
                self.finish_tool_call_at(i);
            }
        }
    }

    /// The events emitted so far (accumulated across `process_delta`/`flush`).
    #[must_use]
    pub fn parts(&self) -> &[ToolCallStreamPart<M>] {
        &self.parts
    }

    /// Clear the accumulated events (the TS test's `parts.length = 0`).
    pub fn clear_parts(&mut self) {
        self.parts.clear();
    }

    fn process_new_tool_call(
        &mut self,
        index: usize,
        delta: &StreamingToolCallDelta,
    ) -> Result<(), TrackerError> {
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

        let id = delta.id.clone().unwrap_or_else(|| (self.generate_id)());
        let function_name = delta
            .function
            .as_ref()
            .and_then(|f| f.name.clone())
            .ok_or(TrackerError::MissingFunctionName)?;

        self.parts.push(ToolCallStreamPart::ToolInputStart {
            id: id.clone(),
            tool_name: function_name.clone(),
        });

        let metadata = self
            .extract_metadata
            .as_ref()
            .and_then(|extract| extract(delta));

        // TS: `toolCallDelta.function.arguments ?? ''`.
        let arguments = delta
            .function
            .as_ref()
            .and_then(|f| f.arguments.clone())
            .unwrap_or_default();

        if index >= self.tool_calls.len() {
            self.tool_calls.resize_with(index + 1, || None);
        }
        self.tool_calls[index] = Some(TrackedToolCall {
            id: id.clone(),
            function_name: function_name.clone(),
            arguments: arguments.clone(),
            has_finished: false,
            metadata,
        });

        // Emit initial delta if arguments already present.
        if !arguments.is_empty() {
            self.parts.push(ToolCallStreamPart::ToolInputDelta {
                id: id.clone(),
                delta: arguments,
            });
        }

        // Tool calls must not finalize before the stream ends (see #13137).
        Ok(())
    }

    fn process_existing_tool_call(&mut self, index: usize, delta: &StreamingToolCallDelta) {
        // TS: `toolCallDelta.function?.arguments != null`.
        let new_args = match delta.function.as_ref().and_then(|f| f.arguments.as_ref()) {
            Some(args) => args.clone(),
            None => return,
        };
        let id = {
            let Some(Some(tool_call)) = self.tool_calls.get_mut(index) else {
                return;
            };
            if tool_call.has_finished {
                return;
            }
            tool_call.arguments.push_str(&new_args);
            tool_call.id.clone()
        };
        self.parts.push(ToolCallStreamPart::ToolInputDelta {
            id,
            delta: new_args,
        });
    }

    fn finish_tool_call_at(&mut self, index: usize) {
        // Scope the mutable borrow of the tracked tool call, extracting the
        // owned data we need to emit the final events.
        let (id, function_name, arguments, metadata) = {
            let Some(Some(tool_call)) = self.tool_calls.get_mut(index) else {
                return;
            };
            if tool_call.has_finished {
                return;
            }
            (
                tool_call.id.clone(),
                tool_call.function_name.clone(),
                tool_call.arguments.clone(),
                tool_call.metadata.take(),
            )
        };
        if let Some(Some(tool_call)) = self.tool_calls.get_mut(index) {
            tool_call.has_finished = true;
        }

        self.parts
            .push(ToolCallStreamPart::ToolInputEnd { id: id.clone() });

        let provider_metadata = self
            .build_provider_metadata
            .as_ref()
            .and_then(|build| build(metadata.as_ref()));

        self.parts.push(ToolCallStreamPart::ToolCall {
            tool_call_id: id,
            tool_name: function_name,
            input: arguments,
            provider_metadata,
        });
    }
}
