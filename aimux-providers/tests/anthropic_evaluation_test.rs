//! `decision_model` on the Anthropic provider: upstream's `evaluationModel`
//! (`@ai-sdk/anthropic@4.0.71 src/anthropic-provider.ts:212-216`), an
//! `EvaluationLanguageModel` over the Messages model, which the registry's
//! decision lookup falls back to (`ai@7.0.128
//! src/registry/provider-registry.ts:447-461`). Upstream has no test for it;
//! these pin the provider string and one decision through the Messages API.

#[path = "common/mock_fetch.rs"]
mod mock_fetch;

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::json;

use aimux_core::decision_model::{DecisionAnswer, DecisionCallOptions};
use aimux_core::provider::Provider;
use aimux_providers::anthropic::{AnthropicProviderSettings, create_anthropic};

use mock_fetch::{Canned, MockFetch};

fn options() -> DecisionCallOptions {
    serde_json::from_value(json!({
        "state": { "message": "A billing issue with a workaround." },
        "questions": {
            "department": {
                "type": "choice",
                "instructions": "Pick the team.",
                "criteria": { "technical": "Bugs", "billing": "Charges" }
            },
            "severity": {
                "type": "score",
                "instructions": ["Rate severity."],
                "criteria": ["Low", "Medium", "High"]
            }
        },
        "headers": null,
        "provider_options": null
    }))
    .unwrap()
}

fn answer() -> serde_json::Value {
    json!({
        "id": "msg_eval",
        "type": "message",
        "role": "assistant",
        "model": "claude-sonnet-4-5",
        "content": [{ "type": "text", "text": "{\"q0\": \"c1\", \"q1\": 1.25}" }],
        "stop_reason": "end_turn",
        "stop_sequence": null,
        "usage": { "input_tokens": 30, "output_tokens": 12 }
    })
}

fn provider(
    mock: &Arc<MockFetch>,
    name: Option<&str>,
) -> aimux_providers::anthropic::AnthropicProvider {
    create_anthropic(AnthropicProviderSettings {
        api_key: Some("test-key".into()),
        name: name.map(str::to_string),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap()
}

/// partial: `selects native structured output automatically for supported models`; full port in src/anthropic/tests/evaluation.rs.
#[tokio::test]
async fn decides_through_the_messages_api() {
    let mock = MockFetch::new(vec![Canned::json(&answer())]);
    let model = provider(&mock, None)
        .decision_model("claude-sonnet-4-5")
        .unwrap()
        .unwrap();
    assert_eq!(model.provider(), "anthropic.evaluation");
    assert_eq!(model.model_id(), "claude-sonnet-4-5");

    let result = model.do_decide(&options()).await.unwrap();
    assert_eq!(
        result.answers["department"],
        DecisionAnswer::Choice {
            choice: "billing".into(),
            probabilities: None
        }
    );
    assert_eq!(
        result.answers["severity"],
        DecisionAnswer::Score {
            score: 1.25,
            probabilities: None
        }
    );
    let seen = mock.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].url, "https://api.anthropic.com/v1/messages");
    let body = seen[0].json_body();
    assert_eq!(body["model"], "claude-sonnet-4-5");
    let format = &body["output_config"]["format"];
    assert_eq!(format["type"], "json_schema");
    assert_eq!(
        format["schema"]["properties"]["q0"]["enum"],
        json!(["c0", "c1"])
    );
    assert_eq!(format["schema"]["required"], json!(["q0", "q1"]));
}

#[test]
fn a_custom_name_keeps_its_prefix() {
    let mock = MockFetch::new(vec![]);
    let model = provider(&mock, Some("acme.messages"))
        .decision_model("claude-sonnet-4-5")
        .unwrap()
        .unwrap();
    assert_eq!(model.provider(), "acme.evaluation");
}

#[test]
fn the_registry_resolves_an_anthropic_decision_model() {
    let mock = MockFetch::new(vec![]);
    let providers: BTreeMap<String, Arc<dyn Provider>> = BTreeMap::from([(
        "anthropic".to_string(),
        Arc::new(provider(&mock, None)) as Arc<dyn Provider>,
    )]);
    let registry = aimux_core::create_provider_registry(providers, Default::default());
    let model = registry
        .decision_model("anthropic:claude-sonnet-4-5")
        .unwrap();
    assert_eq!(model.provider(), "anthropic.evaluation");
}
