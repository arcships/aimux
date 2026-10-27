use std::sync::Arc;

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::options::CallOptions;
use futures::StreamExt;
use serde_json::{Value, json};

use crate::azure::{AzureOpenAIProviderSettings, create_azure};
use crate::openai::{OpenAIProviderSettings, create_openai};
use crate::test_support::mock_fetch::{Canned, MockFetch};

const METHODS: [&str; 2] = ["generate", "stream"];
const URL: &str = "https://api.openai.com/v1";

fn user() -> Value {
    json!({"role": "user", "content": [{"type": "text", "text": "Question"}]})
}

fn wire_user() -> Value {
    json!({"role": "user", "content": [{"type": "input_text", "text": "Question"}]})
}

fn update(effort: &str) -> Value {
    json!({
        "role": "system",
        "content": "",
        "providerOptions": {"openai": {"reasoningEffortUpdate": effort}},
    })
}

fn wire_update(effort: &str) -> Value {
    json!({"type": "configuration_update", "reasoning": {"effort": effort}})
}

/// One `request(...)` of the upstream helper: the request body and the
/// warnings (of the result, or of the `stream-start` part), or the error, and
/// the number of requests the server saw.
struct Outcome {
    result: Result<(Value, Value), AiMuxError>,
    calls: usize,
}

impl Outcome {
    fn ok(self) -> (Value, Value) {
        match self.result {
            Ok(value) => value,
            Err(error) => panic!("request failed: {error:?}"),
        }
    }

    /// `rejects.toMatchObject({ name: 'AI_UnsupportedFunctionalityError',
    /// functionality })` and no request was sent.
    fn assert_unsupported(self, functionality: &str) {
        match &self.result {
            Err(AiMuxError::UnsupportedFunctionality(actual)) => {
                assert_eq!(actual, functionality);
            }
            other => panic!("expected UnsupportedFunctionality({functionality}), got {other:?}"),
        }
        assert_eq!(self.calls, 0);
    }
}

async fn request(method: &str, prompt: &Value, options: &Value) -> Outcome {
    request_with(method, prompt, options, "gpt-6-astra", false).await
}

async fn request_with(
    method: &str,
    prompt: &Value,
    options: &Value,
    model_id: &str,
    azure: bool,
) -> Outcome {
    let response = json!({"id": "resp_test", "created_at": 0, "model": model_id, "output": []});
    let canned = if method == "generate" {
        Canned::json(&response)
    } else {
        Canned {
            status: 200,
            headers: vec![("content-type".into(), "text/event-stream".into())],
            body: format!(
                "data: {}\n\ndata: [DONE]\n\n",
                json!({"type": "response.completed", "response": response})
            )
            .into_bytes(),
        }
    };
    let fetch: Arc<MockFetch> = MockFetch::new(vec![canned]);
    let model: Box<dyn LanguageModel> = if azure {
        Box::new(
            create_azure(AzureOpenAIProviderSettings {
                resource_name: Some("test-resource".into()),
                api_key: Some(aimux_provider_utils::Resolvable::Value(
                    "test-api-key".into(),
                )),
                fetch: Some(fetch.transport()),
                ..Default::default()
            })
            .unwrap()
            .responses(model_id),
        )
    } else {
        Box::new(
            create_openai(OpenAIProviderSettings {
                base_url: Some(URL.into()),
                api_key: Some("test-api-key".into()),
                fetch: Some(fetch.transport()),
                ..Default::default()
            })
            .unwrap()
            .responses(model_id),
        )
    };
    let args: CallOptions = serde_json::from_value(json!({
        "prompt": prompt,
        "providerOptions": {"openai": options},
    }))
    .unwrap();

    let result = if method == "generate" {
        model
            .do_generate(&args)
            .await
            .map(|result| json!(result.warnings))
    } else {
        match model.do_stream(&args).await {
            Ok(result) => {
                let parts: Vec<Value> = result
                    .stream
                    .map(|part| match part {
                        Ok(part) => serde_json::to_value(part).unwrap(),
                        Err(error) => json!({"type": "error", "error": error.to_string()}),
                    })
                    .collect()
                    .await;
                let errors: Vec<&Value> = parts
                    .iter()
                    .filter(|part| part["type"] == "error")
                    .collect();
                assert_eq!(errors, Vec::<&Value>::new());
                Ok(parts
                    .iter()
                    .find(|part| part["type"] == "stream-start")
                    .unwrap()["warnings"]
                    .clone())
            }
            Err(error) => Err(error),
        }
    };
    // The caller's prompt is left as it was.
    assert_eq!(&json!(args.prompt), prompt);
    let seen = fetch.seen();
    let result = result.map(|warnings| {
        let body = seen[0].json_body();
        assert_eq!(
            body.get("stream"),
            (method == "stream").then_some(&json!(true)),
            "{method}"
        );
        (body, warnings)
    });
    Outcome {
        result,
        calls: seen.len(),
    }
}

