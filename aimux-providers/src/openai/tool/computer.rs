//! The computer use provider tool and its arguments (tool/computer.ts).

use aimux_core::tool::ProviderTool;
use serde::{Deserialize, Serialize};

use crate::openai::tools::openai_id;

/// Create the `computer` provider tool.
#[must_use]
pub fn computer() -> ProviderTool {
    ProviderTool {
        id: openai_id("computer"),
        name: "computer".into(),
        args: serde_json::Map::new(),
        supports_deferred_results: None,
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
