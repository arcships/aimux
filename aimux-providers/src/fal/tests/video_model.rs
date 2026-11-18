use aimux_provider_utils::HttpRequest;

use crate::fal::video_model::status_request;

/// The `Key` credential is gated on the submit origin, so a
/// `response_url` on another origin is polled without it (#441). The
/// gate itself is `HttpRequest::credentialed_origin`; that it drops the
/// caller headers is tested in `aimux-provider-utils` (`http.rs`).
#[test]
fn a_status_poll_is_credentialed_and_trusted_only_on_the_submit_origin() {
    let submit_url = "https://queue.fal.run/fal-ai/luma-dream-machine";
    let request = status_request(
        HttpRequest {
            url: "https://elsewhere.example/requests/1/status".into(),
            headers: vec![("authorization".into(), "Key test-api-key".into())],
            ..HttpRequest::default()
        },
        submit_url,
    );

    assert!(request.validate_url);
    assert_eq!(request.credentialed_origin.as_deref(), Some(submit_url));
    assert_eq!(request.trusted_origin.as_deref(), Some(submit_url));
}
