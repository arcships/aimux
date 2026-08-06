//! Model catalogue sync & lookup (RFC-0027).
//!
//! Supplements runtime model discovery (`Provider::list_models`) with a static
//! capability portrait sourced from community knowledge (`models.anya2a.com`).
//!
//! ## Data flow
//!
//! ```text
//! anya2a dist/all.json  ──convert──▶  Catalogue { specs: {provider → {model_id → ModelSpec}} }
//!        (5.5 MB)                          (normalized to aimux provider names)
//!                                             │
//!                          cached to disk (catalogue.json + version stamp)
//!                                             │
//! list_models(runtime) ──lookup──▶  Option<ModelSpec>  ──▶  ResolvedModel
//! ```
//!
//! The portrait is **advisory**: aimux never auto-applies it in the request
//! path. `list_models` attaches it so callers can read capabilities/limits and
//! decide how to configure their requests.
//!
//! ## Provider name normalization
//!
//! anya2a uses kebab-case provider ids (`fireworks-ai`, `google-vertex`); aimux
//! uses snake_case (`fireworks`, `vertex`). [`normalize_provider_name`] maps
//! anya2a ids to aimux registry names via a curated alias table plus a
//! `-`→`_` fallback, so lookups by aimux provider name hit directly.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use aimux_core::AiMuxError;
use aimux_core::model_catalogue::{
    CatalogueSource, Modality, ModelCapabilities, ModelCost, ModelLimits, ModelModalities,
    ModelSpec, ModelType, ReasoningMode, ReasoningSpec, ReasoningVisibility, ResolvedModel,
    RuntimeModel,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::debug;

/// Default anya2a aggregate endpoint (raw `dist/all.json`).
pub const DEFAULT_ANYA2A_URL: &str = "https://raw.githubusercontent.com/ThinkInAIXYZ/PublicProviderConf/refs/heads/dev/dist/all.json";
/// Default anya2a version-stamp endpoint (28-byte `dc_sync_version.json`).
pub const DEFAULT_ANYA2A_VERSION_URL: &str = "https://raw.githubusercontent.com/ThinkInAIXYZ/PublicProviderConf/refs/heads/dev/dist/dc_sync_version.json";
/// Default cache TTL: 24h.
pub const DEFAULT_TTL: Duration = Duration::from_secs(86400);

// ─────────────────────────────────────────────────────────────────────────────
// Provider name normalization
// ─────────────────────────────────────────────────────────────────────────────

/// Curated anya2a-id → aimux-name aliases (cases where `-`→`_` fallback is not
/// enough). Extend as more mismatches are discovered.
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
///
/// 1. Curated alias table first (handles dropped suffixes like `-ai`, `-cn`).
/// 2. Fallback: kebab-case → snake_case (`-` → `_`).
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

/// The in-memory catalogue: per-provider model specs, keyed by aimux provider
/// name then model id.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Catalogue {
    /// Unix-millis timestamp from the source's version stamp.
    #[serde(default)]
    pub updated_at: u64,
    /// `aimux_provider_name → model_id → ModelSpec`.
    #[serde(default)]
    pub specs: HashMap<String, HashMap<String, ModelSpec>>,
}

impl Catalogue {
    /// Look up the portrait for `(provider, model_id)`.
    pub fn lookup(&self, provider: &str, model_id: &str) -> Option<ModelSpec> {
        self.specs.get(provider)?.get(model_id).cloned()
    }

    /// Attach portraits to a list of runtime-discovered models, producing
    /// [`ResolvedModel`]s. Models without a portrait keep `spec = None`.
    pub fn resolve(&self, provider: &str, models: Vec<RuntimeModel>) -> Vec<ResolvedModel> {
        models
            .into_iter()
            .map(|m| {
                let spec = self.lookup(provider, &m.id);
                ResolvedModel::from_runtime(m, spec)
            })
            .collect()
    }

