//! `mapReasoningToProviderEffort` and `mapReasoningToProviderBudget` from
//! `@ai-sdk/provider-utils` (`map-reasoning-to-provider.ts`).
//!
//! `isCustomReasoning` is `ReasoningEffort::is_custom` in `aimux-core`.

use aimux_core::types::{ReasoningEffort, Warning};

/// The AI SDK's `DEFAULT_REASONING_BUDGET_PERCENTAGES`: the share of the max
/// output tokens each reasoning level gets as a budget.
pub const DEFAULT_REASONING_BUDGET_PERCENTAGES: &[(ReasoningEffort, f64)] = &[
    (ReasoningEffort::Minimal, 0.02),
    (ReasoningEffort::Low, 0.1),
    (ReasoningEffort::Medium, 0.3),
    (ReasoningEffort::High, 0.6),
    (ReasoningEffort::Xhigh, 0.9),
];

/// The AI SDK's default `minReasoningBudget`.
pub const DEFAULT_MIN_REASONING_BUDGET: u32 = 1024;

fn lookup<T: Copy>(map: &[(ReasoningEffort, T)], reasoning: ReasoningEffort) -> Option<T> {
    map.iter()
        .find(|(level, _)| *level == reasoning)
        .map(|(_, value)| *value)
}

fn unsupported(reasoning: ReasoningEffort) -> Warning {
    Warning::Unsupported {
        feature: "reasoning".to_owned(),
        details: Some(format!(
            "reasoning \"{reasoning}\" is not supported by this model."
        )),
    }
}

/// Maps a reasoning level to a provider-specific effort string using
/// `effort_map`. Pushes a compatibility warning when the level maps to a
/// different string, or an unsupported warning when the level is not in the
/// map.
///
/// Returns the mapped effort string, or `None` when the level is not
/// supported.
pub fn map_reasoning_to_provider_effort<'a>(
    reasoning: ReasoningEffort,
    effort_map: &[(ReasoningEffort, &'a str)],
    warnings: &mut Vec<Warning>,
) -> Option<&'a str> {
    let Some(mapped) = lookup(effort_map, reasoning) else {
        warnings.push(unsupported(reasoning));
        return None;
    };

    if mapped != reasoning.to_string() {
        warnings.push(Warning::Compatibility {
            feature: "reasoning".to_owned(),
            details: Some(format!(
                "reasoning \"{reasoning}\" is not directly supported by this model. mapped to effort \"{mapped}\"."
            )),
        });
    }

    Some(mapped)
}

/// Maps a reasoning level to an absolute token budget: `max_output_tokens`
/// times the level's share in `budget_percentages`, rounded and clamped
/// between `min_reasoning_budget` and `max_reasoning_budget`. Pushes an
/// unsupported warning when the level is not in `budget_percentages`.
///
/// The AI SDK defaults `min_reasoning_budget` to
/// [`DEFAULT_MIN_REASONING_BUDGET`] and `budget_percentages` to
/// [`DEFAULT_REASONING_BUDGET_PERCENTAGES`].
///
/// Returns the budget, or `None` when the level is not supported.
pub fn map_reasoning_to_provider_budget(
    reasoning: ReasoningEffort,
    max_output_tokens: u32,
    max_reasoning_budget: u32,
    min_reasoning_budget: u32,
    budget_percentages: &[(ReasoningEffort, f64)],
    warnings: &mut Vec<Warning>,
) -> Option<u32> {
    let Some(percentage) = lookup(budget_percentages, reasoning) else {
        warnings.push(unsupported(reasoning));
        return None;
    };

    let budget = (f64::from(max_output_tokens) * percentage).round() as u32;
    Some(max_reasoning_budget.min(min_reasoning_budget.max(budget)))
}
