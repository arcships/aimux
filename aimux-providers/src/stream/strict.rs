//! Strict stream mode (`CallOptions.strict_stream`, #403): the checks that
//! hold for every protocol, made on the parts a vendor stream emits.
//!
//! A vendor stream knows its own terminal event and reports a stream that
//! ended without it ([`StreamContext::fail_without_terminal`]). What is the
//! same for every protocol is checked here, after the vendor transformer:
//!
//! - a delta or end for a text, reasoning or tool-input block that was not
//!   started, and a block started while it is open, fail the stream;
//! - a stream that ends without a `Finish` part fails, unless it reported
//!   an error part of its own;
//! - a client tool call (not provider-executed) is held back until `Finish`
//!   and delivered right before it, or dropped when the stream finishes
//!   with an error or fails.
//!
//! A failure is a terminal `InvalidResponseData` item: nothing follows it.
//! The AI SDK has no such mode; with the option off the vendor stream is
//! returned as it is.
//!
//! [`StreamContext::fail_without_terminal`]: super::StreamContext::fail_without_terminal

use std::collections::HashSet;

use aimux_core::error::AiMuxError;
use aimux_core::options::CallOptions;
use aimux_core::result::StreamResult;
use aimux_core::stream_part::StreamPart;
use aimux_core::types::FinishReasonUnified;
use aimux_provider_utils::{TransformStreamController, Transformer, pipe_through};

/// Wrap `result`'s stream in the strict checks when `options` asks for them.
pub(crate) fn guard(mut result: StreamResult, options: &CallOptions) -> StreamResult {
    if options.strict_stream == Some(true) {
        result.stream = Box::pin(pipe_through(result.stream, StrictParts::default()));
    }
    result
}

/// The error a strict stream fails with.
pub(crate) fn strict_error(problem: impl std::fmt::Display) -> AiMuxError {
    AiMuxError::invalid_response_data(format!("Strict stream: {problem}"))
}

#[derive(Default)]
struct StrictParts {
    text: HashSet<String>,
    reasoning: HashSet<String>,
    tool_input: HashSet<String>,
    /// Client tool calls waiting for `Finish`.
    held: Vec<StreamPart>,
    finished: bool,
    /// The stream already failed (here or upstream).
    failed: bool,
    /// The vendor stream reported an error part: a stream that then ends
    /// without `Finish` already said why.
    errored: bool,
}

/// Which kind of block a part opens, continues or closes.
#[derive(Clone, Copy)]
enum Block {
    Text,
    Reasoning,
    ToolInput,
}

impl Block {
    fn name(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Reasoning => "reasoning",
            Self::ToolInput => "tool-input",
        }
    }
}

enum Step {
    Start,
    Continue(&'static str),
    End,
}

impl StrictParts {
    fn open(&mut self, block: Block) -> &mut HashSet<String> {
        match block {
            Block::Text => &mut self.text,
            Block::Reasoning => &mut self.reasoning,
            Block::ToolInput => &mut self.tool_input,
        }
    }

    fn fail(&mut self, problem: String, controller: &mut TransformStreamController<StreamPart>) {
        self.failed = true;
        self.held.clear();
        controller.error(strict_error(problem));
    }

    /// Track a block part; `false` when it breaks the block's lifecycle.
    fn track(
        &mut self,
        block: Block,
        id: &str,
        step: Step,
        controller: &mut TransformStreamController<StreamPart>,
    ) -> bool {
        let open = self.open(block);
        let problem = match step {
            Step::Start if !open.insert(id.to_owned()) => {
                Some(format!("{} block `{id}` started twice", block.name()))
            }
            Step::Continue(kind) if !open.contains(id) => Some(format!(
                "{kind} for {} block `{id}` that was not started",
                block.name()
            )),
            Step::End if !open.remove(id) => Some(format!(
                "end of {} block `{id}` that was not started",
                block.name()
            )),
            _ => None,
        };
        match problem {
            Some(problem) => {
                self.fail(problem, controller);
                false
            }
            None => true,
        }
    }
}

impl Transformer for StrictParts {
    type Input = Result<StreamPart, AiMuxError>;
    type Output = StreamPart;

    fn transform(
        &mut self,
        chunk: Self::Input,
        controller: &mut TransformStreamController<StreamPart>,
    ) {
        let part = match chunk {
            Ok(part) => part,
            Err(error) => {
                self.failed = true;
                self.held.clear();
                controller.error(error);
                return;
            }
        };
        let tracked = match &part {
            StreamPart::TextStart { id, .. } => Some((Block::Text, id, Step::Start)),
            StreamPart::TextDelta { id, .. } => {
                Some((Block::Text, id, Step::Continue("text-delta")))
            }
            StreamPart::TextEnd { id, .. } => Some((Block::Text, id, Step::End)),
            StreamPart::ReasoningStart { id, .. } => Some((Block::Reasoning, id, Step::Start)),
            StreamPart::ReasoningDelta { id, .. } => {
                Some((Block::Reasoning, id, Step::Continue("reasoning-delta")))
            }
            StreamPart::ReasoningEnd { id, .. } => Some((Block::Reasoning, id, Step::End)),
            StreamPart::ToolInputStart { id, .. } => Some((Block::ToolInput, id, Step::Start)),
            StreamPart::ToolInputDelta { id, .. } => {
                Some((Block::ToolInput, id, Step::Continue("tool-input-delta")))
            }
            StreamPart::ToolInputEnd { id, .. } => Some((Block::ToolInput, id, Step::End)),
            _ => None,
        };
        if let Some((block, id, step)) = tracked {
            let id = id.clone();
            if !self.track(block, &id, step, controller) {
                return;
            }
        }
        match part {
            StreamPart::ToolCall(ref call) if call.provider_executed != Some(true) => {
                self.held.push(part);
            }
            StreamPart::Error { .. } => {
                self.errored = true;
                controller.enqueue(part);
            }
            StreamPart::Finish {
                ref finish_reason, ..
            } => {
                if finish_reason.unified == FinishReasonUnified::Error {
                    self.held.clear();
                } else {
                    for call in self.held.drain(..) {
                        controller.enqueue(call);
                    }
                }
                self.finished = true;
                controller.enqueue(part);
            }
            part => controller.enqueue(part),
        }
    }

    fn flush(mut self, controller: &mut TransformStreamController<StreamPart>) {
        if !self.failed && !self.finished && !self.errored {
            self.fail("the stream ended without a finish part".into(), controller);
        }
    }
}
