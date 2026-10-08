//! Synthetic contracts from official implementations, never presented as live recordings.
use std::collections::HashMap;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use aimux_core::decision_model::*;
use aimux_core::recording::{self, RingRecorder};
use aimux_core::replay::{MockDecisionReplayModel, replay_decision_with_model};
use aimux_core::{AbortSignal, AiMuxError};
use aimux_providers::{ProviderOptions, provider_handle};
use serde_json::{Value, json};
use serial_test::serial;
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

mod common;

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/runtime_decisions.json")).unwrap()
}
fn options() -> DecisionCallOptions {
    serde_json::from_value(fixture()["options"].clone()).unwrap()
}
fn model(provider: &str, server: &MockServer) -> Box<dyn DecisionModel> {
    provider_handle(
        provider,
        Some("runtime-secret".into()),
        Some(ProviderOptions {
            base_url: Some(format!("{}/v1", server.uri())),
            ..Default::default()
        }),
    )
    .unwrap()
    .decision_model(if provider.starts_with("cloudflare") {
        "@cf/cloudflare/clef"
    } else {
        "served-decision"
    })
    .unwrap()
}
struct Stop;
impl Drop for Stop {
    fn drop(&mut self) {
        recording::init_recording(None);
    }
}

async fn replay(
    provider: &str,
    server: &MockServer,
    request: DecisionCallOptions,
    count: usize,
) -> DecisionResult {
    let model = model(provider, server);
    let recorder = Arc::new(RingRecorder::new());
    recording::init_recording(Some(recorder.clone()));
    let _stop = Stop;
    let result = decide(model.as_ref(), request.clone()).await.unwrap();
    recording::init_recording(None);
    let recordings = recorder.completed();
    assert_eq!(recordings.len(), 1);
    assert_eq!(recordings[0].exchanges.len(), count);
    assert!(recordings[0].exchanges.iter().all(|e| e.attempt == 1));
    assert!(
        !serde_json::to_string(&recordings)
            .unwrap()
            .contains("runtime-secret")
    );
    let offline =
        MockDecisionReplayModel::new(provider, model.model_id(), recordings.clone()).unwrap();
    let replayed = decide(&offline, request).await.unwrap();
    assert_eq!(
        serde_json::to_value(&replayed.answers).unwrap(),
        serde_json::to_value(&result.answers).unwrap()
    );
    let server = MockServer::start().await;
    common::replay::mount_recording(&server, &recordings[0]).await;
    let mut snapshot = recordings[0].provider.clone();
    snapshot.base_url = Some(format!("{}/v1", server.uri()));
    let rebuilt = aimux_providers::rebuild_decision_provider(&snapshot, Some("new-key")).unwrap();
    let again = replay_decision_with_model(&recordings[0], rebuilt.as_ref())
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(again.answers).unwrap(),
        serde_json::to_value(&result.answers).unwrap()
    );
    assert_eq!(server.received_requests().await.unwrap().len(), count);
    result
}

#[tokio::test]
#[serial]
async fn official_systemone_providers_record_and_replay_their_own_routes() {
    for provider in [
        "ollama",
        "llamacpp",
        "localai",
        "laya",
        "vllm",
        "sglang",
        "cloudflare",
        "cloudflare_workers_ai",
    ] {
        let server = MockServer::start().await;
        let mut expected = fixture()["request"].clone();
        let mut response = fixture()["response"].clone();
        let endpoint = if provider.starts_with("cloudflare") {
            expected["model"] = json!("clef");
            response = json!({"success":true,"result":response,"errors":[],"messages":[]});
            "/run/@cf/cloudflare/clef"
        } else {
            "/v1/systemone"
        };
        Mock::given(method("POST"))
            .and(path(endpoint))
            .and(header("authorization", "Bearer runtime-secret"))
            .and(body_json(expected))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response))
            .expect(1)
            .mount(&server)
            .await;
        let result = replay(provider, &server, options(), 1).await;
        assert_eq!(result.provider, provider);
        assert_eq!(result.usage.unwrap().input_tokens.total, Some(20));
        assert_eq!(result.response.unwrap().body.unwrap(), response);
    }
}

