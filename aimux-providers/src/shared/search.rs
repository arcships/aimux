//! Shared search exchange, execution, and result-building helpers.

use aimux_core::ErrorCause;
use aimux_core::error::AiMuxError;
use aimux_core::search_model::{SearchCallOptions, SearchResponse, SearchResult, SearchResultItem};
use aimux_core::shared::SharedHeaders;
use aimux_provider_utils::{
    ResponseHandler, create_json_response_handler, get_from_api, post_json_to_api,
};
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::EndpointConfig;
use super::exchange::Exchange;

/// Build a [`SearchResult`] from mapped items, optional answer, and captured HTTP metadata.
fn search_result(
    results: Vec<SearchResultItem>,
    answer: Option<String>,
    response_headers: SharedHeaders,
    response_body: Option<Value>,
) -> SearchResult {
    SearchResult {
        results,
        answer,
        provider_metadata: None,
        warnings: Vec::new(),
        response: Some(SearchResponse {
            headers: Some(response_headers),
            body: response_body,
        }),
    }
}

/// Resolve `config`, send a JSON `POST` search request to `path`, and map the response.
pub(crate) async fn post_json_search<R, F>(
    config: &EndpointConfig,
    path: &str,
    options: &SearchCallOptions,
    body: Value,
    failed_response_handler: ResponseHandler<AiMuxError>,
    map_response: F,
) -> Result<SearchResult, AiMuxError>
where
    R: DeserializeOwned + Send + 'static,
    F: FnOnce(R) -> (Vec<SearchResultItem>, Option<String>),
{
    let exchange = config.exchange(options.headers.as_ref()).await?;
    let resp = post_json_to_api(
        exchange.request(exchange.url(path), options),
        body,
        create_json_response_handler::<R>(),
        failed_response_handler,
    )
    .await?;
    let (results, answer) = map_response(resp.value);
    Ok(search_result(
        results,
        answer,
        resp.response_headers,
        resp.raw_value,
    ))
}

/// Send a `GET` query search using an already-resolved [`Exchange`] and map the response.
pub(crate) async fn get_query_search_with_exchange<R, B, F>(
    exchange: &Exchange,
    path: Option<&str>,
    provider_label: &str,
    options: &SearchCallOptions,
    build_query: B,
    failed_response_handler: ResponseHandler<AiMuxError>,
    map_response: F,
) -> Result<SearchResult, AiMuxError>
where
    R: DeserializeOwned + Send + 'static,
    B: FnOnce(&mut url::Url),
    F: FnOnce(R) -> (Vec<SearchResultItem>, Option<String>),
{
    let raw_url = match path {
        Some(p) => exchange.url(p),
        None => exchange.base_url().to_string(),
    };
    let mut url = url::Url::parse(&raw_url).map_err(|e| AiMuxError::InvalidArgument {
        message: format!("invalid {provider_label} endpoint: {e}"),
        cause: Some(ErrorCause::source(&e)),
    })?;
    build_query(&mut url);
    let resp = get_from_api(
        exchange.request(url.to_string(), options),
        create_json_response_handler::<R>(),
        failed_response_handler,
    )
    .await?;
    let (results, answer) = map_response(resp.value);
    Ok(search_result(
        results,
        answer,
        resp.response_headers,
        resp.raw_value,
    ))
}

/// Resolve `config` and run [`get_query_search_with_exchange`].
pub(crate) async fn get_query_search<R, B, F>(
    config: &EndpointConfig,
    path: Option<&str>,
    options: &SearchCallOptions,
    build_query: B,
    failed_response_handler: ResponseHandler<AiMuxError>,
    map_response: F,
) -> Result<SearchResult, AiMuxError>
where
    R: DeserializeOwned + Send + 'static,
    B: FnOnce(&mut url::Url),
    F: FnOnce(R) -> (Vec<SearchResultItem>, Option<String>),
{
    let exchange = config.exchange(options.headers.as_ref()).await?;
    get_query_search_with_exchange(
        &exchange,
        path,
        &config.provider,
        options,
        build_query,
        failed_response_handler,
        map_response,
    )
    .await
}
