//! Shared decision-model support, runtime endpoints, and candidate scoring.

pub(crate) mod runtime;
mod scoring;
pub(crate) mod support;

pub use support::DecisionProbabilitySource;
