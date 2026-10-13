//! Downloads made by the user operations, as the AI SDK's `download`
//! options (ai@7.0.127 `util/download/`).
//!
//! - `transcribe` and `generate_video` take a [`Download`], which fetches
//!   one URL (`transcribe/transcribe.ts:56`, `:109-117`;
//!   `generate-video/generate-video.ts:136`, `:253-261`).
//! - `generate_text` and `stream_text` take a [`DownloadFunction`] for the
//!   file URLs of the prompt (`util/download/download-function.ts`;
//!   `prompt/convert-to-language-model-prompt.ts:36-55`).
//!
//! An operation given neither uses the guarded download ([`create_download`]):
//! every hop is validated and its connection pinned to the validated
//! addresses ([`crate::download_guard`]), and the provider's fetch and
//! credentials are never used for these URLs. A caller that needs another
//! transport (offline replay, a proxy, an authenticated store) passes its
//! own.
//!
//! The URLs of generated files in a text result are always fetched with the
//! guarded download: upstream's `resolveGeneratedFileData` takes no download
//! option either (`generate-text/resolve-generated-file-data.ts:14-41`).

use std::fmt;
use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;

use futures::StreamExt;
use futures::future::BoxFuture;

use crate::abort_signal::AbortSignal;
use crate::download_guard::{validate_download_target, validate_download_url};
use crate::error::AiMuxError;
use crate::shared::{FileBytes, GeneratedFileData};

const DEFAULT_MAX_DOWNLOAD_SIZE: usize = 2 * 1024 * 1024 * 1024;
const MAX_DOWNLOAD_REDIRECTS: usize = 10;

/// A downloaded file: its bytes and the media type the response reported
/// (`{ data, mediaType }`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadedFile {
    /// The file's bytes.
    pub data: Vec<u8>,
    /// The response's `Content-Type`, when it had one.
    pub media_type: Option<String>,
}

/// What a [`Download`] is asked to fetch (`{ url, abortSignal }`).
#[derive(Debug, Clone)]
pub struct DownloadRequest {
    /// The URL to fetch.
    pub url: String,
    /// The operation's abort signal.
    pub abort_signal: Option<AbortSignal>,
}

type DownloadCallback =
    dyn Fn(DownloadRequest) -> BoxFuture<'static, Result<DownloadedFile, AiMuxError>> + Send + Sync;

/// Fetches one URL: the `download` option of `transcribe` and
/// `generate_video` (ai@7.0.127 `transcribe/transcribe.ts:109-117`,
/// `generate-video/generate-video.ts:253-261`). [`create_download`] builds
/// the default one.
#[derive(Clone)]
pub struct Download(Arc<DownloadCallback>);

impl Download {
    /// Wrap a download callback.
    #[must_use]
    pub fn new<F, Fut>(download: F) -> Self
    where
        F: Fn(DownloadRequest) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<DownloadedFile, AiMuxError>> + Send + 'static,
    {
        Self(Arc::new(move |request| Box::pin(download(request))))
    }

    /// Fetch `url`.
    ///
    /// # Errors
    ///
    /// The callback's error.
    pub async fn download(
        &self,
        url: &str,
        abort_signal: Option<&AbortSignal>,
    ) -> Result<DownloadedFile, AiMuxError> {
        (self.0)(DownloadRequest {
            url: url.to_owned(),
            abort_signal: abort_signal.cloned(),
        })
        .await
    }
}

impl fmt::Debug for Download {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Download(<callback>)")
    }
}

/// The guarded download, the AI SDK's `createDownload({ maxBytes })`
/// (`util/download/create-download.ts`): it fails for a file larger than
/// `max_bytes` (default 2 GiB) and for a URL the download guard rejects.
#[must_use]
pub fn create_download(max_bytes: Option<usize>) -> Download {
    Download::new(move |request: DownloadRequest| async move {
        let (data, media_type) =
            download(&request.url, max_bytes, request.abort_signal.as_ref()).await?;
        Ok(DownloadedFile { data, media_type })
    })
}

/// A file URL of a prompt, as a [`DownloadFunction`] receives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedDownload {
    /// The file's URL.
    pub url: String,
    /// Whether the model fetches this URL itself: the part's media type and
    /// URL match the model's
    /// [`supported_urls`](crate::language_model::LanguageModel::supported_urls).
    pub is_url_supported_by_model: bool,
}

type DownloadFunctionCallback = dyn Fn(Vec<PlannedDownload>) -> BoxFuture<'static, Result<Vec<Option<DownloadedFile>>, AiMuxError>>
    + Send
    + Sync;

