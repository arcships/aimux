use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use aimux_core::AbortSignal;
use aimux_core::error::{AiMuxError, ApiCallError};
use aimux_core::provider::Provider;
use aimux_core::transcription_model::{
    AudioChunk, AudioInput, InputAudioFormat, TranscriptionCallOptions, TranscriptionModel,
    TranscriptionStreamOptions,
};
use aimux_provider_utils::{
    Fetch, FetchError, FetchFunction, FetchRequest, FetchResponse, Resolvable,
};
use async_trait::async_trait;
use futures::future::BoxFuture;
use serde::Serialize;
use serde_json::{Map, Value, json};
use serial_test::serial;

use crate::azure::{AzureOpenAIProvider, AzureOpenAIProviderSettings, create_azure};
use crate::test_support::mock_fetch::{Canned, EnvVar, RouteFetch, Seen};

/// The TS `input`: three bytes of `audio/wav`.
fn input() -> TranscriptionCallOptions {
    TranscriptionCallOptions::new(AudioInput::Binary(vec![1, 2, 3]), "audio/wav")
}

/// The TS `{ ...input, providerOptions }`.
fn with_options(mut call: TranscriptionCallOptions, options: Value) -> TranscriptionCallOptions {
    call.provider_options = Some(serde_json::from_value(options).expect("provider options"));
    call
}

/// The TS `response` constant: a Speech result with one phrase.
fn default_body() -> Value {
    json!({
        "durationMilliseconds": 2000,
        "combinedPhrases": [{ "text": "Hello world." }],
        "phrases": [{
            "text": "Hello world.",
            "offsetMilliseconds": 40,
            "durationMilliseconds": 1500,
            "locale": "en-US",
            "speaker": 1,
            "confidence": 0.9,
            "words": [{ "text": "Hello", "offsetMilliseconds": 40, "durationMilliseconds": 400 }]
        }]
    })
}

/// The settings `setup()` starts from: `resourceName: 'test-resource'`, `apiKey: 'test-key'`.
fn default_settings() -> AzureOpenAIProviderSettings {
    AzureOpenAIProviderSettings {
        resource_name: Some("test-resource".into()),
        api_key: Some(Resolvable::Value("test-key".into())),
        ..Default::default()
    }
}

/// The TS `tokenProvider: vi.fn()`: the nth call resolves to `tokens[n]`, and
/// `calls` counts the calls.
fn token_provider(tokens: &[&str]) -> (Resolvable<String>, Arc<AtomicUsize>) {
    let queue: Arc<Mutex<VecDeque<String>>> =
        Arc::new(Mutex::new(tokens.iter().map(ToString::to_string).collect()));
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let produce = move || -> BoxFuture<'static, Result<String, AiMuxError>> {
        counter.fetch_add(1, Ordering::SeqCst);
        let token = queue
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| "unexpected".to_string());
        Box::pin(async move { Ok(token) })
    };
    (Resolvable::AsyncFn(Arc::new(produce)), calls)
}

/// The transport the provider uses: records each request's abort signal, fails
/// the next call when `reject_next` is set (the TS `mockRejectedValueOnce`), and
/// otherwise answers through the recording route.
struct Transport {
    route: Arc<RouteFetch>,
    signals: Mutex<Vec<Option<AbortSignal>>>,
    reject_next: Mutex<bool>,
}

#[async_trait]
impl Fetch for Transport {
    async fn fetch(&self, request: FetchRequest) -> Result<FetchResponse, FetchError> {
        self.signals.lock().unwrap().push(request.signal.clone());
        if std::mem::take(&mut *self.reject_next.lock().unwrap()) {
            return Err(FetchError::Aborted);
        }
        self.route.fetch(request).await
    }
}

/// The TS `setup(settings, body)`: a provider whose fetch answers
/// `/audio/transcriptions` with `{ text: 'OpenAI' }` and every other request
/// with `body`.
struct Setup {
    provider: AzureOpenAIProvider,
    transport: Arc<Transport>,
    /// The TS `fetch.mockResolvedValueOnce`: answers the next request only.
    once: Arc<Mutex<VecDeque<Canned>>>,
}

fn setup(mut settings: AzureOpenAIProviderSettings, body: Value) -> Setup {
    let once: Arc<Mutex<VecDeque<Canned>>> = Arc::default();
    let next = once.clone();
    let route = RouteFetch::new(move |seen: &Seen| {
        if let Some(canned) = next.lock().unwrap().pop_front() {
            return canned;
        }
        if seen.url.contains("/audio/transcriptions") {
            Canned::json(&json!({ "text": "OpenAI" }))
        } else {
            Canned::json(&body)
        }
    });
    let transport = Arc::new(Transport {
        route,
        signals: Mutex::default(),
        reject_next: Mutex::new(false),
    });
    settings.fetch = Some(transport.clone() as FetchFunction);
    let provider = create_azure(settings).expect("valid Azure settings");
    Setup {
        provider,
        transport,
        once,
    }
}

