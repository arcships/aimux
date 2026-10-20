//! The wiremock harness shared by the `@ai-sdk/deepseek` chat ports: every
//! model is built with `create_deepseek(..).chat(..)` against a local server.

pub(super) use aimux_core::error::AiMuxError;
pub(super) use aimux_core::tool::RawToolCall;
pub(super) use futures::StreamExt;
pub(super) use serde_json::{Value, json};
pub(super) use wiremock::matchers::{method, path_regex};
pub(super) use wiremock::{Mock, MockServer, ResponseTemplate};

pub(super) use aimux_core::language_model::LanguageModel;
pub(super) use aimux_core::language_model_message::{
    AssistantPart, FilePart, LanguageModelMessage, LanguageModelPrompt, ReasoningPart, TextPart,
    ToolCallPart, ToolPart, ToolResultOutput, ToolResultPart, UserPart,
};
pub(super) use aimux_core::options::{CallOptions, ResponseFormat, Tool};
pub(super) use aimux_core::result::{GenerateContent, GenerateResult, ReasoningOutput};
pub(super) use aimux_core::shared::provider_namespace;
pub(super) use aimux_core::shared::{FileBytes, FileData};
pub(super) use aimux_core::stream_part::StreamPart;
pub(super) use aimux_core::tool::FunctionTool;
pub(super) use aimux_core::types::{
    FinishReasonUnified, ReasoningEffort, ResponseMetadata, Warning,
};
pub(super) use aimux_providers::deepseek::{
    DeepSeekChatLanguageModel, DeepSeekProviderSettings, create_deepseek,
};

pub(super) fn with_options(
    mut message: LanguageModelMessage,
    options: Value,
) -> LanguageModelMessage {
    let (LanguageModelMessage::System {
        provider_options, ..
    }
    | LanguageModelMessage::User {
        provider_options, ..
    }
    | LanguageModelMessage::Assistant {
        provider_options, ..
    }
    | LanguageModelMessage::Tool {
        provider_options, ..
    }) = &mut message;
    *provider_options = Some(serde_json::from_value(options).unwrap());
    message
}

/// The upstream `TEST_PROMPT`.
pub(super) fn test_prompt() -> LanguageModelPrompt {
    vec![LanguageModelMessage::user_text("Hello")]
}

pub(super) fn options() -> CallOptions {
    CallOptions::new(test_prompt())
}

pub(super) fn options_for(prompt: LanguageModelPrompt) -> CallOptions {
    CallOptions::new(prompt)
}

pub(super) fn with_provider_options(mut options: CallOptions, value: Value) -> CallOptions {
    options.provider_options = Some(serde_json::from_value(value).unwrap());
    options
}

pub(super) fn weather_tool() -> FunctionTool {
    FunctionTool {
        name: "weather".to_string(),
        description: None,
        input_schema: json!({
            "type": "object",
            "properties": { "location": { "type": "string" } },
            "required": ["location"],
            "additionalProperties": false,
            "$schema": "http://json-schema.org/draft-07/schema#"
        }),
        strict: None,
        provider_options: None,
        input_examples: None,
    }
}

pub(super) fn strict_tool(name: &str, strict: Option<bool>) -> Tool {
    Tool::Function(FunctionTool {
        name: name.to_string(),
        description: None,
        input_schema: json!({ "type": "object", "properties": {} }),
        strict,
        provider_options: None,
        input_examples: None,
    })
}

pub(super) fn settings(server: &MockServer, base_path: &str) -> DeepSeekProviderSettings {
    DeepSeekProviderSettings {
        api_key: Some("test-api-key".to_string()),
        base_url: Some(format!("{}{base_path}", server.uri())),
        ..Default::default()
    }
}

/// `provider.chat(model_id)` of the upstream tests.
pub(super) fn chat(server: &MockServer, model_id: &str) -> DeepSeekChatLanguageModel {
    create_deepseek(settings(server, ""))
        .unwrap()
        .chat(model_id)
}

