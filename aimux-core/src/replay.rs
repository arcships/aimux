//! RFC-0023 回放(P3 — mock 响应回放;P4 — 请求回放)。
//!
//! - [`MockReplayModel`] 实现 [`LanguageModel`],按输入匹配录制(三层
//!   `Recording`),直接返回录制响应——**不发真实 API**。与 RFC-0003 的
//!   wiremock(cfg(test) + MockServer)不同,这是运行时机制:本地 dev 用录制
//!   响应调试、离线测试、降本。
//! - [`replay_with_model`] 是**请求回放**(P4,provider 无关):用录制输入
//!   重建调用,经调用方提供的 model **发真实 API**。自动构造 provider 在
//!   `aimux-providers::replay::rebuild_provider`(避免 core→providers 循环)。
//!
//! **回放范围(定稿 R8)**:MVP 仅支持 OpenAI `chat.completions` wire 格式
//! (非流式 body + 流式 SSE);其他 provider 明确返回 `Unsupported`,不猜测
//! 解析。后续如需通用 mock,走"录制规范化结果"或 decoder 下沉两条路线。

use async_trait::async_trait;
use std::sync::Arc;

use crate::error::AiMuxError;
use crate::generate::{GenerateTextOptions, GenerateTextResult, generate_text};
use crate::language_model::LanguageModel;
use crate::language_model_message::LanguageModelPrompt;
use crate::message::{MessageContent, ModelMessage, ModelPrompt};
use crate::options::{CallOptions, ToolChoice};
use crate::recording::Recording;
use crate::result::{GenerateContent, GenerateResult, StreamResult};
use crate::stream_part::StreamPart;
use crate::types::{FinishReason, FinishReasonUnified, ResponseMetadata, Usage, Warning};

// ── 匹配策略 ────────────────────────────────────────────────────────────────

/// 可插拔匹配策略:从录制集中选一次命中。
pub trait ReplayMatcher: Send + Sync {
    /// 按输入侧匹配;未命中返回明确错误。
    fn r#match<'a>(
        &self,
        options: &CallOptions,
        recordings: &'a [Recording],
    ) -> Result<&'a Recording, AiMuxError>;
}

/// 精确匹配:provider/model_id 相同,且 prompt + 影响响应的可重放选项
/// (temperature/max_output_tokens/seed/response_format/tools/tool_choice)
/// 规范相同才命中。运行时字段(call_id/abort_signal/recording_context)与脱敏
/// 字段(headers/provider_options/body_overrides)不参与比较。
pub struct ExactMatcher {
    provider: String,
    model_id: String,
}

impl ExactMatcher {
    pub fn new(provider: impl Into<String>, model_id: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            model_id: model_id.into(),
        }
    }
}

impl ReplayMatcher for ExactMatcher {
    fn r#match<'a>(
        &self,
        options: &CallOptions,
        recordings: &'a [Recording],
    ) -> Result<&'a Recording, AiMuxError> {
        let needle = canonical_call_key(options);
        recordings
            .iter()
            .find(|r| {
                r.provider.provider == self.provider
                    && r.provider.model_id == self.model_id
                    && canonical_recording_key(r) == needle
            })
            .ok_or_else(|| {
                AiMuxError::InvalidArgument("mock replay: no exact matching recording".into())
            })
    }
}

/// `CallOptions` 中影响响应、可重放选项的规范键(ExactMatcher 用)。
///
/// 含 prompt + temperature/max_output_tokens/seed/response_format/tools/
/// tool_choice;排除运行时字段(call_id/abort_signal/recording_context)与脱敏
/// 字段(headers/provider_options/body_overrides)。序列化为 `Value` 后比较,
/// 等价于"稳定规范哈希"但无碰撞风险。
fn canonical_call_key(opts: &CallOptions) -> serde_json::Value {
    serde_json::json!({
        "prompt": serde_json::to_value(&opts.prompt).unwrap_or_default(),
        "temperature": opts.temperature,
        "max_output_tokens": opts.max_output_tokens,
        "seed": opts.seed,
        "response_format": serde_json::to_value(&opts.response_format).unwrap_or_default(),
        "tools": serde_json::to_value(&opts.tools).unwrap_or_default(),
        "tool_choice": serde_json::to_value(&opts.tool_choice).unwrap_or_default(),
    })
}

/// 从录制侧提取同样的规范键(向后兼容:缺失 Option 字段按 null,缺失
/// `tool_choice` 按 `ToolChoice::default()`=Auto,与 `CallOptions` 缺省一致)。
fn canonical_recording_key(rec: &Recording) -> serde_json::Value {
    let o = &rec.input.options;
    serde_json::json!({
        "prompt": serde_json::to_value(&rec.input.prompt).unwrap_or_default(),
        "temperature": o.get("temperature").cloned().unwrap_or_default(),
        "max_output_tokens": o.get("max_output_tokens").cloned().unwrap_or_default(),
        "seed": o.get("seed").cloned().unwrap_or_default(),
        "response_format": o.get("response_format").cloned().unwrap_or_default(),
        "tools": o.get("tools").cloned().unwrap_or_default(),
        "tool_choice": o
            .get("tool_choice")
            .cloned()
            .unwrap_or_else(|| serde_json::to_value(ToolChoice::default()).unwrap_or_default()),
    })
}

