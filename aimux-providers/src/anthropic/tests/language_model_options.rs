//! `anthropicLanguageModelOptions` (anthropic-language-model-options.ts) as
//! `parse_anthropic_options` reads the merged namespace. Not upstream cases:
//! upstream checks the schema through zod, which has no test file of its
//! own. What is kept, stripped and rejected is pinned here so the schema can
//! change form without changing what reaches the request.

use serde_json::{Value, json};

use aimux_core::error::AiMuxError;

use crate::anthropic::language_model_options::parse_anthropic_options;

fn parse(options: Value) -> Result<Value, AiMuxError> {
    parse_anthropic_options(options.as_object().unwrap()).map(Value::Object)
}

fn parsed(options: Value) -> Value {
    parse(options.clone()).unwrap_or_else(|error| panic!("{options}: {error}"))
}

#[test]
fn unknown_keys_are_stripped_and_the_input_order_is_kept() {
    let options = parsed(json!({
        "speed": "fast",
        "notAnOption": 1,
        "effort": "high",
        "sendReasoning": true,
        "anthropicBeta": ["a-beta"],
    }));
    assert_eq!(
        options.to_string(),
        r#"{"speed":"fast","effort":"high","sendReasoning":true,"anthropicBeta":["a-beta"]}"#
    );
}

#[test]
fn thinking_keeps_the_keys_of_its_variant() {
    // `adaptive`: `display` and `blockBinding`; `budgetTokens` belongs to
    // `enabled` and is stripped, as is an unknown key of `blockBinding`.
    assert_eq!(
        parsed(json!({ "thinking": {
            "type": "adaptive",
            "display": "summarized",
            "blockBinding": { "prefixMismatchBehavior": "drop_block", "extra": 1 },
            "budgetTokens": 5,
        } })),
        json!({ "thinking": {
            "type": "adaptive",
            "display": "summarized",
            "blockBinding": { "prefixMismatchBehavior": "drop_block" },
        } })
    );
    // `enabled`: `budgetTokens` only, written back as given.
    let enabled = parsed(json!({ "thinking": {
        "type": "enabled",
        "budgetTokens": 1024,
        "display": "omitted",
        "blockBinding": { "prefixMismatchBehavior": "error" },
    } }));
    assert_eq!(
        enabled.to_string(),
        r#"{"thinking":{"type":"enabled","budgetTokens":1024}}"#
    );
    for kind in ["disabled", "between_tools"] {
        assert_eq!(
            parsed(json!({ "thinking": {
                "type": kind,
                "display": "omitted",
                "budgetTokens": 5,
                "blockBinding": { "prefixMismatchBehavior": "error" },
            } })),
            json!({ "thinking": { "type": kind } })
        );
    }
    // No `type`: the `blockBinding`-only object.
    assert_eq!(
        parsed(json!({ "thinking": {
            "blockBinding": { "prefixMismatchBehavior": "error" },
            "display": "omitted",
        } })),
        json!({ "thinking": { "blockBinding": { "prefixMismatchBehavior": "error" } } })
    );
}

#[test]
fn nullish_mcp_server_fields_keep_null() {
    assert_eq!(
        parsed(json!({ "mcpServers": [
            {
                "type": "url",
                "name": "a",
                "url": "https://a.example/mcp",
                "authorizationToken": null,
                "toolConfiguration": { "enabled": null, "allowedTools": ["t"], "extra": 1 },
                "extra": 2,
            },
            {
                "type": "url",
                "name": "b",
                "url": "https://b.example/mcp",
                "authorizationToken": "token",
                "toolConfiguration": null,
            },
        ] })),
        json!({ "mcpServers": [
            {
                "type": "url",
                "name": "a",
                "url": "https://a.example/mcp",
                "authorizationToken": null,
                "toolConfiguration": { "enabled": null, "allowedTools": ["t"] },
            },
            {
                "type": "url",
                "name": "b",
                "url": "https://b.example/mcp",
                "authorizationToken": "token",
                "toolConfiguration": null,
            },
        ] })
    );
}

#[test]
fn container_skills_keep_the_keys_of_their_type() {
    assert_eq!(
        parsed(json!({ "container": {
            "id": "container-1",
            "skills": [
                {
                    "type": "anthropic",
                    "skillId": "pptx",
                    "version": "latest",
                    "providerReference": { "k": "v" },
                },
                {
                    "type": "custom",
                    "providerReference": { "anthropic": "skill_01" },
                    "skillId": "ignored",
                    "version": "2",
                },
            ],
            "extra": true,
        } })),
        json!({ "container": {
            "id": "container-1",
            "skills": [
                { "type": "anthropic", "skillId": "pptx", "version": "latest" },
                { "type": "custom", "providerReference": { "anthropic": "skill_01" }, "version": "2" },
            ],
        } })
    );
}

