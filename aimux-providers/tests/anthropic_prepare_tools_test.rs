//! Rust port of the provider-defined-tools section of
//! `anthropic-prepare-tools.test.ts`.
//!
//! These exercise [`prepare_tools_with_provider`], which handles the Vercel AI
//! SDK `type: 'provider'` tools (computer use, text editor, bash, web search,
//! web fetch, tool search, code execution, advisor) in addition to function
//! tools. Cache-control resolution and response-schema validation tests remain
//! out of scope (they need a `CacheControlValidator` / response-schema utility).

use aimux_core::tool::ToolChoice;
use aimux_core::types::Warning;
use aimux_providers::anthropic::prepare_tools::{AnthropicTool, prepare_tools_with_provider};
use serde_json::{Value, json};

fn provider(id: &str, name: &str, args: Value) -> AnthropicTool {
    AnthropicTool::Provider {
        id: id.to_string(),
        name: name.to_string(),
        args,
    }
}

fn run(tools: Vec<AnthropicTool>) -> aimux_providers::anthropic::prepare_tools::PreparedTools {
    prepare_tools_with_provider(Some(&tools), None, false, true, true, false)
}

fn assert_unsupported_warning(warnings: &[Warning], feature: &str) {
    let found = warnings.iter().any(|w| match w {
        Warning::Unsupported { feature: f, .. } => f == feature,
        _ => false,
    });
    assert!(
        found,
        "expected Unsupported warning for {feature:?}, got {warnings:?}"
    );
}

// ---------------------------------------------------------------------------
// computer use
// ---------------------------------------------------------------------------

#[test]
fn should_correctly_prepare_computer_20241022_tool() {
    let result = run(vec![provider(
        "anthropic.computer_20241022",
        "computer",
        json!({ "displayWidthPx": 800, "displayHeightPx": 600, "displayNumber": 1 }),
    )]);
    assert_eq!(
        result.tools.unwrap()[0],
        json!({
            "name": "computer",
            "type": "computer_20241022",
            "display_width_px": 800,
            "display_height_px": 600,
            "display_number": 1,
        })
    );
    assert!(result.betas.contains("computer-use-2024-10-22"));
}

#[test]
fn should_correctly_prepare_computer_20250124_tool() {
    let result = run(vec![provider(
        "anthropic.computer_20250124",
        "computer",
        json!({ "displayWidthPx": 1024, "displayHeightPx": 768, "displayNumber": 1 }),
    )]);
    assert_eq!(
        result.tools.unwrap()[0],
        json!({
            "name": "computer",
            "type": "computer_20250124",
            "display_width_px": 1024,
            "display_height_px": 768,
            "display_number": 1,
        })
    );
    assert!(result.betas.contains("computer-use-2025-01-24"));
}

#[test]
fn should_correctly_prepare_computer_20251124_tool() {
    let result = run(vec![provider(
        "anthropic.computer_20251124",
        "computer",
        json!({ "displayWidthPx": 1024, "displayHeightPx": 768, "displayNumber": 1 }),
    )]);
    assert_eq!(
        result.tools.unwrap()[0],
        json!({
            "name": "computer",
            "type": "computer_20251124",
            "display_width_px": 1024,
            "display_height_px": 768,
            "display_number": 1,
            "enable_zoom": Value::Null,
        })
    );
    assert!(result.betas.contains("computer-use-2025-11-24"));
}

#[test]
fn should_correctly_prepare_computer_20251124_tool_with_enable_zoom() {
    let result = run(vec![provider(
        "anthropic.computer_20251124",
        "computer",
        json!({ "displayWidthPx": 1024, "displayHeightPx": 768, "displayNumber": 1, "enableZoom": true }),
    )]);
    assert_eq!(result.tools.unwrap()[0]["enable_zoom"], json!(true));
}

#[test]
fn should_correctly_prepare_computer_20251124_tool_with_enable_zoom_false() {
    let result = run(vec![provider(
        "anthropic.computer_20251124",
        "computer",
        json!({ "displayWidthPx": 1024, "displayHeightPx": 768, "displayNumber": 1, "enableZoom": false }),
    )]);
    assert_eq!(result.tools.unwrap()[0]["enable_zoom"], json!(false));
}

