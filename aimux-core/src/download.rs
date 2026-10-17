//! Resolve generated file URLs before exposing their file data.

use std::net::SocketAddr;

use futures::StreamExt;

use crate::abort_signal::AbortSignal;
use crate::download_guard::{validate_download_target, validate_download_url};
use crate::error::AiMuxError;
use crate::shared::{FileBytes, GeneratedFileData};

const DEFAULT_MAX_DOWNLOAD_SIZE: usize = 2 * 1024 * 1024 * 1024;
const MAX_DOWNLOAD_REDIRECTS: usize = 10;

pub(crate) async fn resolve_generated_file_data(
    data: &GeneratedFileData,
    abort_signal: Option<&AbortSignal>,
) -> Result<FileBytes, AiMuxError> {
    match data {
        GeneratedFileData::Data { data } => Ok(data.clone()),
        GeneratedFileData::Url { url, .. } => {
            let (data, _) = download(url, None, abort_signal).await?;
            Ok(FileBytes::Binary(data))
        }
    }
}

async fn download(
    url: &str,
    max_bytes: Option<usize>,
    abort_signal: Option<&AbortSignal>,
) -> Result<(Vec<u8>, Option<String>), AiMuxError> {
    let operation = download_inner(url, max_bytes.unwrap_or(DEFAULT_MAX_DOWNLOAD_SIZE));
    if let Some(signal) = abort_signal {
        tokio::select! {
            biased;
            () = signal.cancelled() => Err(AiMuxError::from_abort_signal(signal)),
            result = operation => result,
        }
    } else {
        operation.await
    }
}

async fn download_inner(
    url: &str,
    max_bytes: usize,
) -> Result<(Vec<u8>, Option<String>), AiMuxError> {
    let mut current = url.to_owned();
    for redirects in 0..=MAX_DOWNLOAD_REDIRECTS {
        let parsed = validate_download_url(&current)?;
        let addresses = validate_download_target(&current, None).await?;
        let host = parsed
            .host_str()
            .ok_or_else(|| AiMuxError::InvalidArgument("download URL has no host".into()))?;
        let port = parsed
            .port_or_known_default()
            .ok_or_else(|| AiMuxError::InvalidArgument("download URL has no known port".into()))?;
        let addresses: Vec<_> = addresses
            .into_iter()
            .map(|address| SocketAddr::new(address, port))
            .collect();
        // Direct connections use only the addresses validated above. An
        // automatic proxy or redirect must not bypass the address policy.
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .resolve_to_addrs(host, &addresses)
            .build()
            .map_err(download_error)?;
        let response = client
            .get(parsed.clone())
            .header(
                reqwest::header::USER_AGENT,
                concat!("aimux/", env!("CARGO_PKG_VERSION")),
            )
            .send()
            .await
            .map_err(download_error)?;
        let status = response.status();
        if matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308)
            && let Some(location) = response.headers().get(reqwest::header::LOCATION)
        {
            if redirects == MAX_DOWNLOAD_REDIRECTS {
                return Err(AiMuxError::Other("download exceeded 10 redirects".into()));
            }
            let location = location.to_str().map_err(download_error)?;
            current = parsed.join(location).map_err(download_error)?.to_string();
            drop(response);
            continue;
        }
        if !status.is_success() {
            return Err(AiMuxError::Other(format!(
                "download failed with HTTP status {status}"
            )));
        }
        if response
            .content_length()
            .is_some_and(|length| length > max_bytes as u64)
        {
            return Err(size_error(max_bytes));
        }
        let media_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let mut stream = response.bytes_stream();
        let mut data = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(download_error)?;
            if chunk.len() > max_bytes.saturating_sub(data.len()) {
                return Err(size_error(max_bytes));
            }
            data.extend_from_slice(&chunk);
        }
        return Ok((data, media_type));
    }
    unreachable!("download loop returns a response or redirect error")
}

fn download_error(error: impl std::fmt::Display) -> AiMuxError {
    AiMuxError::Other(format!("download failed: {error}"))
}

fn size_error(max_bytes: usize) -> AiMuxError {
    AiMuxError::Other(format!(
        "download exceeded maximum size of {max_bytes} bytes"
    ))
}