impl Setup {
    fn requests(&self) -> Vec<Seen> {
        self.transport.route.seen()
    }

    fn fetch_calls(&self) -> usize {
        self.requests().len()
    }

    /// The TS `request()`: the last request.
    fn request(&self) -> Seen {
        self.requests().pop().expect("a request was sent")
    }

    /// The TS `definition()`: the parsed `definition` form field of the last request.
    fn definition(&self) -> Value {
        let text = form_text(&self.request(), "definition").expect("definition field");
        serde_json::from_str(&text).expect("definition is JSON")
    }

    /// The abort signal the transport received with the last request.
    fn last_signal(&self) -> AbortSignal {
        self.transport
            .signals
            .lock()
            .unwrap()
            .last()
            .cloned()
            .flatten()
            .expect("the request carried the caller's signal")
    }

    fn respond_once(&self, canned: Canned) {
        self.once.lock().unwrap().push_back(canned);
    }

    fn reject_next(&self) {
        *self.transport.reject_next.lock().unwrap() = true;
    }
}

/// One part of a `multipart/form-data` request body.
struct FormPart {
    name: String,
    filename: Option<String>,
    content_type: Option<String>,
    data: Vec<u8>,
}

fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    haystack[from..]
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|index| index + from)
}

/// The value of `key` in a `Content-Disposition` line: `; key="value"`.
fn quoted_param(line: &str, key: &str) -> Option<String> {
    let marker = format!("; {key}=\"");
    let start = line.find(&marker)? + marker.len();
    let end = line[start..].find('"')? + start;
    Some(line[start..end].to_string())
}

fn form_parts(seen: &Seen) -> Vec<FormPart> {
    let content_type = seen
        .headers
        .get("content-type")
        .expect("content-type header");
    let boundary = content_type
        .split("boundary=")
        .nth(1)
        .expect("multipart boundary");
    let delimiter = format!("--{boundary}");
    let body = seen.body.as_slice();
    let mut parts = Vec::new();
    let mut cursor = find(body, delimiter.as_bytes(), 0).expect("opening boundary");
    while let Some(next) = find(body, delimiter.as_bytes(), cursor + delimiter.len()) {
        let segment = &body[cursor + delimiter.len()..next];
        let segment = segment.strip_prefix(b"\r\n").unwrap_or(segment);
        let segment = segment.strip_suffix(b"\r\n").unwrap_or(segment);
        let split = find(segment, b"\r\n\r\n", 0).expect("header and body separator");
        let head = std::str::from_utf8(&segment[..split]).expect("UTF-8 part headers");
        let disposition = head
            .lines()
            .find(|line| line.starts_with("Content-Disposition"))
            .expect("Content-Disposition header");
        parts.push(FormPart {
            name: quoted_param(disposition, "name").expect("field name"),
            filename: quoted_param(disposition, "filename"),
            content_type: head
                .lines()
                .find_map(|line| line.strip_prefix("Content-Type: "))
                .map(str::to_string),
            data: segment[split + 4..].to_vec(),
        });
        cursor = next;
    }
    parts
}

fn form_text(seen: &Seen, name: &str) -> Option<String> {
    form_parts(seen)
        .into_iter()
        .find(|part| part.name == name)
        .map(|part| String::from_utf8(part.data).expect("UTF-8 form field"))
}

fn form_file(seen: &Seen, name: &str) -> FormPart {
    form_parts(seen)
        .into_iter()
        .find(|part| part.name == name)
        .expect("form file")
}

/// Vitest `toEqual` on JSON: the same keys and values; numbers compare as
/// JavaScript numbers do.
fn assert_js_equal(actual: &Value, expected: &Value) {
    assert!(
        js_equal(actual, expected),
        "expected {expected}, got {actual}"
    );
}

fn js_equal(actual: &Value, expected: &Value) -> bool {
    match (actual, expected) {
        (Value::Number(a), Value::Number(b)) => a.as_f64() == b.as_f64(),
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| js_equal(a, b))
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && b.iter()
                    .all(|(key, value)| a.get(key).is_some_and(|a| js_equal(a, value)))
        }
        _ => actual == expected,
    }
}

