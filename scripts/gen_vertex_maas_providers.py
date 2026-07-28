#!/usr/bin/env python3
"""Generate the 10 vertex_ai_*_models providers.

Each provider is a thin OpenAI-compatible wrapper over `OpenAIProvider` that
points at the Vertex AI MaaS OpenAPI endpoint and authenticates with a Google
Cloud Bearer token. This script writes the 10 `.rs` files and patches
`lib.rs` (module + re-export registration) and the shared
`openai_compatible_test.rs` (imports + macro invocations).

Re-runnable: the `.rs` files are overwritten and the lib/test patches are
applied idempotently (anchored string replacement that asserts a single match).
"""

from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "aimux-providers" / "src"
LIB = SRC / "lib.rs"
TEST = ROOT / "aimux-providers" / "tests" / "openai_compatible_test.rs"

BOM = "\ufeff"

# Ordered alphabetically by Pascal name (ai21 < anthropic < ... < zai), which
# also matches module-name order so rustfmt's reorder_modules/imports is a no-op.
PROVIDERS = [
    {
        "canonical": "vertex_ai_ai21_models",
        "pascal": "VertexAiAi21Models",
        "vendor": "AI21",
        "models": ["ai21/jamba-1.5-large", "ai21/jamba-1.5-mini"],
        "test_model": "ai21/jamba-1.5-large",
    },
    {
        "canonical": "vertex_ai_anthropic_models",
        "pascal": "VertexAiAnthropicModels",
        "vendor": "Anthropic Claude",
        "models": ["anthropic/claude-sonnet-4", "anthropic/claude-opus-4"],
        "test_model": "anthropic/claude-sonnet-4",
    },
    {
        "canonical": "vertex_ai_deepseek_models",
        "pascal": "VertexAiDeepseekModels",
        "vendor": "DeepSeek",
        "models": [
            "deepseek-ai/deepseek-v3.1-maas",
            "deepseek-ai/deepseek-r1-0528-maas",
        ],
        "test_model": "deepseek-ai/deepseek-v3.1-maas",
    },
    {
        "canonical": "vertex_ai_llama_models",
        "pascal": "VertexAiLlamaModels",
        "vendor": "Meta Llama",
        "models": [
            "meta/llama-4-scout-17b-16e-instruct-maas",
            "meta/llama-4-maverick-17b-128e-instruct-maas",
        ],
        "test_model": "meta/llama-4-scout-17b-16e-instruct-maas",
    },
    {
        "canonical": "vertex_ai_minimax_models",
        "pascal": "VertexAiMinimaxModels",
        "vendor": "MiniMax",
        "models": ["minimax/minimax-m2-maas"],
        "test_model": "minimax/minimax-m2-maas",
    },
    {
        "canonical": "vertex_ai_mistral_models",
        "pascal": "VertexAiMistralModels",
        "vendor": "Mistral",
        "models": ["mistralai/mistral-large-2411", "mistralai/codestral-2501"],
        "test_model": "mistralai/mistral-large-2411",
    },
    {
        "canonical": "vertex_ai_moonshot_models",
        "pascal": "VertexAiMoonshotModels",
        "vendor": "Moonshot AI",
        "models": ["moonshotai/kimi-k2-thinking-maas"],
        "test_model": "moonshotai/kimi-k2-thinking-maas",
    },
    {
        "canonical": "vertex_ai_openai_models",
        "pascal": "VertexAiOpenaiModels",
        "vendor": "OpenAI",
        "models": ["openai/gpt-oss-120b-maas", "openai/gpt-oss-20b-maas"],
        "test_model": "openai/gpt-oss-120b-maas",
    },
    {
        "canonical": "vertex_ai_qwen_models",
        "pascal": "VertexAiQwenModels",
        "vendor": "Qwen",
        "models": [
            "qwen/qwen3-coder-480b-a35b-instruct-maas",
            "qwen/qwen3-next-80b-a3b-instruct-maas",
        ],
        "test_model": "qwen/qwen3-coder-480b-a35b-instruct-maas",
    },
    {
        "canonical": "vertex_ai_zai_models",
        "pascal": "VertexAiZaiModels",
        "vendor": "Z.AI",
        "models": ["zai-org/glm-4.7-maas", "zai-org/glm-5-maas"],
        "test_model": "zai-org/glm-4.7-maas",
    },
]

