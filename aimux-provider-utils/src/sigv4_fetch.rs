//! AWS Signature Version 4 request signing, as a [`Fetch`] decorator.
//!
//! [`SigV4Fetch`] wraps another transport and signs each request's *final*
//! body bytes just before they are sent, so Bedrock and Claude Platform on AWS
//! can authenticate through ordinary provider settings (`fetch`) instead of
//! each model re-implementing header construction.
//!
//! The signer is a self-contained implementation using only `sha2` + `hmac`
//! (no AWS SDK dependency). It supports static credentials
//! (`access_key_id` + `secret_access_key` + optional `session_token`), any AWS
//! service name and region, and any method/body.
//!
//! Reference: <https://docs.aws.amazon.com/IAM/latest/UserGuide/create-signed-request.html>

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

use crate::fetch::{Fetch, FetchError, FetchFunction, FetchRequest, FetchResponse};
use crate::resolvable::Resolvable;

/// A set of AWS credentials for SigV4 signing.
#[derive(Debug, Clone)]
pub struct AwsCredentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    /// Optional temporary session token (STS).
    pub session_token: Option<String>,
    pub region: String,
}

/// The result of signing a request.
#[derive(Debug, Clone)]
pub struct SignedRequest {
    /// All headers to send with the request, including the `Authorization`
    /// header and any `X-Amz-*` headers.
    pub headers: Vec<(String, String)>,
}

type HmacSha256 = Hmac<Sha256>;

/// Sign an HTTP request using AWS SigV4, stamped with the current time.
///
/// # Arguments
/// - `credentials` - The AWS credentials to sign with.
/// - `service` - The AWS service name (e.g. `"bedrock"`, `"aws-external-anthropic"`).
/// - `method` - HTTP method (e.g. `"POST"`).
/// - `url` - The full request URL.
/// - `body` - The request body as a string (e.g. JSON).
/// - `extra_headers` - Additional headers to include in the request (these are
///   also included in the canonical headers for signing).
#[must_use]
pub fn sign_request(
    credentials: &AwsCredentials,
    service: &str,
    method: &str,
    url: &str,
    body: &str,
    extra_headers: &[(String, String)],
) -> SignedRequest {
    sign_request_at(
        credentials,
        service,
        method,
        url,
        body.as_bytes(),
        extra_headers,
        Utc::now(),
    )
}

