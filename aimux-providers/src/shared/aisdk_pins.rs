//! The `@ai-sdk/*` packages whose user-agent each provider sends.
//!
//! A request of a pinned package carries `ai-sdk-{name}/{version}`, added by
//! the package's provider headers, and `ai-sdk-provider-utils/<v>`, added by
//! the provider-utils request helper the package runs on. Upstream, `<v>` is
//! the `@ai-sdk/provider-utils` that the package's own `package.json` depends
//! on, so it differs between packages; [`crate::shared::package_fetch`] names
//! it per package.
//!
//! Each version equals its pin in `fixtures/aisdk/VERSIONS.json`, and each
//! `provider_utils` equals the dependency in that version's `package.json`
//! (read from `scripts/aisdk-fixtures/package-lock.json`); the tests below
//! fail on drift. The `@ai-sdk/provider-utils` entry of `VERSIONS.json` is the
//! version the provider-utils port follows: no user-agent depends on it.
//! These are constants rather than read at build time because the published
//! crate does not ship `fixtures/`.

/// One pinned `@ai-sdk/*` package.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Package {
    /// The package name after `@ai-sdk/`, as in `ai-sdk-{name}/{version}`.
    pub name: &'static str,
    pub version: &'static str,
    /// The exact `@ai-sdk/provider-utils` dependency of this version's
    /// `package.json`: the version its `ai-sdk-provider-utils/<v>` part names.
    pub provider_utils: &'static str,
}

/// `@ai-sdk/anthropic@4.0.71` package.json: `"@ai-sdk/provider-utils": "5.0.53"`.
pub(crate) const ANTHROPIC: Package = Package {
    name: "anthropic",
    version: "4.0.71",
    provider_utils: "5.0.53",
};
/// `@ai-sdk/google@4.0.87` package.json: `"@ai-sdk/provider-utils": "5.0.53"`.
pub(crate) const GOOGLE: Package = Package {
    name: "google",
    version: "4.0.87",
    provider_utils: "5.0.53",
};
/// `@ai-sdk/openai@4.0.83` package.json: `"@ai-sdk/provider-utils": "5.0.53"`.
pub(crate) const OPENAI: Package = Package {
    name: "openai",
    version: "4.0.83",
    provider_utils: "5.0.53",
};
/// `@ai-sdk/openai-compatible@3.0.62` package.json:
/// `"@ai-sdk/provider-utils": "5.0.53"`.
pub(crate) const OPENAI_COMPATIBLE: Package = Package {
    name: "openai-compatible",
    version: "3.0.62",
    provider_utils: "5.0.53",
};
/// `@ai-sdk/amazon-bedrock@5.0.100` package.json:
/// `"@ai-sdk/provider-utils": "5.0.51"`.
pub(crate) const AMAZON_BEDROCK: Package = Package {
    name: "amazon-bedrock",
    version: "5.0.100",
    provider_utils: "5.0.51",
};
/// `@ai-sdk/anthropic-aws@2.0.63` package.json:
/// `"@ai-sdk/provider-utils": "5.0.53"`.
pub(crate) const ANTHROPIC_AWS: Package = Package {
    name: "anthropic-aws",
    version: "2.0.63",
    provider_utils: "5.0.53",
};
/// `@ai-sdk/azure@4.0.84` package.json: `"@ai-sdk/provider-utils": "5.0.51"`.
pub(crate) const AZURE: Package = Package {
    name: "azure",
    version: "4.0.84",
    provider_utils: "5.0.51",
};
/// `@ai-sdk/cohere@4.0.52` package.json: `"@ai-sdk/provider-utils": "5.0.51"`.
pub(crate) const COHERE: Package = Package {
    name: "cohere",
    version: "4.0.52",
    provider_utils: "5.0.51",
};
/// `@ai-sdk/deepseek@3.0.56` package.json: `"@ai-sdk/provider-utils": "5.0.51"`.
pub(crate) const DEEPSEEK: Package = Package {
    name: "deepseek",
    version: "3.0.56",
    provider_utils: "5.0.51",
};
/// `@ai-sdk/google-vertex@5.0.100` package.json:
/// `"@ai-sdk/provider-utils": "5.0.53"`. The nested `@ai-sdk/google@4.0.87`
/// (Gemini, the `GOOGLE` pin) and `@ai-sdk/anthropic@4.0.70` (the Anthropic
/// sub-provider, which sends no package part) depend on the same 5.0.53.
pub(crate) const GOOGLE_VERTEX: Package = Package {
    name: "google-vertex",
    version: "5.0.100",
    provider_utils: "5.0.53",
};
/// The `@ai-sdk/openai-compatible` that `@ai-sdk/google-vertex@5.0.100`
/// package.json pins (`"3.0.61"`) for its MaaS and xAI sub-providers. That
/// version's package.json: `"@ai-sdk/provider-utils": "5.0.53"`.
pub(crate) const GOOGLE_VERTEX_OPENAI_COMPATIBLE: Package = Package {
    name: "openai-compatible",
    version: "3.0.61",
    provider_utils: "5.0.53",
};
/// `@ai-sdk/groq@4.0.52` package.json: `"@ai-sdk/provider-utils": "5.0.51"`.
pub(crate) const GROQ: Package = Package {
    name: "groq",
    version: "4.0.52",
    provider_utils: "5.0.51",
};
/// `@ai-sdk/huggingface@2.0.62` package.json:
/// `"@ai-sdk/provider-utils": "5.0.53"` (and `"@ai-sdk/openai-compatible":
/// "3.0.62"`, the [`OPENAI_COMPATIBLE`] pin). Unlike the other packages it
/// adds no `ai-sdk-huggingface/<v>` part: `createHuggingFace`'s headers are
/// the key and the user headers only (huggingface-provider.ts:68-75).
pub(crate) const HUGGINGFACE: Package = Package {
    name: "huggingface",
    version: "2.0.62",
    provider_utils: "5.0.53",
};
/// `@ai-sdk/mistral@4.0.54` package.json: `"@ai-sdk/provider-utils": "5.0.51"`.
pub(crate) const MISTRAL: Package = Package {
    name: "mistral",
    version: "4.0.54",
    provider_utils: "5.0.51",
};
/// `@ai-sdk/open-responses@2.0.58` package.json:
/// `"@ai-sdk/provider-utils": "5.0.53"`.
pub(crate) const OPEN_RESPONSES: Package = Package {
    name: "open-responses",
    version: "2.0.58",
    provider_utils: "5.0.53",
};
/// `@ai-sdk/xai@5.0.12` package.json: `"@ai-sdk/provider-utils": "5.0.51"`.
pub(crate) const XAI: Package = Package {
    name: "xai",
    version: "5.0.12",
    provider_utils: "5.0.51",
};

