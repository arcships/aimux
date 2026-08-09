# RFC-0023: 调用上下文录制与回放

> **Status**: IMPLEMENTED (P1-P6 + config_snapshot 已实施,2026-08-10;实施进度见 [docs/plan/rfc0023-recording.md](../docs/plan/rfc0023-recording.md))
> **Date**: 2026-08-05(重写:从"仅 HTTP 录制"扩展为"三层完整上下文录制 + 两种回放模式")
> **Scope**: `aimux-core` + `aimux-provider-utils` 新增可选的完整调用上下文录制(输入侧 + 配置侧 + HTTP 侧)与两种回放模式(请求回放 / mock 响应回放),opt-in,默认关闭
> **Related**: [RFC-0003](0003-test-cassette.md) 测试 cassette(格式与匹配基础)、[RFC-0014](0014-logging.md) 统一日志(零成本门控范本)、[RFC-0015](0015-cache-trace-audit.md) 缓存审计(sink 抽象兄弟)、[RFC-0020](0020-external-provider-config.md) 外部 provider 配置(配置侧录制关联)

---

## 1. Motivation

aimux 当前**完全没有运行时录制与回放能力**。一次调用失败或返回异常,用户只拿到最终 `AiMuxError` JSON,看不到:实际发出的请求、上游返回的原始响应、当时的调用参数与 provider 配置。无法离线重现问题,也无法用录制的响应做 mock 测试。

现有可观测性都不够:
- RFC-0014(tracing 日志)只记 span 元数据(provider/model/attempt/状态码),不存完整 payload。
- RFC-0003(测试 cassette)是**测试期**录制回放(wiremock `#[cfg(test)]`),aimux 自身零运行时录制能力。
- RFC-0015(缓存审计)只存哈希指纹,**刻意不存明文**,无法用于"看原始请求/响应"或回放。

### 1.1 录制要覆盖三层完整上下文

一次 `generate_text`/`stream_text` 调用的完整上下文分三层,录制必须全部覆盖,缺一层则回放无法重建调用:

| 层 | 内容 | 来源 | 回放用途 |
|---|---|---|---|
| **① 输入侧**(调用参数) | `prompt`(消息数组)+ `GenerateTextOptions`(temperature/tools/reasoning/headers/body_overrides/max_retries/timeout/seed/response_format...) | `generate_text` 入口的 `CallOptions` | 请求回放(重建调用)+ mock 匹配键 |
| **② 配置侧**(provider 身份) | provider name / model_id / base_url / api_key 来源(不存明文 key)/ profile / ProviderOptions(headers/org/project) | `model.provider()`/`model.model_id()` + provider config | 请求回放(重建 provider)+ 审计 |
| **③ HTTP 侧**(wire) | 实际发出的 request(method/url/headers/body)+ response(status/headers/body/流式每帧)+ timing/attempt | `aimux-provider-utils/src/http.rs` 咽喉点 | mock 响应回放(返回录制响应) |

### 1.2 回放分两种模式

| 模式 | 机制 | 用途 |
|---|---|---|
| **请求回放**(request replay) | 拿录制的 ①+②,重新构造 `generate_text` 调用,发**真实 API**。响应是新的(可能不同) | 离线重跑、改 prompt 重发、回归对比、A/B 测试 |
| **mock 响应回放**(mock replay) | 拿录制的 ① 作匹配键,命中则返回录制的 ③(response),**不发真实 API** | 测试、离线 debug、降本(当缓存用) |

两种模式共享同一份录制(三层都录),只是消费方式不同:
- 请求回放只用 ①+②(重建调用),③ 仅作审计参考(对比新旧响应)。
- mock 响应回放用 ①(匹配)+ ③(返回响应),② 仅作日志/过滤。

---

## 2. Design Goals

1. **三层完整录制**:输入侧 + 配置侧 + HTTP 侧全覆盖,缺一则回放无法重建。
2. **两种回放模式**:请求回放(重发真实 API)+ mock 响应回放(返回录制响应)。
3. **零成本关闭**:默认关闭,关闭时热路径 ~0(1 原子读 + 1 分支),对标 RFC-0014 门控范本。
4. **开启低开销**:录制开启 <0.1%(μs 级 vs LLM 调用 50-500ms);热路径不阻塞 I/O(异步落盘)。
5. **隐私受控**:api_key / Authorization 永不录明文(只录来源:`env:VAR` / 明文标记)。**body 默认录**——aimux 是面向开发者的 SDK,录制的目的就是 debug/回放,不录 body 等于没录;录制的 opt-in 开关本身已是隐私门控(默认整体关闭,开启即开发者明确知情)。
6. **复用 RFC-0003**:录制格式在 cassette 格式上扩展;mock 匹配复用 `score()` 逻辑。