mod positioned_reasoning_effort_updates {
    use super::*;

    /// upstream: @ai-sdk/openai src/responses/openai-responses-reasoning-effort-update.test.ts › positioned reasoning effort updates (%s) › retains multiple updates among messages for %s
    #[tokio::test]
    async fn retains_multiple_updates_among_messages_for() {
        for method in METHODS {
            for model_id in ["gpt-6-astra", "gpt-6-sol", "gpt-6-luna"] {
                let (body, warnings) = request_with(
                    method,
                    &json!([
                        user(),
                        {"role": "assistant", "content": [{"type": "text", "text": "Answer"}]},
                        update("high"),
                        user(),
                        update("low"),
                        user(),
                    ]),
                    &json!({"reasoningEffort": "medium", "store": false}),
                    model_id,
                    false,
                )
                .await
                .ok();
                assert_eq!(
                    body["input"],
                    json!([
                        wire_user(),
                        {"role": "assistant", "content": "Answer"},
                        wire_update("high"),
                        wire_user(),
                        wire_update("low"),
                        wire_user(),
                    ]),
                    "{method} {model_id}"
                );
                assert_eq!(body["reasoning"]["effort"], "medium", "{method} {model_id}");
                assert_eq!(warnings, json!([]), "{method} {model_id}");
            }
        }
    }

    mod non_reasoning_updates_for {
        use super::*;

        const MODELS: [&str; 2] = ["gpt-6-sol", "gpt-6-luna"];