// ---------------------------------------------------------------------------
// text editor / bash
// ---------------------------------------------------------------------------

#[test]
fn should_correctly_prepare_text_editor_20241022_tool() {
    let result = run(vec![provider(
        "anthropic.text_editor_20241022",
        "text_editor",
        json!({}),
    )]);
    assert_eq!(
        result.tools.unwrap()[0],
        json!({ "name": "str_replace_editor", "type": "text_editor_20241022" })
    );
    assert!(result.betas.contains("computer-use-2024-10-22"));
}

#[test]
fn should_correctly_prepare_bash_20241022_tool() {
    let result = run(vec![provider("anthropic.bash_20241022", "bash", json!({}))]);
    assert_eq!(
        result.tools.unwrap()[0],
        json!({ "name": "bash", "type": "bash_20241022" })
    );
    assert!(result.betas.contains("computer-use-2024-10-22"));
}

#[test]
fn should_correctly_prepare_text_editor_20250728_with_max_characters() {
    let result = run(vec![provider(
        "anthropic.text_editor_20250728",
        "str_replace_based_edit_tool",
        json!({ "maxCharacters": 10000 }),
    )]);
    assert_eq!(
        result.tools.unwrap()[0],
        json!({
            "name": "str_replace_based_edit_tool",
            "type": "text_editor_20250728",
            "max_characters": 10000,
        })
    );
    assert!(result.betas.is_empty());
}

#[test]
fn should_correctly_prepare_text_editor_20250728_without_max_characters() {
    let result = run(vec![provider(
        "anthropic.text_editor_20250728",
        "str_replace_based_edit_tool",
        json!({}),
    )]);
    assert_eq!(
        result.tools.unwrap()[0],
        json!({
            "name": "str_replace_based_edit_tool",
            "type": "text_editor_20250728",
            "max_characters": Value::Null,
        })
    );
}

// ---------------------------------------------------------------------------
// web search / web fetch
// ---------------------------------------------------------------------------

#[test]
fn should_correctly_prepare_web_search_20250305() {
    let result = run(vec![provider(
        "anthropic.web_search_20250305",
        "web_search",
        json!({
            "maxUses": 10,
            "allowedDomains": ["https://www.google.com"],
            "userLocation": { "type": "approximate", "city": "New York" },
        }),
    )]);
    assert_eq!(
        result.tools.unwrap()[0],
        json!({
            "type": "web_search_20250305",
            "name": "web_search",
            "max_uses": 10,
            "allowed_domains": ["https://www.google.com"],
            "blocked_domains": Value::Null,
            "user_location": { "city": "New York", "type": "approximate" },
        })
    );
    assert!(result.betas.is_empty());
}

#[test]
fn should_correctly_prepare_web_search_20260209() {
    let result = run(vec![provider(
        "anthropic.web_search_20260209",
        "web_search",
        json!({
            "maxUses": 10,
            "allowedDomains": ["https://www.google.com"],
            "userLocation": { "type": "approximate", "city": "New York" },
        }),
    )]);
    assert_eq!(
        result.tools.unwrap()[0]["type"],
        json!("web_search_20260209")
    );
    assert!(result.betas.contains("code-execution-web-tools-2026-02-09"));
}

#[test]
fn should_correctly_prepare_web_fetch_20250910() {
    let result = run(vec![provider(
        "anthropic.web_fetch_20250910",
        "web_fetch",
        json!({
            "maxUses": 10,
            "allowedDomains": ["https://www.google.com"],
            "citations": { "enabled": true },
            "maxContentTokens": 1000,
        }),
    )]);
    assert_eq!(
        result.tools.unwrap()[0],
        json!({
            "type": "web_fetch_20250910",
            "name": "web_fetch",
            "max_uses": 10,
            "allowed_domains": ["https://www.google.com"],
            "blocked_domains": Value::Null,
            "citations": { "enabled": true },
            "max_content_tokens": 1000,
        })
    );
    assert!(result.betas.contains("web-fetch-2025-09-10"));
}