#[tokio::test]
#[serial]
async fn media_is_native_per_runtime_and_recorded_for_replay() {
    let full: Value = serde_json::from_str(include_str!(
        "../../contract-tests/fixtures/decision-openai-full.json"
    ))
    .unwrap();
    let mut image = full["options"]["images"][0].clone();
    image.as_object_mut().unwrap().remove("detail");
    for provider in [
        "ollama",
        "llamacpp",
        "localai",
        "vllm",
        "sglang",
        "cloudflare",
    ] {
        let server = MockServer::start().await;
        let mut request = options();
        request.images = vec![serde_json::from_value(image.clone()).unwrap()];
        let mut expected = fixture()["request"].clone();
        let encoded = image["data"]["Base64"].as_str().unwrap();
        let data_url = format!("data:image/png;base64,{encoded}");
        expected["images"] = match provider {
            "ollama" => json!([encoded]),
            "sglang" => json!([{"url":data_url}]),
            _ => json!([data_url]),
        };
        let mut response = fixture()["response"].clone();
        if provider == "cloudflare" {
            expected["model"] = json!("clef");
            response = json!({"success":true,"result":response});
        }
        Mock::given(body_json(expected))
            .respond_with(ResponseTemplate::new(200).set_body_json(response))
            .mount(&server)
            .await;
        replay(provider, &server, request, 1).await;
    }
}

#[tokio::test]
#[serial]
async fn conditional_skips_and_abstention_do_not_become_false_answers() {
    for provider in ["vllm", "laya"] {
        let server = MockServer::start().await;
        let mut request = options();
        let mut response = fixture()["response"].clone();
        let mut expected = fixture()["request"].clone();
        if provider == "vllm" {
            request.provider_options = Some(HashMap::from([(
                "vllm".into(),
                json!({"samples":"auto","steps":4,"questions":{"severity":{"ask_if":{"team":["billing"]}}}}),
            )]));
            expected["samples"] = json!("auto");
            expected["steps"] = json!(4);
            expected["questions"]["severity"]["ask_if"] = json!({"team":["billing"]});
            response["answers"]["severity"] = Value::Null;
            response["diagnostics"] = json!({"skipped":{"severity":{"because":"team","was":"support","wanted":["billing"]}}});
        } else {
            request.provider_options = Some(HashMap::from([(
                "laya".into(),
                json!({"min_confidence":0.8,"lang":"en"}),
            )]));
            expected["min_confidence"] = json!(0.8);
            expected["lang"] = json!("en");
            response["answers"]["severity"]["abstention"] = json!("abstained");
            response["answers"]["severity"]["action"] = json!({"handoff":0.9});
        }
        Mock::given(body_json(expected))
            .respond_with(ResponseTemplate::new(200).set_body_json(response))
            .mount(&server)
            .await;
        let result = replay(provider, &server, request, 1).await;
        assert!(matches!(
            (&result.answers["severity"], provider),
            (DecisionAnswer::Skipped, "vllm") | (DecisionAnswer::Abstention, "laya")
        ));
    }
}

