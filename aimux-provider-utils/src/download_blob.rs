//! Download helper aligned with AI SDK `downloadBlob`.

use aimux_core::{AbortSignal, AiMuxError};
use bytes::Bytes;

use crate::http::HttpRequest;
use crate::response_handler::{
    ResponseHandlerOutput, create_binary_response_handler,
    create_status_code_error_response_handler,
};

/// Download a URL that came from outside the provider configuration, as the
/// AI SDK's `downloadBlob` does (provider-utils download-blob.ts:106-142).
///
/// The GET goes through the SSRF download guard, like `fetchUntrustedUrl`
/// (fetch-untrusted-url.ts:46-86), and sends no headers: `downloadBlob`
/// passes none, so there is no `user-agent` and no
/// `ai-sdk-provider-utils/<v>` part, which only `getFromApi` and `postToApi`
/// add. A non-2xx status fails like
/// [`create_status_code_error_response_handler`].
///
/// # Errors
///
/// Returns a guard rejection, a transport failure, the status error, or the
/// caller's abort.
pub async fn download_blob(
    url: impl Into<String>,
    abort_signal: Option<AbortSignal>,
) -> Result<ResponseHandlerOutput<Bytes>, AiMuxError> {
    download(HttpRequest {
        url: url.into(),
        abort_signal,
        validate_url: true,
        ..HttpRequest::default()
    })
    .await
}

async fn download(request: HttpRequest) -> Result<ResponseHandlerOutput<Bytes>, AiMuxError> {
    crate::get_from_api(
        HttpRequest {
            omit_provider_utils_user_agent: true,
            ..request
        },
        create_binary_response_handler(),
        create_status_code_error_response_handler(),
    )
    .await
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    async fn server() -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/image.png"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"png".to_vec()))
            .mount(&server)
            .await;
        server
    }

    /// The loopback mock server is reachable only through the trusted-origin
    /// exemption; `download_blob` itself takes none, as `downloadBlob`.
    fn guarded(server: &MockServer) -> HttpRequest {
        HttpRequest {
            url: format!("{}/image.png", server.uri()),
            validate_url: true,
            trusted_origin: Some(server.uri()),
            ..HttpRequest::default()
        }
    }

    /// download-blob.ts:111-114 calls `fetchUntrustedUrl({url, abortSignal})`
    /// with no headers, so nothing adds a `user-agent`.
    #[tokio::test]
    async fn a_download_sends_no_user_agent() {
        let server = server().await;
        let output = download(guarded(&server)).await.unwrap();
        assert_eq!(&output.value[..], b"png");
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].headers.get("user-agent"), None);
    }

    /// `getFromApi` with `validateUrl` still appends its part
    /// (get-from-api.ts:77-90): only `downloadBlob` sends none.
    #[tokio::test]
    async fn a_validated_get_from_api_keeps_its_user_agent_part() {
        let server = server().await;
        crate::get_from_api(
            guarded(&server),
            create_binary_response_handler(),
            create_status_code_error_response_handler(),
        )
        .await
        .unwrap();
        let requests = server.received_requests().await.unwrap();
        assert_eq!(
            requests[0].headers.get("user-agent").unwrap(),
            "ai-sdk-provider-utils/5.0.53"
        );
    }

    /// The URL is untrusted: a non-public address is refused before any
    /// connection (validate-download-url.ts via fetch-with-validated-redirects.ts:164-166).
    #[tokio::test]
    async fn a_download_is_guarded() {
        let error = download_blob("http://127.0.0.1:9/image.png", None)
            .await
            .unwrap_err();
        assert!(
            matches!(error, AiMuxError::InvalidArgument { ref message, .. } if message.contains("non-public")),
            "{error:?}"
        );
    }
}
