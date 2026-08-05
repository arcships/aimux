# RFC-0024: 调用会话聚合(session_id 归组)

> **Status**: IMPLEMENTED (P1/P2/P5 — 2026-08-05;P3/P4 待依赖 RFC-0023/0015,见 [§10](#10-implementation-order))
> **Date**: 2026-08-05
> **Scope**: `aimux-core` 新增 `session_id` 字段(显式为主 + 隐式推断兜底),把连续调用聚合成会话组,为录制(RFC-0023)、缓存探测(RFC-0015)、回放提供链级视图。不做 fork 语义、不做 agent loop。
> **Related**: [RFC-0023](0023-runtime-request-recording.md) 录制(Recording 加 session_id)、[RFC-0015](0015-cache-trace-audit.md) 缓存探测(链级聚合)、[RFC-0019](0019-session-affinity.md) 会话亲和(路由层 session,与本 RFC 可观测层 session 共享 id 概念但职责不同)、[RFC-0016](0016-align-with-aisdk.md) H4(不做 agent loop 边界)

---

## 1. Motivation

aimux 当前的录制(RFC-0023)、缓存探测(RFC-0015)、回放都是**孤立请求视角**——每条 `Recording`/`TraceRecord` 是一次 `generate_text`,看不到连续调用间的关系。

但真实使用是**连续调用链**:agent loop 是一串 `generate_text`,可能同 session 连续,也可能 fork 式分叉(业务侧不同 session)。开发者真正关心的是**这条链的缓存表现怎么演变**——单请求的"命中率 80%"没意义,有意义的是"这个 session 8 步调用,命中率从 0% 涨到 80% 然后稳定"。脱离链谈单请求缓存,看不到东西。

### 1.1 为什么 aimux 做这个(边界)

- **aimux 不做 agent loop**(RFC-0005 / H4 已定)。loop 的执行(工具调用、多轮编排)是上层框架的活。
- **但 aimux 要提供会话归组基础设施**:上层 agent 框架调 8 次 `generate_text`,aimux 得能把这 8 次归到一条会话,报告这条会话的缓存表现。这是**可观测性基础设施**,与 tracing/recording 同类,属访问层职责。
- **类比**:aimux 不做 HTTP 服务,但提供 retry/timeout 基础设施让上层用。会话归组同理——aimux 提供 session_id 聚合与报告,上层 agent loop 消费。
- **fork 不是 aimux 的建模对象**:fork 是业务侧场景(上层框架决定怎么分叉),aimux 这层只需要 `session_id` 做归组。同一个 session_id 的调用归一组;不同 session_id 各自归组。fork 与否 aimux 不关心。

### 1.2 与 RFC-0019(会话亲和)的区别

两者共享 `session_id` 概念,但职责正交:

| | RFC-0019 会话亲和(路由层) | 本 RFC 会话聚合(可观测层) |
|---|---|---|
| 目的 | 传 session 头让**上游**做粘性路由 | 归组调用让**本地**报告链级表现 |
| 载体 | `CallOptions.headers`(`x-session-id` 等,各厂商头名不同) | `CallOptions.session_id`(新字段,统一) |
| 代码改动 | 零(文档方案,复用现有 headers) | 加字段 + 聚合逻辑 |
| 消费者 | 上游网关/provider | 录制/缓存探测/回放 |

**协同**:上层传同一个 `session_id`,aimux 既用它做本地归组(本 RFC),又可选地映射成厂商头喂上游(RFC-0019)。两者可共用一个 id 值,但走不同字段、不同路径。本 RFC 不强制与 0019 绑定——开发者可只用归组不喂上游,或只喂上游不归组。

---

## 2. Design Goals

1. **显式为主**:上层 agent 框架显式传 `session_id`,aimux 不生成、不默认注入、不猜测。
2. **隐式推断兜底**:未传 `session_id` 时,aimux 按 prompt 前缀延续推断归组(连续调用的 prompt 是前缀扩展 → 同一会话)。
3. **轻量**:只加一个字段 + 聚合逻辑,不做链结构/fork/状态机。
4. **三者共用**:录制/缓存探测/回放共享 session_id 归组,不各自零散加字段。
5. **opt-in 归组**:不传 session_id 且推断关闭时,退化为孤立请求视图(现状),零影响。

---

## 3. Design

### 3.1 session_id 字段

```rust
// aimux-core/src/options.rs 增量

pub struct CallOptions {
    // ... 现有字段 ...

    /// 会话标识,用于把连续调用聚合成组(可观测用,见 RFC-0024)。
    /// 显式传入优先;None 时可由隐式推断填充(若开启)。
    /// 与 RFC-0019 的会话亲和头正交:本字段供本地归组,headers 里的
    /// session 头供上游路由,两者可共用同一 id 值但走不同路径。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}
```

`GenerateTextOptions`(用户面)同步加 `session_id: Option<String>`。`#[serde(skip_serializing_if)]` 保证未传时不进 JSON,不影响现有 wire 格式。

**为什么不用 `headers` 复用**(RFC-0019 路径):
- headers 的 session 头是**厂商专属**的(OpenRouter `x-session-id`、Cloudflare `x-session-affinity`、Anthropic `x-claude-code-session-id`...),头名各不同,aimux 无法从中统一提取"会话 id"做归组。
- 可观测归组需要**统一字段**,与厂商头名解耦。两者可共用同一 id 值(开发者传 `session_id="abc"`,aimux 归组用 `abc`,同时可选映射成厂商头喂上游——但映射属 RFC-0019 的 opt-in,本 RFC 不做)。

### 3.2 隐式推断(兜底)

未传 `session_id` 时,aimux 按 prompt 前缀延续推断:

```rust
// aimux-core/src/session.rs (新增)

/// 会话推断器:按 prompt 前缀延续把连续调用归组。
/// opt-in,默认关闭(退化为孤立请求视图)。
pub struct SessionInferer {
    /// 最近 N 个调用的 (prompt_hash_prefix, session_id) 缓存。
    /// 用于检测"新调用的 prompt 是某个近期调用的前缀扩展"。
    recent: LruCache<String, String>,  // prompt_prefix_hash → session_id
}

impl SessionInferer {
    /// 推断或生成 session_id。
    /// - 显式传入:直接用。
    /// - 未传 + 推断开:若新 prompt 的前缀匹配某个近期调用,归入同一 session;
    ///   否则生成新 session_id(新会话)。
    /// - 未传 + 推断关:None(孤立请求)。
    pub fn resolve(&self, explicit: Option<&str>, prompt: &LanguageModelPrompt) -> Option<String> {
        if let Some(id) = explicit { return Some(id.to_string()); }
        if !self.enabled { return None; }
        // 计算 prompt 的前缀哈希(复用 RFC-0015 的块哈希链 LCP 算法)
        let prefix = prompt_prefix_hash(prompt);
        if let Some(id) = self.recent.get(&prefix) {
            return Some(id.clone());
        }
        let new_id = format!("auto-{}", new_uuid());
        self.recent.put(prefix, new_id.clone());
        Some(new_id)
    }
}
```

**推断的歧义与边界**:
- 歧义:两条独立链碰巧前缀相似会误并。缓解:只对**强前缀延续**(新 prompt 完全包含某个近期调用的完整 prompt 作为前缀)归并,弱相似不并。
- 推断结果标记来源:`"auto-xxx"`(推断)vs 用户传入的 id(显式),便于审计区分。
- **默认关闭推断**——显式为主。推断需 `AIMUX_SESSION_INFER=1` 或编程式开启。

### 3.3 归组数据模型

```rust
// aimux-core/src/session.rs

/// 一个会话的聚合视图(按 session_id 归组的调用序列)。
/// 不存调用内容本身——内容在 Recording(0023)/TraceRecord(0015)。
/// 本结构只是索引:session_id → [trace_id 列表]。
pub struct SessionView {
    pub session_id: String,
    pub source: SessionSource,           // Explicit / Inferred
    pub calls: Vec<SessionCall>,         // 有序调用
}

pub struct SessionCall {
    pub trace_id: String,                // 关联 Recording / TraceRecord
    pub step: u32,                       // 第几步(0 起)
    pub recorded_at: String,
}

pub enum SessionSource { Explicit, Inferred }
```

**存储**:session 索引存在 `SessionStore`(内存 LRU,默认 N 个 session),与 `RingRecorder`/`RingTraceStore` 同级。不持久化——进程重启丢索引,但 Recording/TraceRecord 的 session_id 字段保留,可重建索引。

### 3.4 三者如何消费 session_id

| 子系统 | 消费方式 |
|---|---|
| **录制(RFC-0023)** | `Recording` 加 `session_id` + `step` 字段。回放支持"按 session 重放整条链"(按序重发真实 API,或按链前缀做 mock)。查询"这个 session 的所有录制" |
| **缓存探测(RFC-0015)** | LCP 从单请求扩展到 session scope——"这条链 N 步的命中演变:0%→80%→80%→..."。`TraceRecord` 加 session_id,`SessionStore` 聚合 verdict |
| **回放(RFC-0023)** | 请求回放支持重放整条 session(按序);mock 回放按 session 内的前缀匹配提升命中率 |

**关键**:session_id 是三者共享的归组键,但**各自消费**——录制存 session_id 到 Recording,缓存探测存到 TraceRecord,回放按 session_id 过滤。不强行统一三者的存储,只统一归组字段。

---

## 4. Integration Approach

### 4.1 generate_text 入口

```rust
// aimux-core/src/generate.rs 增量

pub async fn generate_text(model: &dyn LanguageModel, prompt: impl Into<ModelPrompt>, options: GenerateTextOptions) -> Result<GenerateTextResult, AiMuxError> {
    let call_options = options.into_call_options(lm_prompt);

    // 解析 session_id(显式优先,隐式推断兜底)
    let session_id = SESSION_INFERER.resolve(call_options.session_id.as_deref(), &call_options.prompt);
    let call_options = call_options.with_session_id(session_id.clone());

    // 录制/缓存探测/会话索引都拿到 session_id
    if let Some(sid) = &session_id {
        SESSION_STORE.append(sid, &trace_id);  // 归组
    }
    // ... generate_text 主体 ...
}
```

### 4.2 绑定层

`session_id` 是 `Option<String>`,经 JSON 边界自动透传(与 `max_retries`/`temperature` 同模式)。Node/Python/FFI 用户在 options 里加 `sessionId` 字段即可,无需新 API。

### 4.3 SessionStore 查询 API

```rust
/// 查询一个 session 的所有调用(按 step 排序)。
pub fn session_calls(session_id: &str) -> Vec<SessionCall>;

/// 查询一个 session 的缓存命中演变(聚合 TraceRecord verdict)。
/// 返回 [(step, hit_rate, verdict), ...]
pub fn session_cache_trajectory(session_id: &str) -> Vec<(u32, f64, Verdict)>;

/// 列出所有已知 session(分页)。
pub fn list_sessions() -> Vec<SessionView>;
```

这些 API 供调试工具/绑定层查询,不在 `generate_text` 热路径。

---

## 5. Relationship with Existing RFCs

| RFC | 关系 |
|-----|------|
| [RFC-0023](0023-runtime-request-recording.md) | **Recording 加 session_id + step**。回放支持按 session 重放整条链。本 RFC 是录制的链级视图基础。 |
| [RFC-0015](0015-cache-trace-audit.md) | **TraceRecord 加 session_id**。LCP 从单请求扩展到 session scope。本 RFC 让缓存探测能报告链级命中演变。 |
| [RFC-0019](0019-session-affinity.md) | **共享 session_id 概念,职责正交**。0019 是路由层(头喂上游),本 RFC 是可观测层(本地归组)。可共用 id 值,走不同字段。本 RFC 不强制与 0019 绑定。 |
| [RFC-0016](0016-align-with-aisdk.md) | **边界确认**。H4 不做 agent loop;本 RFC 只做归组,不做 loop 执行。 |
| [RFC-0005](0005-protocol-conversion.md) | **正交**。会话归组不碰协议转换。 |

---

## 6. Non-Goals

1. **不做 fork 语义**。fork 是业务侧场景(上层框架决定怎么分叉),aimux 只按 session_id 归组。同 session_id 归一组,不同 session_id 各自归组,fork 与否 aimux 不建模。
2. **不做 agent loop**(H4)。session 归组是可观测基础设施,不是 loop 执行。
3. **不做 session 头自动注入**(RFC-0019 的活)。本 RFC 的 session_id 是本地归组用,不自动映射成厂商头喂上游。映射属 0019 的 opt-in,由集成方决定。
4. **不做链结构 / 状态机**。只做 `session_id → [trace_id]` 的扁平归组,不建树、不做父子链。
5. **不持久化 session 索引**。进程内 LRU,重启丢索引(但 Recording/TraceRecord 的 session_id 字段保留,可重建)。
6. **不默认开启隐式推断**。显式为主;推断需 opt-in,因有歧义风险。

---

## 7. Scope of Changes

| 位置 | 改动 | 工作量 |
|------|------|--------|
| `aimux-core/src/options.rs` | `CallOptions` + `GenerateTextOptions` 加 `session_id: Option<String>` | ~10 行 |
| `aimux-core/src/session.rs` | `SessionInferer` + `SessionStore` + `SessionView` + 查询 API | ~250 行 |
| `aimux-core/src/generate.rs` | 入口 resolve session_id + append SessionStore | ~30 行 |
| `aimux-core/src/lib.rs` | `pub mod session;` + prelude | ~5 行 |
| `aimux-ffi/src/lib.rs` | `aimux_session_calls` / `aimux_session_cache_trajectory` 查询 C ABI | ~40 行 |
| `bindings/node/src/lib.rs` | `sessionCalls` / `sessionCacheTrajectory` napi 查询函数 | ~40 行 |
| 测试 | 归组正确性 + 推断歧义边界 + 查询 API + 与录制/探测的集成 | ~200 行 |

**合计:~500-600 行。无 trait 改动、无破坏性变更(字段 Option + skip_serializing_if)、未传时零影响。**

---

## 8. Risks

| 风险 | 等级 | 对策 |
|------|------|------|
| **隐式推断误并**(独立链前缀相似) | 中 | 默认关闭推断;开启时只对强前缀延续归并;推断结果标记 `"auto-"` 前缀可区分 |
| **SessionStore 内存膨胀**(session 多) | 低 | LRU 有界(默认 N 个 session × M calls/session);可配 |
| **session_id 与 RFC-0019 头不同步**(开发者传 id 但忘传头) | 低 | 文档说明两者关系;可选提供"自动映射 session_id→厂商头"的 opt-in helper(属 0019 扩展,本 RFC 不做) |
| **推断的 prompt 前缀哈希与 RFC-0015 LCP 重复实现** | 低 | 共享块哈希链 LCP 算法模块(三者共用) |

---

## 9. Open Questions

1. **隐式推断的"强前缀"阈值**:新 prompt 完全包含某个近期调用的完整 prompt 作为前缀才算归并?还是允许部分匹配?建议 MVP 用强前缀(完整包含),部分匹配留后续。
2. **SessionStore 的 LRU 容量**:默认多少 session × 多少 calls?建议 256 session × 64 calls(≈16K 条索引,内存可忽略)。
3. **session_id 是否自动透传给上游**(映射成厂商头):本 RFC 不做(属 RFC-0019),但是否提供 `session_id → headers` 的 helper 函数让集成方方便用?建议作为 0019 的可选增强,不在本 RFC。
4. **查询 API 的形态**:库 API(Rust 函数)+ 绑定透传?还是独立调试 CLI?建议库 API 为主,CLI 后续。
5. **与 RFC-0021/0022(composite model)的交互**:RouterModel/MoaModel 的 `do_generate` 收到 session_id,应透传给子模型(保持同一 session)。需在 composite 实现里确认 session_id 透传。

---

## 10. Implementation Order

| 阶段 | 内容 | 依赖 | 状态 |
|------|------|------|------|
| **P1** | `CallOptions`/`GenerateTextOptions` 加 `session_id` 字段 + `SessionStore`(显式归组,无推断)+ 查询 API + 单测 | 无 | ✅ 已实施(2026-08-05) |
| **P2** | `SessionInferer`(隐式推断,opt-in)+ 单测(强前缀归并、歧义边界) | P1 | ✅ 已实施(2026-08-05) |
| **P3** | 录制(RFC-0023)集成:Recording 加 session_id + step + 按 session 查询/回放 | P1, RFC-0023 | ⏳ 待 RFC-0023 实施 |
| **P4** | 缓存探测(RFC-0015)集成:TraceRecord 加 session_id + session 级命中演变聚合(`session_cache_trajectory` 随此引入) | P1, RFC-0015 | ⏳ 待 RFC-0015 实施 |
| **P5** | 绑定层透传 + 查询 API(Node/C ABI/Python)+ 文档 | P1 | ✅ 已实施(2026-08-05) |

**建议先做 P1**(显式归组即可用:开发者传 session_id,aimux 归组报告)。P2 推断是增强。P3/P4 是与录制/探测的集成,各自独立。

### 10.1 实施说明(2026-08-05)

- **范围**:P1 + P2 + P5 已落地;P3/P4 的集成对象(RFC-0023 录制、RFC-0015 探测)尚未实施,`session_cache_trajectory` 依赖 RFC-0015 的 `Verdict` 类型,随 P4 引入,本里程碑不提供占位 API。
- **模块**:`aimux-core/src/session.rs`(`SessionStore` LRU 256×64、`SessionInferer` 强前缀消息级延续、全局注册显式 opt-in、查询 API `session_calls`/`list_sessions`)。全局注册沿用 RFC-0023 `init_recording` 的 `OnceLock` 模式,未注册时零状态、零影响;`AIMUX_SESSION_INFER=1` 惰性开启推断(参照 RFC-0014 env 自动初始化)。
- **热路径**:`generate_text`/`stream_text` 入口在调用前 resolve + append(失败调用也归组);未注册 store 时仅一次 RwLock 读。
- **会话来源**:`SessionSource::{Explicit,Inferred}`;推断 id 带 `auto-` 前缀可审计区分。
- **trace_id 说明**:`SessionCall.trace_id` 现为进程内生成的调用级唯一 id;RFC-0015/0023 的 `trace_id` 机制落地后,该字段承载其值。
- **绑定层**:字段经 JSON 边界自动透传(Node/Python/FFI);新增查询入口 C ABI(`aimux_session_store_init`/`aimux_session_infer_init`/`aimux_session_calls`/`aimux_list_sessions`)、Node(napi + TS typed wrapper `getSessionCalls`/`getSessions`)、Python(pyfunction + `session_calls`/`list_sessions` 包装)。Go 契约测试覆盖 `session_id` wire 格式。
- **下游影响**:无破坏性变更(字段 `Option` + `skip_serializing_if`);`LanguageModelPromptMessage`/`ContentPart` 增加 `PartialEq` derive(推断前缀比较所需,零语义变化)。