/// Vitest `toMatchObject` on JSON: every key of `expected` is present in
/// `actual` and matches; arrays must have the same length.
fn assert_match_object(actual: &Value, expected: &Value) {
    assert!(
        match_object(actual, expected),
        "expected {actual} to match {expected}"
    );
}

fn match_object(actual: &Value, expected: &Value) -> bool {
    match (actual, expected) {
        (Value::Number(a), Value::Number(b)) => a.as_f64() == b.as_f64(),
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| match_object(a, b))
        }
        (Value::Object(a), Value::Object(b)) => b
            .iter()
            .all(|(key, value)| a.get(key).is_some_and(|a| match_object(a, value))),
        _ => actual == expected,
    }
}

fn json_of<T: Serialize>(value: &T) -> Value {
    serde_json::to_value(value).expect("serializable")
}

fn api_call_error(error: AiMuxError) -> Box<ApiCallError> {
    match error {
        AiMuxError::ApiCall(error) => error,
        other => panic!("expected an API call error, got {other:?}"),
    }
}

mod api_routing {
    use super::*;

    /// upstream: @ai-sdk/azure src/azure-transcription-model.test.ts › API routing › routes %s with api=%s to %s
    #[tokio::test]
    async fn routes_s_with_api_s_to_s() {
        let rows: [(&str, Option<&str>, &str); 9] = [
            ("mai-transcribe-2", None, "speech"),
            ("MAI-Transcribe-2", None, "speech"),
            ("MaI-TrAnScRiBe-2", None, "speech"),
            ("mai-transcribe-2-custom", None, "openai"),
            ("whisper-1", None, "openai"),
            ("custom-deployment", None, "openai"),
            ("mai-transcribe-2", Some("openai"), "openai"),
            ("Future-Speech-Model", Some("speech"), "speech"),
            ("mai-transcribe-2", Some("speech"), "speech"),
        ];
        for (id, api, expected) in rows {
            let harness = setup(default_settings(), default_body());
            let azure = match api {
                Some(api) => json!({ "api": api }),
                None => json!({}),
            };
            harness
                .provider
                .transcription(id)
                .do_generate(&with_options(input(), json!({ "azure": azure })))
                .await
                .unwrap();
            let url = harness.request().url;
            if expected == "speech" {
                assert!(url.contains("/speechtotext/"), "{url}");
                let model = if id.to_lowercase() == "mai-transcribe-2" {
                    "MAI-Transcribe-2"
                } else {
                    id
                };
                assert_match_object(
                    &harness.definition()["enhancedMode"],
                    &json!({ "enabled": true, "model": model }),
                );
            } else {
                assert!(url.contains("/audio/transcriptions"), "{url}");
                assert_eq!(form_text(&harness.request(), "model").as_deref(), Some(id));
            }
        }
    }

    /// upstream: @ai-sdk/azure src/azure-transcription-model.test.ts › API routing › resolves the API again for each call to the same model
    #[tokio::test]
    async fn resolves_the_api_again_for_each_call_to_the_same_model() {
        let harness = setup(default_settings(), default_body());
        let model = harness.provider.transcription("mai-transcribe-2");
        model.do_generate(&input()).await.unwrap();
        assert!(harness.request().url.contains("/speechtotext/"));
        model
            .do_generate(&with_options(
                input(),
                json!({ "azure": { "api": "openai" } }),
            ))
            .await
            .unwrap();
        assert!(harness.request().url.contains("/audio/transcriptions"));
        model.do_generate(&input()).await.unwrap();
        assert!(harness.request().url.contains("/speechtotext/"));
    }

    /// upstream: @ai-sdk/azure src/azure-transcription-model.test.ts › API routing › preserves OpenAI options and warns about Speech-only options
    #[tokio::test]
    async fn preserves_openai_options_and_warns_about_speech_only_options() {
        let harness = setup(default_settings(), default_body());
        let call = with_options(
            input(),
            json!({
                "openai": { "language": "en", "prompt": "Names" },
                "azure": { "api": "openai", "timestamps": "word" }
            }),
        );
        let result = harness
            .provider
            .transcription("whisper-1")
            .do_generate(&call)
            .await
            .unwrap();
        let request = harness.request();
        assert_eq!(form_text(&request, "language").as_deref(), Some("en"));
        assert_eq!(form_text(&request, "prompt").as_deref(), Some("Names"));
        assert_js_equal(
            &json_of(&result.warnings),
            &json!([{
                "type": "unsupported",
                "feature": "providerOptions.azure.timestamps",
                "details": "This option requires the Azure Speech API."
            }]),
        );
    }

