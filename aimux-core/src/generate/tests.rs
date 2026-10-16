//! Call-layer behaviour of `generate_text` / `stream_text`, checked against
//! the AI SDK sources (ai@7.0.127) cited on each test.

use std::sync::Mutex;

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::json;

use super::*;
use crate::result::GenerateResult;
use crate::shared::{ResponseInfo, StreamResponseInfo};
use crate::tool::RawToolCall;
use crate::types::{FinishReason, InputTokenUsage, OutputTokenUsage, Usage};

/// A model that replays scripted results.
pub(super) struct ScriptedModel {
    pub generate: Mutex<Option<GenerateResult>>,
    pub streams: Mutex<Vec<Vec<Result<StreamPart, AiMuxError>>>>,
    /// Keep each scripted stream open after its last part.
    pub hang_after_script: bool,
}

impl ScriptedModel {
    pub fn generating(result: GenerateResult) -> Self {
        Self {
            generate: Mutex::new(Some(result)),
            streams: Mutex::new(Vec::new()),
            hang_after_script: false,
        }
    }

    pub fn streaming(parts: Vec<Result<StreamPart, AiMuxError>>) -> Self {
        Self {
            generate: Mutex::new(None),
            streams: Mutex::new(vec![parts]),
            hang_after_script: false,
        }
    }
}

#[async_trait]
impl LanguageModel for ScriptedModel {
    fn provider(&self) -> &str {
        "mock.chat"
    }

    fn model_id(&self) -> &str {
        "mock-model"
    }

    async fn do_generate(&self, _options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        Ok(self
            .generate
            .lock()
            .unwrap()
            .clone()
            .expect("a scripted generate result"))
    }

    async fn do_stream(&self, _options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let parts = self.streams.lock().unwrap().remove(0);
        let stream = futures::stream::iter(parts);
        let stream: Pin<Box<dyn Stream<Item = Result<StreamPart, AiMuxError>> + Send>> =
            if self.hang_after_script {
                Box::pin(stream.chain(futures::stream::pending()))
            } else {
                Box::pin(stream)
            };
        Ok(StreamResult {
            stream,
            request: Some(RequestInfo {
                body: Some(json!({ "model": "mock-model" })),
            }),
            response: Some(StreamResponseInfo {
                headers: Some(HashMap::from([(
                    "x-request-id".to_string(),
                    "req-1".to_string(),
                )])),
            }),
        })
    }
}

pub(super) fn usage(input: u32, cache_read: u32, output: u32, reasoning: u32) -> Usage {
    Usage {
        input_tokens: InputTokenUsage {
            total: Some(input),
            no_cache: Some(input - cache_read),
            cache_read: Some(cache_read),
            cache_write: None,
        },
        output_tokens: OutputTokenUsage {
            total: Some(output),
            text: Some(output - reasoning),
            reasoning: Some(reasoning),
        },
        raw: Some(serde_json::Map::from_iter([(
            "prompt_tokens".to_string(),
            json!(input),
        )])),
    }
}

fn finish(unified: FinishReasonUnified, raw: &str) -> FinishReason {
    FinishReason {
        unified,
        raw: Some(raw.to_string()),
    }
}

pub(super) fn text_stream(
    texts: &[&str],
    reason: FinishReasonUnified,
) -> Vec<Result<StreamPart, AiMuxError>> {
    let mut parts = vec![
        Ok(StreamPart::StreamStart {
            warnings: vec![Warning::Other {
                message: "careful".into(),
            }],
        }),
        Ok(StreamPart::ResponseMetadata(
            crate::types::ResponseMetadata {
                id: Some("resp-1".into()),
                timestamp: Some("2026-01-01T00:00:00.000Z".into()),
                model_id: Some("mock-model-2026".into()),
            },
        )),
        Ok(StreamPart::TextStart {
            id: "t1".into(),
            provider_metadata: None,
        }),
    ];
    for text in texts {
        parts.push(Ok(StreamPart::TextDelta {
            id: "t1".into(),
            delta: (*text).into(),
            provider_metadata: None,
        }));
    }
    parts.push(Ok(StreamPart::TextEnd {
        id: "t1".into(),
        provider_metadata: None,
    }));
    parts.push(Ok(StreamPart::Finish {
        finish_reason: finish(reason, "end_turn"),
        usage: usage(10, 4, 6, 2),
        provider_metadata: None,
    }));
    parts
}

pub(super) async fn collect(result: StreamTextResult) -> Vec<TextStreamPart> {
    result
        .stream
        .map(|part| part.expect("no transport error"))
        .collect()
        .await
}

pub(super) fn kind(part: &TextStreamPart) -> String {
    let value = serde_json::to_value(part).unwrap();
    value
        .as_object()
        .and_then(|object| object.keys().next().cloned())
        .unwrap_or_else(|| value.as_str().unwrap_or_default().to_string())
}

fn generated(content: Vec<GenerateContent>, reason: FinishReasonUnified) -> GenerateResult {
    GenerateResult {
        content,
        finish_reason: finish(reason, "stop"),
        usage: usage(10, 4, 6, 2),
        warnings: vec![Warning::Other {
            message: "careful".into(),
        }],
        provider_metadata: Some(indexmap::IndexMap::from([(
            "mock".to_string(),
            serde_json::Map::from_iter([("k".to_string(), json!(1))]),
        )])),
        request: Some(RequestInfo {
            body: Some(json!({ "model": "mock-model" })),
        }),
        response: Some(ResponseInfo {
            id: Some("resp-1".into()),
            timestamp: None,
            model_id: None,
            headers: None,
            body: Some(json!({ "id": "resp-1" })),
        }),
    }
}

fn text_content(text: &str) -> GenerateContent {
    GenerateContent::Text {
        text: text.into(),
        provider_metadata: None,
    }
}

