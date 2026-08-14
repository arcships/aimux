//! A simple multipart/form-data body builder.
//!
//! Builds the raw bytes of a `multipart/form-data` body without relying on
//! reqwest's `multipart` feature (which pulls in `mime_guess`, whose build
//! script may be blocked by application control policies on some systems).

use aimux_core::AiMuxError;

/// A simple multipart form-data builder.
///
/// Produces the raw body bytes and the `Content-Type` header value. The caller
/// is responsible for setting both on the outgoing HTTP request.
#[derive(Debug)]
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
    ///
    /// `name` is validated before it is interpolated into the MIME headers; see
    /// `validate_multipart_param` for the rules.
    pub fn text(&mut self, name: &str, value: &str) -> Result<&mut Self, AiMuxError> {
        validate_multipart_param(name, "name")?;
        self.parts
            .extend_from_slice(format!("--{}\r\n", self.boundary).as_bytes());
        self.parts.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
        );
        self.parts.extend_from_slice(value.as_bytes());
        self.parts.extend_from_slice(b"\r\n");
        Ok(self)
    }

    /// Add a binary file field with a filename and media type.
    ///
    /// `name`, `filename` and `media_type` are validated before they are
    /// interpolated into the MIME headers; see `validate_multipart_param` for
    /// the rules.
    pub fn file(
        &mut self,
        name: &str,
        filename: &str,
        media_type: &str,
        data: &[u8],
    ) -> Result<&mut Self, AiMuxError> {
        validate_multipart_param(name, "name")?;
        validate_multipart_param(filename, "filename")?;
        validate_multipart_param(media_type, "media_type")?;
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
        Ok(self)
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

/// Validate a string interpolated into a multipart MIME header.
///
/// Field parameters (`name`, `filename`, `media_type`) are inserted verbatim
/// into `Content-Disposition` / `Content-Type` header lines. Without
/// validation, a parameter containing `"` breaks out of a quoted-string, and a
/// parameter containing CR/LF injects a new header line (CRLF injection /
/// header splitting). NUL also corrupts parsing. Any of these let an attacker
/// forge MIME parts or smuggle headers.
///
/// To stay fail-fast and unambiguous we **reject** such parameters rather than
/// attempt escaping — callers pass trusted, structured values (model ids,
/// generated filenames, known media types) that never legitimately contain
/// these characters.
fn validate_multipart_param(value: &str, label: &str) -> Result<(), AiMuxError> {
    if value
        .as_bytes()
        .iter()
        .any(|&b| b == b'"' || b == b'\r' || b == b'\n' || b == 0)
    {
        return Err(AiMuxError::InvalidArgument(format!(
            "invalid multipart {label}: must not contain '\"', CR, LF or NUL"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_param_rejected(value: &str) {
        assert!(
            validate_multipart_param(value, "name").is_err(),
            "expected value to be rejected: {value:?}"
        );
    }

    #[test]
    fn validate_accepts_safe_params() {
        assert!(validate_multipart_param("model", "name").is_ok());
        assert!(validate_multipart_param("file", "name").is_ok());
        assert!(validate_multipart_param("audio.wav", "filename").is_ok());
        assert!(validate_multipart_param("audio/wav", "media_type").is_ok());
        assert!(validate_multipart_param("timestamp_granularities[]", "name").is_ok());
    }

    #[test]
    fn validate_rejects_quote() {
        assert_param_rejected("evil\"");
    }

    #[test]
    fn validate_rejects_crlf() {
        assert_param_rejected("evil\r\nX-Injected: yes");
        assert_param_rejected("line1\nline2");
        assert_param_rejected("line1\rline2");
    }

    #[test]
    fn validate_rejects_nul() {
        assert_param_rejected("evil\u{0}more");
    }

    #[test]
    fn text_rejects_malicious_name() {
        // A double-quote would break out of the quoted-string.
        let mut form = MultipartForm::new();
        let err = form.text("evil\"", "value").unwrap_err();
        assert!(matches!(err, AiMuxError::InvalidArgument(_)), "got {err:?}");
        assert!(err.to_string().contains("multipart name"));

        // CRLF would inject a new header line.
        let mut form = MultipartForm::new();
        let err = form.text("evil\r\nX-Injected: yes", "value").unwrap_err();
        assert!(matches!(err, AiMuxError::InvalidArgument(_)));

        // Nothing should have been appended after a rejected field.
        let mut form = MultipartForm::new();
        let _ = form.text("evil\r\n", "value");
        assert!(
            form.parts.is_empty(),
            "parts must remain empty on rejection"
        );
    }

    #[test]
    fn file_rejects_malicious_filename_and_media_type() {
        // Malicious filename (quote injection).
        let mut form = MultipartForm::new();
        let err = form
            .file("file", "evil\".wav", "audio/wav", b"data")
            .unwrap_err();
        assert!(matches!(err, AiMuxError::InvalidArgument(_)));
        assert!(err.to_string().contains("multipart filename"));

        // Malicious filename (CRLF injection).
        let mut form = MultipartForm::new();
        let err = form
            .file("file", "evil\r\nX-Injected: yes", "audio/wav", b"data")
            .unwrap_err();
        assert!(matches!(err, AiMuxError::InvalidArgument(_)));

        // Malicious media type (CRLF injection / header splitting).
        let mut form = MultipartForm::new();
        let err = form
            .file("file", "audio.wav", "audio/wav\r\nX-Injected: yes", b"data")
            .unwrap_err();
        assert!(matches!(err, AiMuxError::InvalidArgument(_)));
        assert!(err.to_string().contains("multipart media_type"));

        // Malicious name is rejected before any bytes are appended.
        let mut form = MultipartForm::new();
        let _ = form.file("evil\"", "audio.wav", "audio/wav", b"data");
        assert!(
            form.parts.is_empty(),
            "parts must remain empty on rejection"
        );
    }

    #[test]
    fn text_and_file_accept_valid_fields() {
        let mut form = MultipartForm::new();
        form.text("model", "whisper-1").unwrap();
        form.file("file", "audio.wav", "audio/wav", b"data")
            .unwrap();
        form.text("diarize", "true").unwrap();
        let (body, content_type) = form.finish();

        assert!(content_type.starts_with("multipart/form-data; boundary="));
        let body = String::from_utf8(body).unwrap();
        assert!(body.contains("name=\"model\""));
        assert!(body.contains("filename=\"audio.wav\""));
        assert!(body.contains("Content-Type: audio/wav"));
        assert!(body.contains("diarize"));
        // No injected headers.
        assert!(!body.contains("X-Injected"));
    }
}
