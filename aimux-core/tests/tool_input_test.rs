use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use aimux_core::error::AiMuxError;
use aimux_core::generate::{
    GenerateTextOptions, generate_text, generate_text_as_openai, stream_text, stream_text_as_openai,
};
use aimux_core::language_model::LanguageModel;
use aimux_core::openai_output::OpenAiStreamOptions;
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateContent, GenerateResult, StreamResult};
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::{FunctionTool, RawToolCall, Tool, ToolCallRepair, parse_tool_call};
use aimux_core::types::{FinishReason, FinishReasonUnified, Usage};
use async_trait::async_trait;
use futures::StreamExt;
use serde_json::json;

fn weather_tool() -> Tool {
    FunctionTool::new("weather", weather_tool_schema()).into()
}

fn raw(name: &str, input: &str) -> RawToolCall {
    RawToolCall {
        tool_call_id: "call-1".into(),
        tool_name: name.into(),
        input: input.into(),
        provider_executed: None,
        dynamic: None,
        thought_signature: None,
        provider_metadata: None,
    }
}

#[tokio::test]
async fn parses_and_validates_exact_json() {
    let mut tool_call = raw("weather", r#"{"city":"Singapore","days":3}"#);
    tool_call.dynamic = Some(true);
    let call = parse_tool_call(tool_call, Some(&[weather_tool()]), None, &[], None).await;

    assert_eq!(call.input, json!({ "city": "Singapore", "days": 3 }));
    assert_eq!(call.dynamic, None);
    assert_eq!(call.invalid, None);
    assert!(call.error.is_none());
}

#[tokio::test]
async fn validates_empty_input_as_an_empty_object() {
    let no_arg_tool = FunctionTool::new(
        "ping",
        json!({ "type": "object", "additionalProperties": false }),
    );
    let call = parse_tool_call(
        raw("ping", " \n"),
        Some(&[no_arg_tool.into()]),
        None,
        &[],
        None,
    )
    .await;

    assert_eq!(call.input, json!({}));
    assert_eq!(call.invalid, None);
}

#[tokio::test]
async fn parses_a_json_string_without_confusing_it_with_the_raw_carrier() {
    let echo_tool = FunctionTool::new("echo", json!({ "type": "string" }));
    let call = parse_tool_call(
        raw("echo", r#""hello""#),
        Some(&[echo_tool.into()]),
        None,
        &[],
        None,
    )
    .await;

    assert_eq!(call.input, json!("hello"));
    assert_eq!(call.invalid, None);
}

#[tokio::test]
async fn does_not_apply_partial_json_repair_to_final_tool_calls() {
    for input in [
        r#"{"city":"Singapore"#,
        r#"{"city":"Singapore",}"#,
        r#"{"city":"Singapore"},"days":3}"#,
    ] {
        let call = parse_tool_call(
            raw("weather", input),
            Some(&[weather_tool()]),
            None,
            &[],
            None,
        )
        .await;

        assert_eq!(call.input, json!(input));
        assert_eq!(call.invalid, Some(true));
        assert!(matches!(
            call.error,
            Some(AiMuxError::InvalidToolInput { .. })
        ));
    }
}

#[tokio::test]
async fn preserves_parsed_input_when_schema_validation_fails() {
    let input = r#"{"city":7}"#;
    let call = parse_tool_call(
        raw("weather", input),
        Some(&[weather_tool()]),
        None,
        &[],
        None,
    )
    .await;

    assert_eq!(call.input, json!({ "city": 7 }));
    assert_eq!(call.invalid, Some(true));
    assert!(matches!(
        call.error,
        Some(AiMuxError::InvalidToolInput { .. })
    ));
}

#[tokio::test]
async fn unknown_tool_is_an_invalid_dynamic_call_with_available_tools() {
    let call = parse_tool_call(
        raw("forecast", "{}"),
        Some(&[weather_tool()]),
        None,
        &[],
        None,
    )
    .await;

    assert_eq!(call.dynamic, Some(true));
    assert_eq!(call.invalid, Some(true));
    assert!(matches!(
        call.error,
        Some(AiMuxError::NoSuchTool { available_tools, .. })
            if available_tools == Some(vec!["weather".to_string()])
    ));
}

#[tokio::test]
async fn repair_runs_once_and_the_replacement_is_fully_revalidated() {
    let calls = Arc::new(AtomicUsize::new(0));
    let repair_calls = Arc::clone(&calls);
    let repair = ToolCallRepair::new(move |context| {
        repair_calls.fetch_add(1, Ordering::SeqCst);
        async move {
            assert!(matches!(context.error, AiMuxError::InvalidToolInput { .. }));
            assert_eq!(context.instructions.as_deref(), Some("Use metric units"));
            assert_eq!(context.system, context.instructions);
            Ok(Some(RawToolCall {
                tool_name: "weather".into(),
                input: r#"{"city":"Singapore","days":3}"#.into(),
                ..context.tool_call
            }))
        }
    });

    let call = parse_tool_call(
        raw("weather", r#"{"city":"Singapore"#),
        Some(&[weather_tool()]),
        Some(&repair),
        &[],
        Some("Use metric units"),
    )
    .await;

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(call.input, json!({ "city": "Singapore", "days": 3 }));
    assert_eq!(call.invalid, None);
}

#[tokio::test]
async fn an_invalid_repair_is_not_repaired_again() {
    let calls = Arc::new(AtomicUsize::new(0));
    let repair_calls = Arc::clone(&calls);
    let repair = ToolCallRepair::new(move |context| {
        repair_calls.fetch_add(1, Ordering::SeqCst);
        async move {
            Ok(Some(RawToolCall {
                input: r#"{"city":7}"#.into(),
                ..context.tool_call
            }))
        }
    });

    let call = parse_tool_call(
        raw("weather", "{"),
        Some(&[weather_tool()]),
        Some(&repair),
        &[],
        None,
    )
    .await;

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(call.input, json!("{"));
    assert_eq!(call.invalid, Some(true));
    assert!(matches!(
        call.error,
        Some(AiMuxError::InvalidToolInput { tool_input, .. })
            if tool_input == r#"{"city":7}"#
    ));
}

#[tokio::test]
async fn missing_tools_bypasses_repair_like_ai_sdk() {
    let calls = Arc::new(AtomicUsize::new(0));
    let repair_calls = Arc::clone(&calls);
    let repair = ToolCallRepair::new(move |_| {
        repair_calls.fetch_add(1, Ordering::SeqCst);
        async { Ok(None) }
    });

    let call = parse_tool_call(raw("weather", "{}"), None, Some(&repair), &[], None).await;

    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(matches!(
        call.error,
        Some(AiMuxError::NoSuchTool {
            available_tools: None,
            ..
        })
    ));
}

#[tokio::test]
async fn repair_returning_none_keeps_the_original_failure() {
    let repair = ToolCallRepair::new(|_| async { Ok(None) });
    let original_input = "{";
    let call = parse_tool_call(
        raw("weather", original_input),
        Some(&[weather_tool()]),
        Some(&repair),
        &[],
        None,
    )
    .await;

    assert_eq!(call.input, json!(original_input));
    assert!(matches!(
        call.error,
        Some(AiMuxError::InvalidToolInput { tool_input, .. })
            if tool_input == original_input
    ));
}

#[tokio::test]
async fn repair_failure_keeps_both_typed_errors() {
    let repair = ToolCallRepair::new(|context| async move {
        assert_eq!(context.input_schema("weather"), weather_tool_schema());
        Err(AiMuxError::Other("repair model failed".into()))
    });

    let call = parse_tool_call(
        raw("weather", "{"),
        Some(&[weather_tool()]),
        Some(&repair),
        &[],
        None,
    )
    .await;

    assert!(matches!(
        call.error,
        Some(AiMuxError::ToolCallRepair { original_error, cause })
            if matches!(*original_error, AiMuxError::InvalidToolInput { .. })
                && matches!(*cause, AiMuxError::Other(_))
    ));
}

#[tokio::test]
async fn provider_executed_dynamic_calls_do_not_require_a_local_tool() {
    let mut tool_call = raw("provider_search", r#"{"query":"rust"}"#);
    tool_call.provider_executed = Some(true);
    tool_call.dynamic = Some(true);

    let call = parse_tool_call(tool_call, None, None, &[], None).await;

    assert_eq!(call.input, json!({ "query": "rust" }));
    assert_eq!(call.invalid, None);
}

fn weather_tool_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "city": { "type": "string" },
            "days": { "type": "integer" }
        },
        "required": ["city"],
        "additionalProperties": false
    })
}

