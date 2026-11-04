//! One call shape for a vendor package or a registry preset.
//!
//! ```text
//! cargo run -p aimux-examples --bin switch-provider
//! cargo run -p aimux-examples --bin switch-provider -- alibaba qwen-plus
//! cargo run -p aimux-examples --bin switch-provider -- moonshotai moonshot-v1-8k
//! cargo run -p aimux-examples --bin switch-provider -- zhipuai_coding_plan glm-4.5-air
//! ```
//!
//! The default pair is `deepseek` / `deepseek-chat` (`DEEPSEEK_API_KEY`).
//! `alibaba` is DashScope (Qwen / 通义); there is no package named `qwen`.
//! The pairs above are ids this repository's tests pass through. The key is
//! the preset's environment variable, read when the request is sent.

use aimux_core::prelude::*;
use aimux_providers::{PresetSettings, create_provider};

#[tokio::main]
async fn main() -> Result<(), AiMuxError> {
    let mut args = std::env::args().skip(1);
    let name = args.next().unwrap_or_else(|| "deepseek".to_string());
    let model_id = args.next().unwrap_or_else(|| "deepseek-chat".to_string());

    let provider = create_provider(&name, PresetSettings::default())?;
    let model = provider.language_model(&model_id)?;
    let result = generate_text(
        model.as_ref(),
        "Name one property of the Rust language.",
        GenerateTextOptions::default(),
    )
    .await?;
    println!("{}", result.text());
    Ok(())
}
