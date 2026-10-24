use crate::shared::{Endpoint, EndpointConfig, ResolveEndpoint};
use crate::test_support::mock_fetch::{Canned, MockFetch, RouteFetch};
use crate::vertex::{
    VertexProvider, VertexProviderSettings, VertexTranscriptionModel, create_google_vertex,
};
use aimux_core::Provider;
use aimux_core::error::AiMuxError;
use aimux_core::shared::SharedProviderOptions;
use aimux_core::transcription_model::{AudioInput, TranscriptionCallOptions, TranscriptionModel};
use aimux_provider_utils::{HeaderMapOpt, Resolvable};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// 8 bytes of audio; base64 -> 'AQIDBAUGBwg='.
const AUDIO_DATA: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];
const AUDIO_BASE64: &str = "AQIDBAUGBwg=";

const URL: &str = "https://us-central1-speech.googleapis.com/v2/projects/test-project/locations/us-central1/recognizers/_:recognize";
const US_URL: &str = "https://us-speech.googleapis.com/v2/projects/test-project/locations/us/recognizers/_:recognize";
const GLOBAL_URL: &str = "https://speech.googleapis.com/v2/projects/test-project/locations/global/recognizers/_:recognize";

/// The pinned user-agent part of the package (upstream mocks `0.0.0-test`).
const USER_AGENT_PART: &str = "ai-sdk-google-vertex/5.0.100";

fn default_body() -> Value {
    json!({
        "results": [{
            "alternatives": [{
                "transcript": "hello world",
                "words": [
                    {"word": "hello", "startOffset": "0s", "endOffset": "0.500s"},
                    {"word": "world", "startOffset": "0.500s", "endOffset": "1s"},
                ],
            }],
            "languageCode": "en-US",
        }],
        "metadata": {"totalBilledDuration": "1s"},
    })
}

fn json_response(body: &Value, headers: &[(&str, &str)]) -> Canned {
    let body = serde_json::to_vec(body).unwrap();
    let mut all = vec![
        ("content-type".to_string(), "application/json".to_string()),
        ("content-length".to_string(), body.len().to_string()),
    ];
    all.extend(headers.iter().map(|(k, v)| (k.to_string(), v.to_string())));
    Canned {
        status: 200,
        headers: all,
        body,
    }
}

/// The AI SDK test server: `url` answers `response`, anything else 404.
fn test_server(url: &'static str, response: Canned) -> Arc<RouteFetch> {
    RouteFetch::new(move |seen| {
        if seen.url == url {
            response.clone()
        } else {
            Canned {
                status: 404,
                headers: Vec::new(),
                body: b"Not found".to_vec(),
            }
        }
    })
}

/// `prepareJsonResponse({ headers })`.
fn prepare_json_response(headers: &[(&str, &str)]) -> Arc<RouteFetch> {
    test_server(URL, json_response(&default_body(), headers))
}