/// 打分匹配:provider/model_id 必匹配。
///
/// 分数 = prompt 公共前缀消息数 × 100 + 第一个不同消息的文本 LCP 长度
/// + temperature 一致 +1。零分(完全不相关)不命中;平局取首个(确定性)。
///
/// 与 RFC-0015 LCP 同源,适合"相同前缀的请求"命中。
pub struct ScoreMatcher {
    provider: String,
    model_id: String,
}

impl ScoreMatcher {
    pub fn new(provider: impl Into<String>, model_id: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            model_id: model_id.into(),
        }
    }
}

impl ReplayMatcher for ScoreMatcher {
    fn r#match<'a>(
        &self,
        options: &CallOptions,
        recordings: &'a [Recording],
    ) -> Result<&'a Recording, AiMuxError> {
        let mut best: Option<(&Recording, u64)> = None;
        for r in recordings {
            if r.provider.provider != self.provider || r.provider.model_id != self.model_id {
                continue;
            }
            let score = match_score(options, r);
            if score == 0 {
                continue; // 完全不相关,不命中
            }
            if best.map(|(_, s)| score > s).unwrap_or(true) {
                best = Some((r, score));
            }
        }
        best.map(|(r, _)| r).ok_or_else(|| {
            AiMuxError::InvalidArgument("mock replay: no scoring match for this input".into())
        })
    }
}

/// 消息的首个文本内容(非文本消息回退到序列化)。
fn message_text(m: &crate::language_model_message::LanguageModelPromptMessage) -> String {
    match m.content.first() {
        Some(crate::content::ContentPart::Text { text, .. }) => text.clone(),
        _ => serde_json::to_string(&m.content).unwrap_or_default(),
    }
}

/// 计算一次候选录制的匹配分。
///
/// prompt 相关性 = 公共前缀消息数 × 100 + 第一个不同消息(或最后一个公共
/// 消息)的文本 LCP 字节数。两者都为 0(完全无关)时直接返回 0,不命中;
/// temperature 一致**只能在已有 prompt 相关性时加分**(否则会命中完全无关
/// 但 temperature 碰巧一致的请求)。
fn match_score(options: &CallOptions, rec: &Recording) -> u64 {
    // 公共前缀消息数(role + content 完全相同)。
    let common = options
        .prompt
        .iter()
        .zip(rec.input.prompt.iter())
        .take_while(|(a, b)| a.role == b.role && a.content == b.content)
        .count();
    // 字符级 LCP:第一个不同消息(或双方最后一个消息),在文本内容上计算
    // (避免 JSON 结构前缀的伪匹配)。任一 prompt 为空时无消息可比,LCP=0
    // (显式处理,避免 `len()-1` 下溢:debug panic / release wrap)。
    let min_len = options.prompt.len().min(rec.input.prompt.len());
    let lcp: u64 = if min_len == 0 {
        0
    } else {
        let lcp_idx = common.min(min_len - 1);
        match (options.prompt.get(lcp_idx), rec.input.prompt.get(lcp_idx)) {
            (Some(a), Some(b)) => message_text(a)
                .bytes()
                .zip(message_text(b).bytes())
                .take_while(|(x, y)| x == y)
                .count() as u64,
            _ => 0,
        }
    };
    let prompt_relevance = (common as u64) * 100 + lcp;
    if prompt_relevance == 0 {
        return 0; // 完全无关:不命中,temperature 也不加分(A1)。
    }
    // temperature 一致加分(弱信号,仅在 prompt 相关时生效)。
    let mut score = prompt_relevance;
    if options.temperature
        == rec
            .input
            .options
            .get("temperature")
            .and_then(|v| v.as_f64())
    {
        score += 1;
    }
    score
}

/// 前缀匹配(P5):录制的 prompt 是输入 prompt 的公共前缀(角色 + content
/// 逐消息完全相同)即命中;取前缀最长的录制,平局取首个(确定性)。
///
/// 与 RFC-0015 LCP 同源,适合"相同前缀的请求"命中(如多轮对话继续 + 请求
/// 变长时复用同一录制)。零公共消息(完全不相关)不命中。
pub struct PrefixMatcher {
    provider: String,
    model_id: String,
}

impl PrefixMatcher {
    pub fn new(provider: impl Into<String>, model_id: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            model_id: model_id.into(),
        }
    }
}

impl ReplayMatcher for PrefixMatcher {
    fn r#match<'a>(
        &self,
        options: &CallOptions,
        recordings: &'a [Recording],
    ) -> Result<&'a Recording, AiMuxError> {
        let mut best: Option<(&Recording, usize)> = None;
        for r in recordings {
            if r.provider.provider != self.provider || r.provider.model_id != self.model_id {
                continue;
            }
            // 公共前缀消息数;要求覆盖录制全部消息(录制是输入的前缀)。
            let common = options
                .prompt
                .iter()
                .zip(r.input.prompt.iter())
                .take_while(|(a, b)| a.role == b.role && a.content == b.content)
                .count();
            if common < r.input.prompt.len() {
                continue; // 录制未被输入完整包含,不命中。
            }
            if common > 0 && best.map(|(_, c)| common > c).unwrap_or(true) {
                best = Some((r, common));
            }
        }
        best.map(|(r, _)| r).ok_or_else(|| {
            AiMuxError::InvalidArgument("mock replay: no prefix match for this input".into())
        })
    }
}

// ── MockReplayModel ─────────────────────────────────────────────────────────

/// Mock 响应回放器:实现 `LanguageModel`,按输入匹配录制响应,**不发真实 API**。
pub struct MockReplayModel {
    provider: String,
    model_id: String,
    recordings: Vec<Recording>,
    matcher: Arc<dyn ReplayMatcher>,
}

