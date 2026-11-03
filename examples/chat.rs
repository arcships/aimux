//! One-shot Chat Completions call.
//!
//! `openai()` reads `OPENAI_API_KEY` when the request is sent.
//! `chat` is Chat Completions. `language_model` on this package selects
//! the Responses API instead.

use aimux_core::prelude::*;
use aimux_providers::openai::openai;

#[tokio::main]
async fn main() -> Result<(), AiMuxError> {
    let model = openai().chat("gpt-4o");
    let result = generate_text(
        &model,
        "Explain Rust ownership in one sentence.",
        GenerateTextOptions::default(),
    )
    .await?;
    println!("{}", result.text());
    Ok(())
}
