//! `prepareTools` (`google-prepare-tools.test.ts`), through
//! `prepare_all_tools(tools, tool_choice, model_id)`. `isVertexProvider: true`
//! has no flag there: aimux drops the server-side tool invocation flag while
//! building a Vertex request body, so that case builds the body with
//! `Namespace::Vertex` and reads `tools` / `toolConfig` and the warnings back.

use serde_json::{Value, json};

use aimux_core::options::CallOptions;
use aimux_core::tool::{LanguageModelV4ToolChoice, Tool};

use crate::google::convert::{build_request_body_with_warnings_for_namespace, prepare_all_tools};
use crate::google::options::Namespace;

fn tools(value: Value) -> Option<Vec<Tool>> {
    Some(serde_json::from_value(value).expect("tools deserialize"))
}

fn tool_choice(value: Value) -> Option<LanguageModelV4ToolChoice> {
    Some(serde_json::from_value(value).expect("tool choice deserializes"))
}

/// The `prepareTools` result as JSON (`undefined` members omitted).
fn prepare(
    tools: Option<Vec<Tool>>,
    tool_choice: Option<LanguageModelV4ToolChoice>,
    model_id: &str,
) -> Value {
    let prepared = prepare_all_tools(&tools, tool_choice.as_ref(), model_id);
    let mut result = json!({ "toolWarnings": prepared.warnings });
    if let Some(tools) = prepared.tools {
        result["tools"] = Value::Array(tools);
    }
    if let Some(tool_config) = prepared.tool_config {
        result["toolConfig"] = tool_config;
    }
    result
}

/// `prepareTools({ ..., isVertexProvider: true })`: the tool members of a
/// Vertex request body and its warnings.
fn prepare_vertex(
    tools: Option<Vec<Tool>>,
    tool_choice: Option<LanguageModelV4ToolChoice>,
    model_id: &str,
) -> Value {
    let mut options = CallOptions::new(Vec::new());
    options.tools = tools;
    options.tool_choice = tool_choice;
    let (body, warnings) =
        build_request_body_with_warnings_for_namespace(model_id, &options, Namespace::Vertex)
            .expect("body builds");
    let mut result = json!({ "toolWarnings": warnings });
    if let Some(tools) = body.get("tools") {
        result["tools"] = tools.clone();
    }
    if let Some(tool_config) = body.get("toolConfig") {
        result["toolConfig"] = tool_config.clone();
    }
    result
}

/// upstream: @ai-sdk/google src/google-prepare-tools.test.ts › should return undefined tools and tool_choice when tools are null
#[test]
fn should_return_undefined_tools_and_tool_choice_when_tools_are_null() {
    let result = prepare(None, None, "gemini-2.5-flash");
    assert_eq!(result, json!({ "toolWarnings": [] }));
}

/// upstream: @ai-sdk/google src/google-prepare-tools.test.ts › should return undefined tools and tool_choice when tools are empty
#[test]
fn should_return_undefined_tools_and_tool_choice_when_tools_are_empty() {
    let result = prepare(tools(json!([])), None, "gemini-2.5-flash");
    assert_eq!(result, json!({ "toolWarnings": [] }));
}

/// upstream: @ai-sdk/google src/google-prepare-tools.test.ts › should correctly prepare function tools
#[test]
fn should_correctly_prepare_function_tools() {
    let result = prepare(
        tools(json!([
            {
                "type": "function",
                "name": "testFunction",
                "description": "A test function",
                "inputSchema": { "type": "object", "properties": {} },
            },
        ])),
        None,
        "gemini-2.5-flash",
    );
    assert_eq!(
        result["tools"],
        json!([
            {
                "functionDeclarations": [
                    {
                        "name": "testFunction",
                        "description": "A test function",
                        "parametersJsonSchema": { "type": "object", "properties": {} },
                    },
                ],
            },
        ])
    );
    assert!(result.get("toolConfig").is_none());
    assert_eq!(result["toolWarnings"], json!([]));
}