FILE_TEMPLATE = BOM + r'''//! {Vendor} models on Vertex AI MaaS — a thin OpenAI-compatible wrapper.
//!
//! Vertex AI serves partner and open models (Anthropic, AI21, DeepSeek, Llama,
//! MiniMax, Mistral, Moonshot, OpenAI, Qwen, Z.AI) through an OpenAI-compatible
//! Chat Completions endpoint — the "Model as a Service" (MaaS) OpenAPI surface
//! — rather than the native `rawPredict` path:
//!
//! `https://{host}/v1/projects/{project}/locations/{location}/endpoints/openapi`
//!
//! The host is derived from the location: `global` uses
//! `aiplatform.googleapis.com`, `eu`/`us` use `aiplatform.{loc}.rep.googleapis.com`,
//! and any other location uses `{loc}-aiplatform.googleapis.com`. Authentication
//! uses a Google Cloud OAuth2 Bearer token (the same `GOOGLE_VERTEX_ACCESS_TOKEN`
//! used by the native Vertex provider), sent as `Authorization: Bearer <token>`.
//!
//! Because the endpoint is OpenAI-compatible, this provider is a thin wrapper
//! over [`OpenAIProvider`](crate::openai::OpenAIProvider): only the base URL,
//! the Bearer-token env var, and the provider name differ. The shared
//! `OpenAIProvider` appends `/chat/completions` to the configured base URL.
//! Sample model ids: {models_doc}.

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;

use crate::openai::{OpenAICompatProfile, OpenAIConfig, OpenAIModel, OpenAIProvider};

const PROVIDER_NAME: &str = "{canonical}";
const TOKEN_ENV_VAR: &str = "GOOGLE_VERTEX_ACCESS_TOKEN";
const PROJECT_ENV_VAR: &str = "GOOGLE_VERTEX_PROJECT";
const LOCATION_ENV_VAR: &str = "GOOGLE_VERTEX_LOCATION";
const DEFAULT_LOCATION: &str = "global";
/// Fallback project when `GOOGLE_VERTEX_PROJECT` is unset and no base URL is
/// supplied via [`{Pascal}Config::with_base_url`]; prefer
/// [`{Pascal}Config::from_env`] or set the project explicitly for real usage.
const DEFAULT_PROJECT: &str = "your-project";

/// Build the Vertex AI MaaS OpenAI-compatible base URL for a project/location.
///
/// - `global` → `https://aiplatform.googleapis.com/v1/projects/{p}/locations/global/endpoints/openapi`
/// - `eu`/`us` → `https://aiplatform.{loc}.rep.googleapis.com/v1/projects/{p}/locations/{loc}/endpoints/openapi`
/// - other → `https://{loc}-aiplatform.googleapis.com/v1/projects/{p}/locations/{loc}/endpoints/openapi`
fn build_maas_base_url(project: &str, location: &str) -> String {
    let host = match location {
        "global" => "aiplatform.googleapis.com".to_string(),
        "eu" | "us" => format!("aiplatform.{}.rep.googleapis.com", location),
        _ => format!("{}-aiplatform.googleapis.com", location),
    };
    format!(
        "https://{}/v1/projects/{}/locations/{}/endpoints/openapi",
        host, project, location
    )
}

/// Assemble the shared [`OpenAIConfig`] for the given token + project/location.
fn build_config(api_key: String, project: &str, location: &str) -> OpenAIConfig {
    OpenAIConfig::new(api_key)
        .with_base_url(build_maas_base_url(project, location))
        .with_provider(PROVIDER_NAME)
        .with_profile(OpenAICompatProfile::full())
}

/// Configuration for the {Vendor} Vertex AI MaaS provider (wraps [`OpenAIConfig`]).
pub struct {Pascal}Config(OpenAIConfig);

impl {Pascal}Config {
    /// Create from a Google Cloud Bearer access token, constructing the base
    /// URL from `GOOGLE_VERTEX_PROJECT` / `GOOGLE_VERTEX_LOCATION` (with
    /// `global` / `your-project` fallbacks). Override the URL with
    /// [`Self::with_base_url`] for tests or proxies.
    pub fn new(api_key: impl Into<String>) -> Self {
        let project =
            std::env::var(PROJECT_ENV_VAR).unwrap_or_else(|_| DEFAULT_PROJECT.to_string());
        let location =
            std::env::var(LOCATION_ENV_VAR).unwrap_or_else(|_| DEFAULT_LOCATION.to_string());
        Self(build_config(api_key.into(), &project, &location))
    }

    /// Create from `GOOGLE_VERTEX_ACCESS_TOKEN` + `GOOGLE_VERTEX_PROJECT` +
    /// `GOOGLE_VERTEX_LOCATION` (location defaults to `global`).
    pub fn from_env() -> Result<Self, AiMuxError> {
        let token = std::env::var(TOKEN_ENV_VAR).map_err(|_| {
            AiMuxError::InvalidArgument(
                "GOOGLE_VERTEX_ACCESS_TOKEN environment variable is required for Vertex AI MaaS"
                    .to_string(),
            )
        })?;
        let project = std::env::var(PROJECT_ENV_VAR).map_err(|_| {
            AiMuxError::InvalidArgument(
                "GOOGLE_VERTEX_PROJECT environment variable is required for Vertex AI MaaS"
                    .to_string(),
            )
        })?;
        let location =
            std::env::var(LOCATION_ENV_VAR).unwrap_or_else(|_| DEFAULT_LOCATION.to_string());
        Ok(Self(build_config(token, &project, &location)))
    }

    /// Override the base URL (useful for tests / proxies).
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.0 = self.0.with_base_url(url);
        self
    }
}

/// {Vendor} Vertex AI MaaS provider — creates [`OpenAIModel`] instances pointed
/// at the Vertex AI MaaS OpenAPI endpoint.
pub struct {Pascal}Provider(OpenAIProvider);

impl {Pascal}Provider {
    pub fn new(config: {Pascal}Config) -> Self {
        Self(OpenAIProvider::new(config.0))
    }

    /// Create a model instance for the given Vertex AI MaaS model id
    /// (e.g. `"{sample_model}"`).
    pub fn model(&self, model_id: &str) -> OpenAIModel {
        self.0.model(model_id)
    }
}

impl Provider for {Pascal}Provider {
    fn name(&self) -> &str {
        PROVIDER_NAME
    }

    fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Ok(Box::new(self.model(model_id)))
    }
}
'''


