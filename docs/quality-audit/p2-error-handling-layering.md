# P2 错误处理分层审查：thiserror(库) vs anyhow(应用)

- 审查日期：2026-08-06
- 范围：aimux-core / aimux-provider-utils / aimux-providers / aimux-ffi / aimux-stream（只读源码审查，未运行 cargo）
- 结论先行：**库/应用边界是干净的**——所有对外库 crate 的公共 API 都只返回 `AiMuxError`（thiserror）或 crate 内 thiserror 枚举，`anyhow` 完全没有进入任何库 crate（含测试）。主要问题集中在 `AiMuxError` 自身的结构化程度（String 载荷、状态码靠字符串解析）、分类一致性（`Timeout` 不可重试、10 处 JSON 解析错误被误标为 `Http`）与少量信息丢失点。

---

## 1. 概述

aimux 统一 325 个 provider 的访问层，错误处理分层如下：

```
provider 代码 ──▶ AiMuxError（统一错误，thiserror）
   │
   ├─ 非流式：Result<T, AiMuxError>
   └─ 流式：Stream<Item = Result<StreamPart, AiMuxError>>（StreamPart::Error 内嵌 AiMuxError）
          │
          ▼
FFI / bindings：{"error","error_type","status_code"} JSON 信封
```

- 库 crate（aimux-core、aimux-provider-utils、aimux-providers、aimux-stream、aimux-ffi）全部使用 `thiserror`，**没有任何一处 `anyhow`**。
- `anyhow = "1"` 只出现在根 `Cargo.toml` 的 `[workspace.dependencies]`（Cargo.toml:48），且**没有成员 crate 使用它**，也未出现在 Cargo.lock 中——是死声明（见 §3）。
- 应用层 `reference/cc-switch`（Tauri 桌面应用）使用 `anyhow = "1.0"` / `thiserror = "2.0"`（reference/cc-switch/src-tauri/Cargo.toml:67-68），但它是独立 workspace（不在根 `[workspace] members` 中），不属于本项目库边界。

---

## 2. AiMuxError 设计审查（aimux-core/src/error.rs）

### 2.1 正面

- `#[derive(Debug, Clone, Serialize, Deserialize, TS, Error)]`：thiserror 派生 ✓，serde + ts-rs 序列化为 FFI/bindings 服务 ✓。
- 错误分类基本覆盖目标维度（18 个变体）：

| 维度 | 变体 |
|---|---|
| auth | `Auth`、`TokenExpired` |
| rate_limit | `RateLimited { retry_after_ms }` |
| network | `Http`、`Timeout`、`Aborted` |
| parse | `Json` |
| provider_specific | `Provider`、`ApiCall`、`ModelNotFound`、`NoSuchModel`、`UnknownProvider`、`Unsupported` |
| 其他 | `InvalidArgument`、`InvalidPrompt`、`Stream`、`Tool`、`Other` |

- 提供 `is_retryable()`、`retry_after_hint()`、`error_type()`、`status_code()` 辅助方法，支撑 retry 与 FFI（error.rs:81-148）。
- `TokenExpired` 变体是好的设计点：codex.rs:234-236 把订阅端点的 401 映射为 `TokenExpired`，让集成方可以主动 refresh 后重试，而不是盲目重试（RFC-0018）。

### 2.2 问题

1. **几乎所有变体都是 `String` 载荷，结构化信息被拍平**（error.rs:11-70）。HTTP 状态码、provider 错误 type、retry-after 都没有独立字段。
2. **`status_code()` 靠解析消息字符串前缀 `"HTTP "` 还原状态码**（error.rs:135-148）。这非常脆弱，且覆盖面极差：
   - 401 → `Auth(message)`（消息为 provider 提取的 message，无 `HTTP ` 前缀）→ `status_code()` 返回 `None`；
   - 404 → `ModelNotFound(message)`（同样无前缀）→ `None`；
   - `Http`（reqwest 网络错误）→ `None`；
   - `RateLimited` 根本不在匹配列表 → `None`；
   - 只有 `Provider`（parse_provider_error 默认分支会盖 `"HTTP {status}: "` 前缀）和 `ApiCall`（http.rs:757 主动加前缀）能解析出状态码。
   - 结论：**FFI 信封里的 status_code 在大多数真实错误上都是 null**（详见 §6）。
