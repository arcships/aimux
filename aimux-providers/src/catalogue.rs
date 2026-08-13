//! Model specs community catalogue (RFC-0027).
//!
//! Provides [`get_model_specs`] — a thin function that fetches the community
//! model catalogue (anya2a `dist/all.json`) and returns a [`Catalogue`] of
//! [`ModelSpec`] entries. This is **deliberately decoupled** from
//! `Provider::list_models`: the host calls both independently and merges them
//! as needed.
//!
//! `get_model_specs` does **no caching, no FS writes, no TTL** — it is a pure
//! fetch + parse. The host decides how to cache/persist the result.

use std::collections::HashMap;
use std::time::Duration;

use aimux_core::AiMuxError;
use aimux_core::ApiCallError;
use aimux_core::model_catalogue::{
    CatalogueSource, Modality, ModelCapabilities, ModelCost, ModelLimits, ModelModalities,
    ModelSpec, ModelType, ReasoningMode, ReasoningSpec, ReasoningVisibility,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Default anya2a aggregate endpoint (raw `dist/all.json`).
pub const DEFAULT_ANYA2A_URL: &str = "https://raw.githubusercontent.com/ThinkInAIXYZ/PublicProviderConf/refs/heads/dev/dist/all.json";

// ─────────────────────────────────────────────────────────────────────────────
// Provider name normalization
// ─────────────────────────────────────────────────────────────────────────────

const PROVIDER_ALIASES: &[(&str, &str)] = &[
    ("fireworks-ai", "fireworks"),
    ("google-vertex", "vertex"),
    ("google-vertex-anthropic", "vertex"),
    ("amazon-bedrock", "bedrock"),
    ("azure-cognitive-services", "azure_ai"),
    ("moonshot-ai", "moonshot"),
    ("moonshot", "moonshot"),
    ("stepfun-ai", "stepfun"),
    ("siliconflow-com", "siliconflow"),
    ("tencent-tokenhub", "tencent_tokenhub"),
    ("alibaba-cn", "alibaba"),
    ("minimax-cn", "minimax"),
    ("xiaomi-token-plan-ams", "xiaomi"),
    ("xiaomi-token-plan-cn", "xiaomi"),
    ("xiaomi-token-plan-sgp", "xiaomi"),
    ("zhipuai", "zhipu"),
    ("zhipuai-coding-plan", "zhipu_coding_plan"),
    ("zai-coding-plan", "zai_coding_plan"),
    ("opencode-go", "opencode"),
    ("tencent-coding-plan", "tencent_coding_plan"),
    ("umans-ai-coding-plan", "umans_ai_coding_plan"),
    ("alibaba-coding-plan-cn", "alibaba_coding_plan_cn"),
    ("alibaba-coding-plan", "alibaba_coding_plan"),
    ("alibaba-token-plan-cn", "alibaba_token_plan_cn"),
    ("alibaba-token-plan", "alibaba_token_plan"),
    ("stepfun-ai-step-plan", "stepfun"),
    ("stepfun-step-plan", "stepfun"),
    ("minimax-cn-coding-plan", "minimax_coding_plan"),
    ("minimax-coding-plan", "minimax_coding_plan"),
    ("kimi-for-coding", "kimi"),
    ("cloudflare-ai-gateway", "cloudflare_ai_gateway"),
    ("cloudflare-workers-ai", "cloudflare_workers_ai"),
    ("google-pse", "google_pse"),
    ("model-oracle-ai", "model_oracle_ai"),
    ("regolo-ai", "regolo_ai"),
    ("sapphire-ai", "sapphire"),
    ("umans-ai", "umans_ai"),
    ("thinkingmachines", "thinkingmachines"),
];

/// Map an anya2a provider id to an aimux registry name.
pub fn normalize_provider_name(anya2a_id: &str) -> String {
    for (from, to) in PROVIDER_ALIASES {
        if *from == anya2a_id {
            return (*to).to_string();
        }
    }
    anya2a_id.replace('-', "_")
}

// ─────────────────────────────────────────────────────────────────────────────
// Catalogue (in-memory)
// ─────────────────────────────────────────────────────────────────────────────

/// In-memory catalogue: per-provider model specs, keyed by aimux provider
/// name then model id.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Catalogue {
    #[serde(default)]
    pub updated_at: u64,
    #[serde(default)]
    pub specs: HashMap<String, HashMap<String, ModelSpec>>,
}

