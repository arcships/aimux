//! Golden JSON for the public wire, pinned to the AI SDK's JSON.
//!
//! The public JSON of results, stream parts, prompts and model calls is the
//! AI SDK's JSON: what its types become in JSON (`Uint8Array` as a base64
//! string, `Date` as `toISOString()`, `undefined` omitted, `null` kept). Each
//! test cites the upstream type it pins (`ai@7.0.127`,
//! `@ai-sdk/provider@4.0.21`, `@ai-sdk/provider-utils@5.0.53`).

use aimux_core::content::{
    AssistantContentPart, FilePartData, ReasoningFilePartData, ToolContentPart, UserContentPart,
};
use aimux_core::error::ApiCallError;
use aimux_core::generate::GenerateTextResult;
use aimux_core::image_model::ImageOutputs;
use aimux_core::language_model_message::{
    convert_file_part_data, convert_to_language_model_prompt,
};
use aimux_core::message::{AssistantContent, ModelMessage, ModelPrompt, UserContent};
use aimux_core::result::{
    ContentPart, GenerateContent, GeneratedFile, GeneratedFileOutput, LanguageModelFile,
    ToolApprovalRequestOutput,
};
use aimux_core::shared::{FileBytes, FileData, GeneratedFileData, RequestInfo, ResponseInfo};
use aimux_core::speech_model::AudioData;
use aimux_core::step_result::{StepModel, StepResponse, StepResult, StepResultPerformance};
use aimux_core::stream_part::{StreamPart, TextStreamPart};
use aimux_core::timestamp::Timestamp;
use aimux_core::tool::ToolCall;
use aimux_core::types::{
    FinishReasonUnified, LanguageModelUsage, ProviderMetadata, ResponseMetadata,
};
use aimux_core::video_model::VideoData;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

/// Serialize `value`, compare with `wire`, and parse `wire` back.
fn assert_json<T: Serialize + DeserializeOwned>(value: &T, wire: Value) -> T {
    assert_eq!(serde_json::to_value(value).unwrap(), wire);
    serde_json::from_value(wire).unwrap()
}

// ── Bytes (#232 C33) ────────────────────────────────────────────────────────

/// `ImagePart.image` and `FilePart.data` are `DataContent`
/// (`provider-utils` `types/content-part.ts:42`, `:75`; `data-content.ts:4`):
/// bytes are a base64 string in the JSON (`uint8-utils.ts:12-23`), never an
/// array of numbers.
#[test]
fn prompt_bytes_are_base64_strings() {
    let image = UserContentPart::image(vec![137, 80, 78, 71], "image/png");
    let back = assert_json(
        &image,
        json!({ "type": "image", "image": "iVBORw==", "mediaType": "image/png" }),
    );
    assert_eq!(back, image);

    let file = UserContentPart::file(vec![0, 1, 2, 250, 255], "application/pdf");
    let back = assert_json(
        &file,
        json!({
            "type": "file",
            "data": { "type": "data", "data": "AAEC+v8=" },
            "mediaType": "application/pdf",
        }),
    );
    assert_eq!(back, file);

    // An array of numbers is not the AI SDK's JSON for bytes.
    assert!(
        serde_json::from_value::<UserContentPart>(
            json!({ "type": "image", "image": [1, 2], "mediaType": "image/png" })
        )
        .is_err()
    );
}

/// `SharedV4FileDataData.data` is `Uint8Array | string`
/// (`provider` `shared/v4/shared-v4-file-data.ts:7-10`): one base64 string in
/// the JSON. Bytes and the same bytes in base64 are equal.
#[test]
fn file_bytes_are_one_base64_string() {
    let binary = GeneratedFileData::Data {
        data: FileBytes::Binary(b"hi".to_vec()),
    };
    let back = assert_json(&binary, json!({ "type": "data", "data": "aGk=" }));
    assert_eq!(
        back,
        GeneratedFileData::Data {
            data: FileBytes::Base64("aGk=".into())
        }
    );
    assert_eq!(back, binary);

    // A base64 string is passed through verbatim, as upstream passes it on.
    let text = FileBytes::Base64("aGk".into());
    assert_eq!(serde_json::to_value(&text).unwrap(), json!("aGk"));
    assert_eq!(text, FileBytes::Binary(b"hi".to_vec()));
    assert!(serde_json::from_value::<FileBytes>(json!([104, 105])).is_err());
}

