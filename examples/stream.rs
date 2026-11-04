//! Stream text deltas from an OpenAI chat model.
//!
//! Reads `OPENAI_API_KEY` when the request is sent.

use aimux_core::prelude::*;
use aimux_providers::openai::openai;
use futures::StreamExt;

#[tokio::main]
async fn main() -> Result<(), AiMuxError> {
    let model = openai().chat("gpt-4o");
    let result = stream_text(
        &model,
        "Write a haiku about Rust.",
        GenerateTextOptions::default(),
    )
    .await?;

    let mut stream = result.stream;
    while let Some(part) = stream.next().await {
        if let TextStreamPart::TextDelta { text, .. } = part? {
            print!("{text}");
        }
    }
    println!();
    Ok(())
}
