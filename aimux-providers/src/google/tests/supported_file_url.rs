use crate::google::utils::is_supported_file_url;

/// `new URL(text)`: the URL as the WHATWG parser serializes it (a bare origin
/// gains its `/` path), which is what upstream hands to `isSupportedFileUrl`.
fn new_url(text: &str) -> String {
    url::Url::parse(text).expect("valid URL").to_string()
}

/// upstream: @ai-sdk/google src/google-supported-file-url.test.ts › should return true for valid Google generative language file URLs
#[test]
fn should_return_true_for_valid_google_generative_language_file_urls() {
    let valid_url = new_url(
        "https://generativelanguage.googleapis.com/v1beta/files/00000000-00000000-00000000-00000000",
    );
    assert!(is_supported_file_url(&valid_url));

    let simple_valid_url =
        new_url("https://generativelanguage.googleapis.com/v1beta/files/test123");
    assert!(is_supported_file_url(&simple_valid_url));
}

/// upstream: @ai-sdk/google src/google-supported-file-url.test.ts › should return true for valid YouTube URLs
#[test]
fn should_return_true_for_valid_you_tube_urls() {
    let valid_you_tube_urls = [
        new_url("https://www.youtube.com/watch?v=dQw4w9WgXcQ"),
        new_url("https://youtube.com/watch?v=dQw4w9WgXcQ"),
        new_url("https://youtu.be/dQw4w9WgXcQ"),
        new_url("https://www.youtube.com/watch?v=dQw4w9WgXcQ&feature=youtu.be"),
        new_url("https://youtu.be/dQw4w9WgXcQ?t=42"),
    ];

    for url in &valid_you_tube_urls {
        assert!(is_supported_file_url(url), "{url}");
    }
}

/// upstream: @ai-sdk/google src/google-supported-file-url.test.ts › should return false for invalid YouTube URLs
#[test]
fn should_return_false_for_invalid_you_tube_urls() {
    let invalid_you_tube_urls = [
        new_url("https://youtube.com/channel/UCdQw4w9WgXcQ"),
        new_url("https://youtube.com/playlist?list=PLdQw4w9WgXcQ"),
        new_url("https://m.youtube.com/watch?v=dQw4w9WgXcQ"),
        new_url("http://youtube.com/watch?v=dQw4w9WgXcQ"),
        new_url("https://vimeo.com/123456789"),
    ];

    for url in &invalid_you_tube_urls {
        assert!(!is_supported_file_url(url), "{url}");
    }
}

/// upstream: @ai-sdk/google src/google-supported-file-url.test.ts › should return false for non-Google generative language file URLs
#[test]
fn should_return_false_for_non_google_generative_language_file_urls() {
    let test_cases = [
        new_url("https://example.com"),
        new_url("https://example.com/foo/bar"),
        new_url("https://generativelanguage.googleapis.com"),
        new_url("https://generativelanguage.googleapis.com/v1/other"),
        new_url("http://generativelanguage.googleapis.com/v1beta/files/test"),
        new_url("https://api.googleapis.com/v1beta/files/test"),
    ];

    for url in &test_cases {
        assert!(!is_supported_file_url(url), "{url}");
    }
}