    /// upstream: @ai-sdk/azure src/azure-transcription-model.test.ts › API routing › rejects invalid options %j before fetching
    #[tokio::test]
    async fn rejects_invalid_options_j_before_fetching() {
        let rows = [
            json!({ "api": "invalid" }),
            json!({ "timestamps": "invalid" }),
            json!({ "locales": ["en", "de"] }),
            json!({ "transcribeStyle": "invalid" }),
            json!({ "diarization": { "enabled": "yes" } }),
            json!({ "diarization": { "enabled": true, "maxSpeakers": 2 } }),
            json!({ "phraseList": { "phrases": ["Vercel"], "biasingWeight": 1.5 } }),
            json!({ "profanityFilterMode": "Masked" }),
            json!({ "timestamp": "word" }),
        ];
        for azure in rows {
            let harness = setup(default_settings(), default_body());
            let error = harness
                .provider
                .transcription("mai-transcribe-2")
                .do_generate(&with_options(input(), json!({ "azure": azure.clone() })))
                .await
                .unwrap_err();
            assert!(
                matches!(error, AiMuxError::InvalidArgument(_)),
                "{azure}: {error:?}"
            );
            assert_eq!(harness.fetch_calls(), 0, "{azure}");
        }
    }
}

mod speech_requests {
    use super::*;

    /// upstream: @ai-sdk/azure src/azure-transcription-model.test.ts › Speech requests › sends audio and the required model definition
    #[tokio::test]
    async fn sends_audio_and_the_required_model_definition() {
        for audio in [
            AudioInput::Binary(vec![1, 2, 3]),
            AudioInput::Base64("AQID".into()),
        ] {
            let harness = setup(default_settings(), default_body());
            let mut call = input();
            call.audio = audio;
            harness
                .provider
                .transcription("mai-transcribe-2")
                .do_generate(&call)
                .await
                .unwrap();
            let request = harness.request();
            assert_eq!(
                request.url,
                "https://test-resource.cognitiveservices.azure.com/speechtotext/transcriptions:transcribe?api-version=2025-10-15"
            );
            assert_eq!(request.headers["ocp-apim-subscription-key"], "test-key");
            assert!(!request.headers.contains_key("api-key"));
            assert!(
                request
                    .headers
                    .get("user-agent")
                    .is_some_and(|agent| agent.contains("ai-sdk-azure/")),
                "{:?}",
                request.headers.get("user-agent")
            );
            assert_js_equal(
                &harness.definition(),
                &json!({
                    "enhancedMode": {
                        "enabled": true,
                        "model": "MAI-Transcribe-2",
                        "modelOptions": { "timestamps": "segment" }
                    }
                }),
            );
            let file = form_file(&request, "audio");
            assert_eq!(file.filename.as_deref(), Some("audio.wav"));
            assert_eq!(file.content_type.as_deref(), Some("audio/wav"));
            assert_eq!(file.data, vec![1, 2, 3]);
        }
    }

    /// upstream: @ai-sdk/azure src/azure-transcription-model.test.ts › Speech requests › serializes all options with %s timestamps
    #[tokio::test]
    async fn serializes_all_options_with_s_timestamps() {
        for timestamps in ["word", "segment", "none"] {
            let harness = setup(default_settings(), default_body());
            let azure = json!({
                "timestamps": timestamps,
                "transcribeStyle": "clean",
                "locales": ["en"],
                "diarization": { "enabled": true },
                "phraseList": { "phrases": ["Vercel"] }
            });
            harness
                .provider
                .transcription("mai-transcribe-2")
                .do_generate(&with_options(input(), json!({ "azure": azure })))
                .await
                .unwrap();
            assert_js_equal(
                &harness.definition(),
                &json!({
                    "enhancedMode": {
                        "enabled": true,
                        "model": "MAI-Transcribe-2",
                        "modelOptions": { "timestamps": timestamps, "transcribeStyle": "clean" }
                    },
                    "locales": ["en"],
                    "diarization": { "enabled": true },
                    "phraseList": { "phrases": ["Vercel"] }
                }),
            );
        }
    }

    /// upstream: @ai-sdk/azure src/azure-transcription-model.test.ts › Speech requests › rejects a resourceName that would rewrite the Speech host
    #[tokio::test]
    async fn rejects_a_resource_name_that_would_rewrite_the_speech_host() {
        let mut settings = default_settings();
        settings.resource_name = Some("user@internal:8080/#".into());
        let harness = setup(settings, default_body());
        let error = harness
            .provider
            .transcription("mai-transcribe-2")
            .do_generate(&input())
            .await
            .unwrap_err();
        // The AI SDK's error also names `argument: 'resourceName'`; aimux's
        // InvalidArgument carries only a message, so the variant is checked.
        assert!(matches!(error, AiMuxError::InvalidArgument(_)), "{error:?}");
        assert_eq!(harness.fetch_calls(), 0);
    }