3. **`ApiCall` 与 `Provider`、`Http` 语义重叠**：同一个 5xx 在 http 层是 `ApiCall`（可重试），被 `api_call_to_provider_error` 转换后是 `Provider`（不可重试），消费者无法只凭变体判断来源层级。
4. **`ModelNotFound` 与 `NoSuchModel` 重复**（error.rs:49 vs 55），职责边界不清（一个来自 HTTP 404，一个来自模型解析）。
5. **`Other(String)` 兜底**在对外库的公共枚举里是分类逃逸口（库内共有 3 处构造，另 2 处是 `"no attempts made"` 哨兵值，不算滥用，但存在）。

---

## 3. thiserror / anyhow 边界审查

### 3.1 各 crate 使用情况

| crate | 角色 | thiserror | anyhow | 公共 API 错误类型 |
|---|---|---|---|---|
| aimux-core | 库（对外发布） | ✓ error.rs、math.rs | ✗ | `AiMuxError`（统一）；`Size::parse`/`ModelId::parse` 返回 `Result<_, String>`（shared.rs:153,206，次要） |
| aimux-provider-utils | 库（对外发布） | 依赖已声明但 src 中**未使用**（Cargo.toml:19） | ✗ | `AiMuxError`（response/retry/http/api_key/url 等全部） |
| aimux-providers | 库（对外发布，325 provider） | 依赖已声明（Cargo.toml:24），具体用例在 provider 内（经 AiMuxError 构造，无独立 derive） | ✗ | 全部 `AiMuxError`（`language_model` 返回 `Result<Box<dyn LanguageModel>, AiMuxError>` 等） |
| aimux-stream | 库（对外发布） | ✓ sse.rs `SseError`、ndjson.rs `NdjsonError`、streaming_tool_call_tracker.rs `TrackerError` | ✗ | crate 内 thiserror 枚举（解析层内部错误，不跨库外泄） |
| aimux-ffi | 库（C ABI） | ✗（无 derive 需求） | ✗ | `*mut c_char` JSON 信封 |
| reference/cc-switch | 应用（独立 workspace，不在根 members） | ✓ `thiserror = "2.0"` | ✓ `anyhow = "1.0"`（services/skill.rs 43 处、profile.rs 1 处） | 应用自有 |

**判定：边界清晰，符合"库用 thiserror、应用用 anyhow"的惯例。** 所有对外发布 crate 的 `pub fn` 返回类型抽查结果（grep `Result<_, ...>`）：

- aimux-core / aimux-providers / aimux-provider-utils：100% `AiMuxError`；
- aimux-stream：`SseError` / `NdjsonError` / `TrackerError`（均为 thiserror derive）；
- 没有发现任何 `Result<_, anyhow::Error>`、`Box<dyn Error>` 出现在库公共签名中。

### 3.2 发现的问题

1. **根 workspace 的 `anyhow = "1"` 是死声明**（Cargo.toml:48）。没有任何成员 crate 的 `Cargo.toml` 引用它，Cargo.lock 中也不存在 anyhow 节点。既然 cc-switch 是独立 workspace，根 workspace 当前没有任何"应用"角色来证明 anyhow 的合理性——建议删除，或等真正需要它的应用 crate 加入时再引入。
2. **aimux-provider-utils 声明了 `thiserror` 但 src 中零使用**（Cargo.toml:19）。建议删除或补上（response.rs 的 ErrorStructure、http.rs 的 HttpRequest/Response 若定义成本地 thiserror 错误类型会更一致）。
3. `Result<Self, String>` 字符串错误出现在 aimux-core 公共 API（shared.rs:153,206）——量小、只用于值对象解析，但既然有 `AiMuxError::InvalidArgument`，建议统一。

---

## 4. 错误上下文与分类审查

### 4.1 parse_provider_error（aimux-provider-utils/src/response.rs:32-83）

职责：把 HTTP 错误响应解析成 `AiMuxError`。映射：401→`Auth`、429→`RateLimited{1000ms 硬编码}`、404→`ModelNotFound`、其他→`Provider("HTTP {status}: {msg}")`。

问题：

