//! Rust translation of `packages/provider-utils/src/map-reasoning-to-provider.test.ts`
//! (@ai-sdk/provider-utils).
//!
//! aimux's `ReasoningEffort` has no `max` level yet, so the upstream `max`
//! entries and the `max` budget case are not ported. `isCustomReasoning` is
//! `ReasoningEffort::is_custom` in `aimux-core`, over an optional level.

use aimux_core::types::{ReasoningEffort, Warning};

use crate::{
    DEFAULT_MIN_REASONING_BUDGET, DEFAULT_REASONING_BUDGET_PERCENTAGES,
    map_reasoning_to_provider_budget, map_reasoning_to_provider_effort,
};

const EFFORT_MAP: &[(ReasoningEffort, &str)] = &[
    (ReasoningEffort::Minimal, "low"),
    (ReasoningEffort::Low, "low"),
    (ReasoningEffort::Medium, "medium"),
    (ReasoningEffort::High, "high"),
    (ReasoningEffort::Xhigh, "max"),
];

fn is_custom_reasoning(reasoning: Option<ReasoningEffort>) -> bool {
    reasoning.is_some_and(ReasoningEffort::is_custom)
}

mod map_reasoning_to_provider_effort {
    use super::*;

    /// upstream: @ai-sdk/provider-utils src/map-reasoning-to-provider.test.ts › mapReasoningToProviderEffort › returns mapped value with no warning for direct match
    #[test]
    fn returns_mapped_value_with_no_warning_for_direct_match() {
        let mut warnings = Vec::new();
        let result =
            map_reasoning_to_provider_effort(ReasoningEffort::Medium, EFFORT_MAP, &mut warnings);
        assert_eq!(result, Some("medium"));
        assert_eq!(warnings, Vec::<Warning>::new());
    }

    /// upstream: @ai-sdk/provider-utils src/map-reasoning-to-provider.test.ts › mapReasoningToProviderEffort › returns mapped value with compatibility warning for renamed match
    #[test]
    fn returns_mapped_value_with_compatibility_warning_for_renamed_match() {
        let mut warnings = Vec::new();
        let result =
            map_reasoning_to_provider_effort(ReasoningEffort::Minimal, EFFORT_MAP, &mut warnings);
        assert_eq!(result, Some("low"));
        assert_eq!(
            warnings,
            vec![Warning::Compatibility {
                feature: "reasoning".into(),
                details: Some(
                    "reasoning \"minimal\" is not directly supported by this model. mapped to effort \"low\"."
                        .into()
                ),
            }]
        );
    }

    /// upstream: @ai-sdk/provider-utils src/map-reasoning-to-provider.test.ts › mapReasoningToProviderEffort › returns mapped value with compatibility warning for "xhigh"
    #[test]
    fn returns_mapped_value_with_compatibility_warning_for_xhigh() {
        let mut warnings = Vec::new();
        let result =
            map_reasoning_to_provider_effort(ReasoningEffort::Xhigh, EFFORT_MAP, &mut warnings);
        assert_eq!(result, Some("max"));
        assert_eq!(
            warnings,
            vec![Warning::Compatibility {
                feature: "reasoning".into(),
                details: Some(
                    "reasoning \"xhigh\" is not directly supported by this model. mapped to effort \"max\"."
                        .into()
                ),
            }]
        );
    }

    /// upstream: @ai-sdk/provider-utils src/map-reasoning-to-provider.test.ts › mapReasoningToProviderEffort › returns undefined with unsupported warning for key missing from effortMap
    #[test]
    fn returns_undefined_with_unsupported_warning_for_key_missing_from_effort_map() {
        let mut warnings = Vec::new();
        let partial_effort_map = [(ReasoningEffort::Medium, "medium")];
        let result = map_reasoning_to_provider_effort(
            ReasoningEffort::High,
            &partial_effort_map,
            &mut warnings,
        );
        assert_eq!(result, None);
        assert_eq!(
            warnings,
            vec![Warning::Unsupported {
                feature: "reasoning".into(),
                details: Some("reasoning \"high\" is not supported by this model.".into()),
            }]
        );
    }
}

mod is_custom_reasoning {
    use super::*;

    /// upstream: @ai-sdk/provider-utils src/map-reasoning-to-provider.test.ts › isCustomReasoning › returns false for undefined
    #[test]
    fn returns_false_for_undefined() {
        assert!(!is_custom_reasoning(None));
    }

    /// upstream: @ai-sdk/provider-utils src/map-reasoning-to-provider.test.ts › isCustomReasoning › returns false for provider-default
    #[test]
    fn returns_false_for_provider_default() {
        assert!(!is_custom_reasoning(Some(ReasoningEffort::ProviderDefault)));
    }

