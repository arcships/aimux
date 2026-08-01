# RFC-0015: 缓存命中审计与请求 Trace(provider cache-hit audit)

> **Status**: DRAFT (pending review)
> **Date**: 2026-08-01
> **Scope**: 在 aimux 统一 LLM 访问层上提供可选的请求 trace + 统计 + 缓存命中审计能力——客户端对连续 agent 调用的原始请求体做前缀对比,审计各 provider 服务端上报的 cache 命中率是否掺水,并提供诊断
> **Related**: [RFC-0014](0014-logging.md) 统一日志体系(本子系统挂载其 span 树),[RFC-0009](0009-request-resilience.md) retry/超时(重试语义影响判定规则),研究存档 [cache-tracing](../docs/internal/cache-tracing/00-research-plan.md)(10-working-document.md + rounds/ 全部调查与设计中间产物)

---

## 1. 背景与动机

### 1.1 问题

LLM 提供商普遍提供 prompt 缓存折扣(OpenAI cached_tokens / Anthropic cache_read_input_tokens / DeepSeek prompt_cache_hit_tokens),命中率直接影响成本与延迟。但服务端上报的命中数**无法从客户端验证**,且存在系统性失真:

| 失真类型 | 实锤案例(均经原文核验) |
|---|---|
| 服务端报高 | OpenAI 2025-01 计费事故(API 报 90%+ 命中,账单按全价);Anthropic TTL 1h→5m 静默回归(#46829,11.9 万次调用 JSONL 实证) |
| 网关改写 | litellm #9812 双重计费;Langfuse #12306 口径叠加 2×;new-api #6144 命中仍按全价收用户(掺水动机模型) |
| 报低/漏报 | Ollama 官方自认恒报 0;vLLM V1 `prompt_tokens_details` 恒 null(14+ 个月未修);Portkey 流式剥字段;OpenRouter 响应缓存 usage 归零 |

**现状 aimux 无法支撑审计**:`request_body` 已在非流式/流式双路径生成([D6 调查](10-working-document.md#L60-L84)),但 `stream_text` 用户面丢弃它;Anthropic 生产路径只填 `total`;`Usage.raw` 是死字段。业界观测产品(Helicone/Langfuse/Braintrust)均为被动采集,**无人做缓存命中真伪校验**(D3 调查)。

### 1.2 目标

1. 客户端可对账的**硬不变量**:命中域 ⊆ 客户端发过的历史前缀;首请求零命中;DeepSeek `hit+miss==prompt`;Anthropic 三字段和;OpenAI 门槛/量化;TTL 时序
2. 判定输出**期望命中区间**而非单点,掺水判定分置信度
3. 零隐式全局状态(库)、零明文落盘、性能预算内(0.1ms 热路径、零内存增长承诺不破坏)
4. 8 语言绑定零改动

### 1.3 非目标

- 不做服务端账单对账(需外部账单输入,仅留接口)
- 不做跨请求全量明文存储
- 不改 LanguageModel trait、不动 251 个 compat provider 实现

---

## 2. 设计总览

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
        aggregate()/session_chain()            Verdict 写回 TraceRecord
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
- **R-2 分段期望模型**(关键设计):命中只依赖前缀精确匹配,与 session 无关。**system/tools 段可跨 session 命中**;conversation 段仅同 session append-only 链可命中。同 session 连续但 0 命中 → 前缀被破坏(动态 system/历史压缩/tools 变化);报命中但无含该前缀的历史请求 → 掺水信号
- **R-3 白名单**:预热期(N=10 或 128s)、模型升级、低门槛
- **R-4 网关**:响应缓存 usage 归零合法(与 provider 缓存区分)、C 级网关降级 U、网关自填标记
- **R-5 数据完整性**:usage 缺失→U、retry 合并(取最后一次 usage)、多进程视界(shared→U)、无 tokenizer 降级、请求侧 cache_control(best-effort)

### 4.3 strict/shared 双模式

最高层开关。strict(自托管单实例/单客户端):视界=本进程全历史,无来源命中可判 W。shared(共享 API key,默认):无本地来源命中 → **UNKNOWN**(他进程可能写过更长前缀),绝不误判合法跨进程共享为掺水。

---

## 5. 数据模型与统计 API(完整定义见 [round-3-trace-data-model.md](../docs/internal/cache-tracing/rounds/round-3-trace-data-model.md))

### 5.1 TraceRecord(serde + ts_rs,FFI JSON 惯例)

请求身份(provider/model/request_id/session_id/时间戳)+ Fingerprint(块哈希链/body_hash/字节数/token_estimate)+ UsageSnapshot(7 字段平铺 + raw 透传)+ ResponseCacheHeaders + RequestCacheHints(请求侧档位,best-effort)+ Verdict + error。全 owned、Clone、Send+Sync;明文永不进 trace。

### 5.2 统计口径(防 D3 口径叠加坑)

- `reported_hit_rate` 严格 = cache_read / input_total,**不与 cache_write 相加**(回避 Langfuse 式 2× 口径 bug)
- `client_upper_bound_hit_rate` = 客户端 LCP token 上界 / input_total
- **两率并列展示,不合成单一"命中率"**
- 聚合:会话级 verdict 分布 + 漂移检测(Δ>5%·mean(U) 或 w>10% 警报)+ 漏报侧独立计数 + UNKNOWN 护栏(可判定率<70% 降级)

### 5.3 接口

```rust
pub trait TraceSink: Send + Sync { fn record(&self, rec: TraceRecord); }
pub trait CacheAuditor: Send + Sync { fn judge(&self, input: &JudgmentInput) -> Verdict; }
pub struct TraceLayer { /* Arc<dyn LanguageModel> + Arc<dyn TraceSink> + Arc<dyn CacheAuditor> */ }
// RingTraceStore::aggregate / session_chain / export_jsonl
```

---

## 6. TraceStore:TTL 窗口、内存与作用域(完整设计见 [round-3-design.md](../docs/internal/cache-tracing/rounds/round-3-design.md))

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

---

## 8. 上游 P0 改动清单(逐行方案见 [round-4-p0-integration-binding.md](../docs/internal/cache-tracing/rounds/round-4-p0-integration-binding.md))

| # | 改动 | 文件 | 破坏面 |
|---|---|---|---|
| P0-1 | `StreamTextResult` 增加 `request_body`/`response_headers`(无 serde,零破坏);FFI 用现有 `Raw{raw_value}` 变体插合成 part 透传(StreamStart 之后发射) | generate.rs、aimux-ffi/src/lib.rs | 编译零破坏;FFI 多 1 个 part(低) |
| P0-2 | Anthropic 非流式 + message_start 走 `convert_anthropic_usage` 补 cache 字段;vertex/anthropic_model.rs 同修;**⚠ 行为变更:`Usage.input_tokens.total` 语义从"末断点后"改为"三字段和"** | anthropic/stream.rs、anthropic/types.rs、vertex/anthropic_model.rs | 既有 fixture 输出不变 |
| P0-3 | `Usage.raw` 填充(13 处构造点;4 处零改 + 7 处 1 行/derive + 2 处随 P0-2) | 8 个 provider 文件 | FFI JSON additive 新键,ts_rs 不变 |
| P0-4 | 时间戳:wrapper 自己记 Instant+SystemTime,结果类型不加字段 | — | 无 |

**8 语言绑定零改动**(逐语言核验):C/Kotlin/Java/Node 透传或类型断言宽容;Go 任意单 key 宽容解析;Python `_SPRaw` 已存在;Swift `case "Raw"` 已存在;Flutter `StreamPartRaw` + Unknown 兜底。必要条件:用现有 Raw 变体、发射在 StreamStart 后、meta 结构对外 opaque。

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
6. **依赖 RFC-0014**:span 树(generate → http_request)挂载审计数据、TTFT 观测点

---

## 11. 决策记录

- 审计基准 = 规范化 request_body 字节(非语义对象;D6 证明 request_body 即 wire 字节,免写 20+ canonical serializer)
- 字节 LCP 是可证伪上界(byte-level BPE 单射);token 级为可选增强
- 双级对比:语义级 LCP(规范化 LanguageModelPrompt,诊断 UX)与字节级 LCP(准绳)分离
- per-process 默认追踪;不变量在进程作用域成立,跨进程命中判 UNKNOWN(strict/shared 双模式)
- reported 与 client_upper_bound 双指标并列,不合成单值
- 调查与设计存档:docs/internal/cache-tracing/(working doc + rounds/ + prototype/)