/// upstream: @ai-sdk/google src/google-prepare-tools.test.ts › should preserve recursive function tool schemas as JSON Schema
#[test]
fn should_preserve_recursive_function_tool_schemas_as_json_schema() {
    let input_schema = json!({
        "type": "object",
        "properties": { "condition": { "$ref": "#/$defs/Condition" } },
        "required": ["condition"],
        "$defs": {
            "Condition": {
                "type": "object",
                "properties": {
                    "children": { "type": "array", "items": { "$ref": "#/$defs/Condition" } },
                },
            },
        },
    });

    let result = prepare(
        tools(json!([
            {
                "type": "function",
                "name": "search",
                "description": "Search with a condition tree",
                "inputSchema": input_schema,
            },
        ])),
        None,
        "gemini-2.5-flash",
    );

    assert_eq!(
        result["tools"],
        json!([
            {
                "functionDeclarations": [
                    {
                        "name": "search",
                        "description": "Search with a condition tree",
                        "parametersJsonSchema": input_schema,
                    },
                ],
            },
        ])
    );
}

/// upstream: @ai-sdk/google src/google-prepare-tools.test.ts › should correctly prepare provider-defined tools as array
#[test]
fn should_correctly_prepare_provider_defined_tools_as_array() {
    let result = prepare(
        tools(json!([
            { "type": "provider", "id": "google.google_search", "name": "google_search", "args": {} },
            { "type": "provider", "id": "google.url_context", "name": "url_context", "args": {} },
            {
                "type": "provider",
                "id": "google.file_search",
                "name": "file_search",
                "args": { "fileSearchStoreNames": ["projects/foo/fileSearchStores/bar"] },
            },
        ])),
        None,
        "gemini-2.5-flash",
    );
    assert_eq!(
        result["tools"],
        json!([
            { "googleSearch": {} },
            { "urlContext": {} },
            { "fileSearch": { "fileSearchStoreNames": ["projects/foo/fileSearchStores/bar"] } },
        ])
    );
    assert!(result.get("toolConfig").is_none());
    assert_eq!(result["toolWarnings"], json!([]));
}

/// upstream: @ai-sdk/google src/google-prepare-tools.test.ts › should correctly prepare single provider-defined tool
#[test]
fn should_correctly_prepare_single_provider_defined_tool() {
    let result = prepare(
        tools(json!([
            { "type": "provider", "id": "google.google_search", "name": "google_search", "args": {} },
        ])),
        None,
        "gemini-2.5-flash",
    );
    assert_eq!(result["tools"], json!([{ "googleSearch": {} }]));
    assert!(result.get("toolConfig").is_none());
    assert_eq!(result["toolWarnings"], json!([]));
}

/// upstream: @ai-sdk/google src/google-prepare-tools.test.ts › should add warnings for unsupported tools
#[test]
fn should_add_warnings_for_unsupported_tools() {
    let result = prepare(
        tools(json!([
            { "type": "provider", "id": "unsupported.tool", "name": "unsupported_tool", "args": {} },
        ])),
        None,
        "gemini-2.5-flash",
    );
    assert!(result.get("tools").is_none());
    assert!(result.get("toolConfig").is_none());
    assert_eq!(
        result["toolWarnings"],
        json!([
            { "feature": "provider-defined tool unsupported.tool", "type": "unsupported" },
        ])
    );
}

/// upstream: @ai-sdk/google src/google-prepare-tools.test.ts › should add warnings for file search on unsupported models
#[test]
fn should_add_warnings_for_file_search_on_unsupported_models() {
    let result = prepare(
        tools(json!([
            {
                "type": "provider",
                "id": "google.file_search",
                "name": "file_search",
                "args": { "fileSearchStoreNames": ["projects/foo/fileSearchStores/bar"] },
            },
        ])),
        None,
        "gemini-1.5-flash-8b",
    );
    assert!(result.get("tools").is_none());
    assert_eq!(
        result["toolWarnings"],
        json!([
            {
                "details": "The file search tool is only supported with Gemini 2.5 models and Gemini 3 models.",
                "feature": "provider-defined tool google.file_search",
                "type": "unsupported",
            },
        ])
    );
}