struct RawToolModel {
    tool_name: &'static str,
    input: &'static str,
    leading_text: bool,
    stream_input: bool,
}

impl RawToolModel {
    fn new(input: &'static str) -> Self {
        Self {
            tool_name: "weather",
            input,
            leading_text: false,
            stream_input: false,
        }
    }

    fn named(mut self, tool_name: &'static str) -> Self {
        self.tool_name = tool_name;
        self
    }

    fn with_streamed_input(mut self) -> Self {
        self.stream_input = true;
        self
    }
}

#[async_trait]
impl LanguageModel for RawToolModel {
    fn provider(&self) -> &str {
        "mock"
    }

    fn model_id(&self) -> &str {
        "raw-tool-model"
    }

    async fn do_generate(&self, _options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        Ok(GenerateResult {
            content: vec![GenerateContent::ToolCall {
                tool_call_id: "call-1".into(),
                tool_name: self.tool_name.into(),
                input: json!(self.input),
                provider_executed: None,
                dynamic: None,
                thought_signature: None,
                provider_metadata: None,
            }],
            finish_reason: FinishReason {
                unified: FinishReasonUnified::ToolCalls,
                raw: Some("tool_calls".into()),
            },
            usage: Usage::default(),
            warnings: vec![],
            provider_metadata: None,
            response: Default::default(),
            request_body: None,
            response_headers: None,
        })
    }

