//! ByteDance Ark Seedance video and Seedream image generation.
//!
//! [`create_bytedance`] creates media models directly. The built-in
//! `create_provider("bytedance", settings)` also preserves the existing text
//! preset's chat, embeddings, and model discovery.

mod image_model;
mod options;
mod provider;
mod video_model;

pub use image_model::ByteDanceImageModel;
pub use provider::{ByteDanceProvider, ByteDanceProviderSettings, bytedance, create_bytedance};
pub use video_model::ByteDanceVideoModel;
