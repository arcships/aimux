# Round 3 设计存档:Trace 数据模型 + 统计 API(2026-08-01)

> 输入依赖:D6(aimux 现状)、D3(usage 可信度)、D4(前缀连续性)、D5(指纹算法)、D2a/D2b/D2c(各 provider 不变量)。判定规则表由另一 agent 产出,本设计只定接口契约(第 5 节)。
> 原则:零隐式全局状态(库)、随结果一次性返回、不存明文、Send+Sync、serde + ts_rs(FFI 全 JSON 惯例)。

---

## 1. TraceRecord 数据模型(Rust 草案,字段级)

```rust
// 新增模块 aimux-core/src/trace.rs(serde + ts_rs,与现有类型同风格)

/// 单请求 trace 快照:全 owned、Clone、Send+Sync、可直接 JSONL 落盘。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TraceRecord {
    pub schema_version: u8,                      // 本设计 = 1;后续兼容演进

    // ── 请求身份 ──
    pub provider: String,                        // LanguageModel::provider()
    pub model: String,                           // LanguageModel::model_id()
    pub request_id: Option<String>,              // GenerateResult.response.id
    pub session_id: Option<String>,              // 显式注入,不自动猜
    pub conversation_id: Option<String>,         // 可选应用层分组(OTel gen_ai.conversation.id 对齐)
    pub started_at_unix_ms: i64,                 // wall clock,TTL 窗口审计必需
    pub streamed: bool,                          // 走 do_stream 则为 true

    // ── 延迟 ──
    pub duration_ms: Option<u64>,                // 总延迟(毫秒整数,FFI 友好)
    pub ttft_ms: Option<u64>,                    // 流式首 TextDelta;非流式 None

    // ── prompt 指纹(不存明文;见 D5 块哈希链)──
    pub fingerprint: Fingerprint,

    // ── usage 原始字段(从 Usage/TokenUsage 平铺复制)──
    pub usage: UsageSnapshot,

    // ── 响应头关键项(从 response_headers 挑选归一化)──
    pub response_cache: ResponseCacheHeaders,

    // ── 请求侧缓存设置(best-effort 从 CallOptions 捕获;判定层 R-1.4/R-1.8 需要)──
    pub request_cache: RequestCacheHints,

    // ── 判定结果(由判定规则层填;采集层只留槽位)──
    pub verdict: Option<Verdict>,

    // ── 失败信息 ──
    pub error: Option<String>,                   // 请求失败时填,其余为 None
}

/// 规范化 request_body 的块哈希链(不存字节本体)
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Fingerprint {
    pub algo: FingerprintAlgo,                   // Xxh3BlockChain | ByteHashOnly
    pub body_hash: u64,                          // 全量 xxh3-64(碰撞 ~2^-64;可选 128 位扩展)
    pub body_len_bytes: u64,                     // 规范字节长度
    pub block_size: u32,                         // 默认 4096
    pub block_hashes: Vec<u64>,                  // h_i = H(key, h_{i-1}, block_i);导出用 64-bit 键(JSON 精度),进程内判定用 TraceStore 128-bit 链(见 round-3-design.md §5),导出仅作证据
    pub key_id: u64,                             // 会话 HMAC 密钥 id(脱敏引用,key 本身不落盘)
    pub canonicalized: bool,                     // true=已做 NFC+噪声字段剥离(诊断用)
    pub token_estimate: Option<u32>,             // 优先 usage.input_total;缺省 bytes/4 粗估;再缺 None。
                                                 // 注意:Anthropic 的 input_tokens 仅=最后断点后,对账须用
                                                 // read+creation+input(P0 修复后 usage.input_total 已是总和)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum FingerprintAlgo { Xxh3BlockChain, ByteHashOnly }

/// Usage 平铺副本(避免 trace 层借用生命周期纠缠)
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct UsageSnapshot {
    pub input_total: Option<u32>,
    pub input_no_cache: Option<u32>,
    pub input_cache_read: Option<u32>,
    pub input_cache_write: Option<u32>,
    pub output_total: Option<u32>,
    pub output_text: Option<u32>,
    pub output_reasoning: Option<u32>,
    pub raw: Option<Value>,                      // Usage.raw 透传(上游补齐后)
}

/// 响应头中与缓存直接相关的项(整表 HashMap 不进 trace,控制体积)
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ResponseCacheHeaders {
    pub openrouter_cache_status: Option<OpenRouterCacheStatus>, // X-OpenRouter-Cache-Status
    pub openrouter_cache: Option<bool>,          // X-OpenRouter-Cache: true(LiteLLM 响应缓存同理走 extra)
    pub cache_control: Option<String>,
    pub extra: Vec<(String, String)>,            // 其余疑似相关头,上限 8 项,超出丢弃
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "kebab-case")]
pub enum OpenRouterCacheStatus { Hit, Miss, Bypass, DynaCache, Unknown }

/// 请求侧缓存设置(wrapper 从 CallOptions 捕获,best-effort;取不到即 None,判定层降级)
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RequestCacheHints {
    pub explicit_breakpoints: bool,              // Anthropic cache_control / OpenAI prompt_cache_breakpoint 是否使用
    pub ttl_requested: Option<String>,           // "5m"/"1h"(Anthropic)、"30m"(OpenAI 5.6+ prompt_cache_options.ttl)
    pub cache_key: Option<String>,               // OpenAI prompt_cache_key / xAI x-grok-conv-id 等
    pub source: Option<String>,                  // "provider_options" | "headers" | 未知
}

/// 判定结果槽位(与判定规则 agent 的契约,第 5 节)
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Verdict {
    pub kind: VerdictKind,                       // Trusted / SuspectOverclaim / SuspectUnderclaim / Unknown
    pub confidence: VerdictConfidence,           // High / Medium / Low
    pub violated: Vec<String>,                   // 违反的规则 id(来自规则表,如 "INV1")
    pub expected_cached_min: Option<u32>,        // 期望命中区间下界(规则表输出)
    pub expected_cached_max: Option<u32>,        // 上界
    pub claimed_cached: Option<u32>,             // 服务端上报 cache_read
    pub matched_request_id: Option<String>,      // 客户端 LCP 命中的历史请求 id
    pub matched_prefix_bytes: u64,
    pub notes: Vec<String>,                      // "合法 0 命中:GPT-5.6+ implicit breakpoint" 等
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum VerdictKind { Trusted, SuspectOverclaim, SuspectUnderclaim, Unknown }
// 与判定规则表 §0 的映射契约:W→SuspectOverclaim;B→SuspectOverclaim(Medium);
// OK→Trusted;U→Unknown;M→SuspectUnderclaim(Low,聚合按 A4 独立计数)

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum VerdictConfidence { High, Medium, Low }
```