    /// upstream: @ai-sdk/provider-utils src/map-reasoning-to-provider.test.ts › isCustomReasoning › returns true for none
    #[test]
    fn returns_true_for_none() {
        assert!(is_custom_reasoning(Some(ReasoningEffort::None)));
    }

    /// upstream: @ai-sdk/provider-utils src/map-reasoning-to-provider.test.ts › isCustomReasoning › returns true for all reasoning levels
    /// partial: `max` is not an aimux reasoning level yet.
    #[test]
    fn returns_true_for_all_reasoning_levels() {
        for value in [
            ReasoningEffort::Minimal,
            ReasoningEffort::Low,
            ReasoningEffort::Medium,
            ReasoningEffort::High,
            ReasoningEffort::Xhigh,
        ] {
            assert!(is_custom_reasoning(Some(value)));
        }
    }
}

mod map_reasoning_to_provider_budget {
    use super::*;

    fn budget(
        reasoning: ReasoningEffort,
        max_output_tokens: u32,
        max_reasoning_budget: u32,
        warnings: &mut Vec<Warning>,
    ) -> Option<u32> {
        map_reasoning_to_provider_budget(
            reasoning,
            max_output_tokens,
            max_reasoning_budget,
            DEFAULT_MIN_REASONING_BUDGET,
            DEFAULT_REASONING_BUDGET_PERCENTAGES,
            warnings,
        )
    }

    /// upstream: @ai-sdk/provider-utils src/map-reasoning-to-provider.test.ts › mapReasoningToProviderBudget › returns correct budget for known key
    #[test]
    fn returns_correct_budget_for_known_key() {
        let mut warnings = Vec::new();
        let result = budget(ReasoningEffort::Medium, 64000, 64000, &mut warnings);
        assert_eq!(result, Some(19200));
        assert_eq!(warnings, Vec::<Warning>::new());
    }

    /// upstream: @ai-sdk/provider-utils src/map-reasoning-to-provider.test.ts › mapReasoningToProviderBudget › caps result at maxReasoningBudget
    #[test]
    fn caps_result_at_max_reasoning_budget() {
        let mut warnings = Vec::new();
        let result = budget(ReasoningEffort::Xhigh, 64000, 50000, &mut warnings);
        assert_eq!(result, Some(50000));
        assert_eq!(warnings, Vec::<Warning>::new());
    }

    /// upstream: @ai-sdk/provider-utils src/map-reasoning-to-provider.test.ts › mapReasoningToProviderBudget › floors result at default minReasoningBudget of 1024
    #[test]
    fn floors_result_at_default_min_reasoning_budget_of_1024() {
        let mut warnings = Vec::new();
        let result = budget(ReasoningEffort::Minimal, 10000, 10000, &mut warnings);
        assert_eq!(result, Some(1024));
        assert_eq!(warnings, Vec::<Warning>::new());
    }

    /// upstream: @ai-sdk/provider-utils src/map-reasoning-to-provider.test.ts › mapReasoningToProviderBudget › respects custom minReasoningBudget
    #[test]
    fn respects_custom_min_reasoning_budget() {
        let mut warnings = Vec::new();
        let result = map_reasoning_to_provider_budget(
            ReasoningEffort::Minimal,
            10000,
            10000,
            512,
            DEFAULT_REASONING_BUDGET_PERCENTAGES,
            &mut warnings,
        );
        assert_eq!(result, Some(512));
        assert_eq!(warnings, Vec::<Warning>::new());
    }

    /// upstream: @ai-sdk/provider-utils src/map-reasoning-to-provider.test.ts › mapReasoningToProviderBudget › respects custom budgetPercentages
    #[test]
    fn respects_custom_budget_percentages() {
        let mut warnings = Vec::new();
        let result = map_reasoning_to_provider_budget(
            ReasoningEffort::Medium,
            10000,
            10000,
            DEFAULT_MIN_REASONING_BUDGET,
            &[(ReasoningEffort::Medium, 0.5)],
            &mut warnings,
        );
        assert_eq!(result, Some(5000));
        assert_eq!(warnings, Vec::<Warning>::new());
    }

    /// upstream: @ai-sdk/provider-utils src/map-reasoning-to-provider.test.ts › mapReasoningToProviderBudget › returns undefined with unsupported warning for key missing from custom budgetPercentages
    #[test]
    fn returns_undefined_with_unsupported_warning_for_key_missing_from_custom_budget_percentages() {
        let mut warnings = Vec::new();
        let result = map_reasoning_to_provider_budget(
            ReasoningEffort::High,
            64000,
            64000,
            DEFAULT_MIN_REASONING_BUDGET,
            &[(ReasoningEffort::Medium, 0.5)],
            &mut warnings,
        );
        assert_eq!(result, None);
        assert_eq!(
            warnings,
            vec![Warning::Unsupported {
                feature: "reasoning".into(),
                details: Some("reasoning \"high\" is not supported by this model.".into()),
            }]
        );
    }
}