def models_doc(models):
    quoted = ", ".join("`\"{}\"`".format(m) for m in models)
    return quoted


def render_file(p):
    return (
        FILE_TEMPLATE
        .replace("{Vendor}", p["vendor"])
        .replace("{canonical}", p["canonical"])
        .replace("{Pascal}", p["pascal"])
        .replace("{models_doc}", models_doc(p["models"]))
        .replace("{sample_model}", p["test_model"])
    )


def write_providers():
    for p in PROVIDERS:
        path = SRC / (p["canonical"] + ".rs")
        path.write_text(render_file(p), encoding="utf-8")
        print("wrote", path.relative_to(ROOT))


def patch_lib():
    mods = "\n".join("pub mod {};".format(p["canonical"]) for p in PROVIDERS)
    uses = "\n".join(
        "pub use {}::{{{name}Config, {name}Provider}};".format(
            p["canonical"], name=p["pascal"]
        )
        for p in PROVIDERS
    )
    block = (
        "// Vertex AI MaaS partner-model providers (OpenAI-compatible thin wrappers).\n"
        "// Each wraps the shared OpenAIProvider against the Vertex AI MaaS OpenAPI\n"
        "// endpoint, authenticating with a Google Cloud Bearer token.\n"
        + mods
        + "\n\n"
        + uses
        + "\n\n"
    )
    anchor = "// Search-only providers (web search modality)."
    text = LIB.read_text(encoding="utf-8")
    count = text.count(anchor)
    if count == 0:
        raise SystemExit("lib.rs anchor not found: " + anchor)
    if "// Vertex AI MaaS partner-model providers" in text:
        raise SystemExit("lib.rs already patched with Vertex AI MaaS block")
    text = text.replace(anchor, block + anchor, 1)
    LIB.write_text(text, encoding="utf-8")
    print("patched", LIB.relative_to(ROOT))


def patch_test():
    text = TEST.read_text(encoding="utf-8")
    # 1. imports: insert the 20 names between VercelProvider and XAIConfig.
    names = []
    for p in PROVIDERS:
        names.append("{p}Config".format(p=p["pascal"]))
        names.append("{p}Provider".format(p=p["pascal"]))
    names_str = ", ".join(names)
    imp_anchor = "VercelConfig, VercelProvider, XAIConfig,"
    if text.count(imp_anchor) != 1:
        raise SystemExit(
            "test import anchor count != 1: {!r}".format(imp_anchor)
        )
    if "VertexAiAi21ModelsConfig" in text:
        raise SystemExit("test file already patched with Vertex AI MaaS imports")
    text = text.replace(
        imp_anchor,
        "VercelConfig, VercelProvider, " + names_str + ", XAIConfig,",
        1,
    )
    # 2. macro invocations: append at end of file.
    macros = []
    for p in PROVIDERS:
        macros.append(
            "openai_compatible_tests!(\n"
            "    {mod},\n"
            "    {pascal}Config,\n"
            "    {pascal}Provider,\n"
            "    \"{model}\"\n"
            ");".format(mod=p["canonical"], pascal=p["pascal"], model=p["test_model"])
        )
    suffix = "\n\n// Vertex AI MaaS partner-model providers (OpenAI-compatible thin wrappers).\n"
    suffix += "\n".join(macros) + "\n"
    if not text.endswith("\n"):
        text += "\n"
    text += suffix
    TEST.write_text(text, encoding="utf-8")
    print("patched", TEST.relative_to(ROOT))


def main():
    write_providers()
    patch_lib()
    patch_test()


if __name__ == "__main__":
    main()