#[test]
fn should_correctly_prepare_web_fetch_20260209() {
    let result = run(vec![provider(
        "anthropic.web_fetch_20260209",
        "web_fetch",
        json!({
            "maxUses": 10,
            "allowedDomains": ["https://www.google.com"],
            "citations": { "enabled": true },
            "maxContentTokens": 1000,
        }),
    )]);
    assert_eq!(
        result.tools.unwrap()[0]["type"],
        json!("web_fetch_20260209")
    );
    assert!(result.betas.contains("code-execution-web-tools-2026-02-09"));
}

// ---------------------------------------------------------------------------
// tool search / code execution
// ---------------------------------------------------------------------------

#[test]
fn should_correctly_prepare_tool_search_regex_20251119() {
    let result = run(vec![provider(
        "anthropic.tool_search_regex_20251119",
        "tool_search",
        json!({}),
    )]);
    assert_eq!(
        result.tools.unwrap()[0],
        json!({ "name": "tool_search_tool_regex", "type": "tool_search_tool_regex_20251119" })
    );
    assert!(result.betas.is_empty());
}

#[test]
fn should_correctly_prepare_tool_search_bm25_20251119() {
    let result = run(vec![provider(
        "anthropic.tool_search_bm25_20251119",
        "tool_search",
        json!({}),
    )]);
    assert_eq!(
        result.tools.unwrap()[0],
        json!({ "name": "tool_search_tool_bm25", "type": "tool_search_tool_bm25_20251119" })
    );
    assert!(result.betas.is_empty());
}

#[test]
fn should_correctly_prepare_code_execution_20260120_without_beta_header() {
    let result = run(vec![provider(
        "anthropic.code_execution_20260120",
        "code_execution",
        json!({}),
    )]);
    assert_eq!(
        result.tools.unwrap()[0],
        json!({ "type": "code_execution_20260120", "name": "code_execution" })
    );
    assert!(result.betas.is_empty());
}

// ---------------------------------------------------------------------------
// advisor
// ---------------------------------------------------------------------------

#[test]
fn should_correctly_prepare_advisor_20260301_with_only_the_required_model() {
    let result = run(vec![provider(
        "anthropic.advisor_20260301",
        "advisor",
        json!({ "model": "claude-opus-4-7" }),
    )]);
    assert_eq!(
        result.tools.unwrap()[0],
        json!({
            "type": "advisor_20260301",
            "name": "advisor",
            "model": "claude-opus-4-7",
        })
    );
    assert!(result.betas.contains("advisor-tool-2026-03-01"));
}

#[test]
fn should_correctly_prepare_advisor_20260301_with_all_optional_args() {
    let result = run(vec![provider(
        "anthropic.advisor_20260301",
        "advisor",
        json!({
            "model": "claude-opus-4-7",
            "maxUses": 5,
            "caching": { "type": "ephemeral", "ttl": "1h" },
        }),
    )]);
    assert_eq!(
        result.tools.unwrap()[0],
        json!({
            "type": "advisor_20260301",
            "name": "advisor",
            "model": "claude-opus-4-7",
            "max_uses": 5,
            "caching": { "type": "ephemeral", "ttl": "1h" },
        })
    );
    assert!(result.betas.contains("advisor-tool-2026-03-01"));
}

// ---------------------------------------------------------------------------
// unsupported tools
// ---------------------------------------------------------------------------

#[test]
fn should_add_warnings_for_unsupported_tools() {
    let result = run(vec![provider(
        "unsupported.tool",
        "unsupported_tool",
        json!({}),
    )]);
    // The unsupported tool is dropped from the tools array (which stays empty,
    // not None, because the input was non-empty).
    assert_eq!(result.tools, Some(vec![]));
    assert!(result.tool_choice.is_none());
    assert_eq!(result.tool_warnings.len(), 1);
    assert_unsupported_warning(
        &result.tool_warnings,
        "provider-defined tool unsupported.tool",
    );
}

// ---------------------------------------------------------------------------
// tool choice still works with provider tools
// ---------------------------------------------------------------------------

#[test]
fn should_handle_tool_choice_tool_with_provider_tools() {
    let tools = vec![provider("anthropic.bash_20241022", "bash", json!({}))];
    let tc = ToolChoice::Tool {
        tool_name: "bash".to_string(),
    };
    let result = prepare_tools_with_provider(Some(&tools), Some(&tc), false, true, true, false);
    assert_eq!(
        result.tool_choice.unwrap(),
        json!({ "type": "tool", "name": "bash" })
    );
}
