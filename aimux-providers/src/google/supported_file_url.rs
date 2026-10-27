//! `google-supported-file-url.ts`.

/// Whether a URL is supported as a file input by the Google Gemini API.
///
/// Mirrors `isSupportedFileUrl` in `google-supported-file-url.ts`:
/// - `https://generativelanguage.googleapis.com/v1beta/files/...` → true
/// - YouTube watch / youtu.be URLs → true
/// - everything else → false
#[cfg(test)]
#[must_use]
pub(super) fn is_supported_file_url(url: &str) -> bool {
    if url.starts_with("https://generativelanguage.googleapis.com/v1beta/files/") {
        return true;
    }

    static YOUTUBE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    YOUTUBE
        .get_or_init(|| {
            regex::Regex::new(
                r"^https://(?:(?:www\.)?youtube\.com/watch\?v=[\w-]+(?:&[\w=&.-]*)?|youtu\.be/[\w-]+(?:\?[\w=&.-]*)?)$",
            )
            .expect("static YouTube URL pattern")
        })
        .is_match(url)
}
