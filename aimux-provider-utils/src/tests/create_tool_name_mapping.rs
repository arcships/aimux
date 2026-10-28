//! Rust translation of `packages/provider-utils/src/create-tool-name-mapping.test.ts`
//! (@ai-sdk/provider-utils).

use aimux_core::tool::{FunctionTool, ProviderTool, Tool};
use aimux_core::types::NamespacedKind;
use serde_json::{Map, json};

use crate::create_tool_name_mapping;

fn provider_tool(id: &str, name: &str) -> Tool {
    Tool::Provider(ProviderTool {
        id: NamespacedKind::new(id).unwrap(),
        name: name.to_owned(),
        args: Map::new(),
        supports_deferred_results: None,
    })
}

fn function_tool(name: &str) -> Tool {
    Tool::Function(
        FunctionTool::new(name, json!({ "type": "object" })).with_description("A function tool"),
    )
}

/// upstream: @ai-sdk/provider-utils src/create-tool-name-mapping.test.ts › createToolNameMapping › should create mappings for provider-defined tools
#[test]
fn should_create_mappings_for_provider_defined_tools() {
    let tools = [
        provider_tool("anthropic.computer-use", "custom-computer-tool"),
        provider_tool("openai.code-interpreter", "custom-code-tool"),
    ];
    let provider_tool_names = [
        ("anthropic.computer-use", "computer_use"),
        ("openai.code-interpreter", "code_interpreter"),
    ];

    let mapping = create_tool_name_mapping(Some(&tools), &provider_tool_names);

    assert_eq!(
        mapping.to_provider_tool_name("custom-computer-tool"),
        "computer_use"
    );
    assert_eq!(
        mapping.to_provider_tool_name("custom-code-tool"),
        "code_interpreter"
    );
    assert_eq!(
        mapping.to_custom_tool_name("computer_use"),
        "custom-computer-tool"
    );
    assert_eq!(
        mapping.to_custom_tool_name("code_interpreter"),
        "custom-code-tool"
    );
}

/// upstream: @ai-sdk/provider-utils src/create-tool-name-mapping.test.ts › createToolNameMapping › should ignore function tools
#[test]
fn should_ignore_function_tools() {
    let tools = [function_tool("my-function-tool")];

    let mapping = create_tool_name_mapping(Some(&tools), &[]);

    assert_eq!(
        mapping.to_provider_tool_name("my-function-tool"),
        "my-function-tool"
    );
    assert_eq!(
        mapping.to_custom_tool_name("my-function-tool"),
        "my-function-tool"
    );
}

/// upstream: @ai-sdk/provider-utils src/create-tool-name-mapping.test.ts › createToolNameMapping › should return input name when tool is not in providerToolNames
#[test]
fn should_return_input_name_when_tool_is_not_in_provider_tool_names() {
    let tools = [provider_tool("unknown.tool", "custom-tool")];

    let mapping = create_tool_name_mapping(Some(&tools), &[]);

    assert_eq!(mapping.to_provider_tool_name("custom-tool"), "custom-tool");
    assert_eq!(mapping.to_custom_tool_name("unknown-name"), "unknown-name");
}

/// upstream: @ai-sdk/provider-utils src/create-tool-name-mapping.test.ts › createToolNameMapping › should return input name when mapping does not exist
#[test]
fn should_return_input_name_when_mapping_does_not_exist() {
    let tools = [provider_tool(
        "anthropic.computer-use",
        "custom-computer-tool",
    )];
    let provider_tool_names = [("anthropic.computer-use", "computer_use")];

    let mapping = create_tool_name_mapping(Some(&tools), &provider_tool_names);

    assert_eq!(
        mapping.to_provider_tool_name("non-existent-tool"),
        "non-existent-tool"
    );
    assert_eq!(
        mapping.to_custom_tool_name("non-existent-provider-tool"),
        "non-existent-provider-tool"
    );
}

/// upstream: @ai-sdk/provider-utils src/create-tool-name-mapping.test.ts › createToolNameMapping › should handle empty tools array
#[test]
fn should_handle_empty_tools_array() {
    let mapping = create_tool_name_mapping(Some(&[]), &[]);

    assert_eq!(mapping.to_provider_tool_name("any-tool"), "any-tool");
    assert_eq!(mapping.to_custom_tool_name("any-tool"), "any-tool");
}

/// upstream: @ai-sdk/provider-utils src/create-tool-name-mapping.test.ts › createToolNameMapping › should handle mixed function and provider-defined tools
#[test]
fn should_handle_mixed_function_and_provider_defined_tools() {
    let tools = [
        function_tool("function-tool"),
        provider_tool("anthropic.computer-use", "provider-tool"),
    ];
    let provider_tool_names = [("anthropic.computer-use", "computer_use")];

    let mapping = create_tool_name_mapping(Some(&tools), &provider_tool_names);

    // Function tool should not be mapped
    assert_eq!(
        mapping.to_provider_tool_name("function-tool"),
        "function-tool"
    );
    assert_eq!(
        mapping.to_custom_tool_name("function-tool"),
        "function-tool"
    );

    // Provider-defined tool should be mapped
    assert_eq!(
        mapping.to_provider_tool_name("provider-tool"),
        "computer_use"
    );
    assert_eq!(mapping.to_custom_tool_name("computer_use"), "provider-tool");
}
