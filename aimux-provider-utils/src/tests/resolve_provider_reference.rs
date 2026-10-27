//! Rust translation of `packages/provider-utils/src/resolve-provider-reference.test.ts`
//! (@ai-sdk/provider-utils).
//!
//! aimux has no `NoSuchProviderReferenceError`; the missing-provider cases
//! check the `InvalidArgument` error, whose message carries the provider and
//! the reference's providers that the upstream error has as fields.

use aimux_core::error::AiMuxError;
use aimux_core::shared::SharedProviderReference;

use crate::resolve_provider_reference;

fn reference(entries: &[(&str, &str)]) -> SharedProviderReference {
    entries
        .iter()
        .map(|(provider, id)| ((*provider).to_owned(), (*id).to_owned()))
        .collect()
}

fn missing(reference: &SharedProviderReference, provider: &str) -> String {
    match resolve_provider_reference(reference, provider) {
        Err(AiMuxError::InvalidArgument { message, .. }) => message,
        other => panic!("expected InvalidArgument, got {other:?}"),
    }
}

/// upstream: @ai-sdk/provider-utils src/resolve-provider-reference.test.ts › resolveProviderReference › should return the provider-specific identifier when the provider key exists
#[test]
fn should_return_the_provider_specific_identifier_when_the_provider_key_exists() {
    let result = resolve_provider_reference(
        &reference(&[("openai", "file-abc"), ("anthropic", "file-xyz")]),
        "openai",
    );
    assert_eq!(result.unwrap(), "file-abc");
}

/// upstream: @ai-sdk/provider-utils src/resolve-provider-reference.test.ts › resolveProviderReference › should return the correct identifier for a different provider
#[test]
fn should_return_the_correct_identifier_for_a_different_provider() {
    let result = resolve_provider_reference(
        &reference(&[("openai", "file-abc"), ("anthropic", "file-xyz")]),
        "anthropic",
    );
    assert_eq!(result.unwrap(), "file-xyz");
}

/// upstream: @ai-sdk/provider-utils src/resolve-provider-reference.test.ts › resolveProviderReference › should throw NoSuchProviderReferenceError when no entry exists for the given provider
#[test]
fn should_throw_no_such_provider_reference_error_when_no_entry_exists_for_the_given_provider() {
    let message = missing(
        &reference(&[("anthropic", "file-xyz"), ("google", "file-123")]),
        "openai",
    );
    assert_eq!(
        message,
        "No provider reference found for provider 'openai'. Available providers: anthropic, google"
    );
}

/// upstream: @ai-sdk/provider-utils src/resolve-provider-reference.test.ts › resolveProviderReference › should throw NoSuchProviderReferenceError when reference is empty
#[test]
fn should_throw_no_such_provider_reference_error_when_reference_is_empty() {
    let message = missing(&reference(&[]), "openai");
    assert_eq!(
        message,
        "No provider reference found for provider 'openai'. Available providers: "
    );
}

/// upstream: @ai-sdk/provider-utils src/resolve-provider-reference.test.ts › resolveProviderReference › should work with a single-provider reference
#[test]
fn should_work_with_a_single_provider_reference() {
    let result = resolve_provider_reference(&reference(&[("openai", "file-only")]), "openai");
    assert_eq!(result.unwrap(), "file-only");
}