    /// upstream: @ai-sdk/azure src/azure-transcription-model.test.ts › Speech requests › loads resource and key from the existing environment variables
    #[tokio::test]
    #[serial]
    async fn loads_resource_and_key_from_the_existing_environment_variables() {
        let _resource = EnvVar::set("AZURE_RESOURCE_NAME", Some("environment-resource"));
        let _key = EnvVar::set("AZURE_API_KEY", Some("environment-key"));
        let mut settings = default_settings();
        settings.resource_name = None;
        settings.api_key = None;
        let harness = setup(settings, default_body());
        harness
            .provider
            .transcription("mai-transcribe-2")
            .do_generate(&input())
            .await
            .unwrap();
        let request = harness.request();
        assert!(
            request
                .url
                .contains("https://environment-resource.cognitiveservices.azure.com/"),
            "{}",
            request.url
        );
        assert_eq!(
            request.headers["ocp-apim-subscription-key"],
            "environment-key"
        );
    }

    /// upstream: @ai-sdk/azure src/azure-transcription-model.test.ts › Speech requests › honors speechBaseURL, headers, and abort overrides
    #[tokio::test]
    async fn honors_speech_base_url_headers_and_abort_overrides() {
        let mut settings = default_settings();
        settings.speech_base_url = Some("https://eastus.api.cognitive.microsoft.com/".into());
        settings.headers = Some(HashMap::from([(
            "x-custom".to_string(),
            Some("provider".to_string()),
        )]));
        let harness = setup(settings, default_body());
        let controller = AbortSignal::new();
        let mut call = input();
        call.headers = Some(HashMap::from([(
            "x-custom".to_string(),
            "call".to_string(),
        )]));
        call.abort_signal = Some(controller.clone());
        harness
            .provider
            .transcription("mai-transcribe-2")
            .do_generate(&call)
            .await
            .unwrap();
        let request = harness.request();
        assert_eq!(
            request.url,
            "https://eastus.api.cognitive.microsoft.com/speechtotext/transcriptions:transcribe?api-version=2025-10-15"
        );
        assert_eq!(request.headers["x-custom"], "call");
        // The TS `request().signal` is the caller's `controller.signal`: the
        // transport's signal shares its cancellation with the caller's.
        let signal = harness.last_signal();
        assert!(!signal.is_aborted());
        controller.abort();
        assert!(signal.is_aborted());
    }

    /// upstream: @ai-sdk/azure src/azure-transcription-model.test.ts › Speech requests › does not require resourceName when speechBaseURL is set
    #[tokio::test]
    async fn does_not_require_resource_name_when_speech_base_url_is_set() {
        let mut settings = default_settings();
        settings.resource_name = None;
        settings.speech_base_url = Some("https://eastus.api.cognitive.microsoft.com".into());
        let harness = setup(settings, default_body());
        harness
            .provider
            .transcription("mai-transcribe-2")
            .do_generate(&input())
            .await
            .unwrap();
        assert!(
            harness
                .request()
                .url
                .contains("https://eastus.api.cognitive.microsoft.com/speechtotext/")
        );
    }

    /// upstream: @ai-sdk/azure src/azure-transcription-model.test.ts › Speech requests › does not apply Azure OpenAI baseURL or apiVersion to Speech requests
    #[tokio::test]
    async fn does_not_apply_azure_openai_base_url_or_api_version_to_speech_requests() {
        let mut settings = default_settings();
        settings.base_url = Some("https://test-resource.openai.azure.com/openai".into());
        settings.api_version = Some("2025-04-01-preview".into());
        settings.use_deployment_based_urls = true;
        let harness = setup(settings, default_body());
        harness
            .provider
            .transcription("mai-transcribe-2")
            .do_generate(&input())
            .await
            .unwrap();
        assert_eq!(
            harness.request().url,
            "https://test-resource.cognitiveservices.azure.com/speechtotext/transcriptions:transcribe?api-version=2025-10-15"
        );
    }