/// `betaProvider.chat(model_id)` of the upstream tests.
pub(super) fn beta_chat(server: &MockServer, model_id: &str) -> DeepSeekChatLanguageModel {
    create_deepseek(settings(server, "/beta"))
        .unwrap()
        .chat(model_id)
}

pub(super) async fn json_server(body: Value) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path_regex(".*/chat/completions$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;
    server
}

pub(super) fn data_event(chunk: &Value) -> String {
    format!("data: {chunk}\n\n")
}

pub(super) async fn sse_server(chunks: Vec<String>) -> MockServer {
    let mut body = chunks.concat();
    body.push_str("data: [DONE]\n\n");
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path_regex(".*/chat/completions$"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(&server)
        .await;
    server
}

pub(super) async fn request_paths(server: &MockServer) -> Vec<String> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|request| request.url.path().to_string())
        .collect()
}

pub(super) fn text_response() -> Value {
    json!({
        "id": "00f10ecd-60b3-4707-b5db-e4bcadf7aea1",
        "object": "chat.completion",
        "created": 1764656316,
        "model": "deepseek-chat",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": "Hello, World!" },
            "logprobs": null,
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 13,
            "completion_tokens": 300,
            "total_tokens": 313,
            "prompt_cache_hit_tokens": 0,
            "prompt_cache_miss_tokens": 13
        },
        "system_fingerprint": "fp_eaab8d114b_prod0820_fp8_kvcache"
    })
}

/// Generate against a canned text response and return the request body.
pub(super) async fn generate_body(model_id: &str, options: &CallOptions) -> Value {
    let server = json_server(text_response()).await;
    chat(&server, model_id)
        .do_generate(options)
        .await
        .unwrap()
        .request
        .and_then(|r| r.body)
        .unwrap()
}

pub(super) async fn generate_result(model_id: &str, options: &CallOptions) -> GenerateResult {
    let server = json_server(text_response()).await;
    chat(&server, model_id).do_generate(options).await.unwrap()
}

/// The wire messages of a generate call.
pub(super) async fn wire_messages(model_id: &str, prompt: LanguageModelPrompt) -> Vec<Value> {
    generate_body(model_id, &options_for(prompt)).await["messages"]
        .as_array()
        .unwrap()
        .clone()
}

/// Run a generate call that must fail before anything is sent.
pub(super) async fn generate_error(model_id: &str, options: &CallOptions) -> String {
    let server = json_server(text_response()).await;
    let error = chat(&server, model_id)
        .do_generate(options)
        .await
        .expect_err("the call must be rejected");
    assert!(
        request_paths(&server).await.is_empty(),
        "nothing may be fetched"
    );
    error.to_string()
}

pub(super) fn warning_values(warnings: &[Warning]) -> Vec<Value> {
    warnings
        .iter()
        .map(|warning| serde_json::to_value(warning).unwrap())
        .collect()
}

pub(super) fn deprecated(setting: &str) -> Warning {
    Warning::Deprecated {
        setting: setting.to_string(),
        message: format!(
            "{setting} is deprecated by DeepSeek and has been omitted. Remove {setting} from the request."
        ),
    }
}

pub(super) fn thinking_unsupported(feature: &str) -> Warning {
    Warning::Unsupported {
        feature: feature.to_string(),
        details: Some(format!(
            "{feature} has no effect when DeepSeek thinking is enabled. Set providerOptions.deepseek.thinking.type to 'disabled' to use {feature}."
        )),
    }
}

pub(super) fn compatibility(feature: &str, details: &str) -> Warning {
    Warning::Compatibility {
        feature: feature.to_string(),
        details: Some(details.to_string()),
    }
}

pub(super) async fn collect(stream: aimux_core::result::StreamResult) -> Vec<StreamPart> {
    let mut parts = Vec::new();
    let mut stream = stream.stream;
    while let Some(part) = stream.next().await {
        parts.push(part.expect("stream part"));
    }
    parts
}

