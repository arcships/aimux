//! Catalogue, host-side repair, subscription refresh and OpenAI output conversion.

use std::time::Duration;

use aimux_core::{
    AiMuxError,
    generate::{GenerateTextOptions, GenerateTextResult},
    message::ModelPrompt,
    parse_tool_call::{self, ToolCallRepairReply},
    tool::ToolCall,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

use crate::{
    dispatch::{CallFuture, Ctx, Handler, Target},
    envelope::{BindingError, OpsError, ProtocolError},
    handle::{Entry, HandleId, HandleKind},
    op::{Cancellation, OpDescriptor, OpMode, RequestBytePolicy, Scope, TypeRef},
};

pub(crate) static OPS: &[OpDescriptor] = &[
    OpDescriptor {
        name: "catalogue.fetch",
        since_ops_version: "1.0",
        mode: OpMode::Call,
        allowed_handle_kinds: &[],
        request_type: TypeRef::of::<CatalogueFetchRequest>(),
        result_type: TypeRef::of::<aimux_providers::catalogue::Catalogue>(),
        event_type: None,
        scope: Scope::Current,
        terminal_type: None,
        runtime_handle_fields: &[("abort", &[HandleKind::Abort])],
        required_capabilities: &[],
        cancellation: Cancellation::Abort,
        result_ownership: "owned",
        request_byte_policy: RequestBytePolicy::Json,
        side_effects: false,
        retry_owner: "none",
        source_symbols: &["aimux_providers::get_model_specs"],
    },
    OpDescriptor {
        name: "toolRepair.context",
        since_ops_version: "1.0",
        mode: OpMode::Call,
        allowed_handle_kinds: &[],
        request_type: TypeRef::of::<ToolRepairContextRequest>(),
        result_type: TypeRef::of::<ToolRepairContextResult>(),
        event_type: None,
        scope: Scope::Current,
        terminal_type: None,
        runtime_handle_fields: &[],
        required_capabilities: &[],
        cancellation: Cancellation::None,
        result_ownership: "owned",
        request_byte_policy: RequestBytePolicy::JsonWithBase64,
        side_effects: false,
        retry_owner: "none",
        source_symbols: &["aimux_core::parse_tool_call::tool_call_repair_context"],
    },
    OpDescriptor {
        name: "toolRepair.apply",
        since_ops_version: "1.0",
        mode: OpMode::Call,
        allowed_handle_kinds: &[],
        request_type: TypeRef::of::<ToolRepairApplyRequest>(),
        result_type: TypeRef::of::<aimux_core::tool::ToolCall>(),
        event_type: None,
        scope: Scope::Current,
        terminal_type: None,
        runtime_handle_fields: &[],
        required_capabilities: &[],
        cancellation: Cancellation::None,
        result_ownership: "owned",
        request_byte_policy: RequestBytePolicy::Json,
        side_effects: false,
        retry_owner: "none",
        source_symbols: &["aimux_core::parse_tool_call::apply_tool_call_repair"],
    },
    OpDescriptor {
        name: "toolRepair.applyResult",
        since_ops_version: "1.0",
        mode: OpMode::Call,
        allowed_handle_kinds: &[],
        request_type: TypeRef::of::<ToolRepairApplyResultRequest>(),
        result_type: TypeRef::of::<aimux_core::generate::GenerateTextResult>(),
        event_type: None,
        scope: Scope::Current,
        terminal_type: None,
        runtime_handle_fields: &[],
        required_capabilities: &[],
        cancellation: Cancellation::None,
        result_ownership: "owned",
        request_byte_policy: RequestBytePolicy::JsonWithBase64,
        side_effects: false,
        retry_owner: "none",
        source_symbols: &["aimux_core::parse_tool_call::apply_tool_call_repair_to_result"],
    },
    OpDescriptor {
        name: "auth.codexRefresh",
        since_ops_version: "1.0",
        mode: OpMode::Call,
        allowed_handle_kinds: &[],
        request_type: TypeRef::of::<CodexRefreshRequest>(),
        result_type: TypeRef::of::<aimux_providers::codex::CodexTokens>(),
        event_type: None,
        scope: Scope::Current,
        terminal_type: None,
        runtime_handle_fields: &[],
        required_capabilities: &[],
        cancellation: Cancellation::None,
        result_ownership: "owned",
        request_byte_policy: RequestBytePolicy::Json,
        side_effects: true,
        retry_owner: "none",
        source_symbols: &["aimux_providers::codex_refresh"],
    },
    OpDescriptor {
        name: "output.toOpenAI",
        since_ops_version: "1.0",
        mode: OpMode::Call,
        allowed_handle_kinds: &[HandleKind::Language],
        request_type: TypeRef::of::<OpenAiOutputRequest>(),
        result_type: TypeRef::of::<aimux_core::openai_output::ChatCompletion>(),
        event_type: None,
        scope: Scope::Current,
        terminal_type: None,
        runtime_handle_fields: &[],
        required_capabilities: &[],
        cancellation: Cancellation::None,
        result_ownership: "owned",
        request_byte_policy: RequestBytePolicy::JsonWithBase64,
        side_effects: false,
        retry_owner: "none",
        source_symbols: &["aimux_core::generate::generate_text_result_to_chat_completion"],
    },
];

pub(crate) fn register(r: &mut Vec<(&'static OpDescriptor, Handler)>) {
    let handlers: [fn(Ctx, Target, Value) -> CallFuture; 6] = [
        catalogue_fetch,
        repair_context,
        repair_apply,
        repair_apply_result,
        codex_refresh,
        to_openai,
    ];
    r.extend(
        OPS.iter()
            .zip(handlers)
            .map(|(op, handler)| (op, Handler::Call(handler))),
    );
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CatalogueFetchRequest {
    source_url: Option<String>,
    #[serde(default, rename = "abort")]
    _abort: Option<HandleId>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ToolRepairContextRequest {
    tool_call: ToolCall,
    prompt: ModelPrompt,
    #[serde(default)]
    options: GenerateTextOptions,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ToolRepairApplyRequest {
    tool_call: ToolCall,
    #[serde(default)]
    options: GenerateTextOptions,
    reply: ToolCallRepairReply,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ToolRepairApplyResultRequest {
    #[schemars(with = "GenerateTextResult")]
    result: Value,
    #[serde(default)]
    options: GenerateTextOptions,
    tool_call_id: String,
    reply: ToolCallRepairReply,
}

// Deliberately no Debug implementation: token-bearing requests must not be logged.
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CodexRefreshRequest {
    refresh_token: String,
    client_id: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OpenAiOutputRequest {
    result: GenerateTextResult,
    metadata_namespace: String,
}

fn params<T: DeserializeOwned>(value: Value) -> Result<T, OpsError> {
    // Avoid including request values (especially refresh tokens) in error messages.
    serde_json::from_value(value).map_err(|_| {
        OpsError::Protocol(ProtocolError::InvalidParams(
            "request does not match the operation schema".into(),
        ))
    })
}

fn output(value: &impl Serialize) -> Result<Value, OpsError> {
    serde_json::to_value(value)
        .map_err(|e| OpsError::Binding(BindingError::ResultSerialization(e.to_string())))
}

fn catalogue_fetch(ctx: Ctx, _target: Target, value: Value) -> CallFuture {
    Box::pin(async move {
        let request: CatalogueFetchRequest = params(value)?;
        // Keep the existing source/default, parser and 60s exchange timeout while
        // enabling the shared SSRF guard and bounded binary response handler.
        let response = aimux_provider_utils::get_from_api(
            aimux_provider_utils::HttpRequest {
                url: request
                    .source_url
                    .unwrap_or_else(|| aimux_providers::catalogue::DEFAULT_ANYA2A_URL.into()),
                validate_url: true,
                response_timeout: Some(Duration::from_secs(60)),
                abort_signal: ctx.abort,
                ..Default::default()
            },
            aimux_provider_utils::create_binary_response_handler(),
            aimux_provider_utils::create_status_code_error_response_handler(),
        )
        .await
        .map_err(OpsError::AiMux)?;
        let raw = serde_json::from_str(&String::from_utf8_lossy(&response.value)).map_err(|e| {
            OpsError::AiMux(AiMuxError::JsonParse(format!(
                "get_model_specs: invalid source JSON: {e}"
            )))
        })?;
        let catalogue =
            aimux_providers::catalogue::parse_anya2a_all(&raw).map_err(OpsError::AiMux)?;
        output(&catalogue)
    })
}

#[derive(Serialize, schemars::JsonSchema)]
#[serde(transparent)]
struct ToolRepairContextResult(#[schemars(with = "Option<ToolRepairContextSchema>")] Option<Value>);

#[derive(schemars::JsonSchema)]
#[schemars(rename_all = "camelCase")]
#[allow(dead_code)]
struct ToolRepairContextSchema {
    #[schemars(with = "aimux_core::tool::TaggedRawToolCall", extend("x-aimux-codec" = "tagged"))]
    tool_call: aimux_core::tool::RawToolCall,
    error: aimux_core::AiMuxError,
    input_schema: Value,
    tools: Vec<aimux_core::tool::Tool>,
    messages: Vec<aimux_core::message::ModelMessage>,
    instructions: Option<String>,
}

fn repair_context(_ctx: Ctx, _target: Target, value: Value) -> CallFuture {
    Box::pin(async move {
        let request: ToolRepairContextRequest = params(value)?;
        let (messages, instructions, tools) =
            parse_tool_call::tool_call_repair_inputs(request.prompt, &request.options);
        let context = parse_tool_call::tool_call_repair_context(
            &request.tool_call,
            tools,
            &messages,
            instructions,
        )
        .map_err(OpsError::AiMux)?;
        output(&ToolRepairContextResult(context))
    })
}

fn repair_apply(_ctx: Ctx, _target: Target, value: Value) -> CallFuture {
    Box::pin(async move {
        let request: ToolRepairApplyRequest = params(value)?;
        let repaired = parse_tool_call::apply_tool_call_repair(
            &request.tool_call,
            request.options.tools.as_deref(),
            request.reply,
        )
        .map_err(OpsError::AiMux)?;
        output(&aimux_core::wire::Tagged(&repaired))
    })
}

fn repair_apply_result(_ctx: Ctx, _target: Target, value: Value) -> CallFuture {
    Box::pin(async move {
        let request: ToolRepairApplyResultRequest = params(value)?;
        parse_tool_call::apply_tool_call_repair_to_result(
            &request.result,
            request.options.tools.as_deref(),
            &request.tool_call_id,
            request.reply,
        )
        .map_err(OpsError::AiMux)
    })
}

fn codex_refresh(_ctx: Ctx, _target: Target, value: Value) -> CallFuture {
    Box::pin(async move {
        let request: CodexRefreshRequest = params(value)?;
        // Exactly one refresh grant. The host owns token storage and retries.
        output(
            &aimux_providers::codex_refresh(&request.refresh_token, &request.client_id)
                .await
                .map_err(OpsError::AiMux)?,
        )
    })
}

fn to_openai(_ctx: Ctx, target: Target, value: Value) -> CallFuture {
    Box::pin(async move {
        let Target::Handle(Entry::Language(model)) = target else {
            return Err(OpsError::Binding(BindingError::InvalidHandle {
                expected: vec![HandleKind::Language],
                actual: match target {
                    Target::Global => None,
                    Target::Handle(entry) => Some(entry.kind()),
                },
            }));
        };
        let request: OpenAiOutputRequest = params(value)?;
        if request.metadata_namespace.is_empty() {
            return Err(OpsError::Protocol(ProtocolError::InvalidParams(
                "metadataNamespace must not be empty".into(),
            )));
        }
        // Core's result facts remain private. Restrict the canonical wire
        // snapshot and lift it back before calling its existing converter.
        let mut result = output(&request.result)?;
        if let Some(steps) = result.get_mut("steps").and_then(Value::as_array_mut) {
            for step in steps {
                if let Some(metadata) = step
                    .get_mut("providerMetadata")
                    .and_then(Value::as_object_mut)
                {
                    metadata.retain(|namespace, _| namespace == &request.metadata_namespace);
                }
            }
        }
        let result: GenerateTextResult = params(result)?;
        output(
            &aimux_core::generate::generate_text_result_to_chat_completion(
                &result,
                model.model_id(),
            ),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handle::Owner;
    use serde_json::json;

    fn ctx() -> Ctx {
        Ctx {
            owner: Owner::CProcess,
            abort: None,
            now: std::time::SystemTime::now,
        }
    }

    fn entry() -> Target {
        Target::Global
    }

    fn tools() -> Vec<aimux_core::tool::Tool> {
        vec![
            aimux_core::tool::FunctionTool::new(
                "weather",
                json!({
                    "type":"object", "properties":{"city":{"type":"string"}}, "required":["city"]
                }),
            )
            .into(),
        ]
    }

    async fn invalid_call() -> ToolCall {
        parse_tool_call::parse_tool_call(
            aimux_core::tool::RawToolCall {
                tool_call_id: "call-1".into(),
                tool_name: "weather".into(),
                input: r#"{"city":7}"#.into(),
                provider_executed: None,
                dynamic: None,
                provider_metadata: None,
            },
            Some(&tools()),
            None,
            &[],
            None,
        )
        .await
    }

    fn reply() -> Value {
        json!({"type":"repaired", "toolCall":{
            "toolCallId":"call-1", "toolName":"weather", "input":r#"{"city":"Tokyo"}"#
        }})
    }

    #[test]
    fn registers_every_descriptor_once() {
        let mut registered = Vec::new();
        register(&mut registered);
        assert_eq!(registered.len(), 6);
        for (descriptor, handler) in registered {
            assert!(matches!(handler, Handler::Call(_)));
            assert_eq!(
                OPS.iter().filter(|op| op.name == descriptor.name).count(),
                1
            );
            assert_eq!(
                descriptor.allowed_handle_kinds.is_empty(),
                descriptor.name != "output.toOpenAI"
            );
        }
        assert_eq!(OPS[4].retry_owner, "none");
        assert!(OPS[4].side_effects);
    }

    #[test]
    fn rejects_unknown_control_fields_without_exposing_values() {
        assert!(params::<CatalogueFetchRequest>(json!({"sourceUrl":null,"extra":true})).is_err());
        for value in [
            json!({"refreshToken":"secret","clientId":"c","secret-field":true}),
            json!({"refreshToken":{"secret":true},"clientId":"c"}),
        ] {
            match params::<CodexRefreshRequest>(value) {
                Err(OpsError::Protocol(ProtocolError::InvalidParams(message))) => {
                    assert!(!message.contains("secret"))
                }
                _ => panic!("expected InvalidParams"),
            }
        }
        assert!(
            params::<CatalogueFetchRequest>(json!({}))
                .unwrap()
                .source_url
                .is_none()
        );
    }

    #[tokio::test]
    async fn context_matches_core_and_preserves_no_tools_null() {
        let call = invalid_call().await;
        let request = json!({"toolCall":call,"prompt":"hello", "options":{
            "tools":tools(), "instructions":"repair carefully"
        }});
        let parsed: ToolRepairContextRequest = params(request.clone()).unwrap();
        let (messages, instructions, tools) =
            parse_tool_call::tool_call_repair_inputs(parsed.prompt, &parsed.options);
        let expected =
            parse_tool_call::tool_call_repair_context(&call, tools, &messages, instructions)
                .unwrap();
        assert_eq!(
            repair_context(ctx(), entry(), request).await.unwrap(),
            json!(expected)
        );
        assert_eq!(
            repair_context(ctx(), entry(), json!({"toolCall":call,"prompt":"hello"}))
                .await
                .unwrap(),
            Value::Null
        );
    }

    #[tokio::test]
    async fn apply_matches_core_for_all_reply_variants() {
        let call = invalid_call().await;
        for reply in [
            reply(),
            json!({"type":"unchanged"}),
            json!({"type":"failed","message":"host failed"}),
        ] {
            let expected = parse_tool_call::apply_tool_call_repair(
                &call,
                Some(&tools()),
                serde_json::from_value(reply.clone()).unwrap(),
            )
            .unwrap();
            let actual = repair_apply(
                ctx(),
                entry(),
                json!({"toolCall":call,"options":{"tools":tools()},"reply":reply}),
            )
            .await
            .unwrap();
            assert_eq!(actual, json!(aimux_core::wire::Tagged(&expected)));
            assert_eq!(actual["type"], "tool-call");
        }
        assert!(matches!(
            repair_apply(ctx(), entry(), json!({"toolCall":call,"reply":reply()})).await,
            Err(OpsError::AiMux(AiMuxError::InvalidArgument(_)))
        ));
    }

    #[tokio::test]
    async fn apply_result_updates_calls_and_response_messages() {
        let call = invalid_call().await;
        let step = aimux_core::step_result::StepResult {
            call_id: "call".into(),
            step_number: 0,
            model: aimux_core::step_result::StepModel {
                provider: "test".into(),
                model_id: "model".into(),
            },
            content: vec![aimux_core::result::ContentPart::ToolCall(call.clone())],
            finish_reason: aimux_core::types::FinishReasonUnified::ToolCalls,
            raw_finish_reason: None,
            usage: Default::default(),
            performance: Default::default(),
            warnings: vec![],
            request: Default::default(),
            response: aimux_core::step_result::StepResponse {
                id: "response".into(),
                timestamp: aimux_core::timestamp::Timestamp::now(),
                model_id: "model".into(),
                headers: None,
                body: None,
                messages: serde_json::from_value(json!([{"role":"assistant", "content":[{
                    "type":"tool-call", "toolCallId":"call-1", "toolName":"weather", "input":{"city":7}
                }]}])).unwrap(),
            },
            provider_metadata: None,
        };
        let result =
            json!({"steps":[step],"totalUsage":aimux_core::types::LanguageModelUsage::default()});
        let expected = parse_tool_call::apply_tool_call_repair_to_result(
            &result,
            Some(&tools()),
            "call-1",
            serde_json::from_value(reply()).unwrap(),
        )
        .unwrap();
        let actual = repair_apply_result(
            ctx(),
            entry(),
            json!({"result":result,"options":{"tools":tools()},
            "toolCallId":"call-1","reply":reply()}),
        )
        .await
        .unwrap();
        assert_eq!(actual, expected);
        assert_eq!(actual["toolCalls"][0]["input"], json!({"city":"Tokyo"}));
        assert_eq!(
            actual["responseMessages"][0]["content"][0]["input"],
            json!({"city":"Tokyo"})
        );
    }

    #[tokio::test]
    async fn catalogue_rejects_untrusted_sources_before_io() {
        for url in [
            "http://127.0.0.1/catalogue",
            "http://169.254.169.254/latest",
            "file:///tmp/catalogue",
            "http://localhost/catalogue",
        ] {
            assert!(matches!(
                catalogue_fetch(ctx(), entry(), json!({"sourceUrl":url})).await,
                Err(OpsError::AiMux(AiMuxError::InvalidArgument(_)))
            ));
        }
    }
}
