use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelMessage, LanguageModelPrompt, UserPart};
use aimux_core::options::CallOptions;
use aimux_core::shared::{FileBytes, FileData};
use aimux_provider_utils::Resolvable;

use crate::openai::config::OpenAIModelConfig;
use crate::openai::responses::{OpenAIResponsesModel, ResponsesNamespace, ResponsesProfile};
use crate::test_support::mock_fetch::{Canned, MockFetch};
use aimux_provider_utils::{Credential, ProviderHeaders};

/// What upstream's `convertToOpenAIResponsesInput` returns.
#[derive(Debug)]
pub(super) struct ConvertResult {
    pub(super) input: Value,
    pub(super) warnings: Value,
}

impl ConvertResult {
    pub(super) fn to_json(&self) -> Value {
        json!({ "input": self.input, "warnings": self.warnings })
    }
}

fn leak(text: &str) -> &'static str {
    Box::leak(text.to_owned().into_boxed_str())
}

/// Upstream's `convertToOpenAIResponsesInput(options)` (with the test file's
/// `toolSearchToolName: 'tool_search'` default), run through the aimux
/// Responses model: the converter options become the model's host profile
/// (`providerOptionsName`, `fileIdPrefixes`, `explicitMessageItemType`), its
/// call options (`store`, `systemMessageMode`, `hasConversation` as a
/// `conversation`, `hasPreviousResponseId` as a `previousResponseId`,
/// `passThroughUnsupportedFiles`) and its tools (`toolSearchToolName`,
/// `has*Tool`, `customProviderToolNames`, `outputSchemaToolNames`: the tools
/// the converter is told about). `input` is the request body's `input`,
/// `warnings` the call warnings.
pub(super) async fn convert(options: Value) -> Result<ConvertResult, AiMuxError> {
    let prompt = serde_json::from_value(options["prompt"].clone()).expect("prompt");
    convert_prompt(options, prompt).await
}

/// [`convert`] with a prompt built in Rust (binary file data has no JSON
/// form).
async fn convert_prompt(
    options: Value,
    prompt: LanguageModelPrompt,
) -> Result<ConvertResult, AiMuxError> {
    let options = options.as_object().expect("converter options object");
    let name = options
        .get("providerOptionsName")
        .and_then(Value::as_str)
        .unwrap_or("openai");
    let namespace = if name == ResponsesNamespace::OPENAI.write_key() {
        ResponsesNamespace::OPENAI
    } else {
        let name = leak(name);
        ResponsesNamespace::new(Box::leak(Box::new([name])), name)
    };
    let mut profile = ResponsesProfile {
        namespace,
        ..ResponsesProfile::default()
    };
    let mut call_options = serde_json::Map::new();
    let mut tools = vec![json!({
        "type": "provider",
        "id": "openai.tool_search",
        "name": options.get("toolSearchToolName").and_then(Value::as_str).unwrap_or("tool_search"),
        "args": {},
    })];
    for (key, value) in options {
        match key.as_str() {
            "prompt" | "providerOptionsName" | "toolSearchToolName" => {}
            "systemMessageMode" | "store" | "passThroughUnsupportedFiles" => {
                call_options.insert(key.clone(), value.clone());
            }
            "hasConversation" => {
                if value == true {
                    call_options.insert("conversation".into(), json!("conv_test"));
                }
            }
            "hasPreviousResponseId" => {
                if value == true {
                    call_options.insert("previousResponseId".into(), json!("resp_previous"));
                }
            }
            "explicitMessageItemType" => {
                profile.explicit_message_item_type = value == true;
            }
            "fileIdPrefixes" => {
                profile.file_id_prefixes = value
                    .as_array()
                    .expect("fileIdPrefixes array")
                    .iter()
                    .map(|prefix| leak(prefix.as_str().expect("prefix")))
                    .collect();
            }
            "hasLocalShellTool" | "hasShellTool" | "hasApplyPatchTool" | "hasComputerTool" => {
                if value == true {
                    let kind = match key.as_str() {
                        "hasLocalShellTool" => "local_shell",
                        "hasShellTool" => "shell",
                        "hasApplyPatchTool" => "apply_patch",
                        _ => "computer",
                    };
                    tools.push(json!({
                        "type": "provider",
                        "id": format!("openai.{kind}"),
                        "name": kind,
                        "args": {},
                    }));
                }
            }
            // A non-identity `toolNameMapping`, as `{ customName: providerToolName }`.
            "toolNameMapping" => {
                for (custom_name, provider_name) in
                    value.as_object().expect("toolNameMapping object")
                {
                    tools.push(json!({
                        "type": "provider",
                        "id": format!("openai.{}", provider_name.as_str().expect("provider tool name")),
                        "name": custom_name,
                        "args": {},
                    }));
                }
            }
            "customProviderToolNames" => {
                for tool_name in value.as_array().expect("customProviderToolNames array") {
                    tools.push(json!({
                        "type": "provider",
                        "id": "openai.custom",
                        "name": tool_name,
                        "args": {},
                    }));
                }
            }
            "outputSchemaToolNames" => {
                for tool_name in value.as_array().expect("outputSchemaToolNames array") {
                    tools.push(json!({
                        "type": "function",
                        "name": tool_name,
                        "inputSchema": { "type": "object" },
                        "providerOptions": { "openai": { "outputSchema": { "type": "object" } } },
                    }));
                }
            }
            other => panic!("converter option {other} has no harness mapping"),
        }
    }
    let mut provider_options = serde_json::Map::new();
    provider_options.insert(name.to_owned(), Value::Object(call_options));
    let mut call: CallOptions = serde_json::from_value(json!({
        "prompt": [],
        "tools": tools,
        "providerOptions": provider_options,
    }))
    .expect("call options");
    call.prompt = prompt;

    let mock = MockFetch::new(vec![Canned::json(&json!({
        "id": "resp_test",
        "created_at": 0,
        "model": "gpt-4o",
        "output": [],
        "usage": { "input_tokens": 0, "output_tokens": 0 },
    }))]);
    let mut config = OpenAIModelConfig::fixed(
        "openai".into(),
        "https://api.openai.com/v1".into(),
        ProviderHeaders::bearer(
            Credential::Explicit(Resolvable::Value("APIKEY".into())),
            Vec::new(),
            None,
        ),
        Some(mock.transport()),
        None,
    );
    config.responses = profile;
    let model = OpenAIResponsesModel::from_config("gpt-4o".into(), config);
    let result = model.do_generate(&call).await?;
    let input = mock.seen()[0].json_body()["input"].clone();
    Ok(ConvertResult {
        input,
        warnings: serde_json::to_value(&result.warnings).expect("warnings"),
    })
}

mod convert_to_openai_responses_input {
    use super::*;

