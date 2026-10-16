//! Schemas for operation controls. Canonical business fields retain type references
//! until their schemas are published by RFC-0042.

use serde_json::{Value, json};

fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object", "properties":properties, "required":required, "additionalProperties":false})
}

fn string() -> Value {
    json!({"type":"string"})
}
fn nullable(schema: Value) -> Value {
    json!({"anyOf":[schema, {"type":"null"}]})
}
fn array(items: Value) -> Value {
    json!({"type":"array", "items":items})
}
fn business(path: &str) -> Value {
    json!({"x-aimux-type":path,"$comment":"Canonical business schema is deferred to RFC-0042 (G-d)."})
}
fn boolean() -> Value {
    json!({"type":"boolean"})
}
fn integer() -> Value {
    json!({"type":"integer","minimum":0,"maximum":u64::MAX})
}
fn methods() -> Value {
    json!({"enum":["languageModel","embeddingModel","imageModel","speechModel","transcriptionModel","rerankingModel","videoModel","searchModel","decision"]})
}
fn kind() -> Value {
    json!({"enum":crate::HandleKind::ALL})
}

pub(super) fn handle() -> Value {
    // DTO decoding permits leading zeroes but checks unsigned u64 overflow.
    // Bound the numeric suffix instead of rounding a JSON number.
    let maximum = u64::MAX.to_string();
    let mut choices = vec!["[0-9]{1,19}".to_owned()];
    for (index, digit) in maximum.bytes().enumerate() {
        if digit > b'0' {
            choices.push(format!(
                "{}[0-{}][0-9]{{{}}}",
                &maximum[..index],
                char::from(digit - 1),
                maximum.len() - index - 1
            ));
        }
    }
    choices.push(maximum);
    json!({"type":"string","pattern":format!("^0*({})$", choices.join("|")),"not":{"pattern":"[^0-9]"}})
}

fn proxy() -> Value {
    object(
        json!({"http_url":nullable(string()),"https_url":nullable(string()),"all_url":nullable(string()),"no_proxy":nullable(string())}),
        &[],
    )
}

fn handle_result(with_kind: bool) -> Value {
    if with_kind {
        object(
            json!({"handle":handle(),"kind":kind()}),
            &["handle", "kind"],
        )
    } else {
        object(json!({"handle":handle()}), &["handle"])
    }
}