**字段取舍理由**
- 时间用 `i64 ms`/`u64 ms` 而非 `Duration`:ts_rs 与 FFI JSON 无 Duration 表示,避免自定义序列化。
- `request_body` 永不进 TraceRecord,只进哈希;D3 取证要求"prompt 哈希 + token 数 + usage + TTFT + 到达时间"逐项满足。
- `block_hashes` 小请求用 `SmallVec<[u64; 16]>` 等价物(Vec 草案示义),200KB ≈ 50 块 ≈ 800B。
- `token_estimate` 双来源:优先服务端 total(审计对账口径),降级 bytes/4(纯上界)。

---

## 2. 从 aimux 现有类型取值 + 上游补齐清单

| TraceRecord 字段 | 取值来源 | 现状 |
|---|---|---|
| provider / model | `LanguageModel::provider()/model_id()`(wrapper 持有) | ✅ |
| request_id | `GenerateResult.response.id` / `Finish.provider_metadata` 无 id 则 None | ✅ |
| started_at / duration | wrapper 在 do_generate/do_stream 调用前后 `Instant` | ✅ |
| ttft | 流式:包装 stream,首个 TextDelta 到达时刻 - 开始时刻 | ✅(需包装 stream,注意借用,D6 已示警) |
| fingerprint | wrapper 对 `result.request_body` 做规范字节 + xxh3 块链 | ✅(request_body 双路径全覆盖,D6) |
| usage | `GenerateResult.usage` / `Finish.usage`(审计以 Finish 为准,D6) | ✅(除 Anthropic/raw 缺口) |
| response_cache | `GenerateResult.response_headers` / `StreamResult.response_headers` | ✅ |
| error | wrapper 捕获 `Err`/`StreamPart::Error` | ✅ |

