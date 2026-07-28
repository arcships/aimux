//! A simple multipart/form-data body builder.
//!
//! Builds the raw bytes of a `multipart/form-data` body without relying on
//! reqwest's `multipart` feature (which pulls in `mime_guess`, whose build
//! script may be blocked by application control policies on some systems).

/// A simple multipart form-data builder.
///
/// Produces the raw body bytes and the `Content-Type` header value. The caller
/// is responsible for setting both on the outgoing HTTP request.
pub struct MultipartForm {
    boundary: String,
    parts: Vec<u8>,
}

impl MultipartForm {
    /// Create a new multipart form with a unique boundary.
    pub fn new() -> Self {
        let boundary = format!(
            "----formdata-aimux-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        Self {
            boundary,
            parts: Vec::new(),
        }
    }

    /// Add a text field.
    pub fn text(&mut self, name: &str, value: &str) -> &mut Self {
        self.parts
            .extend_from_slice(format!("--{}\r\n", self.boundary).as_bytes());
        self.parts.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
        );
        self.parts.extend_from_slice(value.as_bytes());
        self.parts.extend_from_slice(b"\r\n");
        self
    }

    /// Add a binary file field with a filename and media type.
    pub fn file(&mut self, name: &str, filename: &str, media_type: &str, data: &[u8]) -> &mut Self {
        self.parts
            .extend_from_slice(format!("--{}\r\n", self.boundary).as_bytes());
        self.parts.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"{name}\"; filename=\"{filename}\"\r\n")
                .as_bytes(),
        );
        self.parts
            .extend_from_slice(format!("Content-Type: {media_type}\r\n\r\n").as_bytes());
        self.parts.extend_from_slice(data);
        self.parts.extend_from_slice(b"\r\n");
        self
    }

    /// Finalize the body, returning the raw bytes and the content-type header
    /// value.
    pub fn finish(mut self) -> (Vec<u8>, String) {
        self.parts
            .extend_from_slice(format!("--{}--\r\n", self.boundary).as_bytes());
        let content_type = format!("multipart/form-data; boundary={}", self.boundary);
        (self.parts, content_type)
    }
}

impl Default for MultipartForm {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert a media type (e.g. `"audio/wav"`) to a file extension (e.g. `"wav"`).
pub fn media_type_to_extension(media_type: &str) -> String {
    media_type
        .strip_prefix("audio/")
        .unwrap_or(media_type)
        .to_string()
}
