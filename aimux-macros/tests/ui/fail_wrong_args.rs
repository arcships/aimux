use aimux_macros::tool;
use serde_json::Value;

#[tool]
async fn wrong_args(first: Value, second: Value) -> Result<Value, String> {
    Ok(first)
}

fn main() {}