**必须上游补齐(按优先级)**
1. **P0 `stream_text` 透传**:`aimux-core/src/generate.rs:255-257` 丢弃 `StreamResult.request_body`/`response_headers` → `StreamTextResult` 增加 `pub request_body: Option<Value>` + `pub response_headers: Option<HashMap<String,String>>`(与 `GenerateTextResult.raw` 对称);FFI `aimux_stream_text`(lib.rs:432-494)在 on_part 序列化时附带。
2. **P0 Anthropic 生产路径 cache 字段**:`anthropic/stream.rs:180-192,454-469` 只填 total → 补 cache_read/cache_write/no_cache(映射已存在,`anthropic/usage.rs:75-132`,仅测试用)。
3. **P1 `Usage.raw` 填充**:各 provider 把原始 usage JSON 放入 `Usage.raw`(死字段转活),TraceRecord 透传,供判定层复核服务端自报。
4. **P1 流式断流兜底**:断流时 Finish.usage 为 default(D6)→ TraceRecord.usage 保持 None 字段,并打 `notes:["stream-truncated"]`,判定层按"缺失"而非"0"处理。
5. **P2 可选**:`CallOptions` 未来加 `session_id`(现在用 wrapper 实例注入,第 4 节)。

**request_body 缺失兜底**:若 `request_body == None`(不应发生,见 D6 覆盖率),fingerprint 置 `algo=ByteHashOnly` + token_estimate 从 options 无法取 → None;判定层按"无指纹,跳过不变量 1"处理,不 panic。

**TTFT 实现注意**(D6 借用示警,openai/model.rs:391-395):用 `Arc<AtomicU64>` 或 wrapper 闭包捕获首 part 时间,禁止把计时器移入 generator。

---

## 3. 统计 API 面(serde + ts_rs,FFI 可用)

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

/// 装饰器(主推荐钩子,不改 trait、不动 172 个实现):
/// 用户把 Arc<dyn LanguageModel> 包一层,生成/流式双路径自动采集。
pub struct TraceLayer { /* inner: Arc<dyn LanguageModel>, sink: Arc<dyn TraceSink> */ }

impl TraceLayer {
    pub fn new(inner: Arc<dyn LanguageModel>, sink: Arc<dyn TraceSink>) -> Self;
    /// 会话级:注入 session_id + 会话密钥(指纹 HMAC 脱敏,key 只存内存)
    pub fn with_session(self, session_id: String, key: [u8; 32]) -> Self;
    /// 挂判定引擎(第 5 节契约;缺省 no-op,verdict=None,不破坏现有流程)
    pub fn with_auditor(self, auditor: Arc<dyn CacheAuditor>) -> Self;
}
impl LanguageModel for TraceLayer { /* do_generate/do_stream 委托 + 计时 + 指纹 + sink.record */ }

// ── 聚合 ──
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
    pub at_index: usize,
    pub reason: BreakReason,                     // SystemDrift|HistoryRewrite|ModelSwitch|ToolChange|Other
    pub lcp_bytes: u64,
}
```

**导出格式**:JSONL,每行一个 `TraceRecord`(含 verdict)。与 FFI 全 JSON 惯例一致,逐行可增量消费、可离线重跑全部不变量(D3 取证格式)。ts_rs `#[ts(export)]` 生成 TS 类型供 8 语言绑定复用。
**关键统计口径**(防 D3 口径叠加坑):`reported_hit_rate` 严格 = cache_read / input_total(不与 cache_write 相加,回避 langfuse/litellm 式 2× bug);`client_upper_bound` 只作上界不作等值;两率并列展示,不合成单一"命中率"。

---

## 4. 存储与生命周期

| 决策 | 内容 |
|---|---|
| 默认状态 | **零全局状态**。无 static/OnceLock;采集必须显式创建 TraceLayer 并注入 sink |
| 主采集路径 | 用户回调 sink(默认):每请求 `record()` 一次,库内不累积 → 2000 请求零内存增长承诺不受影响 |
| 两阶段记录 | wrapper 在调用开始插入占位(scope+t_send),结果返回后回填指纹/usage 并判定;占位不参与 lookup(排除自身);流式在 do_stream 返回后即可回填链(修正 G5) |
| 内置存储 | `RingTraceStore` 有界环形缓冲(默认 10k 条,可配);只存 `Arc<TraceRecord>`(≈500B-1KB/条,10k ≈ 10-20MB 封顶);满则淘汰最旧 |
| 会话链 | 按 session_id 索引 ring 内记录;append-only 天然有序;ring 淘汰旧记录时链自动截断 |
| 持久化 | 库内不做隐式落盘;用户回调负责(写 JSONL/推 OTel);审计会话由用户控制开关 |
| 并发 | 全程 `Send + Sync`;record 可跨线程调用;sink 内锁为短临界区(O(1) 均摊追加) |
| 脱敏 | 指纹 HMAC 会话密钥(每会话随机 key),明文请求体/完整响应体永不进 trace |
| 失败路径 | 请求 Err 或流中断也 record(带 error 字段),保证序列连续 |

