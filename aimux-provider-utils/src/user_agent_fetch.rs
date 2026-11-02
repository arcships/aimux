//! The user-agent edits a transport makes after the request helpers have
//! built the header (RFC-0046 §3.2).

use crate::FetchFunction;

/// A transport decorator that edits the request's `user-agent` just before
/// forwarding it, after the helper layer has added
/// `ai-sdk-provider-utils/<v>`: it names `provider_utils` in that part (when
/// set) and appends `suffix` (when set). The suffix is the AI SDK AWS
/// transports' own `withUserAgentSuffix` (amazon-bedrock-sigv4-fetch.ts:42-46
/// and :126-130, anthropic-aws-fetch.ts:42-46 and :126-130); the runtime
/// fragment they append is left out, as everywhere in aimux.
pub struct UserAgentFetch {
    /// `None` is the process default transport, resolved per request.
    pub inner: Option<FetchFunction>,
    pub provider_utils: Option<&'static str>,
    pub suffix: Option<String>,
}

#[async_trait::async_trait]
impl crate::Fetch for UserAgentFetch {
    async fn fetch(
        &self,
        mut request: crate::FetchRequest,
    ) -> Result<crate::FetchResponse, crate::FetchError> {
        transport_user_agent(
            &mut request.headers,
            self.provider_utils,
            self.suffix.as_deref(),
        );
        let inner = self.inner.clone().unwrap_or_else(crate::default_fetch);
        inner.fetch(request).await
    }
}

/// Rewrite the last `ai-sdk-provider-utils/<v>` part of the `user-agent` to
/// `provider_utils` (when set), then append `suffix` (when set, creating the
/// header if absent). A value that is not a valid header is left as it was.
pub fn transport_user_agent(
    headers: &mut reqwest::header::HeaderMap,
    provider_utils: Option<&str>,
    suffix: Option<&str>,
) {
    let current = headers
        .get(reqwest::header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let mut parts: Vec<String> = current
        .split(' ')
        .filter(|part| !part.is_empty())
        .map(String::from)
        .collect();
    let last_utils = parts
        .iter_mut()
        .rev()
        .find(|part| part.starts_with("ai-sdk-provider-utils/"));
    match (provider_utils, last_utils) {
        (Some(version), Some(part)) => *part = format!("ai-sdk-provider-utils/{version}"),
        // Nothing to rename or append: the header stays exactly as it is.
        _ if suffix.is_none() => return,
        _ => {}
    }
    parts.extend(suffix.map(String::from));
    let user_agent = parts.join(" ");
    if user_agent == current {
        return;
    }
    if let Ok(value) = reqwest::header::HeaderValue::try_from(user_agent) {
        headers.insert(reqwest::header::USER_AGENT, value);
    }
}