/// Model results: `SpeechModelV4Result.audio: string | Uint8Array`
/// (`speech-model/v4/speech-model-v4-result.ts:16`), `ImageModelV4Result.images:
/// Array<string> | Array<Uint8Array>` (`image-model-v4-result.ts:23`) and the
/// video `{ type: 'binary', data: Uint8Array }` (`video-model-v4-result.ts:28-29`)
/// are base64 strings in the JSON.
#[test]
fn model_result_bytes_are_base64_strings() {
    assert_eq!(
        serde_json::to_value(AudioData::Binary(vec![1, 2, 3])).unwrap(),
        json!("AQID")
    );
    assert!(matches!(
        serde_json::from_value(json!("AQID")).unwrap(),
        AudioData::Base64(text) if text == "AQID"
    ));

    assert_eq!(
        serde_json::to_value(ImageOutputs::Binary(vec![vec![1], vec![2, 3]])).unwrap(),
        json!(["AQ==", "AgM="])
    );
    assert!(matches!(
        serde_json::from_value(json!(["AQ=="])).unwrap(),
        ImageOutputs::Base64(images) if images == ["AQ=="]
    ));

    let video = VideoData::Binary {
        data: vec![0, 0, 0, 24],
        media_type: "video/mp4".into(),
    };
    assert_eq!(
        serde_json::to_value(&video).unwrap(),
        json!({ "type": "binary", "data": "AAAAGA==", "mediaType": "video/mp4" })
    );
    assert!(matches!(
        serde_json::from_value(json!({ "type": "binary", "data": "AAAAGA==", "mediaType": "video/mp4" })).unwrap(),
        VideoData::Binary { data, .. } if data == [0, 0, 0, 24]
    ));
}

// ── null and undefined (#232 C34) ───────────────────────────────────────────

/// `unknown` fields keep `null` apart from a missing value, as the AI SDK's
/// JSON does (`undefined` is omitted, `null` is kept): response and request
/// `body?: unknown` (`provider` `language-model-v4-generate-result.ts:41, :56`),
/// `APICallError.data?: unknown` (`errors/api-call-error.ts:18`) and
/// `ToolApprovalRequest.inputSchemaInput?: unknown` (`provider-utils`
/// `types/tool-approval-request.ts:41`).
#[test]
fn unknown_fields_keep_null_apart_from_missing() {
    for wire in [
        json!({}),
        json!({ "body": null }),
        json!({ "body": { "a": null } }),
    ] {
        let response: ResponseInfo = assert_json_back(wire.clone());
        assert_eq!(response.body.is_some(), wire.get("body").is_some());
        let _: RequestInfo = assert_json_back(wire);
    }

    let mut error = ApiCallError::new("bad", "https://example.com", json!({}));
    error.data = Some(serde_json::Value::Null);
    let wire = serde_json::to_value(&error).unwrap();
    assert_eq!(wire["data"], serde_json::Value::Null);
    let back: ApiCallError = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(back.data, Some(serde_json::Value::Null));
    let mut missing = wire;
    missing.as_object_mut().unwrap().remove("data");
    let back: ApiCallError = serde_json::from_value(missing).unwrap();
    assert_eq!(back.data, None);

    let approval = json!({
        "type": "tool-approval-request",
        "approvalId": "a1",
        "toolCallId": "c1",
        "inputSchemaInput": null,
    });
    let part: AssistantContentPart = assert_json_back(approval);
    assert!(matches!(
        part,
        AssistantContentPart::ToolApprovalRequest {
            input_schema_input: Some(serde_json::Value::Null),
            ..
        }
    ));
}

// ── File parts (#232 C35) ───────────────────────────────────────────────────

/// `FilePart` (`provider-utils` `types/content-part.ts:62-102`) has one tag,
/// `file`; its `data` is a tagged `FileData` (`types/file-data.ts:44-48`) or a
/// bare `DataContent` (base64), `URL` or `ProviderReference`.
/// `ReasoningFilePart.data` (`:164`) is tagged data or a URL, or a bare string.
/// There is no `file-base64`, `file-url` or `file-reference` part.
#[test]
fn file_parts_are_the_ai_sdk_file_part() {
    for wire in [
        json!({ "type": "file", "data": { "type": "data", "data": "aGk=" }, "mediaType": "text/plain", "filename": "a.txt" }),
        json!({ "type": "file", "data": { "type": "url", "url": "https://example.com/a.pdf" }, "mediaType": "application/pdf" }),
        json!({ "type": "file", "data": { "type": "reference", "reference": { "openai": "file-abc" } }, "mediaType": "application/pdf" }),
        json!({ "type": "file", "data": { "type": "text", "text": "hello" }, "mediaType": "text/plain" }),
        json!({ "type": "file", "data": "aGk=", "mediaType": "text/plain" }),
        json!({ "type": "file", "data": "https://example.com/a.pdf", "mediaType": "application/pdf" }),
        json!({ "type": "file", "data": { "openai": "file-abc" }, "mediaType": "application/pdf" }),
    ] {
        let _: UserContentPart = assert_json_back(wire.clone());
        let _: AssistantContentPart = assert_json_back(wire);
    }
    for wire in [
        json!({ "type": "reasoning-file", "data": "aGk=", "mediaType": "image/png" }),
        json!({ "type": "reasoning-file", "data": { "type": "url", "url": "https://example.com/a.png" }, "mediaType": "image/png" }),
    ] {
        let _: AssistantContentPart = assert_json_back(wire);
    }
    for removed in ["file-base64", "file-url", "file-reference"] {
        let wire = json!({
            "type": removed,
            "data": "aGk=",
            "url": "https://example.com/a.pdf",
            "reference": { "openai": "file-abc" },
            "mediaType": "application/pdf",
        });
        assert!(
            serde_json::from_value::<UserContentPart>(wire.clone()).is_err(),
            "{removed}"
        );
        assert!(
            serde_json::from_value::<AssistantContentPart>(wire).is_err(),
            "{removed}"
        );
    }

    // The constructors write the tagged form the AI SDK documents.
    assert_eq!(
        serde_json::to_value(UserContentPart::file_base64("aGk=", "text/plain")).unwrap(),
        json!({ "type": "file", "data": { "type": "data", "data": "aGk=" }, "mediaType": "text/plain" })
    );
    assert_eq!(
        serde_json::to_value(UserContentPart::file_url(
            "https://example.com/a.pdf",
            "application/pdf"
        ))
        .unwrap(),
        json!({ "type": "file", "data": { "type": "url", "url": "https://example.com/a.pdf" }, "mediaType": "application/pdf" })
    );
    let reference = [("openai".to_owned(), "file-abc".to_owned())]
        .into_iter()
        .collect();
    assert_eq!(
        serde_json::to_value(UserContentPart::file_reference(
            "application/pdf",
            reference
        ))
        .unwrap(),
        json!({ "type": "file", "data": { "type": "reference", "reference": { "openai": "file-abc" } }, "mediaType": "application/pdf" })
    );
}

