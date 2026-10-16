//! Rust oracle for the fixture-derived DTO vectors.
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

fn round_trip<T: Serialize + DeserializeOwned>(value: Value) -> Result<Value, serde_json::Error> {
    serde_json::to_value(serde_json::from_value::<T>(value)?)
}

#[test]
fn dto_vectors_round_trip_through_rust() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../contract-tests/dto");
    let index: Value =
        serde_json::from_str(&std::fs::read_to_string(root.join("index.json")).unwrap()).unwrap();
    let manifest = crate::manifest::manifest();
    let known: std::collections::BTreeSet<_> = ["input", "output"]
        .into_iter()
        .flat_map(|direction| {
            manifest["typeDefs"][direction]
                .as_object()
                .unwrap()
                .keys()
                .cloned()
        })
        .collect();
    let mut files = std::collections::BTreeSet::new();
    for entry in index.as_array().unwrap() {
        let name = entry["type"].as_str().unwrap();
        assert!(
            known.contains(name)
                || name
                    .strip_suffix("Input")
                    .is_some_and(|base| known.contains(base)),
            "unreachable vector type {name}"
        );
        let file = entry["file"].as_str().unwrap();
        assert!(files.insert(file), "duplicate vector {file}");
        assert!(
            std::path::Path::new(file)
                .components()
                .all(|part| matches!(part, std::path::Component::Normal(_))),
            "unsafe vector {file}"
        );
        let value: Value =
            serde_json::from_str(&std::fs::read_to_string(root.join(file)).unwrap()).unwrap();
        let result = match entry["type"].as_str().unwrap() {
            "HandleIdInput" => round_trip::<crate::HandleId>(value.clone()),
            "AiMuxErrorInput" => round_trip::<aimux_core::error::AiMuxError>(value.clone()),
            "ContentPartInput" => round_trip::<aimux_core::result::ContentPart>(value.clone()),
            "LanguageModelUsageInput" => {
                round_trip::<aimux_core::types::LanguageModelUsage>(value.clone())
            }
            "StepResponseInput" => {
                round_trip::<aimux_core::step_result::StepResponse>(value.clone())
            }
            "ToolCallInput" => round_trip::<aimux_core::tool::ToolCall>(value.clone()),
            "null" => round_trip::<()>(value.clone()),
            "string" => round_trip::<String>(value.clone()),
            "AnyValue" => round_trip::<Value>(value.clone()),
            "_uint8_" => round_trip::<Vec<u8>>(value.clone()),
            "AiMuxError" => round_trip::<aimux_core::error::AiMuxError>(value.clone()),
            "AssistantContent" => {
                round_trip::<aimux_core::message::AssistantContent>(value.clone())
            }
            "AssistantContentPart" => {
                round_trip::<aimux_core::content::AssistantContentPart>(value.clone())
            }
            "AudioData" => round_trip::<aimux_core::speech_model::AudioData>(value.clone()),
            "AudioInput" => {
                round_trip::<aimux_core::transcription_model::AudioInput>(value.clone())
            }
            "ContentPart" => round_trip::<aimux_core::result::ContentPart>(value.clone()),
            "DecideOptions" => {
                round_trip::<aimux_core::decision_model::DecideOptions>(value.clone())
            }
            "DecideUsage" => round_trip::<aimux_core::decision_model::DecideUsage>(value.clone()),
            "DecisionBooleanCriteria" => {
                round_trip::<aimux_core::decision_model::DecisionBooleanCriteria>(value.clone())
            }
            "DecisionInput" => {
                round_trip::<aimux_core::decision_model::DecisionInput>(value.clone())
            }
            "DecisionQuestion" => {
                round_trip::<aimux_core::decision_model::DecisionQuestion>(value.clone())
            }
            "DecisionRounding" => {
                round_trip::<aimux_core::decision_model::DecisionRounding>(value.clone())
            }
            "EmbeddingCallOptions" => {
                round_trip::<aimux_core::embedding_model::EmbeddingCallOptions>(value.clone())
            }
            "EmbeddingResponse" => {
                round_trip::<aimux_core::embedding_model::EmbeddingResponse>(value.clone())
            }
            "EmbeddingUsage" => {
                round_trip::<aimux_core::embedding_model::EmbeddingUsage>(value.clone())
            }
            "ErrorEnvelope" => round_trip::<crate::ErrorEnvelope>(value.clone()),
            "FileBytes" => round_trip::<aimux_core::shared::FileBytes>(value.clone()),
            "FileData" => round_trip::<aimux_core::shared::FileData>(value.clone()),
            "FilePartData" => round_trip::<aimux_core::content::FilePartData>(value.clone()),
            "FinishReasonUnified" => {
                round_trip::<aimux_core::types::FinishReasonUnified>(value.clone())
            }
            "GenerateContent" => round_trip::<aimux_core::result::GenerateContent>(value.clone()),
            "GenerateObjectResponse" => {
                round_trip::<aimux_core::generate::GenerateObjectResponse>(value.clone())
            }
            "GenerateObjectResult" => {
                round_trip::<aimux_core::generate::GenerateObjectResult>(value.clone())
            }
            "GenerateTextOptions" => {
                round_trip::<aimux_core::generate::GenerateTextOptions>(value.clone())
            }
            "GeneratedFile" => round_trip::<aimux_core::result::GeneratedFile>(value.clone()),
            "GeneratedFileData" => {
                round_trip::<aimux_core::shared::GeneratedFileData>(value.clone())
            }
            "HandleId" => round_trip::<crate::HandleId>(value.clone()),
            "ImageFile" => round_trip::<aimux_core::image_model::ImageFile>(value.clone()),
            "ImageFileData" => round_trip::<aimux_core::image_model::ImageFileData>(value.clone()),
            "ImageOutputs" => round_trip::<aimux_core::image_model::ImageOutputs>(value.clone()),
            "ImageResponse" => round_trip::<aimux_core::image_model::ImageResponse>(value.clone()),
            "ImageResult" => round_trip::<aimux_core::image_model::ImageResult>(value.clone()),
            "ImageUsage" => round_trip::<aimux_core::image_model::ImageUsage>(value.clone()),
            "InputTokenDetails" => {
                round_trip::<aimux_core::types::InputTokenDetails>(value.clone())
            }
            "LanguageModelFile" => {
                round_trip::<aimux_core::result::LanguageModelFile>(value.clone())
            }
            "LanguageModelUsage" => {
                round_trip::<aimux_core::types::LanguageModelUsage>(value.clone())
            }
            "LanguageModelV4ToolChoice" => {
                round_trip::<aimux_core::tool::LanguageModelV4ToolChoice>(value.clone())
            }
            "ModelCost" => round_trip::<aimux_core::model_catalogue::ModelCost>(value.clone()),
            "ModelLimits" => round_trip::<aimux_core::model_catalogue::ModelLimits>(value.clone()),
            "ModelMessage" => round_trip::<aimux_core::message::ModelMessage>(value.clone()),
            "ModelModalities" => {
                round_trip::<aimux_core::model_catalogue::ModelModalities>(value.clone())
            }
            "ModelPrompt" => round_trip::<aimux_core::message::ModelPrompt>(value.clone()),
            "NamespacedKind" => round_trip::<aimux_core::types::NamespacedKind>(value.clone()),
            "Output" => round_trip::<aimux_core::output::Output>(value.clone()),
            "OutputTokenDetails" => {
                round_trip::<aimux_core::types::OutputTokenDetails>(value.clone())
            }
            "RawToolCall" => round_trip::<aimux_core::tool::RawToolCall>(value.clone()),
            "ReasoningFilePartData" => {
                round_trip::<aimux_core::content::ReasoningFilePartData>(value.clone())
            }
            "ReasoningOutput" => round_trip::<aimux_core::result::ReasoningOutput>(value.clone()),
            "ReasoningPart" => round_trip::<aimux_core::result::ReasoningPart>(value.clone()),
            "RequestInfo" => round_trip::<aimux_core::shared::RequestInfo>(value.clone()),
            "RerankingRank" => {
                round_trip::<aimux_core::reranking_model::RerankingRank>(value.clone())
            }
            "RerankingResponse" => {
                round_trip::<aimux_core::reranking_model::RerankingResponse>(value.clone())
            }
            "RerankingResult" => {
                round_trip::<aimux_core::reranking_model::RerankingResult>(value.clone())
            }
            "ResponseInfo" => round_trip::<aimux_core::shared::ResponseInfo>(value.clone()),
            "RuntimeModel" => {
                round_trip::<aimux_core::model_catalogue::RuntimeModel>(value.clone())
            }
            "SearchResponse" => {
                round_trip::<aimux_core::search_model::SearchResponse>(value.clone())
            }
            "SearchResult" => round_trip::<aimux_core::search_model::SearchResult>(value.clone()),
            "SearchResultItem" => {
                round_trip::<aimux_core::search_model::SearchResultItem>(value.clone())
            }
            "SpeechCallOptions" => {
                round_trip::<aimux_core::speech_model::SpeechCallOptions>(value.clone())
            }
            "SpeechRequest" => round_trip::<aimux_core::speech_model::SpeechRequest>(value.clone()),
            "SpeechResponse" => {
                round_trip::<aimux_core::speech_model::SpeechResponse>(value.clone())
            }
            "StepModel" => round_trip::<aimux_core::step_result::StepModel>(value.clone()),
            "StepResponse" => round_trip::<aimux_core::step_result::StepResponse>(value.clone()),
            "StepResponseMetadata" => {
                round_trip::<aimux_core::step_result::StepResponseMetadata>(value.clone())
            }
            "StreamFrame" => round_trip::<crate::StreamFrame>(value.clone()),
            "TextStreamPart" => {
                round_trip::<aimux_core::stream_part::TextStreamPart>(value.clone())
            }
            "TimeoutConfiguration" => {
                round_trip::<aimux_core::options::TimeoutConfiguration>(value.clone())
            }
            "Timestamp" => round_trip::<aimux_core::timestamp::Timestamp>(value.clone()),
            "Tool" => round_trip::<aimux_core::tool::Tool>(value.clone()),
            "ToolCall" => round_trip::<aimux_core::tool::ToolCall>(value.clone()),
            "ToolChoice" => round_trip::<aimux_core::tool::ToolChoice>(value.clone()),
            "ToolContentPart" => round_trip::<aimux_core::content::ToolContentPart>(value.clone()),
            "ToolResultContentPart" => {
                round_trip::<aimux_core::content::ToolResultContentPart>(value.clone())
            }
            "ToolResultFileId" => {
                round_trip::<aimux_core::content::ToolResultFileId>(value.clone())
            }
            "ToolResultOutput" => {
                round_trip::<aimux_core::content::ToolResultOutput>(value.clone())
            }
            "TranscriptionRequest" => {
                round_trip::<aimux_core::transcription_model::TranscriptionRequest>(value.clone())
            }
            "TranscriptionResponse" => {
                round_trip::<aimux_core::transcription_model::TranscriptionResponse>(value.clone())
            }
            "TranscriptionResult" => {
                round_trip::<aimux_core::transcription_model::TranscriptionResult>(value.clone())
            }
            "TranscriptionSegment" => {
                round_trip::<aimux_core::transcription_model::TranscriptionSegment>(value.clone())
            }
            "TranscriptionStreamPart" => round_trip::<
                aimux_core::transcription_model::TranscriptionStreamPart,
            >(value.clone()),
            "UploadFileData" => {
                round_trip::<aimux_core::files_model::UploadFileData>(value.clone())
            }
            "UploadFileResult" => {
                round_trip::<aimux_core::files_model::UploadFileResult>(value.clone())
            }
            "UserContent" => round_trip::<aimux_core::message::UserContent>(value.clone()),
            "UserContentPart" => round_trip::<aimux_core::content::UserContentPart>(value.clone()),
            "VideoData" => round_trip::<aimux_core::video_model::VideoData>(value.clone()),
            "VideoFile" => round_trip::<aimux_core::video_model::VideoFile>(value.clone()),
            "VideoFileData" => round_trip::<aimux_core::video_model::VideoFileData>(value.clone()),
            "VideoPollOptions" => {
                round_trip::<aimux_core::video_model::VideoPollOptions>(value.clone())
            }
            "VideoResponse" => round_trip::<aimux_core::video_model::VideoResponse>(value.clone()),
            "VideoResult" => round_trip::<aimux_core::video_model::VideoResult>(value.clone()),
            "Warning" => round_trip::<aimux_core::types::Warning>(value.clone()),
            name => panic!("unregistered vector type {name}: {file}"),
        }
        .unwrap_or_else(|error| panic!("{file}: {error}"));
        assert_eq!(result, value, "{file}");
    }
}
