//! Model specs community catalogue (RFC-0027).
//!
//! Provides [`get_model_specs`] — a thin function that fetches the community
//! model catalogue (anya2a `dist/all.json`) and returns a [`Catalogue`] of
//! [`ModelSpec`] entries. This is **deliberately decoupled** from
//! `ProviderDiscovery::list_models`: the host calls both independently and merges them
//! as needed.
//!
//! `get_model_specs` does **no caching, no FS writes, no TTL** — it is a pure
//! fetch + parse. The host decides how to cache/persist the result.

use std::collections::HashMap;
use std::time::Duration;

use aimux_core::AiMuxError;
use aimux_core::model_catalogue::{
    CatalogueSource, Modality, ModelCapabilities, ModelCost, ModelLimits, ModelModalities,
    ModelSpec, ModelType, ReasoningMode, ReasoningSpec, ReasoningVisibility,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Default anya2a aggregate endpoint (raw `dist/all.json`).
pub const DEFAULT_ANYA2A_URL: &str = "https://raw.githubusercontent.com/ThinkInAIXYZ/PublicProviderConf/refs/heads/dev/dist/all.json";

// ─────────────────────────────────────────────────────────────────────────────
// Provider name mapping
// ─────────────────────────────────────────────────────────────────────────────

/// anya2a provider ids whose aimux provider has another name. Each target is
/// a name [`crate::create_provider`] accepts, serving the same endpoint; no
/// two ids share a target, so one endpoint's models never overwrite
/// another's.
const PROVIDER_ALIASES: &[(&str, &str)] = &[
    ("302ai", "ai302"),
    ("abliteration-ai", "abliteration_ai"),
    ("ai-router", "ai_router"),
    ("aimlapi", "aiml"),
    ("aki-io", "aki_io"),
    ("alibaba-coding-plan", "alibaba_coding_plan"),
    ("alibaba-coding-plan-cn", "alibaba_coding_plan_cn"),
    ("alibaba-token-plan", "alibaba_token_plan"),
    ("alibaba-token-plan-cn", "alibaba_token_plan_cn"),
    ("amazon-bedrock", "amazon_bedrock"),
    ("atomic-chat", "atomic_chat"),
    ("cline-pass", "cline_pass"),
    ("cloudferro-sherlock", "cloudferro_sherlock"),
    ("cloudflare-workers-ai", "cloudflare_workers_ai"),
    ("fireworks-ai", "fireworks"),
    ("friendli", "friendliai"),
    ("github-copilot", "copilot"),
    ("google-vertex", "google_vertex"),
    ("google-vertex-anthropic", "vertex_ai_anthropic_models"),
    ("hpc-ai", "hpc_ai"),
    ("inference", "inference_net"),
    ("io-net", "io_net"),
    ("kimi-for-coding", "kimi_for_coding"),
    ("kuae-cloud-coding-plan", "kuae_cloud_coding_plan"),
    ("merge-gateway", "merge_gateway"),
    ("minimax-cn", "minimax_cn"),
    ("minimax-cn-coding-plan", "minimax_cn_coding_plan"),
    ("minimax-coding-plan", "minimax_coding_plan"),
    ("model-oracle-ai", "model_oracle_ai"),
    ("moonshot", "moonshotai"),
    ("moonshot-ai", "kimi"),
    ("nano-gpt", "nanogpt"),
    ("novita-ai", "novita"),
    ("nvidia", "nvidia_nim"),
    ("opencode-go", "opencode_go"),
    ("perplexity-agent", "perplexity_agent"),
    ("privatemode-ai", "privatemode_ai"),
    ("qihang-ai", "qihang_ai"),
    ("qiniu-ai", "qiniu_ai"),
    ("regolo-ai", "regolo_ai"),
    ("routing-run", "routing_run"),
    ("scx-ai", "scx_ai"),
    ("snowflake-cortex", "snowflake_cortex"),
    ("stepfun-ai-step-plan", "stepfun_ai_step_plan"),
    ("stepfun-step-plan", "stepfun_step_plan"),
    ("tencent-coding-plan", "tencent_coding_plan"),
    ("tencent-token-plan", "tencent_token_plan"),
    ("tencent-tokenhub", "tencent_tokenhub"),
    ("the-grid-ai", "the_grid_ai"),
    ("umans-ai", "umans_ai"),
    ("volcengine", "volc_engine"),
    ("wafer.ai", "wafer"),
    ("xiaomi", "mimo"),
    ("xiaomi-token-plan-ams", "xiaomi_token_plan_ams"),
    ("xiaomi-token-plan-cn", "xiaomi_token_plan_cn"),
    ("xiaomi-token-plan-sgp", "xiaomi_token_plan_sgp"),
    ("zai-coding-plan", "zai_coding_plan"),
    ("zhipuai", "bigmodel"),
    ("zhipuai-coding-plan", "zhipuai_coding_plan"),
];

/// The aimux provider name for an anya2a provider id: its alias, or the id
/// itself when [`crate::create_provider`] accepts it. `None` for any other
/// id; the name is never derived from the spelling.
#[must_use]
pub fn normalize_provider_name(anya2a_id: &str) -> Option<&'static str> {
    PROVIDER_ALIASES
        .iter()
        .find(|(from, _)| *from == anya2a_id)
        .map(|(_, to)| *to)
        .or_else(|| crate::default_providers::provider_name(anya2a_id))
}