---

## 3. Design

### 3.1 录制数据模型(三层 + 关联 ID)

```rust
// aimux-core/src/recording.rs (新增)

use serde::{Serialize, Deserialize};
use crate::options::CallOptions;
use crate::language_model_message::LanguageModelPrompt;

/// 一次完整调用的录制记录(三层 + call_id 关联)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recording {
    /// 全局唯一 ID,关联三层。
    pub call_id: String,
    pub recorded_at: String,           // ISO 8601

    /// ① 输入侧:调用参数(prompt + options)。
    pub input: InputRecord,

    /// ② 配置侧:provider 身份与配置。
    pub provider: ProviderRecord,

    /// ③ HTTP 侧:实际 wire 交换(含重试,每次 attempt 一条)。
    pub exchanges: Vec<HttpExchange>,

    /// 最终结果摘要(成功/失败 + finish_reason + usage)。
    pub outcome: OutcomeRecord,
}

/// ① 输入侧:完整调用参数,足以重建 generate_text 调用。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputRecord {
    /// 完整 prompt(消息数组,含 ContentPart::Image 等多模态)。
    pub prompt: LanguageModelPrompt,
    /// 序列化的 CallOptions(除 abort_signal 外全部字段,#[serde(skip)] 的已自动排除)。
    pub options: serde_json::Value,
}

/// ② 配置侧:provider 身份,足以重建 provider(请求回放用)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderRecord {
    pub provider: String,              // model.provider(),如 "openai"
    pub model_id: String,              // model.model_id(),如 "gpt-4o"
    pub base_url: Option<String>,      // provider 的 base_url
    /// api_key 来源(不存明文):"env:OPENAI_API_KEY" / "explicit"(明文已传) / "none"。
    pub api_key_source: String,
    pub profile: Option<serde_json::Value>,  // OpenAICompatProfile(能力差异)
    pub provider_options: Option<serde_json::Value>,  // ProviderOptions(headers/org/project/...)
}

/// ③ HTTP 侧:单次 attempt 的 wire 交换。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpExchange {
    pub attempt: u32,                  // 第几次重试(0=首次)
    pub request: HttpRecord,
    pub response: Option<ResponseRecord>,  // None = 请求失败未获响应
    pub timing: TimingRecord,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpRecord {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,  // Authorization 等敏感头脱敏
    pub body: Option<String>,            // 明文(脱敏后);None = 未开 body 录制
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseRecord {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,            // 非流式:完整 JSON;流式:原始 SSE 拼接文本
    pub stream_chunks: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimingRecord {
    pub latency_ms: u64,
    pub ttfb_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutcomeRecord {
    /// 终结状态(2026-08-06 评审修订,替代裸 success: bool):
    /// success / error / incomplete(无 Finish 的 EOF)/ cancelled(消费方提前 drop)。
    pub status: OutcomeStatus,
    pub finish_reason: Option<String>,
    pub usage: Option<serde_json::Value>,  // 序列化的 Usage
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutcomeStatus {
    Success,
    Error,
    Incomplete,  // 流式:EOF 前未见 Finish
    Cancelled,   // 流式:消费方提前 drop
}
```

**completion barrier(评审 R5,2026-08-06)**:writer 不得"outcome 到达即写"。完整 `Recording` 满足:input ✅ + outcome ✅ + 全部 exchange 已终结(流式骨架 exchange 收到 `ExchangeUpdate` 或标记 transport 结束)。满足才写行;shutdown/超时兜底输出 incomplete 记录。`ExchangeUpdate` 带 `attempt` 标识定位目标 exchange,不隐式更新"最后一条"。

### 3.2 录制点:两层,call_id 关联

录制分两层,用同一 `call_id` 关联成一条完整 `Recording`:

**层 A:`generate_text`/`stream_text` 入口(录 ①+②+outcome)**