    mod explicit_message_item_types {
        use super::*;

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › explicit message item types › should add the message type to system, user, and assistant messages
        #[tokio::test]
        async fn should_add_the_message_type_to_system_user_and_assistant_messages() {
            let result = convert(json!({"prompt": [{"role": "system", "content": "You are helpful."}, {"role": "user", "content": [{"type": "text", "text": "Hello"}]}, {"role": "assistant", "content": [{"type": "text", "text": "Hi!"}]}], "systemMessageMode": "system", "providerOptionsName": "azure", "explicitMessageItemType": true, "store": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([{"type": "message", "role": "system", "content": "You are helpful."}, {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "Hello"}]}, {"type": "message", "role": "assistant", "content": "Hi!"}])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › explicit message item types › should add the message type to developer messages
        #[tokio::test]
        async fn should_add_the_message_type_to_developer_messages() {
            let result = convert(json!({"prompt": [{"role": "system", "content": "You are helpful."}], "systemMessageMode": "developer", "providerOptionsName": "azure", "explicitMessageItemType": true, "store": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([{"type": "message", "role": "developer", "content": "You are helpful."}])
            );
        }
    }

    mod system_messages {
        use super::*;

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › system messages › should convert system messages to system role
        #[tokio::test]
        async fn should_convert_system_messages_to_system_role() {
            let result = convert(json!({"prompt": [{"role": "system", "content": "Hello"}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([{"role": "system", "content": "Hello"}])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › system messages › should convert system messages to developer role
        #[tokio::test]
        async fn should_convert_system_messages_to_developer_role() {
            let result = convert(json!({"prompt": [{"role": "system", "content": "Hello"}], "systemMessageMode": "developer", "providerOptionsName": "openai", "store": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([{"role": "developer", "content": "Hello"}])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › system messages › should add a prompt cache breakpoint to a system message
        #[tokio::test]
        async fn should_add_a_prompt_cache_breakpoint_to_a_system_message() {
            let result = convert(json!({"prompt": [{"role": "system", "content": "Hello", "providerOptions": {"openai": {"promptCacheBreakpoint": {"mode": "explicit"}}}}], "systemMessageMode": "developer", "providerOptionsName": "openai", "store": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([{"role": "developer", "content": [{"type": "input_text", "text": "Hello", "prompt_cache_breakpoint": {"mode": "explicit"}}]}])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › system messages › should remove system messages
        #[tokio::test]
        async fn should_remove_system_messages() {
            let result = convert(json!({"prompt": [{"role": "system", "content": "Hello"}], "systemMessageMode": "remove", "providerOptionsName": "openai", "store": true})).await.unwrap();
            assert_eq!(result.input, json!([]));
        }
    }

    mod user_messages {
        use super::*;

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › user messages › should convert messages with only a text part to a string content
        #[tokio::test]
        async fn should_convert_messages_with_only_a_text_part_to_a_string_content() {
            let result = convert(json!({"prompt": [{"role": "user", "content": [{"type": "text", "text": "Hello"}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([{"role": "user", "content": [{"type": "input_text", "text": "Hello"}]}])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › user messages › should add prompt cache breakpoints to supported content blocks
        #[tokio::test]
        async fn should_add_prompt_cache_breakpoints_to_supported_content_blocks() {
            let prompt_cache_breakpoint = json!({"mode": "explicit"});
            let result = convert(json!({"prompt": [{"role": "user", "content": [{"type": "text", "text": "Hello", "providerOptions": {"openai": {"promptCacheBreakpoint": prompt_cache_breakpoint}}}, {"type": "file", "mediaType": "image/png", "data": {"type": "url", "url": "https://example.com/image.png"}, "providerOptions": {"openai": {"promptCacheBreakpoint": prompt_cache_breakpoint}}}, {"type": "file", "mediaType": "application/pdf", "data": {"type": "reference", "reference": {"openai": "file-pdf-123"}}, "providerOptions": {"openai": {"promptCacheBreakpoint": prompt_cache_breakpoint}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([{"role": "user", "content": [{"type": "input_text", "text": "Hello", "prompt_cache_breakpoint": prompt_cache_breakpoint}, {"type": "input_image", "image_url": "https://example.com/image.png", "prompt_cache_breakpoint": prompt_cache_breakpoint}, {"type": "input_file", "file_id": "file-pdf-123", "prompt_cache_breakpoint": prompt_cache_breakpoint}]}])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › user messages › should convert messages with image parts using URL
        #[tokio::test]
        async fn should_convert_messages_with_image_parts_using_url() {
            let result = convert(json!({"prompt": [{"role": "user", "content": [{"type": "text", "text": "Hello"}, {"type": "file", "mediaType": "image/*", "data": {"type": "url", "url": "https://example.com/image.jpg"}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([{"role": "user", "content": [{"type": "input_text", "text": "Hello"}, {"type": "input_image", "image_url": "https://example.com/image.jpg"}]}])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › user messages › should convert messages with image parts using binary data
        #[tokio::test]
        async fn should_convert_messages_with_image_parts_using_binary_data() {
            let result = convert(json!({"prompt": [{"role": "user", "content": [{"type": "file", "mediaType": "image/png", "data": {"type": "data", "data": "AAECAw=="}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([{"role": "user", "content": [{"type": "input_image", "image_url": "data:image/png;base64,AAECAw=="}]}])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › user messages › should convert messages with image parts using Uint8Array
        #[tokio::test]
        async fn should_convert_messages_with_image_parts_using_uint8_array() {
            let mut prompt: LanguageModelPrompt = serde_json::from_value(json!([{"role": "user", "content": [{"type": "file", "mediaType": "image/png", "data": {"type": "data", "data": ""}}]}])).unwrap();
            let LanguageModelMessage::User { content, .. } = &mut prompt[0] else {
                unreachable!()
            };
            let UserPart::File(file) = &mut content[0] else {
                unreachable!()
            };
            file.data = FileData::Data {
                data: FileBytes::Binary(vec![0, 1, 2, 3]),
            };
            let result = convert_prompt(json!({"systemMessageMode": "system", "providerOptionsName": "openai", "store": true}), prompt).await.unwrap();
            assert_eq!(
                result.input,
                json!([{"role": "user", "content": [{"type": "input_image", "image_url": "data:image/png;base64,AAECAw=="}]}])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › user messages › should convert messages with image parts using file_id (deprecated)
        #[tokio::test]
        async fn should_convert_messages_with_image_parts_using_file_id_deprecated() {
            let result = convert(json!({"prompt": [{"role": "user", "content": [{"type": "file", "mediaType": "image/png", "data": {"type": "data", "data": "file-12345"}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "fileIdPrefixes": ["file-"], "store": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([{"role": "user", "content": [{"type": "input_image", "file_id": "file-12345"}]}])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › user messages › should convert messages with image parts using provider reference
        #[tokio::test]
        async fn should_convert_messages_with_image_parts_using_provider_reference() {
            let result = convert(json!({"prompt": [{"role": "user", "content": [{"type": "file", "mediaType": "image/png", "data": {"type": "reference", "reference": {"openai": "file-12345"}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([{"role": "user", "content": [{"type": "input_image", "file_id": "file-12345"}]}])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › user messages › detects mime type from bytes when wildcard image/*
        #[tokio::test]
        async fn detects_mime_type_from_bytes_when_wildcard_image() {
            let png_base64 = "iVBORw0KGgo=";
            let result = convert(json!({"prompt": [{"role": "user", "content": [{"type": "file", "mediaType": "image/*", "data": {"type": "data", "data": png_base64}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([{"role": "user", "content": [{"type": "input_image", "image_url": format!("data:image/png;base64,{png_base64}")}]}])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › user messages › throws when wildcard image/* bytes cannot be detected
        #[tokio::test]
        async fn throws_when_wildcard_image_bytes_cannot_be_detected() {
            let error = convert(json!({"prompt": [{"role": "user", "content": [{"type": "file", "mediaType": "image/*", "data": {"type": "data", "data": "AAECAw=="}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap_err();
            assert!(
                error.to_string().contains(r#"media type "image/*""#),
                "{error}"
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › user messages › passes through URL for top-level-only image (provider accepts URL)
        #[tokio::test]
        async fn passes_through_url_for_top_level_only_image_provider_accepts_url() {
            let result = convert(json!({"prompt": [{"role": "user", "content": [{"type": "file", "mediaType": "image", "data": {"type": "url", "url": "https://example.com/x.png"}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap();
            let first_item = &result.input[0];
            assert_eq!(
                first_item
                    .get("content")
                    .map(|content| content[0].clone())
                    .unwrap_or(Value::Null),
                json!({"type": "input_image", "image_url": "https://example.com/x.png"})
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › user messages › should add image detail when specified through extension
        #[tokio::test]
        async fn should_add_image_detail_when_specified_through_extension() {
            let result = convert(json!({"prompt": [{"role": "user", "content": [{"type": "file", "mediaType": "image/png", "data": {"type": "data", "data": "AAECAw=="}, "providerOptions": {"openai": {"imageDetail": "low"}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([{"role": "user", "content": [{"type": "input_image", "image_url": "data:image/png;base64,AAECAw==", "detail": "low"}]}])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › user messages › should read image detail from providerOptions when providerOptionsName is azure
        #[tokio::test]
        async fn should_read_image_detail_from_provider_options_when_provider_options_name_is_azu()
        {
            let result = convert(json!({"prompt": [{"role": "user", "content": [{"type": "file", "mediaType": "image/png", "data": {"type": "data", "data": "AAECAw=="}, "providerOptions": {"azure": {"imageDetail": "low"}}}]}], "systemMessageMode": "system", "providerOptionsName": "azure", "store": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([{"role": "user", "content": [{"type": "input_image", "image_url": "data:image/png;base64,AAECAw==", "detail": "low"}]}])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › user messages › should convert messages with PDF file parts
        #[tokio::test]
        async fn should_convert_messages_with_pdf_file_parts() {
            let base64_data = "AQIDBAU=";
            let result = convert(json!({"prompt": [{"role": "user", "content": [{"type": "file", "mediaType": "application/pdf", "data": {"type": "data", "data": base64_data}, "filename": "document.pdf"}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([{"role": "user", "content": [{"type": "input_file", "filename": "document.pdf", "file_data": "data:application/pdf;base64,AQIDBAU="}]}])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › user messages › should convert messages with PDF file parts using file_id (deprecated)
        #[tokio::test]
        async fn should_convert_messages_with_pdf_file_parts_using_file_id_deprecated() {
            let result = convert(json!({"prompt": [{"role": "user", "content": [{"type": "file", "mediaType": "application/pdf", "data": {"type": "data", "data": "file-pdf-12345"}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "fileIdPrefixes": ["file-"], "store": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([{"role": "user", "content": [{"type": "input_file", "file_id": "file-pdf-12345"}]}])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › user messages › should convert messages with PDF file parts using provider reference
        #[tokio::test]
        async fn should_convert_messages_with_pdf_file_parts_using_provider_reference() {
            let result = convert(json!({"prompt": [{"role": "user", "content": [{"type": "file", "mediaType": "application/pdf", "data": {"type": "reference", "reference": {"openai": "file-pdf-12345"}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([{"role": "user", "content": [{"type": "input_file", "file_id": "file-pdf-12345"}]}])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › user messages › should use default filename for PDF file parts when not provided
        #[tokio::test]
        async fn should_use_default_filename_for_pdf_file_parts_when_not_provided() {
            let base64_data = "AQIDBAU=";
            let result = convert(json!({"prompt": [{"role": "user", "content": [{"type": "file", "mediaType": "application/pdf", "data": {"type": "data", "data": base64_data}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([{"role": "user", "content": [{"type": "input_file", "filename": "part-0.pdf", "file_data": "data:application/pdf;base64,AQIDBAU="}]}])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › user messages › should throw error for unsupported file types
        #[tokio::test]
        async fn should_throw_error_for_unsupported_file_types() {
            let base64_data = "AQIDBAU=";
            let error = convert(json!({"prompt": [{"role": "user", "content": [{"type": "file", "mediaType": "text/plain", "data": {"type": "data", "data": base64_data}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("file part media type text/plain"),
                "{error}"
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › user messages › should pass through unsupported file types when enabled
        #[tokio::test]
        async fn should_pass_through_unsupported_file_types_when_enabled() {
            let base64_data = "bmFtZSxyb2xlCkFkYSxlbmdpbmVlcgo=";
            let result = convert(json!({"prompt": [{"role": "user", "content": [{"type": "file", "mediaType": "text/csv", "data": {"type": "data", "data": base64_data}, "filename": "names.csv"}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "passThroughUnsupportedFiles": true, "store": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([{"role": "user", "content": [{"type": "input_file", "filename": "names.csv", "file_data": format!("data:text/csv;base64,{base64_data}")}]}])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › user messages › should convert PDF file parts with URL to input_file with file_url
        #[tokio::test]
        async fn should_convert_pdf_file_parts_with_url_to_input_file_with_file_url() {
            let result = convert(json!({"prompt": [{"role": "user", "content": [{"type": "file", "mediaType": "application/pdf", "data": {"type": "url", "url": "https://example.com/document.pdf"}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([{"role": "user", "content": [{"type": "input_file", "file_url": "https://example.com/document.pdf"}]}])
            );
        }

        mod provider_reference_support {
            use super::*;

            /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › user messages › provider reference support › should resolve provider reference for a different providerOptionsName
            #[tokio::test]
            async fn should_resolve_provider_reference_for_a_different_provider_options_name() {
                let result = convert(json!({"prompt": [{"role": "user", "content": [{"type": "file", "mediaType": "image/png", "data": {"type": "reference", "reference": {"azure": "assistant-img-abc123"}}}]}], "systemMessageMode": "system", "providerOptionsName": "azure", "store": true})).await.unwrap();
                assert_eq!(
                    result.input,
                    json!([{"role": "user", "content": [{"type": "input_image", "file_id": "assistant-img-abc123"}]}])
                );
            }

            /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › user messages › provider reference support › should resolve provider reference for PDF parts
            #[tokio::test]
            async fn should_resolve_provider_reference_for_pdf_parts() {
                let result = convert(json!({"prompt": [{"role": "user", "content": [{"type": "file", "mediaType": "application/pdf", "data": {"type": "reference", "reference": {"openai": "file-pdf-abc123"}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap();
                assert_eq!(
                    result.input,
                    json!([{"role": "user", "content": [{"type": "input_file", "file_id": "file-pdf-abc123"}]}])
                );
            }

            /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › user messages › provider reference support › should handle multiple provider references in one message
            #[tokio::test]
            async fn should_handle_multiple_provider_references_in_one_message() {
                let result = convert(json!({"prompt": [{"role": "user", "content": [{"type": "file", "mediaType": "image/png", "data": {"type": "reference", "reference": {"openai": "file-img-abc123", "anthropic": "img-xyz"}}}, {"type": "file", "mediaType": "application/pdf", "data": {"type": "reference", "reference": {"openai": "file-pdf-xyz789", "google": "doc-123"}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap();
                assert_eq!(
                    result.input,
                    json!([{"role": "user", "content": [{"type": "input_image", "file_id": "file-img-abc123"}, {"type": "input_file", "file_id": "file-pdf-xyz789"}]}])
                );
            }

            /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › user messages › provider reference support › should throw when provider is not in the reference
            #[tokio::test]
            async fn should_throw_when_provider_is_not_in_the_reference() {
                let error = convert(json!({"prompt": [{"role": "user", "content": [{"type": "file", "mediaType": "image/png", "data": {"type": "reference", "reference": {"anthropic": "file-xyz"}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap_err();
                assert!(error.to_string().contains("No provider reference found for provider 'openai'. Available providers: anthropic"), "{error}");
            }

            /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › user messages › provider reference support › should treat plain strings as base64 data
            #[tokio::test]
            async fn should_treat_plain_strings_as_base64_data() {
                let result = convert(json!({"prompt": [{"role": "user", "content": [{"type": "file", "mediaType": "image/png", "data": {"type": "data", "data": "file-12345"}}, {"type": "file", "mediaType": "application/pdf", "data": {"type": "data", "data": "assistant-abc123"}, "filename": "test.pdf"}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap();
                assert_eq!(
                    result.input,
                    json!([{"role": "user", "content": [{"type": "input_image", "image_url": "data:image/png;base64,file-12345"}, {"type": "input_file", "filename": "test.pdf", "file_data": "data:application/pdf;base64,assistant-abc123"}]}])
                );
            }
        }

        mod deprecated_file_id_prefixes_support {
            use super::*;

            /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › user messages › deprecated fileIdPrefixes support › should convert image parts with custom prefix
            #[tokio::test]
            async fn should_convert_image_parts_with_custom_prefix() {
                let result = convert(json!({"prompt": [{"role": "user", "content": [{"type": "file", "mediaType": "image/png", "data": {"type": "data", "data": "assistant-img-abc123"}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "fileIdPrefixes": ["assistant-"], "store": true})).await.unwrap();
                assert_eq!(
                    result.input,
                    json!([{"role": "user", "content": [{"type": "input_image", "file_id": "assistant-img-abc123"}]}])
                );
            }

            /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › user messages › deprecated fileIdPrefixes support › should convert PDF parts with custom prefix
            #[tokio::test]
            async fn should_convert_pdf_parts_with_custom_prefix() {
                let result = convert(json!({"prompt": [{"role": "user", "content": [{"type": "file", "mediaType": "application/pdf", "data": {"type": "data", "data": "assistant-pdf-abc123"}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "fileIdPrefixes": ["assistant-"], "store": true})).await.unwrap();
                assert_eq!(
                    result.input,
                    json!([{"role": "user", "content": [{"type": "input_file", "file_id": "assistant-pdf-abc123"}]}])
                );
            }

            /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › user messages › deprecated fileIdPrefixes support › should support multiple file ID prefixes
            #[tokio::test]
            async fn should_support_multiple_file_id_prefixes() {
                let result = convert(json!({"prompt": [{"role": "user", "content": [{"type": "file", "mediaType": "image/png", "data": {"type": "data", "data": "assistant-img-abc123"}}, {"type": "file", "mediaType": "application/pdf", "data": {"type": "data", "data": "file-pdf-xyz789"}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "fileIdPrefixes": ["assistant-", "file-"], "store": true})).await.unwrap();
                assert_eq!(
                    result.input,
                    json!([{"role": "user", "content": [{"type": "input_image", "file_id": "assistant-img-abc123"}, {"type": "input_file", "file_id": "file-pdf-xyz789"}]}])
                );
            }

            /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › user messages › deprecated fileIdPrefixes support › should treat all file data as base64 when fileIdPrefixes is undefined
            #[tokio::test]
            async fn should_treat_all_file_data_as_base64_when_file_id_prefixes_is_undefined() {
                let result = convert(json!({"prompt": [{"role": "user", "content": [{"type": "file", "mediaType": "image/png", "data": {"type": "data", "data": "file-12345"}}, {"type": "file", "mediaType": "application/pdf", "data": {"type": "data", "data": "assistant-abc123"}, "filename": "test.pdf"}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap();
                assert_eq!(
                    result.input,
                    json!([{"role": "user", "content": [{"type": "input_image", "image_url": "data:image/png;base64,file-12345"}, {"type": "input_file", "filename": "test.pdf", "file_data": "data:application/pdf;base64,assistant-abc123"}]}])
                );
            }

            /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › user messages › deprecated fileIdPrefixes support › should handle empty fileIdPrefixes array
            #[tokio::test]
            async fn should_handle_empty_file_id_prefixes_array() {
                let result = convert(json!({"prompt": [{"role": "user", "content": [{"type": "file", "mediaType": "image/png", "data": {"type": "data", "data": "file-12345"}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "fileIdPrefixes": [], "store": true})).await.unwrap();
                assert_eq!(
                    result.input,
                    json!([{"role": "user", "content": [{"type": "input_image", "image_url": "data:image/png;base64,file-12345"}]}])
                );
            }
        }
    }

    mod assistant_messages {
        use super::*;

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › assistant messages › should convert messages with only a text part to a string content
        #[tokio::test]
        async fn should_convert_messages_with_only_a_text_part_to_a_string_content() {
            let result = convert(json!({"prompt": [{"role": "assistant", "content": [{"type": "text", "text": "Hello"}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([{"role": "assistant", "content": "Hello"}])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › assistant messages › should include phase from providerOptions on assistant text messages
        #[tokio::test]
        async fn should_include_phase_from_provider_options_on_assistant_text_messages() {
            let result = convert(json!({"prompt": [{"role": "assistant", "content": [{"type": "text", "text": "I will search for that", "providerOptions": {"openai": {"itemId": "msg_001", "phase": "commentary"}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": false})).await.unwrap();
            assert_eq!(
                result.input,
                json!([{"role": "assistant", "content": "I will search for that", "phase": "commentary"}])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › assistant messages › should include final_answer phase from providerOptions on assistant text messages
        #[tokio::test]
        async fn should_include_final_answer_phase_from_provider_options_on_assistant_text_messag()
        {
            let result = convert(json!({"prompt": [{"role": "assistant", "content": [{"type": "text", "text": "The capital of France is Paris.", "providerOptions": {"openai": {"itemId": "msg_002", "phase": "final_answer"}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": false})).await.unwrap();
            assert_eq!(
                result.input,
                json!([{"role": "assistant", "content": "The capital of France is Paris.", "phase": "final_answer"}])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › assistant messages › should not include phase when not set in providerOptions
        #[tokio::test]
        async fn should_not_include_phase_when_not_set_in_provider_options() {
            let result = convert(json!({"prompt": [{"role": "assistant", "content": [{"type": "text", "text": "Hello", "providerOptions": {"openai": {"itemId": "msg_003"}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": false})).await.unwrap();
            assert_eq!(
                result.input,
                json!([{"role": "assistant", "content": "Hello"}])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › assistant messages › should convert messages with tool call parts
        #[tokio::test]
        async fn should_convert_messages_with_tool_call_parts() {
            let result = convert(json!({"prompt": [{"role": "assistant", "content": [{"type": "text", "text": "I will search for that information."}, {"type": "tool-call", "toolCallId": "call_123", "toolName": "search", "input": {"query": "weather in San Francisco"}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([{"role": "assistant", "content": "I will search for that information."}, {"type": "function_call", "call_id": "call_123", "name": "search", "arguments": json!({"query": "weather in San Francisco"}).to_string()}])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › assistant messages › should convert text parts with ids to item_reference but not client-executed tool calls
        #[tokio::test]
        async fn should_convert_text_parts_with_ids_to_item_reference_but_not_client_executed_too()
        {
            let result = convert(json!({"prompt": [{"role": "assistant", "content": [{"type": "text", "text": "I will search for that information.", "providerOptions": {"openai": {"itemId": "id_123"}}}, {"type": "tool-call", "toolCallId": "call_123", "toolName": "search", "input": {"query": "weather in San Francisco"}, "providerOptions": {"openai": {"itemId": "id_456"}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([
                    {
                        "id": "id_123",
                        "type": "item_reference",
                    },
                    {
                        "arguments": r#"{"query":"weather in San Francisco"}"#,
                        "call_id": "call_123",
                        "name": "search",
                        "type": "function_call",
                    },
                ])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › assistant messages › should convert multiple tool call parts in a single message
        #[tokio::test]
        async fn should_convert_multiple_tool_call_parts_in_a_single_message() {
            let result = convert(json!({"prompt": [{"role": "assistant", "content": [{"type": "tool-call", "toolCallId": "call_123", "toolName": "search", "input": {"query": "weather in San Francisco"}}, {"type": "tool-call", "toolCallId": "call_456", "toolName": "calculator", "input": {"expression": "2 + 2"}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([{"type": "function_call", "call_id": "call_123", "name": "search", "arguments": json!({"query": "weather in San Francisco"}).to_string()}, {"type": "function_call", "call_id": "call_456", "name": "calculator", "arguments": json!({"expression": "2 + 2"}).to_string()}])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › assistant messages › should send client-executed tool calls as function_call items without item id (store: false)
        #[tokio::test]
        async fn should_send_client_executed_tool_calls_as_function_call_items_without_item_id_st()
        {
            let result = convert(json!({"prompt": [{"role": "assistant", "content": [{"type": "tool-call", "toolCallId": "call_a", "toolName": "search", "input": {"query": "first"}, "providerOptions": {"openai": {"itemId": "fc_a"}}}, {"type": "tool-call", "toolCallId": "call_b", "toolName": "search", "input": {"query": "second"}, "providerOptions": {"openai": {"itemId": "fc_b"}}}]}, {"role": "tool", "content": [{"type": "tool-result", "toolCallId": "call_a", "toolName": "search", "output": {"type": "json", "value": {"results": []}}}, {"type": "tool-result", "toolCallId": "call_b", "toolName": "search", "output": {"type": "json", "value": {"results": ["x"]}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": false})).await.unwrap();
            assert_eq!(
                result.input,
                json!([{"type": "function_call", "call_id": "call_a", "name": "search", "arguments": json!({"query": "first"}).to_string()}, {"type": "function_call", "call_id": "call_b", "name": "search", "arguments": json!({"query": "second"}).to_string()}, {"type": "function_call_output", "call_id": "call_a", "output": json!({"results": []}).to_string()}, {"type": "function_call_output", "call_id": "call_b", "output": json!({"results": ["x"]}).to_string()}])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › assistant messages › should not use item_reference for client-executed tool calls (store: true)
        #[tokio::test]
        async fn should_not_use_item_reference_for_client_executed_tool_calls_store_true() {
            let result = convert(json!({"prompt": [{"role": "assistant", "content": [{"type": "tool-call", "toolCallId": "call_a", "toolName": "search", "input": {"query": "first"}, "providerOptions": {"openai": {"itemId": "fc_a"}}}, {"type": "tool-call", "toolCallId": "call_b", "toolName": "search", "input": {"query": "second"}, "providerOptions": {"openai": {"itemId": "fc_b"}}}]}, {"role": "tool", "content": [{"type": "tool-result", "toolCallId": "call_a", "toolName": "search", "output": {"type": "json", "value": {"results": []}}}, {"type": "tool-result", "toolCallId": "call_b", "toolName": "search", "output": {"type": "json", "value": {"results": ["x"]}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([{"type": "function_call", "call_id": "call_a", "name": "search", "arguments": json!({"query": "first"}).to_string()}, {"type": "function_call", "call_id": "call_b", "name": "search", "arguments": json!({"query": "second"}).to_string()}, {"type": "function_call_output", "call_id": "call_a", "output": json!({"results": []}).to_string()}, {"type": "function_call_output", "call_id": "call_b", "output": json!({"results": ["x"]}).to_string()}])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › assistant messages › should round-trip namespace from providerOptions on tool call parts
        #[tokio::test]
        async fn should_round_trip_namespace_from_provider_options_on_tool_call_parts() {
            let result = convert(json!({"prompt": [{"role": "assistant", "content": [{"type": "tool-call", "toolCallId": "call_123", "toolName": "get_weather", "input": {"location": "Tokyo"}, "providerOptions": {"openai": {"itemId": "fc_abc", "namespace": "weather_tools"}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": false})).await.unwrap();
            assert_eq!(
                result.input,
                json!([{"type": "function_call", "call_id": "call_123", "name": "get_weather", "arguments": json!({"location": "Tokyo"}).to_string(), "namespace": "weather_tools"}])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › assistant messages › should round-trip async mode on function tool calls
        #[tokio::test]
        async fn should_round_trip_async_mode_on_function_tool_calls() {
            let result = convert(json!({"prompt": [{"role": "assistant", "content": [{"type": "tool-call", "toolCallId": "call_async", "toolName": "get_weather", "input": {"location": "Berlin"}, "providerOptions": {"openai": {"itemId": "fc_async", "async": true}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": false})).await.unwrap();
            assert_eq!(
                result.input,
                json!([{"type": "function_call", "call_id": "call_async", "name": "get_weather", "arguments": json!({"location": "Berlin"}).to_string(), "async": true}])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › assistant messages › should round-trip async mode on custom tool calls
        #[tokio::test]
        async fn should_round_trip_async_mode_on_custom_tool_calls() {
            let result = convert(json!({"prompt": [{"role": "assistant", "content": [{"type": "tool-call", "toolCallId": "call_custom_async", "toolName": "write_sql", "input": "SELECT 1", "providerOptions": {"openai": {"itemId": "ctc_async", "async": true}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": false, "customProviderToolNames": ["write_sql"]})).await.unwrap();
            assert_eq!(
                result.input,
                json!([{"type": "custom_tool_call", "call_id": "call_custom_async", "name": "write_sql", "input": "SELECT 1", "async": true, "id": "ctc_async"}])
            );
        }

        mod reasoning_messages_store_false {
            use super::*;

            mod single_summary_part {
                use super::*;

                /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › assistant messages › reasoning messages (store: false) › single summary part › should convert single reasoning part with text
                #[tokio::test]
                async fn should_convert_single_reasoning_part_with_text() {
                    let result = convert(json!({"prompt": [{"role": "assistant", "content": [{"type": "reasoning", "text": "Analyzing the problem step by step", "providerOptions": {"openai": {"itemId": "reasoning_001", "reasoningEncryptedContent": "encrypted_content_001"}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": false})).await.unwrap();
                    assert_eq!(
                        result.input,
                        json!([{"type": "reasoning", "id": "reasoning_001", "encrypted_content": "encrypted_content_001", "summary": [{"type": "summary_text", "text": "Analyzing the problem step by step"}]}])
                    );
                    assert_eq!(result.warnings.as_array().unwrap().len(), 0);
                }

                /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › assistant messages › reasoning messages (store: false) › single summary part › should convert single reasoning part with encrypted content
                #[tokio::test]
                async fn should_convert_single_reasoning_part_with_encrypted_content() {
                    let result = convert(json!({"prompt": [{"role": "assistant", "content": [{"type": "reasoning", "text": "Analyzing the problem step by step", "providerOptions": {"openai": {"itemId": "reasoning_001", "reasoningEncryptedContent": "encrypted_content_001"}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": false})).await.unwrap();
                    assert_eq!(
                        result.input,
                        json!([{"type": "reasoning", "id": "reasoning_001", "encrypted_content": "encrypted_content_001", "summary": [{"type": "summary_text", "text": "Analyzing the problem step by step"}]}])
                    );
                    assert_eq!(result.warnings.as_array().unwrap().len(), 0);
                }

                /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › assistant messages › reasoning messages (store: false) › single summary part › should convert single reasoning part with null encrypted content
                #[tokio::test]
                async fn should_convert_single_reasoning_part_with_null_encrypted_content() {
                    let result = convert(json!({"prompt": [{"role": "assistant", "content": [{"type": "reasoning", "text": "Analyzing the problem step by step", "providerOptions": {"openai": {"itemId": "reasoning_001", "reasoningEncryptedContent": "encrypted_content_001"}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": false})).await.unwrap();
                    assert_eq!(
                        result.input,
                        json!([{"type": "reasoning", "id": "reasoning_001", "encrypted_content": "encrypted_content_001", "summary": [{"type": "summary_text", "text": "Analyzing the problem step by step"}]}])
                    );
                    assert_eq!(result.warnings.as_array().unwrap().len(), 0);
                }
            }

            mod single_summary_part_with_empty_text {
                use super::*;

                /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › assistant messages › reasoning messages (store: false) › single summary part with empty text › should create empty summary for initial empty text
                #[tokio::test]
                async fn should_create_empty_summary_for_initial_empty_text() {
                    let result = convert(json!({"prompt": [{"role": "assistant", "content": [{"type": "reasoning", "text": "", "providerOptions": {"openai": {"itemId": "reasoning_001", "reasoningEncryptedContent": "encrypted_content_001"}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": false})).await.unwrap();
                    assert_eq!(
                        result.input,
                        json!([{"type": "reasoning", "id": "reasoning_001", "encrypted_content": "encrypted_content_001", "summary": []}])
                    );
                    assert_eq!(result.warnings.as_array().unwrap().len(), 0);
                }

                /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › assistant messages › reasoning messages (store: false) › single summary part with empty text › should create empty summary for initial empty text with encrypted content
                #[tokio::test]
                async fn should_create_empty_summary_for_initial_empty_text_with_encrypted_content()
                {
                    let result = convert(json!({"prompt": [{"role": "assistant", "content": [{"type": "reasoning", "text": "", "providerOptions": {"openai": {"itemId": "reasoning_001", "reasoningEncryptedContent": "encrypted_content_001"}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": false})).await.unwrap();
                    assert_eq!(
                        result.input,
                        json!([{"type": "reasoning", "id": "reasoning_001", "encrypted_content": "encrypted_content_001", "summary": []}])
                    );
                    assert_eq!(result.warnings.as_array().unwrap().len(), 0);
                }

                /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › assistant messages › reasoning messages (store: false) › single summary part with empty text › should warn when appending empty text to existing sequence
                #[tokio::test]
                async fn should_warn_when_appending_empty_text_to_existing_sequence() {
                    let result = convert(json!({"prompt": [{"role": "assistant", "content": [{"type": "reasoning", "text": "First reasoning step", "providerOptions": {"openai": {"itemId": "reasoning_001"}}}, {"type": "reasoning", "text": "", "providerOptions": {"openai": {"itemId": "reasoning_001", "reasoningEncryptedContent": "encrypted_content_001"}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": false})).await.unwrap();
                    assert_eq!(
                        result.input,
                        json!([{"type": "reasoning", "id": "reasoning_001", "encrypted_content": "encrypted_content_001", "summary": [{"type": "summary_text", "text": "First reasoning step"}]}])
                    );
                    assert_eq!(
                        result.warnings,
                        json!([
                            {
                                "message": r#"Cannot append empty reasoning part to existing reasoning sequence. Skipping reasoning part: {"type":"reasoning","text":"","providerOptions":{"openai":{"itemId":"reasoning_001","reasoningEncryptedContent":"encrypted_content_001"}}}."#,
                                "type": "other",
                            },
                        ])
                    );
                }
            }

            mod merging_and_sequencing {
                use super::*;

                /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › assistant messages › reasoning messages (store: false) › merging and sequencing › should merge consecutive parts with same reasoning ID
                #[tokio::test]
                async fn should_merge_consecutive_parts_with_same_reasoning_id() {
                    let result = convert(json!({"prompt": [{"role": "assistant", "content": [{"type": "reasoning", "text": "First reasoning step", "providerOptions": {"openai": {"itemId": "reasoning_001"}}}, {"type": "reasoning", "text": "Second reasoning step", "providerOptions": {"openai": {"itemId": "reasoning_001", "reasoningEncryptedContent": "encrypted_content_001"}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": false})).await.unwrap();
                    assert_eq!(
                        result.input,
                        json!([
                            {
                                "encrypted_content": "encrypted_content_001",
                                "id": "reasoning_001",
                                "summary": [
                                    {
                                        "text": "First reasoning step",
                                        "type": "summary_text",
                                    },
                                    {
                                        "text": "Second reasoning step",
                                        "type": "summary_text",
                                    },
                                ],
                                "type": "reasoning",
                            },
                        ])
                    );
                    assert_eq!(result.warnings.as_array().unwrap().len(), 0);
                }

                /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › assistant messages › reasoning messages (store: false) › merging and sequencing › should drop reasoning parts without encrypted content when store is false
                #[tokio::test]
                async fn should_drop_reasoning_parts_without_encrypted_content_when_store_is_false()
                {
                    let result = convert(json!({"prompt": [{"role": "assistant", "content": [{"type": "reasoning", "text": "First reasoning step", "providerOptions": {"openai": {"itemId": "reasoning_001"}}}, {"type": "reasoning", "text": "Second reasoning step", "providerOptions": {"openai": {"itemId": "reasoning_001"}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": false})).await.unwrap();
                    assert_eq!(result.input, json!([]));
                    assert_eq!(
                        result.warnings,
                        json!([
                            {
                                "message": "Reasoning parts without encrypted content are not supported when store is false. Skipping reasoning parts.",
                                "type": "other",
                            },
                        ])
                    );
                }

                /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › assistant messages › reasoning messages (store: false) › merging and sequencing › should create separate messages for different reasoning IDs
                #[tokio::test]
                async fn should_create_separate_messages_for_different_reasoning_ids() {
                    let result = convert(json!({"prompt": [{"role": "assistant", "content": [{"type": "reasoning", "text": "First reasoning block", "providerOptions": {"openai": {"itemId": "reasoning_001", "reasoningEncryptedContent": "encrypted_content_001"}}}, {"type": "reasoning", "text": "Second reasoning block", "providerOptions": {"openai": {"itemId": "reasoning_002", "reasoningEncryptedContent": "encrypted_content_002"}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": false})).await.unwrap();
                    assert_eq!(
                        result.input,
                        json!([{"type": "reasoning", "id": "reasoning_001", "encrypted_content": "encrypted_content_001", "summary": [{"type": "summary_text", "text": "First reasoning block"}]}, {"type": "reasoning", "id": "reasoning_002", "encrypted_content": "encrypted_content_002", "summary": [{"type": "summary_text", "text": "Second reasoning block"}]}])
                    );
                    assert_eq!(result.warnings.as_array().unwrap().len(), 0);
                }

                /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › assistant messages › reasoning messages (store: false) › merging and sequencing › should handle reasoning across multiple assistant messages
                #[tokio::test]
                async fn should_handle_reasoning_across_multiple_assistant_messages() {
                    let result = convert(json!({"prompt": [{"role": "user", "content": [{"type": "text", "text": "First user question"}]}, {"role": "assistant", "content": [{"type": "reasoning", "text": "First reasoning step (message 1)", "providerOptions": {"openai": {"itemId": "reasoning_001"}}}, {"type": "reasoning", "text": "Second reasoning step (message 1)", "providerOptions": {"openai": {"itemId": "reasoning_001"}}}, {"type": "text", "text": "First response"}]}, {"role": "user", "content": [{"type": "text", "text": "Second user question"}]}, {"role": "assistant", "content": [{"type": "reasoning", "text": "First reasoning step (message 2)", "providerOptions": {"openai": {"itemId": "reasoning_002"}}}, {"type": "text", "text": "Second response"}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap();
                    assert_eq!(
                        result.input,
                        json!([
                            {
                                "content": [
                                    {
                                        "text": "First user question",
                                        "type": "input_text",
                                    },
                                ],
                                "role": "user",
                            },
                            {
                                "id": "reasoning_001",
                                "type": "item_reference",
                            },
                            {
                                "content": "First response",
                                "role": "assistant",
                            },
                            {
                                "content": [
                                    {
                                        "text": "Second user question",
                                        "type": "input_text",
                                    },
                                ],
                                "role": "user",
                            },
                            {
                                "id": "reasoning_002",
                                "type": "item_reference",
                            },
                            {
                                "content": "Second response",
                                "role": "assistant",
                            },
                        ])
                    );
                    assert_eq!(result.warnings, json!([]));
                }

                /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › assistant messages › reasoning messages (store: false) › merging and sequencing › should handle reasoning across multiple assistant messages
                #[tokio::test]
                async fn should_handle_reasoning_across_multiple_assistant_messages_2() {
                    let result = convert(json!({"prompt": [{"role": "user", "content": [{"type": "text", "text": "First user question"}]}, {"role": "assistant", "content": [{"type": "reasoning", "text": "First reasoning step (message 1)", "providerOptions": {"openai": {"itemId": "reasoning_001"}}}, {"type": "reasoning", "text": "Second reasoning step (message 1)", "providerOptions": {"openai": {"itemId": "reasoning_001", "reasoningEncryptedContent": "encrypted_content_001"}}}, {"type": "text", "text": "First response"}]}, {"role": "user", "content": [{"type": "text", "text": "Second user question"}]}, {"role": "assistant", "content": [{"type": "reasoning", "text": "First reasoning step (message 2)", "providerOptions": {"openai": {"itemId": "reasoning_002", "reasoningEncryptedContent": "encrypted_content_002"}}}, {"type": "text", "text": "Second response"}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": false})).await.unwrap();
                    assert_eq!(
                        result.input,
                        json!([
                            {
                                "content": [
                                    {
                                        "text": "First user question",
                                        "type": "input_text",
                                    },
                                ],
                                "role": "user",
                            },
                            {
                                "encrypted_content": "encrypted_content_001",
                                "id": "reasoning_001",
                                "summary": [
                                    {
                                        "text": "First reasoning step (message 1)",
                                        "type": "summary_text",
                                    },
                                    {
                                        "text": "Second reasoning step (message 1)",
                                        "type": "summary_text",
                                    },
                                ],
                                "type": "reasoning",
                            },
                            {
                                "content": "First response",
                                "role": "assistant",
                            },
                            {
                                "content": [
                                    {
                                        "text": "Second user question",
                                        "type": "input_text",
                                    },
                                ],
                                "role": "user",
                            },
                            {
                                "encrypted_content": "encrypted_content_002",
                                "id": "reasoning_002",
                                "summary": [
                                    {
                                        "text": "First reasoning step (message 2)",
                                        "type": "summary_text",
                                    },
                                ],
                                "type": "reasoning",
                            },
                            {
                                "content": "Second response",
                                "role": "assistant",
                            },
                        ])
                    );
                    assert_eq!(result.warnings.as_array().unwrap().len(), 0);
                }

                /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › assistant messages › reasoning messages (store: false) › merging and sequencing › should handle complex reasoning sequences with tool interactions
                #[tokio::test]
                async fn should_handle_complex_reasoning_sequences_with_tool_interactions() {
                    let result = convert(json!({"prompt": [{"role": "assistant", "content": [{"type": "reasoning", "text": "Initial analysis step 1", "providerOptions": {"openai": {"itemId": "reasoning_001", "reasoningEncryptedContent": "encrypted_content_001"}}}, {"type": "reasoning", "text": "Initial analysis step 2", "providerOptions": {"openai": {"itemId": "reasoning_001", "reasoningEncryptedContent": "encrypted_content_001"}}}, {"type": "tool-call", "toolCallId": "call_001", "toolName": "search", "input": {"query": "initial search"}}]}, {"role": "tool", "content": [{"type": "tool-result", "toolCallId": "call_001", "toolName": "search", "output": {"type": "json", "value": {"results": ["result1", "result2"]}}}]}, {"role": "assistant", "content": [{"type": "reasoning", "text": "Processing results step 1", "providerOptions": {"openai": {"itemId": "reasoning_002", "reasoningEncryptedContent": "encrypted_content_002"}}}, {"type": "reasoning", "text": "Processing results step 2", "providerOptions": {"openai": {"itemId": "reasoning_002", "reasoningEncryptedContent": "encrypted_content_002"}}}, {"type": "reasoning", "text": "Processing results step 3", "providerOptions": {"openai": {"itemId": "reasoning_002", "reasoningEncryptedContent": "encrypted_content_002"}}}, {"type": "tool-call", "toolCallId": "call_002", "toolName": "calculator", "input": {"expression": "2 + 2"}}]}, {"role": "tool", "content": [{"type": "tool-result", "toolCallId": "call_002", "toolName": "calculator", "output": {"type": "json", "value": {"result": 4}}}]}, {"role": "assistant", "content": [{"type": "text", "text": "Based on my analysis and calculations, here is the final answer."}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": false})).await.unwrap();
                    assert_eq!(
                        result.input,
                        json!([{"type": "reasoning", "id": "reasoning_001", "encrypted_content": "encrypted_content_001", "summary": [{"type": "summary_text", "text": "Initial analysis step 1"}, {"type": "summary_text", "text": "Initial analysis step 2"}]}, {"type": "function_call", "call_id": "call_001", "name": "search", "arguments": json!({"query": "initial search"}).to_string()}, {"type": "function_call_output", "call_id": "call_001", "output": json!({"results": ["result1", "result2"]}).to_string()}, {"type": "reasoning", "id": "reasoning_002", "encrypted_content": "encrypted_content_002", "summary": [{"type": "summary_text", "text": "Processing results step 1"}, {"type": "summary_text", "text": "Processing results step 2"}, {"type": "summary_text", "text": "Processing results step 3"}]}, {"type": "function_call", "call_id": "call_002", "name": "calculator", "arguments": json!({"expression": "2 + 2"}).to_string()}, {"type": "function_call_output", "call_id": "call_002", "output": json!({"result": 4}).to_string()}, {"role": "assistant", "content": "Based on my analysis and calculations, here is the final answer."}])
                    );
                    assert_eq!(result.warnings.as_array().unwrap().len(), 0);
                }
            }

            mod error_handling {
                use super::*;

                /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › assistant messages › reasoning messages (store: false) › error handling › should warn when reasoning part has no provider options
                #[tokio::test]
                async fn should_warn_when_reasoning_part_has_no_provider_options() {
                    let result = convert(json!({"prompt": [{"role": "assistant", "content": [{"type": "reasoning", "text": "This is a reasoning part without any provider options"}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": false})).await.unwrap();
                    assert_eq!(result.input.as_array().unwrap().len(), 0);
                    assert_eq!(
                        result.warnings,
                        json!([
                            {
                                "message": r#"Non-OpenAI reasoning parts are not supported. Skipping reasoning part: {"type":"reasoning","text":"This is a reasoning part without any provider options"}."#,
                                "type": "other",
                            },
                        ])
                    );
                }

                /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › assistant messages › reasoning messages (store: false) › error handling › should warn when reasoning part lacks OpenAI-specific reasoning ID provider options
                #[tokio::test]
                async fn should_warn_when_reasoning_part_lacks_openai_specific_reasoning_id_provider_opti()
                 {
                    let result = convert(json!({"prompt": [{"role": "assistant", "content": [{"type": "reasoning", "text": "This is a reasoning part without OpenAI-specific reasoning id provider options", "providerOptions": {"openai": {"reasoning": {"encryptedContent": "encrypted_content_001"}}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": false})).await.unwrap();
                    assert_eq!(result.input.as_array().unwrap().len(), 0);
                    assert_eq!(
                        result.warnings,
                        json!([
                            {
                                "message": r#"Non-OpenAI reasoning parts are not supported. Skipping reasoning part: {"type":"reasoning","text":"This is a reasoning part without OpenAI-specific reasoning id provider options","providerOptions":{"openai":{"reasoning":{"encryptedContent":"encrypted_content_001"}}}}."#,
                                "type": "other",
                            },
                        ])
                    );
                }

                /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › assistant messages › reasoning messages (store: false) › error handling › should include reasoning without id when itemId is absent but encrypted_content is present
                #[tokio::test]
                async fn should_include_reasoning_without_id_when_item_id_is_absent_but_encrypted_content()
                 {
                    let result = convert(json!({"prompt": [{"role": "assistant", "content": [{"type": "reasoning", "text": "Thinking through the problem", "providerOptions": {"openai": {"reasoningEncryptedContent": "encrypted_reasoning_blob"}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": false})).await.unwrap();
                    assert_eq!(
                        result.input,
                        json!([{"type": "reasoning", "encrypted_content": "encrypted_reasoning_blob", "summary": [{"type": "summary_text", "text": "Thinking through the problem"}]}])
                    );
                    assert_eq!(result.warnings.as_array().unwrap().len(), 0);
                }

                /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › assistant messages › reasoning messages (store: false) › error handling › should still warn when itemId is absent and encrypted_content is also absent
                #[tokio::test]
                async fn should_still_warn_when_item_id_is_absent_and_encrypted_content_is_also_absent()
                 {
                    let result = convert(json!({"prompt": [{"role": "assistant", "content": [{"type": "reasoning", "text": "Some reasoning text", "providerOptions": {"openai": {}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": false})).await.unwrap();
                    assert_eq!(result.input.as_array().unwrap().len(), 0);
                    assert_eq!(
                        result.warnings,
                        json!([
                            {
                                "message": r#"Non-OpenAI reasoning parts are not supported. Skipping reasoning part: {"type":"reasoning","text":"Some reasoning text","providerOptions":{"openai":{}}}."#,
                                "type": "other",
                            },
                        ])
                    );
                }
            }
        }
    }

    mod tool_messages {
        use super::*;

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › tool messages › should preserve prompt cache breakpoints on scalar tool results
        #[tokio::test]
        async fn should_preserve_prompt_cache_breakpoints_on_scalar_tool_results() {
            let prompt_cache_breakpoint = json!({"mode": "explicit"});
            let provider_options =
                json!({"openai": {"promptCacheBreakpoint": prompt_cache_breakpoint}});
            let scalar_outputs = json!([{"output": {"type": "text", "value": "stable tool output"}, "expectedText": "stable tool output"}, {"output": {"type": "json", "value": {"stable": true}}, "expectedText": r#"{"stable":true}"#}, {"output": {"type": "error-text", "value": "tool error"}, "expectedText": "tool error"}, {"output": {"type": "error-json", "value": {"error": "boom"}}, "expectedText": r#"{"error":"boom"}"#}, {"output": {"type": "execution-denied", "reason": "execution denied"}, "expectedText": "execution denied"}]);
            let mut tool_results = Vec::new();
            for (output_index, entry) in scalar_outputs.as_array().unwrap().iter().enumerate() {
                for placement in ["output", "tool-result"] {
                    let mut part = json!({
                        "type": "tool-result",
                        "toolCallId": format!("call_{output_index}_{placement}"),
                        "toolName": "lookup",
                        "output": entry["output"],
                    });
                    if placement == "output" {
                        part["output"]["providerOptions"] = provider_options.clone();
                    } else {
                        part["providerOptions"] = provider_options.clone();
                    }
                    tool_results.push(part);
                }
            }
            let result = convert(json!({"prompt": [{"role": "tool", "content": tool_results}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap();
            let mut expected = Vec::new();
            for (output_index, entry) in scalar_outputs.as_array().unwrap().iter().enumerate() {
                for placement in ["output", "tool-result"] {
                    expected.push(json!({
                        "type": "function_call_output",
                        "call_id": format!("call_{output_index}_{placement}"),
                        "output": [{
                            "type": "input_text",
                            "text": entry["expectedText"],
                            "prompt_cache_breakpoint": prompt_cache_breakpoint,
                        }],
                    }));
                }
            }
            assert_eq!(result.input, json!(expected));
            assert_eq!(result.warnings, json!([]));
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › tool messages › should convert single tool result part with json value
        #[tokio::test]
        async fn should_convert_single_tool_result_part_with_json_value() {
            let result = convert(json!({"prompt": [{"role": "tool", "content": [{"type": "tool-result", "toolCallId": "call_123", "toolName": "search", "output": {"type": "json", "value": {"temperature": "72°F", "condition": "Sunny"}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([
                    {
                        "call_id": "call_123",
                        "output": r#"{"temperature":"72°F","condition":"Sunny"}"#,
                        "type": "function_call_output",
                    },
                ])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › tool messages › should convert single tool result part with text value
        #[tokio::test]
        async fn should_convert_single_tool_result_part_with_text_value() {
            let result = convert(json!({"prompt": [{"role": "tool", "content": [{"type": "tool-result", "toolCallId": "call_123", "toolName": "search", "output": {"type": "text", "value": "The weather in San Francisco is 72°F"}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([
                    {
                        "call_id": "call_123",
                        "output": "The weather in San Francisco is 72°F",
                        "type": "function_call_output",
                    },
                ])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › tool messages › should JSON-encode text outputs only for tools with an output schema
        #[tokio::test]
        async fn should_json_encode_text_outputs_only_for_tools_with_an_output_schema() {
            let prompt_cache_breakpoint = json!({"mode": "explicit"});
            let result = convert(json!({"prompt": [{"role": "tool", "content": [{"type": "tool-result", "toolCallId": "call_text", "toolName": "search", "output": {"type": "text", "value": "The weather is sunny"}}, {"type": "tool-result", "toolCallId": "call_error", "toolName": "search", "output": {"type": "error-text", "value": "Error: boom"}}, {"type": "tool-result", "toolCallId": "call_denied", "toolName": "search", "output": {"type": "execution-denied", "reason": "User denied the tool execution"}}, {"type": "tool-result", "toolCallId": "call_without_schema", "toolName": "lookup", "output": {"type": "error-text", "value": "Error: unchanged"}}, {"type": "tool-result", "toolCallId": "call_schema_breakpoint", "toolName": "search", "output": {"type": "text", "value": "Structured output", "providerOptions": {"openai": {"promptCacheBreakpoint": prompt_cache_breakpoint}}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true, "outputSchemaToolNames": ["search"]})).await.unwrap();
            assert_eq!(
                result.input,
                json!([{"type": "function_call_output", "call_id": "call_text", "output": r#""The weather is sunny""#}, {"type": "function_call_output", "call_id": "call_error", "output": r#""Error: boom""#}, {"type": "function_call_output", "call_id": "call_denied", "output": r#""User denied the tool execution""#}, {"type": "function_call_output", "call_id": "call_without_schema", "output": "Error: unchanged"}, {"type": "function_call_output", "call_id": "call_schema_breakpoint", "output": [{"type": "input_text", "text": r#""Structured output""#, "prompt_cache_breakpoint": prompt_cache_breakpoint}]}])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › tool messages › should convert execution-denied tool result to function_call_output
        #[tokio::test]
        async fn should_convert_execution_denied_tool_result_to_function_call_output() {
            let result = convert(json!({"prompt": [{"role": "tool", "content": [{"type": "tool-result", "toolCallId": "call_denied_123", "toolName": "search", "output": {"type": "execution-denied", "reason": "User denied the tool execution"}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([{"type": "function_call_output", "call_id": "call_denied_123", "output": "User denied the tool execution"}])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › tool messages › should reject execution-denied programmatic tool results
        #[tokio::test]
        async fn should_reject_execution_denied_programmatic_tool_results() {
            let error = convert(json!({"prompt": [{"role": "assistant", "content": [{"type": "tool-call", "toolCallId": "call_denied_123", "toolName": "search", "input": {"query": "test"}, "providerOptions": {"openai": {"caller": {"type": "program", "callerId": "program_call_123"}}}}]}, {"role": "tool", "content": [{"type": "tool-result", "toolCallId": "call_denied_123", "toolName": "search", "output": {"type": "execution-denied", "reason": "User denied the tool execution"}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap_err();
            assert!(
                matches!(error, AiMuxError::UnsupportedFunctionality(_)),
                "{error}"
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › tool messages › should convert single tool result part with multipart that contains text
        #[tokio::test]
        async fn should_convert_single_tool_result_part_with_multipart_that_contains_text() {
            let result = convert(json!({"prompt": [{"role": "tool", "content": [{"type": "tool-result", "toolCallId": "call_123", "toolName": "search", "output": {"type": "content", "value": [{"type": "text", "text": "The weather in San Francisco is 72°F"}]}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([
                    {
                        "call_id": "call_123",
                        "output": [
                            {
                                "text": "The weather in San Francisco is 72°F",
                                "type": "input_text",
                            },
                        ],
                        "type": "function_call_output",
                    },
                ])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › tool messages › should convert single tool result part with multipart that contains image
        #[tokio::test]
        async fn should_convert_single_tool_result_part_with_multipart_that_contains_image() {
            let result = convert(json!({"prompt": [{"role": "tool", "content": [{"type": "tool-result", "toolCallId": "call_123", "toolName": "search", "output": {"type": "content", "value": [{"type": "file", "mediaType": "image/png", "data": {"type": "data", "data": "base64_data"}}]}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([
                    {
                        "call_id": "call_123",
                        "output": [
                            {
                                "image_url": "data:image/png;base64,base64_data",
                                "type": "input_image",
                            },
                        ],
                        "type": "function_call_output",
                    },
                ])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › tool messages › should forward openai.imageDetail providerOptions on tool-result image (data)
        #[tokio::test]
        async fn should_forward_openai_image_detail_provider_options_on_tool_result_image_data() {
            let result = convert(json!({"prompt": [{"role": "tool", "content": [{"type": "tool-result", "toolCallId": "call_123", "toolName": "view_image", "output": {"type": "content", "value": [{"type": "file", "mediaType": "image/png", "data": {"type": "data", "data": "base64_data"}, "providerOptions": {"openai": {"imageDetail": "original"}}}]}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([
                    {
                        "call_id": "call_123",
                        "output": [
                            {
                                "detail": "original",
                                "image_url": "data:image/png;base64,base64_data",
                                "type": "input_image",
                            },
                        ],
                        "type": "function_call_output",
                    },
                ])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › tool messages › should forward openai.imageDetail providerOptions on tool-result image (url)
        #[tokio::test]
        async fn should_forward_openai_image_detail_provider_options_on_tool_result_image_url() {
            let result = convert(json!({"prompt": [{"role": "tool", "content": [{"type": "tool-result", "toolCallId": "call_123", "toolName": "view_image", "output": {"type": "content", "value": [{"type": "file", "mediaType": "image/png", "data": {"type": "url", "url": "https://example.com/x.png"}, "providerOptions": {"openai": {"imageDetail": "high"}}}]}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([
                    {
                        "call_id": "call_123",
                        "output": [
                            {
                                "detail": "high",
                                "image_url": "https://example.com/x.png",
                                "type": "input_image",
                            },
                        ],
                        "type": "function_call_output",
                    },
                ])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › tool messages › should convert single tool result part with multipart that contains image URL
        #[tokio::test]
        async fn should_convert_single_tool_result_part_with_multipart_that_contains_image_url() {
            let result = convert(json!({"prompt": [{"role": "tool", "content": [{"type": "tool-result", "toolCallId": "call_123", "toolName": "screenshot", "output": {"type": "content", "value": [{"type": "file", "data": {"type": "url", "url": "https://example.com/screenshot.png"}, "mediaType": "image/png"}]}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([
                    {
                        "call_id": "call_123",
                        "output": [
                            {
                                "image_url": "https://example.com/screenshot.png",
                                "type": "input_image",
                            },
                        ],
                        "type": "function_call_output",
                    },
                ])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › tool messages › should convert provider references in tool result content
        #[tokio::test]
        async fn should_convert_provider_references_in_tool_result_content() {
            let prompt_cache_breakpoint = json!({"mode": "explicit"});
            let result = convert(json!({"prompt": [{"role": "tool", "content": [{"type": "tool-result", "toolCallId": "call_123", "toolName": "search", "output": {"type": "content", "value": [{"type": "text", "text": "Referenced files:"}, {"type": "file", "mediaType": "application/pdf", "data": {"type": "reference", "reference": {"openai": "file-pdf-123"}}, "providerOptions": {"openai": {"promptCacheBreakpoint": prompt_cache_breakpoint}}}, {"type": "file", "mediaType": "image/png", "data": {"type": "reference", "reference": {"openai": "file-image-123"}}, "providerOptions": {"openai": {"imageDetail": "high", "promptCacheBreakpoint": prompt_cache_breakpoint}}}]}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([{"type": "function_call_output", "call_id": "call_123", "output": [{"type": "input_text", "text": "Referenced files:"}, {"type": "input_file", "file_id": "file-pdf-123", "prompt_cache_breakpoint": prompt_cache_breakpoint}, {"type": "input_image", "file_id": "file-image-123", "detail": "high", "prompt_cache_breakpoint": prompt_cache_breakpoint}]}])
            );
            assert_eq!(result.warnings, json!([]));
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › tool messages › should convert single tool result part with multipart that contains file (PDF)
        #[tokio::test]
        async fn should_convert_single_tool_result_part_with_multipart_that_contains_file_pdf() {
            let base64_data = "AQIDBAU=";
            let result = convert(json!({"prompt": [{"role": "tool", "content": [{"type": "tool-result", "toolCallId": "call_123", "toolName": "search", "output": {"type": "content", "value": [{"type": "file", "mediaType": "application/pdf", "data": {"type": "data", "data": base64_data}, "filename": "document.pdf"}]}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([
                    {
                        "call_id": "call_123",
                        "output": [
                            {
                                "file_data": "data:application/pdf;base64,AQIDBAU=",
                                "filename": "document.pdf",
                                "type": "input_file",
                            },
                        ],
                        "type": "function_call_output",
                    },
                ])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › tool messages › should convert single tool result part with multipart that contains file-url
        #[tokio::test]
        async fn should_convert_single_tool_result_part_with_multipart_that_contains_file_url() {
            let result = convert(json!({"prompt": [{"role": "tool", "content": [{"type": "tool-result", "toolCallId": "call_123", "toolName": "search", "output": {"type": "content", "value": [{"type": "file", "data": {"type": "url", "url": "https://example.com/document.pdf"}, "mediaType": "application/pdf"}]}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([
                    {
                        "call_id": "call_123",
                        "output": [
                            {
                                "file_url": "https://example.com/document.pdf",
                                "type": "input_file",
                            },
                        ],
                        "type": "function_call_output",
                    },
                ])
            );
            assert_eq!(result.warnings, json!([]));
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › tool messages › should convert single tool result part with multipart with mixed content including file-url
        #[tokio::test]
        async fn should_convert_single_tool_result_part_with_multipart_with_mixed_content_includi()
        {
            let result = convert(json!({"prompt": [{"role": "tool", "content": [{"type": "tool-result", "toolCallId": "call_123", "toolName": "search", "output": {"type": "content", "value": [{"type": "text", "text": "Here is the file you asked for:"}, {"type": "file", "data": {"type": "url", "url": "https://example.com/test.pdf"}, "mediaType": "application/pdf"}]}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([
                    {
                        "call_id": "call_123",
                        "output": [
                            {
                                "text": "Here is the file you asked for:",
                                "type": "input_text",
                            },
                            {
                                "file_url": "https://example.com/test.pdf",
                                "type": "input_file",
                            },
                        ],
                        "type": "function_call_output",
                    },
                ])
            );
            assert_eq!(result.warnings, json!([]));
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › tool messages › should convert single tool result part with multipart with mixed content (text, image, file)
        #[tokio::test]
        async fn should_convert_single_tool_result_part_with_multipart_with_mixed_content_text_im()
        {
            let base64_data = "AQIDBAU=";
            let result = convert(json!({"prompt": [{"role": "tool", "content": [{"type": "tool-result", "toolCallId": "call_123", "toolName": "search", "output": {"type": "content", "value": [{"type": "text", "text": "The weather in San Francisco is 72°F"}, {"type": "file", "mediaType": "image/png", "data": {"type": "data", "data": "base64_data"}}, {"type": "file", "mediaType": "application/pdf", "data": {"type": "data", "data": base64_data}}]}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([
                    {
                        "call_id": "call_123",
                        "output": [
                            {
                                "text": "The weather in San Francisco is 72°F",
                                "type": "input_text",
                            },
                            {
                                "image_url": "data:image/png;base64,base64_data",
                                "type": "input_image",
                            },
                            {
                                "file_data": "data:application/pdf;base64,AQIDBAU=",
                                "filename": "data",
                                "type": "input_file",
                            },
                        ],
                        "type": "function_call_output",
                    },
                ])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › tool messages › should convert multiple tool result parts in a single message
        #[tokio::test]
        async fn should_convert_multiple_tool_result_parts_in_a_single_message() {
            let result = convert(json!({"prompt": [{"role": "tool", "content": [{"type": "tool-result", "toolCallId": "call_123", "toolName": "search", "output": {"type": "json", "value": {"temperature": "72°F", "condition": "Sunny"}}}, {"type": "tool-result", "toolCallId": "call_456", "toolName": "calculator", "output": {"type": "json", "value": 4}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([{"type": "function_call_output", "call_id": "call_123", "output": json!({"temperature": "72°F", "condition": "Sunny"}).to_string()}, {"type": "function_call_output", "call_id": "call_456", "output": json!(4).to_string()}])
            );
        }
    }

    mod provider_defined_tools {
        use super::*;

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › provider-defined tools › should convert single provider-executed tool call and result into item reference with store: true
        #[tokio::test]
        async fn should_convert_single_provider_executed_tool_call_and_result_into_item_reference()
        {
            let result = convert(json!({"prompt": [{"role": "assistant", "content": [{"input": {"code": "example code", "containerId": "container_123"}, "providerExecuted": true, "toolCallId": "ci_68c2e2cf522c81908f3e2c1bccd1493b0b24aae9c6c01e4f", "toolName": "code_interpreter", "type": "tool-call"}, {"output": {"type": "json", "value": {"outputs": [{"type": "logs", "logs": "example logs"}]}}, "toolCallId": "ci_68c2e2cf522c81908f3e2c1bccd1493b0b24aae9c6c01e4f", "toolName": "code_interpreter", "type": "tool-result"}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([
                    {
                        "id": "ci_68c2e2cf522c81908f3e2c1bccd1493b0b24aae9c6c01e4f",
                        "type": "item_reference",
                    },
                ])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › provider-defined tools › should reconstruct hosted tool search call and output with store: false
        #[tokio::test]
        async fn should_reconstruct_hosted_tool_search_call_and_output_with_store_false() {
            let result = convert(json!({"prompt": [{"role": "assistant", "content": [{"type": "tool-call", "toolCallId": "tsc_hosted_123", "toolName": "tool_search", "input": json!({"arguments": {"paths": ["get_weather"]}, "call_id": null}).to_string(), "providerExecuted": true, "providerOptions": {"openai": {"itemId": "tsc_hosted_123"}}}, {"type": "tool-result", "toolCallId": "tsc_hosted_123", "toolName": "tool_search", "output": {"type": "json", "value": {"tools": [{"type": "function", "name": "get_weather", "defer_loading": true}]}}, "providerOptions": {"openai": {"itemId": "tso_hosted_456"}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": false})).await.unwrap();
            assert_eq!(
                result.to_json(),
                json!({"input": [{"type": "tool_search_call", "id": "tsc_hosted_123", "execution": "server", "call_id": null, "status": "completed", "arguments": {"paths": ["get_weather"]}}, {"type": "tool_search_output", "id": "tso_hosted_456", "execution": "server", "call_id": null, "status": "completed", "tools": [{"type": "function", "name": "get_weather", "defer_loading": true}]}], "warnings": []})
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › provider-defined tools › should use distinct item references for hosted tool search call and output with store: true
        #[tokio::test]
        async fn should_use_distinct_item_references_for_hosted_tool_search_call_and_output_with() {
            let result = convert(json!({"prompt": [{"role": "assistant", "content": [{"type": "tool-call", "toolCallId": "tsc_hosted_123", "toolName": "tool_search", "input": json!({"arguments": {"paths": ["get_weather"]}, "call_id": null}).to_string(), "providerExecuted": true, "providerOptions": {"openai": {"itemId": "tsc_hosted_123"}}}, {"type": "tool-result", "toolCallId": "tsc_hosted_123", "toolName": "tool_search", "output": {"type": "json", "value": {"tools": [{"type": "function", "name": "get_weather"}]}}, "providerOptions": {"openai": {"itemId": "tso_hosted_456"}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap();
            assert_eq!(
                result.to_json(),
                json!({"input": [{"type": "item_reference", "id": "tsc_hosted_123"}, {"type": "item_reference", "id": "tso_hosted_456"}], "warnings": []})
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › provider-defined tools › should serialize client tool search output with call_id from tool role
        #[tokio::test]
        async fn should_serialize_client_tool_search_output_with_call_id_from_tool_role() {
            let result = convert(json!({"prompt": [{"role": "assistant", "content": [{"type": "tool-call", "toolCallId": "call_abc123", "toolName": "tool_search", "input": json!({"arguments": {"goal": "Find weather tools"}, "call_id": "call_abc123"}).to_string(), "providerOptions": {"openai": {"itemId": "tsc_client_1"}}}]}, {"role": "tool", "content": [{"type": "tool-result", "toolCallId": "call_abc123", "toolName": "tool_search", "output": {"type": "json", "value": {"tools": [{"type": "function", "name": "get_weather", "description": "Get weather", "defer_loading": true, "parameters": {"type": "object", "properties": {"location": {"type": "string"}}, "required": ["location"]}}]}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": false})).await.unwrap();
            assert_eq!(
                result.to_json(),
                json!({"input": [{"type": "tool_search_call", "id": "tsc_client_1", "execution": "client", "call_id": "call_abc123", "status": "completed", "arguments": {"goal": "Find weather tools"}}, {"type": "tool_search_output", "execution": "client", "call_id": "call_abc123", "status": "completed", "tools": [{"type": "function", "name": "get_weather", "description": "Get weather", "defer_loading": true, "parameters": {"type": "object", "properties": {"location": {"type": "string"}}, "required": ["location"]}}]}], "warnings": []})
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › provider-defined tools › should use call_id (not item id) for client tool search call serialization
        #[tokio::test]
        async fn should_use_call_id_not_item_id_for_client_tool_search_call_serialization() {
            let result = convert(json!({"prompt": [{"role": "assistant", "content": [{"type": "tool-call", "toolCallId": "call_xyz789", "toolName": "tool_search", "input": json!({"arguments": {"goal": "Find tools"}, "call_id": "call_xyz789"}).to_string(), "providerOptions": {"openai": {"itemId": "tsc_item_id"}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": false})).await.unwrap();
            let tool_search_call = result
                .input
                .as_array()
                .unwrap()
                .iter()
                .find(|item| item["type"] == "tool_search_call")
                .cloned()
                .unwrap_or(Value::Null);
            assert_eq!(tool_search_call["call_id"], "call_xyz789");
            assert_eq!(tool_search_call["id"], "tsc_item_id");
            assert_eq!(tool_search_call["execution"], "client");
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › provider-defined tools › should exclude provider-executed tool calls and results from prompt with store: false
        #[tokio::test]
        async fn should_exclude_provider_executed_tool_calls_and_results_from_prompt_with_store_f()
        {
            let result = convert(json!({"prompt": [{"role": "assistant", "content": [{"type": "text", "text": "Let me search for recent news from San Francisco."}, {"type": "tool-call", "toolCallId": "ws_67cf2b3051e88190b006770db6fdb13d", "toolName": "web_search", "input": {"query": "San Francisco major news events June 22 2025"}, "providerExecuted": true}, {"type": "tool-result", "toolCallId": "ws_67cf2b3051e88190b006770db6fdb13d", "toolName": "web_search", "output": {"type": "json", "value": {"action": {"type": "search", "query": "San Francisco major news events June 22 2025"}, "sources": [{"type": "url", "url": "https://patch.com/california/san-francisco/calendar"}]}}}, {"type": "text", "text": "Based on the search results, several significant events took place in San Francisco yesterday (June 22, 2025)."}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": false})).await.unwrap();
            assert_eq!(
                result.to_json(),
                json!({
                    "input": [
                        {
                            "content": "Let me search for recent news from San Francisco.",
                            "role": "assistant",
                        },
                        {
                            "content": "Based on the search results, several significant events took place in San Francisco yesterday (June 22, 2025).",
                            "role": "assistant",
                        },
                    ],
                    "warnings": [
                        {
                            "message": "Results for OpenAI tool web_search are not sent to the API when store is false",
                            "type": "other",
                        },
                    ],
                })
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › provider-defined tools › should skip provider-executed execution-denied tool results in assistant messages
        #[tokio::test]
        async fn should_skip_provider_executed_execution_denied_tool_results_in_assistant_message()
        {
            let result = convert(json!({"prompt": [{"role": "assistant", "content": [{"type": "text", "text": "I need approval before running that tool."}, {"type": "tool-result", "toolCallId": "ws_denied_123", "toolName": "web_search", "output": {"type": "execution-denied", "reason": "User denied the tool execution"}}, {"type": "text", "text": "The tool was not run."}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": false})).await.unwrap();
            assert_eq!(
                result.to_json(),
                json!({"input": [{"role": "assistant", "content": "I need approval before running that tool."}, {"role": "assistant", "content": "The tool was not run."}], "warnings": []})
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › provider-defined tools › should skip json-wrapped execution-denied tool results in assistant messages
        #[tokio::test]
        async fn should_skip_json_wrapped_execution_denied_tool_results_in_assistant_messages() {
            let result = convert(json!({"prompt": [{"role": "assistant", "content": [{"type": "text", "text": "I need approval before running that tool."}, {"type": "tool-result", "toolCallId": "ws_denied_json_123", "toolName": "web_search", "output": {"type": "json", "value": {"type": "execution-denied", "reason": "User denied the tool execution"}}}, {"type": "text", "text": "The tool was not run."}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": false})).await.unwrap();
            assert_eq!(
                result.to_json(),
                json!({"input": [{"role": "assistant", "content": "I need approval before running that tool."}, {"role": "assistant", "content": "The tool was not run."}], "warnings": []})
            );
        }

        mod provider_executed_shell {
            use super::*;

            /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › provider-defined tools › provider-executed shell › should reconstruct the shell call and output with store: false
            #[tokio::test]
            async fn should_reconstruct_the_shell_call_and_output_with_store_false() {
                let call_id = "call_shell";
                let result = convert(json!({"prompt": [{"role": "user", "content": [{"type": "text", "text": "Run `printf hello` using the shell tool."}]}, {"role": "assistant", "content": [{"type": "tool-call", "toolCallId": call_id, "toolName": "shell", "input": {"action": {"commands": ["printf hello"]}}, "providerExecuted": true, "providerOptions": {"openai": {"itemId": "shell_item"}}}, {"type": "tool-result", "toolCallId": call_id, "toolName": "shell", "output": {"type": "json", "value": {"output": [{"stdout": "hello", "stderr": "", "outcome": {"type": "exit", "exitCode": 0}}]}}}, {"type": "text", "text": "hello"}]}, {"role": "user", "content": [{"type": "text", "text": "What did the command print?"}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": false, "hasShellTool": true})).await.unwrap();
                let input = result.input.as_array().unwrap();
                assert!(
                    input.contains(&json!({"type": "shell_call", "call_id": call_id, "id": "shell_item", "status": "completed", "action": {"commands": ["printf hello"]}})),
                    "{input:?}"
                );
                assert!(
                    input.contains(&json!({"type": "shell_call_output", "call_id": call_id, "output": [{"stdout": "hello", "stderr": "", "outcome": {"type": "exit", "exit_code": 0}}]})),
                    "{input:?}"
                );
            }
        }

        mod local_shell {
            use super::*;

            /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › provider-defined tools › local shell › should convert local shell tool call and result into item reference with store: true
            #[tokio::test]
            async fn should_convert_local_shell_tool_call_and_result_into_item_reference_with_store_t()
             {
                let result = convert(json!({"prompt": [{"role": "assistant", "content": [{"type": "tool-call", "toolCallId": "call_XWgeTylovOiS8xLNz2TONOgO", "toolName": "local_shell", "input": {"action": {"type": "exec", "command": ["ls"]}}, "providerOptions": {"openai": {"itemId": "lsh_68c2e2cf522c81908f3e2c1bccd1493b0b24aae9c6c01e4f"}}}]}, {"role": "tool", "content": [{"type": "tool-result", "toolCallId": "call_XWgeTylovOiS8xLNz2TONOgO", "toolName": "local_shell", "output": {"type": "json", "value": {"output": "example output"}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true, "hasLocalShellTool": true})).await.unwrap();
                assert_eq!(
                    result.input,
                    json!([
                        {
                            "id": "lsh_68c2e2cf522c81908f3e2c1bccd1493b0b24aae9c6c01e4f",
                            "type": "item_reference",
                        },
                        {
                            "call_id": "call_XWgeTylovOiS8xLNz2TONOgO",
                            "output": "example output",
                            "type": "local_shell_call_output",
                        },
                    ])
                );
            }

            /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › provider-defined tools › local shell › should convert local shell tool call and result into item reference with store: false
            #[tokio::test]
            async fn should_convert_local_shell_tool_call_and_result_into_item_reference_with_store_f()
             {
                let result = convert(json!({"prompt": [{"role": "assistant", "content": [{"type": "tool-call", "toolCallId": "call_XWgeTylovOiS8xLNz2TONOgO", "toolName": "local_shell", "input": {"action": {"type": "exec", "command": ["ls"]}}, "providerOptions": {"openai": {"itemId": "lsh_68c2e2cf522c81908f3e2c1bccd1493b0b24aae9c6c01e4f"}}}]}, {"role": "tool", "content": [{"type": "tool-result", "toolCallId": "call_XWgeTylovOiS8xLNz2TONOgO", "toolName": "local_shell", "output": {"type": "json", "value": {"output": "example output"}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": false, "hasLocalShellTool": true})).await.unwrap();
                assert_eq!(
                    result.input,
                    json!([
                        {
                            "action": {
                                "command": [
                                    "ls",
                                ],
                                "type": "exec",
                            },
                            "call_id": "call_XWgeTylovOiS8xLNz2TONOgO",
                            "id": "lsh_68c2e2cf522c81908f3e2c1bccd1493b0b24aae9c6c01e4f",
                            "type": "local_shell_call",
                        },
                        {
                            "call_id": "call_XWgeTylovOiS8xLNz2TONOgO",
                            "output": "example output",
                            "type": "local_shell_call_output",
                        },
                    ])
                );
            }
        }

        mod apply_patch {
            use super::*;

            /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › provider-defined tools › apply_patch › should convert apply_patch tool call into item reference with store: true
            #[tokio::test]
            async fn should_convert_apply_patch_tool_call_into_item_reference_with_store_true() {
                let result = convert(json!({"prompt": [{"role": "assistant", "content": [{"type": "tool-call", "toolCallId": "call_INoksNAffcdh5UmRTWMLk1Ne", "toolName": "apply_patch", "input": {"callId": "call_INoksNAffcdh5UmRTWMLk1Ne", "operation": {"type": "create_file", "path": "index.html", "diff": "+<!doctype html>\n+<html></html>"}}, "providerOptions": {"openai": {"itemId": "apc_0d5dfb28a009b1ee0169713022c3f88195a70b253d2a8cf798"}}}]}, {"role": "tool", "content": [{"type": "tool-result", "toolCallId": "call_INoksNAffcdh5UmRTWMLk1Ne", "toolName": "apply_patch", "output": {"type": "json", "value": {"status": "completed", "output": "Created index.html"}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true, "hasApplyPatchTool": true})).await.unwrap();
                assert_eq!(
                    result.input,
                    json!([
                        {
                            "id": "apc_0d5dfb28a009b1ee0169713022c3f88195a70b253d2a8cf798",
                            "type": "item_reference",
                        },
                        {
                            "call_id": "call_INoksNAffcdh5UmRTWMLk1Ne",
                            "output": "Created index.html",
                            "status": "completed",
                            "type": "apply_patch_call_output",
                        },
                    ])
                );
            }

            /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › provider-defined tools › apply_patch › should convert apply_patch tool call to apply_patch_call with store: false
            #[tokio::test]
            async fn should_convert_apply_patch_tool_call_to_apply_patch_call_with_store_false() {
                let result = convert(json!({"prompt": [{"role": "assistant", "content": [{"type": "tool-call", "toolCallId": "call_INoksNAffcdh5UmRTWMLk1Ne", "toolName": "apply_patch", "input": {"callId": "call_INoksNAffcdh5UmRTWMLk1Ne", "operation": {"type": "create_file", "path": "index.html", "diff": "+<!doctype html>\n+<html></html>"}}, "providerOptions": {"openai": {"itemId": "apc_0d5dfb28a009b1ee0169713022c3f88195a70b253d2a8cf798"}}}]}, {"role": "tool", "content": [{"type": "tool-result", "toolCallId": "call_INoksNAffcdh5UmRTWMLk1Ne", "toolName": "apply_patch", "output": {"type": "json", "value": {"status": "completed", "output": "Created index.html"}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": false, "hasApplyPatchTool": true})).await.unwrap();
                assert_eq!(
                    result.input,
                    json!([
                        {
                            "call_id": "call_INoksNAffcdh5UmRTWMLk1Ne",
                            "id": "apc_0d5dfb28a009b1ee0169713022c3f88195a70b253d2a8cf798",
                            "operation": {
                                "diff": "+<!doctype html>\n+<html></html>",
                                "path": "index.html",
                                "type": "create_file",
                            },
                            "status": "completed",
                            "type": "apply_patch_call",
                        },
                        {
                            "call_id": "call_INoksNAffcdh5UmRTWMLk1Ne",
                            "output": "Created index.html",
                            "status": "completed",
                            "type": "apply_patch_call_output",
                        },
                    ])
                );
            }

            /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › provider-defined tools › apply_patch › should convert apply_patch tool call with update_file operation with store: false
            #[tokio::test]
            async fn should_convert_apply_patch_tool_call_with_update_file_operation_with_store_false()
             {
                let result = convert(json!({"prompt": [{"role": "assistant", "content": [{"type": "tool-call", "toolCallId": "call_UpdateFile123", "toolName": "apply_patch", "input": {"callId": "call_UpdateFile123", "operation": {"type": "update_file", "path": "src/app.ts", "diff": "-old line\n+new line"}}, "providerOptions": {"openai": {"itemId": "apc_update_file_item_id"}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": false, "hasApplyPatchTool": true})).await.unwrap();
                assert_eq!(
                    result.input,
                    json!([
                        {
                            "call_id": "call_UpdateFile123",
                            "id": "apc_update_file_item_id",
                            "operation": {
                                "diff": "-old line\n+new line",
                                "path": "src/app.ts",
                                "type": "update_file",
                            },
                            "status": "completed",
                            "type": "apply_patch_call",
                        },
                    ])
                );
            }

            /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › provider-defined tools › apply_patch › should convert apply_patch tool call with delete_file operation with store: false
            #[tokio::test]
            async fn should_convert_apply_patch_tool_call_with_delete_file_operation_with_store_false()
             {
                let result = convert(json!({"prompt": [{"role": "assistant", "content": [{"type": "tool-call", "toolCallId": "call_DeleteFile456", "toolName": "apply_patch", "input": {"callId": "call_DeleteFile456", "operation": {"type": "delete_file", "path": "temp.txt"}}, "providerOptions": {"openai": {"itemId": "apc_delete_file_item_id"}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": false, "hasApplyPatchTool": true})).await.unwrap();
                assert_eq!(
                    result.input,
                    json!([
                        {
                            "call_id": "call_DeleteFile456",
                            "id": "apc_delete_file_item_id",
                            "operation": {
                                "path": "temp.txt",
                                "type": "delete_file",
                            },
                            "status": "completed",
                            "type": "apply_patch_call",
                        },
                    ])
                );
            }
        }
    }

    mod provider_tool_outputs {
        use super::*;

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › provider tool outputs › should include apply_patch output when multiple tool results are present
        #[tokio::test]
        async fn should_include_apply_patch_output_when_multiple_tool_results_are_present() {
            let result = convert(json!({"prompt": [{"role": "tool", "content": [{"type": "tool-result", "toolCallId": "call-shell", "toolName": "shell", "output": {"type": "json", "value": {"output": [{"stdout": "hi\n", "stderr": "", "outcome": {"type": "exit", "exitCode": 0}}]}}}, {"type": "tool-result", "toolCallId": "call-apply", "toolName": "apply_patch", "output": {"type": "json", "value": {"status": "completed", "output": "patched"}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true, "hasShellTool": true, "hasApplyPatchTool": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([{"type": "shell_call_output", "call_id": "call-shell", "output": [{"stdout": "hi\n", "stderr": "", "outcome": {"type": "exit", "exit_code": 0}}]}, {"type": "apply_patch_call_output", "call_id": "call-apply", "status": "completed", "output": "patched"}])
            );
        }
    }

    mod function_tools {
        use super::*;

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › function tools › should include client-side tool calls in prompt
        #[tokio::test]
        async fn should_include_client_side_tool_calls_in_prompt() {
            let result = convert(json!({"prompt": [{"role": "assistant", "content": [{"type": "tool-call", "toolCallId": "call-1", "toolName": "calculator", "input": {"a": 1, "b": 2}, "providerExecuted": false}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap();
            assert_eq!(
                result.to_json(),
                json!({
                    "input": [
                        {
                            "arguments": r#"{"a":1,"b":2}"#,
                            "call_id": "call-1",
                            "name": "calculator",
                            "type": "function_call",
                        },
                    ],
                    "warnings": [],
                })
            );
        }
    }

    mod mcp_tool_approval_responses {
        use super::*;

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › MCP tool approval responses › should not reference an MCP approval request from a previous response
        #[tokio::test]
        async fn should_not_reference_an_mcp_approval_request_from_a_previous_response() {
            let result = convert(json!({"prompt": [{"role": "tool", "content": [{"type": "tool-approval-response", "approvalId": "mcp-approval-previous-response", "approved": true}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true, "hasPreviousResponseId": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([{"type": "mcp_approval_response", "approval_request_id": "mcp-approval-previous-response", "approve": true}])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › MCP tool approval responses › should not reference an MCP approval request from a conversation
        #[tokio::test]
        async fn should_not_reference_an_mcp_approval_request_from_a_conversation() {
            let result = convert(json!({"prompt": [{"role": "tool", "content": [{"type": "tool-approval-response", "approvalId": "mcp-approval-conversation", "approved": false}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true, "hasConversation": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([{"type": "mcp_approval_response", "approval_request_id": "mcp-approval-conversation", "approve": false}])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › MCP tool approval responses › should convert approved tool-approval-response to mcp_approval_response with store: true
        #[tokio::test]
        async fn should_convert_approved_tool_approval_response_to_mcp_approval_response_with_sto()
        {
            let result = convert(json!({"prompt": [{"role": "tool", "content": [{"type": "tool-approval-response", "approvalId": "mcp-approval-123", "approved": true}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([
                    {
                        "id": "mcp-approval-123",
                        "type": "item_reference",
                    },
                    {
                        "approval_request_id": "mcp-approval-123",
                        "approve": true,
                        "type": "mcp_approval_response",
                    },
                ])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › MCP tool approval responses › should convert denied tool-approval-response to mcp_approval_response with store: true
        #[tokio::test]
        async fn should_convert_denied_tool_approval_response_to_mcp_approval_response_with_store()
        {
            let result = convert(json!({"prompt": [{"role": "tool", "content": [{"type": "tool-approval-response", "approvalId": "mcp-approval-456", "approved": false}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([
                    {
                        "id": "mcp-approval-456",
                        "type": "item_reference",
                    },
                    {
                        "approval_request_id": "mcp-approval-456",
                        "approve": false,
                        "type": "mcp_approval_response",
                    },
                ])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › MCP tool approval responses › should convert tool-approval-response to mcp_approval_response without item_reference when store: false
        #[tokio::test]
        async fn should_convert_tool_approval_response_to_mcp_approval_response_without_item_refe()
        {
            let result = convert(json!({"prompt": [{"role": "tool", "content": [{"type": "tool-approval-response", "approvalId": "mcp-approval-789", "approved": true}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": false})).await.unwrap();
            assert_eq!(
                result.input,
                json!([
                    {
                        "approval_request_id": "mcp-approval-789",
                        "approve": true,
                        "type": "mcp_approval_response",
                    },
                ])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › MCP tool approval responses › should skip duplicate tool-approval-response with same approvalId
        #[tokio::test]
        async fn should_skip_duplicate_tool_approval_response_with_same_approval_id() {
            let result = convert(json!({"prompt": [{"role": "tool", "content": [{"type": "tool-approval-response", "approvalId": "duplicate-approval", "approved": true}, {"type": "tool-approval-response", "approvalId": "duplicate-approval", "approved": true}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([
                    {
                        "id": "duplicate-approval",
                        "type": "item_reference",
                    },
                    {
                        "approval_request_id": "duplicate-approval",
                        "approve": true,
                        "type": "mcp_approval_response",
                    },
                ])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › MCP tool approval responses › should handle multiple different tool-approval-responses
        #[tokio::test]
        async fn should_handle_multiple_different_tool_approval_responses() {
            let result = convert(json!({"prompt": [{"role": "tool", "content": [{"type": "tool-approval-response", "approvalId": "approval-1", "approved": true}, {"type": "tool-approval-response", "approvalId": "approval-2", "approved": false}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([
                    {
                        "id": "approval-1",
                        "type": "item_reference",
                    },
                    {
                        "approval_request_id": "approval-1",
                        "approve": true,
                        "type": "mcp_approval_response",
                    },
                    {
                        "id": "approval-2",
                        "type": "item_reference",
                    },
                    {
                        "approval_request_id": "approval-2",
                        "approve": false,
                        "type": "mcp_approval_response",
                    },
                ])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › MCP tool approval responses › should skip execution-denied output when it has approvalId in providerOptions
        #[tokio::test]
        async fn should_skip_execution_denied_output_when_it_has_approval_id_in_provider_options() {
            let result = convert(json!({"prompt": [{"role": "tool", "content": [{"type": "tool-approval-response", "approvalId": "denied-approval", "approved": false}, {"type": "tool-result", "toolCallId": "call-123", "toolName": "mcp_tool", "output": {"type": "execution-denied", "reason": "User denied the tool execution", "providerOptions": {"openai": {"approvalId": "denied-approval"}}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([
                    {
                        "id": "denied-approval",
                        "type": "item_reference",
                    },
                    {
                        "approval_request_id": "denied-approval",
                        "approve": false,
                        "type": "mcp_approval_response",
                    },
                ])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › MCP tool approval responses › should handle tool-approval-response mixed with regular tool results
        #[tokio::test]
        async fn should_handle_tool_approval_response_mixed_with_regular_tool_results() {
            let result = convert(json!({"prompt": [{"role": "tool", "content": [{"type": "tool-approval-response", "approvalId": "approval-for-mcp", "approved": true}, {"type": "tool-result", "toolCallId": "regular-call-1", "toolName": "calculator", "output": {"type": "json", "value": {"result": 42}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([
                    {
                        "id": "approval-for-mcp",
                        "type": "item_reference",
                    },
                    {
                        "approval_request_id": "approval-for-mcp",
                        "approve": true,
                        "type": "mcp_approval_response",
                    },
                    {
                        "call_id": "regular-call-1",
                        "output": r#"{"result":42}"#,
                        "type": "function_call_output",
                    },
                ])
            );
        }
    }

    mod has_conversation {
        use super::*;

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › hasConversation › should skip assistant text messages with item IDs when hasConversation is true
        #[tokio::test]
        async fn should_skip_assistant_text_messages_with_item_ids_when_has_conversation_is_true() {
            let result = convert(json!({"prompt": [{"role": "user", "content": [{"type": "text", "text": "Hello"}]}, {"role": "assistant", "content": [{"type": "text", "text": "Hi there!", "providerOptions": {"openai": {"itemId": "msg_existing_123"}}}]}, {"role": "user", "content": [{"type": "text", "text": "What is the weather?"}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true, "hasConversation": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([
                    {
                        "content": [
                            {
                                "text": "Hello",
                                "type": "input_text",
                            },
                        ],
                        "role": "user",
                    },
                    {
                        "content": [
                            {
                                "text": "What is the weather?",
                                "type": "input_text",
                            },
                        ],
                        "role": "user",
                    },
                ])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › hasConversation › should skip assistant tool-call messages with item IDs when hasConversation is true
        #[tokio::test]
        async fn should_skip_assistant_tool_call_messages_with_item_ids_when_has_conversation_is() {
            let result = convert(json!({"prompt": [{"role": "user", "content": [{"type": "text", "text": "What is the weather?"}]}, {"role": "assistant", "content": [{"type": "tool-call", "toolCallId": "call_123", "toolName": "getWeather", "input": {"location": "San Francisco"}, "providerOptions": {"openai": {"itemId": "fc_existing_456"}}}]}, {"role": "tool", "content": [{"type": "tool-result", "toolCallId": "call_123", "toolName": "getWeather", "output": {"type": "json", "value": {"temp": 72}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true, "hasConversation": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([
                    {
                        "content": [
                            {
                                "text": "What is the weather?",
                                "type": "input_text",
                            },
                        ],
                        "role": "user",
                    },
                    {
                        "call_id": "call_123",
                        "output": r#"{"temp":72}"#,
                        "type": "function_call_output",
                    },
                ])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › hasConversation › should include assistant messages without item IDs when hasConversation is true
        #[tokio::test]
        async fn should_include_assistant_messages_without_item_ids_when_has_conversation_is_tru() {
            let result = convert(json!({"prompt": [{"role": "user", "content": [{"type": "text", "text": "Hello"}]}, {"role": "assistant", "content": [{"type": "text", "text": "Hi there!"}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true, "hasConversation": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([
                    {
                        "content": [
                            {
                                "text": "Hello",
                                "type": "input_text",
                            },
                        ],
                        "role": "user",
                    },
                    {
                        "content": "Hi there!",
                        "role": "assistant",
                    },
                ])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › hasConversation › should include assistant messages with item IDs when hasConversation is false
        #[tokio::test]
        async fn should_include_assistant_messages_with_item_ids_when_has_conversation_is_false() {
            let result = convert(json!({"prompt": [{"role": "user", "content": [{"type": "text", "text": "Hello"}]}, {"role": "assistant", "content": [{"type": "text", "text": "Hi there!", "providerOptions": {"openai": {"itemId": "msg_existing_123"}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true, "hasConversation": false})).await.unwrap();
            assert_eq!(
                result.input,
                json!([
                    {
                        "content": [
                            {
                                "text": "Hello",
                                "type": "input_text",
                            },
                        ],
                        "role": "user",
                    },
                    {
                        "id": "msg_existing_123",
                        "type": "item_reference",
                    },
                ])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › hasConversation › should skip reasoning parts with item IDs when hasConversation is true
        #[tokio::test]
        async fn should_skip_reasoning_parts_with_item_ids_when_has_conversation_is_true() {
            let result = convert(json!({"prompt": [{"role": "user", "content": [{"type": "text", "text": "Hello"}]}, {"role": "assistant", "content": [{"type": "reasoning", "text": "Let me think...", "providerOptions": {"openai": {"itemId": "reasoning_existing_789"}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true, "hasConversation": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([
                    {
                        "content": [
                            {
                                "text": "Hello",
                                "type": "input_text",
                            },
                        ],
                        "role": "user",
                    },
                ])
            );
        }
    }

    mod has_previous_response_id {
        use super::*;

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › hasPreviousResponseId › should keep client-executed function calls paired with their outputs when hasPreviousResponseId is true
        #[tokio::test]
        async fn should_keep_client_executed_function_calls_paired_with_their_outputs_when_has_pr()
        {
            let result = convert(json!({"prompt": [{"role": "user", "content": [{"type": "text", "text": "Hello"}]}, {"role": "assistant", "content": [{"type": "text", "text": "Hi there!", "providerOptions": {"openai": {"itemId": "msg_existing_123"}}}, {"type": "tool-call", "toolCallId": "call_123", "toolName": "getWeather", "input": {"location": "San Francisco"}, "providerOptions": {"openai": {"itemId": "fc_existing_456"}}}]}, {"role": "tool", "content": [{"type": "tool-result", "toolCallId": "call_123", "toolName": "getWeather", "output": {"type": "json", "value": {"temp": 72}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true, "hasPreviousResponseId": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([
                    {
                        "content": [
                            {
                                "text": "Hello",
                                "type": "input_text",
                            },
                        ],
                        "role": "user",
                    },
                    {
                        "id": "msg_existing_123",
                        "type": "item_reference",
                    },
                    {
                        "arguments": r#"{"location":"San Francisco"}"#,
                        "call_id": "call_123",
                        "name": "getWeather",
                        "type": "function_call",
                    },
                    {
                        "call_id": "call_123",
                        "output": r#"{"temp":72}"#,
                        "type": "function_call_output",
                    },
                ])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › hasPreviousResponseId › should skip reasoning parts with item IDs when hasPreviousResponseId is true
        #[tokio::test]
        async fn should_skip_reasoning_parts_with_item_ids_when_has_previous_response_id_is_true() {
            let result = convert(json!({"prompt": [{"role": "user", "content": [{"type": "text", "text": "Hello"}]}, {"role": "assistant", "content": [{"type": "reasoning", "text": "Let me think...", "providerOptions": {"openai": {"itemId": "rs_existing_789"}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true, "hasPreviousResponseId": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([
                    {
                        "content": [
                            {
                                "text": "Hello",
                                "type": "input_text",
                            },
                        ],
                        "role": "user",
                    },
                ])
            );
        }
    }

    mod compaction {
        use super::*;

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › compaction › should convert compaction to item_reference when store is true
        #[tokio::test]
        async fn should_convert_compaction_to_item_reference_when_store_is_true() {
            let result = convert(json!({"prompt": [{"role": "assistant", "content": [{"type": "custom", "kind": "openai.compaction", "providerOptions": {"openai": {"type": "compaction", "itemId": "cmp_123", "encryptedContent": "encrypted_data_here"}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([
                    {
                        "id": "cmp_123",
                        "type": "item_reference",
                    },
                ])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › compaction › should convert compaction to full compaction item when store is false
        #[tokio::test]
        async fn should_convert_compaction_to_full_compaction_item_when_store_is_false() {
            let result = convert(json!({"prompt": [{"role": "assistant", "content": [{"type": "custom", "kind": "openai.compaction", "providerOptions": {"openai": {"type": "compaction", "itemId": "cmp_456", "encryptedContent": "encrypted_compaction_state"}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": false})).await.unwrap();
            assert_eq!(
                result.input,
                json!([
                    {
                        "encrypted_content": "encrypted_compaction_state",
                        "id": "cmp_456",
                        "type": "compaction",
                    },
                ])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › compaction › should skip compaction when hasConversation is true
        #[tokio::test]
        async fn should_skip_compaction_when_has_conversation_is_true() {
            let result = convert(json!({"prompt": [{"role": "user", "content": [{"type": "text", "text": "Hello"}]}, {"role": "assistant", "content": [{"type": "custom", "kind": "openai.compaction", "providerOptions": {"openai": {"type": "compaction", "itemId": "cmp_789", "encryptedContent": "encrypted_data"}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true, "hasConversation": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([
                    {
                        "content": [
                            {
                                "text": "Hello",
                                "type": "input_text",
                            },
                        ],
                        "role": "user",
                    },
                ])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › compaction › should handle compaction alongside regular text content
        #[tokio::test]
        async fn should_handle_compaction_alongside_regular_text_content() {
            let result = convert(json!({"prompt": [{"role": "assistant", "content": [{"type": "text", "text": "Here is my response.", "providerOptions": {"openai": {"itemId": "msg_001"}}}, {"type": "custom", "kind": "openai.compaction", "providerOptions": {"openai": {"type": "compaction", "itemId": "cmp_001", "encryptedContent": "encrypted_state"}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": false})).await.unwrap();
            assert_eq!(
                result.input,
                json!([
                    {
                        "content": "Here is my response.",
                        "role": "assistant",
                    },
                    {
                        "encrypted_content": "encrypted_state",
                        "id": "cmp_001",
                        "type": "compaction",
                    },
                ])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › compaction › should handle compaction with store: true alongside other content
        #[tokio::test]
        async fn should_handle_compaction_with_store_true_alongside_other_content() {
            let result = convert(json!({"prompt": [{"role": "assistant", "content": [{"type": "text", "text": "Response text", "providerOptions": {"openai": {"itemId": "msg_002"}}}, {"type": "custom", "kind": "openai.compaction", "providerOptions": {"openai": {"type": "compaction", "itemId": "cmp_002", "encryptedContent": "encrypted_data"}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([
                    {
                        "id": "msg_002",
                        "type": "item_reference",
                    },
                    {
                        "id": "cmp_002",
                        "type": "item_reference",
                    },
                ])
            );
        }
    }

    mod custom_tool_calls {
        use super::*;

        fn custom_provider_tool_names() -> Value {
            json!(["write_sql"])
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › custom tool calls › should convert custom tool call to custom_tool_call input item
        #[tokio::test]
        async fn should_convert_custom_tool_call_to_custom_tool_call_input_item() {
            let result = convert(json!({"prompt": [{"role": "assistant", "content": [{"type": "tool-call", "toolCallId": "call_custom_001", "toolName": "write_sql", "input": "SELECT * FROM users WHERE age > 25"}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true, "customProviderToolNames": custom_provider_tool_names()})).await.unwrap();
            assert_eq!(
                result.input,
                json!([
                    {
                        "call_id": "call_custom_001",
                        "input": "SELECT * FROM users WHERE age > 25",
                        "name": "write_sql",
                        "type": "custom_tool_call",
                    },
                ])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › custom tool calls › should JSON.stringify non-string custom tool call input
        #[tokio::test]
        async fn should_json_stringify_non_string_custom_tool_call_input() {
            let result = convert(json!({"prompt": [{"role": "assistant", "content": [{"type": "tool-call", "toolCallId": "call_custom_002", "toolName": "write_sql", "input": {"query": "test"}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true, "customProviderToolNames": custom_provider_tool_names()})).await.unwrap();
            assert_eq!(
                result.input,
                json!([
                    {
                        "call_id": "call_custom_002",
                        "input": r#"{"query":"test"}"#,
                        "name": "write_sql",
                        "type": "custom_tool_call",
                    },
                ])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › custom tool calls › should convert custom tool call with itemId to item_reference when store: true
        #[tokio::test]
        async fn should_convert_custom_tool_call_with_item_id_to_item_reference_when_store_true() {
            let result = convert(json!({"prompt": [{"role": "assistant", "content": [{"type": "tool-call", "toolCallId": "call_custom_003", "toolName": "write_sql", "input": "SELECT 1", "providerOptions": {"openai": {"itemId": "ct_ref_123"}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true, "customProviderToolNames": custom_provider_tool_names()})).await.unwrap();
            assert_eq!(
                result.input,
                json!([
                    {
                        "id": "ct_ref_123",
                        "type": "item_reference",
                    },
                ])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › custom tool calls › should preserve prompt cache breakpoints on scalar custom tool results
        #[tokio::test]
        async fn should_preserve_prompt_cache_breakpoints_on_scalar_custom_tool_results() {
            let prompt_cache_breakpoint = json!({"mode": "explicit"});
            let provider_options =
                json!({"openai": {"promptCacheBreakpoint": prompt_cache_breakpoint}});
            let scalar_outputs = json!([{"output": {"type": "text", "value": "stable tool output"}, "expectedText": "stable tool output"}, {"output": {"type": "json", "value": {"stable": true}}, "expectedText": r#"{"stable":true}"#}, {"output": {"type": "error-text", "value": "tool error"}, "expectedText": "tool error"}, {"output": {"type": "error-json", "value": {"error": "boom"}}, "expectedText": r#"{"error":"boom"}"#}, {"output": {"type": "execution-denied", "reason": "execution denied"}, "expectedText": "execution denied"}]);
            let mut tool_results = Vec::new();
            for (output_index, entry) in scalar_outputs.as_array().unwrap().iter().enumerate() {
                for placement in ["output", "tool-result"] {
                    let mut part = json!({
                        "type": "tool-result",
                        "toolCallId": format!("call_custom_{output_index}_{placement}"),
                        "toolName": "write_sql",
                        "output": entry["output"],
                    });
                    if placement == "output" {
                        part["output"]["providerOptions"] = provider_options.clone();
                    } else {
                        part["providerOptions"] = provider_options.clone();
                    }
                    tool_results.push(part);
                }
            }
            let result = convert(json!({"prompt": [{"role": "tool", "content": tool_results}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true, "customProviderToolNames": custom_provider_tool_names()})).await.unwrap();
            let mut expected = Vec::new();
            for (output_index, entry) in scalar_outputs.as_array().unwrap().iter().enumerate() {
                for placement in ["output", "tool-result"] {
                    expected.push(json!({
                        "type": "custom_tool_call_output",
                        "call_id": format!("call_custom_{output_index}_{placement}"),
                        "output": [{
                            "type": "input_text",
                            "text": entry["expectedText"],
                            "prompt_cache_breakpoint": prompt_cache_breakpoint,
                        }],
                    }));
                }
            }
            assert_eq!(result.input, json!(expected));
            assert_eq!(result.warnings, json!([]));
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › custom tool calls › should convert custom tool result to custom_tool_call_output with text value
        #[tokio::test]
        async fn should_convert_custom_tool_result_to_custom_tool_call_output_with_text_value() {
            let result = convert(json!({"prompt": [{"role": "tool", "content": [{"type": "tool-result", "toolCallId": "call_custom_001", "toolName": "write_sql", "output": {"type": "text", "value": "Query executed successfully. 42 rows returned."}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true, "customProviderToolNames": custom_provider_tool_names()})).await.unwrap();
            assert_eq!(
                result.input,
                json!([
                    {
                        "call_id": "call_custom_001",
                        "output": "Query executed successfully. 42 rows returned.",
                        "type": "custom_tool_call_output",
                    },
                ])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › custom tool calls › should convert custom tool result to custom_tool_call_output with json value
        #[tokio::test]
        async fn should_convert_custom_tool_result_to_custom_tool_call_output_with_json_value() {
            let result = convert(json!({"prompt": [{"role": "tool", "content": [{"type": "tool-result", "toolCallId": "call_custom_002", "toolName": "write_sql", "output": {"type": "json", "value": {"rows": 42, "status": "ok"}}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true, "customProviderToolNames": custom_provider_tool_names()})).await.unwrap();
            assert_eq!(
                result.input,
                json!([
                    {
                        "call_id": "call_custom_002",
                        "output": r#"{"rows":42,"status":"ok"}"#,
                        "type": "custom_tool_call_output",
                    },
                ])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › custom tool calls › should convert execution-denied custom tool result to custom_tool_call_output
        #[tokio::test]
        async fn should_convert_execution_denied_custom_tool_result_to_custom_tool_call_output() {
            let result = convert(json!({"prompt": [{"role": "tool", "content": [{"type": "tool-result", "toolCallId": "call_custom_denied_001", "toolName": "write_sql", "output": {"type": "execution-denied", "reason": "User denied the tool execution"}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true, "customProviderToolNames": custom_provider_tool_names()})).await.unwrap();
            assert_eq!(
                result.input,
                json!([{"type": "custom_tool_call_output", "call_id": "call_custom_denied_001", "output": "User denied the tool execution"}])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › custom tool calls › should convert custom tool result content output
        #[tokio::test]
        async fn should_convert_custom_tool_result_content_output() {
            let result = convert(json!({"prompt": [{"role": "tool", "content": [{"type": "tool-result", "toolCallId": "call_custom_005", "toolName": "write_sql", "output": {"type": "content", "value": [{"type": "text", "text": "hello"}]}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true, "customProviderToolNames": custom_provider_tool_names()})).await.unwrap();
            assert_eq!(
                result.input,
                json!([
                    {
                        "call_id": "call_custom_005",
                        "output": [
                            {
                                "text": "hello",
                                "type": "input_text",
                            },
                        ],
                        "type": "custom_tool_call_output",
                    },
                ])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › custom tool calls › should convert custom tool result content output with file-url
        #[tokio::test]
        async fn should_convert_custom_tool_result_content_output_with_file_url() {
            let result = convert(json!({"prompt": [{"role": "tool", "content": [{"type": "tool-result", "toolCallId": "call_custom_006", "toolName": "write_sql", "output": {"type": "content", "value": [{"type": "text", "text": "Here is the file:"}, {"type": "file", "data": {"type": "url", "url": "https://example.com/test.pdf"}, "mediaType": "application/pdf"}]}}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true, "customProviderToolNames": custom_provider_tool_names()})).await.unwrap();
            assert_eq!(
                result.input,
                json!([
                    {
                        "call_id": "call_custom_006",
                        "output": [
                            {
                                "text": "Here is the file:",
                                "type": "input_text",
                            },
                            {
                                "file_url": "https://example.com/test.pdf",
                                "type": "input_file",
                            },
                        ],
                        "type": "custom_tool_call_output",
                    },
                ])
            );
            assert_eq!(result.warnings, json!([]));
        }

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › custom tool calls › should not emit custom_tool_call when customProviderToolNames is not provided
        #[tokio::test]
        async fn should_not_emit_custom_tool_call_when_custom_provider_tool_names_is_not_provided()
        {
            let result = convert(json!({"prompt": [{"role": "assistant", "content": [{"type": "tool-call", "toolCallId": "call_custom_001", "toolName": "write_sql", "input": "SELECT 1"}]}], "systemMessageMode": "system", "providerOptionsName": "openai", "store": true})).await.unwrap();
            assert_eq!(
                result.input,
                json!([
                    {
                        "arguments": r#""SELECT 1""#,
                        "call_id": "call_custom_001",
                        "name": "write_sql",
                        "type": "function_call",
                    },
                ])
            );
        }
    }

    mod programmatic_tool_calling {
        use super::*;

        /// upstream: @ai-sdk/openai src/responses/convert-to-openai-responses-input.test.ts › convertToOpenAIResponsesInput › programmatic tool calling › should replay program items and preserve nested function caller linkage
        #[tokio::test]
        async fn should_replay_program_items_and_preserve_nested_function_caller_linkage() {
            let result = convert(json!({"prompt": [{"role": "assistant", "content": [{"type": "tool-call", "toolCallId": "program_call_1", "toolName": "program", "input": {"code": r#"const value = await tools.get_inventory({ sku: "A" });"#, "fingerprint": "fingerprint_1"}, "providerExecuted": true, "providerOptions": {"openai": {"itemId": "program_item_1"}}}, {"type": "tool-call", "toolCallId": "function_call_1", "toolName": "get_inventory", "input": {"sku": "A"}, "providerOptions": {"openai": {"itemId": "function_item_1", "caller": {"type": "program", "callerId": "program_call_1"}}}}]}, {"role": "tool", "content": [{"type": "tool-result", "toolCallId": "function_call_1", "toolName": "get_inventory", "output": {"type": "json", "value": {"sku": "A", "availableUnits": 42}}, "providerOptions": {"openai": {"caller": {"type": "program", "callerId": "program_call_1"}}}}]}, {"role": "assistant", "content": [{"type": "tool-result", "toolCallId": "program_call_1", "toolName": "program", "output": {"type": "json", "value": {"result": r#"{"availableUnits":42}"#, "status": "completed"}}, "providerOptions": {"openai": {"itemId": "program_output_item_1"}}}]}], "toolNameMapping": {"program": "programmatic_tool_calling"}, "systemMessageMode": "system", "providerOptionsName": "openai", "store": false})).await.unwrap();
            assert_eq!(
                result.input,
                json!([{"type": "program", "id": "program_item_1", "call_id": "program_call_1", "code": r#"const value = await tools.get_inventory({ sku: "A" });"#, "fingerprint": "fingerprint_1"}, {"type": "function_call", "call_id": "function_call_1", "name": "get_inventory", "arguments": r#"{"sku":"A"}"#, "caller": {"type": "program", "caller_id": "program_call_1"}}, {"type": "function_call_output", "call_id": "function_call_1", "output": r#"{"sku":"A","availableUnits":42}"#, "caller": {"type": "program", "caller_id": "program_call_1"}}, {"type": "program_output", "id": "program_output_item_1", "call_id": "program_call_1", "result": r#"{"availableUnits":42}"#, "status": "completed"}])
            );
        }
    }
}

// ── Older ports (previously the inline tests of `src/openai/responses/convert.rs`) ──
mod older_responses_convert {

    use crate::openai::language_model_capabilities::SystemMessageMode;
    use crate::openai::responses::ResponsesNamespace;
    use crate::openai::responses::convert_to_responses_input::*;
    use aimux_core::language_model_message::AssistantPart;
    use aimux_core::language_model_message::LanguageModelMessage;
    use aimux_core::language_model_message::ReasoningPart;
    use aimux_core::types::Warning;
    use serde_json::Value;
    use serde_json::json;

    fn reasoning(text: &str, options: Value) -> AssistantPart {
        AssistantPart::Reasoning(ReasoningPart {
            text: text.to_string(),
            provider_options: Some(serde_json::from_value(json!({ "openai": options })).unwrap()),
        })
    }

    fn convert(store: bool, content: Vec<AssistantPart>) -> ResponsesInputResult {
        let prompt = vec![LanguageModelMessage::Assistant {
            content,
            provider_options: None,
        }];
        convert_to_responses_input(
            ResponsesNamespace::OPENAI,
            &prompt,
            SystemMessageMode::System,
            store,
            false,
        )
        .unwrap()
    }

    /// partial: `should merge consecutive parts with same reasoning ID`; full port in src/openai/tests/responses/convert_to_responses_input.rs.
    /// Parts of one reasoning item become one input item: the summaries are
    /// appended and the last encrypted content wins (openai@4.0.83
    /// `convert-to-openai-responses-input.ts:1167-1220`).
    #[test]
    fn reasoning_parts_with_one_item_id_are_merged_without_store() {
        let result = convert(
            false,
            vec![
                reasoning(
                    "first",
                    json!({ "itemId": "rs_1", "reasoningEncryptedContent": "enc-1" }),
                ),
                reasoning(
                    "second",
                    json!({ "itemId": "rs_1", "reasoningEncryptedContent": "enc-2" }),
                ),
                reasoning(
                    "other",
                    json!({ "itemId": "rs_2", "reasoningEncryptedContent": "enc-3" }),
                ),
            ],
        );
        assert_eq!(
            result.input,
            vec![
                json!({
                    "type": "reasoning",
                    "id": "rs_1",
                    "encrypted_content": "enc-2",
                    "summary": [
                        { "type": "summary_text", "text": "first" },
                        { "type": "summary_text", "text": "second" },
                    ],
                }),
                json!({
                    "type": "reasoning",
                    "id": "rs_2",
                    "encrypted_content": "enc-3",
                    "summary": [{ "type": "summary_text", "text": "other" }],
                }),
            ]
        );
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
    }

    /// partial: `should warn when appending empty text to existing sequence`; full port in src/openai/tests/responses/convert_to_responses_input.rs.
    /// An empty part cannot extend an item that already has parts.
    #[test]
    fn an_empty_reasoning_part_is_not_appended_to_an_item() {
        let result = convert(
            false,
            vec![
                reasoning(
                    "first",
                    json!({ "itemId": "rs_1", "reasoningEncryptedContent": "enc-1" }),
                ),
                reasoning("", json!({ "itemId": "rs_1" })),
            ],
        );
        assert_eq!(
            result.input,
            vec![json!({
                "type": "reasoning",
                "id": "rs_1",
                "encrypted_content": "enc-1",
                "summary": [{ "type": "summary_text", "text": "first" }],
            })]
        );
        assert!(matches!(
            result.warnings.as_slice(),
            [Warning::Other { message }]
                if message.starts_with("Cannot append empty reasoning part to existing reasoning sequence. Skipping reasoning part: ")
        ));
    }

    /// With `store`, one item reference stands for every part of the item.
    #[test]
    fn reasoning_parts_with_one_item_id_are_one_reference_with_store() {
        let result = convert(
            true,
            vec![
                reasoning("first", json!({ "itemId": "rs_1" })),
                reasoning("second", json!({ "itemId": "rs_1" })),
                reasoning("other", json!({ "itemId": "rs_2" })),
            ],
        );
        assert_eq!(
            result.input,
            vec![
                json!({ "type": "item_reference", "id": "rs_1" }),
                json!({ "type": "item_reference", "id": "rs_2" }),
            ]
        );
    }
}
