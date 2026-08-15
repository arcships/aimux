//! Composite-model shared infrastructure (RFC-0021 / RFC-0022).
//!
//! A *composite model* implements [`LanguageModel`] by composing one or more
//! child models. The two concrete composite models are:
//!
//! - [`crate::router::RouterModel`] (RFC-0021): route to one child + fallback.
//! - [`crate::moa::MoaModel`] (RFC-0022): fan-out to all references + aggregate.
//!
//! This module holds the pieces both share:
//! - [`ChildModel`] — the child-handle shape (`Arc<dyn LanguageModel>`).
//! - [`add_usage`] — per-field `Usage` accumulation.
//! - [`extract_text`] — pull text out of a `GenerateContent` list.
//! - [`build_aggregator_prompt`] — assemble the MoA aggregator prompt.
//!
//! See [`crate::trace::layer::TraceLayer`] for the established decorator pattern
//! (`inner: Arc<dyn LanguageModel>` implementing `LanguageModel`); composite
//! models follow the same shape.

use std::sync::Arc;

use crate::language_model::LanguageModel;
use crate::language_model_message::{LanguageModelPrompt, LanguageModelPromptMessage};
use crate::result::GenerateContent;
use crate::types::{TokenUsage, Usage};

/// Composite-model child handle. `Arc` (not `Box`) so children are `Clone`,
/// match the FFI/Node registry shape, and stay alive after a child handle is
/// dropped (shared by registry + composite).
pub type ChildModel = Arc<dyn LanguageModel>;

/// Add two `Usage` values field-by-field. `Usage::raw` (provider-opaque) is
/// dropped — summing raw maps across providers is meaningless.
///
/// Each `TokenUsage` field is `Option<u32>`; `None` is treated as zero so a
/// child that doesn't report a breakdown doesn't erase the other child's data.
#[must_use]
pub fn add_usage(a: Usage, b: &Usage) -> Usage {
    Usage {
        input_tokens: add_token_usage(a.input_tokens, &b.input_tokens),
        output_tokens: add_token_usage(a.output_tokens, &b.output_tokens),
        raw: None,
    }
}

fn add_token_usage(a: TokenUsage, b: &TokenUsage) -> TokenUsage {
    TokenUsage {
        total: opt_add(a.total, b.total),
        no_cache: opt_add(a.no_cache, b.no_cache),
        cache_read: opt_add(a.cache_read, b.cache_read),
        cache_write: opt_add(a.cache_write, b.cache_write),
        text: opt_add(a.text, b.text),
        reasoning: opt_add(a.reasoning, b.reasoning),
    }
}

fn opt_add(a: Option<u32>, b: Option<u32>) -> Option<u32> {
    match (a, b) {
        (None, None) => None,
        (Some(x), None) => Some(x),
        (None, Some(y)) => Some(y),
        (Some(x), Some(y)) => Some(x + y),
    }
}

/// Extract concatenated text from a `GenerateContent` list. Only `Text` parts
/// are kept; `Reasoning` / `ToolCall` / `Source` / `File` are dropped (MoA
/// thin version — references contribute analysis text, not tool calls).
#[must_use]
pub fn extract_text(content: &[GenerateContent]) -> String {
    let mut out = String::new();
    for c in content {
        if let GenerateContent::Text { text, .. } = c {
            out.push_str(text);
        }
    }
    out
}

/// Build the aggregator prompt for MoA: the original prompt verbatim, with one
/// extra `Role::User` message appended that carries the aggregation instruction
/// and the reference responses under `## {model_id}` headings.
///
/// `instructions` is the (optional) aggregator system instruction prepended to
/// the user message; when `None`, a default aggregation instruction is used.
/// The reference list may be empty — in that case the aggregator just runs the
/// original prompt (degenerates to a single-model call).
#[must_use]
pub fn build_aggregator_prompt(
    prompt: &LanguageModelPrompt,
    instructions: Option<&str>,
    references: &[(String, String)],
) -> LanguageModelPrompt {
    // Original messages, cloned verbatim.
    let mut out: LanguageModelPrompt = prompt.to_vec();

    // With no references, append nothing — the aggregator just runs the
    // original prompt (degenerate MoA = single model). Injecting an empty
    // "Reference model responses" heading would confuse the aggregator and
    // waste tokens.
    if references.is_empty() {
        return out;
    }

    let instruction = instructions.unwrap_or(
        "You are an aggregation assistant. Synthesize the reference model \
         responses below into a single high-quality answer. Resolve conflicts \
         by preferring the most accurate reasoning; do not mention the \
         reference models unless asked.",
    );

    let mut body = String::new();
    body.push_str(instruction);
    body.push_str("\n\n# Reference model responses\n");
    for (model_id, text) in references {
        body.push_str("\n## ");
        body.push_str(model_id);
        body.push('\n');
        body.push_str(text);
        body.push('\n');
    }

    out.push(LanguageModelPromptMessage {
        role: crate::message::Role::User,
        content: vec![crate::content::ContentPart::text(body)],
        provider_options: None,
    });
    out
}