/// stream-text-result.ts:581-607 and stream-text.ts:1855, :2830-2840,
/// :3032-3048, :3128-3134: `start`, `start-step` on the step's first chunk
/// (the provider's `stream-start` only carries warnings), the content,
/// `finish-step` with the step's response, usage, performance and finish
/// reason, then `finish` with the total usage. Text deltas carry `text`
/// (stream-text-result.ts:436-441).
#[tokio::test]
async fn the_stream_is_framed_as_the_ai_sdk_full_stream() {
    let model = ScriptedModel::streaming(text_stream(&["Hel", "lo"], FinishReasonUnified::Stop));
    let parts = collect(
        stream_text(&model, "hi", GenerateTextOptions::default())
            .await
            .unwrap(),
    )
    .await;
    let kinds: Vec<String> = parts.iter().map(kind).collect();
    assert_eq!(
        kinds,
        [
            "Start",
            "StartStep",
            "TextStart",
            "TextDelta",
            "TextDelta",
            "TextEnd",
            "FinishStep",
            "Finish"
        ]
    );

    let TextStreamPart::StartStep { request, warnings } = &parts[1] else {
        unreachable!()
    };
    assert_eq!(request.body, Some(json!({ "model": "mock-model" })));
    assert_eq!(
        serde_json::to_value(warnings).unwrap(),
        json!([{ "Other": { "message": "careful" } }])
    );

    assert_eq!(
        serde_json::to_value(&parts[3]).unwrap(),
        json!({ "TextDelta": { "id": "t1", "text": "Hel" } })
    );

    let TextStreamPart::FinishStep {
        response,
        usage,
        performance,
        finish_reason,
        raw_finish_reason,
        provider_metadata,
    } = &parts[6]
    else {
        unreachable!()
    };
    assert_eq!(response.id, "resp-1");
    assert_eq!(response.model_id, "mock-model-2026");
    assert_eq!(response.timestamp, "2026-01-01T00:00:00.000Z");
    assert_eq!(
        response.headers.as_ref().unwrap()["x-request-id"],
        "req-1".to_string()
    );
    assert_eq!(*finish_reason, FinishReasonUnified::Stop);
    assert_eq!(raw_finish_reason.as_deref(), Some("end_turn"));
    assert!(provider_metadata.is_none());
    // usage.ts:81-102: the step usage keeps the provider's raw usage.
    assert_eq!(usage.input_tokens, Some(10));
    assert_eq!(usage.input_token_details.cache_read_tokens, Some(4));
    assert_eq!(usage.input_token_details.no_cache_tokens, Some(6));
    assert_eq!(usage.output_token_details.reasoning_tokens, Some(2));
    assert_eq!(usage.total_tokens, Some(16));
    assert!(usage.raw.is_some());
    // stream-language-model-call.ts:597-624: a finished streamed call has
    // the streaming rates and the first-output timing.
    assert!(performance.time_to_first_output_ms.is_some());
    assert!(performance.output_tokens_per_second.is_some());
    assert!(performance.input_tokens_per_second.is_some());
    assert_eq!(
        performance
            .time_between_output_chunks_ms
            .map(|stats| stats.min >= 0.0),
        Some(true),
        "two output chunks give one gap"
    );
    assert!(performance.step_time_ms >= performance.response_time_ms);

    let TextStreamPart::Finish {
        finish_reason,
        raw_finish_reason,
        total_usage,
    } = &parts[7]
    else {
        unreachable!()
    };
    assert_eq!(*finish_reason, FinishReasonUnified::Stop);
    assert_eq!(raw_finish_reason.as_deref(), Some("end_turn"));
    // usage.ts:122-155: the sum carries no raw usage.
    assert_eq!(total_usage.total_tokens, Some(16));
    assert!(total_usage.raw.is_none());
    assert_eq!(
        serde_json::to_value(&parts[7]).unwrap()["Finish"]["finish_reason"],
        json!("stop")
    );
}

/// stream-text.ts:1480-1700: the consumed stream records one step from the
/// parts between `start-step` and `finish-step`; text, usage and the response
/// messages derive from it as in `generateText`.
#[tokio::test]
async fn a_consumed_stream_aggregates_its_step() {
    let model = ScriptedModel::streaming(text_stream(&["Hel", "lo"], FinishReasonUnified::Stop));
    let result = stream_text(&model, "hi", GenerateTextOptions::default())
        .await
        .unwrap()
        .consume()
        .await
        .unwrap();
    assert_eq!(result.steps().len(), 1);
    let step = result.final_step();
    assert_eq!(step.step_number, 0);
    assert!(!step.call_id.is_empty());
    assert_eq!(step.model.provider, "mock.chat");
    assert_eq!(step.model.model_id, "mock-model");
    assert_eq!(step.text(), "Hello");
    assert_eq!(step.response.id, "resp-1");
    assert_eq!(step.response.body, None);
    assert_eq!(step.request.body, Some(json!({ "model": "mock-model" })));
    assert_eq!(step.warnings.len(), 1);
    assert_eq!(result.text(), "Hello");
    assert_eq!(result.finish_reason(), FinishReasonUnified::Stop);
    assert_eq!(result.raw_finish_reason(), Some("end_turn"));
    assert_eq!(result.usage().total_tokens, Some(16));
    assert_eq!(result.output().unwrap(), &json!("Hello"));
    assert_eq!(result.response_messages().len(), 1);
}

