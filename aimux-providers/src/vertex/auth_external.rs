//! Subject-token sources for Google workload identity federation.

use std::collections::BTreeMap;

use aimux_core::AiMuxError;
use aimux_provider_utils::{FetchFunction, FetchRequest, default_fetch};
use futures::TryStreamExt;
use serde_json::{Value, json};

use super::auth::required;

pub(super) async fn text(
    fetch: Option<FetchFunction>,
    url: &str,
    method: reqwest::Method,
    headers: &[(String, String)],
) -> Result<String, AiMuxError> {
    let mut request = FetchRequest::new(
        method,
        url.parse().map_err(|error| {
            AiMuxError::InvalidArgument(format!("Invalid Google credential URL: {error}"))
        })?,
    );
    for (name, value) in headers {
        request.headers.insert(
            reqwest::header::HeaderName::from_bytes(name.as_bytes())
                .map_err(|error| AiMuxError::InvalidArgument(error.to_string()))?,
            value
                .parse()
                .map_err(|error: reqwest::header::InvalidHeaderValue| {
                    AiMuxError::InvalidArgument(error.to_string())
                })?,
        );
    }
    let response = fetch
        .unwrap_or_else(default_fetch)
        .fetch(request)
        .await
        .map_err(|error| {
            AiMuxError::InvalidResponseData(format!(
                "Google credential source request failed: {error}"
            ))
        })?;
    if !response.status.is_success() {
        return Err(AiMuxError::InvalidResponseData(format!(
            "Google credential source returned HTTP {}",
            response.status
        )));
    }
    let bytes = response
        .body
        .try_fold(Vec::new(), |mut bytes, chunk| async move {
            bytes.extend_from_slice(&chunk);
            Ok(bytes)
        })
        .await
        .map_err(|error| AiMuxError::InvalidResponseData(error.to_string()))?;
    String::from_utf8(bytes).map_err(|error| AiMuxError::InvalidResponseData(error.to_string()))
}

pub(super) async fn aws(
    credentials: &Value,
    fetch: Option<FetchFunction>,
) -> Result<String, AiMuxError> {
    let source = &credentials["credential_source"];
    if required(source, "environment_id")? != "aws1" {
        return Err(AiMuxError::InvalidArgument(
            "Unsupported Google external AWS environment version".into(),
        ));
    }
    let region = std::env::var("AWS_REGION")
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| {
            std::env::var("AWS_DEFAULT_REGION")
                .ok()
                .filter(|value| !value.is_empty())
        });
    let access = std::env::var("AWS_ACCESS_KEY_ID")
        .ok()
        .filter(|value| !value.is_empty());
    let secret = std::env::var("AWS_SECRET_ACCESS_KEY")
        .ok()
        .filter(|value| !value.is_empty());
    let mut metadata_headers = Vec::new();
    if (region.is_none() || access.is_none() || secret.is_none())
        && let Some(url) = source
            .get("imdsv2_session_token_url")
            .and_then(Value::as_str)
    {
        let token = text(
            fetch.clone(),
            url,
            reqwest::Method::PUT,
            &[("x-aws-ec2-metadata-token-ttl-seconds".into(), "300".into())],
        )
        .await?;
        metadata_headers.push(("x-aws-ec2-metadata-token".into(), token));
    }
    let region = match region {
        Some(region) => region,
        None => {
            let mut zone = text(
                fetch.clone(),
                required(source, "region_url")?,
                reqwest::Method::GET,
                &metadata_headers,
            )
            .await?;
            zone.pop();
            zone
        }
    };
    let (access, secret, token) = match (access, secret) {
        (Some(access), Some(secret)) => (access, secret, std::env::var("AWS_SESSION_TOKEN").ok()),
        _ => {
            let url = required(source, "url")?;
            let role = text(fetch.clone(), url, reqwest::Method::GET, &metadata_headers).await?;
            let value: Value = serde_json::from_str(
                &text(
                    fetch,
                    &format!("{url}/{role}"),
                    reqwest::Method::GET,
                    &metadata_headers,
                )
                .await?,
            )
            .map_err(|error| AiMuxError::InvalidResponseData(error.to_string()))?;
            (
                required(&value, "AccessKeyId")?.into(),
                required(&value, "SecretAccessKey")?.into(),
                value
                    .get("Token")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            )
        }
    };
    let url = required(source, "regional_cred_verification_url")?.replace("{region}", &region);
    let parsed =
        url::Url::parse(&url).map_err(|error| AiMuxError::InvalidArgument(error.to_string()))?;
    let host = parsed.host_str().ok_or_else(|| {
        AiMuxError::InvalidArgument("AWS credential verification URL has no host".into())
    })?;
    let host = parsed
        .port()
        .map_or_else(|| host.into(), |port| format!("{host}:{port}"));
    let now = chrono::Utc::now();
    let date = now.format("%Y%m%d").to_string();
    let time = now.format("%Y%m%dT%H%M%SZ").to_string();
    let mut headers = BTreeMap::from([("host", host.clone()), ("x-amz-date", time.clone())]);
    if let Some(token) = token.filter(|value| !value.is_empty()) {
        headers.insert("x-amz-security-token", token);
    }
    let names = headers.keys().copied().collect::<Vec<_>>().join(";");
    let canonical_headers = headers
        .iter()
        .map(|(key, value)| format!("{key}:{value}\n"))
        .collect::<String>();
    let canonical = format!(
        "POST\n{}\n{}\n{canonical_headers}\n{names}\n{}",
        parsed.path(),
        parsed.query().unwrap_or(""),
        sha256(b"")
    );
    let service = host.split('.').next().unwrap_or("sts");
    let scope = format!("{date}/{region}/{service}/aws4_request");
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{time}\n{scope}\n{}",
        sha256(canonical.as_bytes())
    );
    let key = hmac(format!("AWS4{secret}").as_bytes(), &date);
    let key = hmac(&key, &region);
    let key = hmac(&key, service);
    let key = hmac(&key, "aws4_request");
    let signature = hex(&hmac(&key, &string_to_sign));
    headers.insert("authorization", format!("AWS4-HMAC-SHA256 Credential={access}/{scope}, SignedHeaders={names}, Signature={signature}"));
    headers.insert(
        "x-goog-cloud-target-resource",
        required(credentials, "audience")?.into(),
    );
    let headers = headers
        .into_iter()
        .map(|(key, value)| json!({"key": key, "value": value}))
        .collect::<Vec<_>>();
    let token = json!({"url": url, "method": "POST", "headers": headers}).to_string();
    Ok(
        percent_encoding::utf8_percent_encode(&token, percent_encoding::NON_ALPHANUMERIC)
            .to_string()
            .replace("%2D", "-")
            .replace("%5F", "_")
            .replace("%2E", ".")
            .replace("%7E", "~")
            .replace("%21", "!")
            .replace("%27", "'")
            .replace("%28", "(")
            .replace("%29", ")")
            .replace("%2A", "*"),
    )
}

