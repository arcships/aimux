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
use crate::types::{FinishReason, FinishReasonUnified, ResponseMetadata, Usage};

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
/// 以及 headers/provider_options/body_overrides 规范相同才命中。运行时字段
/// (call_id/abort_signal/recording_context)不参与比较。
///
/// headers/provider_options/body_overrides 用**脱敏感知比较**:录制侧这些
/// 字段经 [`recording::redact_json`](crate::recording::redact_json) 脱敏
/// (敏感键值→`"[REDACTED]"`),脱敏值视为通配,匹配任意显式请求值;非脱敏
/// 部分仍精确比较。规则见 [`redaction_aware_eq`]。
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
                    && canonical_keys_match(&canonical_recording_key(r), &needle)
            })
            .ok_or_else(|| {
                AiMuxError::InvalidArgument("mock replay: no exact matching recording".into())
            })
    }
}

/// `CallOptions` 中影响响应、可重放选项的规范键(ExactMatcher 用)。
///
/// 含 prompt + temperature/max_output_tokens/seed/response_format/tools/
/// tool_choice + headers/provider_options/body_overrides;排除运行时字段
/// (call_id/abort_signal/recording_context)。序列化为 `Value` 后比较,
/// 等价于"稳定规范哈希"但无碰撞风险。headers/provider_options/body_overrides
/// 在比较时走脱敏感知语义(见 [`canonical_keys_match`])。
fn canonical_call_key(opts: &CallOptions) -> serde_json::Value {
    serde_json::json!({
        "prompt": serde_json::to_value(&opts.prompt).unwrap_or_default(),
        "temperature": opts.temperature,
        "max_output_tokens": opts.max_output_tokens,
        "seed": opts.seed,
        "response_format": serde_json::to_value(&opts.response_format).unwrap_or_default(),
        "tools": serde_json::to_value(&opts.tools).unwrap_or_default(),
        "tool_choice": serde_json::to_value(&opts.tool_choice).unwrap_or_default(),
        "headers": serde_json::to_value(&opts.headers).unwrap_or_default(),
        "provider_options": serde_json::to_value(&opts.provider_options).unwrap_or_default(),
        "body_overrides": serde_json::to_value(&opts.body_overrides).unwrap_or_default(),
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
        "headers": o.get("headers").cloned().unwrap_or_default(),
        "provider_options": o.get("provider_options").cloned().unwrap_or_default(),
        "body_overrides": o.get("body_overrides").cloned().unwrap_or_default(),
    })
}

/// 脱敏感知比较(用于 headers/provider_options/body_overrides)。
///
/// 录制侧这些字段经 [`recording::redact_json`](crate::recording::redact_json)
/// 脱敏:敏感键(authorization/api-key/apikey/key/token/cookie/...)的**值**
/// 被替换为字符串 `"[REDACTED]"`(键名保留)。比较规则(**保守原则——宁可 miss
/// 不可误 hit**):
/// - 录制值为 `"[REDACTED]"` → 视为通配,匹配任意**显式(非 null)**请求值
///   (脱敏后无法恢复原值,只能放行);请求值为 null/缺省则不匹配(录制侧曾有
///   值,请求侧缺失 → 视为差异);
/// - 录制值为 null/缺省 → 请求值也必须为 null/缺省;
/// - 否则精确比较(含结构:对象键集一致、数组长度一致,避免多/缺键伪命中)。
///
/// 注意:通配**仅对录制侧** `"[REDACTED]"` 生效——请求侧恰好等于 `"[REDACTED]"`
/// 的字面值不触发通配(仍走精确比较),因为只有录制脱敏路径会产生该哨兵。
fn redaction_aware_eq(rec: &serde_json::Value, req: &serde_json::Value) -> bool {
    // 1. 录制侧脱敏通配:匹配任意显式(非 null)请求值。
    if rec.as_str() == Some("[REDACTED]") {
        return !req.is_null();
    }
    // 2. null/缺省:双侧须同为 null。
    if rec.is_null() || req.is_null() {
        return rec.is_null() && req.is_null();
    }
    // 3. 结构化精确比较(递归,使嵌套脱敏值同样通配)。
    match (rec, req) {
        (serde_json::Value::Object(a), serde_json::Value::Object(b)) => {
            if a.len() != b.len() {
                return false; // 多/缺键 → miss(保守)
            }
            a.iter().all(|(k, rv)| {
                b.get(k)
                    .map(|qv| redaction_aware_eq(rv, qv))
                    .unwrap_or(false)
            })
        }
        (serde_json::Value::Array(a), serde_json::Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| redaction_aware_eq(x, y))
        }
        (x, y) => x == y,
    }
}

