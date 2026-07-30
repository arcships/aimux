# RFC-0010：请求性能对比 — aimux vs Vercel AI SDK

> **状态**：DRAFT（待评审）
> **日期**：2026-07-30
> **范围**：设计一套可复现的请求性能基准，在统一测量基准下对比 aimux（Rust 核心 + napi Node 绑定）与 Vercel AI SDK（纯 TS）的「统一接入层」开销；覆盖**速度、结构化开销、并发能力**三个维度；产出落地步骤与公平性约束
> **关联**：[RFC-0009](0009-request-resilience.md) 请求层优化（对比前置依赖）、[RFC-0003](0003-test-cassette.md) 录播测试方案（mock 数据来源）、[RFC-0001](0001-multilang-bindings.md) 多语言绑定

## 1. 动机

aimux 在 [Cargo.toml](../Cargo.toml) 自定位为 `"Rust alternative to Vercel AI SDK"`。但「Rust 替代品」的性能主张至今**没有可复现的测量支撑**——既无 benchmark 目录，也无与 AISDK 的对比数据。本 RFC 定义一套基准，回答：

> 在同等上游、同等负载下，aimux 这一「统一接入层」相对 Vercel AI SDK 的延迟 / 结构化开销 / 并发能力差异是多少？

**对比的是什么、不是什么**：

| 维度 | 说明 |
|---|---|
| ✅ 对比 | SDK 自身的协议转换、请求构造、序列化、流式解析开销 |
| ✅ 对比 | 统一抽象层带来的额外开销（是否「收敛 172 厂商」要付出性能税） |
| ❌ 不对比 | 哪个语言更快（Rust vs JS 没有跨语言直比意义） |
| ❌ 不对比 | 上游 LLM 自身性能（用 mock 抹平） |

## 2. 核心挑战：不能跨语言直比数字

aimux 核心是 Rust，AISDK 是纯 TS。若用 Rust 的 `#[bench]`/criterion 跑 aimux、用 JS 的 mitata 跑 AISDK，两边**计时器、运行时、FFI 边界、内存模型**全不一致，数字不可比。

**唯一公平的对比姿态**：在**同一个 Node.js 进程**内、面对**同一个本地 mock server**，让 aimux 走其真实生产路径（Node 应用 → napi → Rust 核心 → reqwest → HTTP），AISDK 走（Node 应用 → TS 核心 → undici → HTTP）。两者共用 Node 事件循环和同一条测量基准。

aimux 已具备这条路径的所有前提：