/// upstream: @ai-sdk/google src/google-prepare-tools.test.ts › should correctly prepare file search tool for gemini-2.5 models
#[test]
fn should_correctly_prepare_file_search_tool_for_gemini_2_5_models() {
    let result = prepare(
        tools(json!([
            {
                "type": "provider",
                "id": "google.file_search",
                "name": "file_search",
                "args": {
                    "fileSearchStoreNames": ["projects/foo/fileSearchStores/bar"],
                    "metadataFilter": "author=Robert Graves",
                    "topK": 5,
                },
            },
        ])),
        None,
        "gemini-2.5-pro",
    );
    assert_eq!(
        result["tools"],
        json!([
            {
                "fileSearch": {
                    "fileSearchStoreNames": ["projects/foo/fileSearchStores/bar"],
                    "metadataFilter": "author=Robert Graves",
                    "topK": 5,
                },
            },
        ])
    );
    assert_eq!(result["toolWarnings"], json!([]));
}

/// upstream: @ai-sdk/google src/google-prepare-tools.test.ts › should correctly prepare file search tool for gemini-3 models
#[test]
fn should_correctly_prepare_file_search_tool_for_gemini_3_models() {
    let result = prepare(
        tools(json!([
            {
                "type": "provider",
                "id": "google.file_search",
                "name": "file_search",
                "args": {
                    "fileSearchStoreNames": ["projects/foo/fileSearchStores/bar"],
                    "metadataFilter": "author=Robert Graves",
                    "topK": 5,
                },
            },
        ])),
        None,
        "gemini-3.1-pro-preview",
    );
    assert_eq!(
        result["tools"],
        json!([
            {
                "fileSearch": {
                    "fileSearchStoreNames": ["projects/foo/fileSearchStores/bar"],
                    "metadataFilter": "author=Robert Graves",
                    "topK": 5,
                },
            },
        ])
    );
    assert_eq!(result["toolWarnings"], json!([]));
}

/// upstream: @ai-sdk/google src/google-prepare-tools.test.ts › should use newest tool support for an unknown future Gemini model
#[test]
fn should_use_newest_tool_support_for_an_unknown_future_gemini_model() {
    let result = prepare(
        tools(json!([
            {
                "type": "function",
                "name": "getWeather",
                "description": "Get the weather",
                "inputSchema": { "type": "object", "properties": { "location": { "type": "string" } } },
            },
            { "type": "provider", "id": "google.google_search", "name": "google_search", "args": {} },
            {
                "type": "provider",
                "id": "google.enterprise_web_search",
                "name": "enterprise_web_search",
                "args": {},
            },
            { "type": "provider", "id": "google.url_context", "name": "url_context", "args": {} },
            { "type": "provider", "id": "google.code_execution", "name": "code_execution", "args": {} },
            {
                "type": "provider",
                "id": "google.file_search",
                "name": "file_search",
                "args": { "fileSearchStoreNames": ["fileSearchStores/example-store"] },
            },
        ])),
        None,
        "gemini-99-pro-preview",
    );

    assert_eq!(
        result,
        json!({
            "tools": [
                { "googleSearch": {} },
                { "enterpriseWebSearch": {} },
                { "urlContext": {} },
                { "codeExecution": {} },
                { "fileSearch": { "fileSearchStoreNames": ["fileSearchStores/example-store"] } },
                {
                    "functionDeclarations": [
                        {
                            "name": "getWeather",
                            "description": "Get the weather",
                            "parametersJsonSchema": {
                                "type": "object",
                                "properties": { "location": { "type": "string" } },
                            },
                        },
                    ],
                },
            ],
            "toolConfig": {
                "functionCallingConfig": { "mode": "VALIDATED" },
                "includeServerSideToolInvocations": true,
            },
            "toolWarnings": [],
        })
    );
}