```rust
// aimux-core/src/generate.rs 增量

pub async fn generate_text(model: &dyn LanguageModel, prompt: impl Into<ModelPrompt>, options: GenerateTextOptions) -> Result<GenerateTextResult, AiMuxError> {
    let call_id = new_call_id();
    let call_options = options.into_call_options(lm_prompt);

    // 录制 ①+②(若开):输入侧 + provider 元信息
    let recorder = recording::recorder();
    if let Some(rec) = &recorder {
        rec.record_input(&call_id, &call_options, model.provider(), model.model_id());
        // provider config(base_url/api_key_source/profile)从 model 提取——
        // 见 §3.3 config_snapshot 默认方法
    }

    let result = do_generate_with_logging(model, &call_options, span, started).await;

    // 录制 outcome
    if let Some(rec) = &recorder {
        rec.record_outcome(&call_id, &result);
    }
    // ...
}
```

**层 B:`http.rs` 咽喉点(录 ③,per-attempt)**

```rust
// aimux-provider-utils/src/http.rs 增量

// 录制点移入 send_with_retry_raw 的重试循环(评审 R3,2026-08-06):
// 每次 attempt 录一条 HttpExchange(attempt = 0..max_retries)。
// 成功响应的 body 在 send/send_stream 读完后补录;错误体经 read_error_body
// 捕获(4xx/5xx/网络错误各录一条含 error 字段的 exchange)。
// call_id 从 HttpRequest.call_id 取(见 §3.4)。
```

### 3.3 ConfigSnapshot(提取配置侧)

`LanguageModel` trait 只暴露 `provider()`/`model_id()`,不暴露 base_url/api_key_source/profile。**对齐决策 B1(2026-08-06):`config_snapshot` 作为 `LanguageModel` 的默认方法**(零破坏,非独立 trait):

```rust
// aimux-core/src/language_model.rs 增量

/// RFC-0023: 配置侧快照供录制。默认返回最小信息(provider/model_id);
/// 各 provider 覆盖以提供 base_url/profile/api_key 来源等完整配置。
fn config_snapshot(&self) -> ProviderRecord {
    ProviderRecord::minimal(self.provider(), self.model_id())
}
```

`OpenAIModel` 等覆盖(从 `self.config: OpenAIConfig` 取 base_url/profile/api_key 来源)。

**装饰器规则**(评审补充,2026-08-06):
- transparent decorator(如 `TraceLayer`)必须覆盖并**转发 inner 的快照**,否则录到的是装饰器自身的最小信息。
- provider-changing decorator(Router/MoA)本期**只录逻辑模型快照**;effective provider 若在 `do_generate` 内才选定,由后续版本补录(避免录制时机与路由时机错位)。

### 3.4 call_id 传递

层 A(generate 入口)生成 `call_id`,层 B(http.rs)需要拿到它来关联 ③。传递方式:

- **方案 A(推荐)**:`CallOptions` 加 `#[serde(skip)] call_id: Option<String>` 字段——入口生成后塞进 call_options,http.rs 从 `HttpRequest`(经 provider 透传)读取。serde skip 保证不过 JSON 边界(与 `abort_signal` 同模式,[options.rs:119](../aimux-core/src/options.rs#L119))。
- **方案 B**:thread-local / task-local(`tokio::task_local!`)——无需改 CallOptions,但隐式传递,调试性差。

**选 A**:显式、可测、与现有 `abort_signal` 同模式。`HttpRequest` 已有 `abort_signal` 字段([http.rs:197](../aimux-provider-utils/src/http.rs#L197)),加 `call_id` 同理。

**recorder 快照绑定**(评审 R7,2026-08-06):层 A 入口**读取一次** `Arc<dyn Recorder>` 并随 `call_id` 传到底层(层 B 经 `HttpRequest` 透传的应携带 recorder 引用或由入口统一注入)。**禁止各录制点分别重新读全局 recorder**——调用中途替换全局 recorder 会拆散同一条 `Recording`;全局替换只影响后续新调用。

### 3.5 Recorder trait 与实现

```rust
/// 录制器 trait。
pub trait Recorder: Send + Sync {
    /// 录制输入侧 + 配置侧(层 A 入口调用)。
    fn record_input(&self, call_id: &str, options: &CallOptions, provider: &str, model_id: &str);
    /// 录制配置侧完整快照。
    fn record_provider(&self, call_id: &str, snapshot: &ProviderRecord);
    /// 录制单次 HTTP 交换(层 B http.rs 调用,per-attempt 一条)。
    fn record_exchange(&self, call_id: &str, exchange: &HttpExchange);
    /// 流式响应结束时补全最后一条 exchange 的 response(层 B 流式路径)。
    fn record_exchange_update(&self, call_id: &str, attempt: u32, response: &ResponseRecord);
    /// 录制最终结果(层 A 入口调用)。
    fn record_outcome(&self, call_id: &str, outcome: &OutcomeRecord);
    /// 同步 flush:阻塞直到数据落盘(oneshot 回执),关闭前调用确保不丢。
    fn flush(&self);
}

/// 全局录制开关。None = 关闭(默认)。RwLock 支持运行时替换/关闭/测试隔离。
static RECORDER: RwLock<Option<Arc<dyn Recorder>>> = RwLock::new(None);

pub fn init_recording(recorder: Option<Arc<dyn Recorder>>) { *RECORDER.write() = recorder; }
pub fn init_recording_from_env() -> bool { /* AIMUX_RECORD=1 + AIMUX_RECORD_DIR;body 默认录 */ }

/// 热路径检查:关闭时 1 读锁 + None 分支(次 ns 级)。
fn recorder() -> Option<Arc<dyn Recorder>> { RECORDER.read().unwrap().clone() }
```

**默认实现**:
- `JsonlRecorder`:每条完整 `Recording` 序列化为一行 jsonl 写磁盘。**专用 writer thread + std::sync::mpsc**(对齐决策 B2,2026-08-06——同步 `flush` 与异步 tokio task 不匹配,专用 thread 可 oneshot 确认落盘、不依赖 runtime、不受 runtime shutdown 影响)。分片按 `call_id` 在 writer 内合并;**completion barrier**:outcome 与全部 exchange(含流式 update)都到齐才写完整行,不"outcome 即写"。
- `RingRecorder`(P6,对齐决策 C1/D3):内存有界 ring buffer(默认 2048 条),**与 `RingTraceStore` 同款样式各自实现**(Mutex + VecDeque + with_capacity + export_jsonl),不共用类型;内部需 `pending_by_call_id + completed VecDeque`,容量语义/单条上限/incomplete 导出规则在实施验收中明确。

**队列语义**(评审补充):channel 容量与满载策略须明确(bounded + drop-newest 并计数,或 unbounded + 上限告警);丢弃事件计数对外可查。

### 3.6 回放模式

#### 3.6.1 请求回放(request replay)

用录制的 ①+② 重新构造 `generate_text` 调用,发**真实 API**。

**拆层设计(评审 R4 + 对齐决策,2026-08-06)**:自动重建 provider 需要 `aimux-providers` 的构造能力,不能放 `aimux-core`(会形成 core→providers→core 循环依赖)。拆两层:

```rust
// ── 层 1(aimux-core/src/replay.rs):provider 无关,只恢复输入并重发 ──

/// 用录制输入重建调用,经调用方提供的 model 重发(真实 API)。
pub async fn replay_with_model(
    recording: &Recording,
    model: &dyn LanguageModel,
    overrides: Option<&ReplayOverrides>,
) -> Result<GenerateTextResult, AiMuxError> {
    // 1. prompt/options 从录制输入重建 + overrides
    // 2. generate_text(&*model, prompt, options).await
}

// ── 层 2(aimux-providers 或 CLI):按 ProviderRecord 自动构造 ──

/// 按 ProviderRecord 重建 provider(OpenAI 兼容族)。
/// `api_key_source` 为 env:VAR(读 env)/none(本地模型)时可直接重建;
/// explicit 时调用方传 api_key。返回清晰的"需要手动 model"错误作为兜底提示。
pub fn rebuild_provider(
    p: &ProviderRecord,
    api_key: Option<&str>,
) -> Result<Box<dyn LanguageModel>, AiMuxError>;
```

**能力边界**(与 RFC-0020 解耦,2026-08-06):

| api_key_source | 自动重建 | 说明 |
|---|---|---|
| `env:VAR` | ✅ | 读 env 重解析 |
| `none`(本地模型) | ✅ | 无需 key(仍可能需要 base_url/profile,按 ProviderRecord 提供) |
| `explicit` | ⚠️ 需补 key | 录制不存明文(隐私强制),调用方传 `api_key` |
| 原生协议(anthropic/google/bedrock…) | ⚠️ 需传实例 | `rebuild_provider` 仅覆盖 openai 兼容族;其余用 `replay_with_model` |

**用途**:
- 离线重跑线上流量(debug 难以重现的问题)
- 改 prompt 重发(A/B 测试不同 prompt 对同一调用的效果)
- 回归对比(新旧 aimux 版本对同一输入的输出差异)
- CI 集成(录制真实流量,定期重跑检测 provider API 变化)

**形态**:层 1 为库 API(`aimux::replay::replay_with_model`);层 2 + CLI(`aimux-replay recordings.jsonl`)放独立 crate 或 `tools/`。CLI 更适合离线场景,不污染库;建议提供 `--dry-run`(打印重建后的 provider/prompt/目标 URL,不发请求、不输出凭据)。

#### 3.6.2 mock 响应回放(mock replay)

用录制的 ① 作匹配键,命中则返回录制的 ③(response),不发真实 API。

```rust
// aimux-core/src/replay.rs

/// Mock 回放器:实现 LanguageModel trait,内部按输入匹配录制响应。
pub struct MockReplayModel {
    recordings: Vec<Recording>,        // 加载的录制
    matcher: Box<dyn ReplayMatcher>,   // 匹配策略
}

#[async_trait]
impl LanguageModel for MockReplayModel {
    fn provider(&self) -> &str { "mock-replay" }
    fn model_id(&self) -> &str { "mock-replay" }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        // 1. 用 matcher 按输入侧匹配录制
        let rec = self.matcher.match(&options.prompt, &options, &self.recordings)?;
        // 2. 从录制的 ③(response)重建 GenerateResult
        rebuild_generate_result(&rec.exchanges[0].response, &rec.provider)
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        // 同上,重建 stream(从录制的原始 SSE 文本逐帧回放)
    }
}
```

**匹配策略 `ReplayMatcher` trait**(可插拔):

```rust
pub trait ReplayMatcher: Send + Sync {
    fn r#match(&self, prompt: &LanguageModelPrompt, options: &CallOptions, recordings: &[Recording]) -> Result<&Recording, AiMuxError>;
}
```

内置实现:
- **`ExactMatcher`**:请求体完全相同才命中(body hash)。精确但命中率低。
- **`ScoreMatcher`**:按 provider/model_id 必匹配 + prompt 公共前缀消息数 + 文本 LCP 打分,零分(完全不相关)不命中,平局取首个(确定性)。与 RFC-0015 LCP 同源。
- **`PrefixMatcher`**(P5):按 prompt 前缀匹配(与 RFC-0015 LCP 同源)。适合"相同前缀的请求"命中。

**回放范围(对齐决策 R8,2026-08-06)**:MVP 仅支持 **OpenAI 兼容协议**——录制的 wire body/SSE 可直接解析为 `GenerateResult`/`StreamPart`(复用 `openai_output` 类型)。**其他 provider 明确返回 `Unsupported`**,不猜测解析。后续如需通用 mock,走"录制规范化结果"(层 A 同时存解析后的 `GenerateResult`/`Vec<StreamPart>`)或 decoder 下沉 `aimux-providers` 两条路线之一。

**与现有 wiremock cassette 的关系**:
- RFC-0003 的 wiremock 是**测试期**机制(`#[cfg(test)]` + MockServer 重定向 base_url)。
- `MockReplayModel` 是**运行时**机制(实现 `LanguageModel` trait,直接返回录制响应,无需 MockServer)。
- 两者共享格式与匹配逻辑(`score()`),但运行机制不同。`MockReplayModel` 可替代 wiremock 用于非测试场景(如本地 dev 用录制响应调试,不发真实 API)。

---

## 4. 隐私与安全

aimux 是面向开发者的 SDK,不是终端产品。录制的目的是 debug / 回放 / 测试,需要完整上下文才有意义。隐私门控在**录制开关**本身(opt-in,默认整体关闭),而非在录制内容上做默认裁剪——开发者开启录制即明确知情。

| 维度 | 设计 |
|------|------|
| **默认** | **整体关闭**。不调 `init_recording` / 不设 env,零录制、零开销。开启录制 = 开发者明确知情同意。 |
| **api_key / Authorization** | **永不录明文**。只录来源:`"env:OPENAI_API_KEY"` / `"explicit"`(明文已传但不存值)/ `"none"`。这是唯一强制脱敏项——key 泄露无 debug 价值且有滥用风险。 |
| **敏感头脱敏** | **contains 式检测**(2026-08-06 评审修订):`authorization`/`proxy-authorization`/`cookie`/`set-cookie`/`x-api-key`/`api-key` 及**含 "api-key" 的所有头**(如 Google/Vertex 的 `x-goog-api-key`)恒 `[REDACTED]`。禁止精确匹配清单(漏 `x-goog-api-key` 即明文落盘)。复用 `logging.rs::is_sensitive_key` 同规则。 |
| **输入侧脱敏** | **`CallOptions.headers` / `provider_options` / `body_overrides` 序列化进 `InputRecord.options` 前必须按同规则脱敏**(2026-08-06 评审修订)——调用方若把 Authorization/凭据放这些字段,不得明文落盘。`ProviderRecord.provider_options` 同理。 |
| **body(prompt/request/response)** | **默认录**。开发者 SDK 的录制就该录完整 wire 上下文,否则无法 debug/回放。录制的 opt-in 开关已是隐私门控。 |
| **文档警示** | env 说明标注"录制含明文 prompt,仅建议 debug/开发环境开启";生产环境如需开启,责任在开发者(与开 tracing 日志同性质)。 |
| **回放安全** | 请求回放重发真实 API 会消耗 token/费用,文档警示;mock 回放无此风险。 |

---

## 5. 性能预算

> 2026-08-06 评审修订:删除未经 benchmark 支持的绝对数值("~1ns"/"<0.01%"),改为量级描述;落地后补基准。

| 场景 | 热路径操作 | 量级 | 对照 |
|------|-----------|------|------|
| 关闭 | 1 读锁 + None 分支 | 次 ns 级 | 相对 LLM 调用可忽略 |
| 层 A 录制开启 | clone CallOptions(结构 clone)+ mpsc send | μs 级 | LLM 调用 50-500ms → 占比可忽略 |
| 层 B 每帧开启 | clone `Bytes` chunk(Arc bump)+ `Vec::push` | ns-μs | chunk 间隔 10-100ms → 可忽略 |
| 落盘 | 专用 writer thread 从 channel 取出写 jsonl | 不在热路径 | — |

**流式录制**:cassette 存原始拼接 SSE 文本(非解析帧),录制流式 = 累积原始 `Bytes` chunk,无需解析 SSE;回放原样吐回。累积需字节上限 + 截断标记(避免按 UTF-8 边界切片 panic)。

---

## 6. Relationship with RFC-0015(协调点)

RFC-0015(缓存审计,DRAFT)存哈希指纹 + usage + verdict,**刻意不存明文**。本 RFC 存完整明文上下文。两者是兄弟:

| | RFC-0015 审计 | 本 RFC 录制 |
|---|---|---|
| 层 | `LanguageModel` 装饰器 | `generate` 入口 + `http.rs` 咽喉 |
| 内容 | 哈希指纹 + usage + verdict(无明文) | 完整三层上下文(明文) |
| 默认 | 可常开(隐私安全) | 必须 opt-in |
| 用途 | 缓存命中真伪审计 | debug / 回放 / 测试 |

**协调**:共享 ring-store 模式(`RingTraceStore` / `RingRecorder`),但类型不同(`TraceRecord` vs `Recording`)。不强行统一类型,只统一模式。落地顺序:先实施的一方预留合并口子。

---

## 7. Non-Goals

1. **不做请求回放的生产热路径集成**。请求回放(重发真实 API)是离线/CLI 场景,不进 `generate_text` 运行时路径——它是消费录制数据的工具,不是录制本身。
2. **不做 mock 回放的自动缓存语义**(如 TTL 过期、容量淘汰)。`MockReplayModel` 是确定性回放,不是缓存;缓存(按请求匹配返回响应 + 过期策略)是另一特性,可后续基于 `MockReplayModel` 扩展。
3. **不做配置热重载 / 文件监听**。MVP 是启动期初始化录制,进程内固定。
4. **不录 abort_signal / 内部重试退避细节**。只录 attempt 次数 + timing。
5. **不内置 prompt 脱敏**(PII 检测)。提供 `redact_body` 开关,但不做语义级 PII 识别——那是上层职责。

---

## 8. Scope of Changes

> 2026-08-06 评审修订:ConfigSnapshot 改为默认方法;补 116 处 `HttpRequest` 透传;P4 拆层。

| 位置 | 改动 | 工作量 |
|------|------|--------|
| `aimux-core/src/recording.rs` | `Recording`/`InputRecord`/`ProviderRecord`/`HttpExchange`/`OutcomeRecord` + `Recorder` trait + `JsonlRecorder`/`RingRecorder` + `init_recording` + completion barrier | ~400 行 |
| `aimux-core/src/replay.rs` | `MockReplayModel`(OpenAI 兼容 MVP)+ `ReplayMatcher` trait + `ExactMatcher`/`ScoreMatcher` + `replay_with_model`(provider 无关的请求回放) | ~400 行 |
| `aimux-core/src/language_model.rs` | `config_snapshot()` 默认方法(B1) | ~5 行 |
| `aimux-core/src/generate.rs` | `generate_text`/`stream_text` 录制接入(层 A)+ `call_id` 生成 + 流式 outcome 包装(Finish/Error/drop 终结) | ~100 行 |
| `aimux-core/src/options.rs` | `CallOptions` 加 `#[serde(skip)] call_id: Option<String>` | ~5 行 |
| `aimux-provider-utils/src/http.rs` | `send_with_retry_raw` per-attempt 录制(层 B)+ `send_stream` 骨架 + `ObservedByteStream` 累积 + `HttpRequest` 加 `call_id` + contains 式脱敏 | ~150 行 |
| `aimux-providers/src/**` | **116 处 `HttpRequest` 构造点补 `call_id`(文本路径透传 `options.call_id`,其余 `None`)** + 各 provider 覆盖 `config_snapshot` | ~200 行 |
| `aimux-providers`(replay 自动构造) | `rebuild_provider`(按 ProviderRecord 构造,OpenAI 兼容 + env/none key;explicit 补 key) | ~100 行 |
| `aimux-ffi/src/lib.rs` | `aimux_init_recording` + `aimux_mock_replay_new` C ABI | ~40 行 |
| `bindings/node/src/lib.rs` | `initRecording` + `mockReplay(recordings)` napi | ~40 行 |
| 测试 | 录制三层正确性 + 两种回放 + 匹配策略 + 脱敏(含 `x-goog-api-key`)+ completion barrier | ~350 行 |

**合计:~1400-1600 行。无 trait 破坏性改动(`config_snapshot` 是默认方法)、入口签名不变、关闭时零影响。**

---

## 9. Risks

| 风险 | 等级 | 对策 |
|------|------|------|
| **明文 prompt 落盘**(隐私) | 中 | 录制整体 opt-in(默认关闭),开启即开发者知情;api_key/Authorization 恒脱敏(无 debug 价值且有滥用风险);body 默认录(SDK 面向开发者,不录则录制无意义);文档标注仅建议 debug/开发环境开启 |
| **请求回放消耗真实 API 费用** | 中 | 文档警示;CLI 加 `--dry-run`(只打印不重发);库 API 由调用方显式调 |
| **mock 回放匹配歧义**(多录制命中) | 中 | `ScoreMatcher` 平局取首个(确定性);`ExactMatcher` 无歧义;文档说明各 matcher 特性 |
| **流式超大响应内存膨胀** | 中 | 单流累积上限 + 截断标记;`RingRecorder` 有界兜底 |
| **call_id 传递漏接**(层 B 拿不到) | 低 | 方案 A 显式塞 CallOptions(serde skip);provider 透传 HttpRequest;测试覆盖 |
| **ConfigSnapshot 覆盖不全**(部分 provider 未实现) | 低 | 默认实现返回最小信息(降级录制,仍可用);按 provider 逐步补全 |
| **与 RFC-0015 sink 抽象重复** | 中 | 共享 ring-store 模式;先实施方预留合并口子 |

---

## 10. Open Questions

1. **录制文件组织**:单文件 jsonl(简单)vs 按 call_id 分文件(便于回放加载单条)vs 按 provider/日期分目录?MVP 建议单文件 jsonl + 滚动;回放加载时全读进内存索引。
2. **请求回放的形态**:库 API(`aimux::replay::replay_with_model`)+ CLI(`aimux-replay`)。MVP 两者都做——库 API 供程序化使用,CLI 供离线命令行(含 `--dry-run`)。CLI 可放独立 crate 或 `tools/`。
3. **mock 回放的 `MockReplayModel` 是否支持"部分 mock"**(某些请求 mock,某些透传真实 API)?MVP 不做(全 mock);后续可加 `PassthroughOnMiss` 策略。
4. **`ScoreMatcher` 的匹配键**:MVP 用 prompt 公共前缀消息数 + 文本 LCP(2026-08-06 定稿,见 §3.6.2),PrefixMatcher 作为 P5 可选增强。
5. ~~`ConfigSnapshot` 是否进 `LanguageModel` trait~~ **已定(B1,2026-08-06):`config_snapshot` 默认方法并入主 trait**。
6. **回放录制格式与 RFC-0003 cassette 的互操作**:能否用 RFC-0003 的 cassette 做 mock 回放?格式相近(cassette 是单 exchange,Recording 是三层),可写转换器。MVP 不做,后续兼容。

---

## 11. Implementation Order

| 阶段 | 内容 | 依赖 | 状态 |
|------|------|------|------|
| **P1** | `recording.rs`:`Recorder` trait + `Recording` 数据模型(含 `OutcomeStatus` + completion barrier)+ `JsonlRecorder`(专用 writer thread + oneshot flush)+ `init_recording`(RwLock)+ 层 A 录制接入(generate.rs,流式 outcome 终结)+ `call_id` 传递 + 全侧脱敏 + 单测 | 无 | ✅ 已实施(2026-08-10):completion barrier + 专用 writer thread(oneshot flush + Drop join)+ 脱敏超集 + call_id 统一 + ISO 时间戳 |
| **P2** | 层 B 录制接入(http.rs:`send_with_retry_raw` per-attempt + `send_stream` 骨架 + `ObservedByteStream` 累积)+ `config_snapshot` 默认方法 + OpenAI 等主要 provider 覆盖 + 116 处 `HttpRequest` 透传 | P1 | ✅ 已实施(2026-08-10):per-attempt exchange + 流式骨架 patch 补全(Drop 幂等)+ R7 recorder 快照(RecordingContext 透传)+ UTF-8 安全截断 + config_snapshot 覆盖(OpenAI 兼容族 + 原生族) |
| **P3** | `replay.rs`:`MockReplayModel`(OpenAI 兼容 MVP)+ `ScoreMatcher`/`ExactMatcher` + 单测(mock 响应回放) | P1 | ✅ 已实施(2026-08-10):MockReplayModel + ReplayMatcher(ExactMatcher/ScoreMatcher)+ from_jsonl + 单测 |
| **P4** | `replay.rs`:`replay_with_model`(core,provider 无关)+ `aimux-providers` 的 `rebuild_provider`(自动构造)+ CLI(`--dry-run`) | P1(**不再依赖 RFC-0020**,2026-08-06 对齐修订,见 §3.6.1) | ✅ 已实施(2026-08-10):replay_with_model + RebuildProvider(OpenAI 兼容族)+ tools/aimux-replay CLI(--dry-run/--api-key/--prompt/--call-id) |
| **P5** | 绑定层透传(Node + C ABI 本期)+ `PrefixMatcher` + 脱敏验证 + 文档 | P1-P3 | ✅ 已实施(2026-08-10):C ABI + Node 绑定透传 + PrefixMatcher + 脱敏端到端验证 |
| **P6** | `RingRecorder`(与 RingTraceStore 同款样式,2026-08-06 对齐确认纳入本期) | P1 | ✅ 已实施(2026-08-10):RingRecorder(默认 2048 + FIFO 淘汰 + dropped_count 可查 + export_jsonl/clear) |

**建议顺序**:P1(录制核心)→ P2(HTTP 层 + 配置侧)→ P3(mock 回放,最常用)→ P4(请求回放)→ P5(绑定)→ P6(RingRecorder)。P1+P2 完成即可 debug 取证;P3 完成即可离线测试;P4 完成即可回归/A/B。
