# RFC-0014: 统一日志体系（tracing 接通）

> **Status**: IMPLEMENTED (2026-08-03 阶段1+2+3 完成:logging.rs + http 埋点 + generate span + 流观测 + C ABI + 测试全绿)
> **Date**: 2026-08-01
> **Scope**: 把已声明但零调用的 `tracing` 依赖真正接通——在 HTTP 咽喉点、顶层 generate 入口、FFI 层建立 span 树与事件，提供零成本（关闭时）可配置（env / 编程 / C ABI）的诊断日志
> **Related**: [RFC-0009](0009-request-resilience.md) request resilience（retry/timeout 的日志埋点依赖本 RFC），[cache-tracing](../docs/internal/cache-tracing/00-research-plan.md) 缓存命中审计（结构化子系统，建立在本 RFC 的 span 树之上）

---

## 1. 背景与动机

### 1.1 现状：日志从未接通

| 证据 | 含义 |
|------|------|
| `tracing = "0.1"` 写在 workspace deps，并加进 `aimux-core`/`aimux-providers`/`aimux-provider-utils` | 依赖已就位 |
| 全仓 `tracing::` / `info!` / `warn!` / `#[instrument]` 命中 **0 处** | 依赖是死代码，从未被调用 |
| 全仓无 `tracing-subscriber` | 即使写了宏，也没有 subscriber 输出，等于黑屏 |
| 全仓无 `println!`/`eprintln!`/`dbg!` | 现状是「完全静默」，不是「乱打 print」 |

结论：aimux 目前**完全没有可观测性**。一个请求失败，用户只能拿到最终的 `AiMuxError` JSON，看不到：是否重试过、重试了几次、退避多久、HTTP 状态码、原始响应体、流式在哪一帧断了。

### 1.2 Debug 痛点（来自 RFC-0009 的实测）

| # | 痛点 | 证据 |
|---|------|------|
| 1 | 429/5xx retry 全程静默 | `send_with_retry_raw` 循环里没有任何观测点，用户不知道「第 2 次重试，850ms 后重试，原因是 RateLimited」 |
| 2 | provider 错误经 `parse_provider_error` 映射后丢原始体 | 4xx 分支 `return Err(parse_provider_error(...))` 之前，原始 `body`/`status`/`headers` 无留存 |
| 3 | 流式无可见性 | `send_stream` 建连后直接返回字节流，首字节时间、事件计数、断流位置全不可见 |
| 4 | 跨 FFI 边界错误是黑盒 | Swift/Kotlin 拿到的是 `AiMuxError` 序列化 JSON，没有任何诊断上下文（span、重试历史） |

### 1.3 为什么现在做

- RFC-0009 刚落地 retry/timeout/pool，但**retry 没有日志就无法验证它在生产是否生效**——这正是「日志体系支撑 debug」的最直接理由。
- `cache-tracing` 研究正在设计结构化请求审计子系统，它需要一个已存在的 span 树（`generate` → `http_request`）来挂载审计数据。本 RFC 是它的前置依赖。

---

## 2. 设计目标与非目标

### 2.1 目标

1. **接通 `tracing`**：在关键路径建立 span 树，输出人类可读的诊断日志（stderr）。
2. **零成本关闭**：默认关闭时，每个事件开销 = 一次原子读 + 分支（`tracing` 的 max-level 过滤）。LLM 调用是网络绑定（数十 ms+），tracing 开销（纳秒级）是噪声。
3. **可配置**：env 变量 + 编程 API + C ABI 三种入口，覆盖纯 Rust 消费者、原生绑定（Python/Node）、FFI 绑定（Swift/Kotlin/C）。
4. **隐私安全**：API key、header 值、用户请求体绝不默认落日志。
5. **集中埋点**：在 HTTP 咽喉点统一埋点，而非 172 个 provider 各打各的。

### 2.2 非目标

- ❌ 不做结构化请求体审计（那是 `cache-tracing` 的职责）。
- ❌ 不做分布式追踪（OpenTelemetry exporter）——v1 只输出本地 stderr。记为未来工作。
- ❌ 不做向宿主语言 logger 的回调转发（Python `logging`/Node `console`）——v1 用 Rust `fmt` subscriber 写 stderr，宿主重定向 stderr 即可。记为未来工作。
- ❌ 不实现计费/成本追踪。