impl MockReplayModel {
    /// 默认用 [`ScoreMatcher`]。
    pub fn new(
        provider: impl Into<String>,
        model_id: impl Into<String>,
        recordings: Vec<Recording>,
    ) -> Self {
        let provider = provider.into();
        let model_id = model_id.into();
        let matcher = Arc::new(ScoreMatcher::new(provider.clone(), model_id.clone()));
        Self {
            provider,
            model_id,
            recordings,
            matcher,
        }
    }

    /// 自定义匹配策略。
    pub fn with_matcher(
        provider: impl Into<String>,
        model_id: impl Into<String>,
        recordings: Vec<Recording>,
        matcher: Arc<dyn ReplayMatcher>,
    ) -> Self {
        Self {
            provider: provider.into(),
            model_id: model_id.into(),
            recordings,
            matcher,
        }
    }

    /// 从 jsonl 录制文件加载(每行一条 `Recording`)。
    /// provider/model_id 取首条录制;同一文件应同源录制。
    pub fn from_jsonl(path: &str) -> Result<Self, AiMuxError> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| AiMuxError::InvalidArgument(format!("mock replay: {e}")))?;
        let mut recordings = Vec::new();
        for (idx, line) in content.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let rec: Recording = serde_json::from_str(line)
                .map_err(|e| AiMuxError::Json(format!("mock replay line {}: {e}", idx + 1)))?;
            recordings.push(rec);
        }
        if recordings.is_empty() {
            return Err(AiMuxError::InvalidArgument(
                "mock replay: empty recording file".into(),
            ));
        }
        let provider = recordings[0].provider.provider.clone();
        let model_id = recordings[0].provider.model_id.clone();
        Ok(Self::new(provider, model_id, recordings))
    }

    /// 已加载的录制(只读)。
    pub fn recordings(&self) -> &[Recording] {
        &self.recordings
    }
}

#[async_trait]
impl LanguageModel for MockReplayModel {
    fn provider(&self) -> &str {
        &self.provider
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        let rec = self.matcher.r#match(options, &self.recordings)?;
        rebuild_generate_result(rec)
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let rec = self.matcher.r#match(options, &self.recordings)?;
        rebuild_stream_result(rec)
    }
}

// ── 从录制重建结果(OpenAI chat.completions MVP)──────────────────────────

/// 取录制中最后一次有响应的 exchange。
fn last_response(rec: &Recording) -> Result<&crate::recording::ResponseRecord, AiMuxError> {
    rec.exchanges
        .iter()
        .rev()
        .find_map(|e| e.response.as_ref())
        .ok_or_else(|| AiMuxError::InvalidArgument("mock replay: recording has no response".into()))
}

/// 解析 finish_reason 字符串为 unified 枚举。
fn parse_finish(raw: Option<&str>) -> (FinishReasonUnified, Option<String>) {
    let unified = match raw {
        Some("stop") => FinishReasonUnified::Stop,
        Some("length") => FinishReasonUnified::Length,
        Some("content_filter") => FinishReasonUnified::ContentFilter,
        Some("tool_calls") => FinishReasonUnified::ToolCalls,
        Some("error") => FinishReasonUnified::Error,
        _ => FinishReasonUnified::Other,
    };
    (unified, raw.map(|s| s.to_string()))
}

/// 从 `serde_json::Value` 取一个 u64 usage 计数并转 u32;溢出返回明确错误
/// (替代 `as u64 as u32` 的静默截断)。`null`/缺失 → `None`。
fn u32_from_json(v: &serde_json::Value, field: &str) -> Result<Option<u32>, AiMuxError> {
    v.as_u64()
        .map(|n| {
            u32::try_from(n)
                .map_err(|_| AiMuxError::Json(format!("mock replay: usage '{field}' overflows u32: {n}")))
        })
        .transpose()
}

/// 从 OpenAI `usage` 对象提取 core Usage。
fn parse_usage(v: &serde_json::Value) -> Result<Usage, AiMuxError> {
    let u = &v["usage"];
    if u.is_null() {
        return Ok(Usage::default());
    }
    let prompt_details = &u["prompt_tokens_details"];
    let completion_details = &u["completion_tokens_details"];
    Ok(Usage {
        input_tokens: crate::types::TokenUsage {
            total: u32_from_json(&u["prompt_tokens"], "prompt_tokens")?,
            no_cache: None,
            cache_read: u32_from_json(
                &prompt_details["cached_tokens"],
                "prompt_tokens_details.cached_tokens",
            )?,
            cache_write: None,
            text: None,
            reasoning: None,
        },
        output_tokens: crate::types::TokenUsage {
            total: u32_from_json(&u["completion_tokens"], "completion_tokens")?,
            no_cache: None,
            cache_read: None,
            cache_write: None,
            text: None,
            reasoning: u32_from_json(
                &completion_details["reasoning_tokens"],
                "completion_tokens_details.reasoning_tokens",
            )?,
        },
        raw: Some(u.clone()),
    })
}