pub(super) fn text_chunks() -> Vec<String> {
    vec![
        data_event(&json!({
            "id": "c1", "object": "chat.completion.chunk", "created": 1764656316,
            "model": "deepseek-chat", "system_fingerprint": "fp_eaab8d114b_prod0820_fp8_kvcache",
            "choices": [{ "index": 0, "delta": { "role": "assistant", "content": "" }, "finish_reason": null }]
        })),
        data_event(&json!({
            "id": "c1", "object": "chat.completion.chunk", "created": 1764656316,
            "model": "deepseek-chat", "system_fingerprint": "fp_eaab8d114b_prod0820_fp8_kvcache",
            "choices": [{ "index": 0, "delta": { "content": "Hello" }, "finish_reason": null }]
        })),
        data_event(&json!({
            "id": "c1", "object": "chat.completion.chunk", "created": 1764656316,
            "model": "deepseek-chat", "system_fingerprint": "fp_eaab8d114b_prod0820_fp8_kvcache",
            "choices": [{ "index": 0, "delta": { "content": ", World!" }, "finish_reason": null }]
        })),
        data_event(&json!({
            "id": "c1", "object": "chat.completion.chunk", "created": 1764656316,
            "model": "deepseek-chat", "system_fingerprint": "fp_eaab8d114b_prod0820_fp8_kvcache",
            "choices": [{ "index": 0, "delta": { "content": "" }, "finish_reason": "stop" }],
            "usage": {
                "prompt_tokens": 13, "completion_tokens": 3, "total_tokens": 16,
                "prompt_cache_hit_tokens": 5, "prompt_cache_miss_tokens": 8
            }
        })),
    ]
}

pub(super) fn finish_metadata(parts: &[StreamPart]) -> Value {
    parts
        .iter()
        .find_map(|part| match part {
            StreamPart::Finish {
                provider_metadata, ..
            } => provider_metadata
                .as_ref()
                .map(|metadata| serde_json::to_value(metadata).unwrap()),
            _ => None,
        })
        .expect("a finish part")
}

pub(super) fn stream_start_warnings(parts: &[StreamPart]) -> Vec<Value> {
    match &parts[0] {
        StreamPart::StreamStart { warnings } => warning_values(warnings),
        other => panic!("expected stream-start, got {other:?}"),
    }
}

pub(super) fn logprob(token: &str, logprob: f64, bytes: Value) -> Value {
    json!({
        "token": token, "logprob": logprob, "bytes": bytes,
        "top_logprobs": [{ "token": token, "logprob": logprob, "bytes": bytes }]
    })
}

pub(super) fn reasoning_response() -> Value {
    json!({
        "id": "r1", "object": "chat.completion", "created": 1764656316, "model": "deepseek-reasoner",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "There are 3 r's in strawberry.",
                "reasoning_content": "Let me count the letters."
            },
            "finish_reason": "stop"
        }],
        "usage": { "prompt_tokens": 20, "completion_tokens": 30, "total_tokens": 50,
                   "completion_tokens_details": { "reasoning_tokens": 10 } }
    })
}

pub(super) fn logprobs_response() -> Value {
    let mut body = text_response();
    body["choices"][0]["message"] = json!({
        "role": "assistant", "content": "OK", "reasoning_content": "Reasoning"
    });
    body["choices"][0]["logprobs"] = json!({
        "content": [logprob("OK", -0.00002467602, json!([79, 75]))],
        "reasoning_content": [logprob("Reasoning", -0.1, Value::Null)]
    });
    body
}

pub(super) fn reasoning_options(effort: ReasoningEffort) -> CallOptions {
    CallOptions {
        reasoning: Some(effort),
        ..options()
    }
}

pub(super) fn tool_call_response() -> Value {
    json!({
        "id": "t1", "object": "chat.completion", "created": 1764656316, "model": "deepseek-reasoner",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant", "content": "",
                "reasoning_content": "I should look up the weather.",
                "tool_calls": [{
                    "id": "call_00_abc", "type": "function",
                    "function": { "name": "weather", "arguments": "{\"location\":\"San Francisco\"}" }
                }]
            },
            "finish_reason": "tool_calls"
        }],
        "usage": { "prompt_tokens": 50, "completion_tokens": 20, "total_tokens": 70 }
    })
}

