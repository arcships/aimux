//! Shared V4 value types used across all non-chat model traits.
//!
//! Aligned with Vercel AI SDK `SharedV4*` types
//! (`reference/ai/packages/provider/src/shared/v4/`).
//!
//! These types are the common currency passed between the user-facing API and
//! the provider-facing traits (`EmbeddingModel`, `ImageModel`, …). They are
//! deliberately opaque to the core: providers read/write provider-specific
//! keys inside them.

use std::collections::HashMap;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

// Keep the established shared-module paths while the canonical cancellation
// implementation lives in its focused module.
pub use crate::abort_signal::AbortSignal;
pub use crate::types::Warning;

/// Additional HTTP headers sent with a provider request.
///
/// Aligned with V4 `SharedV4Headers` (`Record<string, string>`).
pub type SharedHeaders = HashMap<String, String>;

/// A provider namespace map: provider name to a JSON object of
/// provider-specific keys.
///
/// The AI SDK types these as `Record<string, JSONObject>`, i.e. JS objects,
/// which keep their (non-integer) keys in insertion order. The map keeps that
/// order on both the outer namespace keys and (through serde_json's
/// `preserve_order`) the inner keys, so a value serializes the way it was
/// built or parsed.
pub type NamespaceMap = IndexMap<String, JsonObject>;

/// Additional provider-specific options (inputs), keyed by provider name.
///
/// Aligned with V4 `SharedV4ProviderOptions`
/// (`@ai-sdk/provider` `shared/v4/shared-v4-provider-options.ts:24`).
///
/// ```json
/// { "anthropic": { "cacheControl": { "type": "ephemeral" } } }
/// ```
pub type SharedProviderOptions = NamespaceMap;

/// Additional provider-specific metadata (outputs), keyed by provider name.
///
/// Aligned with V4 `SharedV4ProviderMetadata`
/// (`shared/v4/shared-v4-provider-metadata.ts:24`). The same structure as
/// [`SharedProviderOptions`], as upstream: replay passes
/// `providerOptions: part.providerMetadata` unchanged.
pub type SharedProviderMetadata = NamespaceMap;

/// A JSON object (`JSONObject` in the AI SDK).
pub type JsonObject = serde_json::Map<String, Value>;

/// Build a one-namespace [`SharedProviderMetadata`] / [`SharedProviderOptions`]
/// (they are the same type) from a JSON object literal.
///
/// `provider_namespace("openai", json!({ "itemId": "x" }))?` is
/// `{ "openai": { "itemId": "x" } }`.
///
/// Several namespaces: `extend` one map with another.
///
/// # Errors
///
/// Returns an invalid argument error when the value is not a JSON object.
pub fn provider_namespace(
    namespace: &str,
    object: Value,
) -> Result<SharedProviderMetadata, crate::AiMuxError> {
    let Value::Object(object) = object else {
        return Err(crate::AiMuxError::InvalidArgument(format!(
            "invalid {namespace} provider options: expected a JSON object"
        )));
    };
    Ok(IndexMap::from([(namespace.to_string(), object)]))
}

/// Deep-merge two provider option maps as the AI SDK's `mergeObjects` does
/// (`ai` `util/merge-objects.ts:14-81`): a key of `overrides` replaces the
/// one in `base` unless both are objects, which merge recursively; arrays and
/// other values are replaced; a missing side yields the other. Keys keep the
/// order of `base`, then new keys in the order of `overrides`, like the JS
/// object spread. `__proto__`, `constructor` and `prototype` keys of
/// `overrides` are skipped, as upstream skips them.
pub(crate) fn merge_objects(
    base: Option<&SharedProviderOptions>,
    overrides: Option<&SharedProviderOptions>,
) -> Option<SharedProviderOptions> {
    fn merge(base: &mut JsonObject, overrides: &JsonObject) {
        for (key, value) in overrides {
            if matches!(key.as_str(), "__proto__" | "constructor" | "prototype") {
                continue;
            }
            match (base.get_mut(key), value) {
                (Some(Value::Object(base)), Value::Object(overrides)) => merge(base, overrides),
                _ => {
                    base.insert(key.clone(), value.clone());
                }
            }
        }
    }
    match (base, overrides) {
        (None, None) => None,
        (Some(base), None) => Some(base.clone()),
        (None, Some(overrides)) => Some(overrides.clone()),
        (Some(base), Some(overrides)) => {
            let mut result = base.clone();
            for (namespace, value) in overrides {
                if matches!(
                    namespace.as_str(),
                    "__proto__" | "constructor" | "prototype"
                ) {
                    continue;
                }
                match result.get_mut(namespace) {
                    Some(base) => merge(base, value),
                    None => {
                        result.insert(namespace.clone(), value.clone());
                    }
                }
            }
            Some(result)
        }
    }
}