#[test]
fn fallbacks_task_budget_safeguards_and_compaction() {
    assert_eq!(
        parsed(json!({ "fallbacks": "default" })),
        json!({ "fallbacks": "default" })
    );
    // The records are kept whole; an unknown key of an entry is stripped.
    let fallbacks = parsed(json!({ "fallbacks": [{
        "model": "claude-sonnet-4-5",
        "max_tokens": 100,
        "thinking": { "type": "enabled", "budget_tokens": 1 },
        "output_config": { "effort": "low", "nested": { "x": [1] } },
        "speed": "fast",
        "extra": 1,
    }] }));
    assert_eq!(
        fallbacks.to_string(),
        r#"{"fallbacks":[{"model":"claude-sonnet-4-5","max_tokens":100,"thinking":{"type":"enabled","budget_tokens":1},"output_config":{"effort":"low","nested":{"x":[1]}},"speed":"fast"}]}"#
    );
    let budget = parsed(json!({ "taskBudget": {
        "type": "tokens",
        "total": 20000,
        "remaining": 0,
        "extra": 1,
    } }));
    assert_eq!(
        budget.to_string(),
        r#"{"taskBudget":{"type":"tokens","total":20000,"remaining":0}}"#
    );
    assert_eq!(
        parsed(json!({ "safeguards": [{
            "type": "dangerous_tool_use",
            "classifierContext": { "a": 1 },
            "extra": 1,
        }] })),
        json!({ "safeguards": [{ "type": "dangerous_tool_use", "classifierContext": { "a": 1 } }] })
    );
    assert_eq!(
        parsed(
            json!({ "compaction": { "type": "summarize", "instructions": "short", "extra": 1 } })
        ),
        json!({ "compaction": { "type": "summarize", "instructions": "short" } })
    );
    assert_eq!(
        parsed(json!({ "cacheControl": { "type": "ephemeral", "ttl": "1h", "extra": 1 } })),
        json!({ "cacheControl": { "type": "ephemeral", "ttl": "1h" } })
    );
    assert_eq!(
        parsed(json!({ "metadata": { "userId": "u", "extra": 1 } })),
        json!({ "metadata": { "userId": "u" } })
    );
}

#[test]
fn context_management_edits_keep_the_keys_of_their_type() {
    let options = parsed(json!({ "contextManagement": { "edits": [
        {
            "type": "clear_tool_uses_20250919",
            "trigger": { "type": "tool_uses", "value": 3 },
            "keep": { "type": "tool_uses", "value": 1 },
            "clearAtLeast": { "type": "input_tokens", "value": 1000 },
            "clearToolInputs": true,
            "excludeTools": ["search"],
            "instructions": "stripped",
            "pauseAfterCompaction": true,
        },
        {
            "type": "clear_thinking_20251015",
            "keep": "all",
            "trigger": { "type": "input_tokens", "value": 1 },
        },
        {
            "type": "clear_thinking_20251015",
            "keep": { "type": "thinking_turns", "value": 2, "extra": 1 },
        },
        {
            "type": "compact_20260112",
            "trigger": { "type": "input_tokens", "value": 150000 },
            "pauseAfterCompaction": false,
            "instructions": "keep the plan",
            "keep": "all",
            "clearToolInputs": true,
        },
    ], "extra": 1 } }));
    assert_eq!(
        options.to_string(),
        concat!(
            r#"{"contextManagement":{"edits":["#,
            r#"{"type":"clear_tool_uses_20250919","trigger":{"type":"tool_uses","value":3},"keep":{"type":"tool_uses","value":1},"clearAtLeast":{"type":"input_tokens","value":1000},"clearToolInputs":true,"excludeTools":["search"]},"#,
            r#"{"type":"clear_thinking_20251015","keep":"all"},"#,
            r#"{"type":"clear_thinking_20251015","keep":{"type":"thinking_turns","value":2}},"#,
            r#"{"type":"compact_20260112","trigger":{"type":"input_tokens","value":150000},"pauseAfterCompaction":false,"instructions":"keep the plan"}"#,
            r#"]}}"#,
        )
    );
}

