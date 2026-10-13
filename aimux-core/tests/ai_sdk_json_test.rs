//! Golden JSON for the public wire, pinned to the AI SDK's JSON.
//!
//! The public JSON of results, stream parts, prompts and model calls is the
//! AI SDK's JSON: what its types become in JSON (`Uint8Array` as a base64
//! string, `Date` as `toISOString()`, `undefined` omitted, `null` kept). Each
//! test cites the upstream type it pins (`ai@7.0.127`,
//! `@ai-sdk/provider@4.0.21`, `@ai-sdk/provider-utils@5.0.53`).

use aimux_core::content::{ContentPart, FilePartData};
use aimux_core::decision_model::DecisionImage;
use aimux_core::error::ApiCallError;
use aimux_core::image_model::ImageOutputs;
use aimux_core::language_model_message::{
    convert_file_part_data, convert_to_language_model_prompt,
};
use aimux_core::message::{MessageContent, ModelMessage, Role};
use aimux_core::result::{
    GenerateContent, GeneratedFile, GeneratedFileOutput, LanguageModelFile, TextContent,
    ToolApprovalRequestOutput,
};
use aimux_core::shared::{FileBytes, FileData, GeneratedFileData, RequestInfo, ResponseInfo};
use aimux_core::speech_model::AudioData;
use aimux_core::stream_part::{StreamPart, TextStreamPart};
use aimux_core::timestamp::Timestamp;
use aimux_core::tool::ToolCall;
use aimux_core::types::{ProviderMetadata, ResponseMetadata};
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
    let image = ContentPart::image(vec![137, 80, 78, 71], "image/png");
    let back = assert_json(
        &image,
        json!({ "type": "image", "image": "iVBORw==", "mediaType": "image/png" }),
    );
    assert_eq!(back, image);

    let file = ContentPart::file(vec![0, 1, 2, 250, 255], "application/pdf");
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
        serde_json::from_value::<ContentPart>(
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

/// A decision image is an inline image like `ImagePart.image` (`DataContent`,
/// `provider-utils` `types/content-part.ts:42`): its bytes are one base64
/// string in the JSON (`uint8-utils.ts:12-23`), never an array of numbers.
#[test]
fn decision_image_bytes_are_a_base64_string() {
    let image = DecisionImage {
        data: FileBytes::Binary(vec![137, 80, 78, 71]),
        media_type: "image/png".into(),
        detail: Some("low".into()),
    };
    let back: DecisionImage = assert_json(
        &image,
        json!({ "data": "iVBORw==", "media_type": "image/png", "detail": "low" }),
    );
    assert_eq!(back.data, FileBytes::Base64("iVBORw==".into()));
    assert_eq!(back.data, image.data);
    assert!(
        serde_json::from_value::<DecisionImage>(
            json!({ "data": [137, 80, 78, 71], "media_type": "image/png" })
        )
        .is_err()
    );
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
    let part: ContentPart = assert_json_back(approval);
    assert!(matches!(
        part,
        ContentPart::ToolApprovalRequest {
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
        json!({ "type": "reasoning-file", "data": "aGk=", "mediaType": "image/png" }),
        json!({ "type": "reasoning-file", "data": { "type": "url", "url": "https://example.com/a.png" }, "mediaType": "image/png" }),
    ] {
        let _: ContentPart = assert_json_back(wire);
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
            serde_json::from_value::<ContentPart>(wire).is_err(),
            "{removed}"
        );
    }

    // The constructors write the tagged form the AI SDK documents.
    assert_eq!(
        serde_json::to_value(ContentPart::file_base64("aGk=", "text/plain")).unwrap(),
        json!({ "type": "file", "data": { "type": "data", "data": "aGk=" }, "mediaType": "text/plain" })
    );
    assert_eq!(
        serde_json::to_value(ContentPart::file_url(
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
        serde_json::to_value(ContentPart::file_reference("application/pdf", reference)).unwrap(),
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
        &[ModelMessage {
            role: Role::User,
            content: MessageContent::Parts(vec![ContentPart::File {
                data: FilePartData::String("data:image/png;base64,iVBORw==".into()),
                media_type: "image".into(),
                filename: None,
                provider_options: None,
            }]),
        }],
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
    assert_json(&TextContent::File(output.clone()), wire("file"));
    assert_json(
        &TextContent::ReasoningFile(output.clone()),
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

/// Parse `wire` and check that it serializes back unchanged.
fn assert_json_back<T: Serialize + DeserializeOwned>(wire: Value) -> T {
    let value: T = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(serde_json::to_value(&value).unwrap(), wire);
    value
}