/// 比较规范键:headers/provider_options/body_overrides 用脱敏感知比较
/// (录制侧可能已脱敏);其余字段(prompt/temperature/...)精确比较。
///
/// 注意:`max_output_tokens` 等含 "token" 子串的字段在录制侧也会被
/// `redact_json` 脱敏(无关的录制侧副作用)。这些字段**不**走通配——若录制值
/// 被脱敏为 `"[REDACTED]"` 而请求值为数值,精确比较会 miss(保守:宁可 miss
/// 不可误 hit,因为 max_output_tokens 影响响应,通配会引入伪命中)。
fn canonical_keys_match(rec_key: &serde_json::Value, call_key: &serde_json::Value) -> bool {
    const REDACTED_FIELDS: [&str; 3] = ["headers", "provider_options", "body_overrides"];
    let (ro, co) = match (rec_key.as_object(), call_key.as_object()) {
        (Some(a), Some(b)) => (a, b),
        _ => return rec_key == call_key,
    };
    if ro.len() != co.len() {
        return false;
    }
    ro.iter().all(|(k, rv)| {
        co.get(k)
            .map(|qv| {
                if REDACTED_FIELDS.contains(&k.as_str()) {
                    redaction_aware_eq(rv, qv)
                } else {
                    rv == qv
                }
            })
            .unwrap_or(false)
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

/// 取录制中最后一次**成功** exchange 的响应(finalized + 无 error + 2xx)。
///
/// C4-9:旧实现 `find_map(|e| e.response.as_ref())` 会选中有响应但失败的
/// attempt(非 2xx / 带 error / 未终结),把错误响应当成功回放。这里只选
/// `finalized && error.is_none() && status 2xx` 的响应;无合法响应时返回
/// 明确错误(不降级到任意 attempt)。
fn last_response(rec: &Recording) -> Result<&crate::recording::ResponseRecord, AiMuxError> {
    for e in rec.exchanges.iter().rev() {
        if !e.finalized || e.error.is_some() {
            continue;
        }
        if let Some(resp) = e.response.as_ref()
            && (200..300).contains(&resp.status)
        {
            return Ok(resp);
        }
    }
    Err(AiMuxError::InvalidArgument(
        "mock replay: recording has no successful (2xx, finalized, error-free) response".into(),
    ))
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
            u32::try_from(n).map_err(|_| {
                AiMuxError::Json(format!("mock replay: usage '{field}' overflows u32: {n}"))
            })
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

/// 解析 OpenAI tool_call `arguments`(JSON 字符串)为 `Value`。
///
/// 非法 JSON → 回退为 `Value::String(raw)`,**不返回错误**:与 openai provider
/// 正向解析一致(`serde_json::from_str(args).unwrap_or_else(|_| Value::String(args))`)。
/// 部分流式拼接偶发非完整 JSON,provider 侧同样容忍——回放须与正向解析同语义,
/// 否则同一录制在真实调用与回放间行为分叉。
fn parse_tool_arguments(args: &str) -> serde_json::Value {
    serde_json::from_str(args).unwrap_or_else(|_| serde_json::Value::String(args.to_string()))
}

/// 重建非流式结果。
///
/// 仅支持 OpenAI `chat.completions` 格式:`choices[0].message` 的 `content`
/// (文本)+ `tool_calls`(C4-4,`content:null + tool_calls:[...]` 不得返回空);
/// 以及 `finish_reason` / `usage` / `id` / `model`。非 OpenAI 格式(无
/// `choices[0].message`)返回 [`AiMuxError::Unsupported`](A7),不再降级为文本。
fn rebuild_generate_result(rec: &Recording) -> Result<GenerateResult, AiMuxError> {
    let resp = last_response(rec)?;
    let body = resp
        .body
        .as_deref()
        .ok_or_else(|| AiMuxError::InvalidArgument("mock replay: response has no body".into()))?;
    let v: serde_json::Value =
        serde_json::from_str(body).map_err(|e| AiMuxError::Json(format!("mock replay: {e}")))?;

    // A7:必须有 choices[0].message 才算 OpenAI chat.completions;否则 Unsupported。
    let message = v
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|choice| choice.get("message"))
        .ok_or_else(|| {
            AiMuxError::Unsupported(
                "mock replay: response is not an OpenAI chat.completions body (no choices[0].message)"
                    .into(),
            )
        })?;

    // C4-4:content(null/空可) + tool_calls。content 为空但 tool_calls 存在时
    // 不得返回空 content。
    let mut content = Vec::new();
    if let Some(text) = message.get("content").and_then(|c| c.as_str())
        && !text.is_empty()
    {
        content.push(GenerateContent::Text {
            text: text.to_string(),
            provider_metadata: None,
        });
    }
    if let Some(tool_calls) = message.get("tool_calls").and_then(|t| t.as_array()) {
        for tc in tool_calls {
            let tool_call_id = tc
                .get("id")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string();
            let function = tc.get("function").cloned().unwrap_or_default();
            let tool_name = function
                .get("name")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string();
            let args_str = function
                .get("arguments")
                .and_then(|x| x.as_str())
                .unwrap_or("");
            let input = parse_tool_arguments(args_str);
            content.push(GenerateContent::ToolCall {
                tool_call_id,
                tool_name,
                input,
                provider_executed: None,
                dynamic: None,
                thought_signature: None,
                provider_metadata: None,
            });
        }
    }

    let raw_finish = v["choices"][0]["finish_reason"]
        .as_str()
        .map(|s| s.to_string());
    let (unified, raw) = parse_finish(raw_finish.as_deref());

    Ok(GenerateResult {
        content,
        finish_reason: FinishReason { unified, raw },
        usage: parse_usage(&v)?,
        warnings: Vec::new(),
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

/// 流式 tool_call 累加器(按 OpenAI `index` 稳定累积)。
struct ToolCallAccumulator {
    id: String,
    name: String,
    arguments: String,
}

/// 重建流式结果:OpenAI SSE body → `StreamPart`(镜像 openai provider 状态机)。
///
/// 支持(C4-8):
/// - `delta.content` → TextStart/TextDelta/TextEnd;
/// - `delta.reasoning_content`/`reasoning` → Reasoning*(优先 reasoning_content);
/// - `delta.tool_calls` → 按 `index` 稳定累积,ToolInputStart/Delta + (finish 或
///   流末)ToolInputEnd + ToolCall;`arguments` 非法 JSON 回退为字符串(与正向
///   解析一致,见 [`parse_tool_arguments`]);
/// - 首帧 `id`/`model` → ResponseMetadata(与 provider 一致);
/// - usage-only 末帧(`choices:[]` + `usage`)→ 累积到 Finish.usage。
///
/// `data: [DONE]` 结束。**A7**:若整段 body 未出现任何 `choices` 数组事件(非
/// OpenAI SSE),返回 [`AiMuxError::Unsupported`],不再按行 Raw 降级成功。
fn rebuild_stream_result(rec: &Recording) -> Result<StreamResult, AiMuxError> {
    let resp = last_response(rec)?;
    let body = resp
        .body
        .as_deref()
        .ok_or_else(|| AiMuxError::InvalidArgument("mock replay: response has no body".into()))?;

    let mut parts: Vec<Result<StreamPart, AiMuxError>> = Vec::new();
    parts.push(Ok(StreamPart::StreamStart { warnings: vec![] }));

    let text_id = "0".to_string();
    let reasoning_id = "reasoning-0".to_string();
    let mut text_started = false;
    let mut reasoning_started = false;
    let mut response_meta_emitted = false;
    let mut final_usage = Usage::default();
    let mut final_finish: Option<FinishReason> = None;
    // tool_call 按 OpenAI `index` 稳定累积;`tool_order` 保插入顺序(确定性 emit)。
    let mut tool_calls: std::collections::HashMap<usize, ToolCallAccumulator> =
        std::collections::HashMap::new();
    let mut tool_order: Vec<usize> = Vec::new();
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

        // OpenAI chat.completions chunk 标志:`choices` 数组(可空,如 usage-only 末帧)。
        let choices_arr = v.get("choices").and_then(|c| c.as_array());
        if choices_arr.is_some() {
            saw_openai = true;
        }

        // 首帧 id/model → ResponseMetadata(与 openai provider 一致)。
        if !response_meta_emitted
            && (v.get("id").and_then(|x| x.as_str()).is_some()
                || v.get("model").and_then(|x| x.as_str()).is_some())
        {
            response_meta_emitted = true;
            parts.push(Ok(StreamPart::ResponseMetadata {
                id: v["id"].as_str().map(|s| s.to_string()),
                timestamp: None,
                model_id: v["model"].as_str().map(|s| s.to_string()),
            }));
        }

        // usage(含 usage-only 末帧 choices:[]+usage)→ 累积到 Finish.usage。
        // 跳过显式 null(避免把已累积的 usage 清空,与 provider 一致)。
        if v.get("usage").is_some_and(|u| !u.is_null()) {
            final_usage = parse_usage(&v)?;
        }

        if let Some(choices) = choices_arr {
            for choice in choices {
                let delta = &choice["delta"];

                // Reasoning delta(优先 reasoning_content,与 provider 一致)。
                let reasoning_delta = delta
                    .get("reasoning_content")
                    .and_then(|x| x.as_str())
                    .or_else(|| delta.get("reasoning").and_then(|x| x.as_str()));
                if let Some(reasoning) = reasoning_delta
                    && !reasoning.is_empty()
                {
                    if !reasoning_started {
                        reasoning_started = true;
                        parts.push(Ok(StreamPart::ReasoningStart {
                            id: reasoning_id.clone(),
                            provider_metadata: None,
                        }));
                    }
                    parts.push(Ok(StreamPart::ReasoningDelta {
                        id: reasoning_id.clone(),
                        delta: reasoning.to_string(),
                        provider_metadata: None,
                    }));
                }

                // Text delta:文本前结束 reasoning(与 provider 一致)。
                if let Some(content) = delta.get("content").and_then(|x| x.as_str())
                    && !content.is_empty()
                {
                    if reasoning_started {
                        parts.push(Ok(StreamPart::ReasoningEnd {
                            id: reasoning_id.clone(),
                            provider_metadata: None,
                        }));
                        reasoning_started = false;
                    }
                    if !text_started {
                        text_started = true;
                        parts.push(Ok(StreamPart::TextStart {
                            id: text_id.clone(),
                            provider_metadata: None,
                        }));
                    }
                    parts.push(Ok(StreamPart::TextDelta {
                        id: text_id.clone(),
                        delta: content.to_string(),
                        provider_metadata: None,
                    }));
                }

                // Tool-call deltas(按 index 累积):tool_calls 前结束 reasoning。
                if let Some(tcs) = delta.get("tool_calls").and_then(|t| t.as_array()) {
                    if reasoning_started {
                        parts.push(Ok(StreamPart::ReasoningEnd {
                            id: reasoning_id.clone(),
                            provider_metadata: None,
                        }));
                        reasoning_started = false;
                    }
                    for dtc in tcs {
                        let idx = dtc
                            .get("index")
                            .and_then(|x| x.as_u64())
                            .map(|n| n as usize)
                            .unwrap_or(0);
                        let func = dtc.get("function").cloned().unwrap_or_default();
                        let is_new = !tool_calls.contains_key(&idx);
                        if is_new {
                            let id = dtc
                                .get("id")
                                .and_then(|x| x.as_str())
                                .unwrap_or_default()
                                .to_string();
                            let name = func
                                .get("name")
                                .and_then(|x| x.as_str())
                                .unwrap_or_default()
                                .to_string();
                            tool_calls.insert(
                                idx,
                                ToolCallAccumulator {
                                    id: id.clone(),
                                    name: name.clone(),
                                    arguments: String::new(),
                                },
                            );
                            tool_order.push(idx);
                            parts.push(Ok(StreamPart::ToolInputStart {
                                id,
                                tool_name: name,
                                provider_executed: None,
                                dynamic: None,
                                title: None,
                                provider_metadata: None,
                            }));
                        }
                        // 参数 delta:新 tool_call 跳过空 args(与 provider 一致);
                        // 续传总是 emit(即使空)。
                        if let Some(args) = func.get("arguments").and_then(|x| x.as_str())
                            && (!is_new || !args.is_empty())
                            && let Some(acc) = tool_calls.get_mut(&idx)
                        {
                            acc.arguments.push_str(args);
                            parts.push(Ok(StreamPart::ToolInputDelta {
                                id: acc.id.clone(),
                                delta: args.to_string(),
                                provider_metadata: None,
                            }));
                        }
                    }
                }

                // finish_reason:结束 reasoning/text/tool_calls,捕获 finish。
                if let Some(fr) = choice.get("finish_reason").and_then(|x| x.as_str()) {
                    if reasoning_started {
                        parts.push(Ok(StreamPart::ReasoningEnd {
                            id: reasoning_id.clone(),
                            provider_metadata: None,
                        }));
                        reasoning_started = false;
                    }
                    if text_started {
                        parts.push(Ok(StreamPart::TextEnd {
                            id: text_id.clone(),
                            provider_metadata: None,
                        }));
                        text_started = false;
                    }
                    for &i in &tool_order {
                        if let Some(acc) = tool_calls.get(&i) {
                            parts.push(Ok(StreamPart::ToolInputEnd {
                                id: acc.id.clone(),
                                provider_metadata: None,
                            }));
                            let input = parse_tool_arguments(&acc.arguments);
                            parts.push(Ok(StreamPart::ToolCall {
                                tool_call_id: acc.id.clone(),
                                tool_name: acc.name.clone(),
                                input,
                                provider_executed: None,
                                dynamic: None,
                                thought_signature: None,
                                provider_metadata: None,
                            }));
                        }
                    }
                    tool_calls.clear();
                    tool_order.clear();
                    let (unified, raw) = parse_finish(Some(fr));
                    final_finish = Some(FinishReason { unified, raw });
                }
            }
        }
    }

    // 收尾:结束未关闭的 reasoning/text/tool_calls(未见 finish_reason 时)。
    if reasoning_started {
        parts.push(Ok(StreamPart::ReasoningEnd {
            id: reasoning_id,
            provider_metadata: None,
        }));
    }
    if text_started {
        parts.push(Ok(StreamPart::TextEnd {
            id: text_id,
            provider_metadata: None,
        }));
    }
    for &i in &tool_order {
        if let Some(acc) = tool_calls.get(&i) {
            parts.push(Ok(StreamPart::ToolInputEnd {
                id: acc.id.clone(),
                provider_metadata: None,
            }));
            let input = parse_tool_arguments(&acc.arguments);
            parts.push(Ok(StreamPart::ToolCall {
                tool_call_id: acc.id.clone(),
                tool_name: acc.name.clone(),
                input,
                provider_executed: None,
                dynamic: None,
                thought_signature: None,
                provider_metadata: None,
            }));
        }
    }

    // A7:未解析出任何 OpenAI 结构 → 非OpenAI SSE,返回 Unsupported(不降级 Raw)。
    if !saw_openai {
        return Err(AiMuxError::Unsupported(
            "mock replay: response is not an OpenAI chat.completions SSE stream".into(),
        ));
    }

    parts.push(Ok(StreamPart::Finish {
        finish_reason: final_finish.unwrap_or(FinishReason {
            unified: FinishReasonUnified::Stop,
            raw: None,
        }),
        usage: final_usage,
        provider_metadata: None,
    }));

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
    fn exact_matcher_different_headers_misses() {
        // A8:headers 纳入规范键——非脱敏头值不同 → miss。
        let mut rec = openai_recording("t1", "ping", "pong", "stop");
        let mut call = sample_options("ping", Some(0.7));
        call.headers = Some([("x-custom".into(), "a".into())].into_iter().collect());
        rec.input.options = serde_json::to_value(&call).unwrap();
        let recs = [rec];
        let matcher = ExactMatcher::new("openai", "gpt-4o");

        // x-custom 值不同 → miss。
        let mut req = sample_options("ping", Some(0.7));
        req.headers = Some([("x-custom".into(), "b".into())].into_iter().collect());
        assert!(matcher.r#match(&req, &recs).is_err());

        // 完全一致 → hit(对照)。
        let mut req2 = sample_options("ping", Some(0.7));
        req2.headers = Some([("x-custom".into(), "a".into())].into_iter().collect());
        assert!(matcher.r#match(&req2, &recs).is_ok());

        // 请求多一个头 → miss(保守:键集不一致)。
        let mut req3 = sample_options("ping", Some(0.7));
        req3.headers = Some(
            [
                ("x-custom".into(), "a".into()),
                ("x-extra".into(), "z".into()),
            ]
            .into_iter()
            .collect(),
        );
        assert!(matcher.r#match(&req3, &recs).is_err());
    }

    #[test]
    fn exact_matcher_different_provider_options_misses() {
        // A8:provider_options 纳入规范键——非脱敏值不同 → miss。
        let mut rec = openai_recording("t1", "ping", "pong", "stop");
        let mut call = sample_options("ping", Some(0.7));
        call.provider_options = Some(
            [("openai".into(), serde_json::json!({ "foo": 1 }))]
                .into_iter()
                .collect(),
        );
        rec.input.options = serde_json::to_value(&call).unwrap();
        let recs = [rec];
        let matcher = ExactMatcher::new("openai", "gpt-4o");

        // foo 值不同 → miss。
        let mut req = sample_options("ping", Some(0.7));
        req.provider_options = Some(
            [("openai".into(), serde_json::json!({ "foo": 2 }))]
                .into_iter()
                .collect(),
        );
        assert!(matcher.r#match(&req, &recs).is_err());

        // 完全一致 → hit(对照)。
        let mut req2 = sample_options("ping", Some(0.7));
        req2.provider_options = Some(
            [("openai".into(), serde_json::json!({ "foo": 1 }))]
                .into_iter()
                .collect(),
        );
        assert!(matcher.r#match(&req2, &recs).is_ok());
    }

    #[test]
    fn exact_matcher_redacted_values_match_any() {
        // A8:录制侧 headers 经 redact_json 脱敏(authorization→"[REDACTED]");
        // 脱敏值视为通配,匹配任意显式请求值。
        let mut rec = openai_recording("t1", "ping", "pong", "stop");
        let mut call = sample_options("ping", Some(0.7));
        call.headers = Some(
            [
                ("authorization".into(), "sk-secret".into()),
                ("content-type".into(), "application/json".into()),
            ]
            .into_iter()
            .collect(),
        );
        // 模拟录制侧 headers 脱敏。真实路径 `redact_json` 会递归脱敏整个 options,
        // 但 `max_output_tokens` 含 "token" 子串也会被脱敏(无关的录制侧副作用,
        // 非 A8 范围);此处仅脱敏 headers 以隔离测试 headers 的脱敏匹配语义。
        let mut opts_val = serde_json::to_value(&call).unwrap();
        if let Some(h) = opts_val.get_mut("headers") {
            *h = crate::recording::redact_json(h.clone());
        }
        rec.input.options = opts_val;
        // 录制侧 authorization 已脱敏,content-type 保留。
        assert_eq!(
            rec.input.options["headers"]["authorization"],
            serde_json::json!("[REDACTED]")
        );
        let recs = [rec];
        let matcher = ExactMatcher::new("openai", "gpt-4o");

        // 请求 authorization 用任意值 → 通配命中(content-type 一致)。
        let mut req = sample_options("ping", Some(0.7));
        req.headers = Some(
            [
                ("authorization".into(), "sk-other".into()),
                ("content-type".into(), "application/json".into()),
            ]
            .into_iter()
            .collect(),
        );
        assert!(matcher.r#match(&req, &recs).is_ok());

        // content-type(非脱敏)不同 → miss(精确比较生效)。
        let mut req2 = sample_options("ping", Some(0.7));
        req2.headers = Some(
            [
                ("authorization".into(), "sk-other".into()),
                ("content-type".into(), "text/plain".into()),
            ]
            .into_iter()
            .collect(),
        );
        assert!(matcher.r#match(&req2, &recs).is_err());

        // 请求缺 authorization(录制侧有)→ miss(保守:键集不一致)。
        let mut req3 = sample_options("ping", Some(0.7));
        req3.headers = Some(
            [("content-type".into(), "application/json".into())]
                .into_iter()
                .collect(),
        );
        assert!(matcher.r#match(&req3, &recs).is_err());

        // 请求 headers 为 None(录制侧有脱敏 headers)→ miss(保守)。
        let req4 = sample_options("ping", Some(0.7));
        assert!(matcher.r#match(&req4, &recs).is_err());
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
        assert!(
            matcher
                .r#match(&sample_options("zzzzz", None), &recs)
                .is_err()
        );
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
        let err = rt.block_on(async {
            model
                .do_generate(&sample_options("ping", None))
                .await
                .unwrap_err()
        });
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
    fn non_openai_body_returns_unsupported() {
        // A7:非 OpenAI chat.completions body(无 choices[0].message)→ Unsupported,
        // 不再把整 body 降级为文本 + warning。
        let mut rec = openai_recording("t1", "ping", "x", "stop");
        rec.exchanges[0].response.as_mut().unwrap().body = Some("{\"foo\":\"bar\"}".to_string()); // 非 chat.completions
        let model = MockReplayModel::new("openai", "gpt-4o", vec![rec]);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let err = rt.block_on(async {
            model
                .do_generate(&sample_options("ping", None))
                .await
                .unwrap_err()
        });
        assert!(matches!(err, AiMuxError::Unsupported(_)), "{err}");
    }

    #[test]
    fn non_openai_stream_returns_unsupported() {
        // A7:非 OpenAI SSE(无任何 choices 数组事件)→ Unsupported,不按行 Raw 降级。
        let mut rec = openai_recording("t1", "ping", "x", "stop");
        rec.exchanges[0].response.as_mut().unwrap().body =
            Some("event: message_start\ndata: {\"type\":\"message_start\"}\n\n".to_string());
        rec.exchanges[0].response.as_mut().unwrap().stream_chunks = Some(1);
        let model = MockReplayModel::new("openai", "gpt-4o", vec![rec]);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let err = rt.block_on(async {
            model
                .do_stream(&sample_options("ping", None))
                .await
                .unwrap_err()
        });
        assert!(matches!(err, AiMuxError::Unsupported(_)), "{err}");
    }

    #[test]
    fn mock_model_rebuilds_tool_calls_from_openai_body() {
        // C4-4:content null + tool_calls 不得返回空 content;解析 id/name/arguments。
        let mut rec = openai_recording("t1", "ping", "x", "tool_calls");
        let body = serde_json::json!({
            "id": "chatcmpl-mock",
            "model": "gpt-4o",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_abc",
                        "type": "function",
                        "function": { "name": "get_weather", "arguments": "{\"city\":\"SF\"}" }
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": { "prompt_tokens": 5, "completion_tokens": 3, "total_tokens": 8 }
        });
        rec.exchanges[0].response.as_mut().unwrap().body = Some(body.to_string());
        let model = MockReplayModel::new("openai", "gpt-4o", vec![rec]);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async {
            model
                .do_generate(&sample_options("ping", None))
                .await
                .unwrap()
        });
        assert_eq!(result.content.len(), 1);
        let Some(GenerateContent::ToolCall {
            tool_call_id,
            tool_name,
            input,
            ..
        }) = result.content.first()
        else {
            panic!("expected ToolCall, got {:?}", result.content);
        };
        assert_eq!(tool_call_id, "call_abc");
        assert_eq!(tool_name, "get_weather");
        assert_eq!(input, &serde_json::json!({ "city": "SF" }));
        assert_eq!(result.finish_reason.unified, FinishReasonUnified::ToolCalls);
    }

    #[test]
    fn mock_model_tool_call_bad_json_arguments_falls_back_to_string() {
        // C4-4:arguments 非法 JSON → 回退为字符串值(与 openai provider 正向解析一致)。
        let mut rec = openai_recording("t1", "ping", "x", "tool_calls");
        let body = serde_json::json!({
            "id": "chatcmpl-mock",
            "model": "gpt-4o",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": { "name": "f", "arguments": "not-json{" }
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        });
        rec.exchanges[0].response.as_mut().unwrap().body = Some(body.to_string());
        let model = MockReplayModel::new("openai", "gpt-4o", vec![rec]);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async {
            model
                .do_generate(&sample_options("ping", None))
                .await
                .unwrap()
        });
        let Some(GenerateContent::ToolCall { input, .. }) = result.content.first() else {
            panic!("expected ToolCall");
        };
        assert_eq!(input, &serde_json::json!("not-json{"));
    }

    #[test]
    fn mock_model_rebuilds_stream_tool_calls() {
        // C4-8:流式 tool_calls 按 index 累积 → ToolInputStart/Delta/End + ToolCall。
        let chunk1 = serde_json::json!({
            "id": "chatcmpl-1", "model": "gpt-4o",
            "choices": [{"index": 0, "delta": {"role": "assistant", "tool_calls": [
                {"index": 0, "id": "call_1", "type": "function", "function": {"name": "get_weather", "arguments": ""}}
            ]}}]
        });
        let chunk2 = serde_json::json!({
            "choices": [{"index": 0, "delta": {"tool_calls": [
                {"index": 0, "function": {"arguments": "{\"city\":\"SF\"}"}}
            ]}}]
        });
        let chunk3 = serde_json::json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}]});
        let sse = format!("data: {chunk1}\n\ndata: {chunk2}\n\ndata: {chunk3}\n\ndata: [DONE]\n\n");
        let mut rec = openai_recording("t1", "ping", "x", "stop");
        rec.exchanges[0].response.as_mut().unwrap().body = Some(sse);
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
        // 首帧 id/model → ResponseMetadata。
        assert!(parts.iter().any(|p| matches!(
            p,
            StreamPart::ResponseMetadata { id, model_id, .. }
                if id.as_deref() == Some("chatcmpl-1") && model_id.as_deref() == Some("gpt-4o")
        )));
        // ToolInputStart。
        assert!(parts.iter().any(|p| matches!(
            p,
            StreamPart::ToolInputStart { tool_name, .. } if tool_name == "get_weather"
        )));
        // ToolInputDelta(初始空 args 跳过;仅续传 emit)。
        let deltas: Vec<String> = parts
            .iter()
            .filter_map(|p| match p {
                StreamPart::ToolInputDelta { delta, .. } => Some(delta.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(deltas, vec!["{\"city\":\"SF\"}"]);
        // ToolInputEnd + ToolCall(finish_reason 触发)。
        assert!(
            parts
                .iter()
                .any(|p| matches!(p, StreamPart::ToolInputEnd { .. }))
        );
        let calls: Vec<_> = parts
            .iter()
            .filter_map(|p| match p {
                StreamPart::ToolCall {
                    tool_call_id,
                    tool_name,
                    input,
                    ..
                } => Some((tool_call_id.clone(), tool_name.clone(), input.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "call_1");
        assert_eq!(calls[0].1, "get_weather");
        assert_eq!(calls[0].2, serde_json::json!({"city":"SF"}));
        // finish_reason tool_calls。
        assert!(parts.iter().any(|p| matches!(
            p,
            StreamPart::Finish { finish_reason, .. }
                if finish_reason.unified == FinishReasonUnified::ToolCalls
        )));
    }

    #[test]
    fn mock_model_rebuilds_stream_metadata_and_usage() {
        // C4-8:首帧 id/model → ResponseMetadata;usage-only 末帧(choices:[]+usage)
        // → 累积到 Finish.usage。
        let chunk1 = serde_json::json!({
            "id": "chatcmpl-2", "model": "gpt-4o",
            "choices": [{"index": 0, "delta": {"content": "hi"}}]
        });
        let chunk2 = serde_json::json!({
            "id": "chatcmpl-2", "choices": [],
            "usage": {"prompt_tokens": 7, "completion_tokens": 2, "total_tokens": 9}
        });
        let sse = format!("data: {chunk1}\n\ndata: {chunk2}\n\ndata: [DONE]\n\n");
        let mut rec = openai_recording("t1", "ping", "x", "stop");
        rec.exchanges[0].response.as_mut().unwrap().body = Some(sse);
        rec.exchanges[0].response.as_mut().unwrap().stream_chunks = Some(3);
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
        assert!(parts.iter().any(|p| matches!(
            p,
            StreamPart::ResponseMetadata { id, model_id, .. }
                if id.as_deref() == Some("chatcmpl-2") && model_id.as_deref() == Some("gpt-4o")
        )));
        let deltas: Vec<String> = parts
            .iter()
            .filter_map(|p| match p {
                StreamPart::TextDelta { delta, .. } => Some(delta.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(deltas, vec!["hi"]);
        let finish = parts
            .iter()
            .find_map(|p| match p {
                StreamPart::Finish { usage, .. } => Some(usage.clone()),
                _ => None,
            })
            .unwrap();
        assert_eq!(finish.input_tokens.total, Some(7));
        assert_eq!(finish.output_tokens.total, Some(2));
    }

    #[test]
    fn last_response_skips_failed_attempt() {
        // C4-9:第 0 次 attempt 非 2xx(失败),第 1 次 2xx(成功)→ 选第 1 次。
        let mut rec = openai_recording("t1", "ping", "pong", "stop");
        rec.exchanges[0].response.as_mut().unwrap().status = 500;
        rec.exchanges[0].attempt = 0;
        let success_body = serde_json::json!({
            "id": "chatcmpl-ok", "model": "gpt-4o",
            "choices": [{"index":0,"message":{"role":"assistant","content":"pong"},"finish_reason":"stop"}],
            "usage": {"prompt_tokens":5,"completion_tokens":3,"total_tokens":8}
        });
        rec.exchanges.push(HttpExchange {
            attempt: 1,
            request: HttpRecord {
                method: "post".into(),
                url: "https://api.openai.com/v1/chat/completions".into(),
                headers: vec![],
                body: Some("{}".into()),
            },
            response: Some(ResponseRecord {
                status: 200,
                headers: vec![],
                body: Some(success_body.to_string()),
                stream_chunks: None,
                ttfb_ms: None,
            }),
            timing: TimingRecord {
                latency_ms: 10,
                ttfb_ms: None,
            },
            error: None,
            finalized: true,
        });
        let model = MockReplayModel::new("openai", "gpt-4o", vec![rec]);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async {
            model
                .do_generate(&sample_options("ping", None))
                .await
                .unwrap()
        });
        let Some(GenerateContent::Text { text, .. }) = result.content.first() else {
            panic!("expected text");
        };
        assert_eq!(text, "pong");
        // 证明取的是第 1 次(id=chatcmpl-ok),而非失败的 exchange[0](id=chatcmpl-mock)。
        assert_eq!(result.response.id.as_deref(), Some("chatcmpl-ok"));
    }

    #[test]
    fn last_response_no_successful_response_errors() {
        // C4-9:唯一 attempt 非 2xx → 无合法响应 → 明确错误(不降级)。
        let mut rec = openai_recording("t1", "ping", "pong", "stop");
        rec.exchanges[0].response.as_mut().unwrap().status = 500;
        let model = MockReplayModel::new("openai", "gpt-4o", vec![rec]);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let err = rt.block_on(async {
            model
                .do_generate(&sample_options("ping", None))
                .await
                .unwrap_err()
        });
        assert!(err.to_string().contains("no successful"), "{err}");
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
