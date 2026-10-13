//! The `@ai-sdk/provider-utils` version whose user-agent suffix
//! (`ai-sdk-provider-utils/{version}`) every request carries. It must equal
//! its pin in `fixtures/aisdk/VERSIONS.json`; the test below fails on drift.
//! The provider package versions live in `aimux-providers`'
//! `shared/aisdk_pins.rs`.

pub(crate) const PROVIDER_UTILS: &str = "5.0.53";

#[cfg(test)]
#[path = "../../fixtures/aisdk/pinned_versions.rs"]
mod pinned_versions;

#[cfg(test)]
mod tests {
    use super::pinned_versions;

    #[test]
    fn provider_utils_version_is_pinned() {
        assert_eq!(
            pinned_versions::pinned("@ai-sdk/provider-utils"),
            super::PROVIDER_UTILS
        );
    }
}