fn test_function() -> Value {
    json!({ "type": "function", "name": "testFunction", "description": "Test", "inputSchema": {} })
}

/// upstream: @ai-sdk/google src/google-prepare-tools.test.ts › should handle tool choice "auto"
#[test]
fn should_handle_tool_choice_auto() {
    let result = prepare(
        tools(json!([test_function()])),
        tool_choice(json!({ "type": "auto" })),
        "gemini-2.5-flash",
    );
    assert_eq!(
        result["toolConfig"],
        json!({ "functionCallingConfig": { "mode": "AUTO" } })
    );
}

/// upstream: @ai-sdk/google src/google-prepare-tools.test.ts › should handle tool choice "required"
#[test]
fn should_handle_tool_choice_required() {
    let result = prepare(
        tools(json!([test_function()])),
        tool_choice(json!({ "type": "required" })),
        "gemini-2.5-flash",
    );
    assert_eq!(
        result["toolConfig"],
        json!({ "functionCallingConfig": { "mode": "ANY" } })
    );
}

/// upstream: @ai-sdk/google src/google-prepare-tools.test.ts › should handle tool choice "none"
#[test]
fn should_handle_tool_choice_none() {
    let result = prepare(
        tools(json!([test_function()])),
        tool_choice(json!({ "type": "none" })),
        "gemini-2.5-flash",
    );
    assert_eq!(
        result["tools"],
        json!([
            {
                "functionDeclarations": [
                    { "name": "testFunction", "description": "Test", "parametersJsonSchema": {} },
                ],
            },
        ])
    );
    assert_eq!(
        result["toolConfig"],
        json!({ "functionCallingConfig": { "mode": "NONE" } })
    );
}

/// upstream: @ai-sdk/google src/google-prepare-tools.test.ts › should handle tool choice "tool"
#[test]
fn should_handle_tool_choice_tool() {
    let result = prepare(
        tools(json!([test_function()])),
        tool_choice(json!({ "type": "tool", "toolName": "testFunction" })),
        "gemini-2.5-flash",
    );
    assert_eq!(
        result["toolConfig"],
        json!({
            "functionCallingConfig": { "mode": "ANY", "allowedFunctionNames": ["testFunction"] },
        })
    );
}

fn function_and_google_search() -> Option<Vec<Tool>> {
    tools(json!([
        {
            "type": "function",
            "name": "testFunction",
            "description": "A test function",
            "inputSchema": { "type": "object", "properties": {} },
        },
        { "type": "provider", "id": "google.google_search", "name": "google_search", "args": {} },
    ]))
}

/// upstream: @ai-sdk/google src/google-prepare-tools.test.ts › should warn when mixing function and provider-defined tools
#[test]
fn should_warn_when_mixing_function_and_provider_defined_tools() {
    let result = prepare(function_and_google_search(), None, "gemini-2.5-flash");

    assert_eq!(result["tools"], json!([{ "googleSearch": {} }]));
    assert_eq!(
        result["toolWarnings"],
        json!([
            { "feature": "combination of function and provider-defined tools", "type": "unsupported" },
        ])
    );
    assert!(result.get("toolConfig").is_none());
}

/// upstream: @ai-sdk/google src/google-prepare-tools.test.ts › should handle tool choice with mixed tools (provider-defined tools only)
#[test]
fn should_handle_tool_choice_with_mixed_tools_provider_defined_tools_only() {
    let result = prepare(
        function_and_google_search(),
        tool_choice(json!({ "type": "auto" })),
        "gemini-2.5-flash",
    );

    assert_eq!(result["tools"], json!([{ "googleSearch": {} }]));
    assert!(result.get("toolConfig").is_none());
    assert_eq!(
        result["toolWarnings"],
        json!([
            { "feature": "combination of function and provider-defined tools", "type": "unsupported" },
        ])
    );
}

