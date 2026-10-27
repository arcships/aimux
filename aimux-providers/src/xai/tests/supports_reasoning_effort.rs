//! Upstream `supports-reasoning-effort.test.ts`.

use crate::xai::supports_reasoning_effort::supports_reasoning_effort;

#[allow(
    clippy::module_inception,
    reason = "the upstream describe level keeps its name"
)]
mod supports_reasoning_effort {
    use super::*;

    /// upstream: @ai-sdk/xai src/supports-reasoning-effort.test.ts › supportsReasoningEffort › should return true for %s
    #[test]
    fn should_return_true_for() {
        for model_id in [
            "grok-4.3",
            "grok-latest",
            "grok-4.20-multi-agent",
            "grok-4.20-multi-agent-0309",
            "grok-3-mini",
        ] {
            assert!(
                supports_reasoning_effort(model_id),
                "expected {model_id} to support reasoning effort"
            );
        }
    }

    /// upstream: @ai-sdk/xai src/supports-reasoning-effort.test.ts › supportsReasoningEffort › should return false for %s
    #[test]
    fn should_return_false_for() {
        for model_id in [
            "grok-4.20-reasoning",
            "grok-4.20-non-reasoning",
            "grok-4.20-0309-reasoning",
            "grok-4.20-0309-non-reasoning",
        ] {
            assert!(
                !supports_reasoning_effort(model_id),
                "expected {model_id} not to support reasoning effort"
            );
        }
    }
}
