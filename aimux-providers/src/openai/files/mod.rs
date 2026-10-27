//! `files/`: OpenAI Files uploads.

#[allow(
    clippy::module_inception,
    reason = "upstream keeps files/openai-files.ts in a files/ folder"
)]
pub(super) mod files;
pub(super) mod files_api;
pub(super) mod files_options;

pub use files::OpenAIFiles;