/// [`sign_request`] with an explicit signing time and a byte body.
#[must_use]
pub fn sign_request_at(
    credentials: &AwsCredentials,
    service: &str,
    method: &str,
    url: &str,
    body: &[u8],
    extra_headers: &[(String, String)],
    now: DateTime<Utc>,
) -> SignedRequest {
    let parsed =
        url::Url::parse(url).unwrap_or_else(|_| url::Url::parse("http://localhost").unwrap());
    // The signed host must match the wire host: keep the port whenever the
    // URL carries a non-default one (Url::port() is Some only for non-default
    // ports). Signing a bare host diverges from the Host header reqwest would
    // otherwise so gateways and proxies on non-standard ports reject the
    // signature (observed as 502 when an env proxy forwarded a signed
    // loopback request whose host lacked the port).
    let host = match parsed.port() {
        Some(port) => format!("{}:{}", parsed.host_str().unwrap_or(""), port),
        None => parsed.host_str().unwrap_or("").to_string(),
    };
    let path = parsed.path();
    let query = parsed.query().unwrap_or("");

    // Timestamp:yyyyMMddTHHmmssZ and date:yyyyMMdd
    let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
    let date_stamp = now.format("%Y%m%d").to_string();

    // Body hash
    let body_hash = hex::encode(Sha256::digest(body));

    // Build canonical headers (sorted, lowercased, trimmed).
    // Always include host, x-amz-date, and x-amz-content-sha256.
    let mut canonical_headers: Vec<(String, String)> = vec![
        ("host".to_string(), host.to_string()),
        ("x-amz-content-sha256".to_string(), body_hash.clone()),
        ("x-amz-date".to_string(), amz_date.clone()),
    ];

    if let Some(ref token) = credentials.session_token {
        canonical_headers.push(("x-amz-security-token".to_string(), token.clone()));
    }

    for (k, v) in extra_headers {
        let lower = k.to_lowercase();
        // Don't override the required headers.
        if lower != "host"
            && lower != "x-amz-date"
            && lower != "x-amz-content-sha256"
            && lower != "x-amz-security-token"
        {
            canonical_headers.push((lower, v.trim().to_string()));
        }
    }

    canonical_headers.sort_by(|a, b| a.0.cmp(&b.0));

    let canonical_headers_str: String = canonical_headers
        .iter()
        .map(|(k, v)| format!("{k}:{v}\n"))
        .collect();
    let signed_headers: String = canonical_headers
        .iter()
        .map(|(k, _)| k.as_str())
        .collect::<Vec<_>>()
        .join(";");

    // Canonical request
    let canonical_request = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        method.to_uppercase(),
        path,
        query,
        canonical_headers_str,
        signed_headers,
        body_hash
    );

    let canonical_request_hash = hex::encode(Sha256::digest(canonical_request.as_bytes()));

    // Credential scope
    let credential_scope = format!(
        "{}/{}/{}/aws4_request",
        date_stamp, credentials.region, service
    );

    // String to sign
    let string_to_sign =
        format!("AWS4-HMAC-SHA256\n{amz_date}\n{credential_scope}\n{canonical_request_hash}");

    // Signing key: derived through chained HMAC
    let k_date = hmac_sha256(
        date_stamp.as_bytes(),
        credentials.secret_access_key.as_bytes(),
    );
    let k_region = hmac_sha256(credentials.region.as_bytes(), &k_date);
    let k_service = hmac_sha256(service.as_bytes(), &k_region);
    let k_signing = hmac_sha256(b"aws4_request", &k_service);

    let signature = hex::encode(hmac_sha256(string_to_sign.as_bytes(), &k_signing));

    let authorization = format!(
        "AWS4-HMAC-SHA256 Credential={}/{}, SignedHeaders={}, Signature={}",
        credentials.access_key_id, credential_scope, signed_headers, signature
    );

    // Build the final header list.
    let mut headers: Vec<(String, String)> = canonical_headers
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    headers.push(("Authorization".to_string(), authorization));

    SignedRequest { headers }
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

type Clock = Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>;

/// A [`Fetch`] decorator that signs every request with SigV4 and forwards it
/// to the wrapped transport.
///
/// Credentials are resolved on each request, so a `Resolvable::AsyncFn` can
/// hand out rotating STS credentials. The signature covers the request's
/// final body bytes and every header present on the request (except
/// `Authorization`, which it replaces); `host`, `x-amz-date`,
/// `x-amz-content-sha256` and, for temporary credentials,
/// `x-amz-security-token` are set from the signed values.
pub struct SigV4Fetch {
    inner: FetchFunction,
    credentials: Resolvable<AwsCredentials>,
    service: String,
    clock: Clock,
}

impl SigV4Fetch {
    /// Sign requests for `service` (e.g. `"bedrock"`) with `credentials`
    /// (whose `region` scopes the signature) and forward them to `inner`.
    #[must_use]
    pub fn new(
        inner: FetchFunction,
        credentials: Resolvable<AwsCredentials>,
        service: impl Into<String>,
    ) -> Self {
        Self {
            inner,
            credentials,
            service: service.into(),
            clock: Arc::new(Utc::now),
        }
    }

    /// Replace the signing clock (deterministic signatures in tests).
    #[must_use]
    pub fn with_clock(mut self, clock: impl Fn() -> DateTime<Utc> + Send + Sync + 'static) -> Self {
        self.clock = Arc::new(clock);
        self
    }
}