/// 重建非流式结果。
///
/// 支持 OpenAI `chat.completions` 格式(`choices[0].message.content` /
/// `finish_reason` / `usage`);其他格式把整个 body 作为纯文本 + Warning。
fn rebuild_generate_result(rec: &Recording) -> Result<GenerateResult, AiMuxError> {
    let resp = last_response(rec)?;
    let body = resp
        .body
        .as_deref()
        .ok_or_else(|| AiMuxError::InvalidArgument("mock replay: response has no body".into()))?;
    let v: serde_json::Value =
        serde_json::from_str(body).map_err(|e| AiMuxError::Json(format!("mock replay: {e}")))?;

    let mut warnings = Vec::new();
    let content = if let Some(text) = v["choices"][0]["message"]["content"].as_str() {
        if text.is_empty() {
            Vec::new()
        } else {
            vec![GenerateContent::Text {
                text: text.to_string(),
                provider_metadata: None,
            }]
        }
    } else if v.get("choices").is_some() {
        Vec::new()
    } else {
        // 非 OpenAI 格式:降级为纯文本 + warning(记录诊断用)。
        warnings.push(Warning::Unsupported {
            feature: "mock replay format".to_string(),
            details: Some(
                "response is not an OpenAI chat.completions body; returned as text".into(),
            ),
        });
        vec![GenerateContent::Text {
            text: body.to_string(),
            provider_metadata: None,
        }]
    };

    let raw_finish = v["choices"][0]["finish_reason"]
        .as_str()
        .map(|s| s.to_string());
    let (unified, raw) = parse_finish(raw_finish.as_deref());

    Ok(GenerateResult {
        content,
        finish_reason: FinishReason { unified, raw },
        usage: parse_usage(&v)?,
        warnings,
        provider_metadata: None,
        response: ResponseMetadata {
            id: v["id"].as_str().map(|s| s.to_string()),
            timestamp: None,
            model_id: v["model"].as_str().map(|s| s.to_string()),
        },
        request_body: None,
        response_headers: None,
    })
}

/// 重建流式结果:OpenAI SSE body 逐行 → `StreamPart`。
///
/// `data: {json}` → TextDelta(delta.content)/Finish;`data: [DONE]` → Finish;
/// 其他行忽略。非 OpenAI 格式按行 Raw 降级。
fn rebuild_stream_result(rec: &Recording) -> Result<StreamResult, AiMuxError> {
    let resp = last_response(rec)?;
    let body = resp
        .body
        .as_deref()
        .ok_or_else(|| AiMuxError::InvalidArgument("mock replay: response has no body".into()))?;

    let mut parts: Vec<Result<StreamPart, AiMuxError>> = Vec::new();
    let id = "mock-replay".to_string();
    parts.push(Ok(StreamPart::StreamStart { warnings: vec![] }));

    let mut saw_openai = false;
    for block in body.split("\n\n") {
        let line = block.trim();
        let data = line.strip_prefix("data:").unwrap_or(line).trim();
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(data) else {
            continue;
        };
        if let Some(delta) = v["choices"][0]["delta"]["content"].as_str()
            && !delta.is_empty()
        {
            saw_openai = true;
            parts.push(Ok(StreamPart::TextDelta {
                id: id.clone(),
                delta: delta.to_string(),
                provider_metadata: None,
            }));
        }
        if let Some(fr) = v["choices"][0]["finish_reason"].as_str() {
            saw_openai = true;
            let (unified, raw) = parse_finish(Some(fr));
            parts.push(Ok(StreamPart::Finish {
                finish_reason: FinishReason { unified, raw },
                usage: parse_usage(&v)?,
                provider_metadata: None,
            }));
        }
    }

    // 没解析出 OpenAI 结构:非 OpenAI SSE 格式按行 Raw 降级。
    if !saw_openai && parts.len() == 1 {
        for line in body.lines() {
            parts.push(Ok(StreamPart::Raw {
                raw_value: serde_json::Value::String(line.to_string()),
            }));
        }
        parts.push(Ok(StreamPart::Finish {
            finish_reason: FinishReason {
                unified: FinishReasonUnified::Other,
                raw: None,
            },
            usage: Usage::default(),
            provider_metadata: None,
        }));
    }

    Ok(StreamResult {
        stream: Box::pin(futures::stream::iter(parts)),
        request_body: None,
        response_headers: None,
    })
}

// ── 请求回放(P4,provider 无关)──────────────────────────────────────────

/// 请求回放的覆盖项。
///
/// 用于"改 prompt / 参数重发"(RFC-0023 §3.6.1 用途)。`None` 字段 = 用录制
/// 值;传入值替换录制值。
#[derive(Debug, Clone, Default)]
pub struct ReplayOverrides {
    /// 替换录制中的整个 prompt(默认用录制原样)。
    pub prompt: Option<LanguageModelPrompt>,
    /// 替换 temperature。
    pub temperature: Option<f64>,
    /// 替换 max_output_tokens。
    pub max_output_tokens: Option<u32>,
}

