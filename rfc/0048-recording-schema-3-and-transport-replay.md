# RFC-0048：录制 schema 3 与传输回放

> **Status**: Draft
>
> **Date**: 2026-10-20
>
> **Baseline**: master `b91829af`。文中 `文件:行号` 都指这个提交；上游引用 `@ai-sdk/provider-utils@5.0.51`（`reference/aisdk-pinned`）、`@ai-sdk/devtools@1.0.30`、`@ai-sdk/test-server@2.0.3` 的 git tag。
>
> **Tracks**: [#270](https://github.com/arcships/aimux/issues/270)（接替 #167），关联 [#179](https://github.com/arcships/aimux/issues/179)。
>
> **Implements**: [RFC-0041](0041-replay-governance.md) §12 的 G1（schema/policy）、G2（leaf/session）、G3（消费闭包）。RFC-0041 已定的格式约束、匹配语义、离线鉴权与错误码不在这里重新设计，本文只定它留给实现的接口、切片和顺序。
>
> **Related**: [对齐文档](../docs/aisdk-architecture-alignment.md) §4.3、§4.4、§8.3、§9.1；[RFC-0039](0039-ops-bindings-contract.md) 登记项 D12、G-b、G-c、G-k（[实现说明](../docs/rfc-0039-implementation.md)）；[RFC-0042](0042-v4-type-boundaries.md)（DTO 与 codegen）；RFC-0044（[PR #279](https://github.com/arcships/aimux/pull/279)：crate 拆分、Fetch 观察点、`aimux-devtools`）。

## 1. 决策摘要

1. **一种格式。** 录制只写 schema 3（RFC-0041 §4），读到其他版本返回 `UnsupportedRecordingSchema`。schema 1、2 的读取、snake_case 别名和 `ProviderRecord` 全部删除，不做数据迁移。
2. **录制在传输叶子。** 每次 HTTP 交换记录交给传输的请求原始字节、响应头、有序的响应分块及其相对时间、终态。分块不拼成文本，二进制不转成有损 UTF-8。
3. **回放就是一个 Fetch。** `ReplaySession::fetch()` 返回实现 `Fetch` 的对象，离线组装的 provider 照常编码请求、解析响应，所以每个 provider 自动获得离线回放，不再有只认 OpenAI chat 的解码器。
4. **离线组装的配方归 provider 包。** `aimux-providers` 提供 `create_offline_provider(name, fetch)`：占位 key、只用给定的 transport。不能离线鉴权的包返回 `OfflineConstructionUnavailable`。
5. **crate 归属以 RFC-0044 为准。** 回放引擎最终在 `aimux-devtools`。本文不定 crate 归属，也不定调用层事件怎样接到操作上（RFC-0044 §4.1 留给的调用观测后续 RFC）。本文只规定这些接口必须提供什么（§4.1）。
6. **每个 PR 改完它影响的全部消费方。** ops、FFI 头文件、八种绑定的生成物、CLI、Web 跟着同一个 PR 走，不留过渡代码（对齐文档 D31）。

## 2. 现状

### 2.1 录制

```text
aimux_core::generate_text                          aimux-core/src/generate.rs:954
 └ generate_in_operation                           aimux-core/src/generate.rs:1040
   └ recording::recorder()                         aimux-core/src/generate.rs:1079   ← 读全局 RECORDER（recording.rs:600）
     └ Recorder::record_input                      aimux-core/src/generate.rs:1082   ← prompt + CallOptions（JSON 脱敏）
     └ Recorder::record_provider                   aimux-core/src/generate.rs:1088   ← ProviderRecord::from_model：provider() 按 "." 截出 provider_id
   └ RecordingContext::scope(context, …)           aimux-core/src/generate.rs:1131   ← task-local
     └ model.do_generate(call_options)             aimux-core/src/generate.rs:2272
       └ provider 编码请求 → HttpRequest::new
         └ RecordingContext::current()             aimux-provider-utils/src/http.rs:126 ← provider-utils 读 core 的 task-local
         └ send_request_once                       aimux-provider-utils/src/http.rs:305
           └ fetch.fetch(fetch_request)            aimux-provider-utils/src/http.rs:375
           └ observe_response                      aimux-provider-utils/src/http.rs:628
             └ record_exchange(HttpExchange)       aimux-provider-utils/src/http.rs:647 ← 请求体是脱敏后的 JSON 文本
             └ ObservedBody → ResponseRecording::finalize
                                                    aimux-provider-utils/src/http.rs:765 ← 分块拼成一段 String，1 MiB 截断，from_utf8_lossy
   └ Recorder::record_outcome                      aimux-core/src/generate.rs:1206
 JsonlRecorder 写线程 writer_loop                  aimux-core/src/recording.rs:1177
   └ try_finalize → Recording::ready()             aimux-core/src/recording.rs:1309 / :185 ← completion barrier
（stream_text 在 generate.rs:1599 起同样三步，outcome 由 RecordingOutcomeStream（:1926）在流结束时写；decide 在 decision_model.rs:620–698）
```

上游没有录制格式。`@ai-sdk/devtools` 是外挂在模型调用层的观察工具：`devToolsMiddleware()`（`devtools/src/middleware.ts:101`）的 `wrapGenerate` / `wrapStream`（`:131`、`:201`）把每步的参数和结果写进本地 `generations.json`（`db.ts:5`），`DevToolsTelemetry`（`integration.ts:113`）收 telemetry 事件，按 `callId` 归组。上游不在 Fetch 层观察，也不回放。

### 2.2 模型层 mock 回放

```text
aimux_ops replay.create                            aimux-ops/src/ops/replay.rs:243
 └ from_jsonl → MockReplayModel::new              aimux-ops/src/ops/replay.rs:217 / :236
宿主把它当 LanguageModel 用
 └ MockReplayModel::do_generate                    aimux-core/src/replay.rs:519
   └ ReplayMatcher::match（Exact/Score/Prefix）     aimux-core/src/replay.rs:46 / :77 / :226 / :362
   └ rebuild_generate_result(rec)                  aimux-core/src/replay.rs:652  ← 按 OpenAI chat.completions 解 choices[0]
 └ MockReplayModel::do_stream → rebuild_stream_result
                                                    aimux-core/src/replay.rs:748  ← 只认 OpenAI SSE；其他 provider 返回 Unsupported
```

回放跳过了 provider 的解析器，录下来的是 OpenAI 以外的 provider 就无法 mock，provider 解析代码的回归也测不到。上游测试同样在 Fetch 层喂预制响应：`createTestServer`（`test-server/src/create-test-server.ts:112`）用 msw 截住 `fetch`，provider 照常编码和解析；provider 的 `fetch` 设置最后落到 `postJsonToApi` 里的 `fetch(url, …)`（`provider-utils/src/post-to-api.ts:98`）。aimux 的 `Fetch` trait（`aimux-provider-utils/src/fetch.rs:197`）就是这个设置的 Rust 形状，回放接在这里与上游测试的做法一致。

### 2.3 live 回放

```text
aimux_ops replay.live（registry 目标）              aimux-ops/src/ops/replay.rs:305
 └ aimux_providers::replay::rebuild_provider       aimux-ops/src/ops/replay.rs:334 → aimux-providers/src/replay.rs:58
   └ registry.provider(provider_id).language_model(model_id)    ← provider_id 来自 provider() 截断，是猜的
 └ aimux_core::replay::replay_with_model           aimux-core/src/replay.rs:1029
   └ generate_text_from_language_model_prompt      aimux-core/src/replay.rs:1087 ← 用录下的规范层 prompt 重跑操作
（tools/aimux-replay 的 --mock 用 MockReplayModel（main.rs:192），live 用 rebuild_provider（:199）；aimux-web 的 /api/mock/load 和 /api/replay 同样）
```

### 2.4 要解决的问题

| 问题 | 位置 |
|---|---|
| 响应体拼成文本、1 MiB 截断、丢分块边界与时间，无法逐字节回放流 | `http.rs:765` |
| 请求体存的是脱敏后的 JSON 文本，与线上发送的字节不同 | `http.rs:829` |
| mock 回放只支持 OpenAI chat 格式，跳过 provider 解析器 | `replay.rs:652`、`:748` |
| 重建目标靠 `provider()` 字符串截断猜 registry 名 | `recording.rs:326`、`providers/src/replay.rs:58` |
| 组合模型的子调用只用 `step` 文本区分，attempt 编号跨整个调用全局分配 | `recording.rs:608` |
| `replay.capabilities` 返回 UnsupportedFunctionality；绑定没有离线 mock 入口 | RFC-0039 G-c、G-k |

## 3. schema 3

字段由 RFC-0041 §4.1 和对齐文档 §4.3 定，这里给出完整形状。JSON 字段 camelCase，联合的 `type` 值 kebab-case，类型从 Rust 生成（RFC-0042），不接受 snake_case 别名。

```text
Recording {
  schema: 3,
  callId, recordedAt,
  operation: "language-model" | "decision",
  target: { modelRef?: "<registry key>:<modelId>", model: { provider, modelId } },
  sessionId?, sessionStep?,
  capturePolicy: "metadata" | "payload",
  input: Captured<{ prompt, options }>,
  modelCalls: [ { seq, attempt, parent?: { seq, attempt }, stepLabel? } ],
  exchanges: [ Exchange ],
  outcome: { status, finishReason?, usage?, error?, errorValue?, decisionResult? },
  complete, transportClosed,
  replayability: { mock, live, reasons: [string] }
}
Captured<T> = { type: "captured", value: T } | { type: "forbidden", reason } | { type: "missing", reason }
Exchange {
  exchangeId: "<seq>.<attempt>.<index>",
  phase: "model" | "auxiliary",
  modelCall?: { seq, attempt },
  exchangeIndex,                                   // 同一 (seq, attempt) 内从 1 递增
  boundary: "transport-leaf",
  request: { method, url, headers: [ { name, value? } ], body: Captured<base64> },
  response?: { status, headers: [ { name, value? } ] },
  chunks: Captured<[ { offsetUs, data: base64 } ]>,
  timing: { responseHeadOffsetUs?, terminalOffsetUs? },
  terminal?: { type: "eof" } | { type: "error", kind: "connect"|"timeout"|"aborted"|"io"|"other", message } | { type: "abandoned" },
  truncated?: { originalBytes, capturedBytes }
}
```

- **`seq`、`attempt` 与 `exchangeIndex`。** 取自 RFC-0044 §4.2 的 `DiagnosticScope`（[PR #279](https://github.com/arcships/aimux/pull/279)）：`model_call { seq, attempt, parent }` 和每个 (seq, attempt) 内的交换计数器，分配规则以那里为准，本文不另定。`exchangeId` 是这三个值的拼接。`stepLabel` 保留现在的 `router[0]:…` 文本，便于阅读。
- **`target`。** `model` 是身份描述；`modelRef` 是操作收到的字符串模型 ID，ops 从 registry 解析出的模型填 `"<key>:<modelId>"`，直接传模型实例时缺省。不从 URL 或 `provider()` 推导。
- **headers。** 有序，保留重复项。`is_sensitive_key` 判定的头（Authorization、Cookie、各包的 key 头等）只留名字，`value` 缺省；URL 去掉 userinfo 和敏感 query。
- **请求体。** 存交给传输的原始字节。JSON 体里有敏感字段（`redact_json` 会改动它）时整段记 `forbidden: sensitive-field`，不存半脱敏的字节，这条录制因此不能 mock 回放（RFC-0041 §5 不让两个脱敏占位符判为相等）。
- **响应分块。** 按到达顺序记录，`offsetUs` 以这次交换开始为原点。单次交换上限 8 MiB，超出部分只计 `originalBytes`，该录制不可 mock 回放。
- **`capturePolicy`。** `metadata` 不存 input、请求体、响应分块（记 `forbidden: capture-policy`），仍记结构、时间、终态；`payload` 全存。`fixture` 导出不在本期（§9）。
- **`outcome` 不包 `Captured`。** `status` 是完成屏障的一部分，任何策略下都要有。`decisionResult`、`errorValue` 只在 `payload` 下存。这是对 RFC-0041 §4.1 `outcome: Captured<…>` 的简化，登记见 §9。
- **`replayability`。** 写入时计算，读者必须重算（`Recording::replayability()`），不信文件里的值。mock 需要：完整、传输已关闭、至少一个 model 交换、每个 model 交换的请求体和分块都已采集、有终态、未截断。live 需要 input 已采集。不满足的原因写成短字符串，如 `truncated:0.1.1`。
- **读取。** 先读 `schema` 字段，不是 3 就返回 `UnsupportedRecordingSchema { found }`；其余字段严格解析（`deny_unknown_fields`），base64 非法、callId 重复或为空都是 `InvalidFixture`。

完成屏障、有界队列、drop-newest、Ring/JSONL、flush/try_flush 的语义沿用现状（`recording.rs:1177` 起）。JSONL 队列容量从 1024 提到 RFC-0041 §4.2 建议的 4096。

## 4. 改后的调用栈

### 4.1 录制

```text
aimux::generate_text
 └ 调用观测接口：操作开始（callId、operation、target、input）          ← 接法由 RFC-0044 §4.1 的后续 RFC 定
   └ 每个模型调用 attempt：retry、router、MoA 设置 (seq, attempt, parent) ← RFC-0044 C3（§4.2）
     └ DiagnosticScope（task-local：call_id、model_call、交换计数器、observers）← RFC-0044 §4.2，provider-utils
       └ provider 编码 → helper（重定向、超时）
         └ Fetch 观察点（叶子 Fetch 之外最内层）                       ← RFC-0044 C3
           ├ ① 交换开始：请求原始字节、headers，取 Instant 作原点
           ├ ② 叶子 Fetch（ReqwestFetch / 用户的 fetch）
           ├ ③ 响应头：status、headers、responseHeadOffsetUs
           └ ④ body 每个分块：{offsetUs, bytes}；结束时终态 eof / error / abandoned
             → scope 的 observers 之一：aimux-devtools Recorder（schema 3 组装、完成屏障、JSONL / Ring），取代全局 RECORDER
 └ 调用观测接口：操作结束（outcome）
```

调用层事件由谁发出、接口叫什么，由 RFC-0044 §4.1 留给的后续 RFC 定。`(seq, attempt, parent)` 和交换序号已由 RFC-0044 §4.2 在 C3 给出，不靠调用层事件。本文对调用层事件的要求只有：带 `callId`；能区分 input 未采集（策略禁止）与丢失；操作结束和传输关闭两个信号都能送到 Recorder，供完成屏障使用。重定向的每一跳在观察点是独立的一次交换，回放时按跳匹配。

### 4.2 离线回放

```text
aimux_ops replay.offline { callId, provider?, matcher? }
 └ ReplaySet::get(callId)                          ← replay.create 时已用 ReplaySet::load 读入并校验
 └ ReplaySession::new(recording, Exact | Sequential)
   └ recording.replayability()                     ← 重算；mock 为 false → ReplayNotPossible{reasons}
 └ aimux_providers::create_offline_provider(name, session.fetch())
   │   name = provider 参数，缺省取 modelRef 的 key；都没有 → InvalidParams
   └ model = provider.language_model / decision_model(target.model.modelId)
       model.provider() ≠ target.model.provider → ReplayNotPossible
 └ replay_with_model / replay_decision_with_model（用录下的 input 重跑操作）
   └ provider 编码请求 → helper
     └ ReplayFetch::fetch(request)                 ← 叶子 Fetch，永不联网
       ├ ① 在候选交换里匹配（§5.2）；没有 → ReplayMiss，多个且响应不同 → ReplayAmbiguous
       ├ ② 原子标记已消费
       └ ③ 返回 FetchResponse：录下的 status、headers；body 按序吐出录下的分块，最后吐出录下的终态
     └ provider 解析器照常解析 → 结果
 └ session.verify_consumed()                       ← 有未消费的 model 交换 → ReplayUnusedExchange
```

### 4.3 live 回放

```text
aimux_ops replay.live { callId, target: model | registry }
 ├ model 目标：调用方给的模型句柄
 └ registry 目标：target.modelRef 按第一个 ":" 拆成 key 与 modelId，从 registry 取模型；没有 modelRef → InvalidParams
 └ replay_with_model / replay_decision_with_model → 真实网络
```

live 不再需要 `rebuild_provider`。没有 `modelRef` 的录制（直接传模型实例录的）只能用 model 目标回放，这是 RFC-0041 §9.1“直接 model 不可恢复时要求显式目标”的要求。

## 5. 传输回放接口

### 5.1 公开接口

```rust
pub struct ReplayLimits { pub max_line_bytes: usize /* 8 MiB */, pub max_total_bytes: usize /* 64 MiB */ }

pub struct ReplaySet { /* 已校验的录制 */ }
impl ReplaySet {
    pub fn load(jsonl: &str, limits: ReplayLimits) -> Result<Self, ReplayError>;
    pub fn recordings(&self) -> &[Recording];
    pub fn get(&self, call_id: &str) -> Result<&Recording, ReplayError>;
    /// 按 prompt 相似度选 scenario（演示、继续对话用）；结果标 approximate。
    pub fn select(&self, prompt: &LanguageModelPrompt, by: Similarity) -> Result<&Recording, ReplayError>;
}
pub enum Similarity { Score, Prefix }
pub enum ExchangeMatcher { Exact, Sequential }

pub struct ReplaySession { /* 一个 scenario 的消费状态 */ }
impl ReplaySession {
    pub fn new(recording: Recording, matcher: ExchangeMatcher) -> Result<Self, ReplayError>;
    pub fn fetch(&self) -> FetchFunction;
    pub fn verify_consumed(&self) -> Result<ReplaySummary, ReplayError>;
    pub fn cancel(&self);
    pub fn recording(&self) -> &Recording;
}
pub struct ReplaySummary { pub consumed: usize, pub total: usize }

pub enum ReplayError {
    UnsupportedRecordingSchema { found }, InvalidFixture { line, message },
    ReplayNotPossible { reasons }, OfflineConstructionUnavailable { provider },
    ReplayMiss { summary }, ReplayAmbiguous { exchange_ids }, ReplayOutOfOrder { exchange_id },
    ReplayUnusedExchange { exchange_ids }, ReplayCancelled, ReplayResourceLimit { what },
}
impl ReplayError { pub fn code(&self) -> &'static str; }
```

没有全局状态。一个 `ReplaySession` 只服务一个 scenario（RFC-0041 §3），并发测试各建各的。RFC-0041 §3 草拟的 `ReplaySession::load(recordings, options)` 在这里拆成 `ReplaySet::load` 加 `ReplaySession::new`：ops 的 `replay.create` 句柄对应一个集合，`inspect`、`dryRun`、`live` 都在集合上做，只有离线回放才需要消费状态。

### 5.2 匹配

- **候选。** 未消费的 model 交换。同一 (seq, attempt) 里编号更小的交换必须已消费；attempt > 1 时，同 seq 上一个 attempt 的交换必须已全部消费。这样重试和 MoA 的并发子调用都能按因果顺序消费，而不依赖运行时的完成顺序。
- **Exact（默认）。** 依次比较：method；URL 的 path 和 query（不比 scheme、host、port，因为离线组装不还原 base URL 设置）；请求体两边都是 JSON 时比较解析后的值（对象键序无关，数组顺序、数字值保留），否则逐字节比较；headers 按名字大小写无关地比较多重集合，只比录下了值的头，排除 `user-agent`、`content-length`、`host`、`accept-encoding`、`connection` 和敏感头。多个候选都匹配时，若它们的响应完全相同就取最早的，否则报 `ReplayAmbiguous`。
- **Sequential。** 取最早的候选，method 和 path 不同时报 `ReplayOutOfOrder`，不比请求体。用于 `select` 选出的近似 scenario。
- **失败。** `ReplayMiss` 的消息只含 method、path 和不同的部分（body，或者 header 名字），不含值、不含 prompt。引擎错误记在 session 里（先到者为准），`fetch` 返回 `FetchError::Other("<Code>: …")`。`Other` 不可重试，坏 fixture 不会白白耗掉操作的重试次数（RFC-0041 §8.3）。

RFC-0041 §7 的 Score、Prefix 在这里是选 scenario 的方式（`ReplaySet::select`），选中后用 Sequential 消费，结果标 `approximate`。它们不用来宣称协议回归通过，与 RFC-0041 一致。

### 5.3 响应

录下的 status 和 headers 原样返回，缺值的头不返回。body 是异步流，按序吐出录下的分块（`Immediate` 节奏：不等待录下的间隔，但每块之间都检查取消），最后吐出录下的终态：`eof` 正常结束；`error` 映射回对应的 `FetchError`；`abandoned` 映射为 `FetchError::Io`。没有响应只有错误终态的交换，`fetch` 直接返回那个错误。请求的 `AbortSignal` 和 `cancel()` 都会让挂起的 `fetch` 与正在读的 body 结束。录下的 429、5xx 是普通响应，走 provider 的错误解析和操作重试，不变成回放引擎错误。

## 6. 离线组装

RFC-0041 §6 的表由 provider 包实现：

```rust
// aimux-providers
pub const OFFLINE_API_KEY: &str = "aimux-replay-offline";
pub fn create_offline_provider(name: &str, fetch: FetchFunction) -> Result<Arc<dyn Provider>, AiMuxError>;
```

普通 API key 与无鉴权的包和 registry 预设：显式占位 key，不读环境变量，`fetch` 是唯一的传输。Bedrock / Anthropic-AWS 的 SigV4、Vertex ADC、Azure token resolver 本期没有离线配方，返回 `OfflineConstructionUnavailable`（§9）。鉴权实现不认识回放，回放只替换最内层的 transport。

## 7. ops、工具与绑定

| op | 改动 |
|---|---|
| `recorder.create` | `capturePolicy` 取 `metadata` 或 `payload`，替换现在唯一的 `full` |
| `replay.create` | `ReplaySet::load`，句柄持有集合；格式错误返回 §5.1 的错误 |
| `replay.capabilities` | 从 Future 改为 Current，返回 `{ http: true, webSocket: false, download: false, matchers, pacing }`，关闭 G-c |
| `replay.inspect` / `replay.dryRun` | 按 schema 3 返回 `target`、重算的 `replayability` 与原因 |
| `replay.offline`（新增） | §4.2；返回 `{ result, consumed, total }`，关闭 G-k |
| `replay.live` | registry 目标改用 `modelRef`（§4.3），删除 C6 移到 ops 的 `rebuild_provider` |

- **错误。** `ReplayError` 进现有的 `recording` 错误域，编码从 106 起按声明顺序排；payload 的 `name` 是 `code()`。codegen 的错误码范围（`tools/aimux-codegen/src/lib.rs:134`）同步扩大。
- **modelRef。** ops 的文本与 decision 处理函数在模型句柄带有唯一的 registry 引用（`ops/registry.rs` 的 `Metadata::Sources`）时，把 `"<key>:<modelId>"` 交给调用观测接口。
- **生成物。** C 头文件、八种绑定的 ops 常量和 DTO、`schema/`、Web 控制台的 TS 类型全部由 `cargo run -p aimux-codegen` 生成，CI 的 `--check` 把关；`contract-tests/dto/Recording/001.json` 由 `UPDATE_DTO_VECTORS=1 cargo test -p aimux-ops` 重写成 schema 3。
- **手写绑定代码。** 只有测试和 README 引用录制与回放（Python `tests/gil_ops.py`、`test_ops_integration.py`；Node `__test__/contract.native.ts`、`native_scenarios.rs`；Go `wire_test.go`；Java、Kotlin 的错误码测试），换成 schema 3 数据和新 op。绑定 API 本身不加新方法：新 op 经已有的通用调用入口可用。
- **aimux-replay。** `--mock` 用 `ReplaySet` + `ReplaySession` + `create_offline_provider`；live 用 `--provider` 或 `modelRef`，都没有就报错；`--dry-run` 打印重算的 replayability。
- **aimux-web。** mock 模式按聊天输入用 `ReplaySet::select(…, Score)` 选录制，用 Sequential 消费，响应标 `approximate`；`/api/replay` 的 provider 来自请求字段或 `modelRef`，Replay 页面把 `target.model.provider` 的第一段预填进可编辑的 provider 输入框，只是界面默认值。
- **依赖回放的 provider 测试。** `aimux-providers/tests/common/replay.rs` 用 wiremock 喂 schema 2 交换，改为用 `ReplaySession` 当 provider 的 `fetch`。`ReplaySession` 在 `aimux-devtools`，而 providers 与 devtools 互不依赖（RFC-0044 §8 第 1 条），所以这些测试搬到 `aimux-ops` 的集成测试（ops 两者都依赖），和 RFC-0046 §5.9 第 9 条（[PR #294](https://github.com/arcships/aimux/pull/294)）对录制、回放类 provider 测试的安排一致。

## 8. 与 RFC-0041、RFC-0044 的关系

| 题目 | RFC-0041（已合并） | RFC-0044 | 本文 |
|---|---|---|---|
| 录制格式 | schema 3 字段与约束（§4） | 不改格式 | 给出完整形状与读取规则（§3） |
| 采集位置 | `transport_leaf`（§4） | Fetch 观察点 + `DiagnosticScope`（C3） | 建在 C3 的观察点上（§4.1）；待讨论 Q1 |
| 调用层事件 | operation/child/attempt 事件（§3） | (seq, attempt, parent) 与交换序号在 C3 的 `DiagnosticScope`（§4.2）；操作开始、结束事件由后续调用观测 RFC 定（§4.1） | 引用 §4.2，不另定；只列操作事件的要求（§4.1）；待讨论 Q3 |
| 回放引擎 | `replay_fetch`、匹配、错误码（§3、§7、§8） | 归 `aimux-devtools`（C6） | 接口（§5）；crate 归属不在本文定 |
| 离线组装 | 配方由 provider 包/descriptor 提供（§3、§6） | — | `create_offline_provider`（§6） |
| `rebuild_provider` | 删除（§2、§12） | C6 时暂移到 ops，传输回放后删除 | C6 移入 ops 之后，由 §10 第 3 步随 live 改用 `modelRef` 在 ops 里删除 |
| 治理命令、证据台账 | G4、G5 | — | 不在本期 |

RFC-0044 C6 先把 `rebuild_provider` 移到 ops，§10 第 3 步排在 C6 之后，在 ops 里把它删掉。

## 9. 本期不做与登记差异

- WebSocket 录制与回放：现在被录制的三个操作（generate_text、stream_text、decide）都不走 WebSocket，schema 3 本期不写 `wsSessions`。以后加的是可选字段，不改版本号（RFC-0041 §4）。
- 下载与轮询回放：input 里有 URL 文件或图片的录制，离线回放返回 `ReplayNotPossible`，以保证不联网。
- 流式增量 sanitizer（RFC-0041 §5）：`payload` 下响应分块原样存。请求 JSON 体只要含敏感字段，整段记 `forbidden`。
- SigV4、Vertex ADC、Azure token 的离线配方（§6）。
- `Recorded`、`Scaled` 节奏和虚拟时钟：只做 `Immediate`。
- `fixture` 导出策略、`probe` / `diff` / `session` 治理命令、证据台账（RFC-0041 G4、G5）。
- `outcome` 不包 `Captured`（§3）。
- 采集上限：单次交换 8 MiB，JSONL 队列 4096 个事件，沿用 RFC-0041 §4.2 的建议值，没有压测数据。

已有真实录制 `aimux-providers/tests/fixtures/jev_systemone_live.jsonl`（schema 2，2026-10-06 经维护者认可的 live 采集）：见 Q4。

## 10. 实施计划

| 顺序 | PR | 内容 | 依赖 | 负责 |
|---|---|---|---|---|
| 0 | 本 RFC、RFC-0044 | 维护者接受 | — | eric |
| 1 | RFC-0044 C3 | Fetch 观察点、`DiagnosticScope`；provider-utils 不再读录制上下文 | RFC-0044 | 包拆分 |
| 2 | 调用观测 RFC → RFC-0044 C5、C6 | 调用层事件接口；新建 `aimux-devtools`，录制、回放搬入 | 1 | 包拆分 |
| 3 | schema 3 写入侧 | §3 的类型与读取规则，观察者写 schema 3，删除 schema 1/2 读取、`ProviderRecord`、`MockReplayModel`，以及 C6 移到 ops 的 `rebuild_provider`；ops `recorder.create` 策略、`replay.create/inspect/dryRun/live` 改为 schema 3；aimux-replay 与 Web 的 mock 模式在此 PR 中暂时去掉，并在 PR 描述登记，由 4 补回；生成物与 DTO 向量 | 2 | 本线程 |
| 4 | 传输回放 | §5、§6；ops `replay.offline`、`replay.capabilities`；aimux-replay `--mock` 与 Web mock 模式；依赖回放的 provider 测试在 ops 集成测试里改用 `ReplaySession`（§7）；Jev 录制按 Q4 处理；生成物 | 3 | 本线程 |
| 5 | 文档登记 | 关闭 RFC-0039 D12、G-b、G-c、G-k；对齐文档 §9.1 相应各行；§9 的未做项写进已知差距 | 4 | 本线程 |
| 6 | 发版 | 首个写 schema 3 的发布冻结新增必需字段（RFC-0041 §4） | 5 | — |

3、4、5 都要在发版前合入（RFC-0039 D12），每个 PR 只做一个方向，描述先给体系调用栈，再给本 PR 的改动。3 和 4 之间 master 上没有 mock 回放，这是两个未发布提交之间的短暂缺口，而不是过渡代码。如果维护者希望没有这个缺口，可以把 3 和 4 合成一个 PR，代价是 PR 变大。

## 11. 待讨论

**Q1 采集位置。** 原提案：采集留在 `aimux-provider-utils/src/http.rs` 把 `FetchRequest` 交给传输的那一处（`http.rs:375`），由 http.rs 继续持有录制上下文。这个方案在本地分支（未推送）上已经实现，`aimux-core`、`aimux-provider-utils` 的测试通过。RFC-0044 改为在 Fetch 链最内层设观察点：它包住用户或默认的叶子 Fetch，位于鉴权之后、真正发出之前，和回放用的是同一个接口。观察点用 `DiagnosticScope` 关联调用，provider-utils 不再认识录制。本文按 RFC-0044 写 §4.1。

两种位置看到的请求不同，供判断：`http.rs:375` 交出的请求还没经过各包的装饰层，即 `SigV4Fetch`、`AnthropicAwsFetch`、Azure 的 `BearerTokenFetch`，以及 `shared::package_fetch` 的 `UserAgentFetch`。而回放 Fetch 作为 provider 的 `fetch` 设置传入，处在这些装饰层里面。所以按原提案录下的请求，与回放时收到的请求在签名头、`authorization`、`user-agent` 上不一致。§5.2 的 Exact 排除了 `user-agent` 和敏感头，可以掩盖前两项差别，但 SigV4 的 `x-amz-date`、`x-amz-content-sha256` 这类头仍会对不上。观察点放在叶子上时，两边看到的是同一份请求。原提案保留在这里待维护者判断。如果 RFC-0044 不采用观察点，就回到原提案，并把上述头加进 §5.2 的排除集合。

**Q2 schema 类型与 Recorder 的归属。** 原提案：在 `aimux-devtools` 建立之前，schema 3 类型、Recorder、Ring/JSONL 留在 `aimux-core::recording`，回放引擎放在 `aimux-provider-utils` 的独立 `replay` 模块，以后整体搬走。RFC-0044 §3.1 把它们放进 `aimux-devtools`，provider-utils 只放观察记录类型。本文按 RFC-0044 的顺序排（§10 先 C6 后 3）。是否允许先落在临时位置再搬，由维护者决定。

**Q3 调用层事件。** 原提案：沿用 task-local 的 `RecordingContext`，由它分配 `(seq, attempt)`：根是 seq 0，`child(step)` 取下一个 seq 并记下 parent，`start_attempt()` 递增本 seq 的 attempt。RFC-0044 §4.2 已在 C3 定下 (seq, attempt, parent) 由 `aimux` 的 retry 循环和 router、MoA、composite 设置，和原提案的分配规则相同，这部分不再待定。剩下的是操作开始、结束等调用层事件的接法，RFC-0044 留给后续 RFC；本文只列要求（§4.1），不预先定接口。

**Q4 Jev 真实录制。** 两种做法。(a) 维护者用真实 key 重新录一份 schema 3。(b) 用确定性的一次性映射转换现有文件：非流式 JSON 响应各记一个分块，`offsetUs` 取录下的延迟，`[REDACTED]` 头去掉值；转换脚本不进仓库，在 PR 里列出供审核。RFC-0041 §2 不允许偷偷转存历史流量。(b) 是公开转换，是否接受由维护者定。原提案是 (b)。

**Q5 3 与 4 是否合并。** 见 §10 末段。原提案是分开。
