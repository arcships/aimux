//! Certificate subject tokens and their native TLS transport.

use std::path::PathBuf;
use std::sync::Arc;

use aimux_core::AiMuxError;
use aimux_provider_utils::{Fetch, FetchError, FetchFunction, FetchRequest, FetchResponse};
use base64::Engine;
use futures::StreamExt;
use serde_json::Value;

fn invalid(message: impl std::fmt::Display) -> AiMuxError {
    AiMuxError::InvalidArgument(message.to_string())
}

fn read(path: &str) -> Result<Vec<u8>, AiMuxError> {
    std::fs::read(path)
        .map_err(|error| invalid(format!("Cannot read certificate source {path}: {error}")))
}

fn nonempty(value: Option<&str>) -> Option<&str> {
    value.filter(|value| !value.is_empty())
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

pub(super) fn certificate(
    options: &Value,
    fetch: Option<FetchFunction>,
) -> Result<(String, Option<FetchFunction>), AiMuxError> {
    let use_default = options["use_default_certificate_config"].as_bool() == Some(true);
    let location = nonempty(options["certificate_config_location"].as_str());
    if use_default == location.is_some() {
        return Err(invalid(
            "Provide either use_default_certificate_config or certificate_config_location",
        ));
    }
    let path = location
        .map(PathBuf::from)
        .or_else(|| env("GOOGLE_API_CERTIFICATE_CONFIG").map(PathBuf::from))
        .unwrap_or_else(|| {
            let directory = env("CLOUDSDK_CONFIG")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    if cfg!(windows) {
                        PathBuf::from(env("APPDATA").unwrap_or_default()).join("gcloud")
                    } else {
                        PathBuf::from(env("HOME").unwrap_or_default()).join(".config/gcloud")
                    }
                });
            directory.join("certificate_config.json")
        });
    let config: Value = serde_json::from_slice(&read(&path.to_string_lossy())?).map_err(invalid)?;
    let workload = &config["cert_configs"]["workload"];
    let cert_path = nonempty(workload["cert_path"].as_str())
        .ok_or_else(|| invalid("Missing workload cert_path"))?;
    let key_path = nonempty(workload["key_path"].as_str())
        .ok_or_else(|| invalid("Missing workload key_path"))?;
    let cert = read(cert_path)?;
    let key = read(key_path)?;
    let certs = pem_certificates(&cert)?;
    let leaf = certs.first().cloned().unwrap_or_else(|| cert.clone());
    let mut identity_pem = if !certs.is_empty() {
        cert
    } else {
        format!(
            "-----BEGIN CERTIFICATE-----\n{}\n-----END CERTIFICATE-----\n",
            base64::engine::general_purpose::STANDARD.encode(&leaf)
        )
        .into_bytes()
    };
    identity_pem.push(b'\n');
    identity_pem.extend(key);
    let identity = reqwest::Identity::from_pem(&identity_pem).map_err(invalid)?;
    let mut chain = match nonempty(options["trust_chain_path"].as_str()) {
        Some(path) => pem_certificates(&read(path)?)?,
        None => Vec::new(),
    };
    match chain.iter().position(|cert| cert == &leaf) {
        Some(0) => {}
        Some(index) => {
            return Err(invalid(format!(
                "Leaf certificate exists in trust chain at index {index}, rather than first"
            )));
        }
        None => chain.insert(0, leaf),
    }
    // Root-store parsing validates DER without changing the mTLS server trust.
    let mut validation = reqwest::Client::builder();
    for der in &chain {
        validation =
            validation.add_root_certificate(reqwest::Certificate::from_der(der).map_err(invalid)?);
    }
    validation.build().map_err(invalid)?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .identity(identity)
        .build()
        .map_err(invalid)?;
    let subject_token = serde_json::to_string(
        &chain
            .iter()
            .map(|der| base64::engine::general_purpose::STANDARD.encode(der))
            .collect::<Vec<_>>(),
    )
    .map_err(invalid)?;
    Ok((
        subject_token,
        Some(fetch.unwrap_or_else(|| Arc::new(CertificateFetch(client)))),
    ))
}

fn pem_certificates(bytes: &[u8]) -> Result<Vec<Vec<u8>>, AiMuxError> {
    let text = String::from_utf8_lossy(bytes);
    let pattern = regex::Regex::new(r"-----BEGIN CERTIFICATE-----[^-]+-----END CERTIFICATE-----")
        .map_err(invalid)?;
    pattern
        .find_iter(&text)
        .map(|block| {
            let encoded: String = block
                .as_str()
                .trim_start_matches("-----BEGIN CERTIFICATE-----")
                .trim_end_matches("-----END CERTIFICATE-----")
                .chars()
                .filter(|character| !character.is_whitespace())
                .collect();
            base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(invalid)
        })
        .collect()
}

pub(super) fn ca_fetch(path: &str) -> Result<FetchFunction, AiMuxError> {
    let certificates = reqwest::Certificate::from_pem_bundle(&read(path)?).map_err(invalid)?;
    let mut builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .tls_built_in_root_certs(false);
    for certificate in certificates {
        builder = builder.add_root_certificate(certificate);
    }
    Ok(Arc::new(CertificateFetch(
        builder.build().map_err(invalid)?,
    )))
}

struct CertificateFetch(reqwest::Client);

#[async_trait::async_trait]
impl Fetch for CertificateFetch {
    async fn fetch(&self, request: FetchRequest) -> Result<FetchResponse, FetchError> {
        let signal = request.signal;
        let mut builder = self
            .0
            .request(request.method, request.url)
            .headers(request.headers)
            .body(request.body);
        if let Some(timeout) = request.timeout {
            builder = builder.timeout(timeout);
        }
        let response = match signal {
            Some(signal) => tokio::select! {
                biased;
                () = signal.cancelled() => return Err(FetchError::Aborted),
                response = builder.send() => response,
            },
            None => builder.send().await,
        }
        .map_err(fetch_error)?;
        Ok(FetchResponse {
            status: response.status(),
            headers: response.headers().clone(),
            url: response.url().clone(),
            body: response
                .bytes_stream()
                .map(|chunk| chunk.map_err(fetch_error))
                .boxed(),
        })
    }
}

fn fetch_error(error: reqwest::Error) -> FetchError {
    if error.is_builder() {
        FetchError::Other(error.to_string())
    } else if error.is_timeout() {
        FetchError::Timeout
    } else if error.is_connect() {
        FetchError::Connect(error.to_string())
    } else {
        FetchError::Io(error.to_string())
    }
}
