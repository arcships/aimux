use schemars::{JsonSchema, Schema, generate::SchemaSettings};
use serde_json::Value;

fn schema_for<T: ?Sized + JsonSchema>(serialize: bool) -> Schema {
    SchemaSettings::default()
        .with(|settings| {
            settings.contract = if serialize {
                schemars::generate::Contract::Serialize
            } else {
                schemars::generate::Contract::Deserialize
            };
        })
        .into_generator()
        .into_root_schema_for::<T>()
}

fn schema(id: &str, serialize: bool) -> Schema {
    macro_rules! roots {
        ($($id:literal => $ty:ty),* $(,)?) => {
            match id {
                $($id => schema_for::<$ty>(serialize),)*
                _ => panic!("uncovered op root: {id}"),
            }
        };
    }
    roots! {
        "()" => (),
        "BinaryInput" => aimux_ops::ops::operation::BinaryInput,
        "CancelResult" => aimux_ops::operation::CancelResult,
        "EmptyParams" => aimux_ops::ops::operation::EmptyParams,
        "ErrorEnvelope" => aimux_ops::envelope::ErrorEnvelope,
        "HandleId" => aimux_ops::handle::HandleId,
        "OperationNextParams" => aimux_ops::ops::operation::OperationNextParams,
        "OperationPollResult" => aimux_ops::ops::operation::OperationPollResult,
        "OperationStatus" => aimux_ops::operation::OperationStatus,
        "StreamFrame" => aimux_ops::frame::StreamFrame,
        "String" => String,
        "TranscriptionStartParams" => aimux_ops::ops::operation::TranscriptionStartParams,
        "aimux_core::decision_model::DecideResult" => aimux_core::decision_model::DecideResult,
        "aimux_core::embedding_model::EmbeddingCallOptions" => aimux_core::embedding_model::EmbeddingCallOptions,
        "aimux_core::embedding_model::EmbeddingResult" => aimux_core::embedding_model::EmbeddingResult,
        "aimux_core::files_model::UploadFileResult" => aimux_core::files_model::UploadFileResult,
        "aimux_core::generate::GenerateObjectResult" => aimux_core::generate::GenerateObjectResult,
        "aimux_core::generate::GenerateTextResult" => aimux_core::generate::GenerateTextResult,
        "aimux_core::image_model::ImageCallOptions" => aimux_core::image_model::ImageCallOptions,
        "aimux_core::image_model::ImageResult" => aimux_core::image_model::ImageResult,
        "aimux_core::openai_output::ChatCompletion" => aimux_core::openai_output::ChatCompletion,
        "aimux_core::openai_output::ChatCompletionChunk" => aimux_core::openai_output::ChatCompletionChunk,
        "aimux_core::reranking_model::RerankingCallOptions" => aimux_core::reranking_model::RerankingCallOptions,
        "aimux_core::reranking_model::RerankingResult" => aimux_core::reranking_model::RerankingResult,
        "aimux_core::search_model::SearchCallOptions" => aimux_core::search_model::SearchCallOptions,
        "aimux_core::search_model::SearchResult" => aimux_core::search_model::SearchResult,
        "aimux_core::speech_model::SpeechCallOptions" => aimux_core::speech_model::SpeechCallOptions,
        "aimux_core::speech_model::SpeechResult" => aimux_core::speech_model::SpeechResult,
        "aimux_core::stream_part::TextStreamPart" => aimux_core::stream_part::TextStreamPart,
        "aimux_core::tool::ToolCall" => aimux_core::tool::ToolCall,
        "aimux_core::trace::SessionChainView" => aimux_core::trace::SessionChainView,
        "aimux_core::transcription_model::TranscriptionCallOptions" => aimux_core::transcription_model::TranscriptionCallOptions,
        "aimux_core::transcription_model::TranscriptionResult" => aimux_core::transcription_model::TranscriptionResult,
        "aimux_core::video_model::VideoCallOptions" => aimux_core::video_model::VideoCallOptions,
        "aimux_core::video_model::VideoResult" => aimux_core::video_model::VideoResult,
        "aimux_ops::ops::composite::MoaParams" => aimux_ops::ops::composite::MoaParams,
        "aimux_ops::ops::composite::RouterParams" => aimux_ops::ops::composite::RouterParams,
        "aimux_ops::ops::decision::DecisionDecideRequest" => aimux_ops::ops::decision::DecisionDecideRequest,
        "aimux_ops::ops::files::UploadFileParams" => aimux_ops::ops::files::UploadFileParams,
        "aimux_ops::ops::handles::CredentialParams" => aimux_ops::ops::handles::CredentialParams,
        "aimux_ops::ops::handles::HandleResult" => aimux_ops::ops::handles::HandleResult,
        "aimux_ops::ops::handles::MiddlewareParams" => aimux_ops::ops::handles::MiddlewareParams,
        "aimux_ops::ops::handles::TransportParams" => aimux_ops::ops::handles::TransportParams,
        "aimux_ops::ops::misc::CatalogueFetchRequest" => aimux_ops::ops::misc::CatalogueFetchRequest,
        "aimux_ops::ops::misc::CodexRefreshRequest" => aimux_ops::ops::misc::CodexRefreshRequest,
        "aimux_ops::ops::misc::OpenAiOutputRequest" => aimux_ops::ops::misc::OpenAiOutputRequest,
        "aimux_ops::ops::misc::ToolRepairApplyRequest" => aimux_ops::ops::misc::ToolRepairApplyRequest,
        "aimux_ops::ops::misc::ToolRepairApplyResultRequest" => aimux_ops::ops::misc::ToolRepairApplyResultRequest,
        "aimux_ops::ops::misc::ToolRepairContextRequest" => aimux_ops::ops::misc::ToolRepairContextRequest,
        "aimux_ops::ops::misc::ToolRepairContextResult" => aimux_ops::ops::misc::ToolRepairContextResult,
        "aimux_ops::ops::provider::HandleResult" => aimux_ops::ops::provider::HandleResult,
        "aimux_ops::ops::provider::ProviderCreateParams" => aimux_ops::ops::provider::ProviderCreateParams,
        "aimux_ops::ops::provider::ProviderCustomParams" => aimux_ops::ops::provider::ProviderCustomParams,
        "aimux_ops::ops::provider::ProviderDefaultParams" => aimux_ops::ops::provider::ProviderDefaultParams,
        "aimux_ops::ops::provider::ProviderInvokeParams" => aimux_ops::ops::provider::ProviderInvokeParams,
        "aimux_ops::ops::provider::ProviderInvokeResult" => aimux_ops::ops::provider::ProviderInvokeResult,
        "aimux_ops::ops::provider::ProviderModelParams" => aimux_ops::ops::provider::ProviderModelParams,
        "aimux_ops::ops::provider::ProviderWrapParams" => aimux_ops::ops::provider::ProviderWrapParams,
        "aimux_ops::ops::recording::CreateRecorderParams" => aimux_ops::ops::recording::CreateRecorderParams,
        "aimux_ops::ops::recording::EmptyParams" => aimux_ops::ops::recording::EmptyParams,
        "aimux_ops::ops::recording::RecorderExport" => aimux_ops::ops::recording::RecorderExport,
        "aimux_ops::ops::recording::RecorderHandleResult" => aimux_ops::ops::recording::RecorderHandleResult,
        "aimux_ops::ops::recording::RecorderStats" => aimux_ops::ops::recording::RecorderStats,
        "aimux_ops::ops::registry::HandleResult" => aimux_ops::ops::registry::HandleResult,
        "aimux_ops::ops::registry::ModelInfo" => aimux_ops::ops::registry::ModelInfo,
        "aimux_ops::ops::registry::ModelInfoParams" => aimux_ops::ops::registry::ModelInfoParams,
        "aimux_ops::ops::registry::ModelResolveParams" => aimux_ops::ops::registry::ModelResolveParams,
        "aimux_ops::ops::registry::ModelWrapParams" => aimux_ops::ops::registry::ModelWrapParams,
        "aimux_ops::ops::registry::RegistryCreateParams" => aimux_ops::ops::registry::RegistryCreateParams,
        "aimux_ops::ops::registry::RegistryProviderParams" => aimux_ops::ops::registry::RegistryProviderParams,
        "aimux_ops::ops::replay::ReplayCreateParams" => aimux_ops::ops::replay::ReplayCreateParams,
        "aimux_ops::ops::replay::ReplayInspection" => aimux_ops::ops::replay::ReplayInspection,
        "aimux_ops::ops::replay::ReplayLiveParams" => aimux_ops::ops::replay::ReplayLiveParams,
        "aimux_ops::ops::replay::ReplayLiveResult" => aimux_ops::ops::replay::ReplayLiveResult,
        "aimux_ops::ops::replay::ReplayPlan" => aimux_ops::ops::replay::ReplayPlan,
        "aimux_ops::ops::replay::ReplaySessionHandle" => aimux_ops::ops::replay::ReplaySessionHandle,
        "aimux_ops::ops::runtime::RuntimeConfigureParams" => aimux_ops::ops::runtime::RuntimeConfigureParams,
        "aimux_ops::ops::runtime::RuntimeDescribeParams" => aimux_ops::ops::runtime::RuntimeDescribeParams,
        "aimux_ops::ops::session::SessionCalls" => aimux_ops::ops::session::SessionCalls,
        "aimux_ops::ops::session::SessionStoreCallsParams" => aimux_ops::ops::session::SessionStoreCallsParams,
        "aimux_ops::ops::session::SessionStoreCreateParams" => aimux_ops::ops::session::SessionStoreCreateParams,
        "aimux_ops::ops::session::SessionStoreHandle" => aimux_ops::ops::session::SessionStoreHandle,
        "aimux_ops::ops::session::SessionViews" => aimux_ops::ops::session::SessionViews,
        "aimux_ops::ops::text::OpenAiTextRequest" => aimux_ops::ops::text::OpenAiTextRequest,
        "aimux_ops::ops::text::TextRequest" => aimux_ops::ops::text::TextRequest,
        "aimux_ops::ops::trace::TraceAggregateParams" => aimux_ops::ops::trace::TraceAggregateParams,
        "aimux_ops::ops::trace::TraceAggregateResult" => aimux_ops::ops::trace::TraceAggregateResult,
        "aimux_ops::ops::trace::TraceCreateParams" => aimux_ops::ops::trace::TraceCreateParams,
        "aimux_ops::ops::trace::TraceSessionParams" => aimux_ops::ops::trace::TraceSessionParams,
        "aimux_ops::ops::trace::TraceTrajectoryResult" => aimux_ops::ops::trace::TraceTrajectoryResult,
        "aimux_providers::catalogue::Catalogue" => aimux_providers::catalogue::Catalogue,
        "aimux_providers::codex::CodexTokens" => aimux_providers::codex::CodexTokens,
        "serde_json::Value" => serde_json::Value,
    }
}