impl Catalogue {
    /// Look up the portrait for `(provider, model_id)`.
    pub fn lookup(&self, provider: &str, model_id: &str) -> Option<ModelSpec> {
        self.specs.get(provider)?.get(model_id).cloned()
    }

    pub fn provider_count(&self) -> usize {
        self.specs.len()
    }
    pub fn model_count(&self) -> usize {
        self.specs.values().map(|m| m.len()).sum()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// get_model_specs — thin fetch + parse
// ─────────────────────────────────────────────────────────────────────────────

/// Fetch the community model catalogue and return it as a [`Catalogue`].
///
/// This is a **thin function**: fetch `all.json` from the source URL, parse it
/// into `ModelSpec` entries, return. No caching, no FS writes, no TTL — the
/// host decides how to manage the result.
///
/// `source_url` defaults to [`DEFAULT_ANYA2A_URL`] (anya2a `dist/all.json`).
pub async fn get_model_specs(source_url: Option<&str>) -> Result<Catalogue, AiMuxError> {
    let url = source_url.unwrap_or(DEFAULT_ANYA2A_URL);
    let body = fetch_text(url).await?;
    let raw: Value = serde_json::from_str(&body)
        .map_err(|e| AiMuxError::JsonParse(format!("get_model_specs: invalid source JSON: {e}")))?;
    parse_anya2a_all(&raw)
}

/// Parse anya2a `dist/all.json` into a [`Catalogue`].
pub fn parse_anya2a_all(json: &Value) -> Result<Catalogue, AiMuxError> {
    let providers = json
        .get("providers")
        .and_then(|v| v.as_object())
        .ok_or_else(|| {
            AiMuxError::JsonParse("anya2a all.json missing 'providers' object".into())
        })?;
    let updated_at = json
        .get("updated_at")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);

    let mut specs: HashMap<String, HashMap<String, ModelSpec>> = HashMap::new();
    for (anya2a_id, provider_val) in providers {
        let aimux_name = normalize_provider_name(anya2a_id);
        let models = provider_val.get("models").and_then(|v| v.as_array());
        let Some(models) = models else {
            continue;
        };
        let bucket = specs.entry(aimux_name).or_default();
        for m in models {
            let Some(id) = m.get("id").and_then(|v| v.as_str()) else {
                continue;
            };
            let spec = convert_anya2a_model(m);
            bucket.insert(id.to_string(), spec);
        }
    }
    Ok(Catalogue { updated_at, specs })
}

// ─────────────────────────────────────────────────────────────────────────────
// anya2a → ModelSpec conversion
// ─────────────────────────────────────────────────────────────────────────────

fn convert_anya2a_model(m: &Value) -> ModelSpec {
    let display_name = m
        .get("display_name")
        .and_then(|v| v.as_str())
        .map(String::from);
    let r#type = m
        .get("type")
        .and_then(|v| v.as_str())
        .map(parse_model_type)
        .unwrap_or_default();

    let limits = ModelLimits {
        context: num_field(m, &["limit", "context"]),
        output: num_field(m, &["limit", "output"]),
        input: num_field(m, &["limit", "input"]),
    };

    let modalities = ModelModalities {
        input: modality_list(m, &["modalities", "input"]),
        output: modality_list(m, &["modalities", "output"]),
    };

    let capabilities = ModelCapabilities {
        tool_call: bool_field(m, "tool_call"),
        structured_output: bool_field(m, "structured_output"),
        temperature: bool_field(m, "temperature"),
        attachment: bool_field(m, "attachment"),
    };

    let reasoning = build_reasoning_spec(m);
    let cost = build_cost(m);

    ModelSpec {
        display_name,
        r#type,
        limits,
        modalities,
        capabilities,
        reasoning,
        cost,
        source: CatalogueSource::Anya2a,
        provider: None,
        raw: Some(m.clone()),
    }
}

fn parse_model_type(s: &str) -> ModelType {
    match s {
        "chat" => ModelType::Chat,
        "completion" => ModelType::Completion,
        "embedding" => ModelType::Embedding,
        "rerank" | "reranker" => ModelType::Rerank,
        "image-generation" | "imageGeneration" => ModelType::ImageGen,
        "image-edit" => ModelType::ImageEdit,
        "video" => ModelType::Video,
        "audio" => ModelType::Audio,
        _ => ModelType::Other,
    }
}

fn parse_modality(s: &str) -> Modality {
    match s {
        "text" => Modality::Text,
        "image" => Modality::Image,
        "audio" => Modality::Audio,
        "video" => Modality::Video,
        "pdf" => Modality::Pdf,
        "embedding" => Modality::Embedding,
        "score" => Modality::Score,
        _ => Modality::Other,
    }
}

fn num_field(m: &Value, path: &[&str]) -> Option<u64> {
    let mut v = m;
    for p in path {
        v = v.get(*p)?;
    }
    v.as_u64().or_else(|| v.as_f64().map(|f| f as u64))
}

fn bool_field(m: &Value, key: &str) -> bool {
    m.get(key).and_then(|v| v.as_bool()).unwrap_or(false)
}

fn modality_list(m: &Value, path: &[&str]) -> Vec<Modality> {
    let mut v = m;
    for p in path {
        v = match v.get(*p) {
            Some(x) => x,
            None => return Vec::new(),
        };
    }
    v.as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(parse_modality))
                .collect()
        })
        .unwrap_or_default()
}