/// generate-text.ts:1382-1401 and :1691-1808: a generated step has the
/// response-time rates but no streaming timing; the result's `usage` is the
/// total over the steps (no raw usage), the step's usage keeps it; the
/// response falls back to a generated id, the current time and the model's
/// id (:1058-1064).
#[tokio::test]
async fn generate_text_derives_its_result_from_the_step() {
    let call = RawToolCall {
        tool_call_id: "call-1".into(),
        tool_name: "weather".into(),
        input: r#"{"city":"Paris"}"#.into(),
        provider_executed: None,
        dynamic: None,
        provider_metadata: None,
    };
    let model = ScriptedModel::generating(generated(
        vec![
            text_content("Checking "),
            GenerateContent::ToolCall(call),
            text_content("the weather."),
        ],
        FinishReasonUnified::ToolCalls,
    ));
    let options = GenerateTextOptions {
        tools: Some(vec![Tool::Function(crate::tool::FunctionTool::new(
            "weather",
            json!({
                "type": "object",
                "properties": { "city": { "type": "string" } },
                "required": ["city"],
            }),
        ))]),
        ..GenerateTextOptions::default()
    };
    let result = generate_text(&model, "hi", options).await.unwrap();
    let step = result.final_step();
    assert_eq!(step.response.id, "resp-1");
    assert_eq!(step.response.model_id, "mock-model");
    assert!(!step.response.timestamp.is_empty());
    assert_eq!(step.response.body, Some(json!({ "id": "resp-1" })));
    assert_eq!(step.performance.time_to_first_output_ms, None);
    assert_eq!(step.performance.output_tokens_per_second, None);
    assert_eq!(step.performance.time_between_output_chunks_ms, None);
    assert!(step.usage.raw.is_some());

    assert_eq!(result.text(), "Checking the weather.");
    assert_eq!(result.content().len(), 3);
    assert_eq!(result.tool_calls().len(), 1);
    assert_eq!(result.tool_calls()[0].input, json!({ "city": "Paris" }));
    assert_eq!(result.static_tool_calls().len(), 1);
    assert!(result.dynamic_tool_calls().is_empty());
    assert_eq!(result.usage().input_tokens, Some(10));
    assert!(result.usage().raw.is_none());
    assert_eq!(result.warnings().len(), 1);
    assert_eq!(result.response_messages().len(), 1);
    #[allow(deprecated)]
    {
        assert_eq!(result.total_usage(), result.usage());
        assert_eq!(result.provider_metadata().unwrap()["mock"]["k"], json!(1));
        assert_eq!(result.response().id, "resp-1");
        assert_eq!(result.reasoning_text(), None);
    }
    // generate-text.ts:1599-1612: a tool-calls step is not parsed, and
    // reading the output is a NoOutputGeneratedError.
    assert!(matches!(
        result.output(),
        Err(AiMuxError::NoOutputGenerated(message)) if message == "No output generated."
    ));
}

/// The serialized result writes the derived values next to the stored ones
/// and reads back only `steps`, `total_usage` and `output`. The finish reason
/// is a string and the usage is the flat `LanguageModelUsage`
/// (usage.ts:10-68).
#[tokio::test]
async fn the_result_snapshot_round_trips() {
    let model = ScriptedModel::generating(generated(
        vec![text_content("Hello")],
        FinishReasonUnified::Stop,
    ));
    let result = generate_text(&model, "hi", GenerateTextOptions::default())
        .await
        .unwrap();
    let wire = serde_json::to_value(&result).unwrap();
    assert_eq!(wire["text"], json!("Hello"));
    assert_eq!(wire["finish_reason"], json!("stop"));
    assert_eq!(wire["raw_finish_reason"], json!("stop"));
    assert_eq!(wire["usage"]["input_tokens"], json!(10));
    assert_eq!(
        wire["usage"]["input_token_details"],
        json!({ "no_cache_tokens": 6, "cache_read_tokens": 4, "cache_write_tokens": null })
    );
    assert_eq!(wire["usage"]["total_tokens"], json!(16));
    assert_eq!(wire["output"], json!("Hello"));
    assert_eq!(wire["steps"].as_array().unwrap().len(), 1);
    assert!(wire.get("raw").is_none());

    let back: GenerateTextResult = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(serde_json::to_value(&back).unwrap(), wire);

    let mut no_steps = wire;
    no_steps["steps"] = json!([]);
    assert!(serde_json::from_value::<GenerateTextResult>(no_steps).is_err());
}