/// `convertToLanguageModelV4FilePart` (`ai` `prompt/file-part-data.ts:89-141`)
/// and the media type rule `mediaType ?? part.mediaType`
/// (`convert-to-language-model-prompt.ts:294-303`).
#[test]
fn file_part_data_converts_as_the_ai_sdk_does() {
    let convert = |wire: Value| {
        convert_file_part_data(&serde_json::from_value::<FilePartData>(wire).unwrap())
    };
    let data = |text: &str| FileData::Data {
        data: FileBytes::Base64(text.into()),
    };

    // A string that is not a URL is base64 (`:125-130`).
    assert_eq!(convert(json!("aGk=")).unwrap(), (data("aGk="), None));
    // A URL string is a URL; parsing changes it, so the text is kept as
    // `originalUrl` (`:50-58`).
    assert_eq!(
        convert(json!("https://example.com/a.pdf")).unwrap(),
        (
            FileData::Url {
                url: "https://example.com/a.pdf".into(),
                original_url: None
            },
            None
        )
    );
    assert_eq!(
        convert(json!("HTTPS://Example.com/a b.pdf")).unwrap(),
        (
            FileData::Url {
                url: "https://example.com/a%20b.pdf".into(),
                original_url: Some("HTTPS://Example.com/a b.pdf".into())
            },
            None
        )
    );
    // A `data:` URL, bare or tagged, is inline data with its media type
    // (`:31-41`, `split-data-url.ts`).
    for wire in [
        json!("data:image/png;base64,iVBORw=="),
        json!({ "type": "url", "url": "data:image/png;base64,iVBORw==" }),
    ] {
        assert_eq!(
            convert(wire).unwrap(),
            (data("iVBORw=="), Some("image/png".into()))
        );
    }
    assert!(convert(json!("data:image/png;base64")).is_err());
    // A `data:` URL is not inline data (`:94-104`).
    assert!(convert(json!({ "type": "data", "data": "data:image/png;base64,iVBORw==" })).is_err());
    // A bare provider reference is a reference (`:133-138`).
    assert_eq!(
        convert(json!({ "openai": "file-abc" })).unwrap(),
        (
            FileData::Reference {
                reference: [("openai".to_owned(), "file-abc".to_owned())]
                    .into_iter()
                    .collect()
            },
            None
        )
    );

    // In the prompt, the data URL's media type wins over the part's.
    let prompt = convert_to_language_model_prompt(
        &[ModelMessage::user_parts(vec![UserContentPart::File {
            data: FilePartData::String("data:image/png;base64,iVBORw==".into()),
            media_type: "image".into(),
            filename: None,
            provider_options: None,
        }])],
        None,
        None,
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(&prompt).unwrap(),
        json!([{ "role": "user", "content": [
            { "type": "file", "data": { "type": "data", "data": "iVBORw==" }, "mediaType": "image/png" }
        ] }])
    );
}

// ── Generated files (#232 C46) ──────────────────────────────────────────────