#[tokio::test]
#[serial]
async fn sglang_native_decisions_keeps_label_mass_separate_from_confidence() {
    let server = MockServer::start().await;
    let mut request = options();
    request.provider_options = Some(HashMap::from([(
        "sglang".into(),
        json!({"protocol":"decisions","temperature":0.7,"return_prompt_token_ids":true,"prompt_format_version":1}),
    )]));
    let response = json!({"object":"decisions","model":"served-decision","prompt_format_version":1,"answers":{
        "urgent":{"type":"yes_no","probabilities":{"yes":0.75,"no":0.25},"label_mass":0.1},
        "team":{"type":"choice","choice":"support","probabilities":{"billing":0.25,"support":0.75},"label_mass":0.15,"prompt_token_ids":[1,2],"label_token_ids":[3,4]},
        "severity":{"type":"score","score":1.5,"probabilities":{"0":0.125,"1":0.25,"2":0.625},"label_mass":0.2}
    },"usage":{"prompt_tokens":20,"completion_tokens":0,"total_tokens":20}});
    Mock::given(path("/v1/decisions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(response.clone()))
        .mount(&server)
        .await;
    let result = replay("sglang", &server, request, 1).await;
    assert_eq!(
        result.probability_source,
        DecisionProbabilitySource::LogitScoring
    );
    assert!(matches!(
        result.answers["team"],
        DecisionAnswer::Choice {
            confidence: None,
            ..
        }
    ));
    assert_eq!(result.response.unwrap().body.unwrap(), response);
    let wire: Value = server.received_requests().await.unwrap()[0]
        .body_json()
        .unwrap();
    assert_eq!(
        wire["questions"][0],
        json!({"id":"urgent","type":"yes_no","question":"Needs a response today?"})
    );
    assert_eq!(
        wire["questions"][1]["options"],
        json!([{"name":"billing","description":"Invoices"},{"name":"support"}])
    );
    assert_eq!(wire["temperature"], 0.7);
}

fn scoring_options(provider: &str) -> DecisionCallOptions {
    let mut request = options();
    request.provider_options = Some(HashMap::from([(
        provider.into(),
        json!({"protocol":if provider=="sglang" {"score"} else {"generative_scoring"},"model_revision":"weights-commit","tokenizer_revision":"tokenizer-commit","prompt_format_version":"test-v1","encoded":{
            "urgent":{"prompt_token_ids":[11,12],"label_token_ids":[1,2]},
            "team":{"prompt_token_ids":[21,22],"label_token_ids":[3,4]},
            "severity":{"prompt_token_ids":[31,32],"label_token_ids":[5,6,7]}
        }}),
    )]));
    request
}

#[tokio::test]
#[serial]
async fn explicit_scoring_preserves_every_exchange_usage_and_replays() {
    for provider in ["sglang", "vllm"] {
        let server = MockServer::start().await;
        if provider == "sglang" {
            Mock::given(path("/v1/score")).and(body_json(json!({"model":"served-decision","query":[],"items":[[11,12],[21,22],[31,32]],"label_token_ids":[[1,2],[3,4],[5,6,7]],"apply_softmax":true,"temperature":1.0,"return_token_logprobs":true})))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"model":"served-decision","scores":[[0.75,0.25],[0.25,0.75],[0.125,0.25,0.625]],"token_logprobs":[[-1.0,-2.0],[-2.0,-1.0],[-3.0,-2.0,-1.0]],"usage":{"prompt_tokens":20,"completion_tokens":0}}))).mount(&server).await;
        } else {
            Mock::given(path("/generative_scoring")).respond_with(|request:&Request| {
                let body:Value=request.body_json().unwrap();
                assert_eq!(body["add_special_tokens"],false); assert_eq!(body["apply_softmax"],true);assert_eq!(body["query"],json!([]));
                let index=body["label_token_ids"][0].as_u64().unwrap() as usize;
                let score=[0.0,0.75,0.25,0.25,0.75,0.125,0.25,0.625][index];
                ResponseTemplate::new(200).set_body_json(json!({"model":"served-decision","data":[{"index":0,"score":score}],"usage":{"prompt_tokens":2,"completion_tokens":1,"total_tokens":3}}))
            }).mount(&server).await;
        }
        let result = replay(
            provider,
            &server,
            scoring_options(provider),
            if provider == "sglang" { 1 } else { 7 },
        )
        .await;
        assert!(matches!(
            result.answers["severity"],
            DecisionAnswer::Score {
                expected_value: 1.5,
                confidence: None,
                ..
            }
        ));
        assert_eq!(
            result.probability_source,
            DecisionProbabilitySource::LogitScoring
        );
        if provider == "vllm" {
            let usage = result.usage.unwrap();
            assert_eq!(usage.input_tokens.total, Some(14));
            assert_eq!(usage.output_tokens.total, Some(7));
        }
    }
}

#[tokio::test]
#[serial]
async fn malformed_native_answers_keep_http_context_and_never_fall_back() {
    for provider in [
        "ollama",
        "llamacpp",
        "localai",
        "laya",
        "vllm",
        "sglang",
        "cloudflare",
    ] {
        let server = MockServer::start().await;
        let mut response = fixture()["response"].clone();
        response["answers"]["team"]["probabilities"]["support"] = json!(0.3);
        if provider == "cloudflare" {
            response = json!({"success":true,"result":response});
        }
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(response))
            .mount(&server)
            .await;
        assert!(matches!(
            decide(model(provider, &server).as_ref(), options()).await,
            Err(AiMuxError::ApiCall(_))
        ));
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
}