/// upstream: @ai-sdk/google src/google-prepare-tools.test.ts › should combine function and provider-defined tools on Gemini 3 models
#[test]
fn should_combine_function_and_provider_defined_tools_on_gemini_3_models() {
    let result = prepare(
        function_and_google_search(),
        None,
        "gemini-3.1-flash-lite-preview",
    );

    assert_eq!(
        result["tools"],
        json!([
            { "googleSearch": {} },
            {
                "functionDeclarations": [
                    {
                        "name": "testFunction",
                        "description": "A test function",
                        "parametersJsonSchema": { "type": "object", "properties": {} },
                    },
                ],
            },
        ])
    );
    assert_eq!(
        result["toolConfig"],
        json!({
            "functionCallingConfig": { "mode": "VALIDATED" },
            "includeServerSideToolInvocations": true,
        })
    );
    assert_eq!(result["toolWarnings"], json!([]));
}

/// upstream: @ai-sdk/google src/google-prepare-tools.test.ts › should omit server-side tool invocation flag for Vertex Gemini 3
#[test]
fn should_omit_server_side_tool_invocation_flag_for_vertex_gemini_3() {
    let result = prepare_vertex(function_and_google_search(), None, "gemini-3-flash-preview");

    assert_eq!(
        result,
        json!({
            "toolConfig": { "functionCallingConfig": { "mode": "VALIDATED" } },
            "toolWarnings": [],
            "tools": [
                { "googleSearch": {} },
                {
                    "functionDeclarations": [
                        {
                            "description": "A test function",
                            "name": "testFunction",
                            "parametersJsonSchema": { "properties": {}, "type": "object" },
                        },
                    ],
                },
            ],
        })
    );
}

/// upstream: @ai-sdk/google src/google-prepare-tools.test.ts › should combine multiple provider tools with function tools on Gemini 3
#[test]
fn should_combine_multiple_provider_tools_with_function_tools_on_gemini_3() {
    let result = prepare(
        tools(json!([
            {
                "type": "function",
                "name": "getWeather",
                "description": "Get weather",
                "inputSchema": { "type": "object", "properties": {} },
            },
            {
                "type": "function",
                "name": "bookVenue",
                "description": "Book a venue",
                "inputSchema": { "type": "object", "properties": {} },
            },
            { "type": "provider", "id": "google.google_search", "name": "google_search", "args": {} },
            { "type": "provider", "id": "google.google_maps", "name": "google_maps", "args": {} },
        ])),
        None,
        "gemini-3-flash-preview",
    );

    assert_eq!(
        result["tools"],
        json!([
            { "googleSearch": {} },
            { "googleMaps": {} },
            {
                "functionDeclarations": [
                    {
                        "name": "getWeather",
                        "description": "Get weather",
                        "parametersJsonSchema": { "type": "object", "properties": {} },
                    },
                    {
                        "name": "bookVenue",
                        "description": "Book a venue",
                        "parametersJsonSchema": { "type": "object", "properties": {} },
                    },
                ],
            },
        ])
    );
    assert_eq!(
        result["toolConfig"],
        json!({
            "functionCallingConfig": { "mode": "VALIDATED" },
            "includeServerSideToolInvocations": true,
        })
    );
    assert_eq!(result["toolWarnings"], json!([]));
}

/// upstream: @ai-sdk/google src/google-prepare-tools.test.ts › should use VALIDATED mode for combined tools with toolChoice auto on Gemini 3
#[test]
fn should_use_validated_mode_for_combined_tools_with_tool_choice_auto_on_gemini_3() {
    let result = prepare(
        function_and_google_search(),
        tool_choice(json!({ "type": "auto" })),
        "gemini-3-flash-preview",
    );

    assert_eq!(
        result["toolConfig"],
        json!({
            "functionCallingConfig": { "mode": "VALIDATED" },
            "includeServerSideToolInvocations": true,
        })
    );
    assert_eq!(result["toolWarnings"], json!([]));
}

