use aimux_macros::tool;
use aimux_tools::ToolFn;
use serde_json::{Value, json};

#[tool("weather", "Returns current weather")]
async fn get_weather(args: Value) -> Result<Value, String> {
    Ok(args)
}

fn main() {
    let tool = WeatherTool;
    assert_eq!(tool.name(), "weather");
    assert_eq!(tool.definition().description.as_deref(), Some("Returns current weather"));
    let _ = json!({});
}
