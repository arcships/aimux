//! AWS Signature Version 4 request signing, as a [`Fetch`] decorator.
//!
//! [`SigV4Fetch`] wraps another transport and signs each request's *final*
//! body bytes just before they are sent, so AWS-hosted providers
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

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

use crate::fetch::{
    Fetch, FetchError, FetchFunction, FetchRequest, FetchResponse, RequestCredentials,
};
use crate::resolvable::Resolvable;

/// A set of AWS credentials for SigV4 signing.
///
/// `Debug` is hand-written so the secret key and session token never reach
/// logs or error output; the access key id and region stay visible because
/// they identify the credentials without granting access.
#[derive(Clone)]
pub struct AwsCredentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    /// Optional temporary session token (STS).
    pub session_token: Option<String>,
    pub region: String,
}

impl fmt::Debug for AwsCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AwsCredentials")
            .field("access_key_id", &self.access_key_id)
            .field("secret_access_key", &"[REDACTED]")
            .field(
                "session_token",
                &self.session_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("region", &self.region)
            .finish()
    }
}

/// The result of signing a request.
#[derive(Debug, Clone)]
struct SignedRequest {
    /// All headers to send with the request, including the `Authorization`
    /// header and any `X-Amz-*` headers.
    pub headers: Vec<(String, String)>,
}

type HmacSha256 = Hmac<Sha256>;