// ─────────────────────────────────────────────────────────────────────────────
// Catalogue (in-memory)
// ─────────────────────────────────────────────────────────────────────────────

/// In-memory catalogue: per-provider model specs, keyed by aimux provider
/// name then model id.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalogue {
    #[serde(default)]
    pub updated_at: u64,
    #[serde(default)]
    pub specs: HashMap<String, HashMap<String, ModelSpec>>,
    /// Specs of source providers with no aimux provider, keyed by the
    /// source's own provider id then model id. The host maps them, if it
    /// wants them.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub unmapped: HashMap<String, HashMap<String, ModelSpec>>,
}

impl Catalogue {
    /// Look up the portrait for `(provider, model_id)`.
    #[must_use]
    pub fn lookup(&self, provider: &str, model_id: &str) -> Option<ModelSpec> {
        self.specs.get(provider)?.get(model_id).cloned()
    }

    #[must_use]
    pub fn provider_count(&self) -> usize {
        self.specs.len()
    }
    #[must_use]
    pub fn model_count(&self) -> usize {
        self.specs
            .values()
            .map(std::collections::HashMap::len)
            .sum()
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
///
/// # Errors
///
/// Returns `ApiCall` when the fetch fails and `JsonParse` when the source
/// JSON does not deserialize into a [`Catalogue`].
pub async fn get_model_specs(source_url: Option<&str>) -> Result<Catalogue, AiMuxError> {
    let url = source_url.unwrap_or(DEFAULT_ANYA2A_URL);
    let body = fetch_text(url).await?;
    let raw: Value = serde_json::from_str(&body)
        .map_err(|e| AiMuxError::JsonParse(format!("get_model_specs: invalid source JSON: {e}")))?;
    parse_anya2a_all(&raw)
}

/// Parse anya2a `dist/all.json` into a [`Catalogue`].
///
/// # Errors
///
/// Returns `AiMuxError::JsonParse` when the JSON lacks the expected
/// `providers` object.
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

    let mut catalogue = Catalogue {
        updated_at,
        ..Catalogue::default()
    };
    for (anya2a_id, provider_val) in providers {
        let aimux_name = normalize_provider_name(anya2a_id);
        let models = provider_val.get("models").and_then(|v| v.as_array());
        let Some(models) = models else {
            continue;
        };
        let bucket = match aimux_name {
            Some(name) => catalogue.specs.entry(name.to_string()),
            None => catalogue.unmapped.entry(anya2a_id.clone()),
        }
        .or_default();
        for m in models {
            let Some(id) = m.get("id").and_then(|v| v.as_str()) else {
                continue;
            };
            let mut spec = convert_anya2a_model(m);
            spec.provider = aimux_name.map(str::to_string);
            bucket.insert(id.to_string(), spec);
        }
    }
    Ok(catalogue)
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
    m.get(key)
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
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
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        None => false,
    };
    let default_enabled = m
        .get("reasoning")
        .and_then(|v| v.get("default"))
        .and_then(serde_json::Value::as_bool)
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
        .and_then(serde_json::Value::as_u64)
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
                            o.get("min").and_then(serde_json::Value::as_u64)
                        } else {
                            None
                        }
                    })
                })
        });
    let interleaved = ec
        .and_then(|v| v.get("interleaved"))
        .and_then(serde_json::Value::as_bool)
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
        input: cost.get("input").and_then(serde_json::Value::as_f64),
        output: cost.get("output").and_then(serde_json::Value::as_f64),
        cache_read: cost.get("cache_read").and_then(serde_json::Value::as_f64),
        cache_write: cost.get("cache_write").and_then(serde_json::Value::as_f64),
    })
}