| 前提 | 现状 |
|---|---|
| Node 绑定可用 | [bindings/node/](../bindings/node/) 导出 `generateText(model, prompt)` / `streamText(model, prompt)`（[bindings/node/src/index.ts](../bindings/node/src/index.ts#L94)） |
| 本地 mock 基础设施 | 100+ Rust 测试用 wiremock mock server，cassette 可回放（见 `aimux-providers/tests/`） |
| 对标对象可访问 | `reference/ai/` 含 Vercel AI SDK 完整源码 |
| 设计语义同源 | aimux `retry.rs` 注释明确 "Mirrors the TS SDK's `getRetryDelayInMs` / `retryWithExponentialBackoffRespectingRetryHeaders`"，两套 SDK 语义对齐，对比有意义 |

## 3. 对比架构

```
                  ┌─────────────────────────────────────┐
                  │   Node.js benchmark 进程            │
                  │   统一计时: mitata / tinybench      │
                  │   统一预热 + 统一 GC 控制            │
                  └────────────┬───────────────┬────────┘
              ┌────────────────┘               └─────────────────┐
              ▼                                                  ▼
   ┌──────────────────────┐                          ┌──────────────────────┐
   │ aimux (napi → Rust)   │                          │ @ai-sdk/openai (TS)  │
   │  generateText(model,  │                          │  generateText({      │
   │    prompt)            │                          │    model, prompt })  │
   │  streamText(...)      │                          │  streamText({...})   │
   └──────────┬───────────┘                          └──────────┬───────────┘
              │  reqwest + rustls-tls                            │  undici
              │  （走 napi FFI 边界）                              │  （纯 JS）
              └────────────────────┬──────────────────────────────┘
                                   ▼
                  ┌─────────────────────────────────────┐
                  │   本地 mock server                  │
                  │   固定 JSON / 可回放 SSE 响应        │
                  │   （抹平网络 RTT 与 LLM 生成时长）    │
                  └─────────────────────────────────────┘
```

**关键设计**：mock server 返回**固定响应**（非流式用固定 JSON；流式用固定分片的 SSE 录像回放）。把网络 RTT 和 LLM 生成时长变成常量后，剩下的差值就是两个 SDK 自身的协议转换 / 序列化 / 流解析开销。

为剥离 aimux 的 napi FFI 边界成本，加第三条基线：

| 基线 | 实现用途 |
|---|---|
| **B0. 纯 Node 直调 mock HTTP** | 用 `undici.request` 直接打 mock server，作为「无 SDK」基线。aimux 数字 − B0 = aimux 接入层 + FFI 开销；aimux 数字 − AISDK 数字 = aimux 相对 AISDK 的净差；AISDK − B0 = AISDK 接入层开销 |

## 4. 三个对比维度

三个维度互补：**速度**测用户感知的端到端延迟；**结构化开销**测 SDK 自身 CPU 税（剥离网络，定位可优化空间）；**并发能力**测规模化表现。三者都纳入对比，缺一不可。

### 4.1 维度一：速度（端到端延迟）

含网络往返，反映真实负载下用户感知。

| 场景 | 测什么 | 为什么有代表性 | 前置条件 |
|---|---|---|---|
| **A. 非流式单请求** | 一次 `generateText` 端到端延迟（P50/P95/P99） | 基线，最常见调用形态 | 无 |
| **B. 流式 TTFT + tokens/s** | 首 token 延迟、稳态吞吐、P99 尾延迟 | aimux 的 SSE 解析在 Rust（`aimux-stream`），AISDK 在 JS，这是 Rust 核心最能体现优势的地方 | SSE mock 回放器 |

### 4.2 维度二：结构化开销（纯 CPU）

这是「统一接入层」最有说服力的对比点：aimux 把 172 厂商收敛成统一 `LanguageModel` 接口，这个收敛要付出多少 CPU。**关键是剥离网络**——只测 SDK 自身的协议转换 / 序列化 / 解析，不掺网络往返。

**两套测法**：

1. **差值法（主）**：`SDK 总延迟 − B0 纯网络延迟 = 结构化开销（含 FFI）`。aimux 与 AISDK 各自减 B0 后相减，得净差。复用维度一的 B0 基线，无额外工作。
2. **分段计时（辅）**：在 SDK 调用路径内打点，分出「请求构造」「网络往返」「响应解析」「流式分片解析」四段，给出每段占比，定位开销热点。aimux 侧打点在 Rust 内、经 napi 透出；AISDK 侧在 TS 内打点。可选——差值法已能回答主问题，分段计时用于深挖。

**payload 规模曲线**：结构化开销应随 payload 线性增长。测 4 档，对比两者的**斜率**：

| 档位 | 请求体 | 响应体 | 目的 |
|---|---|---|---|
| 小 | 1 轮对话 | 短响应 100 token | 基线开销（FFI 边界固定成本占比） |
| 中 | 10 轮对话 | 中等响应 500 token | 常态负载 |
| 大 | 长 prompt 4K token | 长响应 2K token | 大 payload 下 Rust serde 优势能否抵消 FFI |
| 工具 | 5 个工具 schema + tool_call 响应 | 工具调用解析 | 结构化（非文本）解析路径 |

**核心问题**：Rust serde 在大 payload 下的优势（若有）能否抵消 napi FFI 边界成本？这是判断「用 Rust 重写接入层」是否值得的核心证据。小 payload 下 FFI 固定开销占比高（aimux 可能反而慢），大 payload 下 serde 优势显现（aimux 可能反超）——拐点在哪，是本维度的关键产出。

### 4.3 维度三：并发能力（规模化）

不只比单点吞吐，要看**曲线和稳定性**。

| 指标 | 测法 |
|---|---|
| 吞吐曲线 | 并发数 N=1/10/50/100/200，各跑固定时长，画 reqs/s 曲线，看拐点（吞吐不再随并发增长处） |
| 内存增长 | 每档并发记录 RSS 峰值，看是否随并发线性膨胀（泄漏/堆积信号） |
| 压力稳定性 | 高并发下错误率、超时率、P99 尾延迟是否飙升 |
| 连接复用效率 | aimux 落地 RFC-0009 前 vs 后，连接池复用对并发的提升（见 §7） |

**前置条件**：依赖 RFC-0009 落地（见 §7），否则并发结论是「已知缺陷」非「架构上限」。

## 5. 指标清单

每个维度统一采集：

- **延迟**：均值 / P50 / P95 / P99
- **吞吐**：reqs/s（非流式）、tokens/s（流式稳态）
- **TTFT**（Time To First Token，仅流式）：从调用到收到第一个 `TextDelta` 的耗时
- **内存**：进程 RSS 峰值；对比前后增量（剥离 mock server 自身占用）
- **GC 稳定性**：`node --expose-gc` + 定时 `gc()` 采样，观察 aimux napi 是否带来跨边界 GC 抖动
- **结构化开销**（维度二专属）：各段占比（请求构造 / 网络 / 响应解析 / 流式解析）；payload 四档的纯 CPU 耗时 + 回归斜率

输出格式：每次跑产出一份 JSON（`{ dimension, sdk, n, p50, p95, p99, mean, rss_peak_kb, ... }`），最终合并成对比表 + 折线图。

## 6. 公平性控制变量

| 变量 | 控制方式 |
|---|---|
| 网络 | 全部走 `127.0.0.1`，无真实网络 |
| LLM 生成时长 | mock 返回固定响应 / 固定分片 SSE，确定性回放 |
| 进程启动 | 预热（warmup ≥ 50 次）后才开始计时 |
| 计时器 | 同进程同框架（mitata），不用两套语言各自的 bench |
| 连接复用 | 默认配置对比；如 RFC-0009 未落地，须在报告标注 aimux 侧无连接池（见 §7） |
| 重试 | 双方都关闭 retry（`maxRetries: 0`），否则重试次数会污染延迟分布 |
| 并发模型 | 同用 Node 事件循环 + 相同并发原语（`p-limit` 或手写 semaphore） |

## 7. 关键依赖：aimux 请求层现状对对比的影响

[RFC-0009](0009-request-resilience.md) 已查明：aimux 当前 **45 处 `Client::new()` 无连接池共享、无 TLS 会话复用、全仓无超时、retry 是死代码**。这对对比结果有决定性影响，各维度受影响程度不同：

- **维度一·速度（非流式 A）**：受影响小（单请求无连接复用机会）。可先做。
- **维度一·速度（流式 B）**：受影响中等（建连成本摊薄在长流里）。可做，但 TTFT 会含一次 TLS 握手。
- **维度二·结构化开销**：**几乎不受影响**——它剥离网络只测 CPU，连接池与否不影响序列化耗时。可先做。
- **维度三·并发能力（C）**：**受影响致命**。aimux 每个 provider 各自建连，并发下可能每次新握手；AISDK（undici）默认有连接池复用。此时 aimux 看起来差，但**这不是 Rust 慢，是没接连接池**——是已知缺陷，非架构上限。

**两种处置**：

1. **先落地 RFC-0009 的 `shared_client()` + `PoolConfig` 再测维度三**（推荐）。否则并发对比的结论是「aimux 有个已知 bug」而非「aimux 的架构上限」。
2. 若急于出数据，可先做维度一（速度）、维度二（结构化开销），维度三标注「对比的是 RFC-0009 落地前状态」并单列。

## 8. 落地结构

建议在 `bindings/node/` 下新增 benchmark 子目录，复用已有的 node 构建链：

```
bindings/node/
├── bench/
│   ├── README.md            # 如何运行、如何复现
│   ├── mock-server.ts       # 本地 mock server（固定 JSON + SSE 回放）
│   ├── cassettes/           # 从 aimux-providers/tests/cassettes 复用的回放数据
│   ├── payloads/            # 维度二 payload 四档（小/中/大/工具）的固定数据
│   ├── bench-nonstream.ts   # 维度一·速度（非流式 A）
│   ├── bench-stream.ts      # 维度一·速度（流式 B）
│   ├── bench-struct.ts      # 维度二·结构化开销（差值法 + payload 曲线 + 分段计时）
│   ├── bench-concurrent.ts  # 维度三·并发能力（吞吐曲线 + 内存 + 稳定性）
│   └── package.json         # 依赖 mitata + @ai-sdk/openai + openai
└── ...（既有结构不动）
```

**mock server 选型**：Node 原生 `http` 或 `undici` 的 `MockAgent`。优先 `undici` MockAgent——AISDK 本就用 undici，aimux 侧 reqwest 打的是真 HTTP，两者能在同一 mock 后端上对齐。若 reqwest 无法被 undici MockAgent 拦截（不同 HTTP 栈），则回退到起一个真实的本地 `http.Server`，两边都打它。

**维度二 payload 生成**：小/中/大/工具四档优先复用 `aimux-providers/tests/cassettes/` 的真实 cassette（保证解析路径真实），不足的由 mock server 按固定模板生成。

## 9. 实现顺序

| 步骤 | 内容 | 产出 |
|---|---|---|
| 1 | 搭 mock server + B0 基线（纯 undici 直调） | 验证测量基准可用 |
| 2 | 维度一·速度：非流式 A（aimux + AISDK） | 第一份对比数据 |
| 3 | 维度二·结构化开销：差值法（A − B0）+ payload 四档曲线 | 拐点 + 斜率，回答「Rust 重写是否值得」 |
| 4 | 维度一·速度：流式 B（TTFT + tokens/s） | SSE 解析对比 |
| 5 | 评估：维度三前是否先落地 RFC-0009 | 决策点 |
| 6 | 维度三·并发能力：吞吐曲线 + 内存 + 稳定性 | 完整对比报告 |
| 7 | 汇总成对比报告（表格 + 图）放入 `docs/` | 可对外引用的性能数据 |

每步独立可运行，不阻塞后续。步骤 1-4 不依赖 RFC-0009，可立即开始。

## 10. 风险

| 风险 | 等级 | 缓解 |
|---|---|---|
| **mock server 自身成为瓶颈** | 中 | B0 基线先行，确认 mock 吞吐远高于被测 SDK；必要时用多 mock 实例 |
| **napi FFI 序列化吃掉 Rust 优势** | 中 | B0 基线剥离 FFI 成本；维度二 payload 曲线正是回答此问题——若 aimux 优势被 FFI 抹平，结论本身就是重要发现 |
| **流式 SSE 回放时序失真** | 中 | 用真实 cassette 的字节级回放，保留原始分片边界与间隔 |
| **AISDK 默认带 retry/中间件** | 低 | 显式 `maxRetries:0`、关闭无关中间件，双方对齐 |
| **并发维度受 RFC-0009 缺陷污染** | 高 | §7 已述，维度三前先决策 |
| **Node 版本/undici 版本差异** | 低 | 锁定 `engines` + 锁版本，报告标注环境 |

## 11. 不做的事

1. **不对比纯 Rust 直调 vs 纯 JS 直调**——跨语言无意义。所有对比都在 Node 进程内。
2. **不打真实 LLM**——网络与生成时长不可控，污染测量。如需真实端到端压测，另立提案。
3. **不做生产级压测平台**——本基准是可复现的微型基准，不是 wrk/k6 那类负载生成器。
4. **不对比 172 厂商全覆盖**——只取 OpenAI 协议一家做代表（原生协议），薄封装共享同一请求路径，结论可外推。
5. **不在本 RFC 落地 RFC-0009**——两者解耦，但维度三依赖 RFC-0009（§7）。

## 12. 验收

- [ ] `bindings/node/bench/` 可用 `pnpm bench` 一键运行
- [ ] 单次运行产出 JSON 结果文件 + 终端对比表
- [ ] 维度一（速度）：非流式 A + 流式 B 各一组数据
- [ ] 维度二（结构化开销）：payload 四档曲线 + 拐点/斜率结论；FFI 成本可剥离
- [ ] 维度三（并发能力）：吞吐曲线 + 内存增长 + 稳定性数据
- [ ] 报告明确标注 aimux 请求层状态（RFC-0009 前/后）
- [ ] B0 基线数据齐备
- [ ] 结论可复现：同一机器连续两次跑，P50 波动 < 5%