/// 请求回放:用录制输入重建调用,经调用方提供的 model 重发(**真实 API**)。
///
/// - 输入/参数从 `recording.input`(prompt + 序列化的 CallOptions)重建;
///   经 `generate_text` 走完整管线(录制层 A / session / 日志),重发的调用
///   本身也会被录制(若 recorder 开启)——CI 回归对比用。
/// - provider 自动构造不在本函数(`rebuild_provider` 在 aimux-providers,避免
///   core→providers 循环依赖)。原生协议 provider(anthropic/google/...)请
///   直接用实例调本函数。
/// - 注意事项:
///   - `recording.input.options` 中的 headers/provider_options 已脱敏,
///     重发会用脱敏后的 `[REDACTED]` 值——需要真实头时用 `overrides` 或
///     重建 provider 时补充。
///   - 逐消息 `provider_options`(如 anthropic cacheControl)不参与重建。
pub async fn replay_with_model(
    recording: &Recording,
    model: &dyn LanguageModel,
    overrides: Option<&ReplayOverrides>,
) -> Result<GenerateTextResult, AiMuxError> {
    // 1. 从录制输入重建 CallOptions(round-trip:录制时由 CallOptions 序列化)。
    let mut call_options: CallOptions = serde_json::from_value(recording.input.options.clone())
        .map_err(|e| AiMuxError::Json(format!("mock replay: input options invalid: {e}")))?;

    // 2. 应用 overrides。
    if let Some(o) = overrides {
        if let Some(prompt) = &o.prompt {
            call_options.prompt = prompt.clone();
        }
        if o.temperature.is_some() {
            call_options.temperature = o.temperature;
        }
        if o.max_output_tokens.is_some() {
            call_options.max_output_tokens = o.max_output_tokens;
        }
    }

    // 3. 转回用户侧类型,经 generate_text 重发。
    let prompt = model_prompt_from_lm(&call_options.prompt);
    let options = generate_options_from_call_options(call_options);
    generate_text(model, prompt, options).await
}

/// `LanguageModelPrompt`(provider 侧)→ `ModelPrompt`(用户侧)。
///
/// 逐消息 `provider_options` 丢弃(用户侧无对应字段;语义不丢,仅 cacheControl
/// 之类的 provider 提示不参与重放)。
fn model_prompt_from_lm(prompt: &LanguageModelPrompt) -> ModelPrompt {
    ModelPrompt::Messages(
        prompt
            .iter()
            .map(|m| ModelMessage {
                role: m.role,
                content: MessageContent::Parts(m.content.clone()),
            })
            .collect(),
    )
}

/// `CallOptions` → `GenerateTextOptions`(record 侧到用户侧的反向映射)。
///
/// `instructions` 已烘焙进 prompt(system 消息),重建时置 None;
/// `abort_signal` 运行时句柄不跨 JSON,重建时置 None。
fn generate_options_from_call_options(o: CallOptions) -> GenerateTextOptions {
    GenerateTextOptions {
        max_output_tokens: o.max_output_tokens,
        temperature: o.temperature,
        stop_sequences: o.stop_sequences,
        top_p: o.top_p,
        top_k: o.top_k,
        presence_penalty: o.presence_penalty,
        frequency_penalty: o.frequency_penalty,
        response_format: o.response_format,
        seed: o.seed,
        tools: o.tools,
        tool_choice: if o.tool_choice == crate::tool::ToolChoice::Auto {
            None
        } else {
            Some(o.tool_choice)
        },
        headers: o.headers,
        provider_options: o.provider_options,
        reasoning: o.reasoning,
        instructions: None,
        body_overrides: o.body_overrides,
        max_retries: o.max_retries,
        timeout: o.timeout,
        session_id: o.session_id,
        abort_signal: None,
        include_raw_chunks: o.include_raw_chunks,
    }
}

