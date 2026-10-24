//! Upstream `xai-speech-model.test.ts`: the `XaiSpeechModel` describe and the
//! `doGenerate` case that stamps the response.
//!
//! The request goes through a [`MockFetch`], as in the provider tests.
//!
//! Mapping notes:
//! - `specificationVersion`: `SpeechModel::specification_version()` is the
//!   trait's `"v4"`, which xAI keeps.
//! - The injected `_internal.currentDate` has no counterpart: aimux stamps the
//!   response with `Timestamp::now()` and has no clock seam. The stamp is
//!   checked to fall within the call instead of equalling the injected date.

use std::sync::Arc;

use aimux_core::speech_model::{SpeechCallOptions, SpeechModel};

use crate::test_support::mock_fetch::{Canned, MockFetch};
use crate::xai::XaiSpeechModel;
use crate::{XAIProviderSettings, create_xai};

/// The audio the upstream server answers with (`new Uint8Array([1, 2, 3, 4])`).
const AUDIO: [u8; 4] = [1, 2, 3, 4];

/// `createXai({ apiKey: 'test-api-key' }).speech()`, sending through `fetch`.
fn speech_model(fetch: &Arc<MockFetch>) -> XaiSpeechModel {
    create_xai(XAIProviderSettings {
        api_key: Some("test-api-key".to_string()),
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .expect("creating an xAI provider does not fail")
    .speech()
}

/// The `audio/mpeg` answer of the upstream `prepareAudioResponse`, with extra
/// response headers.
fn audio_answer(headers: &[(&str, &str)]) -> Canned {
    let mut all = vec![("content-type".to_string(), "audio/mpeg".to_string())];
    all.extend(
        headers
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string())),
    );
    Canned {
        status: 200,
        headers: all,
        body: AUDIO.to_vec(),
    }
}

mod xai_speech_model {
    use super::*;

    /// upstream: @ai-sdk/xai src/xai-speech-model.test.ts › XaiSpeechModel › should expose correct provider and model information
    #[test]
    fn should_expose_correct_provider_and_model_information() {
        let model = create_xai(XAIProviderSettings::default())
            .expect("creating an xAI provider does not fail")
            .speech();

        assert_eq!(model.provider(), "xai.speech");
        assert_eq!(model.model_id(), "");
        assert_eq!(model.specification_version(), "v4");
    }
}

mod do_generate {
    use super::*;

    /// upstream: @ai-sdk/xai src/xai-speech-model.test.ts › doGenerate › should include response timestamp, model id, and headers
    #[tokio::test]
    async fn should_include_response_timestamp_model_id_and_headers() {
        let fetch = MockFetch::new(vec![audio_answer(&[("x-request-id", "test-request-id")])]);

        let before = chrono::Utc::now().timestamp_millis();
        let result = speech_model(&fetch)
            .do_generate(&SpeechCallOptions::new("Hello from the AI SDK!"))
            .await
            .expect("the call succeeds");
        let after = chrono::Utc::now().timestamp_millis();

        let timestamp = result
            .response
            .timestamp
            .expect("the response has a timestamp")
            .unix_millis();
        assert!(
            before <= timestamp && timestamp <= after,
            "timestamp {timestamp} is outside the call ({before}..={after})"
        );
        assert_eq!(result.response.model_id.as_deref(), Some(""));
        assert_eq!(
            result
                .response
                .headers
                .expect("the response has headers")
                .get("x-request-id")
                .map(String::as_str),
            Some("test-request-id")
        );
    }
}