/// The headers that are sent but never signed under
/// [`SigV4Signing::Aws4Fetch`]: aws4fetch 1.0.20's `UNSIGNABLE_HEADERS`
/// (lowercase names). `authorization` is never signed under either rule.
const UNSIGNABLE_HEADERS: &[&str] = &[
    "authorization",
    "content-type",
    "content-length",
    "user-agent",
    "presigned-expires",
    "expect",
    "x-amzn-trace-id",
    "range",
    "connection",
];

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
#[cfg(test)]
#[must_use]
fn sign_request(
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

/// Sign an HTTP request with an explicit signing time and a byte body, under
/// [`SigV4Signing::AllHeaders`].
#[cfg(test)]
#[must_use]
fn sign_request_at(
    credentials: &AwsCredentials,
    service: &str,
    method: &str,
    url: &str,
    body: &[u8],
    extra_headers: &[(String, String)],
    now: DateTime<Utc>,
) -> SignedRequest {
    sign_request_with(
        credentials,
        service,
        method,
        url,
        body,
        extra_headers,
        now,
        SigV4Signing::AllHeaders,
    )
}

/// [`sign_request_at`] under `signing`: [`SigV4Signing::Aws4Fetch`] neither
/// sends nor signs `x-amz-content-sha256`, as aws4fetch does for every
/// service but s3.
#[allow(clippy::too_many_arguments)]
#[must_use]
fn sign_request_with(
    credentials: &AwsCredentials,
    service: &str,
    method: &str,
    url: &str,
    body: &[u8],
    extra_headers: &[(String, String)],
    now: DateTime<Utc>,
    signing: SigV4Signing,
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
    let path = canonical_uri(parsed.path());
    let query = canonical_query(parsed.query().unwrap_or(""));

    // Timestamp:yyyyMMddTHHmmssZ and date:yyyyMMdd
    let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
    let date_stamp = now.format("%Y%m%d").to_string();

    // Body hash
    let body_hash = hex::encode(Sha256::digest(body));

    // Build canonical headers (sorted, lowercased, trimmed).
    // Always include host and x-amz-date; x-amz-content-sha256 only under
    // `AllHeaders` (aws4fetch adds it for s3 alone).
    let mut canonical_headers: Vec<(String, String)> = vec![
        ("host".to_string(), host.to_string()),
        ("x-amz-date".to_string(), amz_date.clone()),
    ];
    if signing == SigV4Signing::AllHeaders {
        canonical_headers.push(("x-amz-content-sha256".to_string(), body_hash.clone()));
    }

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
            canonical_headers.push((lower, canonical_header_value(v)));
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

    // Signing key: derived through chained HMAC, each step keyed by the
    // previous one and starting from "AWS4" + secret.
    let k_date = hmac_sha256(
        format!("AWS4{}", credentials.secret_access_key).as_bytes(),
        date_stamp.as_bytes(),
    );
    let k_region = hmac_sha256(&k_date, credentials.region.as_bytes());
    let k_service = hmac_sha256(&k_region, service.as_bytes());
    let k_signing = hmac_sha256(&k_service, b"aws4_request");

    let signature = hex::encode(hmac_sha256(&k_signing, string_to_sign.as_bytes()));

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

/// SigV4 `UriEncode`: every byte except the unreserved set
/// (`A-Z a-z 0-9 - _ . ~`) becomes `%XX` with uppercase hex.
fn uri_encode(input: &[u8]) -> String {
    let mut out = String::with_capacity(input.len());
    for &byte in input {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// Canonical URI for every service but S3: each segment of the path as sent
/// is URI-encoded again, so `:` signs as `%3A` and an already-escaped `%3A`
/// signs as `%253A`.
fn canonical_uri(path: &str) -> String {
    if path.is_empty() {
        return "/".to_string();
    }
    path.split('/')
        .map(|segment| uri_encode(segment.as_bytes()))
        .collect::<Vec<_>>()
        .join("/")
}

/// Canonical query string: each name and value percent-decoded, then
/// URI-encoded, and the pairs sorted by name and then value.
fn canonical_query(query: &str) -> String {
    let mut pairs: Vec<(String, String)> = query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
            (
                uri_encode(&percent_decode(name)),
                uri_encode(&percent_decode(value)),
            )
        })
        .collect();
    pairs.sort();
    pairs
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("&")
}

fn percent_decode(input: &str) -> Vec<u8> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let escaped = (bytes[i] == b'%')
            .then(|| bytes.get(i + 1..i + 3))
            .flatten()
            .and_then(|hex| std::str::from_utf8(hex).ok())
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match escaped {
            Some(byte) => {
                out.push(byte);
                i += 3;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    out
}

/// Canonical header value: trimmed, with runs of spaces collapsed to one.
fn canonical_header_value(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

/// The signing clock: a plain function, so a fixed time in tests is a named
/// function or a non-capturing closure.
type Clock = fn() -> DateTime<Utc>;

/// A [`Fetch`] decorator that signs every request with SigV4 and forwards it
/// to the wrapped transport. A request marked [`RequestCredentials::Omit`] is
/// forwarded unsigned.
///
/// Credentials are resolved on each request, so a `Resolvable::AsyncFn` can
/// hand out rotating STS credentials. The signature covers the request's
/// final body bytes and the headers [`SigV4Signing`] names; `host`,
/// `x-amz-date`, `x-amz-content-sha256` (under [`SigV4Signing::AllHeaders`])
/// and, for temporary credentials, `x-amz-security-token` are set from the
/// signed values, and `Authorization` is replaced.
pub struct SigV4Fetch {
    inner: FetchFunction,
    credentials: Resolvable<AwsCredentials>,
    service: String,
    clock: Clock,
    signing: SigV4Signing,
}

/// Which headers a [`SigV4Fetch`] signs: the one per-caller choice. There is
/// no default: every caller names its rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigV4Signing {
    /// Every request header except `authorization`, plus a sent and signed
    /// `x-amz-content-sha256` (botocore's `SigV4Auth`). For callers with no
    /// AI SDK counterpart (Amazon Polly).
    AllHeaders,
    /// aws4fetch 1.0.20, the signer of the AI SDK's AWS providers
    /// (`AwsV4Signer`): the headers in its `UNSIGNABLE_HEADERS`
    /// (user-agent, content-type, ...) are sent but not signed, and
    /// `x-amz-content-sha256` is neither sent nor signed. (aws4fetch adds it
    /// only for service `s3`, as `UNSIGNED-PAYLOAD`; no aimux caller signs
    /// for s3, and this rule does not implement that case.)
    Aws4Fetch,
}

impl SigV4Fetch {
    /// Sign requests for `service` (e.g. `"bedrock"`) with `credentials`
    /// (whose `region` scopes the signature) under `signing`, and forward
    /// them to `inner`.
    #[must_use]
    pub fn new(
        inner: FetchFunction,
        credentials: Resolvable<AwsCredentials>,
        service: impl Into<String>,
        signing: SigV4Signing,
    ) -> Self {
        Self {
            inner,
            credentials,
            service: service.into(),
            clock: Utc::now,
            signing,
        }
    }

    /// Replace the signing clock (deterministic signatures in tests).
    #[must_use]
    pub fn with_clock(mut self, clock: fn() -> DateTime<Utc>) -> Self {
        self.clock = clock;
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
        // A redirect hop off the credentialed origin: neither the signature
        // nor `x-amz-security-token` may reach that host.
        if request.credentials == RequestCredentials::Omit {
            return self.inner.fetch(request).await;
        }
        let credentials = self.credentials.resolve().await.map_err(|error| {
            FetchError::Other(format!("failed to resolve AWS credentials: {error}"))
        })?;
        let mut extra_headers = Vec::with_capacity(request.headers.len());
        for (name, value) in &request.headers {
            let unsigned = match self.signing {
                SigV4Signing::AllHeaders => name == http::header::AUTHORIZATION,
                SigV4Signing::Aws4Fetch => UNSIGNABLE_HEADERS.contains(&name.as_str()),
            };
            if unsigned {
                continue;
            }
            let value = value.to_str().map_err(|_| {
                FetchError::Other(format!("cannot sign non-ASCII header value for {name}"))
            })?;
            extra_headers.push((name.as_str().to_string(), value.to_string()));
        }
        let signed = sign_request_with(
            &credentials,
            &self.service,
            request.method.as_str(),
            request.url.as_str(),
            &request.body,
            &extra_headers,
            (self.clock)(),
            self.signing,
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
    use std::sync::{Arc, Mutex};

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

    #[test]
    fn debug_redacts_secret_key_and_session_token() {
        let mut credentials = creds();
        credentials.session_token = Some("FwoGZXIvYXdzEXAMPLETOKEN".to_string());
        let printed = format!("{credentials:?}");
        assert!(!printed.contains("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"));
        assert!(!printed.contains("FwoGZXIvYXdzEXAMPLETOKEN"));
        assert!(printed.contains("[REDACTED]"));
        assert!(printed.contains("AKIAIOSFODNN7EXAMPLE"));
        assert!(printed.contains("us-east-1"));
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
        let fetch = SigV4Fetch::new(
            capture.clone(),
            Resolvable::from(credentials),
            "bedrock",
            SigV4Signing::AllHeaders,
        )
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

    fn signature(signed: &SignedRequest) -> String {
        header(signed, "authorization")
            .rsplit_once("Signature=")
            .expect("authorization carries a signature")
            .1
            .to_string()
    }

    /// Expected values come from botocore 1.43's `SigV4Auth` (AWS's own
    /// Python signer) for the same credentials, headers, body and fixed
    /// time, not from this implementation. The request was given an explicit
    /// `X-Amz-Content-SHA256: <hex body hash>` header before signing, as the
    /// `AllHeaders` rule sends one; `SigV4Auth` adds that header itself only
    /// for S3 (`S3SigV4Auth`), so botocore's values differ without it.
    #[test]
    fn signatures_match_botocore() {
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
             Signature=538444fb4b17f3cdd777bbd5f1cd4e3e1f53d6ac97167da96c81ec8d14d6468a"
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
             Signature=dd6a2384cb9c8f0db8ea8ee383a61bf06dd85da679cec54d5c944b1b0cb8d9f6"
        );
        assert_eq!(header(&signed, "x-amz-security-token"), "SESSION");
    }

    /// An escaped path segment is encoded again, header whitespace collapses,
    /// and query pairs are sorted (same botocore reference as above).
    #[test]
    fn canonicalization_matches_botocore() {
        let signed = sign_request_at(
            &creds(),
            "bedrock",
            "POST",
            "https://bedrock-runtime.us-east-1.amazonaws.com/model/us.anthropic.claude-3-5-sonnet-20240620-v1%3A0/converse-stream",
            b"{}",
            &[
                ("content-type".to_string(), "application/json".to_string()),
                ("X-Multi".to_string(), "a   b  c".to_string()),
            ],
            fixed_time(),
        );
        assert_eq!(
            signature(&signed),
            "a6abc7f26ad04c0922b03e05c9184a20a8fcf4b68b7dfce358e9a6436c8a9489"
        );

        let signed = sign_request_at(
            &creds(),
            "bedrock",
            "POST",
            "https://polly.us-east-1.amazonaws.com/v1/speech?b=2&a=x%20y&a=1",
            b"{}",
            &[],
            fixed_time(),
        );
        assert_eq!(
            signature(&signed),
            "70c6e5794c4f1456f33f2b41cebc483ee0d19c98b95660411f4b51ccac464006"
        );

        let signed = sign_request_at(
            &creds(),
            "bedrock",
            "GET",
            "https://h.example.com",
            b"",
            &[],
            fixed_time(),
        );
        assert_eq!(
            signature(&signed),
            "4810ddfb5f22377b8c39477909c5abf54ea3c7e03f9db0f8ccb9377ed39ef26d"
        );
    }

    #[test]
    fn canonical_uri_and_query_follow_sigv4() {
        assert_eq!(canonical_uri(""), "/");
        assert_eq!(
            canonical_uri("/model/a.b-v1:0/converse"),
            "/model/a.b-v1%3A0/converse"
        );
        assert_eq!(canonical_uri("/a+b/c%2Fd"), "/a%2Bb/c%252Fd");
        assert_eq!(
            canonical_query("b=2&a=x%20y&a=1&e=&z"),
            "a=1&a=x%20y&b=2&e=&z="
        );
    }

    /// `SigV4Signing::Aws4Fetch` signs like aws4fetch 1.0.20's
    /// `AwsV4Signer` (dist/aws4fetch.esm.mjs): the `UNSIGNABLE_HEADERS`
    /// (user-agent, content-type, ...) are sent but left out of
    /// `SignedHeaders`, and no `x-amz-content-sha256` is added for a non-s3
    /// service. With only unsignable headers on the request, the signature
    /// is the AWS SigV4 test suite's `get-vanilla` / `post-vanilla` one.
    #[tokio::test]
    async fn aws4fetch_signing_matches_the_aws_test_suite_vanilla_vectors() {
        let credentials = AwsCredentials {
            access_key_id: "AKIDEXAMPLE".to_string(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".to_string(),
            session_token: None,
            region: "us-east-1".to_string(),
        };
        for (method, signature) in [
            (
                Method::GET,
                "5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31",
            ),
            (
                Method::POST,
                "5da7c1a2acd57cee7505fc6676e4e544621c30862966e37dddb68e92efbe5d6b",
            ),
        ] {
            let capture = Arc::new(Capture::default());
            let fetch = SigV4Fetch::new(
                capture.clone(),
                Resolvable::from(credentials.clone()),
                "service",
                SigV4Signing::Aws4Fetch,
            )
            .with_clock(|| Utc.with_ymd_and_hms(2015, 8, 30, 12, 36, 0).unwrap());
            let mut request = FetchRequest::new(
                method.clone(),
                "https://example.amazonaws.com/".parse().unwrap(),
            );
            for (name, value) in [
                ("user-agent", "app/1 ai-sdk-provider-utils/5.0.53"),
                ("content-type", "application/json"),
                ("content-length", "0"),
            ] {
                request.headers.insert(name, value.parse().unwrap());
            }
            fetch.fetch(request).await.unwrap();
            let sent = capture.0.lock().unwrap().take().unwrap();
            assert_eq!(
                sent.headers["authorization"].to_str().unwrap(),
                format!(
                    "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request, \
                     SignedHeaders=host;x-amz-date, Signature={signature}"
                ),
                "{method}"
            );
            assert_eq!(sent.headers["x-amz-date"], "20150830T123600Z");
            assert!(!sent.headers.contains_key("x-amz-content-sha256"));
            assert_eq!(
                sent.headers["user-agent"],
                "app/1 ai-sdk-provider-utils/5.0.53"
            );
        }
    }

    /// Under `Aws4Fetch` a signable request header is signed, and a session
    /// token is sent and signed as `x-amz-security-token`, as aws4fetch does.
    #[tokio::test]
    async fn aws4fetch_signing_signs_other_headers_and_the_session_token() {
        let capture = Arc::new(Capture::default());
        let fetch = SigV4Fetch::new(
            capture.clone(),
            Resolvable::from(AwsCredentials {
                session_token: Some("SESSION".to_string()),
                ..creds()
            }),
            "aws-external-anthropic",
            SigV4Signing::Aws4Fetch,
        )
        .with_clock(fixed_time);
        let mut request = FetchRequest::new(Method::POST, URL.parse().unwrap());
        request
            .headers
            .insert("anthropic-version", "2023-06-01".parse().unwrap());
        request.headers.insert(
            "user-agent",
            "ai-sdk-provider-utils/5.0.53".parse().unwrap(),
        );
        request.body = Bytes::from(BODY);
        fetch.fetch(request).await.unwrap();
        let sent = capture.0.lock().unwrap().take().unwrap();
        let authorization = sent.headers["authorization"].to_str().unwrap();
        assert!(
            authorization.contains(
                " SignedHeaders=anthropic-version;host;x-amz-date;x-amz-security-token, "
            ),
            "{authorization}"
        );
        assert_eq!(sent.headers["x-amz-security-token"], "SESSION");
        assert!(!sent.headers.contains_key("x-amz-content-sha256"));
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
    async fn a_request_that_omits_credentials_is_forwarded_unsigned() {
        let session = AwsCredentials {
            session_token: Some("SESSION".to_string()),
            ..creds()
        };
        let (fetch, capture) = decorated(session);
        let mut request = FetchRequest::new(Method::POST, URL.parse().unwrap());
        request.body = Bytes::from(BODY);
        request.credentials = RequestCredentials::Omit;

        fetch.fetch(request).await.unwrap();

        let sent = capture.0.lock().unwrap().take().unwrap();
        for name in [
            "authorization",
            "x-amz-date",
            "x-amz-content-sha256",
            "x-amz-security-token",
        ] {
            assert!(!sent.headers.contains_key(name), "{name}");
        }
        assert_eq!(sent.body, Bytes::from(BODY));
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
        let fetch = SigV4Fetch::new(capture, credentials, "bedrock", SigV4Signing::AllHeaders);
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
            SigV4Signing::AllHeaders,
        );
        let error = fetch
            .fetch(FetchRequest::new(Method::GET, URL.parse().unwrap()))
            .await
            .unwrap_err();
        assert!(matches!(&error, FetchError::Other(m) if m.contains("AWS_ACCESS_KEY_ID")));
        assert!(capture.0.lock().unwrap().is_none());
    }
}