    /// Number of providers / models indexed.
    pub fn provider_count(&self) -> usize {
        self.specs.len()
    }
    pub fn model_count(&self) -> usize {
        self.specs.values().map(|m| m.len()).sum()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// anya2a → ModelSpec conversion
// ─────────────────────────────────────────────────────────────────────────────

/// Parse anya2a `dist/all.json` into a [`Catalogue`].
///
/// Unknown fields are ignored; missing fields default. Each anya2a provider id
/// is normalized to an aimux provider name via [`normalize_provider_name`].
pub fn parse_anya2a_all(json: &Value) -> Result<Catalogue, AiMuxError> {
    let providers = json
        .get("providers")
        .and_then(|v| v.as_object())
        .ok_or_else(|| AiMuxError::Json("anya2a all.json missing 'providers' object".into()))?;
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

/// Convert a single anya2a model entry into a [`ModelSpec`].
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

/// Build a [`ReasoningSpec`] from anya2a's `reasoning` + `reasoning_options` +
/// `extra_capabilities.reasoning` fields.
fn build_reasoning_spec(m: &Value) -> Option<ReasoningSpec> {
    // anya2a `reasoning` may be a bool (models.dev) or an object {supported, default}.
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

    // Prefer the richer extra_capabilities.reasoning portrait.
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
            // reasoning_options [{type:"budget_tokens", min:…}]
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

// ─────────────────────────────────────────────────────────────────────────────
// CatalogueSync — fetch + cache
// ─────────────────────────────────────────────────────────────────────────────

/// Configuration for syncing the community model catalogue.
#[derive(Clone)]
pub struct CatalogueSync {
    cache_dir: PathBuf,
    source_url: String,
    version_url: String,
    ttl: Duration,
    /// When true, skip the network entirely and only read the cache (offline).
    offline: bool,
}

impl Default for CatalogueSync {
    fn default() -> Self {
        let cache_dir = default_cache_dir();
        // `AIMUX_CATALOGUE_OFFLINE=1` skips all network access (cache-only) —
        // lets tests and offline environments stay deterministic.
        let offline = std::env::var("AIMUX_CATALOGUE_OFFLINE")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        Self {
            cache_dir,
            source_url: DEFAULT_ANYA2A_URL.to_string(),
            version_url: DEFAULT_ANYA2A_VERSION_URL.to_string(),
            ttl: DEFAULT_TTL,
            offline,
        }
    }
}

impl CatalogueSync {
    pub fn new() -> Self {
        Self::default()
    }

    /// Override the on-disk cache directory.
    pub fn with_cache_dir(mut self, dir: PathBuf) -> Self {
        self.cache_dir = dir;
        self
    }

    /// Override the aggregate `all.json` source URL (e.g. a self-hosted mirror).
    pub fn with_source_url(mut self, url: impl Into<String>) -> Self {
        self.source_url = url.into();
        self
    }

    /// Override the version-stamp URL.
    pub fn with_version_url(mut self, url: impl Into<String>) -> Self {
        self.version_url = url.into();
        self
    }

    /// Set the cache TTL. `Duration::ZERO` forces a re-fetch on every `sync`.
    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    /// Offline mode: never hit the network, only read the cache.
    pub fn offline(mut self) -> Self {
        self.offline = true;
        self
    }

    fn cache_file(&self) -> PathBuf {
        self.cache_dir.join("catalogue.json")
    }

    /// Load the cached catalogue from disk without touching the network.
    /// Returns `Ok(None)` if no cache exists.
    pub fn load_cached(&self) -> Result<Option<Catalogue>, AiMuxError> {
        let path = self.cache_file();
        if !path.exists() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(&path).map_err(|e| {
            AiMuxError::Provider(format!(
                "catalogue: cannot read cache {}: {e}",
                path.display()
            ))
        })?;
        let cat: Catalogue = serde_json::from_str(&text)
            .map_err(|e| AiMuxError::Json(format!("catalogue: invalid cache JSON: {e}")))?;
        Ok(Some(cat))
    }

    /// Best-effort load: return the cached catalogue if fresh (within TTL),
    /// otherwise fetch a fresh one. On offline mode, returns the stale cache if
    /// present (no error for staleness).
    pub async fn load(&self) -> Result<Option<Catalogue>, AiMuxError> {
        if let Some(cached) = self.load_cached()?
            && (self.is_fresh(&cached) || self.offline)
        {
            debug!(
                "catalogue: using cached (updated_at={}, {} providers, {} models)",
                cached.updated_at,
                cached.provider_count(),
                cached.model_count()
            );
            return Ok(Some(cached));
        }
        if self.offline {
            return Ok(None);
        }
        match self.sync().await {
            Ok(cat) => Ok(Some(cat)),
            Err(e) => {
                // Network failed: fall back to stale cache if present.
                debug!("catalogue: sync failed ({e}); falling back to stale cache");
                self.load_cached()
            }
        }
    }

    /// Is the cached catalogue within TTL? Based on the cached file's mtime.
    fn is_fresh(&self, _cat: &Catalogue) -> bool {
        let path = self.cache_file();
        let Ok(meta) = std::fs::metadata(&path) else {
            return false;
        };
        let Ok(modified) = meta.modified() else {
            return false;
        };
        match modified.elapsed() {
            Ok(age) => age < self.ttl,
            Err(_) => false,
        }
    }

    /// Fetch the latest catalogue from the source, convert it, and write it to
    /// the cache. Returns the fresh catalogue.
    pub async fn sync(&self) -> Result<Catalogue, AiMuxError> {
        if self.offline {
            return Err(AiMuxError::Provider(
                "catalogue: offline mode refuses network sync".into(),
            ));
        }
        let body = fetch_text(&self.source_url).await?;
        let raw: Value = serde_json::from_str(&body)
            .map_err(|e| AiMuxError::Json(format!("catalogue: invalid source JSON: {e}")))?;
        let cat = parse_anya2a_all(&raw)?;
        self.write_cache(&cat)?;
        debug!(
            "catalogue: synced (updated_at={}, {} providers, {} models)",
            cat.updated_at,
            cat.provider_count(),
            cat.model_count()
        );
        Ok(cat)
    }

    fn write_cache(&self, cat: &Catalogue) -> Result<(), AiMuxError> {
        std::fs::create_dir_all(&self.cache_dir).map_err(|e| {
            AiMuxError::Provider(format!(
                "catalogue: cannot create cache dir {}: {e}",
                self.cache_dir.display()
            ))
        })?;
        let json = serde_json::to_string(cat)
            .map_err(|e| AiMuxError::Json(format!("catalogue: serialize cache: {e}")))?;
        std::fs::write(self.cache_file(), json).map_err(|e| {
            AiMuxError::Provider(format!(
                "catalogue: cannot write cache {}: {e}",
                self.cache_file().display()
            ))
        })?;
        Ok(())
    }
}

/// Resolve runtime models against the catalogue, attaching portraits.
/// Convenience wrapper: loads the catalogue (best-effort) and resolves.
pub async fn resolve_with_catalogue(
    sync: &CatalogueSync,
    provider: &str,
    models: Vec<RuntimeModel>,
) -> Vec<ResolvedModel> {
    match sync.load().await {
        Ok(Some(cat)) => cat.resolve(provider, models),
        _ => models
            .into_iter()
            .map(|m| ResolvedModel::from_runtime(m, None))
            .collect(),
    }
}

/// The process-wide default catalogue sync (anya2a, default cache dir, 24h TTL).
///
/// Re-reads `AIMUX_CATALOGUE_DIR` / `AIMUX_CATALOGUE_OFFLINE` on each call so
/// tests can point at a temp cache (cheap: `CatalogueSync` is a few strings).
pub fn default_sync() -> CatalogueSync {
    CatalogueSync::new()
}

// ─────────────────────────────────────────────────────────────────────────────
// helpers
// ─────────────────────────────────────────────────────────────────────────────

fn default_cache_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("AIMUX_CATALOGUE_DIR") {
        return PathBuf::from(dir);
    }
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("aimux")
        .join("catalogue")
}

async fn fetch_text(url: &str) -> Result<String, AiMuxError> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| AiMuxError::Http(format!("catalogue: build client: {e}")))?;
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| AiMuxError::Http(format!("catalogue: fetch {url}: {e}")))?;
    if !resp.status().is_success() {
        return Err(AiMuxError::Provider(format!(
            "catalogue: fetch {url} returned {}",
            resp.status()
        )));
    }
    resp.text()
        .await
        .map_err(|e| AiMuxError::Http(format!("catalogue: read body: {e}")))
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
        // Unknown id → kebab-to-snake.
        assert_eq!(normalize_provider_name("some-provider"), "some_provider");
        assert_eq!(normalize_provider_name("deepseek"), "deepseek");
        assert_eq!(normalize_provider_name("siliconflow"), "siliconflow");
    }

    #[test]
    fn parse_minimal_anya2a() {
        let raw = json!({
            "updated_at": "1785976654990",
            "providers": {
                "deepseek": {
                    "id": "deepseek",
                    "api": "https://api.deepseek.com",
                    "models": [
                        {
                            "id": "deepseek-chat",
                            "name": "DeepSeek Chat",
                            "type": "chat",
                            "tool_call": true,
                            "structured_output": true,
                            "temperature": true,
                            "attachment": false,
                            "limit": { "context": 128000, "output": 8192 },
                            "modalities": { "input": ["text"], "output": ["text"] },
                            "cost": { "input": 0.14, "output": 0.28, "cache_read": 0.0028 }
                        }
                    ]
                },
                "fireworks-ai": {
                    "models": [
                        { "id": "f1", "type": "chat", "tool_call": true, "limit": {"context": 0} }
                    ]
                }
            }
        });
        let cat = parse_anya2a_all(&raw).unwrap();
        assert_eq!(cat.updated_at, 1785976654990);
        // fireworks-ai normalized to fireworks.
        assert!(cat.specs.contains_key("deepseek"));
        assert!(cat.specs.contains_key("fireworks"));

        let spec = cat.lookup("deepseek", "deepseek-chat").unwrap();
        assert_eq!(spec.limits.context, Some(128000));
        assert_eq!(spec.limits.output, Some(8192));
        assert!(spec.capabilities.tool_call);
        assert!(spec.capabilities.structured_output);
        assert!(!spec.capabilities.attachment);
        assert_eq!(spec.r#type, ModelType::Chat);
        assert_eq!(spec.modalities.input, vec![Modality::Text]);
        assert_eq!(spec.cost.as_ref().unwrap().input, Some(0.14));
        assert_eq!(spec.source, CatalogueSource::Anya2a);
    }

    #[test]
    fn parse_reasoning_portrait() {
        let raw = json!({
            "updated_at": "0",
            "providers": {
                "anthropic": {
                    "models": [{
                        "id": "claude-sonnet-4-6",
                        "type": "chat",
                        "reasoning": { "supported": true, "default": false },
                        "extra_capabilities": {
                            "reasoning": {
                                "supported": true,
                                "default_enabled": false,
                                "mode": "mixed",
                                "effort": "high",
                                "effort_options": ["low","medium","high","max"],
                                "interleaved": true,
                                "visibility": "omitted"
                            }
                        }
                    }]
                }
            }
        });
        let cat = parse_anya2a_all(&raw).unwrap();
        let spec = cat.lookup("anthropic", "claude-sonnet-4-6").unwrap();
        let r = spec.reasoning.unwrap();
        assert!(r.supported);
        assert!(!r.default_enabled);
        assert_eq!(r.mode, Some(ReasoningMode::Mixed));
        assert_eq!(r.effort_default.as_deref(), Some("high"));
        assert_eq!(r.effort_options, vec!["low", "medium", "high", "max"]);
        assert!(r.interleaved);
        assert_eq!(r.visibility, Some(ReasoningVisibility::Omitted));
    }

    #[test]
    fn parse_models_dev_bool_reasoning() {
        // models.dev shape: reasoning is a bool, no extra_capabilities.
        let raw = json!({
            "updated_at": "0",
            "providers": {
                "openai": {
                    "models": [{
                        "id": "gpt-4o",
                        "type": "chat",
                        "reasoning": false,
                        "tool_call": true,
                        "limit": { "context": 128000, "output": 16384 }
                    }]
                }
            }
        });
        let cat = parse_anya2a_all(&raw).unwrap();
        let spec = cat.lookup("openai", "gpt-4o").unwrap();
        // reasoning=false with no portrait → None.
        assert!(spec.reasoning.is_none());
        assert!(spec.capabilities.tool_call);
    }

    #[test]
    fn resolve_attaches_specs() {
        let raw = json!({
            "updated_at": "0",
            "providers": { "deepseek": { "models": [
                { "id": "deepseek-chat", "type": "chat", "limit": {"context": 128000} }
            ]}}
        });
        let cat = parse_anya2a_all(&raw).unwrap();
        let runtime = vec![
            RuntimeModel {
                id: "deepseek-chat".into(),
                owned_by: None,
                created: None,
            },
            RuntimeModel {
                id: "deepseek-unknown".into(),
                owned_by: None,
                created: None,
            },
        ];
        let resolved = cat.resolve("deepseek", runtime);
        assert_eq!(resolved.len(), 2);
        assert!(resolved[0].spec.is_some());
        assert_eq!(
            resolved[0].spec.as_ref().unwrap().limits.context,
            Some(128000)
        );
        assert!(resolved[1].spec.is_none());
    }

    #[test]
    fn cache_roundtrip() {
        // Use a unique temp dir to avoid clobbering the real cache.
        let dir = std::env::temp_dir().join(format!(
            "aimux-catalogue-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let sync = CatalogueSync::new().with_cache_dir(dir.clone()).offline();

        let raw = json!({
            "updated_at": "100",
            "providers": { "deepseek": { "models": [
                { "id": "deepseek-chat", "type": "chat", "limit": {"context": 128000} }
            ]}}
        });
        let cat = parse_anya2a_all(&raw).unwrap();
        sync.write_cache(&cat).unwrap();

        let loaded = sync.load_cached().unwrap().unwrap();
        assert_eq!(loaded.updated_at, 100);
        assert!(loaded.lookup("deepseek", "deepseek-chat").is_some());

        // offline load returns the cached (fresh) catalogue.
        let rt = tokio::runtime::Runtime::new().unwrap();
        let got = rt.block_on(sync.load()).unwrap().unwrap();
        assert!(got.lookup("deepseek", "deepseek-chat").is_some());

        // cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_providers_yields_empty_catalogue() {
        let raw = json!({ "updated_at": "0", "providers": {} });
        let cat = parse_anya2a_all(&raw).unwrap();
        assert_eq!(cat.provider_count(), 0);
        assert_eq!(cat.model_count(), 0);
    }
}
