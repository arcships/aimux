//! The `@ai-sdk/provider` release whose V4 model specifications aimux-core
//! ports. It must equal its pin in `fixtures/aisdk/VERSIONS.json`.
//!
//! 4.0.22 is the first release with `DecisionModelV4`
//! (`@ai-sdk/provider@4.0.22 src/decision-model/v4/decision-model-v4.ts:6-17`)
//! and `DecisionUnsupportedQuestionTypeError`
//! (`src/errors/decision-unsupported-question-type-error.ts:3`); 4.0.21 still
//! had the pre-rename `EvaluationModelV4`. The decision model follows the
//! pinned release, so a pin below 4.0.22 is drift.

#[path = "../../fixtures/aisdk/pinned_versions.rs"]
mod pinned_versions;

/// The pinned `@ai-sdk/provider` version.
const PROVIDER: &str = "4.0.22";

#[test]
fn provider_version_is_pinned() {
    assert_eq!(pinned_versions::pinned("@ai-sdk/provider"), PROVIDER);
}
