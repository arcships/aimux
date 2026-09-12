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
    let now = chrono::Utc::now();
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
}