---

## 3. 核心设计决策

### 3.1 用 `tracing`，不用 `log`

项目已选 `tracing`（async 友好，span 携带任务上下文）。坚持此选择，补齐 `tracing-subscriber`。**核心价值**：span 树能还原「一次 `generate_text` → 哪个 provider → 哪次 HTTP → 重试了几次」的调用链，而 `log` 只有扁平行。

### 3.2 订阅者初始化位置（本 RFC 最关键决策）

aimux 是**库**，不是二进制。但被两种方式消费：

| 消费方式 | 消费者 | 谁初始化 subscriber |
|---------|--------|---------------------|
| 纯 Rust 依赖 | Rust 应用 | 消费者自己 `tracing_subscriber::fmt().init()`（惯用法，无冲突） |
| 原生绑定 | Python(pyo3) / Node(napi) / Flutter | 绑定的 Rust 侧暴露 `init_logging()`，宿主语言调用 |
| C ABI (FFI) | Swift / Kotlin / C | aimux-ffi 暴露 `aimux_init_logging()` |

**决策：库本身永不自动初始化**（避免与消费者自己的 subscriber 冲突）。提供两个显式入口：

- **A. env 变量自动初始化**（仅当无人初始化时）：`AIMUX_LOG` 存在且全局未注册 subscriber 时，惰性注册一个 `fmt` subscriber。这给 FFI/绑定用户「设个环境变量就有日志」的零摩擦体验，同时不干扰已自建 subscriber 的 Rust 用户。
- **B. 编程 API**：`aimux_provider_utils::logging::init_logging(level: &str)`，被 `aimux-providers` re-export，也被 `aimux-ffi` 的 `aimux_init_logging` 调用。`Once` 守护，幂等。

### 3.3 埋点位置：HTTP 咽喉点集中化

所有 172 provider 汇流到 `aimux-provider-utils/src/http.rs` 的三个函数：

```
send()            → send_with_retry_raw()  → send_request()   (非流式)
send_stream()     → send_with_retry_raw()  → send_request()   (流式)
```

**只在这层埋点**，不在每个 provider 重复。这保证：
- 172 provider 自动获得一致的日志，无需逐个改造
- retry/timeout 逻辑（RFC-0009）的观测点与实现同处一文件，不会漂移

### 3.4 性能预算

| 场景 | 开销 | 说明 |
|------|------|------|
| 默认关闭（`LevelFilter::OFF`） | 每事件 1 次原子读 + 分支 | `tracing` max-level 过滤，纳秒级 |
| 开启 debug | 每事件 span 构建 + 格式化 | 仅在 ON 时产生；LLM 调用数十 ms，此开销可忽略 |
| release 编译 | 同上 | **不启用** `release_max_level_*` feature——否则生产环境即使设 env 也拿不到 debug 日志。保留运行时过滤能力是 debug 生产问题的关键 |

> 结论：默认关闭时零成本（与现状「完全静默」等价），开启时开销相对网络 IO 可忽略。符合 aimux 性能卖点。

---

## 4. 详细设计

### 4.1 Span 树

```
generate_text / stream_text          [aimux-core/generate.rs]
  └─ span "generate" { provider, model, modality }
       └─ LanguageModel::do_generate / do_stream   [aimux-core/language_model.rs]
            └─ span "http_request" { method, host, attempt }   [aimux-provider-utils/http.rs]
                 ├─ debug! request { url, body_size, header_count }
                 ├─ debug! response { status, latency_ms, body_size }
                 ├─ warn!  retry { attempt, max, status, delay_ms, reason }
                 └─ error! failed { status, reason }
```

- `generate` span 字段：`provider`（如 "openai"）、`model`、`modality`（"text"/"image"/...）。`#[instrument(skip_all)]` 跳过大体积 options。
- `http_request` span 字段：`method`、`host`（仅 host，不含 query）、`attempt`（0=首次）。retry 循环内每次 attempt 复用同 span 的 `attempt` 字段递增。

### 4.2 事件清单