1. **`type_path` 提取的 provider 错误类型被丢弃**（response.rs:34,69-70：`_error_type` 赋值后从未使用）。`ErrorStructure` 明确支持 `type_path`，但提取结果不进错误、不进日志、不进 FFI——Provider 特定错误 type（如 OpenAI 的 `insufficient_quota`、`invalid_api_key`）全部丢失。这是"provider_specific 分类"维度的直接缺口。
2. **429 分支在 send 路径中是死代码且质量更差**：http.rs 的 send_with_retry_raw 在 line 740-753 提前拦截 429（会解析 retry-after 头），因此 parse_provider_error 的 `429 => RateLimited{1000}` 分支实际不可达；若被直接调用，会丢掉真实 retry-after 头和 provider message。两处 429→RateLimited 逻辑并存且行为不一致。
3. **404 → ModelNotFound 也丢弃了 HTTP 上下文**：消息只保留 provider 的 message 文本，没有 `"HTTP 404: "` 前缀（与"其他"分支不一致），导致 `status_code()` 无法还原 404（见 §2.2-2）。

### 4.2 http.rs 的 429 分支（http.rs:739-753）

- 正：正确解析 `retry-after-ms`（优先）/ `retry-after`（秒或 HTTP-date）头，并把 hint 放进 `RateLimited.retry_after_ms`，配合 retry 循环的 `retry_after_hint()` 使用——这是设计上最完整的一环。
- 负：**429 的响应 body 被丢弃**（`let _ = read_error_body(resp, request).await?`，http.rs:750）。`RateLimited` 变体只有 retry_after_ms，没有任何 provider message。FFI/JSON 消费者只能看到 `"rate limited: retry after 1000ms"`，丢失上游说明（如"quota exceeded"）。

### 4.3 5xx 的 ApiCall → Provider 转换（response.rs:95-100）

- `api_call_to_provider_error` 在 3 个 provider（recraft、google、vertex）使用，把 5xx 的 `ApiCall` 重映射为 `Provider`，符合这些 provider 的对外契约。非 `ApiCall` 原样透传，逻辑正确。
- 注意：重映射后错误从"可重试"变为"不可重试"语义（is_retryable 不含 Provider），但由于转换发生在 retry 循环之后，不影响重试逻辑本身——可接受。
- `provider_403_to_auth`（response.rs:110-118）依赖 `"HTTP 403: "` 消息前缀做匹配——与 §2.2-2 同源的字符串耦合，若某处 Provider 消息不带该前缀则静默失配。

### 4.4 JSON 解析错误被误标为 Http（10 处）

`serde_json::from_slice(&resp.body).map_err(|e| AiMuxError::Http(e.to_string()))` 出现在：

- aimux-providers/src/bedrock/model.rs:134、embedding.rs:216、reranking.rs:219
- aimux-providers/src/google/model.rs:135、embedding.rs:142/224、files.rs:239/284
- aimux-providers/src/vertex/model.rs、embedding.rs、anthropic_model.rs

这是**分类错误**：响应体 JSON 解析失败应归 `AiMuxError::Json`（错误类型已有此变体），标成 `Http` 一是误导消费者（以为是网络问题），二是 `Http` 在 `is_retryable()` 中为 true——若这类错误进入重试判定，会对**不可重试的本地解析错误**发起重试。当前它们出现在 provider 响应解析路径、不在共享 retry 循环内，所以不会实际触发重试，但分类语义错误仍然存在。

### 4.5 OpenAI 重复实现映射逻辑（openai/model.rs:816-834）

`stream_error_to_ai_error` 复制了 401/429/404/other 的映射，与 parse_provider_error 重复：
- 429 硬编码 `retry_after_ms: 1000`，不解析任何头；
- 用错误对象里的 `code` 字段当 HTTP 状态码（`unwrap_or(500)`）——对 OpenAI 流式错误对象成立，但对其他 provider 未必，是启发式。
- 建议收敛为共享实现（至少让 429 走 retry-after 头解析或直接复用 parse_provider_error 的语义）。

### 4.6 Anthropic 流内错误（anthropic/stream.rs:486-495）