impl fmt::Debug for SigV4Fetch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SigV4Fetch")
            .field("service", &self.service)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl Fetch for SigV4Fetch {
    async fn fetch(&self, mut request: FetchRequest) -> Result<FetchResponse, FetchError> {
        let credentials = self.credentials.resolve().await.map_err(|error| {
            FetchError::Other(format!("failed to resolve AWS credentials: {error}"))
        })?;
        let mut extra_headers = Vec::with_capacity(request.headers.len());
        for (name, value) in &request.headers {
            if name == http::header::AUTHORIZATION {
                continue;
            }
            let value = value.to_str().map_err(|_| {
                FetchError::Other(format!("cannot sign non-ASCII header value for {name}"))
            })?;
            extra_headers.push((name.as_str().to_string(), value.to_string()));
        }
        let signed = sign_request_at(
            &credentials,
            &self.service,
            request.method.as_str(),
            request.url.as_str(),
            &request.body,
            &extra_headers,
            (self.clock)(),
        );
        for (name, value) in signed.headers {
            let name = http::HeaderName::try_from(name.as_str()).map_err(|error| {
                FetchError::Other(format!("invalid signed header name {name}: {error}"))
            })?;
            let value = http::HeaderValue::try_from(value.as_str()).map_err(|error| {
                FetchError::Other(format!("invalid signed header value for {name}: {error}"))
            })?;
            request.headers.insert(name, value);
        }
        self.inner.fetch(request).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use bytes::Bytes;
    use chrono::TimeZone;
    use http::{HeaderMap, Method, StatusCode};

    use super::*;

    fn creds() -> AwsCredentials {
        AwsCredentials {
            access_key_id: "AKIAIOSFODNN7EXAMPLE".to_string(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_string(),
            session_token: None,
            region: "us-east-1".to_string(),
        }
    }

    fn fixed_time() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2024, 1, 2, 3, 4, 5).unwrap()
    }

    fn header(signed: &SignedRequest, name: &str) -> String {
        signed
            .headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| panic!("signed request carries a {name} header"))
    }

    /// Records the (already signed) request the decorator forwards.
    #[derive(Default)]
    struct Capture(Mutex<Option<FetchRequest>>);

    #[async_trait]
    impl Fetch for Capture {
        async fn fetch(&self, request: FetchRequest) -> Result<FetchResponse, FetchError> {
            let url = request.url.clone();
            *self.0.lock().unwrap() = Some(request);
            Ok(FetchResponse::from_bytes(
                StatusCode::OK,
                HeaderMap::new(),
                url,
                Bytes::new(),
            ))
        }
    }

    fn decorated(credentials: AwsCredentials) -> (SigV4Fetch, Arc<Capture>) {
        let capture = Arc::new(Capture::default());
        let fetch = SigV4Fetch::new(capture.clone(), Resolvable::from(credentials), "bedrock")
            .with_clock(fixed_time);
        (fetch, capture)
    }

    const URL: &str = "https://bedrock-runtime.us-east-1.amazonaws.com/model/anthropic.claude-3-5-sonnet-20240620-v1:0/converse";
    const BODY: &str = r#"{"messages":[]}"#;

    #[test]
    fn host_keeps_non_default_port() {
        let signed = sign_request(
            &creds(),
            "bedrock",
            "POST",
            "http://127.0.0.1:54321/model/x/invoke",
            "{}",
            &[],
        );
        assert_eq!(header(&signed, "host"), "127.0.0.1:54321");
    }

    #[test]
    fn host_omits_default_port() {
        let signed = sign_request(
            &creds(),
            "bedrock",
            "POST",
            "https://bedrock-runtime.us-east-1.amazonaws.com/model/x/invoke",
            "{}",
            &[],
        );
        assert_eq!(
            header(&signed, "host"),
            "bedrock-runtime.us-east-1.amazonaws.com"
        );
    }

    /// Pins the signer byte-for-byte to the pre-move Bedrock signer
    /// implementation: the expected values come from an independent
    /// re-implementation of that exact algorithm run at the same fixed time.
    #[test]
    fn signer_output_is_unchanged_by_the_move() {
        let signed = sign_request_at(
            &creds(),
            "bedrock",
            "POST",
            URL,
            BODY.as_bytes(),
            &[("X-Custom".to_string(), "  value ".to_string())],
            fixed_time(),
        );
        assert_eq!(
            header(&signed, "authorization"),
            "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20240102/us-east-1/bedrock/aws4_request, \
             SignedHeaders=host;x-amz-content-sha256;x-amz-date;x-custom, \
             Signature=50198f4d3c821ae4aa0518965cd3ecaeb81e542a155dc57766e1e28a6ae696b8"
        );

        let session = AwsCredentials {
            session_token: Some("SESSION".to_string()),
            ..creds()
        };
        let signed = sign_request_at(
            &session,
            "bedrock",
            "POST",
            "http://127.0.0.1:54321/model/x/invoke?a=1",
            b"{}",
            &[],
            fixed_time(),
        );
        assert_eq!(
            header(&signed, "authorization"),
            "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20240102/us-east-1/bedrock/aws4_request, \
             SignedHeaders=host;x-amz-content-sha256;x-amz-date;x-amz-security-token, \
             Signature=8421497f0591d30210a67833cdd30e0b8693f941ed28ecf4aff8751531cbf783"
        );
        assert_eq!(header(&signed, "x-amz-security-token"), "SESSION");
    }

    #[tokio::test]
    async fn decorator_authorization_is_byte_identical_to_sign_request() {
        let (fetch, capture) = decorated(creds());
        let mut request = FetchRequest::new(Method::POST, URL.parse().unwrap());
        request
            .headers
            .insert("x-custom", "  value ".parse().unwrap());
        request.body = Bytes::from(BODY);

        fetch.fetch(request).await.unwrap();

        let sent = capture
            .0
            .lock()
            .unwrap()
            .take()
            .expect("inner fetch was called");
        let expected = sign_request_at(
            &creds(),
            "bedrock",
            "POST",
            URL,
            BODY.as_bytes(),
            &[("x-custom".to_string(), "  value ".to_string())],
            fixed_time(),
        );
        assert_eq!(
            sent.headers["authorization"].to_str().unwrap(),
            header(&expected, "authorization")
        );
        for name in ["host", "x-amz-date", "x-amz-content-sha256"] {
            assert_eq!(
                sent.headers[name].to_str().unwrap(),
                header(&expected, name),
                "{name}"
            );
        }
        assert_eq!(sent.body, Bytes::from(BODY));
        assert!(!sent.headers.contains_key("x-amz-security-token"));
    }

    #[tokio::test]
    async fn decorator_replaces_a_caller_authorization_and_signs_the_session_token() {
        let session = AwsCredentials {
            session_token: Some("SESSION".to_string()),
            ..creds()
        };
        let (fetch, capture) = decorated(session);
        let mut request = FetchRequest::new(Method::POST, URL.parse().unwrap());
        request
            .headers
            .insert(http::header::AUTHORIZATION, "Bearer stale".parse().unwrap());
        request.body = Bytes::from(BODY);

        fetch.fetch(request).await.unwrap();

        let sent = capture.0.lock().unwrap().take().unwrap();
        let authorization = sent.headers["authorization"].to_str().unwrap();
        assert!(authorization.starts_with("AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/"));
        assert!(authorization.contains("x-amz-security-token"));
        assert_eq!(sent.headers.get_all("authorization").iter().count(), 1);
        assert_eq!(sent.headers["x-amz-security-token"], "SESSION");
    }

    #[tokio::test]
    async fn credentials_are_resolved_per_request() {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = calls.clone();
        let credentials = Resolvable::from_async_fn(move || {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(creds())
            }
        });
        let capture = Arc::new(Capture::default());
        let fetch = SigV4Fetch::new(capture, credentials, "bedrock");
        for _ in 0..2 {
            fetch
                .fetch(FetchRequest::new(Method::GET, URL.parse().unwrap()))
                .await
                .unwrap();
        }
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn credential_failures_surface_without_calling_the_inner_fetch() {
        let capture = Arc::new(Capture::default());
        let fetch = SigV4Fetch::new(
            capture.clone(),
            Resolvable::from_fn(|| {
                Err(aimux_core::AiMuxError::LoadSetting {
                    env_var: "AWS_ACCESS_KEY_ID".into(),
                    name: "accessKeyId".into(),
                })
            }),
            "bedrock",
        );
        let error = fetch
            .fetch(FetchRequest::new(Method::GET, URL.parse().unwrap()))
            .await
            .unwrap_err();
        assert!(matches!(&error, FetchError::Other(m) if m.contains("AWS_ACCESS_KEY_ID")));
        assert!(capture.0.lock().unwrap().is_none());
    }
}