#[cfg(test)]
#[path = "../../../fixtures/aisdk/pinned_versions.rs"]
mod pinned_versions;

#[cfg(test)]
mod tests {
    use super::{Package, pinned_versions};

    /// Every top-level package (not the nested google-vertex copy).
    const PACKAGES: [Package; 15] = [
        super::ANTHROPIC,
        super::GOOGLE,
        super::OPENAI,
        super::OPENAI_COMPATIBLE,
        super::AMAZON_BEDROCK,
        super::ANTHROPIC_AWS,
        super::AZURE,
        super::COHERE,
        super::DEEPSEEK,
        super::GOOGLE_VERTEX,
        super::GROQ,
        super::HUGGINGFACE,
        super::MISTRAL,
        super::OPEN_RESPONSES,
        super::XAI,
    ];

    #[test]
    fn versions_equal_the_aisdk_pins() {
        for package in PACKAGES {
            let name = format!("@ai-sdk/{}", package.name);
            assert_eq!(pinned_versions::pinned(&name), package.version, "{name}");
        }
    }

    /// Each package names the provider-utils its own package.json depends
    /// on, whatever VERSIONS.json pins for `@ai-sdk/provider-utils`.
    #[test]
    fn provider_utils_equal_each_package_dependency() {
        for package in PACKAGES {
            let name = format!("@ai-sdk/{}", package.name);
            assert_eq!(
                pinned_versions::pinned_dependency(&name, "@ai-sdk/provider-utils"),
                package.provider_utils,
                "{name}"
            );
        }
    }

    /// google-vertex runs its sub-providers on nested packages: Gemini on
    /// `@ai-sdk/google`, the Anthropic sub-provider on `@ai-sdk/anthropic`,
    /// MaaS and xAI on `@ai-sdk/openai-compatible`.
    #[test]
    fn google_vertex_nested_packages_are_pinned() {
        let vertex = "@ai-sdk/google-vertex";
        let compat = super::GOOGLE_VERTEX_OPENAI_COMPATIBLE;
        assert_eq!(
            pinned_versions::pinned_dependency(vertex, "@ai-sdk/openai-compatible"),
            compat.version
        );
        assert_eq!(
            pinned_versions::nested_dependency(
                vertex,
                "@ai-sdk/openai-compatible",
                "@ai-sdk/provider-utils"
            ),
            compat.provider_utils
        );
        for nested in ["@ai-sdk/google", "@ai-sdk/anthropic"] {
            assert_eq!(
                pinned_versions::nested_dependency(vertex, nested, "@ai-sdk/provider-utils"),
                super::GOOGLE_VERTEX.provider_utils,
                "{nested}"
            );
        }
    }

    /// `@ai-sdk/huggingface` is pinned at the version whose openai-compatible
    /// dependency is the openai-compatible pin.
    #[test]
    fn huggingface_depends_on_the_pinned_openai_compatible() {
        assert_eq!(
            pinned_versions::pinned_dependency("@ai-sdk/huggingface", "@ai-sdk/openai-compatible"),
            super::OPENAI_COMPATIBLE.version
        );
    }

    /// Every VERSIONS.json entry is the `{version, integrity}` object form.
    #[test]
    fn every_versions_entry_is_object_form() {
        // `all()` panics on any other shape.
        assert!(pinned_versions::all().len() >= 18);
    }
}
