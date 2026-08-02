//! OpenAI-compatible provider.
//!
//! Works with any OpenAI-compatible API endpoint (OpenAI, Azure OpenAI,
//! Together.ai, Groq, Fireworks, DeepSeek, etc.).

pub mod convert;
pub mod embedding;
pub mod files;
pub mod image;
pub mod model;
pub mod responses;
pub mod speech;
pub mod transcription;
mod types;

pub use embedding::OpenAIEmbeddingModel;
pub use image::OpenAIImageModel;
pub use model::OpenAIModel;
pub use responses::OpenAIResponsesModel;
pub use speech::OpenAISpeechModel;
pub use transcription::OpenAITranscriptionModel;

use std::collections::HashMap;

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;
use aimux_provider_utils::{RetryConfig, load_api_key, without_trailing_slash};
use serde_json::Value;

/// 描述 OpenAI 兼容厂商的差异。
///
/// 薄封装填这个结构，共享的请求构造和响应解析读它决定行为。
/// 这样既保持 `dyn LanguageModel` 能跨厂商互换，又能表达差异。
#[derive(Debug, Clone, Default)]
pub struct OpenAICompatProfile {
    /// 是否支持 top_k 参数。Groq 等厂商不支持，设为 false 时请求体不发送 top_k。
    pub supports_top_k: bool,
    /// 是否支持 tools。少数厂商不支持，设为 false 时请求体不发送 tools/tool_choice。
    pub supports_tools: bool,
    /// 是否支持 response_format。
    pub supports_response_format: bool,
    /// 流式 usage 的特殊 key。Groq 用 "x_groq"，大多数厂商用顶层 "usage"（留 None）。
    pub stream_usage_key: Option<&'static str>,
    /// max-token 字段的 key（内部数据，非用户概念；RFC-0017 阶段 2）。
    /// - `Some("max_tokens")`            → 只发 `max_tokens`
    /// - `Some("max_completion_tokens")` → 只发 `max_completion_tokens`
    /// - `None`                          → 按模型推断（推理模型 mct / 非推理 max_tokens）
    pub max_tokens_key: Option<&'static str>,
}

impl OpenAICompatProfile {
    /// 默认 profile：支持全部能力，无特殊流式 usage key。
    /// 适用于 OpenAI 本身和大多数兼容厂商。
    pub fn full() -> Self {
        Self {
            supports_top_k: true,
            supports_tools: true,
            supports_response_format: true,
            stream_usage_key: None,
            max_tokens_key: None,
        }
    }

    /// Groq profile：不支持 top_k，流式 usage 在 x_groq 字段；
    /// `max_tokens` 已弃用，只发 `max_completion_tokens`（backlog B9）。
    pub fn groq() -> Self {
        Self {
            supports_top_k: false,
            supports_tools: true,
            supports_response_format: true,
            stream_usage_key: Some("x_groq"),
            max_tokens_key: Some("max_completion_tokens"),
        }
    }

    /// 设置 `max_tokens_key`（内部数据，非用户概念）：`"max_tokens"` 或
    /// `"max_completion_tokens"`。注册表薄封装行用此构建差异 profile。
    pub fn with_max_tokens_key(mut self, key: &'static str) -> Self {
        self.max_tokens_key = Some(key);
        self
    }

    /// DeepSeek profile：特化已退役（RFC-0017 阶段 2），回归 `full()`——
    /// thinking / effort 映射等厂商差异由用户 bodyOverrides 定义。
    /// 保留此薄封装以维持注册表与调用方结构不变。
    pub fn deepseek() -> Self {
        Self::full()
    }
}

/// Configuration for the OpenAI provider.
#[derive(Debug, Clone)]
pub struct OpenAIConfig {
    pub api_key: String,
    pub base_url: String,
    pub org_id: Option<String>,
    /// OpenAI project ID sent via the `OpenAI-Project` header.
    pub project: Option<String>,
    /// Extra headers merged into every request.
    pub headers: Option<HashMap<String, String>>,
    /// Provider name — controls provider-specific behaviour in the shared
    /// request builder (e.g. "groq" reads provider options from the "groq"
    /// key and applies a reasoning-effort map). Defaults to "openai".
    pub provider: String,
    /// 厂商能力差异描述。默认 `full()`（支持全部能力）。
    /// 薄封装用 `with_profile()` 设置差异。
    pub profile: OpenAICompatProfile,
    /// 重试配置。默认 `RetryConfig::default()`（max_retries=2）。
    /// 测试中可用 `.with_retry_config(RetryConfig { max_retries: 0, .. })`
    /// 关闭重试（RFC-0009 §4.2）。
    pub retry_config: RetryConfig,
    /// Provider 级请求体覆盖（RFC-0017）。在标准请求体 + 内置厂商 override
    /// 之后 deep-merge。per-call 的 `CallOptions.body_overrides` 在此之后
    /// 再 merge（覆盖 provider 级）。
    pub body_overrides: Option<Value>,
}

