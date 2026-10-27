//! `tool/`: the provider tools of the Responses API, one file per upstream tool.

pub(super) mod apply_patch;
pub(super) mod code_interpreter;
pub(super) mod computer;
pub(super) mod custom;
pub(super) mod file_search;
pub(super) mod image_generation;
pub(super) mod local_shell;
pub(super) mod mcp;
pub(super) mod programmatic_tool_calling;
pub(super) mod shell;
pub(super) mod tool_search;
pub(super) mod web_search;
pub(super) mod web_search_preview;
