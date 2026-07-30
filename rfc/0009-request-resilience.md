# RFC-0009：请求优化 — 参考 catcher 的设计

> **状态**：DRAFT（待评审）
> **日期**：2026-07-29
> **范围**：`aimux-provider-utils` 参考 catcher 的三个具体设计点（连接池配置、jitter 退避、固定超时），用 reqwest 原生 + 现有 retry.rs 实现请求层优化，不引入 catcher-http 依赖
> **关联**：[RFC-0002](0002-provider-improvements.md) 厂商适配层改进、[RFC-0003](0003-test-cassette.md) 测试录像方案

## 1. 动机

aimux 把"统一 LLM 服务接入"做得很全（172 厂商、6 语言绑定），但**底层的请求收发是裸奔的**。证据如下：

| # | 问题 | 证据 |
|---|------|------|
| 1 | **45 个 provider 各自 `reqwest::Client::new()`**，无连接池共享、无 TLS 会话复用 | `grep -rn "Client::new()" aimux-providers/src/` → 45 处。reqwest 官方建议整个应用复用单个 Client |
| 2 | **retry 逻辑是死代码** | `aimux-provider-utils/src/retry.rs` 定义并 re-export 了 `retry_with_exponential_backoff*`，但全仓**零调用点** |
| 3 | **79 个直接 `.send()` 点无 retry 包裹** | `grep -rln ".send().await" aimux-providers/src/` → 79 文件。失败即抛错，429/5xx 不重试 |
| 4 | **全仓无超时** | `grep -rn ".timeout(\|connect_timeout\|pool_idle\|tcp_keepalive"` → 空。挂起的连接会永久阻塞 |
| 5 | **retry 无 jitter** | `retry.rs` 是纯指数退避，并发 429 会惊群 |