| 层级 | 事件 | level | 字段 | 触发点 |
|------|------|-------|------|--------|
| core | generate 开始 | INFO | provider, model | `generate_text`/`stream_text` 入口 |
| core | generate 结束 | INFO | ok, duration_ms, finish_reason | 返回前 |
| http | 请求发出 | DEBUG | method, url(无query), body_size, header_count | `send_request` 前 |
| http | 响应到达 | DEBUG | status, latency_ms, body_size | 拿到 `reqwest::Response` 后 |
| http | 重试 | WARN | attempt, max_retries, status, delay_ms, reason | `send_with_retry_raw` 退避前 |
| http | 最终失败 | ERROR | status, reason, attempts | 耗尽重试返回 Err 前 |
| http | 请求体（可选） | TRACE | body（截断+脱敏） | 仅当 `AIMUX_LOG_BODY=1` |
| http | 响应体（可选） | TRACE | body（截断+脱敏） | 仅当 `AIMUX_LOG_BODY=1` |
| stream | 建连成功 | DEBUG | status | `send_stream` 返回流前 |
| stream | 首字节 | DEBUG | ttfb_ms | 流首个 chunk |
| stream | 流结束/出错 | INFO/WARN | event_count, duration_ms | 流 close/error |

### 4.3 隐私与脱敏规则

**绝不默认落日志的内容**：
- Header **值**——只记 header **名**与数量。`Authorization`/`x-api-key`/`api-key` 等值永远不输出。
- URL query string——仅记 `scheme://host/path`，query 可能含 key。
- 请求/响应体——默认只记 `body_size`。完整体仅在 `AIMUX_LOG_BODY=1` 且 level=trace 时输出，且：
  - 截断至 4KB
  - 对 `Authorization`/`api-key`/`key`/`token` JSON 字段值打码为 `***`

### 4.4 配置入口

| 入口 | 用途 | 示例 |
|------|------|------|
| `AIMUX_LOG` env | RUST_LOG 风格，自动初始化 | `AIMUX_LOG=aimux=debug,aimux_provider_utils::http=trace` |
| `AIMUX_LOG_LEVEL` env | 简化版，不懂 RUST_LOG 语法的 FFI 用户 | `AIMUX_LOG_LEVEL=debug` |
| `AIMUX_LOG_BODY=1` env | 开启请求/响应体 trace 日志 | 见 4.3 |
| `init_logging(level)` API | 编程式（Rust/绑定 Rust 侧） | `aimux_providers::init_logging("debug")` |
| `aimux_init_logging(level)` C ABI | FFI 绑定 | `aimux_init_logging("debug")` |

默认级别：`warn`（只输出 retry 与失败，不打正常请求）。设 `AIMUX_LOG_LEVEL=debug` 看请求/响应摘要，`=trace` 看体。

### 4.5 crate 放置与依赖

| crate | 改动 |
|-------|------|
| `aimux-provider-utils` | 新增 `src/logging.rs`：`init_logging()` 实现（`tracing-subscriber` fmt + env filter + `Once`）。`http.rs` 加 span/事件。新增 `tracing-subscriber` 依赖。 |
| `aimux-providers` | re-export `init_logging`；`do_generate`/`do_stream` 加 `#[instrument(skip_all)]`（可选，低优先）。 |
| `aimux-core` | `generate.rs` 的 `generate_text`/`stream_text` 加 `#[instrument]` span。 |
| `aimux-stream` | 流式观测点（首字节/断流）。新增 `tracing` 依赖。 |
| `aimux-ffi` | 新增 `aimux_init_logging(*const c_char)` C ABI 导出。新增 `tracing-subscriber`（或经 provider-utils 间接）。 |

**`tracing-subscriber` 只进 `aimux-provider-utils` 和 `aimux-ffi`**，不进 `aimux-core`——保持 core 纯净、零 subscriber 依赖。`init_logging` 实现集中在 `provider-utils::logging`，`aimux-ffi` 和 `aimux-providers` 都调它，单一实现。

### 4.6 与 cache-tracing 的关系

| 维度 | 本 RFC（日志） | cache-tracing（审计） |
|------|--------------|---------------------|
| 目的 | 运维诊断（请求成败、重试、耗时） | 缓存命中率真实性审计 |
| 数据 | 标量字段（status/latency/size） | 结构化请求体（canonicalization/LCP/指纹） |
| 产物 | 人类读 stderr | 程序消费的审计 API + 统计 |
| 共用 | 都基于 `tracing` span 树 | 本 RFC 建立 `generate`→`http_request` span 树，cache-tracing 的 wrapper 挂在其下 |