impl OpenAIConfig {
    /// Create from an API key (uses default OpenAI base URL).
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://api.openai.com/v1".to_string(),
            org_id: None,
            project: None,
            headers: None,
            provider: "openai".to_string(),
            profile: OpenAICompatProfile::full(),
            retry_config: RetryConfig::default(),
            body_overrides: None,
        }
    }

    /// Use a custom base URL (for Azure, Groq, etc.).
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = without_trailing_slash(&url.into());
        self
    }

    pub fn with_org_id(mut self, org_id: impl Into<String>) -> Self {
        self.org_id = Some(org_id.into());
        self
    }

    /// Set the OpenAI project ID (sent via the `OpenAI-Project` header).
    pub fn with_project(mut self, project: impl Into<String>) -> Self {
        self.project = Some(project.into());
        self
    }

    /// Attach extra headers merged into every request.
    pub fn with_headers(mut self, headers: HashMap<String, String>) -> Self {
        self.headers = Some(headers);
        self
    }

    /// Set the provider name (e.g. "groq") for provider-specific behaviour.
    pub fn with_provider(mut self, provider: impl Into<String>) -> Self {
        self.provider = provider.into();
        self
    }

    /// 设置厂商能力差异描述。
    pub fn with_profile(mut self, profile: OpenAICompatProfile) -> Self {
        self.profile = profile;
        self
    }

    /// 设置重试配置。传入 `max_retries: 0` 可关闭重试。
    pub fn with_retry_config(mut self, config: RetryConfig) -> Self {
        self.retry_config = config;
        self
    }

    /// 设置 provider 级请求体覆盖（RFC-0017）。
    pub fn with_body_overrides(mut self, overrides: Value) -> Self {
        self.body_overrides = Some(overrides);
        self
    }

    /// Create from environment variable `OPENAI_API_KEY`.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let api_key = load_api_key(None, "OPENAI_API_KEY", "OpenAI")?;
        Ok(Self::new(api_key))
    }
}

/// OpenAI provider — creates `OpenAIModel` instances.
///
/// Does **not** hold an HTTP client — `http::send` / `http::send_stream` use
/// the process-wide shared `Client` internally (RFC-0009 §4.1).
pub struct OpenAIProvider {
    config: OpenAIConfig,
}

impl OpenAIProvider {
    pub fn new(config: OpenAIConfig) -> Self {
        Self { config }
    }

    /// Create a model instance for the given model name (e.g. `"gpt-4o"`).
    pub fn model(&self, model_id: &str) -> model::OpenAIModel {
        model::OpenAIModel::new(model_id.to_string(), self.config.clone())
    }

    /// Create a Responses API model instance for the given model name (e.g.
    /// `"gpt-4o"`). Uses the `/v1/responses` endpoint instead of
    /// `/v1/chat/completions`.
    pub fn responses_model(&self, model_id: &str) -> responses::OpenAIResponsesModel {
        responses::OpenAIResponsesModel::new(model_id.to_string(), self.config.clone())
    }

    /// Create a Files interface for uploading files to OpenAI.
    pub fn files(&self) -> files::OpenAIFiles {
        files::OpenAIFiles::new(self.config.clone())
    }

    /// Create an embedding model instance for the given model name (e.g.
    /// `"text-embedding-3-large"`).
    pub fn embedding_model(&self, model_id: &str) -> embedding::OpenAIEmbeddingModel {
        embedding::OpenAIEmbeddingModel::new(model_id.to_string(), self.config.clone())
    }

    /// Create a speech (TTS) model instance for the given model name (e.g.
    /// `"tts-1"`). Uses the `/audio/speech` endpoint.
    pub fn speech(&self, model_id: &str) -> speech::OpenAISpeechModel {
        speech::OpenAISpeechModel::new(model_id.to_string(), self.config.clone())
    }

    /// Create an image generation model instance for the given model name
    /// (e.g. `"dall-e-3"` or `"gpt-image-1"`). Uses the `/images/generations`
    /// endpoint for generation and `/images/edits` for editing.
    pub fn image(&self, model_id: &str) -> image::OpenAIImageModel {
        image::OpenAIImageModel::new(model_id.to_string(), self.config.clone())
    }

    /// Create a transcription (STT) model instance for the given model name
    /// (e.g. `"whisper-1"` or `"gpt-4o-transcribe"`). Uses the
    /// `/audio/transcriptions` endpoint.
    pub fn transcription(&self, model_id: &str) -> transcription::OpenAITranscriptionModel {
        transcription::OpenAITranscriptionModel::new(model_id.to_string(), self.config.clone())
    }
}

impl Provider for OpenAIProvider {
    fn name(&self) -> &str {
        "openai"
    }

    fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Ok(Box::new(self.model(model_id)))
    }
}