[catcher](https://github.com/eric8810/catcher)（同作者、同 Rust+reqwest+rustls 技栈、同 MIT）的 `catcher-http` crate 已把请求/弹性层做成熟。经逐模块核实，对其取舍结论见 §3。

## 2. 方案选型：参考，不依赖

| | 路线 A：依赖 catcher-http | 路线 B（本 RFC）：参考 catcher 设计，reqwest 原生实现 |
|---|---|---|
| 做法 | 依赖 `catcher-http` + `catcher-core`，包错误转换 + 自建 builder | 抄 catcher 的具体设计点，用 reqwest 原生 + 现有 retry.rs 实现 |
| 新依赖 | catcher-http + catcher-core + catcher-dns + backon + reqwest-middleware + retry-policies + parking_lot + tokio-util + rmp-serde | **零** |
| reqwest 版本 | 强制升级 0.12→0.13（catcher 用 0.13） | 无需升级，保持 0.12 |
| retry-after header | ❌ catcher 的 `HttpError{status,body}` 无结构化透传，backon 固定退避不读 header | ✅ 保留 aimux 现有 retry.rs 的 header 读取能力（更强） |
| 错误转换 | 需映射 17 个 CatcherError 变体 | 不需要，直接用 AiMuxError |
| 工作量 | reqwest 升级 + 错误转换 + 自建 HttpRequestBuilder（catcher 的 HttpRequest 是纯数据结构无链式） + 迁移 172 provider | 抄 3 个设计点，约百行 |
| 自主权 | 跟随 catcher 发版 | 自主可控 |

**结论：catcher 真正对 aimux 有价值的是它的设计模式（PoolConfig 字段、jitter 策略、超时配置），不是它的代码包。** 借鉴模式比背依赖划算，尤其 retry-after 这点证明了"参考"在 LLM 场景反而比"引入"语义更正确。

## 3. 取舍：catcher 各能力对 aimux 的处置

逐模块核实 catcher-http 源码后的判断：

| catcher 能力 | 对 aimux 的处置 | 理由 |
|---|---|---|
| `PoolConfig` 字段设计 | ✅ **引入**（抄字段，reqwest 原生实现） | `max_idle_per_host`/`idle_timeout_secs`/`keep_alive`/`keep_alive_interval_secs`，reqwest ClientBuilder 全支持，收益最高 |
| `Full Jitter` 退避策略 | ✅ **引入**（补到现有 retry.rs） | catcher `backoff.rs` 的 `DecorrelatedJitter` 即 AWS Full Jitter，防惊群；aimux 现有 retry.rs 是纯指数无 jitter |
| 固定超时字段 | ✅ **引入**（抄 `connect_timeout_ms`/`response_timeout_ms`） | reqwest ClientBuilder 原生支持 |
| `AdaptiveTimeout`（P90 RTT 自适应） | ❌ **不引入** | `timeout = clamp(P90_RTT * multiplier)`，LLM 请求时长取决于生成长度/max_tokens 而非网络 RTT，会误杀长生成请求 |
| `CircuitBreaker` 状态机 | 📌 **暂缓** | 实现成熟（CLOSED→OPEN→HALF_OPEN 约 150 行），但 aimux 是库不是网关，无 fallback 目标；retry 先行，retry 落地后若"连续失败每次等超时"成痛点再上 |
| `reqwest-retry` 中间件整体 | ❌ **不引入** | backon 固定退避不读 retry-after header，比 aimux 现有 retry.rs 弱 |
| SSE 自动重连 | 📌 **暂缓** | 流式+retry 语义复杂（已吐 token 后重试会重复内容），独立提案 |
| msgpack / WS / TLS pinning / DNS 缓存 / 网络切换热重建 | ❌ **不引入** | LLM 场景用不到 |

## 4. 设计

### 4.1 引入点 1：共享 Client + PoolConfig（reqwest 原生）

参考 catcher 的 `PoolConfig`（[catcher-http/src/types/http.rs](https://github.com/eric8810/catcher/blob/master/packages/catcher-http/src/types/http.rs)），用 reqwest `ClientBuilder` 原生实现：

```rust
//! aimux-provider-utils/src/http.rs

use std::sync::OnceLock;
use std::time::Duration;
use reqwest::Client;

/// 连接池配置（参考 catcher PoolConfig 字段设计）。
#[derive(Debug, Clone)]
pub struct PoolConfig {
    pub max_idle_per_host: usize,        // catcher: 10
    pub idle_timeout_secs: u64,           // catcher: 30 — 防 retry 复用死连接
    pub keep_alive: bool,                // catcher: true
    pub keep_alive_interval_secs: u64,   // catcher: 20 — 更快发现死连接
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            max_idle_per_host: 10,
            idle_timeout_secs: 30,
            keep_alive: true,
            keep_alive_interval_secs: 20,
        }
    }
}

/// 超时配置（参考 catcher HttpClientConfig 的两个超时字段）。
#[derive(Debug, Clone)]
pub struct TimeoutConfig {
    pub connect_timeout_ms: u64,         // catcher: 10_000
    pub response_timeout_ms: u64,        // catcher: 30_000；流式请求传 0 禁用
}

/// 全局共享的 reqwest::Client。OnceLock 保证只构建一次，
/// 连接池/TLS 会话全仓复用。替代 45 处 Client::new()。
static SHARED: OnceLock<Client> = OnceLock::new();

/// 获取（或惰性初始化）共享 reqwest Client。
pub fn shared_client() -> &'static Client {
    SHARED.get_or_init(|| build_client(PoolConfig::default(), TimeoutConfig::default()))
}

fn build_client(pool: PoolConfig, timeout: TimeoutConfig) -> Client {
    let mut b = Client::builder()
        .connect_timeout(Duration::from_millis(timeout.connect_timeout_ms))
        .pool_max_idle_per_host(pool.max_idle_per_host)
        .pool_idle_timeout(Duration::from_secs(pool.idle_timeout_secs));
    if pool.keep_alive {
        b = b.tcp_keepalive(Duration::from_secs(pool.keep_alive_interval_secs));
    }
    if timeout.response_timeout_ms > 0 {
        b = b.timeout(Duration::from_millis(timeout.response_timeout_ms));
    }
    b.build().expect("shared reqwest Client build failed")
}
```

**收益**：TLS 会话复用、连接池共享——aimux 当前最大的性能缺口，零新依赖解决。干掉 45 处 `Client::new()`。

### 4.2 引入点 2：Jitter 退避（补到现有 retry.rs，保留 retry-after）

catcher 的 `DecorrelatedJitter`（[backoff.rs](https://github.com/eric8810/catcher/blob/master/packages/catcher-http/src/resilience/backoff.rs)）即 AWS Full Jitter：`delay ∈ [0, calculated_backoff]`。

aimux 现有 `retry.rs` 是纯指数退避无 jitter。补 jitter，**且保留 aimux 现有的 retry-after header 读取能力**（这点比 catcher 强，不丢）：

```rust
//! aimux-provider-utils/src/retry.rs（增量补丁，不重写）

/// 在 get_retry_delay_ms 基础上叠加 Full Jitter（参考 catcher DecorrelatedJitter）。
/// delay = random(0, base)，base 仍优先采用 retry-after hint，回退指数退避。
pub fn get_retry_delay_ms_with_jitter(
    hint: Option<i64>,
    exponential_delay_ms: i64,
    rng: &mut impl rand::Rng,
) -> i64 {
    let base = get_retry_delay_ms(hint, exponential_delay_ms); // 复用现有逻辑
    if base <= 0 { return 0; }
    rng.gen_range(0..base) // Full Jitter
}
```

- 现有 `get_retry_delay_ms` / `parse_retry_after` 保留不动，新函数复用它们
- 现有 `retry_with_exponential_backoff_respecting_retry_headers` 内部 sleep 处改用 jitter 版本
- 新增 `rand` 依赖（workspace 已有 `futures`/`tokio`，rand 体积小）

**收益**：防并发 429 惊群，且不丢 retry-after 语义。

### 4.3 引入点 3：固定超时

直接用 §4.1 的 `TimeoutConfig`。关键决策：

- **非流式请求**：`response_timeout_ms = 30_000`（catcher 默认值），reqwest `.timeout()` 守护
- **流式请求**：`response_timeout_ms = 0` 禁用整体超时——LLM 流式时长取决于生成长度，固定超时会误杀长生成。仅保留 `connect_timeout_ms = 10_000` 守护建连阶段

由 provider 在构造请求时按是否流式选择。`shared_client()` 用默认 30s 整体超时；流式 provider 调用时需用单独的不带整体超时的 client（或用 reqwest 的 per-request `.timeout(None)` 覆盖）。

## 5. 不做的事（不引入项的依据）

1. **不引入 catcher-http 依赖**。catcher 真正对 aimux 有价值的 3 个点（§4）用 reqwest 原生 + 现有 retry.rs 零依赖即可复现，而引入依赖的代价（reqwest 强制升级 0.12→0.13 + 17 个 CatcherError 变体转换 + 自建 HttpRequestBuilder + 依赖树膨胀 + retry-after 语义倒退）远大于收益。
2. **不引入 AdaptiveTimeout**。`timeout = P90_RTT * multiplier` 对 LLM 不成立：两个请求 RTT 都是 200ms，但一个生成 10 token（总 500ms）、一个生成 2000 token（总 30s），用 RTT 算超时会误杀后者。
3. **不引入 CircuitBreaker**。aimux 是库不是网关，单 provider 失败就失败，无 fallback 目标。retry 先行，若连续失败每次等超时成痛点再上（届时抄 catcher 状态机约 150 行即可）。
4. **不引入 reqwest-retry 中间件**。backon 固定退避不读 retry-after header，比 aimux 现有 retry.rs 弱。
5. **不引入 SSE 自动重连**。流式+retry 语义复杂，独立提案。
6. **不引入 msgpack / WS / TLS pinning / DNS 缓存 / 网络切换热重建**。LLM 场景用不到。

## 6. 迁移策略

### 6.1 测试安全网

aimux 测试用 **wiremock 本地 mock server**（见 `aimux-providers/tests/openai_image_test.rs`），靠 `base_url` 指向 `localhost`，不依赖特定 Client 实例。重构 client 构造方式不破坏这些测试——它们只依赖 base_url 路由。这是本方案最大的可行性保障。

注意：retry 会在 mock 测试里放大请求次数。wiremock 的 `.expect(N)` 断言需相应调整（未设 `expect` 的 mock 默认允许多次命中，多数测试不受影响）。

### 6.2 分批落地

| 批次 | 范围 | 说明 |
|---|---|---|
| 1 | `aimux-provider-utils` 新增 `src/http.rs`（shared_client + PoolConfig + TimeoutConfig） + retry.rs 补 jitter | 新增，不改 provider |
| 2 | 把现有 retry.rs 接进请求路径（当前是死代码，0 调用） | 新增 `send_with_retry` 包装函数 |
| 3 | 迁移 11 个原生协议 provider（openai/anthropic/google/...）的 `Client::new()` → `shared_client()` | 优先，覆盖主流量 |
| 4 | 145 个 OpenAI 兼容薄封装 | 共享同一请求路径，可脚本化批量替换 |
| 5 | 语音/图像/视频专用 provider | 非流式居多，最简单 |

每批后跑 `cargo test -p aimux-providers --tests` 守护。**无需 reqwest 版本升级**（这是路线 B 相对路线 A 的一大优势）。

## 7. 风险

| 风险 | 等级 | 缓解 |
|---|---|---|
| **172 provider 接触面大** | 中 | 分批迁移（§6.2），每批回归；薄封装共享路径可脚本化批量替换 |
| **流式 + retry 语义** | 中 | 第一版仅覆盖建连阶段重试，已吐 token 后不重试；整体超时对流式禁用 |
| **wiremock `.expect(N)` 与 retry 次数冲突** | 低 | 多数 mock 未设 expect；个别需调整为 N×重试次数 |
| **shared_client 默认超时误杀流式** | 中 | 流式请求禁用整体超时（§4.3），或 per-request 覆盖 |
| **jitter 引入 rand 依赖** | 低 | rand 体积小，标准库生态 |

## 8. 实现顺序

1. **包装层**：`aimux-provider-utils` 新建 `src/http.rs`（shared_client + PoolConfig + TimeoutConfig）。
2. **retry 接入**：现有 `retry.rs` 补 jitter；新增 `send_with_retry` 把 retry 接进请求路径（当前 0 调用）。
3. **试点**：迁移 `aimux-providers/src/openai/`（原生协议，主流量），跑 openai 全部测试验证。
4. **铺开**：按 §6.2 批次迁移剩余 provider。
5. **收尾**：更新 README 架构图与 `aimux-provider-utils` 模块说明。

每一步均可独立合入，不阻塞后续。无前置依赖升级。