/// `GeneratedFile` (`ai` `generate-text/generated-file.ts:10-32`) is
/// `{ base64, mediaType, providerMetadata? }` in the JSON, never tagged
/// `{ type: 'data' | 'url' }` data. A text-generation file part is
/// `{ type: 'file' | 'reasoning-file', file, providerMetadata? }` with the
/// provider metadata on the part (`convert-language-model-content.ts:55-70`,
/// `stream-language-model-call.ts:569-590`). The provider's
/// `LanguageModelV4File` keeps `{ type: 'file', mediaType, data }`
/// (`provider` `language-model-v4-file.ts`).
#[test]
fn generated_files_are_base64_and_media_type() {
    let file = GeneratedFile {
        data: FileBytes::Binary(b"hi".to_vec()),
        media_type: "text/plain".into(),
        provider_metadata: None,
    };
    let back = assert_json(
        &file,
        json!({ "base64": "aGk=", "mediaType": "text/plain" }),
    );
    assert_eq!(back, file);
    assert_eq!(back.base64(), "aGk=");
    assert_eq!(back.bytes().unwrap().as_ref(), b"hi");
    assert!(
        serde_json::from_value::<GeneratedFile>(
            json!({ "data": { "type": "data", "data": "aGk=" }, "mediaType": "text/plain" })
        )
        .is_err()
    );

    let metadata: ProviderMetadata =
        serde_json::from_value(json!({ "google": { "thoughtSignature": "sig" } })).unwrap();
    let output = GeneratedFileOutput {
        file: file.clone(),
        provider_metadata: Some(metadata),
    };
    let wire = |tag: &str| {
        json!({
            "type": tag,
            "file": { "base64": "aGk=", "mediaType": "text/plain" },
            "providerMetadata": { "google": { "thoughtSignature": "sig" } },
        })
    };
    assert_json(
        &ContentPart::File {
            file: output.file.clone(),
            provider_metadata: output.provider_metadata.clone(),
        },
        wire("file"),
    );
    assert_json(
        &ContentPart::ReasoningFile(output.clone()),
        wire("reasoning-file"),
    );
    assert_json(&TextStreamPart::File(output.clone()), wire("file"));
    assert_json(
        &TextStreamPart::ReasoningFile(output),
        wire("reasoning-file"),
    );

    let raw: GenerateContent = GenerateContent::File(LanguageModelFile {
        data: GeneratedFileData::Url {
            url: "https://example.com/a.png".into(),
            original_url: None,
        },
        media_type: "image/png".into(),
        provider_metadata: None,
    });
    assert_json(
        &raw,
        json!({
            "type": "file",
            "data": { "type": "url", "url": "https://example.com/a.png" },
            "mediaType": "image/png",
        }),
    );
}

// ── Timestamps (#232 C48) ────────────────────────────────────────────────────

/// Response timestamps are a `Date` (`provider` `language-model-v4-response-metadata.ts:10`,
/// `ai` `types/language-model-response-metadata.ts:22`, and the image,
/// speech, transcription, video and reranking metadata): `toISOString()` in
/// the JSON, UTC with milliseconds. Providers build them as
/// `new Date(created * 1000)` (`openai` `responses/openai-responses-language-model.ts:1509`)
/// or from the `Date` header (`amazon-bedrock` `amazon-bedrock-chat-language-model.ts:864-867`).
#[test]
fn timestamps_are_iso_strings() {
    let created = Timestamp::from_unix_seconds(1_741_269_019.0).unwrap();
    let response = ResponseInfo {
        id: Some("resp_1".into()),
        timestamp: Some(created),
        model_id: Some("gpt-4o".into()),
        ..ResponseInfo::default()
    };
    assert_json(
        &response,
        json!({ "id": "resp_1", "timestamp": "2025-03-06T13:50:19.000Z", "modelId": "gpt-4o" }),
    );
    let metadata = ResponseMetadata {
        id: None,
        timestamp: Timestamp::from_http_date("Thu, 06 Mar 2025 13:50:19 GMT"),
        model_id: None,
    };
    assert_json(
        &metadata,
        json!({ "timestamp": "2025-03-06T13:50:19.000Z" }),
    );

    // Reading is `new Date(text).toISOString()` for an RFC 3339 date-time.
    let read: ResponseInfo =
        serde_json::from_value(json!({ "timestamp": "2025-03-06T21:50:19.5+08:00" })).unwrap();
    assert_eq!(
        serde_json::to_value(&read).unwrap(),
        json!({ "timestamp": "2025-03-06T13:50:19.500Z" })
    );
    for invalid in [
        json!("1741269019"),
        json!("2025-03-06"),
        json!("2025-13-06T13:50:19Z"),
        json!("Thu, 06 Mar 2025 13:50:19 GMT"),
        json!(1_741_269_019),
    ] {
        assert!(
            serde_json::from_value::<ResponseInfo>(json!({ "timestamp": invalid })).is_err(),
            "{invalid} was read as a timestamp"
        );
    }
}

// ── Text stream deltas (#229 B9) ─────────────────────────────────────────────

/// The text stream's `text-delta` and `reasoning-delta` carry `text`
/// (`ai` `generate-text/stream-text-result.ts:435-440`, `:466-471`), copied
/// from the V4 part's `delta` (`stream-language-model-call.ts:496-501`); a
/// `tool-input-delta` keeps `delta` (`stream-text-result.ts:496-501`), and the
/// provider-level `StreamPart` keeps V4's `delta`.
#[test]
fn text_stream_deltas_are_text() {
    let provider = |tag: &str,
                    delta: &str|
     -> StreamPart<ToolCall, ToolApprovalRequestOutput, GeneratedFileOutput> {
        serde_json::from_value(json!({ "type": tag, "id": "t1", "delta": delta })).unwrap()
    };
    let text = provider("text-delta", "Hello")
        .into_text_stream_part()
        .unwrap();
    assert_json(
        &text,
        json!({ "type": "text-delta", "id": "t1", "text": "Hello" }),
    );
    let reasoning = provider("reasoning-delta", "Hmm")
        .into_text_stream_part()
        .unwrap();
    assert_json(
        &reasoning,
        json!({ "type": "reasoning-delta", "id": "t1", "text": "Hmm" }),
    );
    let tool_input = provider("tool-input-delta", "{\"a\"")
        .into_text_stream_part()
        .unwrap();
    assert_json(
        &tool_input,
        json!({ "type": "tool-input-delta", "id": "t1", "delta": "{\"a\"" }),
    );
    assert!(
        serde_json::from_value::<TextStreamPart>(
            json!({ "type": "text-delta", "id": "t1", "delta": "Hello" })
        )
        .is_err()
    );
}

