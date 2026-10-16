//! The `tool_calls[i]` deltas of a chat-completions stream.
//!
//! OpenAI-style chat streams carry no tool-call boundaries: a call is a
//! series of `tool_calls[i]` fragments, labelled by `index`, `id` and
//! `function.name`, that providers fill in inconsistently. [`ChatToolCalls`]
//! is the part of a chat stream that puts those fragments back together the
//! way `@ai-sdk/provider-utils@5.0.55`'s `streaming-tool-call-tracker.ts`
//! does, so every chat stream (OpenAI, OpenAI-compatible, DeepSeek, Groq,
//! Mistral) emits the same parts for the same deltas.

use std::sync::Arc;

use aimux_core::error::AiMuxError;
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::RawToolCall;
use aimux_core::types::ProviderMetadata;

use super::tool_call_arguments::{StreamingToolCallArgumentState, starts_with_structured_value};
use super::{Controller, Emit, GenerateId};

/// Id used only when the generator returns a blank string
/// (provider-utils@5.0.55 src/streaming-tool-call-tracker.ts:461).
const FALLBACK_TOOL_CALL_ID: &str = "tool-call";

/// One `tool_calls[i]` entry of a chunk, borrowed from the vendor's wire
/// type. A missing `function` object is both `name` and `arguments` absent.
#[derive(Debug, Default)]
pub(crate) struct ToolCallDelta<'a> {
    pub(crate) index: Option<usize>,
    pub(crate) id: Option<&'a str>,
    pub(crate) name: Option<&'a str>,
    /// `None` is the wire `null` / absent; `Some("")` is an empty fragment.
    pub(crate) arguments: Option<&'a str>,
    /// The delta's `type` breaks the vendor's rule (`'function'` when
    /// present for OpenAI, always `'function'` for Groq). Only checked when
    /// the delta starts a call.
    pub(crate) invalid_type: bool,
    /// Metadata for the `ToolCall` part (the upstream `extractMetadata`);
    /// only read from the delta that starts a call.
    pub(crate) provider_metadata: Option<ProviderMetadata>,
}

/// A call being streamed.
struct ToolCall {
    id: String,
    name: String,
    arguments: String,
    argument_state: StreamingToolCallArgumentState,
    /// Index of the starting delta; orders [`ChatToolCalls::finish`].
    index: Option<usize>,
    /// Every wire id and index seen for this call.
    wire_ids: Vec<String>,
    indices: Vec<usize>,
    provider_metadata: Option<ProviderMetadata>,
}

/// Which call a delta belongs to.
enum Resolution {
    Existing(usize),
    New,
    Ambiguous,
}

/// The tool calls of one chat stream, in arrival order.
///
/// Every delta is emitted as it arrives ([`StreamPart::ToolInputStart`] /
/// [`StreamPart::ToolInputDelta`]); every call is ended only by
/// [`finish`](Self::finish) when the stream flushes, because a parsable
/// argument buffer can still be the prefix of longer arguments (ai-sdk
/// #13137).
///
/// Deltas are matched to calls by wire `id`, `index` and function name, not
/// by `index` alone, so providers that reuse indices, repeat ids, drop ids on
/// continuations or send blank names are handled:
///
/// | ID evidence | index/name evidence | start evidence | resolution |
/// | --- | --- | --- | --- |
/// | known | matching | any | matching call, new call, or ambiguity |
/// | known | conflicting | named | new call |
/// | unseen | matching | structured start | new call |
/// | unseen | matching | named empty arguments | incomplete call, new call, or ambiguity |
/// | unseen | matching | continuation | matching call or ambiguity |
/// | absent | matching | any | matching call, new call, or ambiguity |
/// | absent | absent | named | new call |
/// | absent | absent | unnamed | sole unfinished call, new call, or ambiguity |
///
/// An ambiguous delta is dropped (`hasEmptyArguments` in 5.0.55 keeps
/// parallel calls apart when labels are missing or reused).
pub(crate) struct ChatToolCalls {
    calls: Vec<ToolCall>,
    /// Names a call without a usable wire id. Blank or repeated outputs are
    /// made unique.
    generate_id: GenerateId,
}

/// Ids from [`aimux_provider_utils::generate_id`].
impl Default for ChatToolCalls {
    fn default() -> Self {
        Self::new(Arc::new(aimux_provider_utils::generate_id))
    }
}

impl ChatToolCalls {
    pub(crate) fn new(generate_id: GenerateId) -> Self {
        Self {
            calls: Vec::new(),
            generate_id,
        }
    }

