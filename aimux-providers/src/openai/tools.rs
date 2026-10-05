//! Typed factories for the native Responses provider tools.

use std::collections::BTreeMap;

use aimux_core::{AiMuxError, tool::ProviderTool};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchContextSize {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageAction {
    Generate,
    Edit,
    Auto,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageBackground {
    Auto,
    Opaque,
    Transparent,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputFidelity {
    Low,
    High,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageModeration {
    Auto,
    Low,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageOutputFormat {
    Png,
    Jpeg,
    Webp,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageQuality {
    Auto,
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolSearchExecution {
    Server,
    Client,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrammarSyntax {
    Regex,
    Lark,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonOperator {
    Eq,
    Ne,
    Gt,
    Gte,
    Lt,
    Lte,
    In,
    Nin,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompoundOperator {
    And,
    Or,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub enum UserLocationType {
    #[default]
    #[serde(rename = "approximate")]
    Approximate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserLocation {
    pub r#type: UserLocationType,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub city: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebSearchFilters {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_domains: Option<Vec<String>>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_domains: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebSearchArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_web_access: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filters: Option<WebSearchFilters>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_context_size: Option<SearchContextSize>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_location: Option<UserLocation>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebSearchPreviewArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_context_size: Option<SearchContextSize>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_location: Option<UserLocation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FileSearchFilterValue {
    String(String),
    Number(f64),
    Boolean(bool),
    Strings(Vec<String>),
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FileSearchFilter {
    Comparison {
        key: String,
        r#type: ComparisonOperator,
        value: FileSearchFilterValue,
    },
    Compound {
        r#type: CompoundOperator,
        filters: Vec<FileSearchFilter>,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileSearchRanking {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ranker: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score_threshold: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileSearchArgs {
    pub vector_store_ids: Vec<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_num_results: Option<f64>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ranking: Option<FileSearchRanking>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filters: Option<FileSearchFilter>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CodeInterpreterContainer {
    Id(String),
    Files {
        #[serde(rename = "fileIds", default, skip_serializing_if = "Option::is_none")]
        file_ids: Option<Vec<String>>,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeInterpreterArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container: Option<CodeInterpreterContainer>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InputImageMask {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_id: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageGenerationArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<ImageAction>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<ImageBackground>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_fidelity: Option<InputFidelity>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_image_mask: Option<InputImageMask>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moderation: Option<ImageModeration>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_compression: Option<u8>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_format: Option<ImageOutputFormat>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partial_images: Option<u8>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<ImageQuality>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CustomFormat {
    Text,
    Grammar {
        syntax: GrammarSyntax,
        definition: String,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomToolArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#async: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<CustomFormat>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum McpAllowedTools {
    Names(Vec<String>),
    Filter {
        #[serde(rename = "readOnly", default, skip_serializing_if = "Option::is_none")]
        read_only: Option<bool>,
        #[serde(rename = "toolNames", default, skip_serializing_if = "Option::is_none")]
        tool_names: Option<Vec<String>>,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpApprovalPolicy {
    Always,
    Never,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpApprovalFilter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_names: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum McpRequireApproval {
    Policy(McpApprovalPolicy),
    Filter {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        never: Option<McpApprovalFilter>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpArgs {
    pub server_label: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_tools: Option<McpAllowedTools>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connector_id: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<BTreeMap<String, String>>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require_approval: Option<McpRequireApproval>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_description: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ShellMemoryLimit {
    #[serde(rename = "1g")]
    OneG,
    #[serde(rename = "4g")]
    FourG,
    #[serde(rename = "16g")]
    SixteenG,
    #[serde(rename = "64g")]
    SixtyFourG,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellDomainSecret {
    pub domain: String,

    pub name: String,

    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ShellNetworkPolicy {
    Disabled,
    Allowlist {
        #[serde(rename = "allowedDomains")]
        allowed_domains: Vec<String>,
        #[serde(
            rename = "domainSecrets",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        domain_secrets: Option<Vec<ShellDomainSecret>>,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ShellSkill {
    SkillReference {
        #[serde(rename = "providerReference")]
        provider_reference: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        version: Option<String>,
    },
    Inline {
        name: String,
        description: String,
        source: ShellSkillSource,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ShellSkillSource {
    Base64 {
        #[serde(rename = "mediaType")]
        media_type: ShellSkillMediaType,
        data: String,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ShellSkillMediaType {
    #[serde(rename = "application/zip")]
    Zip,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalShellSkill {
    pub name: String,

    pub description: String,

    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ShellEnvironment {
    ContainerAuto {
        #[serde(rename = "fileIds", default, skip_serializing_if = "Option::is_none")]
        file_ids: Option<Vec<String>>,
        #[serde(
            rename = "memoryLimit",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        memory_limit: Option<ShellMemoryLimit>,
        #[serde(
            rename = "networkPolicy",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        network_policy: Option<ShellNetworkPolicy>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        skills: Option<Vec<ShellSkill>>,
    },
    ContainerReference {
        #[serde(rename = "containerId")]
        container_id: String,
    },
    Local {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        skills: Option<Vec<LocalShellSkill>>,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<ShellEnvironment>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolSearchArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution: Option<ToolSearchExecution>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameters: Option<BTreeMap<String, Value>>,
}

fn provider_tool(kind: &str, args: impl Serialize) -> Result<ProviderTool, AiMuxError> {
    let args = serde_json::to_value(args)
        .map_err(|error| AiMuxError::InvalidArgument(error.to_string()))?;
    super::responses::tool_args::validate_tool_args(kind, &args)?;
    Ok(ProviderTool {
        id: format!("openai.{kind}"),
        name: kind.to_owned(),
        args,
    })
}

/// Configure the `web_search` provider tool.
///
/// # Errors
/// Returns an error when arguments do not match the provider tool schema.
pub fn web_search(args: WebSearchArgs) -> Result<ProviderTool, AiMuxError> {
    provider_tool("web_search", args)
}

/// Configure the `web_search_preview` provider tool.
///
/// # Errors
/// Returns an error when arguments do not match the provider tool schema.
pub fn web_search_preview(args: WebSearchPreviewArgs) -> Result<ProviderTool, AiMuxError> {
    provider_tool("web_search_preview", args)
}

/// Configure the `file_search` provider tool.
///
/// # Errors
/// Returns an error when arguments do not match the provider tool schema.
pub fn file_search(args: FileSearchArgs) -> Result<ProviderTool, AiMuxError> {
    provider_tool("file_search", args)
}

/// Configure the `code_interpreter` provider tool.
///
/// # Errors
/// Returns an error when arguments do not match the provider tool schema.
pub fn code_interpreter(args: CodeInterpreterArgs) -> Result<ProviderTool, AiMuxError> {
    provider_tool("code_interpreter", args)
}

/// Configure the `image_generation` provider tool.
///
/// # Errors
/// Returns an error when arguments do not match the provider tool schema.
pub fn image_generation(args: ImageGenerationArgs) -> Result<ProviderTool, AiMuxError> {
    provider_tool("image_generation", args)
}

/// Configure the `custom_tool` provider tool.
///
/// # Errors
/// Returns an error when arguments do not match the provider tool schema.
pub fn custom_tool(args: CustomToolArgs) -> Result<ProviderTool, AiMuxError> {
    provider_tool("custom", args)
}

/// Configure the `mcp` provider tool.
///
/// # Errors
/// Returns an error when arguments do not match the provider tool schema.
pub fn mcp(args: McpArgs) -> Result<ProviderTool, AiMuxError> {
    provider_tool("mcp", args)
}

/// Configure the `shell` provider tool.
///
/// # Errors
/// Returns an error when arguments do not match the provider tool schema.
pub fn shell(args: ShellArgs) -> Result<ProviderTool, AiMuxError> {
    provider_tool("shell", args)
}

/// Configure the `tool_search` provider tool.
///
/// # Errors
/// Returns an error when arguments do not match the provider tool schema.
pub fn tool_search(args: ToolSearchArgs) -> Result<ProviderTool, AiMuxError> {
    provider_tool("tool_search", args)
}

/// Create the `apply_patch` provider tool.
#[must_use]
pub fn apply_patch() -> ProviderTool {
    ProviderTool {
        id: "openai.apply_patch".into(),
        name: "apply_patch".into(),
        args: serde_json::json!({}),
    }
}

/// Create the `computer` provider tool.
#[must_use]
pub fn computer() -> ProviderTool {
    ProviderTool {
        id: "openai.computer".into(),
        name: "computer".into(),
        args: serde_json::json!({}),
    }
}

/// Create the `local_shell` provider tool.
#[must_use]
pub fn local_shell() -> ProviderTool {
    ProviderTool {
        id: "openai.local_shell".into(),
        name: "local_shell".into(),
        args: serde_json::json!({}),
    }
}

/// Create the `programmatic_tool_calling` provider tool.
#[must_use]
pub fn programmatic_tool_calling() -> ProviderTool {
    ProviderTool {
        id: "openai.programmatic_tool_calling".into(),
        name: "programmatic_tool_calling".into(),
        args: serde_json::json!({}),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAIComputerSafetyCheck {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputerButton {
    Left,
    Right,
    Wheel,
    Back,
    Forward,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComputerPoint {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OpenAIComputerAction {
    Click {
        button: ComputerButton,
        x: f64,
        y: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        keys: Option<Vec<String>>,
    },
    DoubleClick {
        x: f64,
        y: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        keys: Option<Vec<String>>,
    },
    Drag {
        path: Vec<ComputerPoint>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        keys: Option<Vec<String>>,
    },
    Keypress {
        keys: Vec<String>,
    },
    Move {
        x: f64,
        y: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        keys: Option<Vec<String>>,
    },
    Screenshot,
    Scroll {
        x: f64,
        y: f64,
        #[serde(rename = "scrollX")]
        scroll_x: f64,
        #[serde(rename = "scrollY")]
        scroll_y: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        keys: Option<Vec<String>>,
    },
    Type {
        text: String,
    },
    Wait,
}
