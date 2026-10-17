//! The OpenAI-compatible output (`generate_text_as_openai`,
//! `stream_text_as_openai`) on top of Core tool-call parsing and repair.
//!
//! The OpenAI output is an aimux extension with no AI SDK counterpart, so
//! these tests pin aimux behaviour (RFC-0035, RFC-0042 §5 items 7-8) rather
//! than upstream behaviour: the non-streaming completion is the native result
//! after parsing and repair; the stream forwards the provider's tool-input
//! deltas as they arrive and never rewrites them; an invalid call keeps the
//! provider's argument text byte for byte on both paths.

use aimux_core::error::AiMuxError;
use aimux_core::generate::{
    GenerateTextOptions, generate_text, generate_text_as_openai, stream_text, stream_text_as_openai,
};
use aimux_core::language_model::LanguageModel;
use aimux_core::openai_output::OpenAiStreamOptions;
use aimux_core::options::CallOptions;
use aimux_core::parse_tool_call::ToolCallRepair;
use aimux_core::result::{GenerateContent, GenerateResult, StreamResult};
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::{FunctionTool, RawToolCall, Tool, ToolCall};
use aimux_core::types::{FinishReason, FinishReasonUnified, Usage};
use async_trait::async_trait;
use futures::StreamExt;
use serde_json::json;

fn weather_tool() -> Tool {
    FunctionTool::new("weather", weather_tool_schema()).into()
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

/// A model that answers with one tool call `call-1` whose argument text is
/// `input`, verbatim. When streaming with `stream_input`, the same text first
/// arrives as a single tool-input delta between a start and an end.
struct RawToolModel {
    tool_name: &'static str,
    input: &'static str,
    stream_input: bool,
}

impl RawToolModel {
    fn new(input: &'static str) -> Self {
        Self {
            tool_name: "weather",
            input,
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
            content: vec![GenerateContent::ToolCall(RawToolCall {
                tool_call_id: "call-1".into(),
                tool_name: self.tool_name.into(),
                input: self.input.to_string(),
                provider_executed: None,
                dynamic: None,
                provider_metadata: None,
            })],
            finish_reason: FinishReason {
                unified: FinishReasonUnified::ToolCalls,
                raw: Some("tool_calls".into()),
            },
            usage: Usage::default(),
            warnings: vec![],
            provider_metadata: None,
            response: None,
            request: None,
        })
    }

    async fn do_stream(&self, _options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let mut parts: Vec<Result<StreamPart, AiMuxError>> =
            vec![Ok(StreamPart::StreamStart { warnings: vec![] })];
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
            Ok(StreamPart::ToolCall(RawToolCall {
                tool_call_id: "call-1".into(),
                tool_name: self.tool_name.into(),
                input: self.input.to_string(),
                provider_executed: None,
                dynamic: None,
                provider_metadata: None,
            })),
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
            request: None,
            response: None,
        })
    }
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