/// A mapping of provider names to provider-specific file identifiers.
///
/// Allows a logical file to be referenced across providers without
/// re-uploading. Aligned with V4 `SharedV4ProviderReference`
/// (`shared/v4/shared-v4-provider-reference.ts:20`), a JS object, so the
/// entries keep their insertion order.
///
/// ```json
/// { "openai": "file-abc123", "anthropic": "file-xyz789" }
/// ```
pub type SharedProviderReference = IndexMap<String, String>;

/// Either raw bytes or a base64-encoded string.
///
/// Providers should pass file data through without unnecessary conversion: if
/// the upstream API returns base64, return [`FileBytes::Base64`]; if it
/// returns binary, return [`FileBytes::Binary`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ts-export", ts(export))]
pub enum FileBytes {
    /// Raw binary bytes.
    Binary(Vec<u8>),
    /// A base64-encoded string.
    Base64(String),
}

/// File data as a tagged discriminated union.
///
/// Aligned with V4 `SharedV4FileData`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ts-export", ts(export))]
pub enum FileData {
    /// Raw bytes (`Uint8Array`) or a base64-encoded string.
    Data { data: FileBytes },
    /// A URL that points to the file.
    Url {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        original_url: Option<String>,
    },
    /// A provider reference (`{ [provider]: id }`).
    Reference { reference: SharedProviderReference },
    /// Inline text content (e.g. an inline text document).
    Text { text: String },
}

/// Data or a URL returned for a generated file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ts-export", ts(export))]
pub enum GeneratedFileData {
    Data {
        data: FileBytes,
    },
    Url {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        original_url: Option<String>,
    },
}

/// Image/video size in `{width}x{height}` format (e.g. `"1024x1024"`).
///
/// Newtype that enforces the `WxH` format used by V4 image `size` and video
/// `resolution` options. Aligned with the TS template-literal type
/// `` `${number}x${number}` ``.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ts-export", ts(export))]
pub struct Size {
    width: u32,
    height: u32,
}

impl Size {
    /// Create a size from explicit pixel dimensions.
    #[must_use]
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    /// Parse a `{width}x{height}` string (e.g. `"1280x720"`).
    ///
    /// Returns an error if the string is not in the expected format or either
    /// dimension is zero.
    ///
    /// # Errors
    ///
    /// Returns a `String` describing the malformed input when it is not
    /// `{width}x{height}` or either dimension is zero.
    pub fn parse(s: &str) -> Result<Self, String> {
        let (w, h) = parse_pair(s, 'x').ok_or_else(|| {
            format!("invalid size `{s}`: expected format `{{width}}x{{height}}` (e.g. `1024x1024`)")
        })?;
        if w == 0 || h == 0 {
            return Err(format!("invalid size `{s}`: width and height must be > 0"));
        }
        Ok(Self::new(w, h))
    }

    /// Width in pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }
}

impl std::fmt::Display for Size {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}x{}", self.width, self.height)
    }
}

impl std::str::FromStr for Size {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

/// Aspect ratio in `{width}:{height}` format (e.g. `"16:9"`).
///
/// Newtype that enforces the `W:H` format used by V4 image and video
/// `aspectRatio` options. Aligned with the TS template-literal type
/// `` `${number}:${number}` ``.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ts-export", ts(export))]
pub struct AspectRatio {
    width: u32,
    height: u32,
}

impl AspectRatio {
    /// Create an aspect ratio from explicit components.
    #[must_use]
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    /// Parse a `{width}:{height}` string (e.g. `"16:9"`).
    ///
    /// # Errors
    ///
    /// Returns a `String` describing the malformed input when it is not
    /// `{width}:{height}` or either value is zero.
    pub fn parse(s: &str) -> Result<Self, String> {
        let (w, h) = parse_pair(s, ':').ok_or_else(|| {
            format!(
                "invalid aspect ratio `{s}`: expected format `{{width}}:{{height}}` (e.g. `16:9`)"
            )
        })?;
        if w == 0 || h == 0 {
            return Err(format!(
                "invalid aspect ratio `{s}`: width and height must be > 0"
            ));
        }
        Ok(Self::new(w, h))
    }

    /// Width component.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Height component.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }
}

impl std::fmt::Display for AspectRatio {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.width, self.height)
    }
}

