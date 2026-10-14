//! The `LanguageModel` trait — the provider-facing interface.
//!
//! Every LLM provider implements this trait. Users never call `do_generate` /
//! `do_stream` directly; they call `generate_text` / `stream_text` instead.

use std::collections::HashMap;

use async_trait::async_trait;
use regex::Regex;

use crate::error::AiMuxError;
use crate::options::CallOptions;
use crate::result::{GenerateResult, StreamResult};

/// URL patterns a language model can fetch itself, keyed by media-type
/// pattern (`"image/*"`, `"application/pdf"`, …). Each value is a list of
/// regular expressions matched against the full URL.
///
/// The empty default means the model fetches nothing itself.
#[derive(Debug, Clone, Default)]
pub struct SupportedUrls(pub HashMap<String, Vec<Regex>>);

impl SupportedUrls {
    /// Whether no media type has a supported URL pattern.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.values().all(Vec::is_empty)
    }

    /// Whether the model fetches `url` itself for a file of `media_type`,
    /// the AI SDK's `isUrlSupported` (provider-utils@5.0.55
    /// `is-url-supported.ts`).
    ///
    /// Media types and the URL are compared in lower case. A `*` or `*/*`
    /// key covers every media type and a `type/*` key every subtype of
    /// `type`. A top-level `media_type` such as `image` matches only an
    /// `image/*` key, since its subtype is unknown. The URL is supported when
    /// one pattern of a matching key finds a match in it.
    #[must_use]
    pub fn is_supported(&self, url: &str, media_type: &str) -> bool {
        let url = url.to_lowercase();
        let media_type = media_type.to_lowercase();
        let top_level_only = !media_type.contains('/');
        self.0
            .iter()
            .filter(|(key, _)| {
                let key = key.to_lowercase();
                let prefix = if key == "*" || key == "*/*" {
                    String::new()
                } else {
                    key.replacen('*', "", 1)
                };
                if prefix.is_empty() {
                    true
                } else if top_level_only {
                    format!("{media_type}/") == prefix
                } else if prefix.ends_with('/') {
                    media_type.starts_with(&prefix)
                } else {
                    media_type == prefix
                }
            })
            .flat_map(|(_, patterns)| patterns)
            .any(|pattern| pattern.is_match(&url))
    }
}

/// The unified language model trait (provider-facing).
///
/// Aligned with Vercel AI SDK `LanguageModelV4`.
///
/// # Implementation notes
///
/// - `do_generate` / `do_stream` are **not** user-facing API. Users call
///   `generate_text` / `stream_text` (free functions in [`crate::generate`]).
/// - Providers must emit `StreamStart` as the first stream part, and `Finish`
///   as the last (before any `Error`).
/// - Unsupported `CallOptions` fields must produce a `Warning::Unsupported`
///   rather than being silently dropped.
#[async_trait]
pub trait LanguageModel: Send + Sync {
    /// Specification version (always `"v4"`).
    fn specification_version(&self) -> &'static str {
        "v4"
    }

    /// Provider name, e.g. `"openai"`.
    fn provider(&self) -> &str;

    /// Model identifier, e.g. `"gpt-4o"`.
    fn model_id(&self) -> &str;

    /// Generate a complete (non-streaming) response.
    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError>;

    /// Generate a streaming response.
    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError>;

    /// URLs the model can fetch on its own, keyed by media-type pattern
    /// (e.g. `"image/*"`), AI SDK `supportedUrls`. Async, as the AI SDK's
    /// value may be a `PromiseLike` (provider@4.0.21
    /// `language-model-v4.ts:36-38`).
    ///
    /// Default: empty, meaning every file part is sent as inline data.
    /// Transparent decorators must forward the inner model's value.
    /// `generate_text` and `stream_text` read it before each model call and
    /// download the file URLs it does not cover
    /// ([`crate::download::DownloadFunction`]).
    async fn supported_urls(&self) -> SupportedUrls {
        SupportedUrls::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn urls(entries: &[(&str, &str)]) -> SupportedUrls {
        let mut map: HashMap<String, Vec<Regex>> = HashMap::new();
        for (media_type, pattern) in entries {
            map.entry((*media_type).to_owned())
                .or_default()
                .push(Regex::new(pattern).unwrap());
        }
        SupportedUrls(map)
    }

    #[test]
    fn a_subtype_wildcard_covers_its_subtypes_and_a_top_level_type() {
        let supported = urls(&[("image/*", r"^https://")]);
        assert!(supported.is_supported("https://a.test/x.png", "image/png"));
        assert!(supported.is_supported("https://a.test/x", "image"));
        assert!(!supported.is_supported("http://a.test/x.png", "image/png"));
        assert!(!supported.is_supported("https://a.test/x.pdf", "application/pdf"));
    }

    #[test]
    fn an_exact_key_needs_the_same_media_type() {
        let supported = urls(&[("application/pdf", r"^https://")]);
        assert!(supported.is_supported("https://a.test/x.pdf", "application/pdf"));
        assert!(!supported.is_supported("https://a.test/x.pdf", "application/pdfx"));
        // A top-level type cannot say which subtype it is.
        assert!(!supported.is_supported("https://a.test/x.pdf", "application"));
    }

    #[test]
    fn a_star_key_covers_every_media_type_and_case_is_ignored() {
        let supported = urls(&[("*", r"^https://cdn\.test/")]);
        assert!(supported.is_supported("HTTPS://CDN.TEST/X", "AUDIO/WAV"));
        assert!(!supported.is_supported("https://other.test/x", "audio/wav"));
        let supported = urls(&[("IMAGE/*", r"^https://")]);
        assert!(supported.is_supported("https://a.test/x", "Image/PNG"));
    }

    #[test]
    fn no_entry_supports_nothing() {
        assert!(!SupportedUrls::default().is_supported("https://a.test/x", "image/png"));
    }
}