---

## 5. 与判定规则表的接口契约(统计层需要什么输入)

统计层只负责**证据采集与聚合**,判定计算放 `CacheAuditor` trait(规则 agent 产出实现),保持解耦:

```rust
/// 判定引擎契约:消费证据,产出 Verdict
pub trait CacheAuditor: Send + Sync {
    fn judge(&self, input: &JudgmentInput) -> Verdict;
}

/// 统计层向判定层提供的输入(契约,字段保证非 None 的都有硬来源)
pub struct JudgmentInput<'a> {
    pub record: &'a TraceRecord,                 // 全量证据(第 1 节)
    pub claimed_cached: Option<u32>,             // = record.usage.input_cache_read
    pub claimed_write: Option<u32>,
    pub lcp: Option<LcpResult>,                  // 客户端块级 LCP(只对同 provider+model)
    pub history_len: usize,                      // 同 provider+model 更早请求数
    pub rules_meta: &'a CacheRulesMeta,          // 规则 agent 提供的表(见下)
}

pub struct LcpResult {
    pub matched_request_id: Option<String>,
    pub lcp_bytes: u64, pub lcp_blocks: u32,
    pub token_lcp_upper: Option<u32>,            // tiktoken 可选扩展(D5)
    pub adjacent_to_prev: bool,                  // 是否紧邻前一请求(D4 连续调用特征)
}

/// 规则 agent 必须提供的表(统计层不硬编码 provider 参数)
pub struct CacheRulesMeta {
    pub invariants: Vec<InvariantDef>,           // id + 描述 + provider 适用范围(如 INV1 命中⊆LCP)
    pub provider_limits: Vec<ProviderLimit>,     // 最低缓存门槛 / TTL 档位 / 粒度(OpenAI 1024、Anthropic 5m/1h、DeepSeek 64-token…)
    pub legit_zero_hit_reasons: Vec<String>,     // "GPT-5.6+ implicit breakpoint" 等(D1 列表)
}
```

**契约要点**
1. 判定输入 = TraceRecord 全量 + LCP 结果 + 规则元数据;统计层保证 fingerprint/usage/timestamps/response_cache 四类字段在正常路径下非 None。
2. 判定输出 = `Verdict`,写回 `TraceRecord.verdict`;统计层只聚合 `verdict_counts`,不做二次解释。
3. 缺数据降级:任意必需输入缺失 → 判定层返回 `VerdictKind::Unknown`,统计层照常计入,不丢记录。
4. 规则 agent 的 TTL/门槛表在 `TraceLayer` 构造时注入(`with_auditor` 携带 `CacheRulesMeta`),后续 provider 参数变更不改数据模型。

---

## 6. 性能预算表(每请求)

| 项 | 预算 | 说明 |
|---|---|---|
| 指纹计算 CPU | ≤ 50μs(200KB body)/ ≤ 5μs(2KB) | xxh3 数十 GB/s;request_body 已 preserve_order 确定性序列化,零规范化成本(NFC 默认关) |
| 块链 LCP CPU | O(δ + B) 均摊;200KB ≈ 50 块比对 | 相对 3-10s LLM 请求可忽略(D6) |
| 堆分配次数 | ≤ 2/请求(默认) | TraceRecord 1 次 + block_hashes 1 次;小 body 用 SmallVec 后为 0 |
| 常驻内存 | ~500B-1KB/条,仅 ring 内 | 默认 10k 条 ≈ 10-20MB 封顶;不随请求数增长 |
| 明文存储 | 0 | 只存哈希 + 头关键项;明文永不落 trace |
| 延迟注入 | 非流式 ≈ 0;流式 +1 次 Instant + 首 part 包装 | 对总延迟/TTFT 测量本身零额外成本 |
| 全局状态 | 0 | 无隐式全局;用户不建 TraceLayer 则完全无开销 |
| 2000 请求内存增长 | 0(默认回调路径) | ring 仅在有界容量内累积,回调路径零累积 |

**风险与不确定性**
- Anthropic cache_read 上报精确度 [UNVERIFIED];块哈希链 + 客户端对账为原创组合(D5 标注)。
- 流式 TTFT 的"首 part"含 StreamStart(无内容)事件,需明确以首 TextDelta 计(设计已固定)。
- 判定层未就位前 verdict=None:不影响采集与统计,接口已预留,无破坏性变更。