/// `createGoogleVertex({ project: 'test-project', location: 'us-central1' })`.
/// aimux has one entry point, and it authenticates: an access token stands in
/// for upstream's credential-less base provider.
fn provider_with(fetch: &Arc<RouteFetch>, headers: Option<HeaderMapOpt>) -> VertexProvider {
    create_google_vertex(VertexProviderSettings {
        project: Some("test-project".into()),
        location: Some("us-central1".into()),
        access_token: Some(Resolvable::Value("test-token".into())),
        headers: headers.map(Resolvable::Value),
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .unwrap()
}

/// `provider.transcription('chirp_2')`.
fn model(fetch: &Arc<RouteFetch>) -> Arc<dyn TranscriptionModel> {
    provider_with(fetch, None).transcription("chirp_2")
}

fn options(audio: AudioInput, provider_options: Option<Value>) -> TranscriptionCallOptions {
    let mut options = TranscriptionCallOptions::new(audio, "audio/wav");
    options.provider_options = provider_options
        .map(|value| serde_json::from_value::<SharedProviderOptions>(value).unwrap());
    options
}

fn audio() -> AudioInput {
    AudioInput::Binary(AUDIO_DATA.to_vec())
}

/// A base URL and headers fixed for every request (the upstream model config).
struct StaticEndpoint {
    base_url: String,
    headers: HeaderMapOpt,
}

#[async_trait::async_trait]
impl ResolveEndpoint for StaticEndpoint {
    async fn resolve(&self) -> Result<Endpoint, AiMuxError> {
        Ok(Endpoint {
            base_url: self.base_url.clone(),
            headers: self.headers.clone(),
        })
    }
}

mod do_generate {
    use super::*;

    /// upstream: @ai-sdk/google-vertex src/google-vertex-transcription-model.test.ts › doGenerate › should send the model, languageCodes, features and base64 content
    #[tokio::test]
    async fn should_send_the_model_language_codes_features_and_base64_content() {
        let server = prepare_json_response(&[]);
        model(&server)
            .do_generate(&options(audio(), None))
            .await
            .unwrap();
        assert_eq!(
            server.seen()[0].json_body(),
            json!({
                "config": {
                    "model": "chirp_2",
                    "languageCodes": ["auto"],
                    "autoDecodingConfig": {},
                    "features": {
                        "enableWordTimeOffsets": true,
                        "enableAutomaticPunctuation": true,
                    },
                },
                "content": AUDIO_BASE64,
            })
        );
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-transcription-model.test.ts › doGenerate › should target the regional Speech-to-Text endpoint
    #[tokio::test]
    async fn should_target_the_regional_speech_to_text_endpoint() {
        let server = prepare_json_response(&[]);
        model(&server)
            .do_generate(&options(audio(), None))
            .await
            .unwrap();
        assert_eq!(server.seen()[0].url, URL);
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-transcription-model.test.ts › doGenerate › should extract text, segments, language and duration
    #[tokio::test]
    async fn should_extract_text_segments_language_and_duration() {
        let server = prepare_json_response(&[]);
        let result = model(&server)
            .do_generate(&options(audio(), None))
            .await
            .unwrap();
        assert_eq!(result.text, "hello world");
        assert_eq!(
            serde_json::to_value(&result.segments).unwrap(),
            json!([
                {"text": "hello", "startSecond": 0.0, "endSecond": 0.5},
                {"text": "world", "startSecond": 0.5, "endSecond": 1.0},
            ])
        );
        // BCP-47 `en-US` is reduced to the ISO-639-1 `en`.
        assert_eq!(result.language.as_deref(), Some("en"));
        assert_eq!(result.duration_in_seconds, Some(1.0));
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-transcription-model.test.ts › doGenerate › should accept a base64 string audio input directly
    #[tokio::test]
    async fn should_accept_a_base64_string_audio_input_directly() {
        let server = prepare_json_response(&[]);
        model(&server)
            .do_generate(&options(AudioInput::Base64(AUDIO_BASE64.into()), None))
            .await
            .unwrap();
        assert_eq!(server.seen()[0].json_body()["content"], AUDIO_BASE64);
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-transcription-model.test.ts › doGenerate › should pass languageCodes and features from provider options
    #[tokio::test]
    async fn should_pass_language_codes_and_features_from_provider_options() {
        let server = prepare_json_response(&[]);
        model(&server)
            .do_generate(&options(
                audio(),
                Some(json!({
                    "googleVertex": {
                        "languageCodes": ["en-US", "fr-FR"],
                        "enableAutomaticPunctuation": false,
                        "enableWordTimeOffsets": false,
                    },
                })),
            ))
            .await
            .unwrap();
        // `toMatchObject`
        let body = server.seen()[0].json_body();
        assert_eq!(body["config"]["languageCodes"], json!(["en-US", "fr-FR"]));
        assert_eq!(
            body["config"]["features"]["enableAutomaticPunctuation"],
            json!(false)
        );
        assert_eq!(
            body["config"]["features"]["enableWordTimeOffsets"],
            json!(false)
        );
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-transcription-model.test.ts › doGenerate › should accept the legacy vertex provider options key
    #[tokio::test]
    async fn should_accept_the_legacy_vertex_provider_options_key() {
        let server = test_server(US_URL, json_response(&default_body(), &[]));
        model(&server)
            .do_generate(&options(audio(), Some(json!({"vertex": {"region": "us"}}))))
            .await
            .unwrap();
        assert_eq!(server.seen()[0].url, US_URL);
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-transcription-model.test.ts › doGenerate › should accept the google provider options key
    #[tokio::test]
    async fn should_accept_the_google_provider_options_key() {
        let server = test_server(US_URL, json_response(&default_body(), &[]));
        model(&server)
            .do_generate(&options(audio(), Some(json!({"google": {"region": "us"}}))))
            .await
            .unwrap();
        assert_eq!(server.seen()[0].url, US_URL);
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-transcription-model.test.ts › doGenerate › should prefer googleVertex provider options
    #[tokio::test]
    async fn should_prefer_google_vertex_provider_options() {
        let server = test_server(US_URL, json_response(&default_body(), &[]));
        model(&server)
            .do_generate(&options(
                audio(),
                Some(json!({
                    "googleVertex": {"region": "us"},
                    "vertex": {"region": "eu"},
                    "google": {"region": "asia-southeast1"},
                })),
            ))
            .await
            .unwrap();
        assert_eq!(server.seen()[0].url, US_URL);
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-transcription-model.test.ts › doGenerate › should route to a region override from provider options
    #[tokio::test]
    async fn should_route_to_a_region_override_from_provider_options() {
        let server = test_server(US_URL, json_response(&default_body(), &[]));
        model(&server)
            .do_generate(&options(
                audio(),
                Some(json!({"googleVertex": {"region": "us"}})),
            ))
            .await
            .unwrap();
        assert_eq!(server.seen()[0].url, US_URL);
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-transcription-model.test.ts › doGenerate › should use the unprefixed host for the global location
    #[tokio::test]
    async fn should_use_the_unprefixed_host_for_the_global_location() {
        let server = test_server(GLOBAL_URL, json_response(&default_body(), &[]));
        provider_with(&server, None)
            .transcription("latest_long")
            .do_generate(&options(
                audio(),
                Some(json!({"googleVertex": {"region": "global"}})),
            ))
            .await
            .unwrap();
        assert_eq!(server.seen()[0].url, GLOBAL_URL);
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-transcription-model.test.ts › doGenerate › should pass headers and user agent
    #[tokio::test]
    async fn should_pass_headers_and_user_agent() {
        let server = prepare_json_response(&[]);
        let provider_with_headers = provider_with(
            &server,
            Some(
                [(
                    "Custom-Provider-Header".to_string(),
                    Some("provider-header-value".to_string()),
                )]
                .into(),
            ),
        );
        let mut call = options(audio(), None);
        call.headers = Some(
            [(
                "Custom-Request-Header".to_string(),
                "request-header-value".to_string(),
            )]
            .into(),
        );
        provider_with_headers
            .transcription("chirp_2")
            .do_generate(&call)
            .await
            .unwrap();
        let seen = server.seen();
        let headers = &seen[0].headers;
        assert_eq!(
            headers.get("custom-provider-header").map(String::as_str),
            Some("provider-header-value")
        );
        assert_eq!(
            headers.get("custom-request-header").map(String::as_str),
            Some("request-header-value")
        );
        let user_agent = headers.get("user-agent").cloned().unwrap_or_default();
        assert!(user_agent.contains(USER_AGENT_PART), "{user_agent}");
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-transcription-model.test.ts › doGenerate › should return empty text and segments when there are no results
    #[tokio::test]
    async fn should_return_empty_text_and_segments_when_there_are_no_results() {
        let server = test_server(URL, json_response(&json!({}), &[]));
        let result = model(&server)
            .do_generate(&options(audio(), None))
            .await
            .unwrap();
        assert_eq!(result.text, "");
        assert!(result.segments.is_empty());
        assert_eq!(result.language, None);
        assert_eq!(result.duration_in_seconds, None);
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-transcription-model.test.ts › doGenerate › should omit segments without word time offsets
    #[tokio::test]
    async fn should_omit_segments_without_word_time_offsets() {
        let server = test_server(
            URL,
            json_response(
                &json!({
                    "results": [{
                        "alternatives": [{
                            "transcript": "hello world",
                            "words": [{"word": "hello"}, {"word": "world"}],
                        }],
                        "languageCode": "en-US",
                    }],
                }),
                &[],
            ),
        );
        let result = model(&server)
            .do_generate(&options(audio(), None))
            .await
            .unwrap();
        assert!(result.segments.is_empty());
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-transcription-model.test.ts › doGenerate › should canonicalize detected languages to ISO-639-1
    #[tokio::test]
    async fn should_canonicalize_detected_languages_to_iso_639_1() {
        let server = test_server(
            URL,
            json_response(
                &json!({
                    "results": [{
                        "alternatives": [{"transcript": "你好"}],
                        "languageCode": "cmn-Hans-CN",
                    }],
                }),
                &[],
            ),
        );
        let result = model(&server)
            .do_generate(&options(audio(), None))
            .await
            .unwrap();
        assert_eq!(result.language.as_deref(), Some("zh"));
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-transcription-model.test.ts › doGenerate › should omit detected languages without an ISO-639-1 code
    #[tokio::test]
    async fn should_omit_detected_languages_without_an_iso_639_1_code() {
        let server = test_server(
            URL,
            json_response(
                &json!({
                    "results": [{
                        "alternatives": [{"transcript": "hello world"}],
                        "languageCode": "ast-ES",
                    }],
                }),
                &[],
            ),
        );
        let result = model(&server)
            .do_generate(&options(audio(), None))
            .await
            .unwrap();
        assert_eq!(result.language, None);
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-transcription-model.test.ts › doGenerate › should parse billed duration
    #[tokio::test]
    async fn should_parse_billed_duration() {
        let server = test_server(
            URL,
            json_response(
                &json!({
                    "results": [{
                        "alternatives": [{"transcript": "hello world"}],
                        "languageCode": "en-US",
                    }],
                    "metadata": {"totalBilledDuration": "1.250s"},
                }),
                &[],
            ),
        );
        let result = model(&server)
            .do_generate(&options(audio(), None))
            .await
            .unwrap();
        assert_eq!(result.duration_in_seconds, Some(1.25));
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-transcription-model.test.ts › doGenerate › should include response data with timestamp, modelId and headers
    #[tokio::test]
    async fn should_include_response_data_with_timestamp_model_id_and_headers() {
        let server = prepare_json_response(&[("x-request-id", "test-request-id")]);
        // `new GoogleVertexTranscriptionModel('chirp_2', { provider, project,
        // location, headers: () => ({}) })`: the project and location come
        // from a provider with those settings.
        let custom_model = VertexTranscriptionModel::from_config(
            "chirp_2".to_string(),
            EndpointConfig::new(
                "google.vertex.transcription".to_string(),
                StaticEndpoint {
                    base_url: "https://us-central1-aiplatform.googleapis.com/v1beta1/projects/test-project/locations/us-central1/publishers/google".to_string(),
                    headers: HeaderMapOpt::new(),
                },
                Some(server.transport()),
            ),
            provider_with(&server, None).project_location("transcription"),
        );
        // aimux cannot inject `currentDate`: the timestamp is asserted to be
        // set, not to equal a fixed date.
        let before = aimux_core::Timestamp::now();
        let result = custom_model
            .do_generate(&options(audio(), None))
            .await
            .unwrap();
        let timestamp = result.response.timestamp.expect("timestamp");
        assert!(timestamp.unix_millis() >= before.unix_millis());
        assert_eq!(result.response.model_id.as_deref(), Some("chirp_2"));
        assert_eq!(
            result
                .response
                .headers
                .as_ref()
                .and_then(|headers| headers.get("x-request-id"))
                .map(String::as_str),
            Some("test-request-id")
        );
    }
}

mod region_validation {
    use super::*;

    /// upstream: @ai-sdk/google-vertex src/google-vertex-transcription-model.test.ts › region validation › rejects an injected per-request region under %s before fetching
    #[tokio::test]
    async fn rejects_an_injected_per_request_region_before_fetching() {
        for namespace in ["googleVertex", "vertex", "google"] {
            let fetch = MockFetch::new(Vec::new());
            let model = create_google_vertex(VertexProviderSettings {
                project: Some("test".into()),
                location: Some("us-central1".into()),
                fetch: Some(fetch.transport()),
                ..Default::default()
            })
            .unwrap()
            .transcription("chirp_3");
            let error = model
                .do_generate(&options(
                    audio(),
                    Some(json!({namespace: {"region": "user@internal:8080/#"}})),
                ))
                .await
                .unwrap_err();
            // `AI_InvalidArgumentError` with `argument: 'region'`: aimux's
            // invalid-argument error names the argument in its message.
            assert!(
                matches!(&error, AiMuxError::InvalidArgument { message, .. } if message.contains("region")),
                "{namespace}: {error:?}"
            );
            assert!(fetch.seen().is_empty(), "{namespace}: fetch was called");
        }
    }
}

// ── Older ports (previously `tests/vertex_transcription_test.rs`) ──
//
// Rust translation of the Google Vertex transcription model tests.
// Source: `reference/ai/packages/google-vertex/src/google-vertex-transcription-model.test.ts`

fn mock_audio() -> Vec<u8> {
    vec![1u8, 2, 3, 4, 5, 6, 7, 8]
}
fn wiremock_options(audio: Vec<u8>, mt: &str) -> TranscriptionCallOptions {
    TranscriptionCallOptions::new(AudioInput::Binary(audio), mt)
}

fn wiremock_default_body() -> Value {
    json!({
        "results": [{
            "alternatives": [{
                "transcript": "hello world",
                "words": [
                    {"word": "hello", "startOffset": "0s", "endOffset": "0.500s"},
                    {"word": "world", "startOffset": "0.500s", "endOffset": "1s"}
                ]
            }],
            "languageCode": "en-US"
        }],
        "metadata": {"totalBilledDuration": "1s"}
    })
}

fn make_provider(server_uri: &str) -> VertexProvider {
    create_google_vertex(VertexProviderSettings {
        access_token: Some("test-token".to_string().into()),
        project: Some("test-project".to_string()),
        location: Some("us-central1".to_string()),
        base_url: Some(server_uri.to_string()),
        ..Default::default()
    })
    .unwrap()
}

async fn mock_response(server: &MockServer, body: &Value, headers: &[(&str, &str)]) {
    let path_str = "/v2/projects/test-project/locations/us-central1/recognizers/_:recognize";
    let mut t = ResponseTemplate::new(200)
        .insert_header("content-type", "application/json")
        .set_body_json(body.clone());
    for (k, v) in headers {
        t = t.insert_header(*k, *v);
    }
    Mock::given(method("POST"))
        .and(path(path_str))
        .respond_with(t)
        .mount(server)
        .await;
}

/// partial: `should send the model, languageCodes, features and base64 content`; full port in src/vertex/tests/transcription_model.rs.
#[tokio::test]
async fn should_send_model_language_codes_features_and_base64() {
    let server = MockServer::start().await;
    mock_response(&server, &wiremock_default_body(), &[]).await;

    let provider = make_provider(&server.uri());
    let model = provider.transcription_model("chirp_2").unwrap().unwrap();

    model
        .do_generate(&wiremock_options(mock_audio(), "audio/wav"))
        .await
        .unwrap();

    let requests = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["config"]["model"], "chirp_2");
    assert_eq!(body["config"]["languageCodes"], json!(["auto"]));
    assert_eq!(body["config"]["autoDecodingConfig"], json!({}));
    assert_eq!(body["config"]["features"]["enableWordTimeOffsets"], true);
    assert_eq!(
        body["config"]["features"]["enableAutomaticPunctuation"],
        true
    );
    assert_eq!(body["content"], "AQIDBAUGBwg=");
}

/// partial: `should extract text, segments, language and duration`; full port in src/vertex/tests/transcription_model.rs.
#[tokio::test]
async fn should_extract_text_segments_language_and_duration() {
    let server = MockServer::start().await;
    mock_response(&server, &wiremock_default_body(), &[]).await;

    let provider = make_provider(&server.uri());
    let model = provider.transcription_model("chirp_2").unwrap().unwrap();

    let result = model
        .do_generate(&wiremock_options(mock_audio(), "audio/wav"))
        .await
        .unwrap();

    assert_eq!(result.text, "hello world");
    assert_eq!(result.segments.len(), 2);
    assert_eq!(result.segments[0].text, "hello");
    assert_eq!(result.segments[0].start_second, 0.0);
    assert_eq!(result.segments[0].end_second, 0.5);
    assert_eq!(result.segments[1].text, "world");
    assert_eq!(result.segments[1].start_second, 0.5);
    assert_eq!(result.segments[1].end_second, 1.0);
    assert_eq!(result.language, Some("en".to_string()));
    assert_eq!(result.duration_in_seconds, Some(1.0));
}

/// partial: `should pass headers and user agent`; full port in src/vertex/tests/transcription_model.rs.
#[tokio::test]
async fn should_pass_headers() {
    let server = MockServer::start().await;
    mock_response(&server, &wiremock_default_body(), &[]).await;

    let provider = create_google_vertex(VertexProviderSettings {
        access_token: Some("test-token".to_string().into()),
        project: Some("test-project".to_string()),
        location: Some("us-central1".to_string()),
        base_url: Some(server.uri()),
        headers: Some(
            [(
                "Custom-Provider-Header".to_string(),
                Some("provider-header-value".to_string()),
            )]
            .into_iter()
            .collect::<HashMap<_, _>>()
            .into(),
        ),
        ..Default::default()
    })
    .unwrap();
    let model = provider.transcription_model("chirp_2").unwrap().unwrap();

    let mut opts = wiremock_options(mock_audio(), "audio/wav");
    let mut rh = HashMap::new();
    rh.insert(
        "Custom-Request-Header".to_string(),
        "request-header-value".to_string(),
    );
    opts.headers = Some(rh);

    model.do_generate(&opts).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    let h = &requests[0].headers;
    assert_eq!(h.get("authorization").unwrap(), "Bearer test-token");
    assert_eq!(
        h.get("custom-provider-header").unwrap(),
        "provider-header-value"
    );
    assert_eq!(
        h.get("custom-request-header").unwrap(),
        "request-header-value"
    );
}

/// partial: `should include response data with timestamp, modelId and headers`; full port in src/vertex/tests/transcription_model.rs.
#[tokio::test]
async fn should_include_response_data() {
    let server = MockServer::start().await;
    mock_response(
        &server,
        &wiremock_default_body(),
        &[("x-request-id", "test-req")],
    )
    .await;

    let provider = make_provider(&server.uri());
    let model = provider.transcription_model("chirp_2").unwrap().unwrap();

    let result = model
        .do_generate(&wiremock_options(mock_audio(), "audio/wav"))
        .await
        .unwrap();

    assert!(result.response.timestamp.is_some());
    assert_eq!(result.response.model_id, Some("chirp_2".to_string()));
    assert_eq!(
        result
            .response
            .headers
            .unwrap()
            .get("x-request-id")
            .map(String::as_str),
        Some("test-req")
    );
}

/// partial: `should return empty text and segments when there are no results`; full port in src/vertex/tests/transcription_model.rs.
#[tokio::test]
async fn should_handle_no_results() {
    let server = MockServer::start().await;
    mock_response(&server, &json!({}), &[]).await;

    let provider = make_provider(&server.uri());
    let model = provider.transcription_model("chirp_2").unwrap().unwrap();

    let result = model
        .do_generate(&wiremock_options(mock_audio(), "audio/wav"))
        .await
        .unwrap();

    assert_eq!(result.text, "");
    assert!(result.segments.is_empty());
    assert_eq!(result.language, None);
    assert_eq!(result.duration_in_seconds, None);
}