**本 RFC 是 cache-tracing 的前置依赖**：先把 span 树立起来，审计子系统才能附着。

---

## 5. 实施计划

### 阶段 1：接通最小可用（MVP）—— ✅ 完成 (2026-08-03)
1. ✅ `aimux-provider-utils/src/logging.rs`：`init_logging()` + env 自动初始化 + `Once` 守护
2. ✅ `http.rs`：`http_request` span + 4 个核心事件（request/response/retry/failed）
3. ✅ `aimux-ffi`：`aimux_init_logging` C ABI 导出
4. ✅ 脱敏：header 值、URL query、body 默认不打
5. ✅ 文档：`docs/api/rust.md` 「Logging」段，FFI 头文件 `aimux_init_logging` 声明

### 阶段 2：覆盖面 —— ✅ 完成 (2026-08-03)
6. ✅ `aimux-core/generate.rs`：顶层 `generate` span + `generate_end` 事件
7. ✅ 流式观测：`ObservedByteStream`（首字节 TTFB / 断流 / 事件计数，http.rs 内实现，aimux-stream 保持零依赖）
8. ✅ `AIMUX_LOG_BODY` 可选体日志 + 脱敏（JSON 字段 `authorization`/`api-key`/`key`/`token` → `***`，截断 4KB）

### 阶段 3：验证 —— ✅ 完成 (2026-08-03)
9. ✅ 单测：`init_logging` 幂等、env filter 生效（行为断言）、脱敏正确（`CaptureWriter` 捕获 + `set_default`）
10. ✅ 集成测试：429 retry 日志链（warn 级）、request/response 摘要（debug 级）、body 脱敏（trace 级）、流事件（`tests/logging_test.rs`，wiremock）

### 工作量估计
- 阶段 1：~150 行（logging.rs ~60，http.rs 埋点 ~50，ffi ~20，文档）
- 阶段 2：~80 行
- 阶段 3：~100 行测试

---

## 6. 风险与权衡

| 风险 | 缓解 |
|------|------|
| 默认仍输出 warn 级 retry 日志，可能干扰宿主 | retry 本就是异常信号，warn 合理；若需完全静默设 `AIMUX_LOG=off` |
| body 日志可能泄露 PII | 默认关闭，需显式 `AIMUX_LOG_BODY=1` + 脱敏 + 截断 |
| 多次 `init` 冲突 | `Once` 守护 + env 自动初始化前检测全局 subscriber |
| 嵌入式/受限环境不想要 subscriber 依赖 | `tracing-subscriber` 仅在 provider-utils/ffi，core 无此依赖；且可用 feature gate 排除 |
| 与消费者自建 subscriber 冲突 | 库不自动 init；env 自动 init 仅在「全局未注册」时触发 |

---

## 7. 开放问题

1. ~~**是否需要 feature gate**~~：**已解决 (2026-08-03)**——不 gate。`tracing-subscriber` 只进 `aimux-provider-utils`（core 零新增依赖），嵌入场景可依赖排除该 crate 或接受 ~1s 编译增量；逃生口保留为未来选项。
2. ~~**span 字段命名**~~：**已解决**——扁平 `provider`/`model`/`modality`，与 `tracing` 惯例一致。
3. ~~**是否在本 RFC 一并加 `#[instrument]` 到各 provider 的 `do_generate`**~~：**已解决**——阶段 1/2 均不做（172 provider 改造成本高），http 层 + core 顶层已覆盖核心信息。

---

## 附录 A：用户视角示例

### A.1 默认（warn）——只看异常
```
WARN attempt=1 max_retries=3 status=429 delay_ms=850 reason=RateLimited
```

### A.2 debug——看请求摘要
```
INFO generate provider=openai model=gpt-4o modality=text
DEBUG request method=POST url=https://api.openai.com/v1/chat/completions body_size=2048 header_count=4
DEBUG response status=200 latency_ms=842 body_size=1024
INFO generate ok=true duration_ms=860 finish_reason=stop
```

### A.3 trace + body——深度调试
```
DEBUG request ... body_size=2048
TRACE request_body {"model":"gpt-4o","messages":[{"role":"system","content":"..."}]}  (截断4KB, api_key打码)
```
