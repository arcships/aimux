//! `convertToGoogleMessages` (`convert-to-google-messages.test.ts`).
//!
//! The upstream converter takes options; aimux splits it:
//! - `validate_call_options_for_namespace` raises the errors upstream throws
//!   while converting (it runs before the conversion in `GoogleModel`);
//! - `convert_to_google_messages_for_namespace(prompt, namespace,
//!   supports_function_response_parts)` is the conversion
//!   (`providerOptionsNames: ['googleVertex', 'vertex']` is
//!   `Namespace::Vertex`, `supportsFunctionResponseParts` the flag);
//! - `isGemmaModel` and `isGemini3Model` (sentinel injection and its warning)
//!   live in the request-body builder, keyed by the model id, so those cases
//!   build the body for a Gemma / Gemini 3 / Gemini 2.5 model id and read
//!   `contents` / `systemInstruction` and the warnings from it.

use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::options::CallOptions;

use crate::google::convert_to_messages::convert_to_google_messages_for_namespace;
use crate::google::language_model::{
    build_request_body_with_warnings_for_namespace, validate_call_options_for_namespace,
};
use crate::google::options::Namespace;

const SKIP_THOUGHT_SIGNATURE_VALIDATOR: &str = "skip_thought_signature_validator";

/// A Gemma model id (`isGemmaModel: true`).
const GEMMA_MODEL: &str = "gemma-3-27b-it";
/// A Gemini 3 model id (`isGemini3Model: true`).
const GEMINI_3_MODEL: &str = "gemini-3-pro-preview";
/// A pre-Gemini-3 model id (`isGemini3Model: false`).
const GEMINI_2_5_MODEL: &str = "gemini-2.5-flash";

fn call_options(prompt: Value) -> CallOptions {
    CallOptions::new(serde_json::from_value(prompt).expect("prompt deserializes"))
}

fn try_convert_with(
    prompt: Value,
    namespace: Namespace,
    supports_function_response_parts: bool,
) -> Result<Value, AiMuxError> {
    let options = call_options(prompt);
    validate_call_options_for_namespace(&options, namespace)?;
    let result = convert_to_google_messages_for_namespace(
        &options.prompt,
        namespace,
        supports_function_response_parts,
    );
    let mut value = json!({ "contents": result.contents });
    if let Some(system_instruction) = result.system_instruction {
        value["systemInstruction"] = system_instruction;
    }
    Ok(value)
}

/// `convertToGoogleMessages(prompt)` with the default options.
fn convert(prompt: Value) -> Value {
    try_convert_with(prompt, Namespace::Google, true).expect("conversion succeeds")
}

/// `providerOptionsNames: ['googleVertex', 'vertex']`.
fn convert_vertex(prompt: Value) -> Value {
    try_convert_with(prompt, Namespace::Vertex, true).expect("conversion succeeds")
}

/// Upstream's expected value without `functionCall.id`. The Rust conversion
/// for the Vertex namespace is the one the Vertex model runs, which passes
/// `includeFunctionCallIds: !isVertexProvider` (google-language-model.ts:329);
/// the direct upstream call keeps the ids because that option defaults to
/// `true`, a combination no aimux caller can reach.
fn without_function_call_ids(mut expected: Value) -> Value {
    fn strip(value: &mut Value) {
        match value {
            Value::Object(map) => {
                if let Some(Value::Object(call)) = map.get_mut("functionCall") {
                    call.remove("id");
                }
                map.values_mut().for_each(strip);
            }
            Value::Array(items) => items.iter_mut().for_each(strip),
            _ => {}
        }
    }
    strip(&mut expected);
    expected
}

/// `supportsFunctionResponseParts: false`.
fn convert_legacy(prompt: Value) -> Value {
    try_convert_with(prompt, Namespace::Google, false).expect("conversion succeeds")
}

/// The message of the error the conversion throws.
fn convert_error(prompt: Value) -> String {
    try_convert_with(prompt, Namespace::Google, true)
        .expect_err("conversion fails")
        .to_string()
}

/// The conversion as the request-body builder runs it for `model_id`: the
/// `contents` / `systemInstruction` and the warnings (the `onWarning` calls).
fn convert_for_model(model_id: &str, namespace: Namespace, prompt: Value) -> (Value, Vec<Value>) {
    let options = call_options(prompt);
    validate_call_options_for_namespace(&options, namespace).expect("valid prompt");
    let (body, warnings) =
        build_request_body_with_warnings_for_namespace(model_id, &options, namespace)
            .expect("body builds");
    let mut value = json!({ "contents": body["contents"] });
    if let Some(system_instruction) = body.get("systemInstruction") {
        value["systemInstruction"] = system_instruction.clone();
    }
    let warnings = warnings
        .into_iter()
        .map(|warning| serde_json::to_value(warning).unwrap())
        .collect();
    (value, warnings)
}

/// `expect(actual).toMatchObject(expected)`: every expected key is present
/// and matches, recursively; arrays match element-wise.
fn assert_match_object(actual: &Value, expected: &Value) {
    fn matches(actual: &Value, expected: &Value) -> bool {
        match (actual, expected) {
            (Value::Object(actual), Value::Object(expected)) => expected
                .iter()
                .all(|(key, value)| actual.get(key).is_some_and(|a| matches(a, value))),
            (Value::Array(actual), Value::Array(expected)) => {
                actual.len() == expected.len()
                    && actual.iter().zip(expected).all(|(a, e)| matches(a, e))
            }
            _ => actual == expected,
        }
    }
    assert!(
        matches(actual, expected),
        "expected {actual:#} to match {expected:#}"
    );
}

