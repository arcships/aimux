use aimux_macros::tool;
use serde_json::Value;

#[tool]
async fn wrong_return(args: Value) -> Value {
    args
}

fn main() {}