/// upstream: @ai-sdk/google src/google-prepare-tools.test.ts › should use ANY mode for combined tools with toolChoice required on Gemini 3
#[test]
fn should_use_any_mode_for_combined_tools_with_tool_choice_required_on_gemini_3() {
    let result = prepare(
        function_and_google_search(),
        tool_choice(json!({ "type": "required" })),
        "gemini-3-flash-preview",
    );

    assert_eq!(
        result["toolConfig"],
        json!({
            "functionCallingConfig": { "mode": "ANY" },
            "includeServerSideToolInvocations": true,
        })
    );
}

/// upstream: @ai-sdk/google src/google-prepare-tools.test.ts › should use NONE mode for combined tools with toolChoice none on Gemini 3
#[test]
fn should_use_none_mode_for_combined_tools_with_tool_choice_none_on_gemini_3() {
    let result = prepare(
        function_and_google_search(),
        tool_choice(json!({ "type": "none" })),
        "gemini-3-flash-preview",
    );

    assert_eq!(
        result["toolConfig"],
        json!({
            "functionCallingConfig": { "mode": "NONE" },
            "includeServerSideToolInvocations": true,
        })
    );
}

/// upstream: @ai-sdk/google src/google-prepare-tools.test.ts › should use ANY mode with allowedFunctionNames for combined tools with specific tool choice on Gemini 3
#[test]
fn should_use_any_mode_with_allowed_function_names_for_combined_tools_on_gemini_3() {
    let result = prepare(
        function_and_google_search(),
        tool_choice(json!({ "type": "tool", "toolName": "testFunction" })),
        "gemini-3-flash-preview",
    );

    assert_eq!(
        result["toolConfig"],
        json!({
            "functionCallingConfig": { "mode": "ANY", "allowedFunctionNames": ["testFunction"] },
            "includeServerSideToolInvocations": true,
        })
    );
}

fn single_provider_tool(id: &str, name: &str, args: Value) -> Option<Vec<Tool>> {
    tools(json!([{ "type": "provider", "id": id, "name": name, "args": args }]))
}

/// upstream: @ai-sdk/google src/google-prepare-tools.test.ts › should handle latest modelId for provider-defined tools correctly
#[test]
fn should_handle_latest_model_id_for_provider_defined_tools_correctly() {
    let result = prepare(
        single_provider_tool("google.google_search", "google_search", json!({})),
        None,
        "gemini-flash-latest",
    );
    assert_eq!(result["tools"], json!([{ "googleSearch": {} }]));
    assert!(result.get("toolConfig").is_none());
    assert_eq!(result["toolWarnings"], json!([]));
}

/// upstream: @ai-sdk/google src/google-prepare-tools.test.ts › should handle gemini-3 modelId for provider-defined tools correctly
#[test]
fn should_handle_gemini_3_model_id_for_provider_defined_tools_correctly() {
    let result = prepare(
        single_provider_tool("google.google_search", "google_search", json!({})),
        None,
        "gemini-3.1-pro-preview",
    );
    assert_eq!(result["tools"], json!([{ "googleSearch": {} }]));
    assert!(result.get("toolConfig").is_none());
    assert_eq!(result["toolWarnings"], json!([]));
}

/// upstream: @ai-sdk/google src/google-prepare-tools.test.ts › should handle code execution tool
#[test]
fn should_handle_code_execution_tool() {
    let result = prepare(
        single_provider_tool("google.code_execution", "code_execution", json!({})),
        None,
        "gemini-2.5-flash",
    );
    assert_eq!(result["tools"], json!([{ "codeExecution": {} }]));
    assert!(result.get("toolConfig").is_none());
    assert_eq!(result["toolWarnings"], json!([]));
}