impl std::str::FromStr for AspectRatio {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

/// Parse a `{a}{sep}{b}` pair of unsigned integers.
fn parse_pair(s: &str, sep: char) -> Option<(u32, u32)> {
    let (a, b) = s.split_once(sep)?;
    let a: u32 = a.trim().parse().ok()?;
    let b: u32 = b.trim().parse().ok()?;
    Some((a, b))
}

/// Response information for telemetry and debugging.
///
/// Aligned with the `response` object that appears on most V4 model results.
/// All fields are optional because different providers populate different
/// subsets; traits whose TS spec marks `response` as required use
/// `ResponseInfo` directly, while those that mark it optional use
/// `Option<ResponseInfo>`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ts-export", ts(export))]
pub struct ResponseInfo {
    /// Timestamp for the start of the generated response (ISO 8601 string).
    pub timestamp: Option<String>,
    /// The ID of the model that was used to generate the response.
    pub model_id: Option<String>,
    /// Response headers.
    pub headers: Option<SharedHeaders>,
    /// The response body (opaque JSON).
    pub body: Option<Value>,
}

/// Optional request information for telemetry and debugging.
///
/// Aligned with the `request?` object that appears on some V4 model results.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ts-export", ts(export))]
pub struct RequestInfo {
    /// The request body that was sent (opaque JSON).
    pub body: Option<Value>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // The AI SDK's namespace maps are JS objects
    // (`@ai-sdk/provider@4.0.21` `shared-v4-provider-options.ts:24`,
    // `shared-v4-provider-metadata.ts:24`, `shared-v4-provider-reference.ts:20`),
    // so `JSON.stringify` writes their keys in insertion order. Keys here are
    // deliberately out of alphabetical order.

    #[test]
    fn provider_options_keep_insertion_order_through_serde() {
        let mut options = SharedProviderOptions::new();
        options.insert(
            "zeta".to_string(),
            json!({ "z": 1, "a": 2 }).as_object().unwrap().clone(),
        );
        options.insert("alpha".to_string(), JsonObject::new());
        options.insert("mid".to_string(), JsonObject::new());

        let text = serde_json::to_string(&options).unwrap();
        assert_eq!(text, r#"{"zeta":{"z":1,"a":2},"alpha":{},"mid":{}}"#);

        let parsed: SharedProviderMetadata = serde_json::from_str(&text).unwrap();
        assert_eq!(
            parsed.keys().map(String::as_str).collect::<Vec<_>>(),
            ["zeta", "alpha", "mid"]
        );
        assert_eq!(serde_json::to_string(&parsed).unwrap(), text);
    }

    #[test]
    fn provider_reference_keeps_insertion_order_through_serde() {
        let text = r#"{"openai":"file-abc123","anthropic":"file-xyz789","google":"files/1"}"#;
        let reference: SharedProviderReference = serde_json::from_str(text).unwrap();
        assert_eq!(
            reference.keys().map(String::as_str).collect::<Vec<_>>(),
            ["openai", "anthropic", "google"]
        );
        assert_eq!(serde_json::to_string(&reference).unwrap(), text);
    }

    // `mergeObjects` (`ai@7.0.127` `util/merge-objects.ts:14-81`): nested
    // objects merge, other values and arrays are replaced, base keys keep
    // their place and new keys follow in override order.
    #[test]
    fn merge_objects_deep_merges_like_upstream() {
        let options =
            |value: Value| -> SharedProviderOptions { serde_json::from_value(value).unwrap() };
        let base = options(json!({
            "anthropic": { "cacheControl": { "type": "ephemeral", "ttl": "5m" }, "tags": [1, 2] },
            "openai": { "store": true }
        }));
        let overrides = options(json!({
            "zeta": { "x": 1 },
            "anthropic": { "cacheControl": { "ttl": "1h" }, "tags": [3], "__proto__": { "p": 1 } },
            "constructor": { "c": 1 }
        }));
        let merged = merge_objects(Some(&base), Some(&overrides)).unwrap();
        assert_eq!(
            serde_json::to_string(&merged).unwrap(),
            serde_json::to_string(&json!({
                "anthropic": { "cacheControl": { "type": "ephemeral", "ttl": "1h" }, "tags": [3] },
                "openai": { "store": true },
                "zeta": { "x": 1 }
            }))
            .unwrap()
        );
        assert_eq!(merge_objects(None, Some(&base)), Some(base.clone()));
        assert_eq!(merge_objects(Some(&base), None), Some(base));
        assert_eq!(merge_objects(None, None), None);
    }
}