流内 `StreamEvent::Error`（如 `overloaded_error`）一律映射为 `AiMuxError::Provider`。`overloaded_error`（HTTP 529）本质是瞬时限流，但 `Provider` 不可重试。流内错误随流终止、无法重试（镜像 TS 行为），可接受；但建议考虑将 529/overload 类错误映射为 `RateLimited`（至少带上 retry 语义信息），供集成方自行处理。

### 4.7 上下文保留小结

| 场景 | 保留 | 丢失 |
|---|---|---|
| 4xx（parse_provider_error） | provider message（有回退到 raw body） | HTTP 状态码（多数变体）、provider error type |
| 429（http.rs） | retry-after hint | 响应 body message |
| 5xx（ApiCall） | `HTTP {status}: {body}` 全文 | —（最好） |
| 流内错误（anthropic） | provider message | 状态码/类型 |
| JSON 解析失败 | 错误文本 | 分类（误为 Http） |

---

## 5. retry 与错误分类配合（aimux-provider-utils/src/retry.rs + http.rs）

### 5.1 现状（正确部分）

- `is_retryable()`（error.rs:81-86）= `RateLimited | Http | ApiCall`。非 4xx 非 5xx 错误在 http.rs:759-761 立即返回、不进重试判定；429/5xx/网络错误进入重试——**可重试/不可重试的基本划分正确**：`Auth`、`InvalidArgument`、`ModelNotFound`、`Aborted` 均不重试。
- `retry.rs` 提供两套：纯指数退避 + 尊重 retry-after 头的版本；Full Jitter（`get_retry_delay_ms_with_jitter`）防惊群；hint 合理性检查（<60s 或 <指数退避）镜像 TS SDK。实现质量高，测试充分（retry.rs 测试覆盖 hint/负数/日期回退等）。
- http.rs 的 429 分支读取真实 retry-after 头并把 hint 带进 `RateLimited`（http.rs:740-753），与 `retry_after_hint()`（error.rs:96-101）闭环——这是整个 retry 设计里最完整的链路。

### 5.2 问题

1. **`Timeout` 不可重试**（error.rs:81-86 未包含）。超时是典型的瞬时错误（连接/首字节超时），Vercel AI SDK 把超时视为可重试。当前 `Timeout` 不进入重试，`http.rs:389/416` 的 30s 总超时一旦触发就立即失败——对网络抖动场景不友好。**建议把 `Timeout` 加入 `is_retryable()`**（注意：total-timeout 场景重试可能加剧堆积，至少应支持配置化）。
2. `retry_after_ms: u64`（error.rs:35）与 `retry_after_hint() -> i64`（error.rs:98）类型往返转换，无实际危害但可统一。
3. `retry.rs` 的两个入口与 http.rs 内部 retry 逻辑并存（三套退避实现），对外公开的 `retry_with_exponential_backoff*` 目前只在测试中使用（grep 未见 provider 调用）——属于"已发布但未接线"的 API，应确认其存在必要。

---

## 6. FFI 错误传递（aimux-ffi/src/lib.rs）

### 6.1 机制

- 统一 JSON 信封（lib.rs:214-223）：`{"error":"<message>","error_type":"<variant>","status_code":<u16|null>}`。
- `error_json_from`（lib.rs:232-234）用 `err.error_type()`（变体名，机器可读）与 `err.status_code()` 组装；`fire_error_struct`（lib.rs:262-266）给回调同款信封。
- 构造函数/调用失败统一返回该信封的 C 字符串；FFI 自身参数错误（null/非 UTF-8）返回 `error_type: "InvalidArgument"`（lib.rs:247-249）。**机制本身干净、一致。**

### 6.2 信息丢失

1. **`retry_after_ms` 不进信封**：`RateLimited` 的 retry hint 只出现在 Display 文本里（`"rate limited: retry after 1000ms"`），C 侧拿不到机器可读的延迟值。bindings 若想做 429 智能重试只能解析字符串。
2. **`status_code` 绝大多数为 null**（承 §2.2-2）：只有 `Provider`/`ApiCall` 带 `HTTP ` 前缀的消息能解析；`Auth`(401)、`RateLimited`(429)、`ModelNotFound`(404) 全部为 null——恰恰是最需要状态码的三个场景。
3. **provider 错误 type（error.rs type_path 提取值）完全没有传递路径**。
4. 无错误链/source 信息（对 C ABI 可接受，但 Go/Python bindings 同样只能拿到这三字段，见 bindings/python/src/lib.rs:65 用 `error_type()` 拼消息）。