/// upstream: @ai-sdk/google src/google-prepare-tools.test.ts › should handle url context tool alone
#[test]
fn should_handle_url_context_tool_alone() {
    let result = prepare(
        single_provider_tool("google.url_context", "url_context", json!({})),
        None,
        "gemini-2.5-flash",
    );
    assert_eq!(result["tools"], json!([{ "urlContext": {} }]));
    assert!(result.get("toolConfig").is_none());
    assert_eq!(result["toolWarnings"], json!([]));
}

/// upstream: @ai-sdk/google src/google-prepare-tools.test.ts › should handle google maps tool
#[test]
fn should_handle_google_maps_tool() {
    let result = prepare(
        single_provider_tool("google.google_maps", "google_maps", json!({})),
        None,
        "gemini-2.5-flash",
    );
    assert_eq!(result["tools"], json!([{ "googleMaps": {} }]));
    assert!(result.get("toolConfig").is_none());
    assert_eq!(result["toolWarnings"], json!([]));
}

/// upstream: @ai-sdk/google src/google-prepare-tools.test.ts › should pass searchTypes args through for google search
#[test]
fn should_pass_search_types_args_through_for_google_search() {
    let result = prepare(
        single_provider_tool(
            "google.google_search",
            "google_search",
            json!({ "searchTypes": { "webSearch": {}, "imageSearch": {} } }),
        ),
        None,
        "gemini-3.1-flash-image-preview",
    );
    assert_eq!(
        result["tools"],
        json!([
            { "googleSearch": { "searchTypes": { "webSearch": {}, "imageSearch": {} } } },
        ])
    );
    assert_eq!(result["toolWarnings"], json!([]));
}

/// upstream: @ai-sdk/google src/google-prepare-tools.test.ts › should pass timeRangeFilter args through for google search
#[test]
fn should_pass_time_range_filter_args_through_for_google_search() {
    let result = prepare(
        single_provider_tool(
            "google.google_search",
            "google_search",
            json!({
                "timeRangeFilter": {
                    "startTime": "2025-01-01T00:00:00Z",
                    "endTime": "2025-12-31T23:59:59Z",
                },
            }),
        ),
        None,
        "gemini-2.5-flash",
    );
    assert_eq!(
        result["tools"],
        json!([
            {
                "googleSearch": {
                    "timeRangeFilter": {
                        "startTime": "2025-01-01T00:00:00Z",
                        "endTime": "2025-12-31T23:59:59Z",
                    },
                },
            },
        ])
    );
    assert_eq!(result["toolWarnings"], json!([]));
}

/// upstream: @ai-sdk/google src/google-prepare-tools.test.ts › should add warnings for google search on unsupported models
#[test]
fn should_add_warnings_for_google_search_on_unsupported_models() {
    let result = prepare(
        single_provider_tool("google.google_search", "google_search", json!({})),
        None,
        "gemini-1.5-flash",
    );
    assert!(result.get("tools").is_none());
    assert_eq!(
        result["toolWarnings"],
        json!([
            {
                "details": "Google Search requires Gemini 2.0 or newer.",
                "feature": "provider-defined tool google.google_search",
                "type": "unsupported",
            },
        ])
    );
}

/// upstream: @ai-sdk/google src/google-prepare-tools.test.ts › should add warnings for google maps on unsupported models
#[test]
fn should_add_warnings_for_google_maps_on_unsupported_models() {
    let result = prepare(
        single_provider_tool("google.google_maps", "google_maps", json!({})),
        None,
        "gemini-1.5-flash",
    );
    assert!(result.get("tools").is_none());
    assert_eq!(
        result["toolWarnings"],
        json!([
            {
                "details": "The Google Maps grounding tool is not supported with Gemini models other than Gemini 2 or newer.",
                "feature": "provider-defined tool google.google_maps",
                "type": "unsupported",
            },
        ])
    );
}