#[test]
fn values_outside_the_schema_are_rejected() {
    for options in [
        // `.optional()`: `null` fails as a wrong type does.
        json!({ "sendReasoning": null }),
        json!({ "thinking": null }),
        json!({ "structuredOutputMode": "json" }),
        json!({ "effort": "extreme" }),
        json!({ "speed": "slow" }),
        json!({ "serviceTier": "priority" }),
        json!({ "inferenceGeo": "eu" }),
        json!({ "anthropicBeta": "a-beta" }),
        json!({ "anthropicBeta": [1] }),
        json!({ "thinking": { "type": "bogus" } }),
        json!({ "thinking": {} }),
        json!({ "thinking": { "type": null, "blockBinding": { "prefixMismatchBehavior": "error" } } }),
        json!({ "thinking": { "type": "adaptive", "display": null } }),
        json!({ "thinking": { "type": "adaptive", "display": "full" } }),
        json!({ "thinking": { "type": "adaptive", "blockBinding": {} } }),
        json!({ "thinking": { "blockBinding": { "prefixMismatchBehavior": "keep" } } }),
        json!({ "thinking": { "type": "enabled", "budgetTokens": "1024" } }),
        json!({ "cacheControl": { "ttl": "5m" } }),
        json!({ "cacheControl": { "type": "ephemeral", "ttl": "10m" } }),
        json!({ "metadata": { "userId": 1 } }),
        json!({ "mcpServers": [{ "type": "url", "name": "a" }] }),
        json!({ "mcpServers": [{ "type": "stdio", "name": "a", "url": "u" }] }),
        json!({ "mcpServers": [{ "type": "url", "name": "a", "url": "u", "authorizationToken": 1 }] }),
        json!({ "mcpServers": [{
            "type": "url", "name": "a", "url": "u",
            "toolConfiguration": { "allowedTools": [1] },
        }] }),
        json!({ "mcpServers": { "type": "url", "name": "a", "url": "u" } }),
        json!({ "container": { "id": 1 } }),
        json!({ "container": { "skills": [{ "type": "anthropic" }] } }),
        json!({ "container": { "skills": [{ "type": "custom", "providerReference": { "k": 1 } }] } }),
        json!({ "container": { "skills": [{ "type": "custom", "skillId": "s" }] } }),
        json!({ "container": { "skills": [{ "skillId": "s" }] } }),
        json!({ "taskBudget": { "type": "tokens", "total": 19999 } }),
        json!({ "taskBudget": { "type": "tokens", "total": 20000.5 } }),
        json!({ "taskBudget": { "type": "tokens", "total": 20000, "remaining": -1 } }),
        json!({ "taskBudget": { "type": "tokens" } }),
        json!({ "taskBudget": { "type": "requests", "total": 20000 } }),
        json!({ "fallbacks": "none" }),
        json!({ "fallbacks": [{ "max_tokens": 1 }] }),
        json!({ "fallbacks": [{ "model": "m", "max_tokens": 1.5 }] }),
        json!({ "fallbacks": [{ "model": "m", "thinking": "enabled" }] }),
        json!({ "fallbacks": [{ "model": "m", "speed": "slow" }] }),
        json!({ "safeguards": [{ "type": "other" }] }),
        json!({ "safeguards": [{ "type": "dangerous_tool_use", "classifierContext": [] }] }),
        json!({ "compaction": {} }),
        json!({ "compaction": { "type": "summarize", "instructions": 1 } }),
        json!({ "contextManagement": {} }),
        json!({ "contextManagement": { "edits": [{ "type": "unknown" }] } }),
        json!({ "contextManagement": { "edits": [{
            "type": "clear_tool_uses_20250919",
            "trigger": { "type": "tool_uses" },
        }] } }),
        json!({ "contextManagement": { "edits": [{
            "type": "clear_tool_uses_20250919",
            "keep": { "type": "input_tokens", "value": 1 },
        }] } }),
        json!({ "contextManagement": { "edits": [{
            "type": "compact_20260112",
            "trigger": { "type": "tool_uses", "value": 1 },
        }] } }),
        json!({ "contextManagement": { "edits": [{
            "type": "clear_thinking_20251015",
            "keep": "some",
        }] } }),
        json!({ "contextManagement": { "edits": [{
            "type": "clear_thinking_20251015",
            "keep": { "type": "thinking_turns", "value": "2" },
        }] } }),
    ] {
        assert!(
            parse(options.clone()).is_err(),
            "{options} should be rejected"
        );
    }
}