/// output.ts and generate-text.ts:1599-1612: the output's response format
/// goes to the model; a stop step is parsed and a parse failure fails
/// `generateText`.
#[tokio::test]
async fn generate_text_parses_the_output() {
    let object = Output::Object {
        schema: json!({
            "type": "object",
            "properties": { "name": { "type": "string" } },
            "required": ["name"],
        }),
        name: None,
        description: None,
    };
    let options = || GenerateTextOptions {
        output: Some(object.clone()),
        ..GenerateTextOptions::default()
    };
    let model = ScriptedModel::generating(generated(
        vec![text_content(r#"{"name":"Ada"}"#)],
        FinishReasonUnified::Stop,
    ));
    let result = generate_text(&model, "hi", options()).await.unwrap();
    assert_eq!(result.output().unwrap(), &json!({ "name": "Ada" }));

    let model = ScriptedModel::generating(generated(
        vec![text_content("not json")],
        FinishReasonUnified::Stop,
    ));
    let error = generate_text(&model, "hi", options()).await.unwrap_err();
    assert!(
        matches!(&error, AiMuxError::JsonParse(message)
            if message.starts_with("No object generated: could not parse the response.")),
        "{error:?}"
    );

    // A length step with text is parsed too (`finishReason !== 'tool-calls'
    // && text.length > 0`).
    let model = ScriptedModel::generating(generated(
        vec![text_content(r#"{"name":"Bo"}"#)],
        FinishReasonUnified::Length,
    ));
    let result = generate_text(&model, "hi", options()).await.unwrap();
    assert_eq!(result.output().unwrap(), &json!({ "name": "Bo" }));
}

/// stream-text.ts:3419-3439: a streamed result always parses its final text,
/// and a failure only fails reading `output`.
#[tokio::test]
async fn a_stream_output_failure_only_fails_the_output() {
    let model = ScriptedModel::streaming(text_stream(&["not json"], FinishReasonUnified::Stop));
    let result = stream_text(
        &model,
        "hi",
        GenerateTextOptions {
            output: Some(Output::Json {
                name: None,
                description: None,
            }),
            ..GenerateTextOptions::default()
        },
    )
    .await
    .unwrap()
    .consume()
    .await
    .unwrap();
    assert_eq!(result.text(), "not json");
    assert!(matches!(result.output(), Err(AiMuxError::JsonParse(_))));
    assert!(
        serde_json::to_value(&result)
            .unwrap()
            .get("output")
            .is_none()
    );
}

/// stream-text.ts:1860-1882: an aborted stream ends with an `abort` part
/// carrying the abort reason's message, and the result fails with the abort
/// when no step finished (:1724-1736).
#[tokio::test]
async fn an_aborted_stream_ends_with_an_abort_part() {
    let mut model = ScriptedModel::streaming(vec![
        Ok(StreamPart::StreamStart {
            warnings: Vec::new(),
        }),
        Ok(StreamPart::TextDelta {
            id: "t1".into(),
            delta: "partial".into(),
            provider_metadata: None,
        }),
    ]);
    model.hang_after_script = true;
    let signal = AbortSignal::new();
    let result = stream_text(
        &model,
        "hi",
        GenerateTextOptions {
            abort_signal: Some(signal.clone()),
            ..GenerateTextOptions::default()
        },
    )
    .await
    .unwrap();
    let mut stream = result.stream;
    let mut kinds = Vec::new();
    while let Some(part) = stream.next().await {
        let part = part.unwrap();
        kinds.push(kind(&part));
        if matches!(part, TextStreamPart::TextDelta { .. }) {
            signal.abort();
        }
        if let TextStreamPart::Abort { reason } = &part {
            assert_eq!(reason.as_deref(), Some("request aborted"));
        }
    }
    assert_eq!(kinds, ["Start", "StartStep", "TextDelta", "Abort"]);

    let mut model = ScriptedModel::streaming(vec![Ok(StreamPart::StreamStart {
        warnings: Vec::new(),
    })]);
    model.hang_after_script = true;
    let signal = AbortSignal::new();
    let result = stream_text(
        &model,
        "hi",
        GenerateTextOptions {
            abort_signal: Some(signal.clone()),
            ..GenerateTextOptions::default()
        },
    )
    .await
    .unwrap();
    signal.abort();
    assert!(matches!(
        result.consume().await,
        Err(AiMuxError::Aborted(message)) if message == "request aborted"
    ));
}

/// stream-text.ts:3015-3030: a model stream that ends without a finish chunk
/// and without output ends with a NoOutputGeneratedError part and no
/// `finish-step`; with partial output the step finishes with `other`.
#[tokio::test]
async fn an_incomplete_stream_without_output_is_an_error() {
    let model = ScriptedModel::streaming(vec![Ok(StreamPart::StreamStart {
        warnings: Vec::new(),
    })]);
    let parts = collect(
        stream_text(&model, "hi", GenerateTextOptions::default())
            .await
            .unwrap(),
    )
    .await;
    let kinds: Vec<String> = parts.iter().map(kind).collect();
    assert_eq!(kinds, ["Start", "Error"]);
    assert!(matches!(
        &parts[1],
        TextStreamPart::Error { error: AiMuxError::NoOutputGenerated(message) }
            if message == "No output generated. The model stream ended without a finish chunk."
    ));

    let model = ScriptedModel::streaming(vec![Ok(StreamPart::TextDelta {
        id: "t1".into(),
        delta: "partial".into(),
        provider_metadata: None,
    })]);
    let parts = collect(
        stream_text(&model, "hi", GenerateTextOptions::default())
            .await
            .unwrap(),
    )
    .await;
    let kinds: Vec<String> = parts.iter().map(kind).collect();
    assert_eq!(
        kinds,
        ["Start", "StartStep", "TextDelta", "FinishStep", "Finish"]
    );
    let TextStreamPart::FinishStep {
        finish_reason,
        usage,
        performance,
        ..
    } = &parts[3]
    else {
        unreachable!()
    };
    assert_eq!(*finish_reason, FinishReasonUnified::Other);
    assert_eq!(usage, &LanguageModelUsage::default());
    // stream-text.ts:2755-2767: without a finish chunk the model call's
    // performance stays zero.
    assert_eq!(performance.response_time_ms, 0.0);
    assert_eq!(performance.time_to_first_output_ms, None);
}

/// generate-object.ts:120 deprecates `generateObject` for
/// `generateText({ output })`; the wrapper returns the AI SDK's
/// `GenerateObjectResult` fields (generate-object-result.ts:17-55).
#[tokio::test]
#[allow(deprecated)]
async fn generate_object_wraps_the_output() {
    let model = ScriptedModel::generating(generated(
        vec![text_content(r#"{"name":"Ada"}"#)],
        FinishReasonUnified::Stop,
    ));
    let result = generate_object(&model, "hi", GenerateTextOptions::default())
        .await
        .unwrap();
    assert_eq!(result.object, json!({ "name": "Ada" }));
    assert_eq!(result.finish_reason, FinishReasonUnified::Stop);
    assert_eq!(result.usage.input_tokens, Some(10));
    assert_eq!(result.response.id, "resp-1");
    assert_eq!(result.response.body, Some(json!({ "id": "resp-1" })));
    assert_eq!(result.reasoning, None);
    let wire = serde_json::to_value(&result).unwrap();
    assert!(wire.get("raw").is_none());
    assert!(wire.get("raw_finish_reason").is_none());

    let model = ScriptedModel::generating(generated(
        vec![text_content("[1,2")],
        FinishReasonUnified::Stop,
    ));
    assert!(matches!(
        generate_object(&model, "hi", GenerateTextOptions::default()).await,
        Err(AiMuxError::JsonParse(_))
    ));
}

/// stream-text.ts:2881-2891: a `text-delta` is enqueued only when its text is
/// non-empty or it carries provider metadata.
#[tokio::test]
async fn an_empty_text_delta_is_forwarded_only_with_provider_metadata() {
    let metadata = indexmap::IndexMap::from([(
        "mock".to_string(),
        serde_json::Map::from_iter([("itemId".to_string(), json!("msg_1"))]),
    )]);
    let mut parts = text_stream(&["", "hi"], FinishReasonUnified::Stop);
    parts.insert(
        4,
        Ok(StreamPart::TextDelta {
            id: "t1".into(),
            delta: String::new(),
            provider_metadata: Some(metadata.clone()),
        }),
    );
    let model = ScriptedModel::streaming(parts);
    let result = stream_text(&model, "hi", GenerateTextOptions::default())
        .await
        .unwrap();
    let deltas: Vec<(String, bool)> = collect(result)
        .await
        .into_iter()
        .filter_map(|part| match part {
            TextStreamPart::TextDelta {
                text,
                provider_metadata,
                ..
            } => Some((text, provider_metadata.is_some())),
            _ => None,
        })
        .collect();
    assert_eq!(
        deltas,
        vec![(String::new(), true), ("hi".to_string(), false)]
    );
}

fn provider_result(id: &str, name: &str, result: Value, is_error: Option<bool>) -> ToolResult {
    ToolResult {
        tool_call_id: id.into(),
        tool_name: name.into(),
        result,
        is_error,
        preliminary: None,
        dynamic: Some(true),
        provider_metadata: None,
    }
}

/// convert-language-model-content.ts:87-104: a provider tool result whose
/// call is not in the response fails with `Tool call {id} not found.` unless
/// it belongs to a provider tool of that name with `supportsDeferredResults`.
#[tokio::test]
async fn generate_text_rejects_a_tool_result_without_its_call() {
    let deferred = GenerateContent::ToolResult(provider_result(
        "srv-0",
        "code_execution",
        json!({ "stdout": "2" }),
        None,
    ));
    let run = |tools: Option<Vec<Tool>>| {
        let model = ScriptedModel::generating(generated(
            vec![text_content("hi"), deferred.clone()],
            FinishReasonUnified::Stop,
        ));
        async move {
            let options = GenerateTextOptions {
                tools,
                ..GenerateTextOptions::default()
            };
            generate_text(&model, "hi", options).await
        }
    };
    let code_execution = |supports_deferred_results| {
        Tool::Provider(crate::tool::ProviderTool {
            id: crate::types::NamespacedKind::new("anthropic.code_execution_20250825").unwrap(),
            name: "code_execution".into(),
            args: serde_json::Map::new(),
            supports_deferred_results,
        })
    };

    for tools in [None, Some(vec![code_execution(None)])] {
        let error = run(tools).await.unwrap_err();
        assert_eq!(error.to_string(), "Tool call srv-0 not found.");
    }
    let result = run(Some(vec![code_execution(Some(true))])).await.unwrap();
    assert!(result.final_step().content.iter().any(|part| matches!(
        part,
        ContentPart::ToolResult(result) if result.tool_call_id == "srv-0"
    )));
}

fn provider_call(id: &str, name: &str, input: &str) -> StreamPart {
    StreamPart::ToolCall(RawToolCall {
        tool_call_id: id.into(),
        tool_name: name.into(),
        input: input.into(),
        provider_executed: Some(true),
        dynamic: Some(true),
        provider_metadata: None,
    })
}

/// Provider tool calls, their results (one an error) and a deferred result,
/// before the step's `finish`.
fn tool_stream() -> Vec<Result<StreamPart, AiMuxError>> {
    let mut parts = text_stream(&["hi"], FinishReasonUnified::ToolCalls);
    let finish = parts.pop().unwrap();
    parts.extend([
        Ok(provider_call("srv-1", "web_search", r#"{"q":"rust"}"#)),
        Ok(StreamPart::ToolResult(provider_result(
            "srv-1",
            "web_search",
            json!([{ "url": "https://example.com" }]),
            None,
        ))),
        Ok(provider_call(
            "srv-2",
            "code_execution",
            r#"{"code":"1/0"}"#,
        )),
        Ok(StreamPart::ToolResult(provider_result(
            "srv-2",
            "code_execution",
            json!({ "error_code": "unavailable" }),
            Some(true),
        ))),
        Ok(StreamPart::ToolResult(provider_result(
            "srv-0",
            "code_execution",
            json!({ "stdout": "2" }),
            None,
        ))),
    ]);
    parts.push(finish);
    parts
}

/// stream-language-model-call.ts:760-792: a provider `tool-result` becomes a
/// `tool-result` (or, with `isError`, a `tool-error`) part with
/// `providerExecuted: true`, the chunk's `dynamic` and the input of its call
/// (`undefined` for a deferred result, whose call is not in the stream).
#[tokio::test]
async fn stream_tool_results_and_errors_follow_the_ai_sdk() {
    let model = ScriptedModel::streaming(tool_stream());
    let parts = collect(
        stream_text(&model, "hi", GenerateTextOptions::default())
            .await
            .unwrap(),
    )
    .await;
    let kinds: Vec<String> = parts.iter().map(kind).collect();
    assert_eq!(
        kinds,
        [
            "Start",
            "StartStep",
            "TextStart",
            "TextDelta",
            "TextEnd",
            "ToolCall",
            "ToolResult",
            "ToolCall",
            "ToolError",
            "ToolResult",
            "FinishStep",
            "Finish"
        ]
    );
    assert_eq!(
        serde_json::to_value(&parts[6]).unwrap(),
        json!({ "ToolResult": {
            "tool_call_id": "srv-1",
            "tool_name": "web_search",
            "input": { "q": "rust" },
            "output": [{ "url": "https://example.com" }],
            "provider_executed": true,
            "dynamic": true,
        } })
    );
    assert_eq!(
        serde_json::to_value(&parts[8]).unwrap(),
        json!({ "ToolError": {
            "tool_call_id": "srv-2",
            "tool_name": "code_execution",
            "input": { "code": "1/0" },
            "error": { "error_code": "unavailable" },
            "provider_executed": true,
            "dynamic": true,
        } })
    );
    assert_eq!(
        serde_json::to_value(&parts[9]).unwrap(),
        json!({ "ToolResult": {
            "tool_call_id": "srv-0",
            "tool_name": "code_execution",
            "output": { "stdout": "2" },
            "provider_executed": true,
            "dynamic": true,
        } })
    );
}

/// The step content of a consumed stream keeps the provider's results and
/// error results (stream-text.ts:1529-1538 records `tool-error` parts,
/// :1648-1650 non-preliminary `tool-result` parts).
#[tokio::test]
async fn a_consumed_stream_keeps_provider_tool_results_and_errors() {
    let model = ScriptedModel::streaming(tool_stream());
    let result = stream_text(&model, "hi", GenerateTextOptions::default())
        .await
        .unwrap()
        .consume()
        .await
        .unwrap();
    let results: Vec<(&str, &str)> = result
        .final_step()
        .content
        .iter()
        .filter_map(|part| match part {
            ContentPart::ToolResult(result) => Some((result.tool_call_id.as_str(), "result")),
            ContentPart::ToolError(error) => Some((error.tool_call_id.as_str(), "error")),
            _ => None,
        })
        .collect();
    assert_eq!(
        results,
        [("srv-1", "result"), ("srv-2", "error"), ("srv-0", "result")]
    );
}

/// stream-language-model-call.ts:709-725: a client tool call that could not
/// be parsed is followed by a `tool-error` with the call's input,
/// `getErrorMessage(error)` and `dynamic: true`; an invalid provider-executed
/// call is not.
#[tokio::test]
async fn an_invalid_tool_call_is_followed_by_a_tool_error() {
    let mut parts = text_stream(&["hi"], FinishReasonUnified::ToolCalls);
    let finish = parts.pop().unwrap();
    parts.extend([
        Ok(StreamPart::ToolCall(RawToolCall {
            tool_call_id: "call-1".into(),
            tool_name: "weather".into(),
            input: r#"{"city":"Paris"}"#.into(),
            provider_executed: None,
            dynamic: None,
            provider_metadata: None,
        })),
        Ok(StreamPart::ToolCall(RawToolCall {
            tool_call_id: "srv-1".into(),
            tool_name: "web_search".into(),
            input: "{".into(),
            provider_executed: Some(true),
            dynamic: Some(true),
            provider_metadata: None,
        })),
    ]);
    parts.push(finish);
    let model = ScriptedModel::streaming(parts);
    let parts = collect(
        stream_text(&model, "hi", GenerateTextOptions::default())
            .await
            .unwrap(),
    )
    .await;
    let kinds: Vec<String> = parts.iter().map(kind).collect();
    assert_eq!(
        kinds,
        [
            "Start",
            "StartStep",
            "TextStart",
            "TextDelta",
            "TextEnd",
            "ToolCall",
            "ToolError",
            "ToolCall",
            "FinishStep",
            "Finish"
        ]
    );
    let TextStreamPart::ToolCall(invalid) = &parts[5] else {
        unreachable!()
    };
    assert_eq!(invalid.invalid, Some(true));
    let TextStreamPart::ToolError(error) = &parts[6] else {
        unreachable!()
    };
    assert_eq!(error.tool_call_id, "call-1");
    assert_eq!(error.tool_name, "weather");
    assert_eq!(error.input, Some(json!({ "city": "Paris" })));
    assert_eq!(
        error.error,
        json!(invalid.error.as_ref().unwrap().to_string())
    );
    assert_eq!(error.provider_executed, None);
    assert_eq!(error.dynamic, Some(true));
    let TextStreamPart::ToolCall(provider_call) = &parts[7] else {
        unreachable!()
    };
    assert_eq!(provider_call.invalid, Some(true));
}

/// stream-text.ts:1529-1538 records a `tool-error` part in the step content
/// where it arrives, so a consumed stream keeps an invalid call's tool error
/// right after the call (the generated path appends it after the content,
/// generate-text.ts:1303-1323). The response messages replay it in a tool
/// message as error text (to-response-messages.ts:158-225).
#[tokio::test]
async fn a_consumed_stream_keeps_an_invalid_calls_tool_error_in_place() {
    let mut parts = text_stream(&["hi"], FinishReasonUnified::ToolCalls);
    let finish = parts.pop().unwrap();
    // The call goes before the text, so "in place" differs from "appended".
    parts.insert(
        1,
        Ok(StreamPart::ToolCall(RawToolCall {
            tool_call_id: "call-1".into(),
            tool_name: "weather".into(),
            input: r#"{"city":"Paris"}"#.into(),
            provider_executed: None,
            dynamic: None,
            provider_metadata: None,
        })),
    );
    parts.push(finish);
    let model = ScriptedModel::streaming(parts);
    let result = stream_text(&model, "hi", GenerateTextOptions::default())
        .await
        .unwrap()
        .consume()
        .await
        .unwrap();
    let step = result.final_step();
    let kinds: Vec<&str> = step
        .content
        .iter()
        .map(|part| match part {
            ContentPart::ToolCall(_) => "ToolCall",
            ContentPart::ToolError(_) => "ToolError",
            ContentPart::Text { .. } => "Text",
            other => panic!("unexpected content {other:?}"),
        })
        .collect();
    assert_eq!(kinds, ["ToolCall", "ToolError", "Text"]);
    let ContentPart::ToolError(error) = &step.content[1] else {
        unreachable!()
    };
    assert_eq!(error.tool_call_id, "call-1");
    assert_eq!(error.dynamic, Some(true));
    let [
        ModelMessage::Assistant { .. },
        ModelMessage::Tool { content, .. },
    ] = &step.response.messages[..]
    else {
        panic!("{:#?}", step.response.messages)
    };
    let [crate::content::ToolContentPart::ToolResult(result)] = &content[..] else {
        panic!("{content:#?}")
    };
    assert_eq!(result.tool_call_id, "call-1");
    assert!(matches!(
        result.output,
        crate::content::ToolResultOutput::ErrorText { .. }
    ));
}

/// The `type` of each AI SDK `TextStreamPart`, in the union's order
/// (stream-text-result.ts:581-607).
const UPSTREAM_STREAM_PART_TYPES: [&str; 26] = [
    "text-start",
    "text-end",
    "text-delta",
    "reasoning-start",
    "reasoning-end",
    "reasoning-delta",
    "custom",
    "tool-input-start",
    "tool-input-end",
    "tool-input-delta",
    "source",
    "file",
    "reasoning-file",
    "tool-call",
    "tool-result",
    "tool-error",
    "tool-output-denied",
    "tool-approval-request",
    "tool-approval-response",
    "start-step",
    "finish-step",
    "start",
    "finish",
    "abort",
    "error",
    "raw",
];

/// `TextStream` → `text-stream`.
fn kebab(name: &str) -> String {
    let mut out = String::new();
    for (index, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if index > 0 {
                out.push('-');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// NP2-5: `TextStreamPart` has exactly the AI SDK's 26 part types, in the
/// union's order (stream-text-result.ts:581-607). The variant tags are read
/// from the exported TypeScript type.
#[test]
fn the_stream_part_set_is_the_ai_sdks() {
    let ts = <TextStreamPart as ts_rs::TS>::inline(&ts_rs::Config::default());
    let tags: Vec<String> = ts
        .match_indices("{ \"")
        .filter_map(|(index, _)| {
            let rest = &ts[index + 3..];
            let tag = &rest[..rest.find('"')?];
            tag.starts_with(|c: char| c.is_ascii_uppercase())
                .then(|| kebab(tag))
        })
        .collect();
    assert_eq!(tags, UPSTREAM_STREAM_PART_TYPES);
}

/// NP2-5 stream state: text and reasoning segments keep the position of
/// their start part while deltas update them by id, interleaved or not
/// (stream-text.ts:1540-1600); the response message replays them in that
/// order.
#[tokio::test]
async fn interleaved_segments_keep_their_start_positions() {
    let mut parts = text_stream(&[], FinishReasonUnified::Stop);
    let finish = parts.pop().unwrap();
    let _text_end = parts.pop().unwrap();
    let text = |id: &str, delta: &str| {
        Ok(StreamPart::TextDelta {
            id: id.into(),
            delta: delta.into(),
            provider_metadata: None,
        })
    };
    parts.extend([
        Ok(StreamPart::TextStart {
            id: "t2".into(),
            provider_metadata: None,
        }),
        text("t2", "B"),
        text("t1", "A"),
        Ok(StreamPart::ReasoningStart {
            id: "r1".into(),
            provider_metadata: None,
        }),
        Ok(StreamPart::ReasoningDelta {
            id: "r1".into(),
            delta: "think".into(),
            provider_metadata: None,
        }),
        text("t1", "a"),
        Ok(StreamPart::TextEnd {
            id: "t2".into(),
            provider_metadata: None,
        }),
        Ok(StreamPart::TextEnd {
            id: "t1".into(),
            provider_metadata: None,
        }),
        Ok(StreamPart::ReasoningEnd {
            id: "r1".into(),
            provider_metadata: None,
        }),
        finish,
    ]);
    let model = ScriptedModel::streaming(parts);
    let result = stream_text(&model, "hi", GenerateTextOptions::default())
        .await
        .unwrap()
        .consume()
        .await
        .unwrap();
    let content: Vec<(&str, &str)> = result
        .final_step()
        .content
        .iter()
        .map(|part| match part {
            ContentPart::Text { text, .. } => ("text", text.as_str()),
            ContentPart::Reasoning(reasoning) => ("reasoning", reasoning.text.as_str()),
            other => panic!("unexpected content {other:?}"),
        })
        .collect();
    assert_eq!(
        content,
        [("text", "Aa"), ("text", "B"), ("reasoning", "think")]
    );
    let messages = serde_json::to_value(result.response_messages()).unwrap();
    let replayed: Vec<String> = messages[0]["content"]
        .as_array()
        .unwrap()
        .iter()
        .map(|part| {
            part.as_object().unwrap()["text"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(replayed, ["Aa", "B", "think"]);
}

/// NP2-5 stream state: a preliminary tool result reaches the stream but not
/// the step content (stream-text.ts:1648-1650; alignment doc rule 6).
#[tokio::test]
async fn a_preliminary_tool_result_stays_out_of_the_step_content() {
    let mut parts = text_stream(&["hi"], FinishReasonUnified::Stop);
    let finish = parts.pop().unwrap();
    let mut preliminary = provider_result("srv-1", "image_generation", json!("partial"), None);
    preliminary.preliminary = Some(true);
    parts.extend([
        Ok(provider_call("srv-1", "image_generation", "{}")),
        Ok(StreamPart::ToolResult(preliminary)),
        Ok(StreamPart::ToolResult(provider_result(
            "srv-1",
            "image_generation",
            json!("final"),
            None,
        ))),
        finish,
    ]);
    let model = ScriptedModel::streaming(parts.clone());
    let streamed = collect(
        stream_text(&model, "hi", GenerateTextOptions::default())
            .await
            .unwrap(),
    )
    .await;
    let outputs: Vec<(Value, Option<bool>)> = streamed
        .iter()
        .filter_map(|part| match part {
            TextStreamPart::ToolResult(result) => Some((result.output.clone(), result.preliminary)),
            _ => None,
        })
        .collect();
    assert_eq!(
        outputs,
        [(json!("partial"), Some(true)), (json!("final"), None)]
    );

    let model = ScriptedModel::streaming(parts);
    let result = stream_text(&model, "hi", GenerateTextOptions::default())
        .await
        .unwrap()
        .consume()
        .await
        .unwrap();
    let kept: Vec<&Value> = result
        .final_step()
        .content
        .iter()
        .filter_map(|part| match part {
            ContentPart::ToolResult(result) => Some(&result.output),
            _ => None,
        })
        .collect();
    assert_eq!(kept, [&json!("final")]);
}

/// NP2-5 message replay: the same model output gives the same response
/// messages whether it was generated or streamed (generate-text.ts:1437-1455
/// and stream-text.ts build both with `toResponseMessages`).
#[tokio::test]
async fn generated_and_streamed_output_replay_the_same_messages() {
    let generated_model = ScriptedModel::generating(generated(
        vec![
            GenerateContent::Reasoning(crate::result::ReasoningOutput {
                text: "think".into(),
                provider_metadata: None,
            }),
            text_content("hi"),
            GenerateContent::ToolCall(RawToolCall {
                tool_call_id: "srv-1".into(),
                tool_name: "web_search".into(),
                input: r#"{"q":"rust"}"#.into(),
                provider_executed: Some(true),
                dynamic: Some(true),
                provider_metadata: None,
            }),
            GenerateContent::ToolResult(provider_result(
                "srv-1",
                "web_search",
                json!([{ "url": "https://example.com" }]),
                None,
            )),
        ],
        FinishReasonUnified::Stop,
    ));
    let generated = generate_text(&generated_model, "hi", GenerateTextOptions::default())
        .await
        .unwrap();

    let mut parts = text_stream(&["hi"], FinishReasonUnified::Stop);
    let finish = parts.pop().unwrap();
    parts.splice(
        2..2,
        [
            Ok(StreamPart::ReasoningStart {
                id: "r1".into(),
                provider_metadata: None,
            }),
            Ok(StreamPart::ReasoningDelta {
                id: "r1".into(),
                delta: "think".into(),
                provider_metadata: None,
            }),
            Ok(StreamPart::ReasoningEnd {
                id: "r1".into(),
                provider_metadata: None,
            }),
        ],
    );
    parts.extend([
        Ok(provider_call("srv-1", "web_search", r#"{"q":"rust"}"#)),
        Ok(StreamPart::ToolResult(provider_result(
            "srv-1",
            "web_search",
            json!([{ "url": "https://example.com" }]),
            None,
        ))),
        finish,
    ]);
    let streamed_model = ScriptedModel::streaming(parts);
    let streamed = stream_text(&streamed_model, "hi", GenerateTextOptions::default())
        .await
        .unwrap()
        .consume()
        .await
        .unwrap();

    assert_eq!(
        serde_json::to_value(streamed.response_messages()).unwrap(),
        serde_json::to_value(generated.response_messages()).unwrap()
    );
}

/// stream-text.ts:1725-1757: an abort after a step finished keeps the step;
/// the result resolves with the recorded step and, without a `finish` part,
/// the null total usage. Only a stream that recorded no step fails with the
/// abort (:1729-1740; `an_aborted_stream_ends_with_an_abort_part`).
#[tokio::test]
async fn an_abort_after_a_finished_step_keeps_the_step() {
    let model = ScriptedModel::streaming(text_stream(&["Hel", "lo"], FinishReasonUnified::Stop));
    let mut result = stream_text(&model, "hi", GenerateTextOptions::default())
        .await
        .unwrap();
    let mut parts: Vec<TextStreamPart> =
        std::mem::replace(&mut result.stream, Box::pin(futures::stream::empty()))
            .map(|part| part.expect("no transport error"))
            .collect()
            .await;
    // stream_text itself never lets an abort in here: Finish directly
    // follows FinishStep.
    assert!(matches!(parts.pop(), Some(TextStreamPart::Finish { .. })));
    assert!(matches!(
        parts.last(),
        Some(TextStreamPart::FinishStep { .. })
    ));
    parts.push(TextStreamPart::Abort {
        reason: Some("request aborted".into()),
    });
    result.stream = Box::pin(futures::stream::iter(parts.into_iter().map(Ok)));

    let result = result.consume().await.expect("the finished step is kept");
    assert_eq!(result.steps().len(), 1);
    assert_eq!(result.text(), "Hello");
    assert_eq!(result.usage(), &LanguageModelUsage::default());
}

/// stream-language-model-call.ts:432, :705-707, :764: both calls with a
/// repeated id are forwarded, and a provider result for that id carries the
/// input of the last one.
#[tokio::test]
async fn a_repeated_tool_call_id_resolves_to_the_last_call_in_the_stream() {
    let mut parts = text_stream(&["hi"], FinishReasonUnified::ToolCalls);
    let finish = parts.pop().unwrap();
    parts.extend([
        Ok(provider_call("srv-1", "web_search", r#"{"q":"first"}"#)),
        Ok(provider_call("srv-1", "web_search", r#"{"q":"second"}"#)),
        Ok(StreamPart::ToolResult(provider_result(
            "srv-1",
            "web_search",
            json!(["hit"]),
            None,
        ))),
    ]);
    parts.push(finish);
    let model = ScriptedModel::streaming(parts);
    let parts = collect(
        stream_text(&model, "hi", GenerateTextOptions::default())
            .await
            .unwrap(),
    )
    .await;

    let inputs: Vec<&Value> = parts
        .iter()
        .filter_map(|part| match part {
            TextStreamPart::ToolCall(call) => Some(&call.input),
            _ => None,
        })
        .collect();
    assert_eq!(
        inputs,
        [&json!({ "q": "first" }), &json!({ "q": "second" })]
    );

    let result = parts
        .iter()
        .find(|part| matches!(part, TextStreamPart::ToolResult(_)))
        .expect("the provider result");
    assert_eq!(
        serde_json::to_value(result).unwrap()["ToolResult"]["input"],
        json!({ "q": "second" })
    );
}

/// convert-language-model-content.ts:74-84: the generated content maps each
/// `tool-call` part to the first parsed call with its id, so both parts of a
/// repeated id show the first call.
#[tokio::test]
async fn a_repeated_tool_call_id_maps_to_the_first_call_when_generated() {
    let call = |input: &str| {
        GenerateContent::ToolCall(RawToolCall {
            tool_call_id: "srv-1".into(),
            tool_name: "web_search".into(),
            input: input.into(),
            provider_executed: Some(true),
            dynamic: Some(true),
            provider_metadata: None,
        })
    };
    let model = ScriptedModel::generating(generated(
        vec![call(r#"{"q":"first"}"#), call(r#"{"q":"second"}"#)],
        FinishReasonUnified::ToolCalls,
    ));
    let result = generate_text(&model, "hi", GenerateTextOptions::default())
        .await
        .unwrap();
    let inputs: Vec<&Value> = result
        .final_step()
        .content
        .iter()
        .filter_map(|part| match part {
            ContentPart::ToolCall(call) => Some(&call.input),
            _ => None,
        })
        .collect();
    assert_eq!(inputs, [&json!({ "q": "first" }), &json!({ "q": "first" })]);
}