// ── 测试 ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generate::GenerateTextOptions;
    use crate::recording::{
        HttpExchange, HttpRecord, InputRecord, ProviderRecord, ResponseRecord, TimingRecord,
    };
    use futures::StreamExt;

    fn sample_options(text: &str, temperature: Option<f64>) -> CallOptions {
        let prompt = vec![crate::language_model_message::LanguageModelPromptMessage {
            role: crate::message::Role::User,
            content: vec![crate::content::ContentPart::Text {
                text: text.to_string(),
                provider_options: None,
            }],
            provider_options: None,
        }];
        GenerateTextOptions {
            temperature,
            ..Default::default()
        }
        .into_call_options(prompt)
    }

    /// 构造 OpenAI 格式录制。
    fn openai_recording(trace_id: &str, prompt_text: &str, reply: &str, finish: &str) -> Recording {
        let input_prompt: crate::language_model_message::LanguageModelPrompt =
            vec![crate::language_model_message::LanguageModelPromptMessage {
                role: crate::message::Role::User,
                content: vec![crate::content::ContentPart::Text {
                    text: prompt_text.to_string(),
                    provider_options: None,
                }],
                provider_options: None,
            }];
        let body = serde_json::json!({
            "id": "chatcmpl-mock",
            "object": "chat.completion",
            "model": "gpt-4o",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": reply },
                "finish_reason": finish
            }],
            "usage": { "prompt_tokens": 5, "completion_tokens": 3, "total_tokens": 8 }
        });
        Recording {
            schema: crate::recording::RECORDING_SCHEMA,
            call_id: trace_id.to_string(),
            recorded_at: "2026-08-06T00:00:00Z".to_string(),
            input: InputRecord {
                prompt: input_prompt,
                options: serde_json::json!({ "temperature": 0.7 }),
            },
            provider: ProviderRecord {
                provider: "openai".into(),
                model_id: "gpt-4o".into(),
                base_url: None,
                api_key_source: "none".into(),
                profile: None,
                provider_options: None,
            },
            exchanges: vec![HttpExchange {
                attempt: 0,
                request: HttpRecord {
                    method: "post".into(),
                    url: "https://api.openai.com/v1/chat/completions".into(),
                    headers: vec![],
                    body: Some("{}".into()),
                },
                response: Some(ResponseRecord {
                    status: 200,
                    headers: vec![],
                    body: Some(body.to_string()),
                    stream_chunks: None,
                    ttfb_ms: None,
                }),
                timing: TimingRecord {
                    latency_ms: 10,
                    ttfb_ms: None,
                },
                error: None,
                finalized: true,
            }],
            outcome: crate::recording::OutcomeRecord {
                status: crate::recording::OutcomeStatus::Success,
                finish_reason: Some("stop".into()),
                error: None,
                usage: None,
            },
            complete: true,
            transport_closed: true,
            session_id: None,
            step: None,
        }
    }

    #[test]
    fn exact_matcher_hits_only_identical_prompt() {
        // openai_recording 的 options 含 temperature=0.7;ExactMatcher 现把生成
        // 参数纳入匹配,故命中需 prompt + temperature 都一致(A8)。
        let recs = [openai_recording("t1", "ping", "pong", "stop")];
        let matcher = ExactMatcher::new("openai", "gpt-4o");
        assert!(
            matcher
                .r#match(&sample_options("ping", Some(0.7)), &recs)
                .is_ok()
        );
        // prompt 不同 → miss(temperature 已对齐,差异仅来自 prompt)。
        assert!(
            matcher
                .r#match(&sample_options("pong", Some(0.7)), &recs)
                .is_err()
        );
    }

    #[test]
    fn exact_matcher_different_temperature_misses() {
        // A8:prompt 相同但 temperature 不同 → miss。
        let recs = [openai_recording("t1", "ping", "pong", "stop")]; // temp 0.7
        let matcher = ExactMatcher::new("openai", "gpt-4o");
        assert!(
            matcher
                .r#match(&sample_options("ping", Some(0.1)), &recs)
                .is_err()
        );
        // temperature 一致 → hit(对照)。
        assert!(
            matcher
                .r#match(&sample_options("ping", Some(0.7)), &recs)
                .is_ok()
        );
    }

    #[test]
    fn exact_matcher_different_options_misses() {
        // A8:prompt + temperature 都相同,但 max_output_tokens 不同 → miss。
        let mut rec = openai_recording("t1", "ping", "pong", "stop");
        // 录制侧用完整 CallOptions(temperature 0.7 + max_output_tokens 128),
        // 与真实录制路径(InputRecord::from_call_options)一致。
        let mut call = sample_options("ping", Some(0.7));
        call.max_output_tokens = Some(128);
        rec.input.options = serde_json::to_value(&call).unwrap();
        let recs = [rec];
        let matcher = ExactMatcher::new("openai", "gpt-4o");

        // max_output_tokens=256 ≠ 128 → miss。
        let mut req = sample_options("ping", Some(0.7));
        req.max_output_tokens = Some(256);
        assert!(matcher.r#match(&req, &recs).is_err());

        // 完全一致 → hit(对照)。
        let mut req2 = sample_options("ping", Some(0.7));
        req2.max_output_tokens = Some(128);
        assert!(matcher.r#match(&req2, &recs).is_ok());
    }

    #[test]
    fn score_matcher_prefers_longer_prefix_and_ties_to_first() {
        let recs = [
            openai_recording("t1", "hello", "a", "stop"),
            openai_recording("t2", "hello world", "b", "stop"),
        ];
        let matcher = ScoreMatcher::new("openai", "gpt-4o");
        // "hello world!" 与 t2 前缀更长。
        let hit = matcher
            .r#match(&sample_options("hello world!", None), &recs)
            .unwrap();
        assert_eq!(hit.call_id, "t2");
        // 平局(t1/t2 都是 "hello" 前缀)→ 取首个 t1。
        let hit = matcher
            .r#match(&sample_options("hello", None), &recs)
            .unwrap();
        assert_eq!(hit.call_id, "t1");
        // temperature 一致加分。
        let hit = matcher
            .r#match(&sample_options("hello", Some(0.7)), &recs)
            .unwrap();
        assert_eq!(hit.call_id, "t1");
    }

    #[test]
    fn match_score_empty_prompt_no_underflow() {
        // A2:任一 prompt 为空时 `len()-1` 曾下溢(debug panic / release wrap)。
        let rec = openai_recording("t1", "hello", "a", "stop");
        let empty_req = GenerateTextOptions::default().into_call_options(vec![]);
        // 请求侧空 prompt → 安全返回 0,不 panic。
        assert_eq!(match_score(&empty_req, &rec), 0);
        // 录制侧空 prompt → 安全返回 0。
        let mut rec_empty = rec.clone();
        rec_empty.input.prompt = vec![];
        assert_eq!(match_score(&sample_options("hello", None), &rec_empty), 0);
        // 双侧空 prompt → 安全返回 0。
        assert_eq!(match_score(&empty_req, &rec_empty), 0);
    }

    #[test]
    fn score_matcher_unrelated_prompt_with_matching_temperature_misses() {
        // A1:双方 temperature 都是 None(相等),但 prompt 完全无关 → 必须 miss。
        // 旧实现 temperature 一致 +1 会使 score=1>0 错误命中。
        let mut rec = openai_recording("t1", "alpha", "a", "stop");
        rec.input.options = serde_json::json!({}); // temperature 缺失→None
        let recs = [rec];
        let matcher = ScoreMatcher::new("openai", "gpt-4o");
        // "zzzzz" 与 "alpha" 无公共前缀消息、LCP=0。
        assert!(matcher
            .r#match(&sample_options("zzzzz", None), &recs)
            .is_err());
    }

    #[test]
    fn score_matcher_related_prompt_with_matching_temperature_hits() {
        // A1:prompt 相关 + temperature 一致 → 正常命中(temperature 加分仍生效)。
        let recs = [openai_recording("t1", "hello", "a", "stop")]; // temp 0.7
        let matcher = ScoreMatcher::new("openai", "gpt-4o");
        let hit = matcher
            .r#match(&sample_options("hello", Some(0.7)), &recs)
            .unwrap();
        assert_eq!(hit.call_id, "t1");
    }

    #[test]
    fn prefix_matcher_matches_message_prefix() {
        let rec = openai_recording("t1", "hello", "a", "stop"); // 单消息 "hello"
        let matcher = PrefixMatcher::new("openai", "gpt-4o");
        // 输入两条消息 ["hello", "world"]:rec 是消息级前缀 → 命中。
        let prompt = vec![
            crate::language_model_message::LanguageModelPromptMessage {
                role: crate::message::Role::User,
                content: vec![crate::content::ContentPart::Text {
                    text: "hello".into(),
                    provider_options: None,
                }],
                provider_options: None,
            },
            crate::language_model_message::LanguageModelPromptMessage {
                role: crate::message::Role::User,
                content: vec![crate::content::ContentPart::Text {
                    text: "world".into(),
                    provider_options: None,
                }],
                provider_options: None,
            },
        ];
        let opts = GenerateTextOptions::default().into_call_options(prompt);
        let recs = [rec.clone()];
        let hit = matcher.r#match(&opts, &recs).unwrap();
        assert_eq!(hit.call_id, "t1");

        // 首消息不同 → 不命中(整条消息级前缀,非字符级)。
        let opts2 = GenerateTextOptions::default().into_call_options(vec![
            crate::language_model_message::LanguageModelPromptMessage {
                role: crate::message::Role::User,
                content: vec![crate::content::ContentPart::Text {
                    text: "hi".into(),
                    provider_options: None,
                }],
                provider_options: None,
            },
        ]);
        assert!(matcher.r#match(&opts2, &[rec]).is_err());
    }

    #[test]
    fn prefix_matcher_prefers_longest_prefix() {
        // 多消息录制:rec-a = ["hello"],rec-b = ["hello", "world"]。
        let mut rec_a = openai_recording("ta", "hello", "a", "stop");
        let mut rec_b = openai_recording("tb", "hello world", "b", "stop");
        let mk = |text: &str| crate::language_model_message::LanguageModelPromptMessage {
            role: crate::message::Role::User,
            content: vec![crate::content::ContentPart::Text {
                text: text.into(),
                provider_options: None,
            }],
            provider_options: None,
        };
        rec_a.input.prompt = vec![mk("hello")];
        rec_b.input.prompt = vec![mk("hello"), mk("world")];

        let matcher = PrefixMatcher::new("openai", "gpt-4o");
        // 输入 ["hello", "world", "!"]:rec-a 与 rec-b 都是前缀,rec-b 更长 → 命中 tb。
        let opts = GenerateTextOptions::default().into_call_options(vec![
            mk("hello"),
            mk("world"),
            mk("!"),
        ]);
        let recs = [rec_a, rec_b];
        let hit = matcher.r#match(&opts, &recs).unwrap();
        assert_eq!(hit.call_id, "tb");
    }

    #[test]
    fn unmatched_returns_clear_error() {
        let recs = [openai_recording("t1", "ping", "pong", "stop")];
        let model = MockReplayModel::new("openai", "gpt-4o", vec![recs[0].clone()]);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let err = rt.block_on(async {
            model
                .do_generate(&sample_options("nope", None))
                .await
                .unwrap_err()
        });
        assert!(err.to_string().contains("mock replay"), "{err}");
    }

    #[test]
    fn mock_model_rebuilds_generate_result_from_openai_body() {
        let rec = openai_recording("t1", "ping", "pong", "stop");
        let model = MockReplayModel::new("openai", "gpt-4o", vec![rec]);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async {
            model
                .do_generate(&sample_options("ping", None))
                .await
                .unwrap()
        });
        let Some(GenerateContent::Text { text, .. }) = result.content.first() else {
            panic!("expected text content");
        };
        assert_eq!(text, "pong");
        assert_eq!(result.finish_reason.unified, FinishReasonUnified::Stop);
        assert_eq!(result.usage.input_tokens.total, Some(5));
        assert_eq!(result.response.model_id.as_deref(), Some("gpt-4o"));
    }

    #[test]
    fn mock_model_usage_overflow_errors() {
        // A11:usage 计数超过 u32 上限应返回明确错误,而非 `as u32` 静默截断。
        let mut rec = openai_recording("t1", "ping", "pong", "stop");
        let body = serde_json::json!({
            "id": "chatcmpl-mock",
            "model": "gpt-4o",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "pong" },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 5_000_000_000_u64, "completion_tokens": 3, "total_tokens": 8 }
        });
        rec.exchanges[0].response.as_mut().unwrap().body = Some(body.to_string());
        let model = MockReplayModel::new("openai", "gpt-4o", vec![rec]);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let err = rt
            .block_on(async { model.do_generate(&sample_options("ping", None)).await.unwrap_err() });
        assert!(matches!(err, AiMuxError::Json(_)), "{err}");
        assert!(err.to_string().contains("overflows u32"), "{err}");
    }

    #[test]
    fn mock_model_rebuilds_stream_from_sse() {
        let sse = "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\",\"content\":\"hel\"}}]}\n\n\
                   data: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n\n\
                   data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n\
                   data: [DONE]\n\n";
        let mut rec = openai_recording("t1", "ping", "x", "stop");
        rec.exchanges[0].response.as_mut().unwrap().body = Some(sse.to_string());
        rec.exchanges[0].response.as_mut().unwrap().stream_chunks = Some(4);
        let model = MockReplayModel::new("openai", "gpt-4o", vec![rec]);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let parts: Vec<StreamPart> = rt.block_on(async {
            model
                .do_stream(&sample_options("ping", None))
                .await
                .unwrap()
                .stream
                .collect::<Vec<_>>()
                .await
                .into_iter()
                .map(|p| p.unwrap())
                .collect()
        });
        assert!(matches!(parts[0], StreamPart::StreamStart { .. }));
        let deltas: Vec<String> = parts
            .iter()
            .filter_map(|p| match p {
                StreamPart::TextDelta { delta, .. } => Some(delta.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(deltas, vec!["hel", "lo"]);
        assert!(parts.iter().any(|p| matches!(p, StreamPart::Finish { .. })));
    }

    #[test]
    fn non_openai_body_falls_back_to_text_with_warning() {
        let mut rec = openai_recording("t1", "ping", "x", "stop");
        rec.exchanges[0].response.as_mut().unwrap().body = Some("{\"foo\":\"bar\"}".to_string()); // 非 chat.completions
        let model = MockReplayModel::new("openai", "gpt-4o", vec![rec]);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async {
            model
                .do_generate(&sample_options("ping", None))
                .await
                .unwrap()
        });
        // 整 body 作为文本 + warning。
        assert!(!result.warnings.is_empty());
        let Some(GenerateContent::Text { text, .. }) = result.content.first() else {
            panic!("expected text");
        };
        assert!(text.contains("foo"));
    }

    #[test]
    fn from_jsonl_loads_recordings() {
        let dir = std::env::temp_dir().join(format!("aimux-replay-jsonl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rec.jsonl");
        let rec = openai_recording("t1", "ping", "pong", "stop");
        std::fs::write(&path, serde_json::to_string(&rec).unwrap() + "\n").unwrap();

        let model = MockReplayModel::from_jsonl(path.to_str().unwrap()).unwrap();
        assert_eq!(model.provider(), "openai");
        assert_eq!(model.model_id(), "gpt-4o");
        assert_eq!(model.recordings().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── P4 请求回放 ─────────────────────────────────────────────────────

    /// 测试用 echo model:把收到的 prompt 文本原样回显。
    #[derive(Clone)]
    struct EchoModel {
        provider: &'static str,
        model_id: &'static str,
    }

    impl EchoModel {
        fn new() -> Self {
            Self {
                provider: "openai",
                model_id: "gpt-4o",
            }
        }
    }

    #[async_trait]
    impl LanguageModel for EchoModel {
        fn provider(&self) -> &str {
            self.provider
        }
        fn model_id(&self) -> &str {
            self.model_id
        }
        async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
            let text = options
                .prompt
                .iter()
                .filter_map(|m| match m.content.first() {
                    Some(crate::content::ContentPart::Text { text, .. }) => Some(text.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(" ");
            Ok(GenerateResult {
                content: vec![GenerateContent::Text {
                    text,
                    provider_metadata: None,
                }],
                finish_reason: FinishReason {
                    unified: FinishReasonUnified::Stop,
                    raw: Some("stop".into()),
                },
                usage: Usage::default(),
                warnings: vec![],
                provider_metadata: None,
                response: ResponseMetadata::default(),
                request_body: None,
                response_headers: None,
            })
        }
        async fn do_stream(&self, _options: &CallOptions) -> Result<StreamResult, AiMuxError> {
            unreachable!("not used in tests")
        }
    }

    /// 构造带 options 的录制(temperature/max_output_tokens 有值)。
    /// options 用真实 CallOptions 序列化(与录制路径一致,全字段 round-trip)。
    fn optioned_recording(prompt_text: &str) -> Recording {
        let mut rec = openai_recording("t1", prompt_text, "pong", "stop");
        let mut call = sample_options(prompt_text, Some(0.7));
        call.max_output_tokens = Some(128);
        rec.input.options = serde_json::to_value(&call).unwrap();
        rec
    }

    #[test]
    fn replay_with_model_rebuilds_input_and_resends() {
        let rec = optioned_recording("hello");
        let model = EchoModel::new();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async { replay_with_model(&rec, &model, None).await.unwrap() });
        assert_eq!(result.text, "hello");
    }

    #[test]
    fn replay_with_model_applies_overrides() {
        let rec = optioned_recording("hello");
        let model = EchoModel::new();
        let overrides = ReplayOverrides {
            prompt: Some(vec![
                crate::language_model_message::LanguageModelPromptMessage {
                    role: crate::message::Role::User,
                    content: vec![crate::content::ContentPart::Text {
                        text: "overridden".into(),
                        provider_options: None,
                    }],
                    provider_options: None,
                },
            ]),
            ..Default::default()
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async {
            replay_with_model(&rec, &model, Some(&overrides))
                .await
                .unwrap()
        });
        assert_eq!(result.text, "overridden");
    }

    #[test]
    fn replay_with_model_bad_input_options_errors() {
        let mut rec = openai_recording("t1", "hello", "pong", "stop");
        rec.input.options = serde_json::json!({ "not": "call-options" });
        let model = EchoModel::new();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let err = rt.block_on(async { replay_with_model(&rec, &model, None).await.unwrap_err() });
        assert!(matches!(err, AiMuxError::Json(_)), "{err}");
    }
}
