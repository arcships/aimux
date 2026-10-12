//! Simplified AWS Signature Version 4 (SigV4) signer.
//!
//! Implements the core SigV4 signing algorithm for authenticating requests to
//! AWS services (Amazon Bedrock, Claude Platform on AWS). The signer produces
//! the `Authorization` header and supporting `X-Amz-*` headers that AWS
//! expects.
//!
//! Reference: <https://docs.aws.amazon.com/IAM/latest/UserGuide/create-signed-request.html>
//!
//! This is a self-contained implementation using only `sha2` + `hmac` (no AWS
//! SDK dependency). It supports:
//! - Static credentials (`access_key_id` + `secret_access_key` + optional
//!   `session_token`).
//! - Any AWS service name and region.
//! - POST requests with a JSON body.

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

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

impl std::fmt::Debug for AwsCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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
pub struct SignedRequest {
    /// All headers to send with the request, including the `Authorization`
    /// header and any `X-Amz-*` headers.
    pub headers: Vec<(String, String)>,
}

type HmacSha256 = Hmac<Sha256>;

/// Sign an HTTP request using AWS SigV4.
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
        body,
        extra_headers,
        chrono::Utc::now(),
    )
}

/// [`sign_request`] with an explicit signing time.
fn sign_request_at(
    credentials: &AwsCredentials,
    service: &str,
    method: &str,
    url: &str,
    body: &str,
    extra_headers: &[(String, String)],
    now: chrono::DateTime<chrono::Utc>,
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
    let body_hash = hex::encode(Sha256::digest(body.as_bytes()));

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

#[cfg(test)]
mod tests {
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
        assert!(printed.contains("us-east-1"));
    }

    fn host_header(signed: &SignedRequest) -> String {
        signed
            .headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("host"))
            .map(|(_, v)| v.clone())
            .expect("signed request carries a host header")
    }

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
        assert_eq!(host_header(&signed), "127.0.0.1:54321");
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
            host_header(&signed),
            "bedrock-runtime.us-east-1.amazonaws.com"
        );
    }

    fn fixed_time() -> chrono::DateTime<chrono::Utc> {
        use chrono::TimeZone;
        chrono::Utc.with_ymd_and_hms(2024, 1, 2, 3, 4, 5).unwrap()
    }

    fn signature(signed: &SignedRequest) -> String {
        signed
            .headers
            .iter()
            .find(|(k, _)| k == "Authorization")
            .and_then(|(_, v)| v.rsplit_once("Signature="))
            .expect("signed request carries a signature")
            .1
            .to_string()
    }

    /// Expected values come from botocore 1.43's `SigV4Auth` (AWS's own
    /// Python signer) for the same credentials, headers, body and fixed
    /// time, not from this implementation.
    #[test]
    fn signatures_match_botocore() {
        let signed = sign_request_at(
            &creds(),
            "bedrock",
            "POST",
            "https://bedrock-runtime.us-east-1.amazonaws.com/model/anthropic.claude-3-5-sonnet-20240620-v1:0/converse",
            r#"{"messages":[]}"#,
            &[("X-Custom".to_string(), "  value ".to_string())],
            fixed_time(),
        );
        assert_eq!(
            signature(&signed),
            "538444fb4b17f3cdd777bbd5f1cd4e3e1f53d6ac97167da96c81ec8d14d6468a"
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
            "{}",
            &[],
            fixed_time(),
        );
        assert_eq!(
            signature(&signed),
            "dd6a2384cb9c8f0db8ea8ee383a61bf06dd85da679cec54d5c944b1b0cb8d9f6"
        );

        let signed = sign_request_at(
            &creds(),
            "bedrock",
            "POST",
            "https://bedrock-runtime.us-east-1.amazonaws.com/model/us.anthropic.claude-3-5-sonnet-20240620-v1%3A0/converse-stream",
            "{}",
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
            "{}",
            &[],
            fixed_time(),
        );
        assert_eq!(
            signature(&signed),
            "70c6e5794c4f1456f33f2b41cebc483ee0d19c98b95660411f4b51ccac464006"
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
}