---

## 7. 发现的问题与建议汇总

### 高优先级

| # | 问题 | 位置 | 建议 |
|---|---|---|---|
| H1 | `status_code()` 靠 `"HTTP "` 前缀字符串解析，Auth/RateLimited/ModelNotFound 均解析不出 | error.rs:135-148 | 为 `Http`/`Auth`/`RateLimited`/`ModelNotFound` 等增加结构化 `status_code: u16` 字段（或把消息前缀约定固化为唯一构造路径，删除字符串解析） |
| H2 | JSON 解析错误 10 处误标 `AiMuxError::Http`（应属 Json，且 Http 可重试） | bedrock/model.rs:134、embedding.rs:216、reranking.rs:219；google/model.rs:135、embedding.rs:142/224、files.rs:239/284；vertex/model.rs、embedding.rs、anthropic_model.rs | 改为 `AiMuxError::Json`；建议提供 `From<serde_json::Error>` 的 `?` 用法（error.rs:73-77 已有 impl，72 处 `map_err(|e| Json(e.to_string()))` 可简化） |
| H3 | 429 响应 body 在 http.rs 被丢弃，`RateLimited` 无 provider message | http.rs:750 | 把 body 文本并入 `RateLimited` 消息（如加 `message` 字段或拼接进 Display） |
| H4 | provider error type（type_path）提取后丢弃 | response.rs:34,69-70 | 引入 `ProviderError { provider_type, message, status }` 结构化变体或在 Provider 消息中拼接 type |

### 中优先级

| # | 问题 | 位置 | 建议 |
|---|---|---|---|
| M1 | `Timeout` 不在 `is_retryable()` | error.rs:81-86 | 评估将 `Timeout` 纳入可重试（或提供配置开关） |
| M2 | parse_provider_error 的 429 分支死代码 + 硬编码 1000ms | response.rs:77-79 | 删除或统一为 http.rs 的头感知路径 |
| M3 | OpenAI `stream_error_to_ai_error` 重复 401/429/404 映射，429 硬编码 | openai/model.rs:816-834 | 收敛到共享实现 |
| M4 | 根 workspace `anyhow = "1"` 死声明；provider-utils `thiserror` 未使用 | Cargo.toml:48；aimux-provider-utils/Cargo.toml:19 | 删除未使用依赖（anyhow 不在 Cargo.lock 中，确认无隐式依赖） |
| M5 | FFI 信封缺 `retry_after_ms`；status_code 大多为 null | aimux-ffi/src/lib.rs:214-234 | 扩展信封加 `retry_after_ms` 字段；与 H1 一并解决 status_code |

### 低优先级 / 备注

| # | 问题 | 位置 | 建议 |
|---|---|---|---|
| L1 | `ApiCall`/`Provider`/`Http` 语义重叠，`ModelNotFound`/`NoSuchModel` 重复 | error.rs | 在文档中明确各变体来源层级，或合并 |
| L2 | `Result<Self, String>` 出现在公共 API | shared.rs:153,206 | 换 `InvalidArgument` |
| L3 | `provider_403_to_auth` 依赖消息前缀 | response.rs:110-118 | 随 H1 一起改为结构化字段判断 |
| L4 | Anthropic 流内 overloaded_error → Provider（不可重试语义） | anthropic/stream.rs:486-495 | 考虑映射为 RateLimited 或保留 Provider 但文档注明 |
| L5 | `retry.rs` 两个公开 retry 函数无调用方 | retry.rs:26,86 | 确认用途或标记 internal |

### 边界判定结论

- ✅ **库 vs 应用边界清晰**：thiserror 用于所有库 crate；anyhow 仅在独立应用（cc-switch）中；无任何库公共 API 泄露 `anyhow::Error`。
- ⚠️ 主要改进空间在 **AiMuxError 的结构化程度与分类一致性**（H1-H4），而非 thiserror/anyhow 分层本身。
- 建议的最小闭环改动：结构化 `status_code`（H1）+ 修正 Json/Http 误标（H2）+ 保留 429 body（H3），即可显著提升 FFI 与集成方可编程处理能力。
