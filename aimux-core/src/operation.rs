//! The settings of the user operation that a model call runs in.
//!
//! The AI SDK keeps `maxRetries` and `timeout` on `generateText` /
//! `streamText` (`RequestOptions`, ai@7.0.127 `prompt/request-options.ts:104-127`)
//! and reports them on the operation's start event
//! (`generate-text/generate-text-events.ts:54-62`). The model call options
//! carry neither (`LanguageModelV4CallOptions`,
//! @ai-sdk/provider@4.0.21 `language-model/v4/language-model-v4-call-options.ts:8-138`).
//! Aimux's session id (RFC-0024) is an operation setting too.
//!
//! `generate_text`, `stream_text` and `decide` run their model calls inside
//! [`OperationSettings::scope`]. Code below the model interface that needs a
//! setting reads [`OperationSettings::current`]: the router and MoA take the
//! child retry count from it, the trace layer the session id, and the recorder
//! writes the settings into the input record (docs/aisdk-architecture-alignment.md
//! §3.5, §4.5).

use std::future::Future;

use serde::{Deserialize, Serialize};

use crate::options::TimeoutConfiguration;

tokio::task_local! {
    static CURRENT: OperationSettings;
}

/// Operation settings that are not part of the model call.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationSettings {
    /// Maximum number of retries, the AI SDK's `maxRetries`; `Some(0)`
    /// disables retrying. `None` uses the default.
    #[serde(default)]
    pub max_retries: Option<u32>,
    /// Timeout configuration (total / step / first-chunk / chunk idle).
    #[serde(default)]
    pub timeout: Option<TimeoutConfiguration>,
    /// Explicit session id (RFC-0024), for grouping consecutive calls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

impl OperationSettings {
    /// The settings of the operation the current task runs in, or `None`
    /// outside an operation (a direct `do_generate` / `do_stream` call).
    #[must_use]
    pub fn current() -> Option<Self> {
        CURRENT.try_with(Clone::clone).ok()
    }

    /// Run `future` with these settings as the current operation settings.
    ///
    /// `generate_text` and `stream_text` call it themselves. A caller that
    /// drives a model directly uses it to give composite children a retry
    /// count or the trace layer a session id. A task spawned inside the
    /// future does not inherit the settings.
    pub async fn scope<F: Future>(self, future: F) -> F::Output {
        CURRENT.scope(self, future).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_settings_are_current_only_inside_their_scope() {
        assert_eq!(OperationSettings::current(), None);
        let settings = OperationSettings {
            max_retries: Some(0),
            timeout: None,
            session_id: Some("s-1".into()),
        };
        let inside = settings
            .clone()
            .scope(async { OperationSettings::current() })
            .await;
        assert_eq!(inside, Some(settings));
        assert_eq!(OperationSettings::current(), None);
    }

    #[test]
    fn the_settings_serialize_as_the_call_options_fields_did() {
        // The recorded input keeps the keys it had when the three settings
        // were `CallOptions` fields: `max_retries` and `timeout` always,
        // `session_id` only when set.
        let json = serde_json::to_value(OperationSettings::default()).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "max_retries": null, "timeout": null })
        );
        let back: OperationSettings = serde_json::from_value(serde_json::json!({
            "max_retries": 1,
            "timeout": { "total_ms": 5, "step_ms": null, "first_chunk_ms": null, "chunk_ms": null },
            "session_id": "s",
            "temperature": 0.5
        }))
        .unwrap();
        assert_eq!(back.max_retries, Some(1));
        assert_eq!(back.timeout.unwrap().total_ms, Some(5));
        assert_eq!(back.session_id.as_deref(), Some("s"));
    }
}