fn hmac(key: &[u8], message: &str) -> Vec<u8> {
    ring::hmac::sign(
        &ring::hmac::Key::new(ring::hmac::HMAC_SHA256, key),
        message.as_bytes(),
    )
    .as_ref()
    .to_vec()
}
fn sha256(value: &[u8]) -> String {
    hex(ring::digest::digest(&ring::digest::SHA256, value).as_ref())
}
fn hex(value: &[u8]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(super) async fn executable(credentials: &Value) -> Result<String, AiMuxError> {
    if std::env::var("GOOGLE_EXTERNAL_ACCOUNT_ALLOW_EXECUTABLES").as_deref() != Ok("1") {
        return Err(AiMuxError::InvalidArgument("Google external account executables require GOOGLE_EXTERNAL_ACCOUNT_ALLOW_EXECUTABLES=1".into()));
    }
    let options = &credentials["credential_source"]["executable"];
    let output_file = options
        .get("output_file")
        .and_then(Value::as_str)
        .filter(|path| !path.is_empty());
    if let Some(path) = output_file
        && let Ok(output) = std::fs::read_to_string(path)
        && !output.is_empty()
    {
        let value: Value = serde_json::from_str(&output).map_err(|error| {
            AiMuxError::InvalidResponseData(format!("Invalid cached executable response: {error}"))
        })?;
        validate_executable_response(&value)?;
        if value.get("success").and_then(Value::as_bool) == Some(true)
            && value
                .get("expiration_time")
                .and_then(Value::as_i64)
                .is_none_or(|expiry| expiry >= (chrono::Utc::now().timestamp_millis() + 500) / 1000)
        {
            return executable_response(&value, true);
        }
    }
    let timeout = options
        .get("timeout_millis")
        .and_then(Value::as_u64)
        .unwrap_or(30_000);
    if !(5_000..=120_000).contains(&timeout) {
        return Err(AiMuxError::InvalidArgument(
            "Google credential executable timeout must be between 5000 and 120000 milliseconds"
                .into(),
        ));
    }
    let command = required(options, "command")?;
    let pattern = regex::Regex::new(r#"(?:[^\s"]+|"[^"]*")+"#).expect("static command pattern");
    let components = pattern
        .find_iter(command)
        .map(|part| {
            let part = part.as_str();
            part.strip_prefix('"')
                .and_then(|part| part.strip_suffix('"'))
                .unwrap_or(part)
        })
        .collect::<Vec<_>>();
    let program = components.first().ok_or_else(|| {
        AiMuxError::InvalidArgument("Google credential executable command is empty".into())
    })?;
    let mut command = tokio::process::Command::new(program);
    command
        .args(&components[1..])
        .kill_on_drop(true)
        .env(
            "GOOGLE_EXTERNAL_ACCOUNT_AUDIENCE",
            required(credentials, "audience")?,
        )
        .env(
            "GOOGLE_EXTERNAL_ACCOUNT_TOKEN_TYPE",
            required(credentials, "subject_token_type")?,
        )
        .env("GOOGLE_EXTERNAL_ACCOUNT_INTERACTIVE", "0");
    if let Some(path) = output_file {
        command.env("GOOGLE_EXTERNAL_ACCOUNT_OUTPUT_FILE", path);
    }
    if let Some(url) = credentials
        .get("service_account_impersonation_url")
        .and_then(Value::as_str)
        && let Some(email) = url
            .rsplit('/')
            .next()
            .and_then(|target| target.strip_suffix(":generateAccessToken"))
    {
        command.env("GOOGLE_EXTERNAL_ACCOUNT_IMPERSONATED_EMAIL", email);
    }
    let output = tokio::time::timeout(std::time::Duration::from_millis(timeout), command.output())
        .await
        .map_err(|_| {
            AiMuxError::InvalidResponseData("Google credential executable timed out".into())
        })?
        .map_err(|error| {
            AiMuxError::InvalidResponseData(format!("Google credential executable failed: {error}"))
        })?;
    if !output.status.success() {
        return Err(AiMuxError::InvalidResponseData(format!(
            "Google credential executable failed with {}",
            output.status
        )));
    }
    let mut bytes = output.stdout;
    bytes.extend(output.stderr);
    let response: Value = serde_json::from_slice(&bytes).map_err(|error| {
        AiMuxError::InvalidResponseData(format!("Invalid credential executable response: {error}"))
    })?;
    executable_response(&response, output_file.is_some())
}

fn executable_response(response: &Value, output_file: bool) -> Result<String, AiMuxError> {
    validate_executable_response(response)?;
    if response
        .get("version")
        .and_then(Value::as_u64)
        .is_some_and(|version| version > 1)
    {
        return Err(AiMuxError::InvalidResponseData(
            "Google credential executable response requires version 1".into(),
        ));
    }
    if response.get("success").and_then(Value::as_bool) != Some(true) {
        return Err(AiMuxError::InvalidResponseData(format!(
            "Google credential executable returned {}: {}",
            response["code"], response["message"]
        )));
    }
    let expiry = response.get("expiration_time").and_then(Value::as_i64);
    if (output_file && expiry.is_none())
        || expiry
            .is_some_and(|expiry| expiry < (chrono::Utc::now().timestamp_millis() + 500) / 1000)
    {
        return Err(AiMuxError::InvalidResponseData(
            "Google credential executable response has missing or expired expiration_time".into(),
        ));
    }
    let field = match required(response, "token_type")? {
        "urn:ietf:params:oauth:token-type:saml2" => "saml_response",
        "urn:ietf:params:oauth:token-type:id_token" | "urn:ietf:params:oauth:token-type:jwt" => {
            "id_token"
        }
        _ => {
            return Err(AiMuxError::InvalidResponseData(
                "Unsupported Google credential executable token_type".into(),
            ));
        }
    };
    Ok(required(response, field)?.into())
}

fn validate_executable_response(response: &Value) -> Result<(), AiMuxError> {
    if response
        .get("version")
        .and_then(Value::as_u64)
        .is_none_or(|version| version == 0)
        || response.get("success").and_then(Value::as_bool).is_none()
    {
        return Err(AiMuxError::InvalidResponseData(
            "Google credential executable response requires version and success".into(),
        ));
    }
    if response.get("success").and_then(Value::as_bool) == Some(false) {
        required(response, "code")?;
        required(response, "message")?;
    } else {
        let field = match required(response, "token_type")? {
            "urn:ietf:params:oauth:token-type:saml2" => "saml_response",
            "urn:ietf:params:oauth:token-type:id_token"
            | "urn:ietf:params:oauth:token-type:jwt" => "id_token",
            _ => {
                return Err(AiMuxError::InvalidResponseData(
                    "Unsupported Google credential executable token_type".into(),
                ));
            }
        };
        required(response, field)?;
    }
    Ok(())
}