        /// upstream: @ai-sdk/openai src/responses/openai-responses-reasoning-effort-update.test.ts › positioned reasoning effort updates (%s) › non-reasoning updates for %s › preserves positioned none updates and the initial reasoning effort
        #[tokio::test]
        async fn preserves_positioned_none_updates_and_the_initial_reasoning_effort() {
            for method in METHODS {
                for model_id in MODELS {
                    let (body, warnings) = request_with(
                        method,
                        &json!([user(), update("none"), user(), update("low"), user()]),
                        &json!({"reasoningEffort": "low", "reasoningSummary": null}),
                        model_id,
                        false,
                    )
                    .await
                    .ok();
                    assert_eq!(
                        body["input"],
                        json!([
                            wire_user(),
                            wire_update("none"),
                            wire_user(),
                            wire_update("low"),
                            wire_user(),
                        ]),
                        "{method} {model_id}"
                    );
                    assert_eq!(
                        body["reasoning"],
                        json!({"effort": "low"}),
                        "{method} {model_id}"
                    );
                    assert_eq!(warnings, json!([]), "{method} {model_id}");
                }
            }
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-reasoning-effort-update.test.ts › positioned reasoning effort updates (%s) › non-reasoning updates for %s › prepends a request-level none update
        #[tokio::test]
        async fn prepends_a_request_level_none_update() {
            for method in METHODS {
                for model_id in MODELS {
                    let (body, warnings) = request_with(
                        method,
                        &json!([user()]),
                        &json!({"reasoningEffort": "low", "reasoningEffortUpdate": "none"}),
                        model_id,
                        false,
                    )
                    .await
                    .ok();
                    assert_eq!(
                        body["input"],
                        json!([wire_update("none"), wire_user()]),
                        "{method} {model_id}"
                    );
                    assert_eq!(body["reasoning"]["effort"], "low", "{method} {model_id}");
                    assert_eq!(warnings, json!([]), "{method} {model_id}");
                }
            }
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-reasoning-effort-update.test.ts › positioned reasoning effort updates (%s) › non-reasoning updates for %s › deduplicates matching request-level and first positioned none updates
        #[tokio::test]
        async fn deduplicates_matching_request_level_and_first_positioned_none_updates() {
            for method in METHODS {
                for model_id in MODELS {
                    let (body, warnings) = request_with(
                        method,
                        &json!([update("none"), user()]),
                        &json!({"reasoningEffortUpdate": "none"}),
                        model_id,
                        false,
                    )
                    .await
                    .ok();
                    assert_eq!(
                        body["input"],
                        json!([wire_update("none"), wire_user()]),
                        "{method} {model_id}"
                    );
                    assert_eq!(warnings, json!([]), "{method} {model_id}");
                }
            }
        }
    }

    /// upstream: @ai-sdk/openai src/responses/openai-responses-reasoning-effort-update.test.ts › positioned reasoning effort updates (%s) › warns and omits request-level none updates for Astra while retaining valid history
    #[tokio::test]
    async fn warns_and_omits_request_level_none_updates_for_astra_while_retaining_valid_history() {
        for method in METHODS {
            let (body, warnings) = request(
                method,
                &json!([user(), update("low"), user()]),
                &json!({"reasoningEffort": "medium", "reasoningEffortUpdate": "none"}),
            )
            .await
            .ok();
            assert_eq!(
                body["input"],
                json!([wire_user(), wire_update("low"), wire_user()]),
                "{method}"
            );
            assert_eq!(body["reasoning"]["effort"], "medium", "{method}");
            assert_eq!(
                warnings,
                json!([{
                    "type": "unsupported",
                    "feature": "reasoningEffortUpdate",
                    "details": "gpt-6-astra only supports the following reasoning efforts: low, medium, high, xhigh, max",
                }]),
                "{method}"
            );
        }
    }

    /// upstream: @ai-sdk/openai src/responses/openai-responses-reasoning-effort-update.test.ts › positioned reasoning effort updates (%s) › rejects unsupported Azure configurations when using OpenAI message options
    #[tokio::test]
    async fn rejects_unsupported_azure_configurations_when_using_open_ai_message_options() {
        for method in METHODS {
            request_with(
                method,
                &json!([user(), update("high"), user()]),
                &json!({"truncation": "auto"}),
                "gpt-6-astra",
                true,
            )
            .await
            .assert_unsupported("Message-level reasoningEffortUpdate");
        }
    }

    /// upstream: @ai-sdk/openai src/responses/openai-responses-reasoning-effort-update.test.ts › positioned reasoning effort updates (%s) › keeps the request-level update prepended and historical updates positioned
    #[tokio::test]
    async fn keeps_the_request_level_update_prepended_and_historical_updates_positioned() {
        for method in METHODS {
            let (body, warnings) = request(
                method,
                &json!([user(), update("high"), user()]),
                &json!({"reasoningEffort": "low", "reasoningEffortUpdate": "medium"}),
            )
            .await
            .ok();
            assert_eq!(
                body["input"],
                json!([
                    wire_update("medium"),
                    wire_user(),
                    wire_update("high"),
                    wire_user()
                ]),
                "{method}"
            );
            assert_eq!(body["reasoning"]["effort"], "low", "{method}");
            assert_eq!(warnings, json!([]), "{method}");
        }
    }

    /// upstream: @ai-sdk/openai src/responses/openai-responses-reasoning-effort-update.test.ts › positioned reasoning effort updates (%s) › uses an identical first historical update without prepending a duplicate
    #[tokio::test]
    async fn uses_an_identical_first_historical_update_without_prepending_a_duplicate() {
        for method in METHODS {
            let (body, warnings) = request(
                method,
                &json!([update("high"), user()]),
                &json!({
                    "reasoningEffort": "low",
                    "reasoningSummary": "concise",
                    "reasoningEffortUpdate": "high",
                }),
            )
            .await
            .ok();
            assert_eq!(
                body["input"],
                json!([wire_update("high"), wire_user()]),
                "{method}"
            );
            assert_eq!(
                body["reasoning"],
                json!({"effort": "low", "summary": "concise"}),
                "{method}"
            );
            assert_eq!(warnings, json!([]), "{method}");
        }
    }

    /// upstream: @ai-sdk/openai src/responses/openai-responses-reasoning-effort-update.test.ts › positioned reasoning effort updates (%s) › checks the first item after system text has been removed
    #[tokio::test]
    async fn checks_the_first_item_after_system_text_has_been_removed() {
        for method in METHODS {
            let (body, warnings) = request(
                method,
                &json!([{"role": "system", "content": "Removed"}, update("high"), user()]),
                &json!({"reasoningEffortUpdate": "high", "systemMessageMode": "remove"}),
            )
            .await
            .ok();
            assert_eq!(
                body["input"],
                json!([wire_update("high"), wire_user()]),
                "{method}"
            );
            assert_eq!(
                warnings,
                json!([{"type": "other", "message": "system messages are removed for this model"}]),
                "{method}"
            );
        }
    }

    /// upstream: @ai-sdk/openai src/responses/openai-responses-reasoning-effort-update.test.ts › positioned reasoning effort updates (%s) › prepends when an identical historical update is not the first item
    #[tokio::test]
    async fn prepends_when_an_identical_historical_update_is_not_the_first_item() {
        for method in METHODS {
            let (body, warnings) = request(
                method,
                &json!([user(), update("high"), user()]),
                &json!({"reasoningEffortUpdate": "high"}),
            )
            .await
            .ok();
            assert_eq!(
                body["input"],
                json!([
                    wire_update("high"),
                    wire_user(),
                    wire_update("high"),
                    wire_user()
                ]),
                "{method}"
            );
            assert_eq!(warnings, json!([]), "{method}");
        }
    }

    /// upstream: @ai-sdk/openai src/responses/openai-responses-reasoning-effort-update.test.ts › positioned reasoning effort updates (%s) › preserves request-level continuation behavior
    #[tokio::test]
    async fn preserves_request_level_continuation_behavior() {
        for method in METHODS {
            let (body, warnings) = request(
                method,
                &json!([user()]),
                &json!({
                    "previousResponseId": "resp_previous",
                    "reasoningEffort": "low",
                    "reasoningEffortUpdate": "high",
                }),
            )
            .await
            .ok();
            assert_eq!(
                body["input"],
                json!([wire_update("high"), wire_user()]),
                "{method}"
            );
            assert_eq!(body["previous_response_id"], "resp_previous", "{method}");
            assert_eq!(body["reasoning"]["effort"], "low", "{method}");
            assert_eq!(warnings, json!([]), "{method}");
        }
    }

    mod message_level_continuation_with_field {
        use super::*;

        fn rows() -> [(Value, &'static str, &'static str); 2] {
            [
                (
                    json!({"previousResponseId": "resp_previous"}),
                    "previous_response_id",
                    "resp_previous",
                ),
                (
                    json!({"conversation": "conv_test"}),
                    "conversation",
                    "conv_test",
                ),
            ]
        }

        fn previous_reasoning() -> Value {
            json!({
                "role": "assistant",
                "content": [{
                    "type": "reasoning",
                    "text": "Earlier reasoning",
                    "providerOptions": {"openai": {"itemId": "rs_previous"}},
                }],
            })
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-reasoning-effort-update.test.ts › positioned reasoning effort updates (%s) › message-level continuation with $field › preserves an update after filtering reasoning already stored in history
        #[tokio::test]
        async fn preserves_an_update_after_filtering_reasoning_already_stored_in_history() {
            for method in METHODS {
                for (options, field, value) in rows() {
                    let mut options = options;
                    options["reasoningEffort"] = json!("low");
                    let (body, warnings) = request(
                        method,
                        &json!([previous_reasoning(), update("high"), user()]),
                        &options,
                    )
                    .await
                    .ok();
                    assert_eq!(body[field], value, "{method} {field}");
                    assert_eq!(
                        body["input"],
                        json!([wire_update("high"), wire_user()]),
                        "{method} {field}"
                    );
                    assert_eq!(body["reasoning"]["effort"], "low", "{method} {field}");
                    assert_eq!(warnings, json!([]), "{method} {field}");
                }
            }
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-reasoning-effort-update.test.ts › positioned reasoning effort updates (%s) › message-level continuation with $field › rejects updates made adjacent by filtering stored reasoning
        #[tokio::test]
        async fn rejects_updates_made_adjacent_by_filtering_stored_reasoning() {
            for method in METHODS {
                for (options, _field, _value) in rows() {
                    request(
                        method,
                        &json!([update("high"), previous_reasoning(), update("low"), user()]),
                        &options,
                    )
                    .await
                    .assert_unsupported("Adjacent reasoning effort configuration updates");
                }
            }
        }
    }

    mod unsupported_configuration_model_options {
        use super::*;

        fn rows() -> [(&'static str, Value); 6] {
            [
                ("gpt-5.6", json!({})),
                ("custom-model", json!({"forceReasoning": true})),
                ("gpt-6-astra", json!({"reasoningMode": "pro"})),
                ("gpt-6-astra", json!({"truncation": "auto"})),
                (
                    "gpt-6-astra",
                    json!({"contextManagement": [{"type": "compaction", "compactThreshold": 1000}]}),
                ),
                ("gpt-6-astra", json!({"contextManagement": []})),
            ]
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-reasoning-effort-update.test.ts › positioned reasoning effort updates (%s) › unsupported configuration: $model $options › rejects historical updates before sending and preserves caller input
        #[tokio::test]
        async fn rejects_historical_updates_before_sending_and_preserves_caller_input() {
            for method in METHODS {
                for (model, options) in rows() {
                    let original = options.clone();
                    request_with(
                        method,
                        &json!([update("high"), user()]),
                        &options,
                        model,
                        false,
                    )
                    .await
                    .assert_unsupported("Message-level reasoningEffortUpdate");
                    assert_eq!(options, original, "{method} {model}");
                }
            }
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-reasoning-effort-update.test.ts › positioned reasoning effort updates (%s) › unsupported configuration: $model $options › preserves legacy request-level warn-and-omit behavior
        #[tokio::test]
        async fn preserves_legacy_request_level_warn_and_omit_behavior() {
            for method in METHODS {
                for (model, options) in rows() {
                    let mut options = options;
                    options["reasoningEffortUpdate"] = json!("low");
                    let (body, warnings) =
                        request_with(method, &json!([user()]), &options, model, false)
                            .await
                            .ok();
                    assert_eq!(
                        body["input"],
                        json!([wire_user()]),
                        "{method} {model} {options}"
                    );
                    let matching = warnings
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter(|w| {
                            w["type"] == "unsupported" && w["feature"] == "reasoningEffortUpdate"
                        })
                        .count();
                    assert_eq!(matching, 1, "{method} {model} {options}");
                }
            }
        }
    }

    /// upstream: @ai-sdk/openai src/responses/openai-responses-reasoning-effort-update.test.ts › positioned reasoning effort updates (%s) › emits controls independently of systemMessageMode=%s
    #[tokio::test]
    async fn emits_controls_independently_of_system_message_mode() {
        for method in METHODS {
            for mode in ["system", "developer", "remove"] {
                let (body, warnings) = request(
                    method,
                    &json!([update("high"), user()]),
                    &json!({"systemMessageMode": mode}),
                )
                .await
                .ok();
                assert_eq!(
                    body["input"],
                    json!([wire_update("high"), wire_user()]),
                    "{method} {mode}"
                );
                assert_eq!(warnings, json!([]), "{method} {mode}");
            }
        }
    }

    /// upstream: @ai-sdk/openai src/responses/openai-responses-reasoning-effort-update.test.ts › positioned reasoning effort updates (%s) › preserves ordinary empty system messages
    #[tokio::test]
    async fn preserves_ordinary_empty_system_messages() {
        for method in METHODS {
            let (body, _warnings) = request(
                method,
                &json!([{"role": "system", "content": ""}, user()]),
                &json!({}),
            )
            .await
            .ok();
            assert_eq!(
                body["input"],
                json!([{"role": "developer", "content": ""}, wire_user()]),
                "{method}"
            );
        }
    }

    /// upstream: @ai-sdk/openai src/responses/openai-responses-reasoning-effort-update.test.ts › positioned reasoning effort updates (%s) › allows explicit compaction with standard mode and disabled truncation
    #[tokio::test]
    async fn allows_explicit_compaction_with_standard_mode_and_disabled_truncation() {
        for method in METHODS {
            let (body, warnings) = request(
                method,
                &json!([update("high"), user()]),
                &json!({
                    "reasoningMode": "standard",
                    "truncation": "disabled",
                    "compactionTrigger": true,
                }),
            )
            .await
            .ok();
            assert_eq!(
                body["input"],
                json!([wire_update("high"), wire_user(), {"type": "compaction_trigger"}]),
                "{method}"
            );
            assert_eq!(warnings, json!([]), "{method}");
        }
    }

    /// upstream: @ai-sdk/openai src/responses/openai-responses-reasoning-effort-update.test.ts › positioned reasoning effort updates (%s) › rejects adjacent serialized updates before sending: $options
    #[tokio::test]
    async fn rejects_adjacent_serialized_updates_before_sending() {
        let removed = json!({"role": "system", "content": "Removed"});
        for method in METHODS {
            for (prompt, options) in [
                (json!([update("high"), update("low"), user()]), json!({})),
                (json!([update("high"), update("high"), user()]), json!({})),
                (
                    json!([update("high"), update("high"), user()]),
                    json!({"reasoningEffortUpdate": "high"}),
                ),
                (
                    json!([update("high"), user()]),
                    json!({"reasoningEffortUpdate": "low"}),
                ),
                (
                    json!([update("high"), removed, update("low"), user()]),
                    json!({"systemMessageMode": "remove"}),
                ),
                (
                    json!([update("high"), removed, update("high"), user()]),
                    json!({"systemMessageMode": "remove"}),
                ),
            ] {
                request(method, &prompt, &options)
                    .await
                    .assert_unsupported("Adjacent reasoning effort configuration updates");
            }
        }
    }

    /// upstream: @ai-sdk/openai src/responses/openai-responses-reasoning-effort-update.test.ts › positioned reasoning effort updates (%s) › rejects minimal message-level updates even on Luna
    #[tokio::test]
    async fn rejects_minimal_message_level_updates_even_on_luna() {
        for method in METHODS {
            let outcome = request_with(
                method,
                &json!([update("minimal"), user()]),
                &json!({}),
                "gpt-6-luna",
                false,
            )
            .await;
            assert!(
                matches!(outcome.result, Err(AiMuxError::InvalidArgument { .. })),
                "{method}: {:?}",
                outcome.result
            );
            assert_eq!(outcome.calls, 0, "{method}");
        }
    }

    /// upstream: @ai-sdk/openai src/responses/openai-responses-reasoning-effort-update.test.ts › positioned reasoning effort updates (%s) › rejects minimal request-level updates even on Luna
    #[tokio::test]
    async fn rejects_minimal_request_level_updates_even_on_luna() {
        for method in METHODS {
            let outcome = request_with(
                method,
                &json!([user()]),
                &json!({"reasoningEffortUpdate": "minimal"}),
                "gpt-6-luna",
                false,
            )
            .await;
            assert!(
                matches!(outcome.result, Err(AiMuxError::InvalidArgument { .. })),
                "{method}: {:?}",
                outcome.result
            );
            assert_eq!(outcome.calls, 0, "{method}");
        }
    }
}