/// Whole-exchange bound for the catalogue download (the aggregate file is a
/// few MiB; the helper default is tuned for API calls).
const FETCH_TIMEOUT: Duration = Duration::from_secs(60);

async fn fetch_text(url: &str) -> Result<String, AiMuxError> {
    let output = aimux_provider_utils::get_from_api(
        aimux_provider_utils::HttpRequest {
            url: url.to_string(),
            response_timeout: Some(FETCH_TIMEOUT),
            ..Default::default()
        },
        aimux_provider_utils::create_binary_response_handler(),
        aimux_provider_utils::create_status_code_error_response_handler(),
    )
    .await?;
    Ok(String::from_utf8_lossy(&output.value).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn aliases_name_the_provider_serving_the_same_endpoint() {
        assert_eq!(normalize_provider_name("fireworks-ai"), Some("fireworks"));
        assert_eq!(
            normalize_provider_name("google-vertex"),
            Some("google_vertex")
        );
        assert_eq!(
            normalize_provider_name("amazon-bedrock"),
            Some("amazon_bedrock")
        );
        assert_eq!(
            normalize_provider_name("xiaomi-token-plan-ams"),
            Some("xiaomi_token_plan_ams")
        );
        assert_eq!(
            normalize_provider_name("zhipuai-coding-plan"),
            Some("zhipuai_coding_plan")
        );
    }

    #[test]
    fn an_id_without_alias_maps_only_when_it_is_a_provider_name() {
        assert_eq!(normalize_provider_name("deepseek"), Some("deepseek"));
        assert_eq!(normalize_provider_name("some-provider"), None);
        assert_eq!(normalize_provider_name("siliconflow-com"), None);
    }

    #[test]
    fn every_alias_names_a_distinct_provider() {
        let mut targets = std::collections::HashSet::new();
        for (from, to) in PROVIDER_ALIASES {
            assert!(
                crate::default_providers::provider_name(to).is_some(),
                "{from} -> {to}: not a provider name"
            );
            assert!(
                crate::default_providers::provider_name(from).is_none(),
                "{from} is itself a provider name"
            );
            assert!(targets.insert(to), "{to} is the target of two aliases");
        }
    }

    #[test]
    fn unknown_providers_keep_their_source_id() {
        let raw = json!({
            "updated_at": "0",
            "providers": {
                "fireworks-ai": { "models": [{ "id": "f1", "type": "chat" }] },
                "siliconflow-com": { "models": [{ "id": "s1", "type": "chat" }] }
            }
        });
        let cat = parse_anya2a_all(&raw).unwrap();
        let mapped = cat.lookup("fireworks", "f1").unwrap();
        assert_eq!(mapped.provider.as_deref(), Some("fireworks"));
        assert!(!cat.specs.contains_key("siliconflow_com"));
        assert!(cat.lookup("siliconflow", "s1").is_none());
        let unmapped = &cat.unmapped["siliconflow-com"]["s1"];
        assert_eq!(unmapped.provider, None);
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
            "providers": { "deepseek": { "models": [{
                "id": "m1", "type": "chat",
                "reasoning": { "supported": true },
                "reasoning_options": [{ "type": "budget_tokens", "min": 2048 }]
            }]}}
        });
        let cat = parse_anya2a_all(&raw).unwrap();
        let r = cat.lookup("deepseek", "m1").unwrap().reasoning.unwrap();
        assert_eq!(r.budget_min, Some(2048));
    }

    #[test]
    fn budget_min_extra_capabilities_takes_precedence() {
        let raw = json!({
            "updated_at": "0",
            "providers": { "deepseek": { "models": [{
                "id": "m1", "type": "chat",
                "reasoning": { "supported": true },
                "reasoning_options": [{ "type": "budget_tokens", "min": 999 }],
                "extra_capabilities": { "reasoning": { "supported": true, "budget": { "min": 1024 } }}
            }]}}
        });
        let cat = parse_anya2a_all(&raw).unwrap();
        let r = cat.lookup("deepseek", "m1").unwrap().reasoning.unwrap();
        assert_eq!(r.budget_min, Some(1024));
    }
}
