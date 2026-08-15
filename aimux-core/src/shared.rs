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

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_util::sync::CancellationToken;
use ts_rs::TS;

// Re-export the existing `Warning` so providers can import everything they
// need from `crate::shared` in one place.
pub use crate::types::Warning;

/// Additional HTTP headers sent with a provider request.
///
/// Aligned with V4 `SharedV4Headers` (`Record<string, string>`).
pub type SharedHeaders = HashMap<String, String>;

/// Additional provider-specific options (inputs), keyed by provider name.
///
/// The inner value is a JSON object whose keys are provider-specific option
/// names. Aligned with V4 `SharedV4ProviderOptions`
/// (`Record<string, JSONObject>`).
///
/// ```json
/// { "anthropic": { "cacheControl": { "type": "ephemeral" } } }
/// ```
pub type SharedProviderOptions = HashMap<String, Value>;

/// Additional provider-specific metadata (outputs), keyed by provider name.
///
/// Aligned with V4 `SharedV4ProviderMetadata` (`Record<string, JSONObject>`).
pub type SharedProviderMetadata = HashMap<String, Value>;

/// A mapping of provider names to provider-specific file identifiers.
///
/// Allows a logical file to be referenced across providers without
/// re-uploading. Aligned with V4 `SharedV4ProviderReference`.
///
/// ```json
/// { "openai": "file-abc123", "anthropic": "file-xyz789" }
/// ```
pub type SharedProviderReference = HashMap<String, String>;

/// Either raw bytes or a base64-encoded string.
///
/// Providers should pass file data through without unnecessary conversion: if
/// the upstream API returns base64, return [`FileBytes::Base64`]; if it
/// returns binary, return [`FileBytes::Binary`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
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
#[ts(export)]
pub enum FileData {
    /// Raw bytes (`Uint8Array`) or a base64-encoded string.
    Data { data: FileBytes },
    /// A URL that points to the file.
    Url { url: String },
    /// A provider reference (`{ [provider]: id }`).
    Reference { reference: SharedProviderReference },
    /// Inline text content (e.g. an inline text document).
    Text { text: String },
}

/// A cancellation signal analogous to the Web `AbortSignal`.
///
/// Event-driven: backed by `tokio_util::sync::CancellationToken` (itself a
/// `tokio::sync::Notify`). Consumers can poll [`is_aborted`](Self::is_aborted)
/// synchronously, or await [`cancelled`](Self::cancelled) for prompt,
/// notification-based wakeup — no polling loop needed.
///
/// This type is `Send + Sync` and cheap to clone.
#[derive(Debug, Clone)]
pub struct AbortSignal {
    token: CancellationToken,
}

impl Default for AbortSignal {
    fn default() -> Self {
        Self::new()
    }
}

impl AbortSignal {
    /// Create a fresh, un-aborted signal.
    #[must_use]
    pub fn new() -> Self {
        Self {
            token: CancellationToken::new(),
        }
    }

    /// Request cancellation. All clones observe the aborted state and any
    /// pending [`cancelled`](Self::cancelled) futures resolve.
    pub fn abort(&self) {
        self.token.cancel();
    }

    /// Returns `true` once [`abort`](Self::abort) has been called.
    #[must_use]
    pub fn is_aborted(&self) -> bool {
        self.token.is_cancelled()
    }

    /// A future that resolves as soon as the signal is aborted.
    ///
    /// If the signal is already aborted, the future resolves immediately on
    /// the first poll. Suitable for `tokio::select!` arms.
    pub fn cancelled(&self) -> impl std::future::Future<Output = ()> + Send + 'static {
        let token = self.token.clone();
        async move { token.cancelled_owned().await }
    }
}

/// Image/video size in `{width}x{height}` format (e.g. `"1024x1024"`).
///
/// Newtype that enforces the `WxH` format used by V4 image `size` and video
/// `resolution` options. Aligned with the TS template-literal type
/// `` `${number}x${number}` ``.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
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
#[ts(export)]
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
#[ts(export)]
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
#[ts(export)]
pub struct RequestInfo {
    /// The request body that was sent (opaque JSON).
    pub body: Option<Value>,
}