    /// upstream: @ai-sdk/azure src/azure-transcription-model.test.ts › Speech requests › gets a fresh bearer token for each request
    #[tokio::test]
    async fn gets_a_fresh_bearer_token_for_each_request() {
        let (token_provider, calls) = token_provider(&["first", "second"]);
        let mut settings = default_settings();
        settings.api_key = None;
        settings.token_provider = Some(token_provider);
        let harness = setup(settings, default_body());
        let model = harness.provider.transcription("mai-transcribe-2");
        model.do_generate(&input()).await.unwrap();
        let request = harness.request();
        assert_eq!(request.headers["authorization"], "Bearer first");
        assert!(!request.headers.contains_key("ocp-apim-subscription-key"));
        model.do_generate(&input()).await.unwrap();
        assert_eq!(harness.request().headers["authorization"], "Bearer second");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    /// upstream: @ai-sdk/azure src/azure-transcription-model.test.ts › Speech requests › does not replace an explicitly supplied bearer token
    #[tokio::test]
    async fn does_not_replace_an_explicitly_supplied_bearer_token() {
        let (token_provider, calls) = token_provider(&[]);
        let mut settings = default_settings();
        settings.api_key = None;
        settings.token_provider = Some(token_provider);
        let harness = setup(settings, default_body());
        let mut call = input();
        call.headers = Some(HashMap::from([(
            "Authorization".to_string(),
            "Bearer custom".to_string(),
        )]));
        harness
            .provider
            .transcription("mai-transcribe-2")
            .do_generate(&call)
            .await
            .unwrap();
        assert_eq!(harness.request().headers["authorization"], "Bearer custom");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
}

mod speech_responses {
    use super::*;

    /// upstream: @ai-sdk/azure src/azure-transcription-model.test.ts › Speech responses › normalizes results and preserves speaker, word, locale, and confidence metadata
    #[tokio::test]
    async fn normalizes_results_and_preserves_speaker_word_locale_and_confidence_metadata() {
        let harness = setup(default_settings(), default_body());
        let result = harness
            .provider
            .transcription("mai-transcribe-2")
            .do_generate(&input())
            .await
            .unwrap();
        assert_match_object(
            &json_of(&result),
            &json!({
                "text": "Hello world.",
                "durationInSeconds": 2.0,
                "language": "en",
                "segments": [{ "text": "Hello world.", "startSecond": 0.04, "endSecond": 1.54 }],
                "providerMetadata": { "azure": { "phrases": default_body()["phrases"] } },
                "warnings": [],
                "response": { "modelId": "mai-transcribe-2", "body": default_body() }
            }),
        );
        assert!(result.response.timestamp.is_some());
    }

    /// upstream: @ai-sdk/azure src/azure-transcription-model.test.ts › Speech responses › accepts missing or null timestamps (%s)
    #[tokio::test]
    async fn accepts_missing_or_null_timestamps_s() {
        // `None` is `undefined` (the key is absent); `Some(null)` is `null`.
        for missing in [None, Some(Value::Null)] {
            let mut body = Map::new();
            body.insert(
                "combinedPhrases".into(),
                json!([{ "text": "Hello world." }]),
            );
            let mut phrase = Map::new();
            phrase.insert("text".into(), json!("Hello world."));
            phrase.insert("locale".into(), json!("en-US"));
            if let Some(value) = &missing {
                body.insert("durationMilliseconds".into(), value.clone());
                for key in [
                    "offsetMilliseconds",
                    "durationMilliseconds",
                    "words",
                    "speaker",
                ] {
                    phrase.insert(key.into(), value.clone());
                }
            }
            body.insert("phrases".into(), json!([phrase]));
            let harness = setup(default_settings(), Value::Object(body));
            let result = harness
                .provider
                .transcription("mai-transcribe-2")
                .do_generate(&with_options(
                    input(),
                    json!({ "azure": { "timestamps": "none" } }),
                ))
                .await
                .unwrap();
            assert_eq!(result.text, "Hello world.");
            assert!(result.segments.is_empty());
            assert_eq!(result.duration_in_seconds, None);
        }
    }

    /// upstream: @ai-sdk/azure src/azure-transcription-model.test.ts › Speech responses › accepts empty transcripts and absent phrases
    #[tokio::test]
    async fn accepts_empty_transcripts_and_absent_phrases() {
        let harness = setup(
            default_settings(),
            json!({ "combinedPhrases": [], "phrases": null }),
        );
        let result = harness
            .provider
            .transcription("mai-transcribe-2")
            .do_generate(&input())
            .await
            .unwrap();
        assert_match_object(&json_of(&result), &json!({ "text": "", "segments": [] }));
        assert_eq!(result.language, None);
    }

    /// upstream: @ai-sdk/azure src/azure-transcription-model.test.ts › Speech responses › handles phrase locales %j
    #[tokio::test]
    async fn handles_phrase_locales_j() {
        let rows: Vec<(Vec<&str>, Option<&str>)> = vec![
            (vec!["en-US", "en-GB"], Some("en")),
            (vec!["en-US", "de-DE"], None),
            (vec!["fil"], None),
            (vec!["en", "fil"], None),
            (vec!["en-US", "yue"], None),
        ];
        for (locales, expected) in rows {
            let mut body = default_body();
            body["phrases"] = Value::Array(
                locales
                    .iter()
                    .map(|locale| json!({ "text": "text", "locale": locale }))
                    .collect(),
            );
            let harness = setup(default_settings(), body);
            let result = harness
                .provider
                .transcription("mai-transcribe-2")
                .do_generate(&input())
                .await
                .unwrap();
            assert_eq!(result.language.as_deref(), expected, "{locales:?}");
        }
    }

    /// upstream: @ai-sdk/azure src/azure-transcription-model.test.ts › Speech responses › returns structured API errors for HTTP %s
    #[tokio::test]
    async fn returns_structured_api_errors_for_http_s() {
        for status in [400u16, 429, 500] {
            let harness = setup(default_settings(), default_body());
            harness.respond_once(Canned {
                status,
                headers: vec![
                    ("content-type".into(), "application/json".into()),
                    ("x-request-id".into(), "request-1".into()),
                ],
                body: serde_json::to_vec(&json!({
                    "error": { "code": "InvalidRequest", "message": "Bad audio" }
                }))
                .unwrap(),
            });
            let error = api_call_error(
                harness
                    .provider
                    .transcription("mai-transcribe-2")
                    .do_generate(&input())
                    .await
                    .unwrap_err(),
            );
            assert_eq!(error.message, "Bad audio");
            assert_eq!(error.status_code, Some(status));
            assert_eq!(error.is_retryable, status != 400);
            assert_eq!(
                error
                    .response_headers
                    .as_ref()
                    .and_then(|headers| headers.get("x-request-id"))
                    .map(String::as_str),
                Some("request-1")
            );
        }
    }

    /// upstream: @ai-sdk/azure src/azure-transcription-model.test.ts › Speech responses › preserves non-JSON error bodies
    #[tokio::test]
    async fn preserves_non_json_error_bodies() {
        let harness = setup(default_settings(), default_body());
        harness.respond_once(Canned {
            status: 503,
            headers: vec![("content-type".into(), "text/plain;charset=UTF-8".into())],
            body: b"Unavailable".to_vec(),
        });
        let error = api_call_error(
            harness
                .provider
                .transcription("mai-transcribe-2")
                .do_generate(&input())
                .await
                .unwrap_err(),
        );
        assert_eq!(error.response_body.as_deref(), Some("Unavailable"));
        assert_eq!(error.status_code, Some(503));
    }

    /// upstream: @ai-sdk/azure src/azure-transcription-model.test.ts › Speech responses › reads the top-level error message documented by the Speech REST API
    #[tokio::test]
    async fn reads_the_top_level_error_message_documented_by_the_speech_rest_api() {
        let harness = setup(default_settings(), default_body());
        harness.respond_once(Canned {
            status: 400,
            headers: vec![("content-type".into(), "application/json".into())],
            body: serde_json::to_vec(&json!({ "code": "InvalidRequest", "message": "Bad audio" }))
                .unwrap(),
        });
        let error = api_call_error(
            harness
                .provider
                .transcription("mai-transcribe-2")
                .do_generate(&input())
                .await
                .unwrap_err(),
        );
        assert_eq!(error.message, "Bad audio");
        assert_eq!(error.status_code, Some(400));
    }

    /// upstream: @ai-sdk/azure src/azure-transcription-model.test.ts › Speech responses › rejects malformed success responses
    #[tokio::test]
    async fn rejects_malformed_success_responses() {
        let harness = setup(default_settings(), json!({ "unrelated": true }));
        let error = harness
            .provider
            .transcription("mai-transcribe-2")
            .do_generate(&input())
            .await
            .unwrap_err();
        api_call_error(error);
    }

    /// upstream: @ai-sdk/azure src/azure-transcription-model.test.ts › Speech responses › preserves abort errors
    #[tokio::test]
    async fn preserves_abort_errors() {
        let harness = setup(default_settings(), default_body());
        harness.reject_next();
        let error = harness
            .provider
            .transcription("mai-transcribe-2")
            .do_generate(&input())
            .await
            .unwrap_err();
        assert!(matches!(error, AiMuxError::Aborted(_)), "{error:?}");
    }
}

mod streaming_and_serialization {
    use super::*;

    /// upstream: @ai-sdk/azure src/azure-transcription-model.test.ts › streaming and serialization › rejects Speech streaming before sending audio
    #[tokio::test]
    async fn rejects_speech_streaming_before_sending_audio() {
        let harness = setup(default_settings(), default_body());
        let options = TranscriptionStreamOptions {
            audio: Box::pin(futures::stream::empty::<AudioChunk>()),
            input_audio_format: InputAudioFormat {
                format_type: "audio/pcm".into(),
                rate: Some(24000),
            },
            provider_options: None,
            abort_signal: None,
            headers: None,
            include_raw_chunks: false,
            timeout: None,
        };
        let model = harness.provider.transcription("mai-transcribe-2");
        let stream = model
            .do_stream(options)
            .expect("the Azure transcription model has a streaming method");
        let error = stream
            .await
            .expect_err("Speech streaming is rejected before any audio is sent");
        assert!(
            matches!(error, AiMuxError::UnsupportedFunctionality(_)),
            "{error:?}"
        );
        assert_eq!(harness.fetch_calls(), 0);
    }
}

mod recorded_mai_transcribe_2_responses {
    use super::*;

    /// upstream: @ai-sdk/azure src/azure-transcription-model.test.ts › recorded MAI-Transcribe-2 responses › maps diarized word-level results into segments and metadata
    #[tokio::test]
    async fn maps_diarized_word_level_results_into_segments_and_metadata() {
        let body: Value = serde_json::from_str(include_str!(
            "fixtures/azure-speech-mai-transcribe-2-word-diarization.1.json"
        ))
        .unwrap();
        let harness = setup(default_settings(), body.clone());
        let result = harness
            .provider
            .transcription("mai-transcribe-2")
            .do_generate(&with_options(
                input(),
                json!({ "azure": { "timestamps": "word", "diarization": { "enabled": true } } }),
            ))
            .await
            .unwrap();
        assert_eq!(result.text, body["combinedPhrases"][0]["text"]);
        assert_eq!(result.language.as_deref(), Some("en"));
        assert_eq!(result.duration_in_seconds, Some(17.579));
        assert_js_equal(
            &json_of(&result.segments),
            &json!([
                {
                    "text": "Um, so, uh, did you finish the quarterly report for Versal yet?",
                    "startSecond": 0.12,
                    "endSecond": 4.32
                },
                {
                    "text": "Yeah, I, uh, I sent it over this morning. It covers the AI gateway numbers.",
                    "startSecond": 4.48,
                    "endSecond": 9.439
                },
                {
                    "text": "Great. Uh, can you, like, add the transcription pricing too?",
                    "startSecond": 9.68,
                    "endSecond": 13.8
                },
                {
                    "text": "Sure. I'll, um, update it by Friday.",
                    "startSecond": 14.04,
                    "endSecond": 17.159
                }
            ]),
        );
        let metadata = json_of(&result.provider_metadata);
        let phrases = &metadata["azure"]["phrases"];
        let speakers: Vec<Value> = phrases
            .as_array()
            .unwrap()
            .iter()
            .map(|phrase| phrase["speaker"].clone())
            .collect();
        assert_js_equal(&Value::Array(speakers), &json!([0, 1, 0, 1]));
        assert_js_equal(
            &phrases[0]["words"][0],
            &json!({ "text": "Um,", "offsetMilliseconds": 120, "durationMilliseconds": 220 }),
        );
    }

    /// upstream: @ai-sdk/azure src/azure-transcription-model.test.ts › recorded MAI-Transcribe-2 responses › maps a timestamps=none result to one segment spanning the audio
    #[tokio::test]
    async fn maps_a_timestamps_none_result_to_one_segment_spanning_the_audio() {
        let body: Value = serde_json::from_str(include_str!(
            "fixtures/azure-speech-mai-transcribe-2-no-timestamps.1.json"
        ))
        .unwrap();
        let harness = setup(default_settings(), body);
        let result = harness
            .provider
            .transcription("mai-transcribe-2")
            .do_generate(&with_options(
                input(),
                json!({ "azure": { "timestamps": "none" } }),
            ))
            .await
            .unwrap();
        assert_eq!(result.segments.len(), 1);
        assert_match_object(
            &json_of(&result.segments[0]),
            &json!({ "startSecond": 0.0, "endSecond": 17.579 }),
        );
    }
}

mod provider {
    use super::*;

    /// upstream: @ai-sdk/azure src/azure-transcription-model.test.ts › provider › exposes transcriptionModel as an alias of transcription
    #[tokio::test]
    async fn exposes_transcription_model_as_an_alias_of_transcription() {
        let harness = setup(default_settings(), default_body());
        let model = harness
            .provider
            .transcription_model("mai-transcribe-2")
            .expect("the Azure provider offers transcription models")
            .expect("the transcription model is created");
        model.do_generate(&input()).await.unwrap();
        assert!(harness.request().url.contains("/speechtotext/"));
    }
}