mod system_messages {
    use super::*;

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › system messages › should store system message in system instruction
    #[test]
    fn should_store_system_message_in_system_instruction() {
        let result = convert(json!([{ "role": "system", "content": "Test" }]));

        assert_eq!(
            result,
            json!({
                "systemInstruction": { "parts": [{ "text": "Test" }] },
                "contents": [],
            })
        );
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › system messages › should throw error when there was already a user message
    #[test]
    fn should_throw_error_when_there_was_already_a_user_message() {
        let message = convert_error(json!([
            { "role": "user", "content": [{ "type": "text", "text": "Test" }] },
            { "role": "system", "content": "Test" },
        ]));

        assert!(
            message.contains(
                "system messages are only supported at the beginning of the conversation"
            ),
            "{message}"
        );
    }
}

mod thought_signatures {
    use super::*;

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › thought signatures › should preserve thought signatures in assistant messages
    #[test]
    fn should_preserve_thought_signatures_in_assistant_messages() {
        let result = convert(json!([
            {
                "role": "assistant",
                "content": [
                    {
                        "type": "text",
                        "text": "Regular text",
                        "providerOptions": { "google": { "thoughtSignature": "sig1" } },
                    },
                    {
                        "type": "reasoning",
                        "text": "Reasoning text",
                        "providerOptions": { "google": { "thoughtSignature": "sig2" } },
                    },
                    {
                        "type": "tool-call",
                        "toolCallId": "call1",
                        "toolName": "test",
                        "input": { "value": "test" },
                        "providerOptions": { "google": { "thoughtSignature": "sig3" } },
                    },
                ],
            },
        ]));

        assert_eq!(
            result,
            json!({
                "contents": [
                    {
                        "parts": [
                            { "text": "Regular text", "thoughtSignature": "sig1" },
                            {
                                "text": "Reasoning text",
                                "thought": true,
                                "thoughtSignature": "sig2",
                            },
                            {
                                "functionCall": {
                                    "args": { "value": "test" },
                                    "id": "call1",
                                    "name": "test",
                                },
                                "thoughtSignature": "sig3",
                            },
                        ],
                        "role": "model",
                    },
                ],
            })
        );
    }
}

mod thought_signatures_with_vertex_provider_options_name {
    use super::*;

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › thought signatures with vertex providerOptionsName › should resolve thoughtSignature from google namespace when using vertex providerOptionsName
    #[test]
    fn should_resolve_thought_signature_from_google_namespace_when_using_vertex_name() {
        let result = convert_vertex(json!([
            {
                "role": "assistant",
                "content": [
                    {
                        "type": "text",
                        "text": "Regular text",
                        "providerOptions": { "google": { "thoughtSignature": "sig1" } },
                    },
                    {
                        "type": "reasoning",
                        "text": "Reasoning text",
                        "providerOptions": { "google": { "thoughtSignature": "sig2" } },
                    },
                    {
                        "type": "tool-call",
                        "toolCallId": "call1",
                        "toolName": "getWeather",
                        "input": { "location": "London" },
                        "providerOptions": { "google": { "thoughtSignature": "sig3" } },
                    },
                ],
            },
        ]));

        assert_eq!(
            result,
            without_function_call_ids(json!({
                "contents": [
                    {
                        "parts": [
                            { "text": "Regular text", "thoughtSignature": "sig1" },
                            {
                                "text": "Reasoning text",
                                "thought": true,
                                "thoughtSignature": "sig2",
                            },
                            {
                                "functionCall": {
                                    "args": { "location": "London" },
                                    "id": "call1",
                                    "name": "getWeather",
                                },
                                "thoughtSignature": "sig3",
                            },
                        ],
                        "role": "model",
                    },
                ],
            }))
        );
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › thought signatures with vertex providerOptionsName › should prefer vertex namespace over google namespace when both are present
    #[test]
    fn should_prefer_vertex_namespace_over_google_namespace_when_both_are_present() {
        let result = convert_vertex(json!([
            {
                "role": "assistant",
                "content": [
                    {
                        "type": "tool-call",
                        "toolCallId": "call1",
                        "toolName": "getWeather",
                        "input": { "location": "London" },
                        "providerOptions": {
                            "vertex": { "thoughtSignature": "vertex_sig" },
                            "google": { "thoughtSignature": "google_sig" },
                        },
                    },
                ],
            },
        ]));

        assert_eq!(
            result["contents"][0]["parts"][0],
            without_function_call_ids(json!({
                "functionCall": {
                    "id": "call1",
                    "name": "getWeather",
                    "args": { "location": "London" },
                },
                "thoughtSignature": "vertex_sig",
            }))
        );
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › thought signatures with vertex providerOptionsName › should resolve thoughtSignature from vertex namespace directly
    #[test]
    fn should_resolve_thought_signature_from_vertex_namespace_directly() {
        let result = convert_vertex(json!([
            {
                "role": "assistant",
                "content": [
                    {
                        "type": "tool-call",
                        "toolCallId": "call1",
                        "toolName": "getWeather",
                        "input": { "location": "London" },
                        "providerOptions": { "vertex": { "thoughtSignature": "vertex_sig" } },
                    },
                ],
            },
        ]));

        assert_eq!(
            result["contents"][0]["parts"][0],
            without_function_call_ids(json!({
                "functionCall": {
                    "id": "call1",
                    "name": "getWeather",
                    "args": { "location": "London" },
                },
                "thoughtSignature": "vertex_sig",
            }))
        );
    }
}

mod thought_signatures_with_google_provider_options_name_gateway_failover {
    use super::*;

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › thought signatures with google providerOptionsName (gateway failover) › should resolve thoughtSignature from vertex namespace when using google providerOptionsName
    #[test]
    fn should_resolve_thought_signature_from_vertex_namespace_when_using_google_name() {
        let result = convert(json!([
            {
                "role": "assistant",
                "content": [
                    {
                        "type": "text",
                        "text": "Regular text",
                        "providerOptions": { "vertex": { "thoughtSignature": "sig1" } },
                    },
                    {
                        "type": "reasoning",
                        "text": "Reasoning text",
                        "providerOptions": { "vertex": { "thoughtSignature": "sig2" } },
                    },
                    {
                        "type": "tool-call",
                        "toolCallId": "call1",
                        "toolName": "getWeather",
                        "input": { "location": "London" },
                        "providerOptions": { "vertex": { "thoughtSignature": "sig3" } },
                    },
                ],
            },
        ]));

        assert_eq!(
            result,
            json!({
                "contents": [
                    {
                        "parts": [
                            { "text": "Regular text", "thoughtSignature": "sig1" },
                            {
                                "text": "Reasoning text",
                                "thought": true,
                                "thoughtSignature": "sig2",
                            },
                            {
                                "functionCall": {
                                    "args": { "location": "London" },
                                    "id": "call1",
                                    "name": "getWeather",
                                },
                                "thoughtSignature": "sig3",
                            },
                        ],
                        "role": "model",
                    },
                ],
            })
        );
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › thought signatures with google providerOptionsName (gateway failover) › should prefer google namespace over vertex namespace when both are present
    #[test]
    fn should_prefer_google_namespace_over_vertex_namespace_when_both_are_present() {
        let result = convert(json!([
            {
                "role": "assistant",
                "content": [
                    {
                        "type": "tool-call",
                        "toolCallId": "call1",
                        "toolName": "getWeather",
                        "input": { "location": "London" },
                        "providerOptions": {
                            "google": { "thoughtSignature": "google_sig" },
                            "vertex": { "thoughtSignature": "vertex_sig" },
                        },
                    },
                ],
            },
        ]));

        assert_eq!(
            result["contents"][0]["parts"][0],
            json!({
                "functionCall": {
                    "id": "call1",
                    "name": "getWeather",
                    "args": { "location": "London" },
                },
                "thoughtSignature": "google_sig",
            })
        );
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › thought signatures with google providerOptionsName (gateway failover) › should resolve thoughtSignature from vertex namespace when google namespace is absent (default providerOptionsName)
    #[test]
    fn should_resolve_thought_signature_from_vertex_namespace_when_google_is_absent() {
        let result = convert(json!([
            {
                "role": "assistant",
                "content": [
                    {
                        "type": "tool-call",
                        "toolCallId": "call1",
                        "toolName": "getWeather",
                        "input": { "location": "London" },
                        "providerOptions": { "vertex": { "thoughtSignature": "vertex_sig" } },
                    },
                ],
            },
        ]));

        assert_eq!(
            result["contents"][0]["parts"][0],
            json!({
                "functionCall": {
                    "id": "call1",
                    "name": "getWeather",
                    "args": { "location": "London" },
                },
                "thoughtSignature": "vertex_sig",
            })
        );
    }
}

mod gemma_model_system_instructions {
    use super::*;

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › Gemma model system instructions › should prepend system instruction to first user message for Gemma models
    #[test]
    fn should_prepend_system_instruction_to_first_user_message_for_gemma_models() {
        let (result, _) = convert_for_model(
            GEMMA_MODEL,
            Namespace::Google,
            json!([
                { "role": "system", "content": "You are a helpful assistant." },
                { "role": "user", "content": [{ "type": "text", "text": "Hello" }] },
            ]),
        );

        assert_eq!(
            result,
            json!({
                "contents": [
                    {
                        "parts": [
                            { "text": "You are a helpful assistant.\n\n" },
                            { "text": "Hello" },
                        ],
                        "role": "user",
                    },
                ],
            })
        );
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › Gemma model system instructions › should handle multiple system messages for Gemma models
    #[test]
    fn should_handle_multiple_system_messages_for_gemma_models() {
        let (result, _) = convert_for_model(
            GEMMA_MODEL,
            Namespace::Google,
            json!([
                { "role": "system", "content": "You are helpful." },
                { "role": "system", "content": "Be concise." },
                { "role": "user", "content": [{ "type": "text", "text": "Hi" }] },
            ]),
        );

        assert_eq!(
            result,
            json!({
                "contents": [
                    {
                        "parts": [
                            { "text": "You are helpful.\n\nBe concise.\n\n" },
                            { "text": "Hi" },
                        ],
                        "role": "user",
                    },
                ],
            })
        );
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › Gemma model system instructions › should not affect non-Gemma models
    #[test]
    fn should_not_affect_non_gemma_models() {
        let result = convert(json!([
            { "role": "system", "content": "You are helpful." },
            { "role": "user", "content": [{ "type": "text", "text": "Hello" }] },
        ]));

        assert_eq!(
            result,
            json!({
                "contents": [
                    { "parts": [{ "text": "Hello" }], "role": "user" },
                ],
                "systemInstruction": { "parts": [{ "text": "You are helpful." }] },
            })
        );
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › Gemma model system instructions › should handle Gemma model with system instruction but no user messages
    #[test]
    fn should_handle_gemma_model_with_system_instruction_but_no_user_messages() {
        let (result, _) = convert_for_model(
            GEMMA_MODEL,
            Namespace::Google,
            json!([{ "role": "system", "content": "You are helpful." }]),
        );

        assert_eq!(result, json!({ "contents": [] }));
    }
}

mod user_messages {
    use super::*;

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › user messages › should preserve original Google Cloud Storage file URIs
    #[test]
    fn should_preserve_original_google_cloud_storage_file_uris() {
        // `new URL('gs://my-bucket/folder/My File.pdf').toString()`.
        let result = convert(json!([
            {
                "role": "user",
                "content": [
                    {
                        "type": "file",
                        "data": {
                            "type": "url",
                            "url": "gs://my-bucket/folder/My%20File.pdf",
                            "originalUrl": "gs://my-bucket/folder/My File.pdf",
                        },
                        "mediaType": "application/pdf",
                    },
                ],
            },
        ]));

        assert_eq!(
            result,
            json!({
                "contents": [
                    {
                        "role": "user",
                        "parts": [
                            {
                                "fileData": {
                                    "mimeType": "application/pdf",
                                    "fileUri": "gs://my-bucket/folder/My File.pdf",
                                },
                            },
                        ],
                    },
                ],
            })
        );
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › user messages › should add image parts
    #[test]
    fn should_add_image_parts() {
        let result = convert(json!([
            {
                "role": "user",
                "content": [
                    {
                        "type": "file",
                        "data": { "type": "data", "data": "AAECAw==" },
                        "mediaType": "image/png",
                    },
                ],
            },
        ]));

        assert_eq!(
            result,
            json!({
                "contents": [
                    {
                        "role": "user",
                        "parts": [
                            { "inlineData": { "data": "AAECAw==", "mimeType": "image/png" } },
                        ],
                    },
                ],
            })
        );
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › user messages › should add file parts for base64 encoded files
    #[test]
    fn should_add_file_parts_for_base64_encoded_files() {
        let result = convert(json!([
            {
                "role": "user",
                "content": [
                    {
                        "type": "file",
                        "data": { "type": "data", "data": "AAECAw==" },
                        "mediaType": "image/png",
                    },
                ],
            },
        ]));

        assert_eq!(
            result,
            json!({
                "contents": [
                    {
                        "role": "user",
                        "parts": [
                            { "inlineData": { "data": "AAECAw==", "mimeType": "image/png" } },
                        ],
                    },
                ],
            })
        );
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › user messages › should convert file parts with provider reference to fileData
    #[test]
    fn should_convert_file_parts_with_provider_reference_to_file_data() {
        let result = convert(json!([
            {
                "role": "user",
                "content": [
                    {
                        "type": "file",
                        "data": {
                            "type": "reference",
                            "reference": {
                                "google": "https://generativelanguage.googleapis.com/v1beta/files/abc123",
                                "openai": "file-xyz789",
                            },
                        },
                        "mediaType": "image/png",
                    },
                ],
            },
        ]));

        assert_eq!(
            result,
            json!({
                "contents": [
                    {
                        "role": "user",
                        "parts": [
                            {
                                "fileData": {
                                    "mimeType": "image/png",
                                    "fileUri": "https://generativelanguage.googleapis.com/v1beta/files/abc123",
                                },
                            },
                        ],
                    },
                ],
            })
        );
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › user messages › should convert image file parts with provider reference to fileData
    #[test]
    fn should_convert_image_file_parts_with_provider_reference_to_file_data() {
        let result = convert(json!([
            {
                "role": "user",
                "content": [
                    {
                        "type": "file",
                        "data": {
                            "type": "reference",
                            "reference": {
                                "google": "https://generativelanguage.googleapis.com/v1beta/files/img456",
                            },
                        },
                        "mediaType": "image/jpeg",
                    },
                ],
            },
        ]));

        assert_eq!(
            result,
            json!({
                "contents": [
                    {
                        "role": "user",
                        "parts": [
                            {
                                "fileData": {
                                    "mimeType": "image/jpeg",
                                    "fileUri": "https://generativelanguage.googleapis.com/v1beta/files/img456",
                                },
                            },
                        ],
                    },
                ],
            })
        );
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › user messages › should throw when provider reference is missing google key in user file part
    #[test]
    fn should_throw_when_provider_reference_is_missing_google_key_in_user_file_part() {
        let message = convert_error(json!([
            {
                "role": "user",
                "content": [
                    {
                        "type": "file",
                        "data": { "type": "reference", "reference": { "openai": "file-xyz789" } },
                        "mediaType": "image/png",
                    },
                ],
            },
        ]));

        assert!(
            message.contains(
                "No provider reference found for provider 'google'. Available providers: openai"
            ),
            "{message}"
        );
    }
}

mod tool_messages {
    use super::*;

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › tool messages › should convert tool result messages to function responses
    #[test]
    fn should_convert_tool_result_messages_to_function_responses() {
        let result = convert(json!([
            {
                "role": "tool",
                "content": [
                    {
                        "type": "tool-result",
                        "toolName": "testFunction",
                        "toolCallId": "testCallId",
                        "output": { "type": "json", "value": { "someData": "test result" } },
                    },
                ],
            },
        ]));

        assert_eq!(
            result,
            json!({
                "contents": [
                    {
                        "role": "user",
                        "parts": [
                            {
                                "functionResponse": {
                                    "id": "testCallId",
                                    "name": "testFunction",
                                    "response": {
                                        "name": "testFunction",
                                        "content": { "someData": "test result" },
                                    },
                                },
                            },
                        ],
                    },
                ],
            })
        );
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › tool messages › should serialize JSON Schema references in function response content
    #[test]
    fn should_serialize_json_schema_references_in_function_response_content() {
        let tool_result = json!({
            "tools": [
                {
                    "name": "find_records",
                    "inputSchema": {
                        "$defs": {
                            "Node": {
                                "type": "object",
                                "properties": { "child": { "$ref": "#/$defs/Node" } },
                            },
                        },
                        "$ref": "#/$defs/Node",
                    },
                },
            ],
        });

        let result = convert(json!([
            {
                "role": "tool",
                "content": [
                    {
                        "type": "tool-result",
                        "toolName": "get_schema",
                        "toolCallId": "testCallId",
                        "output": { "type": "json", "value": tool_result },
                    },
                ],
            },
        ]));

        // `JSON.stringify(toolResult)`.
        assert_eq!(
            result["contents"][0]["parts"][0],
            json!({
                "functionResponse": {
                    "id": "testCallId",
                    "name": "get_schema",
                    "response": {
                        "name": "get_schema",
                        "content": serde_json::to_string(&tool_result).unwrap(),
                    },
                },
            })
        );
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › tool messages › should convert tool result content with image-data into functionResponse parts
    #[test]
    fn should_convert_tool_result_content_with_image_data_into_function_response_parts() {
        let result = convert(json!([
            {
                "role": "tool",
                "content": [
                    {
                        "type": "tool-result",
                        "toolName": "imageGenerator",
                        "toolCallId": "testCallId",
                        "output": {
                            "type": "content",
                            "value": [
                                { "type": "text", "text": "Here is the generated image:" },
                                {
                                    "type": "file",
                                    "data": { "type": "data", "data": "base64encodedimagedata" },
                                    "mediaType": "image/jpeg",
                                },
                            ],
                        },
                    },
                ],
            },
        ]));

        assert_eq!(
            result,
            json!({
                "contents": [
                    {
                        "role": "user",
                        "parts": [
                            {
                                "functionResponse": {
                                    "id": "testCallId",
                                    "name": "imageGenerator",
                                    "response": {
                                        "name": "imageGenerator",
                                        "content": "Here is the generated image:",
                                    },
                                    "parts": [
                                        {
                                            "inlineData": {
                                                "mimeType": "image/jpeg",
                                                "data": "base64encodedimagedata",
                                            },
                                        },
                                    ],
                                },
                            },
                        ],
                    },
                ],
            })
        );
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › tool messages › should convert tool result content with file-data into functionResponse parts
    #[test]
    fn should_convert_tool_result_content_with_file_data_into_function_response_parts() {
        let result = convert(json!([
            {
                "role": "tool",
                "content": [
                    {
                        "type": "tool-result",
                        "toolName": "documentReader",
                        "toolCallId": "testCallId",
                        "output": {
                            "type": "content",
                            "value": [
                                {
                                    "type": "file",
                                    "data": { "type": "data", "data": "base64pdfdata" },
                                    "mediaType": "application/pdf",
                                    "filename": "report.pdf",
                                },
                            ],
                        },
                    },
                ],
            },
        ]));

        assert_eq!(
            result["contents"][0]["parts"][0],
            json!({
                "functionResponse": {
                    "id": "testCallId",
                    "name": "documentReader",
                    "response": {
                        "name": "documentReader",
                        "content": "Tool executed successfully.",
                    },
                    "parts": [
                        { "inlineData": { "mimeType": "application/pdf", "data": "base64pdfdata" } },
                    ],
                },
            })
        );
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › tool messages › should convert tool result content with image-url data URL into functionResponse parts
    #[test]
    fn should_convert_tool_result_content_with_image_url_data_url_into_function_response_parts() {
        let result = convert(json!([
            {
                "role": "tool",
                "content": [
                    {
                        "type": "tool-result",
                        "toolName": "imageGenerator",
                        "toolCallId": "testCallId",
                        "output": {
                            "type": "content",
                            "value": [
                                {
                                    "type": "file",
                                    "data": {
                                        "type": "url",
                                        "url": "data:image/png;base64,base64pngdata",
                                    },
                                    "mediaType": "image/png",
                                },
                            ],
                        },
                    },
                ],
            },
        ]));

        assert_eq!(
            result["contents"][0]["parts"][0],
            json!({
                "functionResponse": {
                    "id": "testCallId",
                    "name": "imageGenerator",
                    "response": {
                        "name": "imageGenerator",
                        "content": "Tool executed successfully.",
                    },
                    "parts": [
                        { "inlineData": { "mimeType": "image/png", "data": "base64pngdata" } },
                    ],
                },
            })
        );
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › tool messages › should derive the full media type from a tool result data URL
    #[test]
    fn should_derive_the_full_media_type_from_a_tool_result_data_url() {
        let result = convert(json!([
            {
                "role": "tool",
                "content": [
                    {
                        "type": "tool-result",
                        "toolName": "imageGenerator",
                        "toolCallId": "testCallId",
                        "output": {
                            "type": "content",
                            "value": [
                                {
                                    "type": "file",
                                    "data": {
                                        "type": "url",
                                        "url": "data:image/png;base64,base64pngdata",
                                    },
                                    "mediaType": "image",
                                },
                            ],
                        },
                    },
                ],
            },
        ]));

        assert_eq!(
            result["contents"][0]["parts"][0],
            json!({
                "functionResponse": {
                    "id": "testCallId",
                    "name": "imageGenerator",
                    "response": {
                        "name": "imageGenerator",
                        "content": "Tool executed successfully.",
                    },
                    "parts": [
                        { "inlineData": { "mimeType": "image/png", "data": "base64pngdata" } },
                    ],
                },
            })
        );
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › tool messages › should forward non-data image-url tool result parts as text content
    #[test]
    fn should_forward_non_data_image_url_tool_result_parts_as_text_content() {
        let result = convert(json!([
            {
                "role": "tool",
                "content": [
                    {
                        "type": "tool-result",
                        "toolName": "imageGenerator",
                        "toolCallId": "testCallId",
                        "output": {
                            "type": "content",
                            "value": [
                                {
                                    "type": "file",
                                    "data": { "type": "url", "url": "https://example.com/image.png" },
                                    "mediaType": "image/png",
                                },
                            ],
                        },
                    },
                ],
            },
        ]));

        assert_eq!(
            result["contents"][0]["parts"][0],
            json!({
                "functionResponse": {
                    "id": "testCallId",
                    "name": "imageGenerator",
                    "response": {
                        "name": "imageGenerator",
                        "content": r#"{"type":"file","data":{"type":"url","url":"https://example.com/image.png"},"mediaType":"image/png"}"#,
                    },
                },
            })
        );
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › tool messages › should forward unsupported non-data URLs with top-level media types as text content
    #[test]
    fn should_forward_unsupported_non_data_urls_with_top_level_media_types_as_text() {
        let result = convert(json!([
            {
                "role": "tool",
                "content": [
                    {
                        "type": "tool-result",
                        "toolName": "imageGenerator",
                        "toolCallId": "testCallId",
                        "output": {
                            "type": "content",
                            "value": [
                                {
                                    "type": "file",
                                    "data": { "type": "url", "url": "https://example.com/image.png" },
                                    "mediaType": "image",
                                },
                            ],
                        },
                    },
                ],
            },
        ]));

        assert_eq!(
            result["contents"][0]["parts"][0],
            json!({
                "functionResponse": {
                    "id": "testCallId",
                    "name": "imageGenerator",
                    "response": {
                        "name": "imageGenerator",
                        "content": r#"{"type":"file","data":{"type":"url","url":"https://example.com/image.png"},"mediaType":"image"}"#,
                    },
                },
            })
        );
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › tool messages › should forward non-data file-url tool result parts as text content
    #[test]
    fn should_forward_non_data_file_url_tool_result_parts_as_text_content() {
        let result = convert(json!([
            {
                "role": "tool",
                "content": [
                    {
                        "type": "tool-result",
                        "toolName": "documentReader",
                        "toolCallId": "testCallId",
                        "output": {
                            "type": "content",
                            "value": [
                                {
                                    "type": "file",
                                    "data": { "type": "url", "url": "https://example.com/report.pdf" },
                                    "mediaType": "application/pdf",
                                },
                            ],
                        },
                    },
                ],
            },
        ]));

        assert_eq!(
            result["contents"][0]["parts"][0],
            json!({
                "functionResponse": {
                    "id": "testCallId",
                    "name": "documentReader",
                    "response": {
                        "name": "documentReader",
                        "content": r#"{"type":"file","data":{"type":"url","url":"https://example.com/report.pdf"},"mediaType":"application/pdf"}"#,
                    },
                },
            })
        );
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › tool messages › should use legacy tool-result conversion when functionResponse parts are unsupported
    #[test]
    fn should_use_legacy_tool_result_conversion_when_function_response_parts_are_unsupported() {
        let result = convert_legacy(json!([
            {
                "role": "tool",
                "content": [
                    {
                        "type": "tool-result",
                        "toolName": "imageGenerator",
                        "toolCallId": "testCallId",
                        "output": {
                            "type": "content",
                            "value": [
                                { "type": "text", "text": "Here is the generated image:" },
                                {
                                    "type": "file",
                                    "data": { "type": "data", "data": "base64encodedimagedata" },
                                    "mediaType": "image/jpeg",
                                },
                                {
                                    "type": "file",
                                    "data": { "type": "data", "data": "base64pdfdata" },
                                    "mediaType": "application/pdf",
                                    "filename": "report.pdf",
                                },
                            ],
                        },
                    },
                ],
            },
        ]));

        assert_eq!(
            result["contents"][0]["parts"],
            json!([
                {
                    "functionResponse": {
                        "id": "testCallId",
                        "name": "imageGenerator",
                        "response": {
                            "name": "imageGenerator",
                            "content": "Here is the generated image:",
                        },
                    },
                },
                { "inlineData": { "mimeType": "image/jpeg", "data": "base64encodedimagedata" } },
                { "text": "Tool executed successfully and returned this image as a response" },
                { "inlineData": { "mimeType": "application/pdf", "data": "base64pdfdata" } },
                { "text": "Tool executed successfully and returned this file as a response" },
            ])
        );
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › tool messages › issue #16072: should not serialize PDF file tool results as text on the non-Gemini-3 path
    #[test]
    fn issue_16072_should_not_serialize_pdf_file_tool_results_as_text_on_non_gemini_3_path() {
        let result = convert_legacy(json!([
            {
                "role": "tool",
                "content": [
                    {
                        "type": "tool-result",
                        "toolName": "catalogSearch",
                        "toolCallId": "testCallId",
                        "output": {
                            "type": "content",
                            "value": [
                                { "type": "text", "text": "metadata" },
                                {
                                    "type": "file",
                                    "data": { "type": "data", "data": "JVBERi0xLjQK" },
                                    "mediaType": "application/pdf",
                                },
                            ],
                        },
                    },
                ],
            },
        ]));

        let text_parts: Vec<&str> = result["contents"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|content| content["parts"].as_array().unwrap())
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect();

        assert!(
            !text_parts.iter().any(|text| text.contains("JVBERi0xLjQK")),
            "{text_parts:?}"
        );
        assert!(
            result["contents"][0]["parts"]
                .as_array()
                .unwrap()
                .contains(&json!({
                    "inlineData": { "mimeType": "application/pdf", "data": "JVBERi0xLjQK" },
                })),
            "{result:#}"
        );
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › tool messages › should keep URL tool result parts on the legacy path
    #[test]
    fn should_keep_url_tool_result_parts_on_the_legacy_path() {
        let result = convert_legacy(json!([
            {
                "role": "tool",
                "content": [
                    {
                        "type": "tool-result",
                        "toolName": "documentReader",
                        "toolCallId": "testCallId",
                        "output": {
                            "type": "content",
                            "value": [
                                {
                                    "type": "file",
                                    "data": { "type": "url", "url": "https://example.com/image.png" },
                                    "mediaType": "image/png",
                                },
                                {
                                    "type": "file",
                                    "data": { "type": "url", "url": "https://example.com/report.pdf" },
                                    "mediaType": "application/pdf",
                                },
                            ],
                        },
                    },
                ],
            },
        ]));

        assert_eq!(
            result["contents"][0]["parts"],
            json!([
                {
                    "text": r#"{"type":"file","data":{"type":"url","url":"https://example.com/image.png"},"mediaType":"image/png"}"#,
                },
                {
                    "text": r#"{"type":"file","data":{"type":"url","url":"https://example.com/report.pdf"},"mediaType":"application/pdf"}"#,
                },
            ])
        );
    }
}

mod assistant_messages {
    use super::*;

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › assistant messages › should add PNG image parts for base64 encoded files
    #[test]
    fn should_add_png_image_parts_for_base64_encoded_files() {
        let result = convert(json!([
            {
                "role": "assistant",
                "content": [
                    {
                        "type": "file",
                        "data": { "type": "data", "data": "AAECAw==" },
                        "mediaType": "image/png",
                    },
                ],
            },
        ]));

        assert_eq!(
            result,
            json!({
                "contents": [
                    {
                        "role": "model",
                        "parts": [
                            { "inlineData": { "data": "AAECAw==", "mimeType": "image/png" } },
                        ],
                    },
                ],
            })
        );
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › assistant messages › should include thought flag on file parts when set in providerOptions
    #[test]
    fn should_include_thought_flag_on_file_parts_when_set_in_provider_options() {
        let result = convert(json!([
            {
                "role": "assistant",
                "content": [
                    {
                        "type": "file",
                        "data": { "type": "data", "data": "AAECAw==" },
                        "mediaType": "image/png",
                        "providerOptions": {
                            "google": { "thought": true, "thoughtSignature": "sig1" },
                        },
                    },
                    {
                        "type": "file",
                        "data": { "type": "data", "data": "BAUG" },
                        "mediaType": "image/jpeg",
                    },
                ],
            },
        ]));

        assert_eq!(
            result,
            json!({
                "contents": [
                    {
                        "role": "model",
                        "parts": [
                            {
                                "inlineData": { "data": "AAECAw==", "mimeType": "image/png" },
                                "thought": true,
                                "thoughtSignature": "sig1",
                            },
                            { "inlineData": { "data": "BAUG", "mimeType": "image/jpeg" } },
                        ],
                    },
                ],
            })
        );
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › assistant messages › should convert reasoning-file parts with thought flag and signature
    #[test]
    fn should_convert_reasoning_file_parts_with_thought_flag_and_signature() {
        let result = convert(json!([
            {
                "role": "assistant",
                "content": [
                    {
                        "type": "reasoning-file",
                        "data": { "type": "data", "data": "AAECAw==" },
                        "mediaType": "image/png",
                        "providerOptions": {
                            "google": { "thoughtSignature": "sig_reasoning_file" },
                        },
                    },
                ],
            },
        ]));

        assert_eq!(
            result,
            json!({
                "contents": [
                    {
                        "parts": [
                            {
                                "inlineData": { "data": "AAECAw==", "mimeType": "image/png" },
                                "thought": true,
                                "thoughtSignature": "sig_reasoning_file",
                            },
                        ],
                        "role": "model",
                    },
                ],
            })
        );
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › assistant messages › should convert reasoning-file parts without thoughtSignature
    #[test]
    fn should_convert_reasoning_file_parts_without_thought_signature() {
        let result = convert(json!([
            {
                "role": "assistant",
                "content": [
                    {
                        "type": "reasoning-file",
                        "data": { "type": "data", "data": "BAUG" },
                        "mediaType": "image/jpeg",
                    },
                ],
            },
        ]));

        assert_eq!(
            result,
            json!({
                "contents": [
                    {
                        "parts": [
                            {
                                "inlineData": { "data": "BAUG", "mimeType": "image/jpeg" },
                                "thought": true,
                            },
                        ],
                        "role": "model",
                    },
                ],
            })
        );
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › assistant messages › should throw error for URL file data in reasoning-file assistant messages
    #[test]
    fn should_throw_error_for_url_file_data_in_reasoning_file_assistant_messages() {
        let message = convert_error(json!([
            {
                "role": "assistant",
                "content": [
                    {
                        "type": "reasoning-file",
                        "data": { "type": "url", "url": "https://example.com/image.png" },
                        "mediaType": "image/png",
                    },
                ],
            },
        ]));

        assert!(
            message.contains("File data URLs in assistant messages are not supported"),
            "{message}"
        );
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › assistant messages › should handle mixed reasoning, reasoning-file, text, and tool-call parts
    #[test]
    fn should_handle_mixed_reasoning_reasoning_file_text_and_tool_call_parts() {
        let result = convert(json!([
            {
                "role": "assistant",
                "content": [
                    {
                        "type": "reasoning",
                        "text": "Thinking about this...",
                        "providerOptions": { "google": { "thoughtSignature": "sig1" } },
                    },
                    {
                        "type": "reasoning-file",
                        "data": { "type": "data", "data": "AAECAw==" },
                        "mediaType": "image/png",
                        "providerOptions": { "google": { "thoughtSignature": "sig2" } },
                    },
                    {
                        "type": "text",
                        "text": "Here is my response",
                        "providerOptions": { "google": { "thoughtSignature": "sig3" } },
                    },
                ],
            },
        ]));

        assert_eq!(
            result,
            json!({
                "contents": [
                    {
                        "parts": [
                            {
                                "text": "Thinking about this...",
                                "thought": true,
                                "thoughtSignature": "sig1",
                            },
                            {
                                "inlineData": { "data": "AAECAw==", "mimeType": "image/png" },
                                "thought": true,
                                "thoughtSignature": "sig2",
                            },
                            { "text": "Here is my response", "thoughtSignature": "sig3" },
                        ],
                        "role": "model",
                    },
                ],
            })
        );
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › assistant messages › should throw error for URL file data in assistant messages
    #[test]
    fn should_throw_error_for_url_file_data_in_assistant_messages() {
        let message = convert_error(json!([
            {
                "role": "assistant",
                "content": [
                    {
                        "type": "file",
                        "data": { "type": "url", "url": "https://example.com/image.png" },
                        "mediaType": "image/png",
                    },
                ],
            },
        ]));

        assert!(
            message.contains("File data URLs in assistant messages are not supported"),
            "{message}"
        );
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › assistant messages › should convert assistant file parts with provider reference to fileData
    #[test]
    fn should_convert_assistant_file_parts_with_provider_reference_to_file_data() {
        let result = convert(json!([
            {
                "role": "assistant",
                "content": [
                    {
                        "type": "file",
                        "data": {
                            "type": "reference",
                            "reference": {
                                "google": "https://generativelanguage.googleapis.com/v1beta/files/abc123",
                            },
                        },
                        "mediaType": "image/png",
                    },
                ],
            },
        ]));

        assert_eq!(
            result,
            json!({
                "contents": [
                    {
                        "role": "model",
                        "parts": [
                            {
                                "fileData": {
                                    "mimeType": "image/png",
                                    "fileUri": "https://generativelanguage.googleapis.com/v1beta/files/abc123",
                                },
                            },
                        ],
                    },
                ],
            })
        );
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › assistant messages › should convert assistant file parts with provider reference and thought flag
    #[test]
    fn should_convert_assistant_file_parts_with_provider_reference_and_thought_flag() {
        let result = convert(json!([
            {
                "role": "assistant",
                "content": [
                    {
                        "type": "file",
                        "data": {
                            "type": "reference",
                            "reference": {
                                "google": "https://generativelanguage.googleapis.com/v1beta/files/abc123",
                            },
                        },
                        "mediaType": "image/png",
                        "providerOptions": {
                            "google": { "thought": true, "thoughtSignature": "sig1" },
                        },
                    },
                ],
            },
        ]));

        assert_eq!(
            result,
            json!({
                "contents": [
                    {
                        "role": "model",
                        "parts": [
                            {
                                "fileData": {
                                    "mimeType": "image/png",
                                    "fileUri": "https://generativelanguage.googleapis.com/v1beta/files/abc123",
                                },
                                "thought": true,
                                "thoughtSignature": "sig1",
                            },
                        ],
                    },
                ],
            })
        );
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › assistant messages › should throw when provider reference is missing google key in assistant file part
    #[test]
    fn should_throw_when_provider_reference_is_missing_google_key_in_assistant_file_part() {
        let message = convert_error(json!([
            {
                "role": "assistant",
                "content": [
                    {
                        "type": "file",
                        "data": { "type": "reference", "reference": { "openai": "file-xyz789" } },
                        "mediaType": "image/png",
                    },
                ],
            },
        ]));

        assert!(
            message.contains(
                "No provider reference found for provider 'google'. Available providers: openai"
            ),
            "{message}"
        );
    }
}

mod parallel_tool_calls {
    use super::*;

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › parallel tool calls › should include thought signature on functionCall when provided
    #[test]
    fn should_include_thought_signature_on_function_call_when_provided() {
        let result = convert(json!([
            {
                "role": "assistant",
                "content": [
                    {
                        "type": "tool-call",
                        "toolCallId": "call1",
                        "toolName": "checkweather",
                        "input": { "city": "paris" },
                        "providerOptions": { "google": { "thoughtSignature": "sig_parallel" } },
                    },
                    {
                        "type": "tool-call",
                        "toolCallId": "call2",
                        "toolName": "checkweather",
                        "input": { "city": "london" },
                    },
                ],
            },
        ]));

        assert_eq!(
            result["contents"][0]["parts"][0],
            json!({
                "functionCall": { "id": "call1", "args": { "city": "paris" }, "name": "checkweather" },
                "thoughtSignature": "sig_parallel",
            })
        );
        assert_eq!(
            result["contents"][0]["parts"][1],
            json!({
                "functionCall": { "id": "call2", "args": { "city": "london" }, "name": "checkweather" },
            })
        );
    }
}

mod tool_results_with_thought_signatures {
    use super::*;

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › tool results with thought signatures › should include thought signature on functionCall but not on functionResponse
    #[test]
    fn should_include_thought_signature_on_function_call_but_not_on_function_response() {
        let result = convert(json!([
            {
                "role": "assistant",
                "content": [
                    {
                        "type": "tool-call",
                        "toolCallId": "call1",
                        "toolName": "readdata",
                        "input": { "userId": "123" },
                        "providerOptions": { "google": { "thoughtSignature": "sig_original" } },
                    },
                ],
            },
            {
                "role": "tool",
                "content": [
                    {
                        "type": "tool-result",
                        "toolCallId": "call1",
                        "toolName": "readdata",
                        "output": { "type": "error-text", "value": "file not found" },
                        "providerOptions": { "google": { "thoughtSignature": "sig_original" } },
                    },
                ],
            },
        ]));

        assert_eq!(
            result["contents"][0]["parts"][0],
            json!({
                "functionCall": { "id": "call1", "args": { "userId": "123" }, "name": "readdata" },
                "thoughtSignature": "sig_original",
            })
        );
        assert_eq!(
            result["contents"][1]["parts"][0],
            json!({
                "functionResponse": {
                    "id": "call1",
                    "name": "readdata",
                    "response": { "content": "file not found", "name": "readdata" },
                },
            })
        );
        assert!(
            result["contents"][1]["parts"][0]
                .get("thoughtSignature")
                .is_none()
        );
    }
}

mod server_tool_combination_round_trip {
    use super::*;

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › server tool combination round-trip › should parse stringified code execution input
    #[test]
    fn should_parse_stringified_code_execution_input() {
        let input =
            serde_json::to_string(&json!({ "language": "PYTHON", "code": "print(17 * 19)" }))
                .unwrap();
        let result = convert(json!([
            {
                "role": "assistant",
                "content": [
                    {
                        "type": "tool-call",
                        "toolCallId": "code-call-1",
                        "toolName": "code_execution",
                        "input": input,
                        "providerExecuted": true,
                    },
                ],
            },
        ]));

        assert_eq!(
            result["contents"][0]["parts"][0],
            json!({ "executableCode": { "language": "PYTHON", "code": "print(17 * 19)" } })
        );
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › server tool combination round-trip › should preserve code execution parts alongside a function tool call
    #[test]
    fn should_preserve_code_execution_parts_alongside_a_function_tool_call() {
        let (result, _) = convert_for_model(
            GEMINI_3_MODEL,
            Namespace::Google,
            json!([
                {
                    "role": "assistant",
                    "content": [
                        {
                            "type": "tool-call",
                            "toolCallId": "code-call-1",
                            "toolName": "code_execution",
                            "input": { "language": "PYTHON", "code": "print(17 * 19)" },
                            "providerExecuted": true,
                        },
                        {
                            "type": "tool-call",
                            "toolCallId": "function-call-1",
                            "toolName": "listItems",
                            "input": {},
                            "providerOptions": {
                                "google": { "thoughtSignature": "function-signature" },
                            },
                        },
                        {
                            "type": "tool-result",
                            "toolCallId": "code-call-1",
                            "toolName": "code_execution",
                            "output": {
                                "type": "json",
                                "value": { "outcome": "OUTCOME_OK", "output": "323\n" },
                            },
                        },
                    ],
                },
                {
                    "role": "tool",
                    "content": [
                        {
                            "type": "tool-result",
                            "toolCallId": "function-call-1",
                            "toolName": "listItems",
                            "output": { "type": "json", "value": { "items": ["a", "b"] } },
                        },
                    ],
                },
            ]),
        );

        assert_eq!(
            result["contents"],
            json!([
                {
                    "role": "model",
                    "parts": [
                        { "executableCode": { "language": "PYTHON", "code": "print(17 * 19)" } },
                        {
                            "functionCall": { "id": "function-call-1", "name": "listItems", "args": {} },
                            "thoughtSignature": "function-signature",
                        },
                        { "codeExecutionResult": { "outcome": "OUTCOME_OK", "output": "323\n" } },
                    ],
                },
                {
                    "role": "user",
                    "parts": [
                        {
                            "functionResponse": {
                                "id": "function-call-1",
                                "name": "listItems",
                                "response": { "name": "listItems", "content": { "items": ["a", "b"] } },
                            },
                        },
                    ],
                },
            ])
        );
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › server tool combination round-trip › should convert assistant tool-call with serverToolCallId to toolCall wire format
    #[test]
    fn should_convert_assistant_tool_call_with_server_tool_call_id_to_tool_call_wire_format() {
        let input = serde_json::to_string(&json!({ "query": "test" })).unwrap();
        let result = convert(json!([
            {
                "role": "assistant",
                "content": [
                    {
                        "type": "tool-call",
                        "toolCallId": "tc-1",
                        "toolName": "server:GOOGLE_SEARCH_WEB",
                        "input": input,
                        "providerOptions": {
                            "google": {
                                "serverToolCallId": "server-id-1",
                                "serverToolType": "GOOGLE_SEARCH_WEB",
                                "thoughtSignature": "sig-abc",
                            },
                        },
                    },
                ],
            },
        ]));

        assert_eq!(
            result["contents"][0]["parts"][0],
            json!({
                "toolCall": {
                    "toolType": "GOOGLE_SEARCH_WEB",
                    "args": { "query": "test" },
                    "id": "server-id-1",
                },
                "thoughtSignature": "sig-abc",
            })
        );
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › server tool combination round-trip › should convert assistant tool-call without serverToolCallId to functionCall wire format
    #[test]
    fn should_convert_assistant_tool_call_without_server_tool_call_id_to_function_call() {
        let result = convert(json!([
            {
                "role": "assistant",
                "content": [
                    {
                        "type": "tool-call",
                        "toolCallId": "tc-1",
                        "toolName": "weather",
                        "input": { "location": "SF" },
                    },
                ],
            },
        ]));

        assert_eq!(
            result["contents"][0]["parts"][0],
            json!({
                "functionCall": { "id": "tc-1", "name": "weather", "args": { "location": "SF" } },
            })
        );
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › server tool combination round-trip › should convert tool result with serverToolCallId to toolResponse on last model content
    #[test]
    fn should_convert_tool_result_with_server_tool_call_id_to_tool_response_on_last_model() {
        let input = serde_json::to_string(&json!({ "query": "test" })).unwrap();
        let result = convert(json!([
            {
                "role": "assistant",
                "content": [
                    {
                        "type": "tool-call",
                        "toolCallId": "tc-1",
                        "toolName": "server:GOOGLE_SEARCH_WEB",
                        "input": input,
                        "providerOptions": {
                            "google": {
                                "serverToolCallId": "server-id-1",
                                "serverToolType": "GOOGLE_SEARCH_WEB",
                            },
                        },
                    },
                ],
            },
            {
                "role": "tool",
                "content": [
                    {
                        "type": "tool-result",
                        "toolCallId": "tc-1",
                        "toolName": "server:GOOGLE_SEARCH_WEB",
                        "output": { "type": "json", "value": { "results": ["a"] } },
                        "providerOptions": {
                            "google": {
                                "serverToolCallId": "server-id-1",
                                "serverToolType": "GOOGLE_SEARCH_WEB",
                                "thoughtSignature": "sig-resp",
                            },
                        },
                    },
                ],
            },
        ]));

        assert_eq!(result["contents"][0]["role"], "model");
        assert_eq!(result["contents"][0]["parts"].as_array().unwrap().len(), 2);
        assert_eq!(
            result["contents"][0]["parts"][0],
            json!({
                "toolCall": {
                    "toolType": "GOOGLE_SEARCH_WEB",
                    "args": { "query": "test" },
                    "id": "server-id-1",
                },
            })
        );
        assert_eq!(
            result["contents"][0]["parts"][1],
            json!({
                "toolResponse": {
                    "toolType": "GOOGLE_SEARCH_WEB",
                    "response": { "results": ["a"] },
                    "id": "server-id-1",
                },
                "thoughtSignature": "sig-resp",
            })
        );
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › server tool combination round-trip › should parse string input for server tool call args
    #[test]
    fn should_parse_string_input_for_server_tool_call_args() {
        let result = convert(json!([
            {
                "role": "assistant",
                "content": [
                    {
                        "type": "tool-call",
                        "toolCallId": "tc-1",
                        "toolName": "server:GOOGLE_SEARCH_WEB",
                        "input": r#"{"query":"hello"}"#,
                        "providerOptions": {
                            "google": {
                                "serverToolCallId": "sid-1",
                                "serverToolType": "GOOGLE_SEARCH_WEB",
                            },
                        },
                    },
                ],
            },
        ]));

        assert_eq!(
            result["contents"][0]["parts"][0],
            json!({
                "toolCall": {
                    "toolType": "GOOGLE_SEARCH_WEB",
                    "args": { "query": "hello" },
                    "id": "sid-1",
                },
            })
        );
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › server tool combination round-trip › should pass object input directly for server tool call args
    #[test]
    fn should_pass_object_input_directly_for_server_tool_call_args() {
        let result = convert(json!([
            {
                "role": "assistant",
                "content": [
                    {
                        "type": "tool-call",
                        "toolCallId": "tc-1",
                        "toolName": "server:GOOGLE_SEARCH_WEB",
                        "input": { "query": "hello" },
                        "providerOptions": {
                            "google": {
                                "serverToolCallId": "sid-1",
                                "serverToolType": "GOOGLE_SEARCH_WEB",
                            },
                        },
                    },
                ],
            },
        ]));

        assert_eq!(
            result["contents"][0]["parts"][0],
            json!({
                "toolCall": {
                    "toolType": "GOOGLE_SEARCH_WEB",
                    "args": { "query": "hello" },
                    "id": "sid-1",
                },
            })
        );
    }
}

mod gemini_3_missing_thought_signature_mitigation {
    use super::*;

    fn prompt_with_tool_call_missing_signature() -> Value {
        json!([
            { "role": "user", "content": [{ "type": "text", "text": "hi" }] },
            {
                "role": "assistant",
                "content": [
                    {
                        "type": "tool-call",
                        "toolCallId": "tc_1",
                        "toolName": "weather",
                        "input": { "location": "SF" },
                    },
                ],
            },
            {
                "role": "tool",
                "content": [
                    {
                        "type": "tool-result",
                        "toolCallId": "tc_1",
                        "toolName": "weather",
                        "output": { "type": "json", "value": { "temperature": 72 } },
                    },
                ],
            },
        ])
    }

    fn assistant(result: &Value) -> &Value {
        result["contents"]
            .as_array()
            .unwrap()
            .iter()
            .find(|content| content["role"] == "model")
            .expect("a model content")
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › Gemini 3 missing thoughtSignature mitigation › injects skip_thought_signature_validator and emits a warning for Gemini 3 when a tool-call has no signature
    #[test]
    fn injects_skip_thought_signature_validator_and_emits_a_warning_for_gemini_3() {
        let (result, warnings) = convert_for_model(
            GEMINI_3_MODEL,
            Namespace::Google,
            prompt_with_tool_call_missing_signature(),
        );

        assert_match_object(
            &assistant(&result)["parts"][0],
            &json!({
                "functionCall": { "id": "tc_1", "name": "weather", "args": { "location": "SF" } },
                "thoughtSignature": SKIP_THOUGHT_SIGNATURE_VALIDATOR,
            }),
        );
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(warnings[0]["type"], "other");
        let message = warnings[0]["message"].as_str().unwrap();
        assert!(
            message.contains("skip_thought_signature_validator"),
            "{message}"
        );
        assert!(message.contains("`weather`"), "{message}");
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › Gemini 3 missing thoughtSignature mitigation › does NOT inject the sentinel or warn for unsigned parallel calls after a signed call
    #[test]
    fn does_not_inject_the_sentinel_or_warn_for_unsigned_parallel_calls_after_a_signed_call() {
        let (result, warnings) = convert_for_model(
            GEMINI_3_MODEL,
            Namespace::Vertex,
            json!([
                {
                    "role": "assistant",
                    "content": [
                        {
                            "type": "tool-call",
                            "toolCallId": "tc_paris",
                            "toolName": "get_weather",
                            "input": { "city": "Paris" },
                            "providerOptions": {
                                "vertex": { "thoughtSignature": "parallel_batch_signature" },
                            },
                        },
                        {
                            "type": "tool-call",
                            "toolCallId": "tc_tokyo",
                            "toolName": "get_weather",
                            "input": { "city": "Tokyo" },
                        },
                        {
                            "type": "tool-call",
                            "toolCallId": "tc_new_york",
                            "toolName": "get_weather",
                            "input": { "city": "New York" },
                        },
                    ],
                },
            ]),
        );

        assert_eq!(
            result["contents"][0]["parts"],
            without_function_call_ids(json!([
                {
                    "functionCall": { "id": "tc_paris", "name": "get_weather", "args": { "city": "Paris" } },
                    "thoughtSignature": "parallel_batch_signature",
                },
                {
                    "functionCall": { "id": "tc_tokyo", "name": "get_weather", "args": { "city": "Tokyo" } },
                },
                {
                    "functionCall": {
                        "id": "tc_new_york",
                        "name": "get_weather",
                        "args": { "city": "New York" },
                    },
                },
            ]))
        );
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › Gemini 3 missing thoughtSignature mitigation › does NOT inject the sentinel when other response parts separate parallel function calls
    #[test]
    fn does_not_inject_the_sentinel_when_other_response_parts_separate_parallel_calls() {
        let (result, warnings) = convert_for_model(
            GEMINI_3_MODEL,
            Namespace::Google,
            json!([
                {
                    "role": "assistant",
                    "content": [
                        {
                            "type": "tool-call",
                            "toolCallId": "tc_signed",
                            "toolName": "weather",
                            "input": { "location": "SF" },
                            "providerOptions": { "google": { "thoughtSignature": "signed_batch" } },
                        },
                        { "type": "text", "text": "Checking another city in the same response." },
                        {
                            "type": "tool-call",
                            "toolCallId": "tc_unsigned",
                            "toolName": "weather",
                            "input": { "location": "NYC" },
                        },
                    ],
                },
            ]),
        );

        assert_eq!(
            result["contents"][0]["parts"][2],
            json!({
                "functionCall": { "id": "tc_unsigned", "name": "weather", "args": { "location": "NYC" } },
            })
        );
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › Gemini 3 missing thoughtSignature mitigation › does NOT inject the sentinel when server tool parts separate parallel function calls
    #[test]
    fn does_not_inject_the_sentinel_when_server_tool_parts_separate_parallel_calls() {
        let (result, warnings) = convert_for_model(
            GEMINI_3_MODEL,
            Namespace::Google,
            json!([
                {
                    "role": "assistant",
                    "content": [
                        {
                            "type": "tool-call",
                            "toolCallId": "signed_function_call",
                            "toolName": "weather",
                            "input": { "location": "SF" },
                            "providerOptions": {
                                "google": { "thoughtSignature": "function_signature" },
                            },
                        },
                        {
                            "type": "tool-call",
                            "toolCallId": "server_call",
                            "toolName": "server:GOOGLE_SEARCH_WEB",
                            "input": { "query": "weather" },
                            "providerOptions": {
                                "google": {
                                    "serverToolCallId": "server_call",
                                    "serverToolType": "GOOGLE_SEARCH_WEB",
                                    "thoughtSignature": "server_call_signature",
                                },
                            },
                        },
                        {
                            "type": "tool-result",
                            "toolCallId": "server_call",
                            "toolName": "server:GOOGLE_SEARCH_WEB",
                            "output": { "type": "json", "value": { "results": [] } },
                            "providerOptions": {
                                "google": {
                                    "serverToolCallId": "server_call",
                                    "serverToolType": "GOOGLE_SEARCH_WEB",
                                    "thoughtSignature": "server_response_signature",
                                },
                            },
                        },
                        {
                            "type": "tool-call",
                            "toolCallId": "unsigned_function_call",
                            "toolName": "weather",
                            "input": { "location": "NYC" },
                        },
                    ],
                },
            ]),
        );

        assert_eq!(
            result["contents"][0]["parts"][3],
            json!({
                "functionCall": {
                    "id": "unsigned_function_call",
                    "name": "weather",
                    "args": { "location": "NYC" },
                },
            })
        );
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › Gemini 3 missing thoughtSignature mitigation › injects the sentinel when a signed server tool call precedes an unsigned function call
    #[test]
    fn injects_the_sentinel_when_a_signed_server_tool_call_precedes_an_unsigned_function_call() {
        let (result, warnings) = convert_for_model(
            GEMINI_3_MODEL,
            Namespace::Google,
            json!([
                {
                    "role": "assistant",
                    "content": [
                        {
                            "type": "tool-call",
                            "toolCallId": "server_call",
                            "toolName": "server:GOOGLE_SEARCH_WEB",
                            "input": { "query": "weather" },
                            "providerOptions": {
                                "google": {
                                    "serverToolCallId": "server_call",
                                    "serverToolType": "GOOGLE_SEARCH_WEB",
                                    "thoughtSignature": "server_signature",
                                },
                            },
                        },
                        {
                            "type": "tool-call",
                            "toolCallId": "function_call",
                            "toolName": "weather",
                            "input": { "location": "NYC" },
                        },
                    ],
                },
            ]),
        );

        assert_eq!(
            result["contents"][0]["parts"],
            json!([
                {
                    "toolCall": {
                        "toolType": "GOOGLE_SEARCH_WEB",
                        "args": { "query": "weather" },
                        "id": "server_call",
                    },
                    "thoughtSignature": "server_signature",
                },
                {
                    "functionCall": { "id": "function_call", "name": "weather", "args": { "location": "NYC" } },
                    "thoughtSignature": SKIP_THOUGHT_SIGNATURE_VALIDATOR,
                },
            ])
        );
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(
            warnings[0]["message"]
                .as_str()
                .unwrap()
                .contains("`weather`"),
            "{warnings:?}"
        );
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › Gemini 3 missing thoughtSignature mitigation › does NOT inject the sentinel for non-Gemini-3 models
    #[test]
    fn does_not_inject_the_sentinel_for_non_gemini_3_models() {
        let (result, warnings) = convert_for_model(
            GEMINI_2_5_MODEL,
            Namespace::Google,
            prompt_with_tool_call_missing_signature(),
        );

        let part = &assistant(&result)["parts"][0];
        assert_match_object(
            part,
            &json!({
                "functionCall": { "id": "tc_1", "name": "weather", "args": { "location": "SF" } },
            }),
        );
        assert!(part.get("thoughtSignature").is_none(), "{part}");
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › Gemini 3 missing thoughtSignature mitigation › does NOT inject the sentinel when a real signature is present under `google`
    #[test]
    fn does_not_inject_the_sentinel_when_a_real_signature_is_present_under_google() {
        let (result, warnings) = convert_for_model(
            GEMINI_3_MODEL,
            Namespace::Google,
            json!([
                { "role": "user", "content": [{ "type": "text", "text": "hi" }] },
                {
                    "role": "assistant",
                    "content": [
                        {
                            "type": "tool-call",
                            "toolCallId": "tc_1",
                            "toolName": "weather",
                            "input": { "location": "SF" },
                            "providerOptions": { "google": { "thoughtSignature": "real_sig" } },
                        },
                    ],
                },
            ]),
        );

        assert_match_object(
            &assistant(&result)["parts"][0],
            &json!({ "thoughtSignature": "real_sig" }),
        );
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › Gemini 3 missing thoughtSignature mitigation › does NOT inject the sentinel when a real signature is present under `vertex`
    #[test]
    fn does_not_inject_the_sentinel_when_a_real_signature_is_present_under_vertex() {
        let (result, warnings) = convert_for_model(
            GEMINI_3_MODEL,
            Namespace::Vertex,
            json!([
                { "role": "user", "content": [{ "type": "text", "text": "hi" }] },
                {
                    "role": "assistant",
                    "content": [
                        {
                            "type": "tool-call",
                            "toolCallId": "tc_1",
                            "toolName": "weather",
                            "input": { "location": "SF" },
                            "providerOptions": { "vertex": { "thoughtSignature": "vertex_sig" } },
                        },
                    ],
                },
            ]),
        );

        assert_match_object(
            &assistant(&result)["parts"][0],
            &json!({ "thoughtSignature": "vertex_sig" }),
        );
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › Gemini 3 missing thoughtSignature mitigation › does NOT inject the sentinel when a real signature is present under `googleVertex`
    #[test]
    fn does_not_inject_the_sentinel_when_a_real_signature_is_present_under_google_vertex() {
        let (result, warnings) = convert_for_model(
            GEMINI_3_MODEL,
            Namespace::Google,
            json!([
                { "role": "user", "content": [{ "type": "text", "text": "hi" }] },
                {
                    "role": "assistant",
                    "content": [
                        {
                            "type": "tool-call",
                            "toolCallId": "tc_1",
                            "toolName": "weather",
                            "input": { "location": "SF" },
                            "providerOptions": {
                                "googleVertex": { "thoughtSignature": "google_vertex_sig" },
                            },
                        },
                    ],
                },
            ]),
        );

        assert_match_object(
            &assistant(&result)["parts"][0],
            &json!({ "thoughtSignature": "google_vertex_sig" }),
        );
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › Gemini 3 missing thoughtSignature mitigation › emits one warning per request listing each affected tool name
    #[test]
    fn emits_one_warning_per_request_listing_each_affected_tool_name() {
        let (_, warnings) = convert_for_model(
            GEMINI_3_MODEL,
            Namespace::Google,
            json!([
                { "role": "user", "content": [{ "type": "text", "text": "hi" }] },
                {
                    "role": "assistant",
                    "content": [
                        {
                            "type": "tool-call",
                            "toolCallId": "tc_1",
                            "toolName": "weather",
                            "input": { "location": "SF" },
                        },
                        {
                            "type": "tool-call",
                            "toolCallId": "tc_2",
                            "toolName": "weather",
                            "input": { "location": "NYC" },
                        },
                        {
                            "type": "tool-call",
                            "toolCallId": "tc_3",
                            "toolName": "search",
                            "input": { "query": "q" },
                        },
                    ],
                },
            ]),
        );

        assert_eq!(warnings.len(), 1, "{warnings:?}");
        let message = warnings[0]["message"].as_str().unwrap();
        assert!(message.contains("3 "), "{message}");
        assert!(message.contains("`weather`"), "{message}");
        assert!(message.contains("`search`"), "{message}");
    }
}

mod top_level_only_media_type_resolution {
    use super::*;

    const PNG_BASE64: &str = "iVBORw0KGgo=";

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › top-level-only media type resolution › passes full image/png through unchanged for inline data
    #[test]
    fn passes_full_image_png_through_unchanged_for_inline_data() {
        let result = convert(json!([
            {
                "role": "user",
                "content": [
                    {
                        "type": "file",
                        "mediaType": "image/png",
                        "data": { "type": "data", "data": PNG_BASE64 },
                    },
                ],
            },
        ]));

        assert_eq!(
            result["contents"][0]["parts"][0],
            json!({ "inlineData": { "mimeType": "image/png", "data": PNG_BASE64 } })
        );
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › top-level-only media type resolution › detects image subtype from inline bytes for top-level "image"
    #[test]
    fn detects_image_subtype_from_inline_bytes_for_top_level_image() {
        let result = convert(json!([
            {
                "role": "user",
                "content": [
                    {
                        "type": "file",
                        "mediaType": "image",
                        "data": { "type": "data", "data": PNG_BASE64 },
                    },
                ],
            },
        ]));

        assert_eq!(
            result["contents"][0]["parts"][0],
            json!({ "inlineData": { "mimeType": "image/png", "data": PNG_BASE64 } })
        );
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › top-level-only media type resolution › throws for top-level-only image with URL source (no bytes to detect)
    #[test]
    fn throws_for_top_level_only_image_with_url_source() {
        let message = convert_error(json!([
            {
                "role": "user",
                "content": [
                    {
                        "type": "file",
                        "mediaType": "image",
                        "data": { "type": "url", "url": "https://example.com/x.png" },
                    },
                ],
            },
        ]));

        assert!(
            regex::Regex::new(r#"media type "image".*not passed as inline bytes"#)
                .unwrap()
                .is_match(&message),
            "{message}"
        );
    }

    /// upstream: @ai-sdk/google src/convert-to-google-messages.test.ts › top-level-only media type resolution › normalizes image/* wildcard via detection
    #[test]
    fn normalizes_image_wildcard_via_detection() {
        let result = convert(json!([
            {
                "role": "user",
                "content": [
                    {
                        "type": "file",
                        "mediaType": "image/*",
                        "data": { "type": "data", "data": PNG_BASE64 },
                    },
                ],
            },
        ]));

        assert_eq!(
            result["contents"][0]["parts"][0],
            json!({ "inlineData": { "mimeType": "image/png", "data": PNG_BASE64 } })
        );
    }
}