pub(super) fn tool_options() -> CallOptions {
    let mut options = with_provider_options(
        options(),
        json!({ "deepseek": { "thinking": { "type": "enabled" } } }),
    );
    options.tools = Some(vec![Tool::Function(weather_tool())]);
    options
}

pub(super) fn weather_wire_tool() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": "weather",
            "parameters": {
                "$schema": "http://json-schema.org/draft-07/schema#",
                "additionalProperties": false,
                "properties": { "location": { "type": "string" } },
                "required": ["location"],
                "type": "object"
            }
        }
    })
}

pub(super) fn json_options(schema: Option<Value>) -> CallOptions {
    let mut options = tool_options();
    options.response_format = Some(ResponseFormat::Json {
        schema,
        name: None,
        description: None,
    });
    options
}

pub(super) fn prefix_prompt(prefix_options: Value) -> LanguageModelPrompt {
    vec![
        LanguageModelMessage::user_text("Complete this sentence."),
        with_options(
            LanguageModelMessage::Assistant {
                content: vec![AssistantPart::Text(TextPart {
                    text: ("The answer is").into(),
                    provider_options: None,
                })],
                provider_options: None,
            },
            json!({ "deepseek": prefix_options }),
        ),
    ]
}

// Upstream emits provider error envelopes as error parts, including the first event.
pub(super) async fn stream_error(data: &Value) -> aimux_core::error::ApiCallError {
    let server = sse_server(vec![data_event(data)]).await;
    let result = chat(&server, "deepseek-chat")
        .do_stream(&options())
        .await
        .unwrap();
    collect(result)
        .await
        .into_iter()
        .find_map(|part| match part {
            StreamPart::Error {
                error: AiMuxError::ApiCall(error),
            } => Some(*error),
            _ => None,
        })
        .expect("expected a provider error part")
}

pub(super) async fn stream_parts(
    model_id: &str,
    options: &CallOptions,
    chunks: Vec<String>,
) -> (Value, Vec<StreamPart>) {
    let server = sse_server(chunks).await;
    let result = chat(&server, model_id).do_stream(options).await.unwrap();
    let body = result.request.clone().and_then(|r| r.body).unwrap();
    (body, collect(result).await)
}

pub(super) fn png_data() -> UserPart {
    UserPart::File(FilePart {
        data: FileData::Data {
            data: FileBytes::Base64("AAECAw==".into()),
        },
        media_type: "image/png".into(),
        filename: None,
        provider_options: None,
    })
}

pub(super) fn file_with_options(mut part: UserPart, options: Value) -> UserPart {
    if let UserPart::File(file) = &mut part {
        file.provider_options = Some(serde_json::from_value(options).unwrap());
    }
    part
}

pub(super) fn tool_turn(reasoning: bool) -> Vec<LanguageModelMessage> {
    let mut assistant = vec![];
    if reasoning {
        assistant.push(AssistantPart::Reasoning(ReasoningPart {
            text: ("I think the tool will return the correct value.").into(),
            provider_options: None,
        }));
    }
    assistant.push(AssistantPart::ToolCall(ToolCallPart {
        tool_call_id: ("quux").into(),
        tool_name: ("thwomp").into(),
        input: json!({ "foo": "bar123" }),
        provider_executed: None,
        provider_options: None,
    }));
    vec![
        LanguageModelMessage::Assistant {
            content: assistant,
            provider_options: None,
        },
        LanguageModelMessage::Tool {
            content: vec![ToolPart::ToolResult(ToolResultPart {
                tool_call_id: ("quux").into(),
                output: ToolResultOutput::Json {
                    value: json!({ "oof": "321rab" }),
                    provider_options: None,
                },
                tool_name: "thwomp".into(),
                provider_options: None,
            })],
            provider_options: None,
        },
    ]
}

pub(super) fn wire_tool_call() -> Value {
    json!([{
        "id": "quux", "type": "function",
        "function": { "name": "thwomp", "arguments": "{\"foo\":\"bar123\"}" }
    }])
}