/// The prompt download of `generate_text` and `stream_text`: the AI SDK's
/// experimental `DownloadFunction` (`util/download/download-function.ts`),
/// given as `experimental_download`.
///
/// It is called once per model call with every file URL of the prompt, in
/// prompt order, and returns one entry per URL in the same order: the
/// downloaded file, or `None` to send the URL to the model as it is.
/// [`create_default_download_function`] builds the default one.
#[derive(Clone)]
pub struct DownloadFunction(Arc<DownloadFunctionCallback>);

impl DownloadFunction {
    /// Wrap a prompt download callback.
    #[must_use]
    pub fn new<F, Fut>(download: F) -> Self
    where
        F: Fn(Vec<PlannedDownload>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Vec<Option<DownloadedFile>>, AiMuxError>> + Send + 'static,
    {
        Self(Arc::new(move |downloads| Box::pin(download(downloads))))
    }

    /// Download the planned URLs.
    ///
    /// # Errors
    ///
    /// The callback's error, or `InvalidArgument` when it does not return
    /// one entry per URL.
    pub async fn download(
        &self,
        downloads: Vec<PlannedDownload>,
    ) -> Result<Vec<Option<DownloadedFile>>, AiMuxError> {
        let requested = downloads.len();
        let files = (self.0)(downloads).await?;
        if files.len() != requested {
            return Err(AiMuxError::InvalidArgument(format!(
                "the download function returned {} results for {requested} URLs",
                files.len()
            )));
        }
        Ok(files)
    }
}

impl fmt::Debug for DownloadFunction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DownloadFunction(<callback>)")
    }
}

/// The default prompt download, the AI SDK's
/// `createDefaultDownloadFunction(download, abortSignal)`
/// (`util/download/download-function.ts`): it leaves the URLs the model
/// fetches itself to the model and downloads the others concurrently with
/// `download` (default [`create_download`]`(None)`).
#[must_use]
pub fn create_default_download_function(
    download: Option<Download>,
    abort_signal: Option<AbortSignal>,
) -> DownloadFunction {
    let download = download.unwrap_or_else(|| create_download(None));
    DownloadFunction::new(move |downloads: Vec<PlannedDownload>| {
        let files = downloads.into_iter().map(|planned| {
            let download = download.clone();
            let abort_signal = abort_signal.clone();
            async move {
                if planned.is_url_supported_by_model {
                    Ok(None)
                } else {
                    download
                        .download(&planned.url, abort_signal.as_ref())
                        .await
                        .map(Some)
                }
            }
        });
        futures::future::try_join_all(files)
    })
}

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

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Mutex;

    fn recording_download(requests: Arc<Mutex<Vec<String>>>) -> Download {
        Download::new(move |request: DownloadRequest| {
            requests.lock().unwrap().push(request.url.clone());
            async move {
                Ok(DownloadedFile {
                    data: request.url.into_bytes(),
                    media_type: Some("image/png".into()),
                })
            }
        })
    }

    fn planned(url: &str, is_url_supported_by_model: bool) -> PlannedDownload {
        PlannedDownload {
            url: url.into(),
            is_url_supported_by_model,
        }
    }

    #[tokio::test]
    async fn the_default_function_downloads_only_the_urls_the_model_does_not_fetch() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let function =
            create_default_download_function(Some(recording_download(requests.clone())), None);
        let files = function
            .download(vec![
                planned("https://a.test/1.png", false),
                planned("https://a.test/2.png", true),
                planned("https://a.test/3.png", false),
            ])
            .await
            .unwrap();
        assert_eq!(
            files,
            [
                Some(DownloadedFile {
                    data: b"https://a.test/1.png".to_vec(),
                    media_type: Some("image/png".into()),
                }),
                None,
                Some(DownloadedFile {
                    data: b"https://a.test/3.png".to_vec(),
                    media_type: Some("image/png".into()),
                }),
            ]
        );
        assert_eq!(
            *requests.lock().unwrap(),
            ["https://a.test/1.png", "https://a.test/3.png"]
        );
    }

    #[tokio::test]
    async fn a_function_must_return_one_entry_per_url() {
        let function = DownloadFunction::new(|_| async { Ok(vec![None]) });
        let error = function
            .download(vec![
                planned("https://a.test/1", false),
                planned("https://a.test/2", false),
            ])
            .await
            .unwrap_err();
        assert!(
            matches!(&error, AiMuxError::InvalidArgument(message)
                if message == "the download function returned 1 results for 2 URLs"),
            "{error:?}"
        );
    }

    /// The default transport is the guarded download: a loopback URL is
    /// refused before any connection is made.
    #[tokio::test]
    async fn the_default_download_is_guarded() {
        let error = create_download(None)
            .download("http://127.0.0.1:9/file", None)
            .await
            .unwrap_err();
        assert!(
            matches!(&error, AiMuxError::InvalidArgument(message)
                if message == "download URL targets a non-public address: 127.0.0.1"),
            "{error:?}"
        );
    }
}
