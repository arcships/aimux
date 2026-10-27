//! The provider's identifier in a provider reference
//! (`resolve-provider-reference.ts`).

use aimux_core::error::AiMuxError;
use aimux_core::shared::SharedProviderReference;

/// The identifier the reference maps `provider` to
/// (`resolveProviderReference`).
///
/// # Errors
///
/// Returns `InvalidArgument` with the AI SDK's `NoSuchProviderReferenceError`
/// message, the reference's providers in their order, when the reference has
/// no entry for `provider`.
pub fn resolve_provider_reference(
    reference: &SharedProviderReference,
    provider: &str,
) -> Result<String, AiMuxError> {
    reference.get(provider).cloned().ok_or_else(|| {
        AiMuxError::invalid_argument(format!(
            "No provider reference found for provider '{provider}'. Available providers: {}",
            reference
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        ))
    })
}
