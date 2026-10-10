//! Live SigV4 check against real AWS endpoints. Ignored by default; run with
//! real credentials in `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` (and
//! `AWS_SESSION_TOKEN` for temporary ones):
//!
//! ```sh
//! cargo test -p aimux-provider-utils --test live_sigv4_test -- --ignored --nocapture
//! ```
//!
//! STS `GetCallerIdentity` needs no IAM permission, so it must succeed. The
//! Bedrock and Polly calls may be denied by IAM or model access, but AWS
//! checks the signature first, so any response other than a signature error
//! means the signature was accepted.

use aimux_provider_utils::sigv4_fetch::{AwsCredentials, SigV4Fetch};
use aimux_provider_utils::{Fetch, FetchRequest, Resolvable, default_fetch};
use futures::StreamExt;
use http::Method;

const SIGNATURE_ERRORS: &[&str] = &[
    "SignatureDoesNotMatch",
    "IncompleteSignature",
    "InvalidSignatureException",
    "signature we calculated does not match",
];

async fn call(
    service: &str,
    method: Method,
    url: &str,
    body: &str,
    content_type: Option<&str>,
) -> (u16, String) {
    let credentials = AwsCredentials {
        access_key_id: std::env::var("AWS_ACCESS_KEY_ID").expect("AWS_ACCESS_KEY_ID"),
        secret_access_key: std::env::var("AWS_SECRET_ACCESS_KEY").expect("AWS_SECRET_ACCESS_KEY"),
        session_token: std::env::var("AWS_SESSION_TOKEN").ok(),
        region: "us-east-1".into(),
    };
    let fetch = SigV4Fetch::new(default_fetch(), Resolvable::from(credentials), service);
    let mut request = FetchRequest::new(method, url.parse().unwrap());
    if let Some(content_type) = content_type {
        request
            .headers
            .insert("content-type", content_type.parse().unwrap());
    }
    request.body = bytes::Bytes::from(body.to_string());
    let response = fetch.fetch(request).await.unwrap();
    let status = response.status().as_u16();
    let mut stream = response.bytes_stream();
    let mut buf = Vec::new();
    while let Some(chunk) = stream.next().await {
        buf.extend_from_slice(&chunk.unwrap());
    }
    let body = String::from_utf8_lossy(&buf).into_owned();
    println!("{service} {url} -> {status}");
    assert!(
        !SIGNATURE_ERRORS.iter().any(|error| body.contains(error)),
        "{service} rejected the signature: {body}"
    );
    (status, body)
}

#[tokio::test]
#[ignore = "calls AWS with real credentials"]
async fn aws_accepts_the_signature() {
    let (status, body) = call(
        "sts",
        Method::POST,
        "https://sts.us-east-1.amazonaws.com/",
        "Action=GetCallerIdentity&Version=2011-06-15",
        Some("application/x-www-form-urlencoded"),
    )
    .await;
    assert_eq!(status, 200, "sts: {body}");

    call(
        "bedrock",
        Method::GET,
        "https://bedrock.us-east-1.amazonaws.com/foundation-models?byProvider=anthropic",
        "",
        None,
    )
    .await;
    // `:` in the model id exercises path canonicalization.
    call(
        "bedrock",
        Method::POST,
        "https://bedrock-runtime.us-east-1.amazonaws.com/model/anthropic.claude-3-haiku-20240307-v1:0/converse",
        r#"{"messages":[{"role":"user","content":[{"text":"hi"}]}],"inferenceConfig":{"maxTokens":5}}"#,
        Some("application/json"),
    )
    .await;
    call(
        "polly",
        Method::GET,
        "https://polly.us-east-1.amazonaws.com/v1/voices?LanguageCode=en-US",
        "",
        None,
    )
    .await;
}
