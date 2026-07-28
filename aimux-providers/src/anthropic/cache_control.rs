//! Anthropic `cache_control` validation.
//!
//! Rust port of the TS `get-cache-control.ts` `CacheControlValidator`. The
//! validator is constructed once per prompt conversion and tracks the number of
//! cache breakpoints emitted so far. Anthropic allows a maximum of 4 cache
//! breakpoints per request; breakpoints beyond the limit (and any cache_control
//! set on a non-cacheable context such as a thinking block) are dropped with a
//! `Warning::Unsupported`.

use aimux_core::types::Warning;
use serde_json::Value;

/// Anthropic allows a maximum of 4 cache breakpoints per request.
const MAX_CACHE_BREAKPOINTS: u32 = 4;

/// Extract the `cache_control` value from provider metadata, mirroring the TS
/// `getCacheControl` helper. Accepts both the camelCase `cacheControl` and the
/// snake_case `cache_control` keys under `providerOptions.anthropic`.
///
/// The value is passed through unchanged (the Anthropic API validates it).
pub fn extract_cache_control(provider_options: Option<&Value>) -> Option<Value> {
    let opts = provider_options?;
    let anthropic = opts.get("anthropic")?;
    anthropic
        .get("cacheControl")
        .or_else(|| anthropic.get("cache_control"))
        .cloned()
}

/// Tracks cache breakpoint usage across a single prompt conversion and emits
/// warnings when cache_control is set in an unsupported context or when the
/// breakpoint limit is exceeded.
#[derive(Debug, Default)]
pub struct CacheControlValidator {
    breakpoint_count: u32,
    warnings: Vec<Warning>,
}

impl CacheControlValidator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolve the `cache_control` value (if any) for a content block, applying
    /// the validation rules.
    ///
    /// - `provider_options` is the part- or message-level provider metadata to
    ///   read `anthropic.cacheControl` from.
    /// - `context_type` is a human-readable label for the block kind (e.g.
    ///   `"user message part"`, `"thinking block"`) used in warning messages.
    /// - `can_cache` is `false` for contexts that cannot carry cache_control
    ///   (thinking / redacted-thinking blocks); the value is dropped with a
    ///   warning in that case.
    ///
    /// Returns `Some(value)` when a cache_control breakpoint should be emitted,
    /// or `None` when there is none or it was rejected.
    pub fn get_cache_control(
        &mut self,
        provider_options: Option<&Value>,
        context_type: &str,
        can_cache: bool,
    ) -> Option<Value> {
        let cache_control_value = extract_cache_control(provider_options);

        let cc = cache_control_value?;

        if !can_cache {
            self.warnings.push(Warning::Unsupported {
                feature: "cache_control on non-cacheable context".to_string(),
                details: Some(format!(
                    "cache_control cannot be set on {}. It will be ignored.",
                    context_type
                )),
            });
            return None;
        }

        self.breakpoint_count += 1;
        if self.breakpoint_count > MAX_CACHE_BREAKPOINTS {
            self.warnings.push(Warning::Unsupported {
                feature: "cacheControl breakpoint limit".to_string(),
                details: Some(format!(
                    "Maximum {} cache breakpoints exceeded (found {}). This breakpoint will be ignored.",
                    MAX_CACHE_BREAKPOINTS, self.breakpoint_count
                )),
            });
            return None;
        }

        Some(cc)
    }

    /// Drain the accumulated warnings, leaving the validator's list empty.
    pub fn take_warnings(&mut self) -> Vec<Warning> {
        std::mem::take(&mut self.warnings)
    }
}