pub(super) fn schema(id: &str) -> Option<Value> {
    let name = id.strip_prefix("aimux_ops::ops::")?;
    Some(match name {
        "runtime::RuntimeDescribeParams"
        | "registry::ModelInfoParams"
        | "recording::EmptyParams" => object(json!({}), &[]),
        "runtime::RuntimeConfigureParams" => object(
            json!({
                "logging":nullable(object(json!({"level":{"enum":["off","error","warn","info","debug","trace"]}}), &["level"])),
                "proxy":nullable(proxy())
            }),
            &[],
        ),
        "provider::ProviderCreateParams" => object(
            json!({"package":string(),"settings":provider_settings()}),
            &["package"],
        ),
        "provider::ProviderDefaultParams" => object(json!({"package":string()}), &["package"]),
        "provider::ProviderModelParams" => object(
            json!({"method":nullable(string()),"modelId":string()}),
            &["modelId"],
        ),
        "provider::ProviderInvokeParams" => object(
            json!({"method":string(),"abort":nullable(handle())}),
            &["method"],
        ),
        "provider::ProviderWrapParams" => object(
            json!({"languageModelMiddleware":array(handle()),"imageModelMiddleware":array(handle())}),
            &[],
        ),
        "provider::ProviderCustomParams" => provider_custom(),
        "provider::HandleResult" | "registry::HandleResult" | "handles::HandleResult" => {
            handle_result(true)
        }
        "provider::ProviderInvokeResult" => {
            array(business("aimux_core::model_catalogue::RuntimeModel"))
        }
        "registry::RegistryCreateParams" => object(
            json!({"namespace":string(),"entries":array(object(json!({"key":string(),"provider":handle()}), &["key","provider"]))}),
            &["entries"],
        ),
        "registry::RegistryProviderParams" => object(json!({"key":string()}), &["key"]),
        "registry::ModelResolveParams" => object(
            json!({"modelRef":object(json!({"namespace":string(),"key":string(),"method":methods(),"modelId":string()}), &["namespace","key","method","modelId"])}),
            &["modelRef"],
        ),
        "registry::ModelWrapParams" => {
            object(json!({"middleware":array(handle())}), &["middleware"])
        }
        "registry::ModelInfo" => object(
            json!({"provider":string(),"modelId":string(),"modality":string(),"capabilities":{},"source":{}}),
            &["provider", "modelId", "modality", "capabilities", "source"],
        ),
        "composite::RouterParams" => {
            let mut children = array(handle());
            children["minItems"] = json!(1);
            object(
                json!({"children":children,"strategy":{"oneOf":[
                object(json!({"type":{"const":"rule"}}), &["type"]),
                object(json!({"type":{"const":"weighted"},"weights":array(json!({"type":"number"}))}), &["type","weights"])
            ]},"fallback":{"enum":["on-error","none"]},"providerName":nullable(string()),"modelId":nullable(string())}),
                &["children", "strategy"],
            )
        }
        "composite::MoaParams" => object(
            json!({"references":array(handle()),"aggregator":handle(),"providerName":string(),"modelId":string(),"aggregatorInstructions":nullable(string()),"stripReferenceTools":boolean(),"failMode":{"enum":["best-effort","fail-fast"]}}),
            &["references", "aggregator"],
        ),
        "text::TextRequest" => text_request(false),
        "text::OpenAiTextRequest" => text_request(true),
        "decision::DecisionDecideRequest" => object(
            json!({"options":business("aimux_core::decision_model::DecideOptions"),"abort":nullable(handle())}),
            &["options"],
        ),
        "files::UploadFileParams" => object(
            json!({"data":{"oneOf":[
            object(json!({"type":{"const":"data"},"data":{"type":"string","contentEncoding":"base64"}}), &["type","data"]),
            object(json!({"type":{"const":"text"},"text":string()}), &["type","text"])
        ]},"mediaType":string(),"filename":nullable(string()),"providerOptions":nullable(json!({"type":"object"})),"abort":nullable(handle())}),
            &["data", "mediaType"],
        ),
        "handles::TransportParams" => json!({"oneOf":[
            object(json!({"kind":{"const":"fetch"},"proxy":proxy()}), &["kind"]),
            object(json!({"kind":{"const":"ws"},"proxy":proxy()}), &["kind"]),
            object(json!({"kind":{"const":"download"},"maxBytes":nullable(integer())}), &["kind"])
        ]}),
        "handles::CredentialParams" => object(json!({"kind":string()}), &["kind"]),
        "handles::MiddlewareParams" => object(
            json!({"kind":string(),"traceStore":nullable(handle()),"scopeName":nullable(string()),"auditMode":nullable(json!({"enum":["disabled","shared","strict"]}))}),
            &["kind"],
        ),
        "trace::TraceCreateParams" => {
            let mut capacity = integer();
            capacity["minimum"] = json!(1);
            object(
                json!({"capacity":capacity,"perScopeCapacity":capacity,"auditMode":{"enum":["off","shared","strict"]}}),
                &[],
            )
        }
        "trace::TraceAggregateParams" => object(
            json!({"filter":object(json!({"provider":nullable(string()),"model":nullable(string()),"sessionId":nullable(string()),"sinceUnixMs":nullable(json!({"type":"integer","minimum":i64::MIN,"maximum":i64::MAX}))}), &[])}),
            &[],
        ),
        "trace::TraceSessionParams" | "session::SessionStoreCallsParams" => {
            object(json!({"sessionId":string()}), &["sessionId"])
        }
        "trace::TraceAggregateResult" => array(business("aimux_core::trace::TraceStats")),
        "trace::TraceTrajectoryResult" => array(business("aimux_core::trace::SessionStepStat")),
        "session::SessionStoreCreateParams" => object(json!({"infer":boolean()}), &["infer"]),
        "session::SessionStoreHandle"
        | "recording::RecorderHandleResult"
        | "replay::ReplaySessionHandle" => handle_result(false),
        "session::SessionViews" => array(business("aimux_core::session::SessionView")),
        "session::SessionCalls" => array(business("aimux_core::session::SessionCall")),
        "recording::CreateRecorderParams" => object(
            json!({"backend":{"oneOf":[
            object(json!({"type":{"const":"ring"},"capacity":nullable(integer())}), &["type"]),
            object(json!({"type":{"const":"jsonl"},"directory":string()}), &["type","directory"])
        ]},"capturePolicy":{"const":"full"}}),
            &["backend", "capturePolicy"],
        ),
        "recording::RecorderExport" => object(json!({"jsonl":string()}), &["jsonl"]),
        "recording::RecorderStats" => recorder_stats(),
        "replay::ReplayCreateParams" => {
            object(json!({"recordingsJsonl":string()}), &["recordingsJsonl"])
        }
        "replay::ReplayLiveParams" => object(
            json!({"callId":string(),"target":object(json!({"kind":{"enum":["model","registry"]},"handle":handle()}), &["kind","handle"]),"abort":nullable(handle())}),
            &["callId", "target"],
        ),
        "replay::ReplayInspection" => object(
            json!({"recordings":array(object(json!({"callId":string(),"provider":business("aimux_core::recording::ProviderRecord"),"operation":business("aimux_core::recording::RecordingOperation"),"complete":boolean(),"transportClosed":boolean(),"exchangeCount":integer()}), &["callId","provider","operation","complete","transportClosed","exchangeCount"]))}),
            &["recordings"],
        ),
        "replay::ReplayPlan" => object(
            json!({"calls":array(object(json!({"callId":string(),"provider":business("aimux_core::recording::ProviderRecord"),"operation":business("aimux_core::recording::RecordingOperation"),"liveSupported":boolean(),"requiresTarget":boolean()}), &["callId","provider","operation","liveSupported","requiresTarget"]))}),
            &["calls"],
        ),
        "replay::ReplayLiveResult" => business(
            "aimux_core::generate::GenerateTextResult | aimux_core::decision_model::DecideResult",
        ),
        "misc::CatalogueFetchRequest" => object(
            json!({"sourceUrl":nullable(string()),"abort":nullable(handle())}),
            &[],
        ),
        "misc::ToolRepairContextRequest" => object(
            json!({"toolCall":business("aimux_core::ToolCall"),"prompt":business("aimux_core::ModelPrompt"),"options":business("aimux_core::GenerateTextOptions")}),
            &["toolCall", "prompt"],
        ),
        "misc::ToolRepairApplyRequest" => object(
            json!({"toolCall":business("aimux_core::ToolCall"),"reply":business("aimux_core::ToolCallRepairReply"),"options":business("aimux_core::GenerateTextOptions")}),
            &["toolCall", "reply"],
        ),
        "misc::ToolRepairApplyResultRequest" => object(
            json!({"result":{},"toolCallId":string(),"reply":business("aimux_core::ToolCallRepairReply"),"options":business("aimux_core::GenerateTextOptions")}),
            &["result", "toolCallId", "reply"],
        ),
        "misc::CodexRefreshRequest" => object(
            json!({"refreshToken":string(),"clientId":string()}),
            &["refreshToken", "clientId"],
        ),
        "misc::OpenAiOutputRequest" => object(
            json!({"result":business("aimux_core::generate::GenerateTextResult"),"metadataNamespace":{"type":"string","minLength":1}}),
            &["result", "metadataNamespace"],
        ),
        "misc::ToolRepairContextResult" => nullable(business("serde_json::Value")),
        _ => return None,
    })
}