#[tokio::test]
#[serial]
async fn unsupported_fields_and_bad_encodings_fail_before_http() {
    let server = MockServer::start().await;
    for provider in ["sglang", "vllm"] {
        for mutation in [0, 1, 2, 3] {
            let mut request = scoring_options(provider);
            let opts = request
                .provider_options
                .as_mut()
                .unwrap()
                .get_mut(provider)
                .unwrap();
            match mutation {
                0 => opts["encoded"]["urgent"]["label_token_ids"] = json!([1, 1]),
                1 => {
                    opts["encoded"].as_object_mut().unwrap().remove("team");
                }
                2 => opts["tokenizer_revision"] = json!(""),
                _ => opts["encoded"]["severity"]["prompt_token_ids"] = json!([]),
            }
            assert!(matches!(
                decide(model(provider, &server).as_ref(), request).await,
                Err(AiMuxError::InvalidArgument(_))
            ));
        }
    }
    let mut request = options();
    request.provider_options = Some(HashMap::from([(
        "ollama".into(),
        json!({"temperature":0.7}),
    )]));
    assert!(
        decide(model("ollama", &server).as_ref(), request)
            .await
            .is_err()
    );
    let mut request = options();
    request.provider_options = Some(HashMap::from([("vllm".into(), json!({"ask":["urgent"]}))]));
    assert!(
        decide(model("vllm", &server).as_ref(), request)
            .await
            .is_err()
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
#[serial]
async fn retry_and_cancellation_use_shared_transport() {
    let server = MockServer::start().await;
    let calls = Arc::new(AtomicUsize::new(0));
    let responder = calls.clone();
    Mock::given(path("/v1/systemone"))
        .respond_with(move |_: &Request| {
            if responder.fetch_add(1, Ordering::SeqCst) == 0 {
                ResponseTemplate::new(429)
                    .insert_header("retry-after", "0")
                    .set_body_json(json!({"error":{"message":"busy"}}))
            } else {
                ResponseTemplate::new(200).set_body_json(&fixture()["response"])
            }
        })
        .mount(&server)
        .await;
    let mut request = options();
    request.max_retries = Some(1);
    decide(model("ollama", &server).as_ref(), request)
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let delayed = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_secs(5))
                .set_body_json(&fixture()["response"]),
        )
        .mount(&delayed)
        .await;
    let controller = AbortSignal::new();
    let mut request = options();
    request.abort_signal = Some(controller.clone());
    let model = model("localai", &delayed);
    let (result, ()) = tokio::join!(decide(model.as_ref(), request), async {
        tokio::time::sleep(Duration::from_millis(40)).await;
        controller.abort();
    });
    assert!(matches!(result, Err(AiMuxError::Aborted(_))));
}

#[test]
fn local_factories_and_wrapper_factories_preserve_credentials_for_replay() {
    use aimux_core::Provider;
    for provider in ["ollama", "llamacpp", "localai", "laya", "sglang", "vllm"] {
        let handle = provider_handle(
            provider,
            None,
            Some(ProviderOptions {
                base_url: Some("http://127.0.0.1:1/v1".into()),
                ..Default::default()
            }),
        )
        .unwrap();
        let model = handle.decision_model("served-alias").unwrap();
        assert_eq!(model.provider(), provider);
        let record = model.config_snapshot();
        let rebuilt = aimux_providers::rebuild_decision_provider(&record, None).unwrap();
        assert_eq!(rebuilt.provider(), provider);
    }
    let provider =
        aimux_providers::VllmProvider::new(aimux_providers::VllmConfig::new("explicit-key"));
    let model = provider.decision_model("served-alias").unwrap();
    assert_eq!(model.config_snapshot().api_key_source, "explicit");
    assert!(aimux_providers::rebuild_decision_provider(&model.config_snapshot(), None).is_err());
}

#[tokio::test]
#[serial]
async fn scoring_rejects_wrong_dimensions_and_inconsistent_probabilities() {
    for scores in [
        json!([[0.75, 0.25]]),
        json!([[0.75, 0.25], [0.25], [0.125, 0.25, 0.625]]),
        json!([[0.75, 0.75], [0.25, 0.75], [0.125, 0.25, 0.625]]),
    ] {
        let server = MockServer::start().await;
        Mock::given(path("/v1/score"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"model":"served-decision","scores":scores})),
            )
            .mount(&server)
            .await;
        assert!(matches!(
            decide(model("sglang", &server).as_ref(), scoring_options("sglang")).await,
            Err(AiMuxError::ApiCall(_))
        ));
    }
    let server = MockServer::start().await;
    Mock::given(path("/generative_scoring"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"model":"served-decision","data":[{"index":0,"score":0.9}]})),
        )
        .mount(&server)
        .await;
    assert!(matches!(
        decide(model("vllm", &server).as_ref(), scoring_options("vllm")).await,
        Err(AiMuxError::InvalidResponseData(_))
    ));
    assert_eq!(server.received_requests().await.unwrap().len(), 7);
}

#[tokio::test]
#[serial]
async fn runtime_errors_preserve_native_message_code_and_status() {
    for (provider, body, message) in [
        (
            "ollama",
            json!({"error":"missing decision model"}),
            "missing decision model",
        ),
        (
            "laya",
            json!({"detail":"unsupported checkpoint"}),
            "unsupported checkpoint",
        ),
        (
            "cloudflare",
            json!({"success":false,"errors":[{"code":1234,"message":"wrong model"}]}),
            "wrong model",
        ),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(400).set_body_json(body))
            .mount(&server)
            .await;
        let AiMuxError::ApiCall(error) = decide(model(provider, &server).as_ref(), options())
            .await
            .unwrap_err()
        else {
            panic!("expected native API error")
        };
        assert_eq!(error.status_code, Some(400));
        assert_eq!(error.message, message);
        assert!(!error.is_retryable);
        if provider == "cloudflare" {
            assert_eq!(error.provider_code.as_deref(), Some("1234"));
        }
    }
}