// ── Standalone type tags (#229 NP2-7) ───────────────────────────────────────

/// Outside `content`, a tool call is the AI SDK's `TypedToolCall`, tagged
/// `{type: 'tool-call', ...}` (`generate-text/tool-call.ts:8-14`), and a
/// source is its `Source`, a `LanguageModelV4Source` tagged
/// `{type: 'source', sourceType, ...}` (`types/language-model.ts:92`;
/// provider `language-model-v4-source.ts:8`, `:36`). These are the text
/// results' `toolCalls` and `sources` (`generate-text-result.ts:73`, `:78`;
/// `stream-text-result.ts:173`, `:180`) and an approval request's `toolCall`
/// (`tool-approval-request-output.ts:20`).
#[test]
fn standalone_tool_calls_and_sources_are_tagged() {
    let call = json!({
        "type": "tool-call",
        "toolCallId": "call-1",
        "toolName": "weather",
        "input": { "city": "SG" },
        "providerMetadata": { "openai": { "itemId": "fc_1" } },
    });
    let url = json!({
        "type": "source",
        "sourceType": "url",
        "id": "s1",
        "url": "https://example.com",
        "title": "Example",
    });
    let document = json!({
        "type": "source",
        "sourceType": "document",
        "id": "s2",
        "mediaType": "application/pdf",
        "title": "Doc",
    });
    // A one-step result whose content holds the call and the sources.
    let step = StepResult {
        call_id: "call".into(),
        step_number: 0,
        model: StepModel {
            provider: "test".into(),
            model_id: "model".into(),
        },
        content: serde_json::from_value(json!([call, url, document])).unwrap(),
        finish_reason: FinishReasonUnified::ToolCalls,
        raw_finish_reason: None,
        usage: LanguageModelUsage::default(),
        performance: StepResultPerformance::default(),
        warnings: Vec::new(),
        request: RequestInfo::default(),
        response: StepResponse {
            id: "response".into(),
            timestamp: Timestamp::from_unix_millis(0).unwrap(),
            model_id: "model".into(),
            headers: None,
            body: None,
            messages: Vec::new(),
        },
        provider_metadata: None,
    };
    let generated: GenerateTextResult = serde_json::from_value(json!({
        "steps": [step],
        "totalUsage": LanguageModelUsage::default(),
    }))
    .unwrap();
    let written = serde_json::to_value(&generated).unwrap();
    assert_eq!(written["toolCalls"], json!([call]));
    assert_eq!(written["sources"], json!([url, document]));
    // The tag comes first, as in the AI SDK's objects.
    let text = serde_json::to_string(&generated).unwrap();
    assert!(
        text.contains(r#""toolCalls":[{"type":"tool-call","toolCallId":"call-1""#),
        "{text}"
    );
    assert!(
        text.contains(r#""sources":[{"type":"source","sourceType":"url","id":"s1""#),
        "{text}"
    );

    // An approval request's call is tagged, standalone and in `content`.
    assert_json_back::<ToolApprovalRequestOutput>(json!({ "approvalId": "a1", "toolCall": call }));
    assert_json_back::<ContentPart>(json!({
        "type": "tool-approval-request",
        "approvalId": "a1",
        "toolCall": call,
    }));
    // In `content` the union writes a tool call's tag, once.
    let part = assert_json_back::<ContentPart>(call.clone());
    assert_eq!(
        serde_json::to_string(&part)
            .unwrap()
            .matches(r#""type""#)
            .count(),
        1
    );

    // JSON written before the tag still reads, and gains it; another tag is
    // rejected.
    let untagged = json!({ "toolCallId": "call-1", "toolName": "weather", "input": {} });
    let read: ToolApprovalRequestOutput =
        serde_json::from_value(json!({ "approvalId": "a1", "toolCall": untagged })).unwrap();
    assert_eq!(
        serde_json::to_value(&read).unwrap()["toolCall"]["type"],
        "tool-call"
    );
    let mistagged = json!({ "type": "source", "toolCallId": "c", "toolName": "t", "input": {} });
    assert!(
        serde_json::from_value::<ToolApprovalRequestOutput>(
            json!({ "approvalId": "a1", "toolCall": mistagged })
        )
        .is_err()
    );
}

/// A step is what `JSON.stringify` writes for the AI SDK's `DefaultStepResult`
/// (`generate-text/step-result.ts:138-278`): its own fields camelCase, `model`
/// `{provider, modelId}` (`:155-161`), the performance (`:62-133`) and the
/// response `{id, timestamp, modelId, headers?, body?, messages}`
/// (`types/language-model-response-metadata.ts:6-38`), an `undefined` field
/// absent. The usage is `LanguageModelUsage` (`types/usage.ts:10-67`): flat
/// camelCase counts, an unknown count absent, the detail objects present.
/// A result's `toolResults` are tagged `TypedToolResult`s
/// (`generate-text-result.ts:93`, `tool-result.ts:10-42`); tool errors
/// (`tool-error.ts:6-36`), denied outputs (`tool-output-denied.ts:7-15`) and
/// approval responses, whose `toolCall` is a tagged `TypedToolCall`
/// (`tool-approval-response-output.ts:7-35`), are camelCase.
#[test]
fn steps_usage_and_tool_outputs_are_the_ai_sdk_json() {
    let call = json!({ "type": "tool-call", "toolCallId": "c1", "toolName": "t", "input": {} });
    let result = json!({
        "type": "tool-result",
        "toolCallId": "c1",
        "toolName": "t",
        "input": {},
        "output": "ok",
        "providerExecuted": true,
    });
    let step_wire = json!({
        "callId": "call",
        "stepNumber": 0,
        "model": { "provider": "test", "modelId": "model" },
        "content": [call, result],
        "finishReason": "tool-calls",
        "usage": {
            "inputTokens": 3,
            "inputTokenDetails": {},
            "outputTokens": 2,
            "outputTokenDetails": { "reasoningTokens": 1 },
            "totalTokens": 5,
        },
        "performance": {
            "effectiveOutputTokensPerSecond": 20.0,
            "effectiveTotalTokensPerSecond": 50.0,
            "stepTimeMs": 120.0,
            "responseTimeMs": 100.0,
            "toolExecutionMs": {},
        },
        "warnings": [],
        "request": {},
        "response": {
            "id": "r1",
            "timestamp": "1970-01-01T00:00:00.000Z",
            "modelId": "model",
            "body": null,
            "messages": [],
        },
    });
    let step: StepResult = assert_json_back(step_wire.clone());
    assert_eq!(step.response.body, Some(Value::Null));
    assert_eq!(step.usage.input_token_details.cache_read_tokens, None);

    let generated: GenerateTextResult = serde_json::from_value(json!({
        "steps": [step_wire],
        "totalUsage": LanguageModelUsage::default(),
    }))
    .unwrap();
    let written = serde_json::to_value(&generated).unwrap();
    assert_eq!(written["toolResults"], json!([result]));
    assert_eq!(
        written["totalUsage"],
        json!({ "inputTokenDetails": {}, "outputTokenDetails": {} })
    );
    assert_eq!(written["response"]["modelId"], "model");
    assert!(written.get("rawFinishReason").is_none());

    assert_json_back::<ContentPart>(json!({
        "type": "tool-error",
        "toolCallId": "c2",
        "toolName": "t",
        "input": { "q": 1 },
        "error": "boom",
        "dynamic": true,
    }));
    assert_json_back::<TextStreamPart>(json!({
        "type": "tool-output-denied",
        "toolCallId": "c3",
        "toolName": "t",
    }));
    let response = json!({
        "type": "tool-approval-response",
        "approvalId": "a1",
        "toolCall": call,
        "approved": false,
        "reason": "no",
    });
    assert_json_back::<ContentPart>(response.clone());
    assert_json_back::<TextStreamPart>(response);
}

/// `Output` has no JSON upstream (it carries functions); aimux's JSON is
/// tagged by `type` with the output kind, upstream's mode `name`
/// (`generate-text/output.ts:68`, `:114`, `:241`, `:520`, `:657`), and the
/// `Output.array()` arguments are camelCase (`:200-215`).
#[test]
fn output_specifications_are_tagged() {
    use aimux_core::output::Output;
    assert_json_back::<Output>(json!({ "type": "text" }));
    assert_json_back::<Output>(json!({ "type": "object", "schema": {}, "name": "person" }));
    let array: Output = assert_json_back(json!({
        "type": "array",
        "element": { "type": "string" },
        "minItems": 1,
        "maxItems": 3,
    }));
    assert!(matches!(
        array,
        Output::Array {
            min_items: Some(1),
            max_items: Some(3),
            ..
        }
    ));
    assert_json_back::<Output>(json!({ "type": "choice", "options": ["a", "b"] }));
    assert_json_back::<Output>(json!({ "type": "json", "description": "any" }));
    assert!(serde_json::from_value::<Output>(json!({ "Object": { "schema": {} } })).is_err());
}

/// Decisions are the AI SDK's JSON. The model's call options
/// (`@ai-sdk/provider@4.0.22 decision-model/v4/decision-model-v4-call-options.ts:7-14`)
/// and result (`decision-model-v4-result.ts:31-56`: `rounding
/// {probabilityDecimals, scoreDecimals}`, `usage {inputTokens,
/// outputTokens}`, `providerMetadata`, `response {id, timestamp, modelId,
/// headers, body}`), and `decide`'s options (`ai@7.0.128
/// src/decide/decide.ts:53-61`) and result (`decision-result.ts:22-39`,
/// whose unknown usage counts are `undefined`, so absent) are camelCase.
#[test]
fn decisions_are_the_ai_sdk_json() {
    use aimux_core::decision_model::{
        DecideOptions, DecideResult, DecideUsage, DecisionCallOptions, DecisionResult,
    };
    let questions = json!({
        "urgent": { "type": "boolean", "instructions": "Urgent?", "criteria": { "true": null } },
        "team": { "type": "choice", "instructions": "Team?", "criteria": { "a": "A", "b": null } },
    });
    assert_json_back::<DecisionCallOptions>(json!({
        "state": "text",
        "questions": questions,
        "headers": { "x-a": "1" },
        "providerOptions": { "openai": { "safetyIdentifier": "u" } },
    }));
    let options: DecideOptions = assert_json_back(json!({
        "state": { "ticket": 1 },
        "questions": questions,
        "maxRetries": 0,
        "timeout": { "totalMs": 1000 },
        "providerOptions": { "jev": {} },
    }));
    assert_eq!(options.max_retries, Some(0));

    let answers = json!({
        "urgent": { "type": "boolean", "probability": 0.75 },
        "team": { "type": "choice", "choice": "a", "probabilities": { "a": 0.5, "b": 0.5 } },
    });
    let response = json!({
        "id": "r1",
        "timestamp": "2026-10-06T13:24:17.375Z",
        "modelId": "m",
        "headers": { "content-type": "application/json" },
        "body": null,
    });
    assert_json_back::<DecisionResult>(json!({
        "answers": answers,
        "rounding": { "probabilityDecimals": 2, "scoreDecimals": 1 },
        "usage": { "inputTokens": 3 },
        "warnings": [],
        "providerMetadata": { "jev": { "probabilitySource": "native" } },
        "response": response,
    }));
    assert_json_back::<DecideResult>(json!({
        "answers": answers,
        "usage": { "inputTokens": 3, "outputTokens": 2, "totalTokens": 5 },
        "warnings": [],
        "rounding": { "probabilityDecimals": 2 },
        "providerMetadata": { "jev": {} },
        "response": response,
    }));
    assert_eq!(
        serde_json::to_value(DecideUsage {
            input_tokens: Some(3),
            ..Default::default()
        })
        .unwrap(),
        json!({ "inputTokens": 3 })
    );
}

// ── Unknown types ───────────────────────────────────────────────────────────

/// Every tagged union of the public JSON rejects an unknown `type` (or
/// `role`) with an error that names it, never as empty content (RFC-0042
/// §7.2, `rfc/0042-v4-type-boundaries.md:211`).
#[test]
fn an_unknown_type_is_a_decode_error_naming_it() {
    use aimux_core::output::Output;
    let unknown = json!({ "type": "no-such-type" });
    for error in [
        decode_error::<TextStreamPart>(unknown.clone()),
        decode_error::<StreamPart>(unknown.clone()),
        decode_error::<ContentPart>(unknown.clone()),
        decode_error::<GenerateContent>(unknown.clone()),
        decode_error::<UserContentPart>(unknown.clone()),
        decode_error::<AssistantContentPart>(unknown.clone()),
        decode_error::<ToolContentPart>(unknown.clone()),
        decode_error::<FileData>(unknown.clone()),
        decode_error::<GeneratedFileData>(unknown.clone()),
        decode_error::<Output>(unknown),
    ] {
        assert!(error.contains("unknown variant `no-such-type`"), "{error}");
    }
    let error = decode_error::<ModelMessage>(json!({ "role": "robot", "content": "hi" }));
    assert!(error.contains("unknown variant `robot`"), "{error}");
}

/// The wrappers around those unions keep the error. A prompt or message
/// content with an unknown part names the part's `type`, and file `data`
/// with a `type` is tagged data, never a provider reference (`ai@7.0.127`
/// `src/prompt/file-part-data.ts:14-21` `isTaggedFileData`;
/// `@ai-sdk/provider-utils@5.0.55` `src/is-provider-reference.ts:9-21`,
/// `!('type' in data)`).
#[test]
fn untagged_wrappers_keep_the_unknown_type_error() {
    let part = json!({ "type": "no-such-part", "text": "hi" });
    for error in [
        decode_error::<ModelPrompt>(json!([{ "role": "user", "content": [part.clone()] }])),
        decode_error::<ModelPrompt>(json!([{ "role": "assistant", "content": [part.clone()] }])),
        decode_error::<UserContent>(json!([part.clone()])),
        decode_error::<AssistantContent>(json!([part])),
    ] {
        assert!(error.contains("unknown variant `no-such-part`"), "{error}");
    }

    for (data, tag) in [
        (
            json!({ "type": "URL", "url": "https://example.com/a.pdf" }),
            "URL",
        ),
        (json!({ "type": "bogus" }), "bogus"),
    ] {
        let error = decode_error::<FilePartData>(data.clone());
        assert!(
            error.contains(&format!("unknown variant `{tag}`")),
            "{error}"
        );
        // The same inside a prompt.
        let error = decode_error::<ModelPrompt>(json!([{ "role": "user", "content": [
            { "type": "file", "data": data, "mediaType": "application/pdf" }
        ] }]));
        assert!(
            error.contains(&format!("unknown variant `{tag}`")),
            "{error}"
        );
    }
    let error = decode_error::<ReasoningFilePartData>(
        json!({ "type": "reference", "reference": { "openai": "file-abc" } }),
    );
    assert!(error.contains("unknown variant `reference`"), "{error}");

    // A bare `{ provider: id }` map is still a reference and a string still a
    // string (`file_part_data_converts_as_the_ai_sdk_does` covers the rest).
    assert!(matches!(
        serde_json::from_value::<FilePartData>(json!({ "openai": "file-abc" })).unwrap(),
        FilePartData::Reference(_)
    ));
    assert!(matches!(
        serde_json::from_value::<FilePartData>(json!("aGk=")).unwrap(),
        FilePartData::String(_)
    ));
    // Neither a string nor an array says what was expected.
    let error = decode_error::<ModelPrompt>(json!(42));
    assert!(
        error.contains("expected a prompt string or an array of messages"),
        "{error}"
    );
}

// ── Request configuration ───────────────────────────────────────────────────

/// The message of a value that must not decode.
fn decode_error<T: DeserializeOwned>(wire: Value) -> String {
    match serde_json::from_value::<T>(wire) {
        Ok(_) => panic!("decoded a value that must not decode"),
        Err(error) => error.to_string(),
    }
}

/// Request configuration rejects unknown keys (RFC-0042 §7.2,
/// `rfc/0042-v4-type-boundaries.md:212`): aimux 0.5's snake_case keys and
/// typos fail naming the key instead of falling back to the default.
#[test]
fn request_options_reject_unknown_keys() {
    use aimux_core::decision_model::DecideOptions;
    use aimux_core::embedding_model::EmbeddingCallOptions;
    use aimux_core::files_model::UploadFileCallOptions;
    use aimux_core::generate::GenerateTextOptions;
    use aimux_core::image_model::ImageCallOptions;
    use aimux_core::moa::MoaConfig;
    use aimux_core::options::{CallOptions, TimeoutConfiguration};
    use aimux_core::reranking_model::RerankingCallOptions;
    use aimux_core::search_model::SearchCallOptions;
    use aimux_core::speech_model::SpeechCallOptions;
    use aimux_core::transcription_model::TranscriptionCallOptions;
    use aimux_core::video_model::VideoCallOptions;

    let options: GenerateTextOptions = serde_json::from_value(json!({
        "maxRetries": 0,
        "timeout": { "totalMs": 10000 },
        "maxOutputTokens": 5,
    }))
    .unwrap();
    assert_eq!(options.max_retries, Some(0));
    assert_eq!(options.timeout.and_then(|t| t.total_ms), Some(10000));
    assert_eq!(options.max_output_tokens, Some(5));
    for (wire, key) in [
        (json!({ "max_retries": 0 }), "max_retries"),
        (json!({ "timeout": { "total_ms": 10000 } }), "total_ms"),
        (json!({ "maxRetries": 0, "temprature": 0.2 }), "temprature"),
    ] {
        let error = decode_error::<GenerateTextOptions>(wire);
        assert!(error.contains(&format!("unknown field `{key}`")), "{error}");
    }

    // The other operations' options, the nested settings objects and the MoA
    // config. The unknown key is reported before any missing field.
    for (error, key) in [
        (
            decode_error::<TimeoutConfiguration>(json!({ "first_chunk_ms": 1 })),
            "first_chunk_ms",
        ),
        (
            decode_error::<EmbeddingCallOptions>(json!({ "max_retries": 0 })),
            "max_retries",
        ),
        (
            decode_error::<SpeechCallOptions>(json!({ "output_format": "mp3" })),
            "output_format",
        ),
        (
            decode_error::<ImageCallOptions>(json!({ "aspect_ratio": "1:1" })),
            "aspect_ratio",
        ),
        (
            decode_error::<TranscriptionCallOptions>(json!({ "media_type": "audio/wav" })),
            "media_type",
        ),
        (
            decode_error::<RerankingCallOptions>(json!({ "top_n": 2 })),
            "top_n",
        ),
        (
            decode_error::<VideoCallOptions>(json!({ "poll": { "interval_ms": 10 } })),
            "interval_ms",
        ),
        (
            decode_error::<SearchCallOptions>(json!({ "max_results": 5 })),
            "max_results",
        ),
        (
            decode_error::<UploadFileCallOptions>(json!({ "media_type": "text/plain" })),
            "media_type",
        ),
        (
            decode_error::<DecideOptions>(json!({ "max_retries": 0 })),
            "max_retries",
        ),
        (
            decode_error::<MoaConfig>(json!({ "fail_mode": "fail_fast" })),
            "fail_mode",
        ),
    ] {
        assert!(error.contains(&format!("unknown field `{key}`")), "{error}");
    }

    // `CallOptions` stays lenient: a recording holds the call options and
    // the operation settings in one object, and replay reads both from it.
    let call: CallOptions = serde_json::from_value(json!({
        "prompt": [],
        "temperature": 0.5,
        "maxRetries": 1,
        "timeout": { "totalMs": 5 },
        "sessionId": "s",
    }))
    .unwrap();
    assert_eq!(call.temperature, Some(0.5));
}

/// Parse `wire` and check that it serializes back unchanged.
fn assert_json_back<T: Serialize + DeserializeOwned>(wire: Value) -> T {
    let value: T = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(serde_json::to_value(&value).unwrap(), wire);
    value
}