/// RFC-0035: the non-streaming OpenAI output reflects repair; the stream does
/// not — tool-input deltas are the provider's text, forwarded as they arrive,
/// exactly as in the AI SDK.
#[tokio::test]
async fn openai_repair_shows_in_the_completion_but_not_in_the_stream() {
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
    assert_eq!(non_stream_call.function.name, "weather");
    assert_eq!(
        non_stream_call.function.arguments,
        r#"{"city":"Singapore","days":3}"#
    );

    // The stream opened on the provider's name before repair could run, and
    // its deltas carry the provider's own text.
    assert_eq!(stream_calls[0].function.name.as_deref(), Some("forecast"));
    let streamed_arguments = stream_calls
        .iter()
        .filter_map(|call| call.function.arguments.as_deref())
        .collect::<String>();
    assert_eq!(streamed_arguments, r#"{"place":"Singapore"}"#);
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

/// Both OpenAI outputs show `model`'s tool name and exactly `expected` as the
/// arguments: the completion's call, and the stream's first name plus its
/// argument chunks joined.
async fn assert_openai_arguments(
    model: &RawToolModel,
    options: GenerateTextOptions,
    expected: &str,
) {
    let completion = generate_text_as_openai(model, "weather", options.clone())
        .await
        .unwrap();
    let call = &completion.choices[0].message.tool_calls.as_ref().unwrap()[0];
    assert_eq!(call.function.name, model.tool_name);
    assert_eq!(call.function.arguments, expected);

    let result = stream_text_as_openai(model, "weather", options, OpenAiStreamOptions::default())
        .await
        .unwrap();
    let chunks = result
        .stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let calls: Vec<_> = chunks
        .iter()
        .flat_map(|chunk| &chunk.choices)
        .filter_map(|choice| choice.delta.tool_calls.as_ref())
        .flatten()
        .collect();
    assert_eq!(
        calls.iter().find_map(|call| call.function.name.as_deref()),
        Some(model.tool_name)
    );
    let arguments: String = calls
        .iter()
        .filter_map(|call| call.function.arguments.as_deref())
        .collect();
    assert_eq!(arguments, expected);
}

/// A repair whose replacement still fails validation leaves the original call
/// (ai@7.0.127 parse-tool-call.ts:112-133): both OpenAI outputs keep the
/// original name and the provider's original argument text, not the
/// replacement's.
#[tokio::test]
async fn invalid_repair_keeps_original_arguments_with_original_name() {
    for name in ["forecast", "weather"] {
        let repair = ToolCallRepair::new(|context| async move {
            Ok(Some(RawToolCall {
                tool_name: "weather".into(),
                input: r#"{"city":7}"#.into(),
                ..context.tool_call
            }))
        });
        let model = RawToolModel::new(r#"{"place":"Tokyo"}"#)
            .named(name)
            .with_streamed_input();
        assert_openai_arguments(
            &model,
            GenerateTextOptions {
                tools: Some(vec![weather_tool()]),
                repair_tool_call: Some(repair),
                ..Default::default()
            },
            r#"{"place":"Tokyo"}"#,
        )
        .await;
    }
}

/// An unknown tool's arguments reach both OpenAI outputs byte for byte — a
/// JSON string keeps its quotes, non-JSON text, `null` and surrounding
/// whitespace survive — whether or not the provider streamed input deltas.
#[tokio::test]
async fn unknown_tool_preserves_raw_arguments_with_and_without_input_deltas() {
    for input in [r#""hello""#, "hello", "null", " { \"city\": \"Tokyo\" } "] {
        for deltas in [false, true] {
            let mut model = RawToolModel::new(input).named("unknown");
            model.stream_input = deltas;
            assert_openai_arguments(
                &model,
                GenerateTextOptions {
                    tools: Some(vec![weather_tool()]),
                    ..Default::default()
                },
                input,
            )
            .await;
        }
    }
}

/// A known tool's arguments that fail parsing or validation reach both OpenAI
/// outputs byte for byte, with and without input deltas: truncated JSON, a
/// JSON string, text that is not JSON, `null`, and a wrong-typed object with
/// surrounding whitespace.
#[tokio::test]
async fn known_tool_invalid_arguments_stay_raw_with_and_without_input_deltas() {
    for input in [
        r#"{"city":"Singapore"#,
        r#""hello""#,
        "hello",
        "null",
        " { \"city\": 7 } ",
    ] {
        for deltas in [false, true] {
            let mut model = RawToolModel::new(input);
            model.stream_input = deltas;
            assert_openai_arguments(
                &model,
                GenerateTextOptions {
                    tools: Some(vec![weather_tool()]),
                    ..Default::default()
                },
                input,
            )
            .await;
        }
    }
}

// ── Also deleted by cc5446fa, not in the review's list ──────────────────────
// Native-API counterparts of the tests above (same mock).

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
        GenerateContent::ToolCall(RawToolCall { input, .. })
            if input == r#"{"city":"Singapore"}"#
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
        if let StreamPart::ToolCall(ToolCall { input, invalid, .. }) = part.unwrap() {
            assert_eq!(input, json!({ "city": "Singapore" }));
            assert_eq!(invalid, None);
            return;
        }
    }
    panic!("expected a parsed tool call");
}
