//! MiniMax H3 media models: video (`/v2/video_generation`) and image
//! (`/v1/images/generations`).
//!
//! No AI SDK package covers these endpoints; the wire shapes follow the
//! MiniMax documentation:
//!
//! - [video creation](https://platform.minimax.cn/docs/api-reference/video-generation-v2-create)
//!   and [task query](https://platform.minimax.cn/docs/api-reference/video-generation-v2-query):
//!   `MiniMax-H3`, `MiniMax-H3-Highspeed`, `MiniMax-H3-Lite`, `MiniMax-H3-Max`;
//! - [image generation](https://platform.minimax.cn/docs/api-reference/image-generation-h3-generations):
//!   `MiniMax-H3-Image`, `MiniMax-H3-Image-Highspeed`.
//!
//! This package offers the media models. [`crate::create_provider`] combines
//! them with the existing OpenAI-compatible text preset for `minimax`,
//! `minimax_cn`, `minimax_coding_plan`, and `minimax_cn_coding_plan`: the four
//! preset names route the IDs in `IMAGE_MODEL_IDS` and `VIDEO_MODEL_IDS` to
//! this package, and every other image or video ID keeps the preset's own factory
//! (an image model that exists only on a relay still goes to the relay). Chat,
//! embeddings, and discovery keep the preset behavior. The preset's regional
//! base URL, explicit key, extra headers, and custom transport also configure
//! the media requests. The optional `transform_request_body` setting applies
//! only to the text preset.
//!
//! Both generation requests are billable and not idempotent, so a failed
//! create (video) or generate (image) request is never sent again
//! automatically, except after an HTTP 429, which MiniMax answers before
//! accepting the work. The video status query keeps the usual retryable
//! classification (429, 5xx, transport failures).
//!
//! [`create_minimax`] takes [`MiniMaxProviderSettings`], normalizes and
//! validates the base URL, fixes the provider name and returns a
//! [`MiniMaxProvider`]. The credential is not read there: it is loaded for
//! every request, from the setting or from `MINIMAX_API_KEY`. [`minimax()`] is
//! the default instance; it reads nothing and cannot fail.

mod image_model;
mod image_probe;
pub(crate) mod options;
mod provider;
mod video_model;

pub use image_model::MiniMaxImageModel;
pub use provider::{
    MINIMAX_CN_BASE_URL, MiniMaxProvider, MiniMaxProviderSettings, create_minimax, minimax,
};
pub use video_model::MiniMaxVideoModel;

/// The image model IDs the native provider serves, and the only image IDs the
/// named MiniMax presets route to it.
pub(crate) const IMAGE_MODEL_IDS: [&str; 2] = ["MiniMax-H3-Image", "MiniMax-H3-Image-Highspeed"];

/// The video model IDs the native provider serves, and the only video IDs the
/// named MiniMax presets route to it.
pub(crate) const VIDEO_MODEL_IDS: [&str; 4] = [
    "MiniMax-H3",
    "MiniMax-H3-Highspeed",
    "MiniMax-H3-Lite",
    "MiniMax-H3-Max",
];

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

use aimux_core::error::AiMuxError;

/// `encodeURIComponent`'s unreserved set.
const URI_COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'!')
    .remove(b'~')
    .remove(b'*')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')');

/// A task id as one path segment. Dot segments are normalized by URL
/// parsing, so they are encoded twice.
fn encode_path_segment(value: &str) -> String {
    match utf8_percent_encode(value, URI_COMPONENT)
        .to_string()
        .as_str()
    {
        "." => "%252E".to_owned(),
        ".." => "%252E%252E".to_owned(),
        encoded => encoded.to_owned(),
    }
}

/// The error of a billable request, made safe for Core's retry: retryable
/// only when MiniMax rejected it with HTTP 429 before accepting the work. A
/// transport failure, a local exchange timeout or a 5xx may come after the
/// task was created and billed, so they are not retried.
fn not_retryable_unless_rate_limited(error: AiMuxError) -> AiMuxError {
    match error {
        AiMuxError::ApiCall(mut detail)
            if detail.is_retryable && detail.status_code != Some(429) =>
        {
            detail.is_retryable = false;
            AiMuxError::ApiCall(detail)
        }
        other => other,
    }
}

/// The error of a step that runs after a billable request was accepted: never
/// retryable, because Core's retry would send the billable request again.
fn not_retryable(error: AiMuxError) -> AiMuxError {
    match error {
        AiMuxError::ApiCall(mut detail) => {
            detail.is_retryable = false;
            AiMuxError::ApiCall(detail)
        }
        other => other,
    }
}

/// `base_resp` of MiniMax's older envelope: a 2xx body may still report a
/// failure with a non-zero `status_code`.
#[derive(Debug, serde::Deserialize)]
struct BaseResp {
    #[serde(default)]
    status_code: Option<i64>,
    #[serde(default)]
    status_msg: Option<String>,
}

impl BaseResp {
    /// The failure this envelope reports, if any.
    fn failure(&self) -> Option<(i64, String)> {
        match self.status_code {
            Some(code) if code != 0 => Some((
                code,
                self.status_msg
                    .clone()
                    .unwrap_or_else(|| format!("MiniMax error code: {code}")),
            )),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aimux_core::error::ApiCallError;

    fn api_error(status: Option<u16>, retryable: bool) -> AiMuxError {
        AiMuxError::ApiCall(Box::new(ApiCallError {
            status_code: status,
            is_retryable: retryable,
            ..ApiCallError::new("x", "http://h", serde_json::json!({}))
        }))
    }

    #[test]
    fn only_a_rate_limit_keeps_a_billable_request_retryable() {
        assert!(not_retryable_unless_rate_limited(api_error(Some(429), true)).is_retryable());
        assert!(!not_retryable_unless_rate_limited(api_error(Some(500), true)).is_retryable());
        assert!(!not_retryable_unless_rate_limited(api_error(None, true)).is_retryable());
        assert!(!not_retryable(api_error(Some(429), true)).is_retryable());
    }

    #[test]
    fn a_task_id_is_one_path_segment() {
        assert_eq!(encode_path_segment("424010985738629"), "424010985738629");
        assert_eq!(encode_path_segment("a/b"), "a%2Fb");
        assert_eq!(encode_path_segment(".."), "%252E%252E");
    }
}
