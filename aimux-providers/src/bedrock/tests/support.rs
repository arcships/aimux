//! Helpers shared by the ported tests.
//!
//! [`inject_fetch_headers`] is the Rust form of upstream's
//! `src/inject-fetch-headers.ts`, a test helper of `@ai-sdk/amazon-bedrock`
//! (not exported): a fetch that merges fixed headers over the request's,
//! appends the `ai-sdk-amazon-bedrock/<v>` user-agent part and calls the
//! wrapped transport, which stands in for `globalThis.fetch`. The embedding,
//! image and reranking ports use it the way their upstream tests do.

use std::sync::Arc;

use async_trait::async_trait;
use reqwest::header::{HeaderName, HeaderValue};

use aimux_provider_utils::{Fetch, FetchError, FetchFunction, FetchRequest, FetchResponse};

/// The `ai-sdk-amazon-bedrock/<v>` user-agent part, pinned.
pub(super) const BEDROCK_USER_AGENT_PART: &str = "ai-sdk-amazon-bedrock/5.0.100";

/// `injectFetchHeaders(customHeaders)` over `inner`.
struct InjectFetchHeaders {
    custom_headers: Vec<(HeaderName, HeaderValue)>,
    inner: FetchFunction,
}

#[async_trait]
impl Fetch for InjectFetchHeaders {
    async fn fetch(&self, mut request: FetchRequest) -> Result<FetchResponse, FetchError> {
        for (name, value) in &self.custom_headers {
            request.headers.insert(name.clone(), value.clone());
        }
        aimux_provider_utils::transport_user_agent(
            &mut request.headers,
            None,
            Some(BEDROCK_USER_AGENT_PART),
        );
        self.inner.fetch(request).await
    }
}

/// A transport that adds `custom_headers` (and the package user-agent part)
/// to every request and forwards it to `inner`.
pub(super) fn inject_fetch_headers(
    custom_headers: &[(&str, &str)],
    inner: FetchFunction,
) -> FetchFunction {
    Arc::new(InjectFetchHeaders {
        custom_headers: custom_headers
            .iter()
            .map(|(name, value)| {
                (
                    HeaderName::from_bytes(name.as_bytes()).expect("valid header name"),
                    HeaderValue::from_str(value).expect("valid header value"),
                )
            })
            .collect(),
        inner,
    })
}