fn build_reasoning_spec(m: &Value) -> Option<ReasoningSpec> {
    let supported = match m.get("reasoning") {
        Some(Value::Bool(b)) => *b,
        Some(obj) => obj
            .get("supported")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        None => false,
    };
    let default_enabled = m
        .get("reasoning")
        .and_then(|v| v.get("default"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let ec = m.get("extra_capabilities").and_then(|v| v.get("reasoning"));

    let mode = ec
        .and_then(|v| v.get("mode"))
        .and_then(|v| v.as_str())
        .map(parse_reasoning_mode);

    let effort_default = ec
        .and_then(|v| v.get("effort"))
        .and_then(|v| v.as_str())
        .map(String::from);
    let effort_options: Vec<String> = ec
        .and_then(|v| v.get("effort_options"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let budget_min = ec
        .and_then(|v| v.get("budget"))
        .and_then(|v| v.get("min"))
        .and_then(|v| v.as_u64())
        .or_else(|| {
            m.get("reasoning_options")
                .and_then(|v| v.as_array())
                .and_then(|arr| {
                    arr.iter().find_map(|o| {
                        let is_budget = o
                            .get("type")
                            .and_then(|t| t.as_str())
                            .is_some_and(|t| t == "budget_tokens");
                        if is_budget {
                            o.get("min").and_then(|v| v.as_u64())
                        } else {
                            None
                        }
                    })
                })
        });
    let interleaved = ec
        .and_then(|v| v.get("interleaved"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let visibility = ec
        .and_then(|v| v.get("visibility"))
        .and_then(|v| v.as_str())
        .map(parse_reasoning_visibility);

    if !supported
        && mode.is_none()
        && effort_default.is_none()
        && effort_options.is_empty()
        && budget_min.is_none()
        && ec.is_none()
    {
        return None;
    }

    Some(ReasoningSpec {
        supported,
        default_enabled,
        mode,
        effort_default,
        effort_options,
        budget_min,
        interleaved,
        visibility,
    })
}

fn parse_reasoning_mode(s: &str) -> ReasoningMode {
    match s {
        "budget" => ReasoningMode::Budget,
        "effort" => ReasoningMode::Effort,
        "level" => ReasoningMode::Level,
        "fixed" => ReasoningMode::Fixed,
        "mixed" => ReasoningMode::Mixed,
        "toggle" => ReasoningMode::Toggle,
        _ => ReasoningMode::Other,
    }
}

fn parse_reasoning_visibility(s: &str) -> ReasoningVisibility {
    match s {
        "hidden" => ReasoningVisibility::Hidden,
        "summary" => ReasoningVisibility::Summary,
        "full" => ReasoningVisibility::Full,
        "mixed" => ReasoningVisibility::Mixed,
        "omitted" => ReasoningVisibility::Omitted,
        _ => ReasoningVisibility::Other,
    }
}

fn build_cost(m: &Value) -> Option<ModelCost> {
    let cost = m.get("cost")?;
    if cost.is_null() {
        return None;
    }
    Some(ModelCost {
        input: cost.get("input").and_then(|v| v.as_f64()),
        output: cost.get("output").and_then(|v| v.as_f64()),
        cache_read: cost.get("cache_read").and_then(|v| v.as_f64()),
        cache_write: cost.get("cache_write").and_then(|v| v.as_f64()),
    })
}

async fn fetch_text(url: &str) -> Result<String, AiMuxError> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| {
            AiMuxError::ApiCall(ApiCallError {
                message: format!("get_model_specs: build client: {e}"),
                is_retryable: true,
                ..Default::default()
            })
        })?;
    let resp = client.get(url).send().await.map_err(|e| {
        AiMuxError::ApiCall(ApiCallError {
            message: format!("get_model_specs: fetch {url}: {e}"),
            is_retryable: true,
            ..Default::default()
        })
    })?;
    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        return Err(AiMuxError::ApiCall(ApiCallError {
            status_code: Some(status),
            message: format!("get_model_specs: fetch {url}"),
            response_body: resp.text().await.ok().filter(|b| !b.is_empty()),
            is_retryable: status == 429 || status >= 500,
            ..Default::default()
        }));
    }
    resp.text().await.map_err(|e| {
        AiMuxError::ApiCall(ApiCallError {
            message: format!("get_model_specs: read body: {e}"),
            is_retryable: true,
            ..Default::default()
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normalize_known_aliases() {
        assert_eq!(normalize_provider_name("fireworks-ai"), "fireworks");
        assert_eq!(normalize_provider_name("google-vertex"), "vertex");
        assert_eq!(normalize_provider_name("amazon-bedrock"), "bedrock");
        assert_eq!(normalize_provider_name("zhipuai"), "zhipu");
    }

    #[test]
    fn normalize_fallback_kebab_to_snake() {
        assert_eq!(normalize_provider_name("some-provider"), "some_provider");
        assert_eq!(normalize_provider_name("deepseek"), "deepseek");
    }

    #[test]
    fn normalize_multi_target_aliases() {
        assert_eq!(normalize_provider_name("google-vertex-anthropic"), "vertex");
        assert_eq!(normalize_provider_name("xiaomi-token-plan-ams"), "xiaomi");
        assert_eq!(
            normalize_provider_name("zhipuai-coding-plan"),
            "zhipu_coding_plan"
        );
        assert_eq!(normalize_provider_name("siliconflow-com"), "siliconflow");
    }

    #[test]
    fn parse_minimal_anya2a() {
        let raw = json!({
            "updated_at": "1785976654990",
            "providers": {
                "deepseek": {
                    "models": [{
                        "id": "deepseek-chat",
                        "type": "chat",
                        "tool_call": true,
                        "structured_output": true,
                        "temperature": true,
                        "attachment": false,
                        "limit": { "context": 128000, "output": 8192 },
                        "modalities": { "input": ["text"], "output": ["text"] },
                        "cost": { "input": 0.14, "output": 0.28, "cache_read": 0.0028 }
                    }]
                },
                "fireworks-ai": {
                    "models": [{ "id": "f1", "type": "chat", "tool_call": true, "limit": {"context": 0} }]
                }
            }
        });
        let cat = parse_anya2a_all(&raw).unwrap();
        assert_eq!(cat.updated_at, 1785976654990);
        assert!(cat.specs.contains_key("deepseek"));
        assert!(cat.specs.contains_key("fireworks"));
        let spec = cat.lookup("deepseek", "deepseek-chat").unwrap();
        assert_eq!(spec.limits.context, Some(128000));
        assert!(spec.capabilities.tool_call);
        assert_eq!(spec.r#type, ModelType::Chat);
        assert_eq!(spec.cost.as_ref().unwrap().input, Some(0.14));
    }

    #[test]
    fn parse_reasoning_portrait() {
        let raw = json!({
            "updated_at": "0",
            "providers": { "anthropic": { "models": [{
                "id": "claude-sonnet-4-6", "type": "chat",
                "reasoning": { "supported": true, "default": false },
                "extra_capabilities": { "reasoning": {
                    "supported": true, "default_enabled": false, "mode": "mixed",
                    "effort": "high", "effort_options": ["low","medium","high","max"],
                    "interleaved": true, "visibility": "omitted"
                }}
            }]}}
        });
        let cat = parse_anya2a_all(&raw).unwrap();
        let r = cat
            .lookup("anthropic", "claude-sonnet-4-6")
            .unwrap()
            .reasoning
            .unwrap();
        assert!(r.supported);
        assert_eq!(r.mode, Some(ReasoningMode::Mixed));
        assert_eq!(r.effort_default.as_deref(), Some("high"));
        assert!(r.interleaved);
        assert_eq!(r.visibility, Some(ReasoningVisibility::Omitted));
    }

    #[test]
    fn parse_models_dev_bool_reasoning() {
        let raw = json!({
            "updated_at": "0",
            "providers": { "openai": { "models": [{
                "id": "gpt-4o", "type": "chat", "reasoning": false, "tool_call": true
            }]}}
        });
        let cat = parse_anya2a_all(&raw).unwrap();
        let spec = cat.lookup("openai", "gpt-4o").unwrap();
        assert!(spec.reasoning.is_none());
        assert!(spec.capabilities.tool_call);
    }

    #[test]
    fn empty_providers_yields_empty_catalogue() {
        let raw = json!({ "updated_at": "0", "providers": {} });
        let cat = parse_anya2a_all(&raw).unwrap();
        assert_eq!(cat.provider_count(), 0);
    }

    #[test]
    fn missing_providers_key_is_error() {
        let raw = json!({"updated_at": "0"});
        let err = parse_anya2a_all(&raw).unwrap_err();
        assert!(matches!(err, AiMuxError::JsonParse(_)));
    }

    #[test]
    fn budget_min_via_reasoning_options_fallback() {
        let raw = json!({
            "updated_at": "0",
            "providers": { "test": { "models": [{
                "id": "m1", "type": "chat",
                "reasoning": { "supported": true },
                "reasoning_options": [{ "type": "budget_tokens", "min": 2048 }]
            }]}}
        });
        let cat = parse_anya2a_all(&raw).unwrap();
        let r = cat.lookup("test", "m1").unwrap().reasoning.unwrap();
        assert_eq!(r.budget_min, Some(2048));
    }

    #[test]
    fn budget_min_extra_capabilities_takes_precedence() {
        let raw = json!({
            "updated_at": "0",
            "providers": { "test": { "models": [{
                "id": "m1", "type": "chat",
                "reasoning": { "supported": true },
                "reasoning_options": [{ "type": "budget_tokens", "min": 999 }],
                "extra_capabilities": { "reasoning": { "supported": true, "budget": { "min": 1024 } }}
            }]}}
        });
        let cat = parse_anya2a_all(&raw).unwrap();
        let r = cat.lookup("test", "m1").unwrap().reasoning.unwrap();
        assert_eq!(r.budget_min, Some(1024));
    }
}
