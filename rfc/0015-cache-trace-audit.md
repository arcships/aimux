# RFC-0015: 缓存命中探测(cache-hit probe)

> **Status**: IMPLEMENTED (2026-08-05 — 探测本身进 core:TraceLayer + 判定引擎 + RingTraceStore + 查询 API + FFI/Node/Python/Go 透传;探测业务归 CLI(RFC-0025),告警外部消费)
> **Date**: 2026-08-01(初稿)/ 2026-08-05(重写)
> **Scope**: aimux 统一 LLM 访问层上的可选缓存命中探测能力——采集 request_body 指纹 + usage 快照,客户端对连续 agent 调用的原始请求体做前缀对比,判定各 provider 服务端上报的 cache 命中率是否掺水,暴露 verdict 数据与查询接口供消费。**探测本身进 core,探测业务(CLI)独立,告警外部消费。**
> **Related**: [RFC-0014](0014-logging.md) 统一日志(挂载其 span 树)、[RFC-0023](0023-runtime-request-recording.md) 录制与回放(明文兄弟子系统,trace_id 关联)、[RFC-0024](0024-session-aggregation.md) 会话聚合(session_id 统一来源)、[RFC-0009](0009-request-resilience.md) retry/超时(重试语义影响判定规则)、研究存档 [cache-tracing](../docs/internal/cache-tracing/00-research-plan.md)

---

## 1. 背景与动机

### 1.1 问题

LLM 提供商普遍提供 prompt 缓存折扣(OpenAI cached_tokens / Anthropic cache_read_input_tokens / DeepSeek prompt_cache_hit_tokens),命中率直接影响成本与延迟。但服务端上报的命中数**无法从客户端验证**,且存在系统性失真:

| 失真类型 | 实锤案例(均经原文核验) |
|---|---|
| 服务端报高 | OpenAI 2025-01 计费事故(API 报 90%+ 命中,账单按全价);Anthropic TTL 1h→5m 静默回归(#46829,11.9 万次调用 JSONL 实证) |
| 网关改写 | litellm #9812 双重计费;Langfuse #12306 口径叠加 2×;new-api #6144 命中仍按全价收用户(掺水动机模型) |
| 报低/漏报 | Ollama 官方自认恒报 0;vLLM V1 `prompt_tokens_details` 恒 null(14+ 个月未修);Portkey 流式剥字段;OpenRouter 响应缓存 usage 归零 |

**现状 aimux 无法支撑探测**:`request_body` 已在非流式/流式双路径生成([D6 调查](10-working-document.md#L60-L84)),但 `stream_text` 用户面丢弃它;Anthropic 生产路径只填 `total`;`Usage.raw` 是死字段。业界观测产品(Helicone/Langfuse/Braintrust)均为被动采集,**无人做缓存命中真伪校验**(D3 调查)。

### 1.2 三层结构(2026-08-05 对齐)

缓存探测按"探测本身 / 探测业务 / 告警"三层拆分:

```
① 探测本身(core infrastructure,常开)
   TraceLayer 装饰器 → 采集指纹+usage → LCP+不变量 → verdict → TraceRecord → RingTraceStore
   │ 只负责:采数据、算判定、存哈希、暴露查询接口
   │ 不负责:告警阈值、报表、调优建议
   ▼
② 探测业务 client(tools/aimux-cli,独立产物,基于 aimux 构建)
   一个可运行的 client:审计指定 provider 缓存能力、诊断链级命中、调试 prompt 结构
   消费 ① 的查询接口(进程内)或 ① 导出的 jsonl(离线)
   │ 独立二进制,不进 SDK 运行时路径
   ▼
③ 告警/校准/跟踪(外部消费)
   core 只暴露 verdict + TraceRecord + 查询 API
   告警业务(阈值/通知/报表)由外部应用做,aimux 不直接做
```

| 层 | 位置 | 状态 |
|---|---|---|
| ① 探测本身 | `aimux-core`(本 RFC) | 本 RFC 设计 |
| ② 探测业务 client | `tools/aimux-cli`(独立 crate,本 repo) | 见 [RFC-0025](0025-aimux-cli-cache-probe.md),不展开在本 RFC |
| ③ 告警/校准/跟踪 | 外部应用 | 本 RFC 只定义暴露的接口,不做业务 |

### 1.3 目标

1. 客户端可对账的**硬不变量**:命中域 ⊆ 客户端发过的历史前缀;首请求零命中;DeepSeek `hit+miss==prompt`;Anthropic 三字段和;OpenAI 门槛/量化;TTL 时序
2. 判定输出**期望命中区间**而非单点,掺水判定分置信度
3. 零隐式全局状态(库)、探测层零明文落盘、性能预算内(0.1ms 热路径、零内存增长承诺不破坏)
4. 暴露**数据与查询接口**,不内置业务逻辑(告警/报表由②③消费)
5. 8 语言绑定零改动(探测数据经 FFI JSON 透传)

### 1.4 非目标

- 不做服务端账单对账(需外部账单输入,仅留接口)
- 不做跨请求全量明文存储(明文录制见 [RFC-0023](0023-runtime-request-recording.md),探测层只存哈希)
- 不改 LanguageModel trait、不动 251 个 compat provider 实现
- **不做告警业务**(阈值/通知/报表):由外部消费,core 只暴露 verdict 数据
- **不做调优建议**(如自动改 prompt 提升命中):那是应用层逻辑
- **不做预判决策**(调用前预测命中并改路由):决策归 [RFC-0021](0021-composite-model-routing.md),探测只提供历史统计数据

---

## 2. 探测本身设计总览

```
                    ┌──────────────────────────────────────────┐
                    │             TraceLayer (装饰器)            │
   Arc<dyn LanguageModel> ──► do_generate / do_stream ──► 委托  │
                    │  1) 调用开始:占位记录(scope+t_send)        │
                    │  2) 结果返回:块哈希链指纹 + usage 回填      │
                    │  3) CacheAuditor::judge(判定)             │
                    │  4) sink.record(TraceRecord)             │
                    └──────────────┬───────────────────────────┘
                                   │
                ┌──────────────────┼──────────────────┐
                ▼                  ▼                  ▼
        RingTraceStore      TraceSink(用户回调)     CacheAuditor
      (有界环形+反向索引)      (JSONL/OTel/自定义)    (规则引擎,strict/shared)
                │                                        │
                ▼                                        ▼
        aggregate()/session_chain()/export_jsonl  Verdict 写回 TraceRecord
                │
                ▼
        ② CLI / ③ 外部消费(查询接口或 jsonl)
```

**核心不变量(字节级单射性论证)**:byte-level BPE 编码是单射的(enc(a)=enc(b) ⟹ a=b),故**字节相同 ⟺ token 序列相同**——服务端命中必然是客户端某历史请求前缀的字节匹配。因此客户端字节级 LCP 是服务端匹配字节长度的**必要条件**:`claimed_cached > 客户端 LCP` 直接证伪多报(唯一例外:跨进程/跨客户端共享缓存,见 strict/shared 模式)。

---

## 3. 核心算法:块哈希链增量 LCP

- **基准字节**:`request_body` 即最终发给服务的 JSON(preserve_order 确定性序列化,[D6 实证](10-working-document.md#L60));审计基准 = 去噪原始字节(剔除随机 request_id/timestamp/nonce;NFC 仅用于语义级诊断,服务端 BPE 吃原始字节)
- **指纹**:规范字节流按 4KB 块切分,`h_i = H(key, h_{i-1}, block_i)`(vLLM hash_block_tokens 式父链);xxh3-128;128-bit 碰撞率 ~2⁻¹²⁸
- **增量 LCP**:新请求从头比对块哈希,首个失配块停止;均摊 O(δ + B)(δ=增量字节,B=块大小),无关请求最坏 O(L) 不可避免
- **token 级扩展**(可选):tiktoken-rs(cl100k/o200k/deepseek_v3 等),每轮只编码增量 O(δ);无 tokenizer 时字节代理 `min(LCPb, bytes/4)`
- **脱敏**:每会话随机 HMAC 密钥哈希,明文永不落盘;64-bit 索引键 + 128-bit 链验证;跨租户哈希不可比(scope 盐)

性能实测(原型,占位哈希;换 xxh3 快 ~30×):200KB body 链计算 release 288µs、lookup 0.8µs、全管线 1.7ms、每记录 ~880B。

---

## 4. 判定规则摘要(完整规则表见 [round-3-verdict-rules.md](../docs/internal/cache-tracing/rounds/round-3-verdict-rules.md))

### 4.1 期望命中区间

```
U = min(prompt, quantize_down(token_upper(LCP_i), gran))
```
- `LCP_i` = 块粒度共享下界的字节;U 按块上界 `(j+1)·B` 计算(j=共享块数),±1 块容差由 τ 承担
- `token_upper`:有 tokenizer 用 token 级 LCP_tok;无 tokenizer 用字节代理,此时 **W 封顶为 B(中)**
- gran:OpenAI<5.6=128 / **Azure 全系含 5.6+=128(与 OpenAI 原生 5.6+ 字节精确不同源,矩阵实证)** / DeepSeek·Mistral=64 / vLLM=16 / Anthropic=断点前段+20-block 回看(定性)/ OpenAI 5.6+ 原生=无量化
- 判定:`[0,U]→OK`;`(U, U×(1+τ)]→B`;`>U×(1+τ)→W`;绝对上限 `claimed>prompt→W(高)`
- 判定类映射:W→SuspectOverclaim / B→SuspectOverclaim(Medium) / OK→Trusted / U→Unknown / M→SuspectUnderclaim(Low,聚合独立计数)

### 4.2 规则组(8 硬不变量 + 3 诊断组)

- **R-1 硬不变量**:前缀包含域(R-1.1,strict=W/shared=U)、首请求零命中(R-1.2)、DeepSeek 等式(R-1.3)、Anthropic 三字段(R-1.4)、Bedrock quota(R-1.5)、OpenAI 门槛/量化(R-1.6)、5.6+ 写读等式(R-1.7)、TTL 时序(R-1.8)
- **R-2 分段期望模型**(关键设计):判定层面命中只依赖前缀精确匹配,与 session 无关(命中是字节级事实);**聚合层面**按 session_id 归组报告链级演变([RFC-0024](0024-session-aggregation.md))。system/tools 段可跨 session 命中;conversation 段仅同 session append-only 链可命中。同 session 连续但 0 命中 → 前缀被破坏(动态 system/历史压缩/tools 变化);报命中但无含该前缀的历史请求 → 掺水信号
- **R-3 白名单**:预热期(N=10 或 128s)、模型升级、低门槛
- **R-4 网关**:响应缓存 usage 归零合法(与 provider 缓存区分)、C 级网关降级 U、网关自填标记
- **R-5 数据完整性**:usage 缺失→U、retry 合并(取最后一次 usage)、多进程视界(shared→U)、无 tokenizer 降级、请求侧 cache_control(best-effort)

### 4.3 strict/shared 双模式

最高层开关。strict(自托管单实例/单客户端):视界=本进程全历史,无来源命中可判 W。shared(共享 API key,默认):无本地来源命中 → **UNKNOWN**(他进程可能写过更长前缀),绝不误判合法跨进程共享为掺水。

---

## 5. 数据模型与统计 API

### 5.1 TraceRecord(serde + ts_rs,FFI JSON 惯例)

请求身份(provider/model/request_id/**session_id(来自 RFC-0024 CallOptions 字段)**/时间戳)+ Fingerprint(块哈希链/body_hash/字节数/token_estimate)+ UsageSnapshot(7 字段平铺 + raw 透传)+ ResponseCacheHeaders + RequestCacheHints(请求侧档位,best-effort)+ Verdict + error。全 owned、Clone、Send+Sync;明文永不进 trace。

**session_id 来源(2026-08-05 对齐)**:`TraceRecord.session_id` 统一来自 [RFC-0024](0024-session-aggregation.md) 的 `CallOptions.session_id`(显式为主 + 隐式推断兜底)。**不再用 `TraceLayer::with_session` 实例注入**——同一 wrapper 无法跟随 agent loop 中途换 session。`with_session` 降级为"默认 session_id"(CallOptions 未传时 fallback),并保持会话密钥注入(HMAC 脱敏 key)。

### 5.2 统计口径(防 D3 口径叠加坑)

- `reported_hit_rate` 严格 = cache_read / input_total,**不与 cache_write 相加**(回避 Langfuse 式 2× 口径 bug)
- `client_upper_bound_hit_rate` = 客户端 LCP token 上界 / input_total
- **两率并列展示,不合成单一"命中率"**
- 聚合统计:verdict 分布、reported vs client_upper_bound 双率、TTFT 分位、错误数(见 `TraceStats`)

> **注(2026-08-05)**:漂移检测警报(Δ>5%·mean(U) 或 w>10%)、漏报侧独立计数、UNKNOWN 护栏降级等**审计业务逻辑**从本 RFC 移出,归 ② CLI / ③ 外部消费。core 只提供原始统计(`TraceStats`)与 verdict 分布,业务判断由消费方做。

### 5.3 接口(探测本身暴露)

```rust
/// 采集入口:用户回调 sink。库零全局状态,一切显式注入。
pub trait TraceSink: Send + Sync {
    fn record(&self, rec: TraceRecord);          // 每请求一次,线程安全
    fn flush(&self) {}                            // 可选(持久化 sink 用)
}

/// 内置 sink:有界环形缓冲 + 会话链索引(Send+Sync,内部可变性)
pub struct RingTraceStore { /* Mutex<VecDeque<Arc<TraceRecord>>> + per-session 索引 */ }

impl RingTraceStore {
    pub fn with_capacity(n: usize) -> Self;      // 默认 10_000
    pub fn aggregate(&self, f: &TraceFilter) -> Vec<TraceStats>;
    pub fn session_chain(&self, session_id: &str) -> Option<SessionChainView>;
    pub fn export_jsonl(&self, w: &mut impl std::io::Write) -> std::io::Result<()>;
    pub fn clear(&self);
}

/// 装饰器(主推荐钩子,不改 trait、不动 provider 实现):
/// 用户把 Arc<dyn LanguageModel> 包一层,生成/流式双路径自动采集。
pub struct TraceLayer { /* inner: Arc<dyn LanguageModel>, sink: Arc<dyn TraceSink> */ }

impl TraceLayer {
    pub fn new(inner: Arc<dyn LanguageModel>, sink: Arc<dyn TraceSink>) -> Self;
    /// 默认 session_id(CallOptions.session_id 未传时 fallback)+ 会话密钥(指纹 HMAC 脱敏)
    pub fn with_default_session(self, session_id: String, key: [u8; 32]) -> Self;
    /// 挂判定引擎(缺省 no-op,verdict=None,不破坏现有流程)
    pub fn with_auditor(self, auditor: Arc<dyn CacheAuditor>) -> Self;
}
impl LanguageModel for TraceLayer { /* do_generate/do_stream 委托 + 计时 + 指纹 + sink.record */ }
```

**判定引擎 trait**(可插拔,内置规则引擎,消费方可替换):

```rust
pub trait CacheAuditor: Send + Sync {
    fn judge(&self, input: &JudgmentInput) -> Verdict;
}
```

**查询接口**(供 ② CLI / ③ 外部消费):

```rust
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TraceFilter {
    pub provider: Option<String>, pub model: Option<String>,
    pub session_id: Option<String>, pub since_unix_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TraceStats {
    pub provider: String, pub model: String,
    pub requests: u64,
    pub input_tokens_total: u64,
    pub claimed_cache_read_total: u64,           // Σ 服务端上报
    pub claimed_cache_write_total: u64,
    pub reported_hit_rate: Option<f64>,          // cache_read / input_total
    pub client_upper_bound_hit_rate: Option<f64>,// 客户端 LCP token 上界 / input_total
    pub verdict_counts: BTreeMap<VerdictKind, u64>,  // 掺水判定分布
    pub ttft_p50_ms: Option<u64>, pub ttft_p95_ms: Option<u64>,
    pub errors: u64,
}

// ── 会话级序列分析(append-only 链 + 前缀稳定性)──
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SessionChainView {
    pub session_id: String,
    pub record_ids: Vec<String>,                 // append-only 顺序
    pub prefix_stability: f64,                   // 相邻请求字节 LCP / 前请求长度的均值(0..=1)
    pub breaks: Vec<PrefixBreak>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PrefixBreak {
    pub at_record_id: String,
    pub prev_record_id: String,
    pub lcp_bytes: u64,                          // 断点处与上一请求的共享字节
    pub expected_break: bool,                    // 用户提示(如显式改 system)
    pub kind: BreakKind,                         // SystemChanged / ToolsChanged / ConversationReset / Unknown
}
```

**JSONL 导出格式**:`export_jsonl` 每行一个 `TraceRecord`(serde JSON)。供 ② CLI 离线分析(读文件跑审计/诊断),与 [RFC-0023](0023-runtime-request-recording.md) 的录制 jsonl 并存(不同文件,内容不同:探测=哈希,录制=明文,trace_id 可互查)。

---

## 6. TraceStore:TTL 窗口、内存与作用域

- **TTL_idle 语义修正**:provider 缓存是**空闲淘汰**而非绝对 TTL(前缀持续使用就常驻)。判定:`now − last_touch(P) ≤ TTL_idle` 才可作为上界;**TTL_idle 宁可高估**(低估→误报,高估→少判,安全方向)
- TTL 表:OpenAI 60min(5.6+ 显式 30m;extended 24h)/ Anthropic 5m-1h / Gemini 24h / DeepSeek 48h / 自托管 ∞
- **内存**:FIFO 环形缓冲硬上界(默认 2048 条全局 + 512/scope ≈ 6-7MB)+ 惰性 TTL 过期 + (slot,gen) 世代懒失效;明文 0
- **作用域**:`ScopeKey = H(base_url ‖ fp(api_key) ‖ model ‖ session_pin?)`;多租户天然隔离(vLLM cache_salt 同理);OpenRouter sticky routing 的 pin 并入 scope
- **并发**:无锁算链(90% 时间)+ 16 片 RwLock 短临界区;同 scope 串行、异 scope 并行;1000 req/s 锁占用 ≈5%
- **两阶段记录**:调用开始插占位(scope+t_send,不参与 lookup 排除自身),结果返回后回填指纹+usage 并判定;流式在 do_stream 返回后即可回填链
- **共享后端**:`TraceStorage` trait(query_and_append / export_window),默认不注册零依赖;跨进程候选仅作参考上界

---

## 7. Provider 缓存审计矩阵摘要(完整 19 行见 [round-3-provider-cache-audit-matrix.md](../docs/internal/cache-tracing/rounds/round-3-provider-cache-audit-matrix.md))

| 等级 | Provider | 要点 |
|---|---|---|
| **A 直接审计** | OpenAI Chat/Responses、Anthropic、Anthropic-Bedrock、DeepSeek、Cohere、Mistral、xAI、OpenRouter(透传) | 字段齐全;对账不变量明确(如 Mistral 64-token 整除、DeepSeek 等式、Anthropic 三字段) |
| **B 有坑** | Azure(128-token 异于 OpenAI 5.6+ 字节精确、不报写侧)、Gemini/Vertex、Bedrock Converse、LiteLLM、Portkey(流式剥)、SGLang(#25972 双重计数)、vLLM V0 | 按部署形态降级或放宽量化校验 |
| **C 报不出→降级** | vLLM V1(恒 null)、Ollama(恒 0)、llama.cpp、TRT-LLM、one-api、simple-one-api | 客户端 LCP + prefix_cache_hits 指标 + TTFT 旁证,不信 usage |

补查结论(2026-07-08 官方文档):Azure gpt-5.6 保持 128-token 粒度且不报 cache_write;Groq 自动缓存 2h 无使用过期(仅 GPT-OSS 3 模型);Qwen 显式+implicit 双模互斥(≥256 token,20-block 回看);Kimi/Moonshot 自动缓存(>256 token,interleaved thinking 丢命中)。

**矩阵的消费方**:矩阵数据(provider 能力档位)是探测本身的**判定参数**(gran/量化/降级规则),进 core;矩阵的"哪些 provider 值得探测、怎么展示结果"是 ② CLI 的展示逻辑,不进 core。

---

## 8. 与录制/回放的协同(2026-08-05 新增)

| 协同点 | 机制 | 归属 |
|---|---|---|
| **离线明文深查** | 探测在线只存哈希(常开安全);深查可疑 verdict 时用 trace_id 拉 [RFC-0023](0023-runtime-request-recording.md) 的明文录制跑完整 LCP 定位"哪段前缀掺水" | 探测暴露 trace_id,RFC-0023 存明文,trace_id 互查 |
| **缓存可复现性验证** | 对同一录制多次[请求回放](0023-runtime-request-recording.md)(重发真实 API),跑探测算法看 verdict 是否稳定(验证 provider 缓存稳定性/抖动) | ② CLI 组合两者 |
| **共享 ring-store 模式** | 探测的 `RingTraceStore` 与录制的 `RingRecorder` 共用 FIFO 环形 + TTL 过期 + export 模式,类型不同(`TraceRecord` vs `Recording`);先实施方定义抽象,后实施方复用 | core 内协调 |
| **共享 LCP/前缀算法** | 探测的块哈希链 LCP 与回放的 `PrefixMatcher`(RFC-0023)共用"前缀最长公共匹配"算法模块 | core 内协调 |
| **session_id 统一** | 探测/录制/回放共用 [RFC-0024](0024-session-aggregation.md) 的 `CallOptions.session_id` 归组 | RFC-0024 |

**告警闭环(③外部消费,远期)**:探测 verdict → 外部应用告警"provider 掺水" → 回放策略切换(RFC-0021 RouterModel 对该 provider 优先用客户端 mock 回放)。这是三者的终极闭环,但属外部应用逻辑,本 RFC 只保证数据可消费。

---

## 9. 原理论证(原型 [prototype/](../docs/internal/cache-tracing/prototype/),12/12 测试绿)

| # | 场景 | 期望 → 实际 |
|---|---|---|
| 1 | append-only agent loop,claimed=前缀 | OK → Trusted ✅ |
| 2 | 虚报 claimed=前缀×1.1 | W → SuspectOverclaim(High) ✅ |
| 3 | 首请求 claimed>0 | W ✅ |
| 4 | 5.6+ implicit 断点(大 LCP,claimed=0) | 不误报 ✅ |
| 5 | DeepSeek 等式违反/成立 | W / OK ✅ |
| 6 | 超 TTL 仍 claimed>0 / 间隔内 | W / OK ✅ |
| 7 | <1024 门槛有命中 | W ✅ |
| 8 | 跨 session 并行(system 段/越段/整段) | OK / W / OK ✅ |

---

## 10. 已知限制与后续工作

1. **未原型化**:Anthropic 三字段/20-block(R-1.4)、Bedrock quota(R-1.5)、响应缓存头(R-4.1)、网关剥除(R-4.2)、聚合 A1-A5、并发分片锁——需实现阶段补测试
2. **[UNVERIFIED] 外部项**:Anthropic cache_read 上报精确度;Gemini API 侧 TTL;OpenAI 5.6+ ttl"最短保留"上界语义;vLLM V1 null 是否已修复
3. **真实 provider 数据回放验证**:Azure 128 量化 vs OpenAI 5.6+ 字节精确双轨分派、字节代理偏差——依赖真实 API 响应回放
4. **原型同步**:块上界 (j+1)B 与字节代理 W 封顶 B 的规则定案(F2/F3)需同步进原型 verdict 逻辑
5. **FFI meta 体积**:200KB request_body 序列化翻倍,建议加 `meta_cap_bytes` 配置
6. **依赖 RFC-0014**:span 树(generate → http_request)挂载探测数据、TTFT 观测点
7. **依赖 RFC-0024**:`CallOptions.session_id` 字段落地后,探测的 session 归组改用该字段(替代 wrapper 实例注入)
8. **集群/路由部署(2026-08-05)**:生产模型多为集群部署,节点本地 KV 缓存不共享,路由变化即缓存失效,报 0 命中是常态。R-2.2 低命中预警在 `route_affinity_known=false`(默认)时抑制为备注;DeepSeek 磁盘全局共享缓存跨节点一致(实测表现最好)。详见 [round-5 §3.5](docs/internal/cache-tracing/rounds/round-5-source-verification.md)

---

## 11. 决策记录

- 审计基准 = 规范化 request_body 字节(非语义对象;D6 证明 request_body 即 wire 字节,免写 20+ canonical serializer)
- 字节 LCP 是可证伪上界(byte-level BPE 单射);token 级为可选增强
- 双级对比:语义级 LCP(规范化 LanguageModelPrompt,诊断 UX)与字节级 LCP(准绳)分离
- per-process 默认追踪;不变量在进程作用域成立,跨进程命中判 UNKNOWN(strict/shared 双模式)
- reported 与 client_upper_bound 双指标并列,不合成单值
- **2026-08-05:三层拆分**——探测本身进 core(常开 infrastructure),探测业务进 `tools/aimux-cli`(独立 client,基于 aimux 构建),告警外部消费。RFC-0015 收窄为"探测本身"。
- **2026-08-05:session_id 统一**——探测的 session 归组改用 RFC-0024 的 `CallOptions.session_id`,`with_session` 降级为默认值 + 会话密钥注入。
- **2026-08-05:审计业务逻辑移出**——漂移检测警报、漏报侧独立计数、UNKNOWN 护栏降级归 ② CLI / ③ 外部;core 只暴露 `TraceStats` + verdict 分布。
- 调查与设计存档:docs/internal/cache-tracing/(working doc + rounds/ + prototype/)