fn provider_settings() -> Value {
    object(
        json!({"apiKey":nullable(string()),"baseUrl":nullable(string()),"headers":{"type":"object","additionalProperties":nullable(string())},"organization":nullable(string()),"project":nullable(string()),"params":{"type":"object","additionalProperties":string()},"fetch":nullable(handle()),"webSocket":nullable(handle()),"credentialResolver":nullable(handle()),"endpoint":nullable(string()),"probabilitySource":nullable(string())}),
        &[],
    )
}

fn provider_custom() -> Value {
    let mut properties = json!({"files":nullable(handle()),"fallbackProvider":nullable(handle()),"defaultMethod":nullable(string()),"languageModelMiddleware":array(handle()),"imageModelMiddleware":array(handle())});
    for field in [
        "languageModels",
        "embeddingModels",
        "imageModels",
        "speechModels",
        "transcriptionModels",
        "rerankingModels",
        "videoModels",
        "searchModels",
        "decisionModels",
    ] {
        properties[field] = json!({"type":"object","additionalProperties":handle()});
    }
    object(properties, &[])
}

fn text_request(openai: bool) -> Value {
    let mut properties = json!({"prompt":business("aimux_core::ModelPrompt"),"options":business("aimux_core::GenerateTextOptions"),"abort":nullable(handle())});
    if openai {
        properties["streamOptions"] = object(
            json!({"includeUsage":boolean(),"includeReasoning":boolean()}),
            &[],
        );
    }
    object(properties, &["prompt"])
}

fn recorder_stats() -> Value {
    json!({"oneOf":[
        object(json!({"type":{"const":"ring"},"completed":integer(),"pending":integer(),"dropped":integer(),"pendingEvicted":integer(),"inconsistentCallIds":array(string())}), &["type","completed","pending","dropped","pendingEvicted","inconsistentCallIds"]),
        object(json!({"type":{"const":"jsonl"},"path":string(),"dropped":integer(),"inconsistentCallIds":array(string()),"lastWriteError":nullable(string())}), &["type","path","dropped","inconsistentCallIds","lastWriteError"])
    ]})
}
