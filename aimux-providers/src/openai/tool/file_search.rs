//! The file search provider tool and its arguments (tool/file-search.ts).

use aimux_core::AiMuxError;
use aimux_core::tool::ProviderTool;
use serde::{Deserialize, Serialize};

use crate::openai::tools::provider_tool;

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
    pub max_num_results: Option<u32>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ranking: Option<FileSearchRanking>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filters: Option<FileSearchFilter>,
}

/// Configure the `file_search` provider tool.
///
/// # Errors
/// Returns an error when arguments do not match the provider tool schema.
pub fn file_search(args: FileSearchArgs) -> Result<ProviderTool, AiMuxError> {
    provider_tool("file_search", args)
}