fn strict_get_weather() -> Value {
    json!({
        "type": "function",
        "name": "getWeather",
        "description": "Get weather",
        "inputSchema": {
            "type": "object",
            "properties": { "city": { "type": "string" } },
            "required": ["city"],
            "additionalProperties": false,
        },
        "strict": true,
    })
}

/// upstream: @ai-sdk/google src/google-prepare-tools.test.ts › should use VALIDATED mode when any function tool has strict: true
#[test]
fn should_use_validated_mode_when_any_function_tool_has_strict_true() {
    let result = prepare(
        tools(json!([
            {
                "type": "function",
                "name": "createMeeting",
                "description": "Create a meeting",
                "inputSchema": {
                    "type": "object",
                    "properties": { "title": { "type": "string" } },
                    "required": ["title"],
                    "additionalProperties": false,
                },
                "strict": true,
            },
        ])),
        None,
        "gemini-3-flash-preview",
    );
    assert_eq!(
        result["toolConfig"],
        json!({ "functionCallingConfig": { "mode": "VALIDATED" } })
    );
    assert_eq!(result["toolWarnings"], json!([]));
}

/// upstream: @ai-sdk/google src/google-prepare-tools.test.ts › should use VALIDATED mode with toolChoice auto when strict: true
#[test]
fn should_use_validated_mode_with_tool_choice_auto_when_strict_true() {
    let result = prepare(
        tools(json!([strict_get_weather()])),
        tool_choice(json!({ "type": "auto" })),
        "gemini-3-flash-preview",
    );
    assert_eq!(
        result["toolConfig"],
        json!({ "functionCallingConfig": { "mode": "VALIDATED" } })
    );
}

/// upstream: @ai-sdk/google src/google-prepare-tools.test.ts › should use ANY mode with toolChoice required when strict: true
#[test]
fn should_use_any_mode_with_tool_choice_required_when_strict_true() {
    let result = prepare(
        tools(json!([strict_get_weather()])),
        tool_choice(json!({ "type": "required" })),
        "gemini-3-flash-preview",
    );
    assert_eq!(
        result["toolConfig"],
        json!({ "functionCallingConfig": { "mode": "ANY" } })
    );
}

/// upstream: @ai-sdk/google src/google-prepare-tools.test.ts › should use ANY mode with named toolChoice when another tool has strict: true
#[test]
fn should_use_any_mode_with_named_tool_choice_when_another_tool_has_strict_true() {
    let result = prepare(
        tools(json!([
            {
                "type": "function",
                "name": "createMeeting",
                "description": "Create meeting",
                "inputSchema": {
                    "type": "object",
                    "properties": { "title": { "type": "string" } },
                    "required": ["title"],
                    "additionalProperties": false,
                },
            },
            strict_get_weather(),
        ])),
        tool_choice(json!({ "type": "tool", "toolName": "createMeeting" })),
        "gemini-3-flash-preview",
    );
    assert_eq!(
        result["toolConfig"],
        json!({
            "functionCallingConfig": { "mode": "ANY", "allowedFunctionNames": ["createMeeting"] },
        })
    );
}

/// upstream: @ai-sdk/google src/google-prepare-tools.test.ts › should use AUTO mode when no tools have strict: true
#[test]
fn should_use_auto_mode_when_no_tools_have_strict_true() {
    let result = prepare(
        tools(json!([
            {
                "type": "function",
                "name": "getWeather",
                "description": "Get weather",
                "inputSchema": {
                    "type": "object",
                    "properties": { "city": { "type": "string" } },
                    "required": ["city"],
                    "additionalProperties": false,
                },
            },
        ])),
        tool_choice(json!({ "type": "auto" })),
        "gemini-3-flash-preview",
    );
    assert_eq!(
        result["toolConfig"],
        json!({ "functionCallingConfig": { "mode": "AUTO" } })
    );
}