    async fn do_stream(&self, _options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let mut parts = vec![Ok(StreamPart::StreamStart { warnings: vec![] })];
        if self.leading_text {
            parts.push(Ok(StreamPart::TextDelta {
                id: "text-1".into(),
                delta: "ready".into(),
                provider_metadata: None,
            }));
        }
        if self.stream_input {
            parts.extend([
                Ok(StreamPart::ToolInputStart {
                    id: "call-1".into(),
                    tool_name: self.tool_name.into(),
                    provider_executed: None,
                    dynamic: None,
                    title: None,
                    provider_metadata: None,
                }),
                Ok(StreamPart::ToolInputDelta {
                    id: "call-1".into(),
                    delta: self.input.into(),
                    provider_metadata: None,
                }),
                Ok(StreamPart::ToolInputEnd {
                    id: "call-1".into(),
                    provider_metadata: None,
                }),
            ]);
        }
        parts.extend([
            Ok(StreamPart::ToolCall {
                tool_call_id: "call-1".into(),
                tool_name: self.tool_name.into(),
                input: json!(self.input),
                provider_executed: None,
                dynamic: None,
                thought_signature: None,
                invalid: None,
                error: None,
                provider_metadata: None,
            }),
            Ok(StreamPart::Finish {
                finish_reason: FinishReason {
                    unified: FinishReasonUnified::ToolCalls,
                    raw: Some("tool_calls".into()),
                },
                usage: Usage::default(),
                provider_metadata: None,
            }),
        ]);
        Ok(StreamResult {
            stream: Box::pin(futures::stream::iter(parts)),
            request_body: None,
            response_headers: None,
        })
    }
}