    /// Apply one delta, emitting the parts it produces.
    ///
    /// A delta that starts a call with an invalid `type` or without a
    /// function name is the upstream `InvalidResponseDataError`: the stream
    /// ends with it and `false` is returned.
    #[must_use]
    pub(crate) fn delta(&mut self, delta: ToolCallDelta<'_>, controller: &mut Controller) -> bool {
        let ToolCallDelta {
            index,
            id,
            name: wire_name,
            arguments,
            invalid_type,
            provider_metadata,
        } = delta;
        let wire_id = non_blank(id);
        let name = non_blank(wire_name);
        let explicit_start = name.is_some() && starts_with_structured_value(arguments);
        let empty_arguments = arguments.is_none_or(|arguments| arguments.trim().is_empty());

        let call = match self.resolve(wire_id, index, name, explicit_start, empty_arguments) {
            Resolution::Ambiguous => return true,
            // Blank names cannot start a usable call, but some providers
            // repeat a blank name on continuations; those were matched
            // above, so only an unmatched blank-name delta is ignored.
            Resolution::New if wire_name.is_some() && name.is_none() => return true,
            Resolution::New => match (invalid_type, name) {
                (false, Some(name)) => {
                    let arguments = arguments.unwrap_or("");
                    self.start(
                        wire_id,
                        index,
                        name,
                        arguments,
                        provider_metadata,
                        controller,
                    )
                }
                (true, _) => return invalid(controller, "Expected 'function' type."),
                (false, None) => {
                    return invalid(controller, "Expected 'function.name' to be a string.");
                }
            },
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
                    controller.tool_input_delta(tool_call.id.clone(), arguments.to_string());
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
        true
    }

    /// End every call with `ToolInputEnd` and `ToolCall`, ordered by index
    /// when every call has one, otherwise by arrival.
    pub(crate) fn finish(mut self, controller: &mut Controller) {
        if self.calls.iter().all(|c| c.index.is_some()) {
            self.calls.sort_by_key(|c| c.index); // stable: ties keep arrival order
        }
        for call in self.calls {
            controller.tool_input_end(call.id.clone());
            controller.enqueue(StreamPart::ToolCall(RawToolCall {
                tool_call_id: call.id,
                tool_name: call.name,
                // Raw argument text; Core parses it.
                input: call.arguments,
                provider_executed: None,
                dynamic: None,
                provider_metadata: call.provider_metadata,
            }));
        }
    }

    /// Apply the matching table in the type docs.
    fn resolve(
        &self,
        wire_id: Option<&str>,
        index: Option<usize>,
        name: Option<&str>,
        explicit_start: bool,
        empty_arguments: bool,
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
            // A named structured start with an unseen id is a new call.
            // Empty arguments also occur when a continuation changes its id:
            // keep the incomplete matching call then, and start a new call
            // only once its structured arguments are complete.
            if explicit_start {
                return Resolution::New;
            }
            return self.matching(
                |c| at_index(c) && named(c),
                name.is_some() && empty_arguments,
            );
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

    /// Resolve among the calls matching `predicate` (the upstream
    /// `resolveMatchingToolCall`).
    ///
    /// A repeated name can occur on continuations. A fresh structured start
    /// is evidence of another call only once the matching call has completed
    /// its own structured arguments. Without an explicit start, a sole
    /// candidate is the continuation target; otherwise the only candidate
    /// without complete structured arguments is.
    fn matching(&self, predicate: impl Fn(&ToolCall) -> bool, explicit_start: bool) -> Resolution {
        let candidates: Vec<(usize, &ToolCall)> = self
            .calls
            .iter()
            .enumerate()
            .filter(|(_, c)| predicate(c))
            .collect();
        match candidates[..] {
            [] => return Resolution::New,
            [(call, _)] if !explicit_start => return Resolution::Existing(call),
            _ => {}
        }
        let mut continuable = candidates
            .iter()
            .filter(|(_, c)| !c.argument_state.has_complete_structured_value());
        match (continuable.next(), continuable.next()) {
            (Some(&(call, _)), None) => Resolution::Existing(call),
            (None, _) if explicit_start => Resolution::New,
            _ => Resolution::Ambiguous,
        }
    }

    /// Start a call: `ToolInputStart`, then its first argument fragment.
    fn start(
        &mut self,
        wire_id: Option<&str>,
        index: Option<usize>,
        name: &str,
        arguments: &str,
        provider_metadata: Option<ProviderMetadata>,
        controller: &mut Controller,
    ) -> usize {
        let id = self.unique_id(wire_id);
        controller.tool_input_start(id.clone(), name.to_string());
        if !arguments.is_empty() {
            controller.tool_input_delta(id.clone(), arguments.to_string());
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
        // `generated` is one of the `n` ids in use, so at most `n - 1` of
        // `generated-1 ..= generated-n` are: one of them is free (the
        // pigeonhole bound of streaming-tool-call-tracker.ts:468-485). Ids are
        // never released, so the first free suffix is what upstream's
        // resume-from-the-last-suffix search returns.
        let n = self.calls.len();
        let suffix = (1..=n)
            .find(|suffix| !used(&format!("{generated}-{suffix}")))
            .unwrap_or(n + 1);
        format!("{generated}-{suffix}")
    }
}

/// End the stream with the upstream `InvalidResponseDataError`.
fn invalid(controller: &mut Controller, message: &str) -> bool {
    controller.error(AiMuxError::InvalidResponseData(message.to_string()));
    false
}

fn non_blank(value: Option<&str>) -> Option<&str> {
    value.filter(|v| !v.trim().is_empty())
}
