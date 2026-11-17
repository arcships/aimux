//! Helpers the native packages and the presets share.
//!
//! Only transport plumbing lives here: where the base URL and the
//! provider-level request headers of a request come from, as data. The few
//! vendors whose endpoint is decided by more than a URL and a credential
//! (Vertex, Bedrock, Polly) keep that logic in their own modules.

pub(crate) mod aisdk_pins;
mod discovery;
mod error_body;
mod exchange;
mod factory;
mod image_size;
mod media;
mod poll;
mod provider_options;
mod results;
mod search;

pub(crate) use discovery::{ModelListEntry, list_data_models};
pub(crate) use error_body::{
    json_error_code, message_type_error_response_handler, nested_error_response_handler,
};
pub(crate) use exchange::{
    BaseUrl, Endpoint, EndpointConfig, EndpointHeaders, HttpEndpoint, ResolveEndpoint,
    SupportedUrlsSource,
};
pub(crate) use factory::{
    VendorFactoryConfig, VendorParts, build_vendor_parts, debug_vendor_settings,
    define_vendor_provider,
};
pub(crate) use image_size::size_to_aspect_ratio;
pub(crate) use media::{audio_input_to_base64, audio_input_to_bytes, video_file_to_url_or_base64};
pub(crate) use poll::{
    POLL_INTERVAL_MILLIS_KEY, POLL_INTERVAL_MS_KEY, PollStep, is_poll_control_key,
    poll_interval_ms, poll_until, retry_download,
};
pub(crate) use provider_options::namespaced_options;
pub(crate) use results::{
    image_response, speech_response, task_failed, transcription_response, unsupported,
    video_response,
};
pub(crate) use search::{get_query_search, get_query_search_with_exchange, post_json_search};

use std::sync::Arc;

use serde_json::Value;

use aimux_provider_utils::{FetchFunction, UserAgentFetch};

/// A provider-level rewrite of every JSON request body, called once after the
/// body is serialized and before it is sent.
pub type TransformRequestBody = Arc<dyn Fn(Value) -> Value + Send + Sync>;

/// The transport of a pinned package: `fetch` (the process default, resolved
/// per request, when `None`) behind a [`UserAgentFetch`] that names the
/// provider-utils `package` depends on in the `ai-sdk-provider-utils/<v>` part
/// the helper layer adds. Upstream that part comes from the provider-utils the
/// package itself imports (provider-utils post-to-api.ts:100-104,
/// get-from-api.ts:77-81), so it follows the package's `package.json`, not
/// the workspace pin.
pub(crate) fn package_fetch(
    fetch: Option<FetchFunction>,
    package: aisdk_pins::Package,
) -> FetchFunction {
    Arc::new(UserAgentFetch {
        inner: fetch,
        provider_utils: Some(package.provider_utils),
        suffix: None,
    })
}
