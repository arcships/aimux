//! `GET /api/providers` — provider list + suggested models (RFC-0029 §5.1).

use axum::Json;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

/// Provider names: every built-in provider.
pub fn provider_names() -> Vec<String> {
    aimux_providers::default_providers().into_keys().collect()
}

/// `GET /api/providers` — the provider list and suggested models for common ones.
pub async fn list() -> Response {
    let suggested: Value = json!({
        "openai": ["gpt-4o", "gpt-4o-mini", "o3-mini", "gpt-4.1"],
        "anthropic": ["claude-sonnet-4-20250514", "claude-haiku-4-5-20251001"],
        "google": ["gemini-2.5-flash", "gemini-2.5-pro"],
        "mistral": ["mistral-large-latest", "mistral-small-latest"],
        "xai": ["grok-4", "grok-3-mini"],
        "cohere": ["command-r-plus", "command-a"],
        "deepseek": ["deepseek-chat", "deepseek-reasoner"],
        "groq": ["llama-3.3-70b-versatile", "llama-3.1-8b-instant"],
        "openrouter": ["openai/gpt-4o", "anthropic/claude-3.7-sonnet", "deepseek/deepseek-r1"],
        "ollama": ["llama3.1", "qwen2.5", "deepseek-r1:8b"],
        "siliconflow": ["deepseek-ai/DeepSeek-V3", "Qwen/Qwen2.5-72B-Instruct"],
        "moonshotai": ["moonshot-v1-32k", "kimi-k2-0711-preview"],
        "zhipu": ["glm-4-plus", "glm-4-flash"],
        "qwen": ["qwen-max", "qwen-plus"],
        "openai-compatible": ["custom model id…"],
    });
    Json(json!({
        "providers": provider_names(),
        "suggested_models": suggested,
    }))
    .into_response()
}