#[tokio::test]
async fn generate_text_parses_provider_raw_input_at_the_core_boundary() {
    let model = RawToolModel::new(r#"{"city":"Singapore"}"#);
    let result = generate_text(
        &model,
        "weather",
        GenerateTextOptions {
            tools: Some(vec![weather_tool()]),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    assert_eq!(result.tool_calls[0].input, json!({ "city": "Singapore" }));
    assert_eq!(result.tool_calls[0].invalid, None);
    assert!(matches!(
        &result.raw.content[0],
        GenerateContent::ToolCall { input, .. }
            if input == &json!(r#"{"city":"Singapore"}"#)
    ));
}

#[tokio::test]
async fn stream_text_parses_provider_raw_input_at_the_core_boundary() {
    let model = RawToolModel::new(r#"{"city":"Singapore"}"#);
    let mut result = stream_text(
        &model,
        "weather",
        GenerateTextOptions {
            tools: Some(vec![weather_tool()]),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    while let Some(part) = result.stream.next().await {
        if let StreamPart::ToolCall { input, invalid, .. } = part.unwrap() {
            assert_eq!(input, json!({ "city": "Singapore" }));
            assert_eq!(invalid, None);
            return;
        }
    }
    panic!("expected a parsed tool call");
}

#[tokio::test]
async fn openai_outputs_preserve_invalid_raw_tool_arguments() {
    let raw_input = r#"{"city":"Singapore"#;
    let model = RawToolModel::new(raw_input);
    let completion = generate_text_as_openai(
        &model,
        "weather",
        GenerateTextOptions {
            tools: Some(vec![weather_tool()]),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        completion.choices[0].message.tool_calls.as_ref().unwrap()[0]
            .function
            .arguments,
        raw_input
    );

    let result = stream_text_as_openai(
        &model,
        "weather",
        GenerateTextOptions {
            tools: Some(vec![weather_tool()]),
            ..Default::default()
        },
        OpenAiStreamOptions::default(),
    )
    .await
    .unwrap();
    let chunks = result
        .stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    let arguments = chunks
        .iter()
        .flat_map(|chunk| &chunk.choices)
        .filter_map(|choice| choice.delta.tool_calls.as_ref())
        .flatten()
        .find_map(|tool_call| tool_call.function.arguments.as_deref())
        .expect("expected complete tool call arguments");
    assert_eq!(arguments, raw_input);
}

#[tokio::test]
async fn openai_outputs_use_repaired_tool_name_and_input_for_complete_calls() {
    let repair = ToolCallRepair::new(|context| async move {
        Ok(Some(RawToolCall {
            tool_name: "weather".into(),
            input: r#"{"city":"Singapore","days":3}"#.into(),
            ..context.tool_call
        }))
    });
    let model = RawToolModel::new(r#"{"place":"Singapore"}"#)
        .named("forecast")
        .with_streamed_input();
    let options = GenerateTextOptions {
        tools: Some(vec![weather_tool()]),
        repair_tool_call: Some(repair.clone()),
        ..Default::default()
    };

    let completion = generate_text_as_openai(&model, "weather", options)
        .await
        .unwrap();
    let non_stream_call = &completion.choices[0].message.tool_calls.as_ref().unwrap()[0];

    let stream = stream_text_as_openai(
        &model,
        "weather",
        GenerateTextOptions {
            tools: Some(vec![weather_tool()]),
            repair_tool_call: Some(repair),
            ..Default::default()
        },
        OpenAiStreamOptions::default(),
    )
    .await
    .unwrap();
    let chunks = stream
        .stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let stream_calls = chunks
        .iter()
        .flat_map(|chunk| &chunk.choices)
        .filter_map(|choice| choice.delta.tool_calls.as_ref())
        .flatten()
        .collect::<Vec<_>>();
    assert_eq!(
        stream_calls.len(),
        1,
        "provider input frames must stay buffered when repair can replace them"
    );
    let stream_call = stream_calls[0];

    assert_eq!(non_stream_call.function.name, "weather");
    assert_eq!(
        non_stream_call.function.arguments,
        r#"{"city":"Singapore","days":3}"#
    );
    assert_eq!(
        stream_call.function.name.as_deref(),
        Some(non_stream_call.function.name.as_str())
    );
    assert_eq!(
        stream_call.function.arguments.as_deref(),
        Some(non_stream_call.function.arguments.as_str())
    );
}

#[tokio::test]
async fn openai_stream_without_repair_preserves_provider_tool_input_deltas() {
    let model = RawToolModel::new(r#"{"city":"Singapore"}"#).with_streamed_input();
    let result = stream_text_as_openai(
        &model,
        "weather",
        GenerateTextOptions {
            tools: Some(vec![weather_tool()]),
            ..Default::default()
        },
        OpenAiStreamOptions::default(),
    )
    .await
    .unwrap();
    let chunks = result
        .stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let tool_calls = chunks
        .iter()
        .flat_map(|chunk| &chunk.choices)
        .filter_map(|choice| choice.delta.tool_calls.as_ref())
        .flatten()
        .collect::<Vec<_>>();

    assert_eq!(tool_calls.len(), 2);
    assert_eq!(tool_calls[0].id.as_deref(), Some("call-1"));
    assert_eq!(tool_calls[0].function.name.as_deref(), Some("weather"));
    assert_eq!(tool_calls[0].function.arguments.as_deref(), Some(""));
    assert_eq!(tool_calls[1].id, None);
    assert_eq!(tool_calls[1].function.name, None);
    assert_eq!(
        tool_calls[1].function.arguments.as_deref(),
        Some(r#"{"city":"Singapore"}"#)
    );
}