/// `true` is `serde_json::Value`, the deliberate opaque JSON type; a missing
/// description is a compile error, so only codec names need checking here.
fn check(schema: &Value, root: &str) {
    match schema {
        Value::Object(object) => {
            if let Some(codec) = object.get("x-aimux-codec") {
                assert!(
                    matches!(
                        codec.as_str(),
                        Some(
                            "base64"
                                | "u64-string"
                                | "presence"
                                | "timestamp"
                                | "tagged"
                                | "ordered-untagged"
                        )
                    ),
                    "{root}: unknown codec {codec}"
                );
            }
            object.values().for_each(|child| check(child, root));
        }
        Value::Array(items) => items.iter().for_each(|child| check(child, root)),
        _ => {}
    }
}

#[test]
fn every_op_root_has_a_schema_with_known_codecs() {
    let manifest = aimux_ops::manifest::manifest();
    for op in manifest["ops"].as_array().unwrap() {
        for key in ["requestType", "resultType", "eventType", "terminalType"] {
            if let Some(id) = op[key].as_str() {
                let schema = schema(id, key != "requestType").to_value();
                check(&schema, &format!("{} {key} {id}", op["name"]));
            }
        }
    }
    check(&schema("ErrorEnvelope", true).to_value(), "ErrorEnvelope");
}
