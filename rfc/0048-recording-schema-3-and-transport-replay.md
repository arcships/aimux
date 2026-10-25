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
> **Related**: [对齐文档](../docs/aisdk-architecture-alignment.md) §4.3、§4.4、§8.3、§9.1；[RFC-0039](0039-ops-bindings-contract.md) 登记项 D12、G-b、G-c、G-k（[实现说明](../docs/rfc-0039-implementation.md)）；[RFC-0042](0042-v4-type-boundaries.md)（DTO 与 codegen）；[RFC-0044](0044-crate-layering-and-dependency-inversion.md)（#279：crate 拆分、Fetch 观察点、`DiagnosticScope`、`aimux-devtools`）；[RFC-0046](0046-provider-protocol-abstraction.md)（#294：§5.9 的录制、回放类测试位置）。
>
> **评审结论**：维护者 2026-10-21 批准，§11 的问题已定（[评审](https://github.com/arcships/aimux/pull/325#pullrequestreview-5594462993)、[决定](https://github.com/arcships/aimux/pull/325#issuecomment-6307035442)）；2026-10-20 评审的七条意见在 §3、§4.3、§5、§7、§10 处理。

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
  target: { modelRef?: { namespace, key, method, modelId }, model: { provider, modelId } },
  sessionId?, sessionStep?,
  capturePolicy: "metadata" | "payload",
  input: Captured<{ prompt, options }>,
  modelCalls: [ { seq, attempt, parent?: { seq, attempt }, stepLabel?, model: { provider, modelId } } ],
  exchanges: [ Exchange ],
  outcome: { status, finishReason?, usage?, error?: { name, code }, errorDetail: Captured<{ message, value }>, decisionResult: Captured<…> },
  complete, transportClosed,
  replayability: { mock, live, reasons: [string] }
}
Captured<T> = { type: "captured", value: T } | { type: "forbidden", reason } | { type: "missing", reason }
Exchange {
  exchangeId: "<seq>.<attempt>.<index>",
  phase: "prepare" | "model" | "auxiliary",
  modelCall: { seq, attempt },                     // 所在 DiagnosticScope 的 model_call，操作本身是 seq 0
  exchangeIndex,                                   // 同一 (seq, attempt) 内从 1 递增
  boundary: "transport-leaf",
  request: { method, url, headers: [ { name, value? } ], body: Captured<base64> },
  response?: { status, headers: [ { name, value? } ] },
  chunks: Captured<[ { offsetUs, data: base64 } ]>,
  timing: { responseHeadOffsetUs?, terminalOffsetUs? },
  terminal?: { type: "eof" } | { type: "error", kind: "connect"|"timeout"|"aborted"|"io"|"other", message: Captured<string> } | { type: "abandoned" },
  truncated?: { originalBytes, capturedBytes }
}
```

- **`seq`、`attempt` 与 `exchangeIndex`。** 取自 RFC-0044 §4.2 的 `DiagnosticScope`（[PR #279](https://github.com/arcships/aimux/pull/279)）：`model_call { seq, attempt, parent }` 和每个 (seq, attempt) 内的交换计数器，分配规则以那里为准，本文不另定。`exchangeId` 是这三个值的拼接。scope 总有 `model_call`，所以每个交换都带 `modelCall`，`phase` 为 `prepare`、`auxiliary` 的交换也一样；这比 RFC-0041 §4.1 的“可选 model_call”更严，`modelCalls` 因此包含操作本身的 seq 0。`stepLabel` 保留现在的 `router[0]:…` 文本，便于阅读。`model` 是这次模型调用的身份（模型的 `provider()` 与 `modelId()`），router、MoA 的每个子调用各记各的，离线回放靠它为每个子调用组装模型（§4.2）。
- **`target`。** `model` 是身份描述；`modelRef` 是 RFC-0041 §4.1 的 registry key、模型 ID 与命名方法，形状就是 ops `registry.languageModel` 等 op 已经收的结构化引用（`aimux-ops/src/ops/registry.rs:138` 的 `ModelRef`：`namespace`、`key`、`method`、`modelId`），不拼成 `"<key>:<modelId>"` 字符串：registry 的分隔符可配置（`aimux-core/src/provider_registry.rs:31`），key 和模型 ID 里都可能出现分隔符，拼接后拆不回来，也会丢掉 namespace 和 method。直接传模型实例时缺省。不从 URL 或 `provider()` 推导。
- **headers。** 有序，保留重复项。`is_sensitive_key` 判定的头（Authorization、Cookie、各包的 key 头等）只留名字，`value` 缺省；URL 去掉 userinfo 和敏感 query。
- **input。** `redact_json` 会改动的 input（prompt 或 options 里有敏感字段）整段记 `forbidden: sensitive-field`，不存脱敏后的副本。这条录制因此不能 live 回放：live 会把 input 原样发出去，脱敏副本里的 `[REDACTED]` 会被当成真实值发给厂商。
- **请求体。** 存交给传输的原始字节。JSON 体里有敏感字段（`redact_json` 会改动它）时整段记 `forbidden: sensitive-field`，不存半脱敏的字节，这条录制因此不能 mock 回放（RFC-0041 §5 不让两个脱敏占位符判为相等）。
- **响应分块。** 按到达顺序记录，`offsetUs` 以这次交换开始为原点。分块在 Recorder 里按交换缓存到终态（完成屏障本来就要等终态），进入 writer 队列之前做 RFC-0041 §5 的资格判定：`content-type` 是 JSON 时整段解析后检查；是 SSE（`text/event-stream`）时按 SSE 规则拆行，注释行（`:` 开头）、`event:`、`id:`、`retry:`、空的 `data:` 和 `data: [DONE]`（OpenAI 及兼容接口的结束标记）是控制行，不解析，其余每个事件的 `data` 拼起来按 JSON 解析后检查。凡是 `redact_json` 会改动的就整段记 `forbidden: sensitive-field`；其他媒体类型、解析失败或无法判定的，同样整段 `forbidden`（`unknown-media` / `unparsed`）。所以错误响应体或流里带回的凭证不会进队列和文件。本期没有增量 sanitizer（§9），判定前的字节只在 Recorder 的有界内存里，计入 RFC-0041 §4.2 的 64 MiB 在途预算。
- **大小。** writer 和 reader 用同一个单行上限 `RECORDING_LINE_LIMIT`（8 MiB，RFC-0041 §4.2、§5），即 `ReplayLimits::max_line_bytes` 的默认值。采集时按 base64 后的大小记账：一条录制里所有请求体和响应分块编码后的总量超过 6 MiB（给 JSON 结构留余量）时，后续的 body 只计 `originalBytes`、记 `truncated`；writer 序列化后若仍超过单行上限，把 `input` 和全部 body 改记 `missing: size-limit` 再写。因此 writer 写出的每一行 `ReplaySet::load` 都能读回，超限的录制只是不可回放，不会变成读不了的坏行。
- **`capturePolicy`。** `metadata` 不存 input、请求体、响应分块（记 `forbidden: capture-policy`），仍记结构、时间、终态；`payload` 全存。`fixture` 导出不在本期（§9）。
- **错误文本。** 错误的文字会带出 provider 响应体、prompt 片段和 URL，所以和正文同一套规则（RFC-0041 §5），不当作元数据随手存：
  - `outcome.error` 只有 `name`（AI SDK 错误名）和 `code`（ops 错误码），任何策略下都存，供 `status` 与完成屏障使用。
  - `outcome.errorDetail` 是错误的 message 和 AI SDK JSON。`metadata` 下记 `forbidden: capture-policy`；`payload` 下 JSON 先过 `redact_json`，会被改动就整段 `forbidden: sensitive-field`，message 跟着一起禁（message 常常就是响应体里的文字）；JSON 里的 `url` 按请求 URL 的规则去掉 userinfo 和敏感 query。
  - `terminal.error.message` 是传输错误自己的文字（reqwest 的错误会带 URL）。`metadata` 下 `forbidden: capture-policy`，`payload` 下存去掉 userinfo 和敏感 query 之后的文字；`kind` 任何策略下都存，回放只靠 `kind` 还原 `FetchError`。
  - `decisionResult` 同 `errorDetail`：`metadata` 下禁止，`payload` 下过 `redact_json`。
- **`outcome` 只部分包 `Captured`。** `status`、`finishReason`、`usage`、`error` 是结构信息，任何策略下都要有；会带正文的 `errorDetail`、`decisionResult` 用 `Captured`。RFC-0041 §4.1 写的是整个 `outcome: Captured<…>`，这里只包会带正文的部分，登记见 §9。
- **`replayability`。** 写入时计算，读者必须重算（`Recording::replayability()`），不信文件里的值。mock 需要：完整、传输已关闭、至少一个 model 交换、每个 model 交换的请求体和分块都已采集、有终态、未截断。`prepare`、`auxiliary` 交换没采集不影响 mock（§5.2 不让它们挡住 model 交换）。live 需要 input 是 `captured`（`forbidden: sensitive-field` 的 input 不行，见上）。不满足的原因写成短字符串，如 `truncated:0.1.1`。
- **读取。** 先读 `schema` 字段，不是 3 就返回 `UnsupportedRecordingSchema { found }`；其余字段严格解析（`deny_unknown_fields`），base64 非法、callId 重复或为空都是 `InvalidFixture`。结构校验也在读取时做，违反任一条都是 `InvalidFixture`（RFC-0041 §4.1、§5）：
  - `modelCalls` 的 `(seq, attempt)` 唯一，`parent` 指向已有条目且无环；
  - 每个交换的 `modelCall` 指向 `modelCalls` 里的条目；`phase` 只说明交换的用途；
  - `exchangeId` 等于 `<seq>.<attempt>.<exchangeIndex>`，`exchangeIndex` 在同一 `modelCall` 内从 1 严格递增；
  - 每个交换都有 `terminal` 和 `timing.terminalOffsetUs`，没有响应的交换只能是 `error` 终态；`responseHeadOffsetUs ≤ 每个分块的 offsetUs ≤ terminalOffsetUs`，分块之间不逆序；
  - `complete: true` 时 `outcome.status` 必须是 `success`、`error`、`cancelled` 之一，`transportClosed` 必须为 true。

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
aimux_ops replay.session { replay, callId, matcher? }                 新 op，返回会话句柄
 └ ReplaySet::get(callId)                          ← replay.create 时已用 ReplaySet::load 读入并校验
 └ ReplaySession::new(recording, Exact | Sequential)
   └ recording.replayability()                     ← 重算；mock 为 false → ReplayNotPossible{reasons}

aimux_ops replay.offlineModel { session, seq?, provider? }            新 op，返回模型句柄
 └ call = modelCalls 里 seq 对应的条目（缺省 seq 0，即操作本身的目标）
 └ aimux_providers::create_offline_provider(name, session.fetch())
   │   name = provider 参数；缺省按 call.model.provider 在包的离线表里精确查（§6）；查不到 → InvalidParams，要求显式 provider
   └ model = provider.language_model / decision_model(call.model.modelId)
       model.provider() ≠ call.model.provider → ReplayNotPossible

router、MoA：宿主对每个子调用 seq 取 replay.offlineModel，再用已有的 composite.router / composite.moa 组装
（组合策略不在录制里，RFC-0041 §9.1 与对齐文档 §4.4 要求由宿主给出组合定义）

aimux_ops replay.offline { session, model? }                          新 op
 └ model 缺省 = replay.offlineModel(session, seq 0)；录制有子调用（modelCalls 里有 parent）而没给 model → ReplayNotPossible{"composite-needs-model"}
 └ replay_with_model / replay_decision_with_model（用录下的 input 重跑操作）
   └ provider 编码请求 → helper
     └ ReplayFetch::fetch(request)                 ← 叶子 Fetch，永不联网；所有子模型共用这一个会话
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
 └ registry 目标：target.modelRef 的 namespace 必须等于 registry 句柄的 namespace，再按 key、method、modelId 精确取模型（同 ops 的 resolve_model，registry.rs:393，不拼接不拆分）；没有 modelRef 或 namespace 不符 → InvalidParams
 └ replay_with_model / replay_decision_with_model → 真实网络
```

live 不再需要 `rebuild_provider`。没有 `modelRef` 的录制（直接传模型实例录的）只能用 model 目标回放，这是 RFC-0041 §9.1“直接 model 不可恢复时要求显式目标”的要求。

## 5. 传输回放接口

### 5.1 公开接口

```rust
pub const RECORDING_LINE_LIMIT: usize = 8 * 1024 * 1024; // writer 与 reader 共用（§3）
pub struct ReplayLimits { pub max_line_bytes: usize /* RECORDING_LINE_LIMIT */, pub max_total_bytes: usize /* 64 MiB */ }

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
    ReplayBeyondRecording { exchange_id },
}
impl ReplayError { pub fn code(&self) -> &'static str; }
```

没有全局状态。一个 `ReplaySession` 只服务一个 scenario（RFC-0041 §3），并发测试各建各的。同一个会话的 `fetch()` 可以交给多个离线 provider：router、MoA 的子模型共用一个会话，按 §5.2 的因果顺序各取各的交换。RFC-0041 §3 草拟的 `ReplaySession::load(recordings, options)` 在这里拆成 `ReplaySet::load` 加 `ReplaySession::new`：ops 的 `replay.create` 句柄对应一个集合，`inspect`、`dryRun`、`live` 都在集合上做，只有离线回放才需要消费状态。

### 5.2 匹配

- **候选。** 未消费的 model 交换。同一 (seq, attempt) 里编号更小的 model 交换必须已消费；attempt > 1 时，同 seq 上一个 attempt 的 model 交换必须已全部消费。这样重试和 MoA 的并发子调用都能按因果顺序消费，而不依赖运行时的完成顺序。顺序约束只在 `phase: "model"` 的交换之间：`prepare`、`auxiliary` 交换（下载、token 换取等）可能根本不经过回放 Fetch（离线组装不换 token，下载本期不回放，§9），所以它们不挡后面的 model 交换；请求到了回放 Fetch 时，它们在同一 (seq, attempt) 的未消费非 model 交换里按 Exact 匹配，不排序，也不计入 `verify_consumed`。
- **Exact（默认）。** 依次比较：method；URL 的 path 和 query（不比 scheme、host、port，因为离线组装不还原 base URL 设置）；请求体两边都是 JSON 时比较解析后的值（对象键序无关，数组顺序、数字值保留），否则逐字节比较；headers 按名字大小写无关地比较多重集合，只比录下了值的头，排除 `user-agent`、`content-length`、`host`、`accept-encoding`、`connection` 和敏感头。多个候选都匹配时，若它们的响应完全相同就取最早的，否则报 `ReplayAmbiguous`。
- **Sequential。** 取最早的候选，method 和 path 不同时报 `ReplayOutOfOrder`，不比请求体。用于 `select` 选出的近似 scenario。
- **失败。** `ReplayMiss` 的消息只含 method、path 和不同的部分（body，或者 header 名字），不含值、不含 prompt。引擎错误记在 session 里（先到者为准），`fetch` 返回 `FetchError::Other("<Code>: …")`。`Other` 不可重试，坏 fixture 不会白白耗掉操作的重试次数（RFC-0041 §8.3）。

RFC-0041 §7 的 Score、Prefix 在这里是选 scenario 的方式（`ReplaySet::select`），选中后用 Sequential 消费，结果标 `approximate`。它们不用来宣称协议回归通过，与 RFC-0041 一致。

### 5.3 响应

录下的 status 和 headers 原样返回，缺值的头不返回。body 是异步流，按序吐出录下的分块（`Immediate` 节奏：不等待录下的间隔，但每块之间都检查取消），最后吐出录下的终态：`eof` 正常结束；`error` 按录下的 `kind` 映射回对应的 `FetchError`，可重试性不变（connect、timeout、io 可重试）。接下来要不要重试由操作的重试循环决定，它用的是录下的 input（含 `maxRetries`），所以和录制时走同一条路：要么录制里有下一个 attempt，要么这就是最后一次，循环耗尽后返回和录制时一样的 RetryError。循环如果要一个录制里没有的 attempt（比如宿主改了重试设置），那个请求在 §5.2 找不到候选，按 `ReplayMiss` 处理，不可重试；`abandoned` 表示原操作读到这里就不再读了，回放时同一个操作也应该在这里停下，所以吐完录下的分块后如果还有人继续读，body 返回 `FetchError::Other("ReplayBeyondRecording: …")` 并把引擎错误记为 `ReplayBeyondRecording`。`Other` 不可重试（`aimux-provider-utils/src/fetch.rs:188`），不会触发录制里没有的重试，也就不会改变录下的 attempt 序列。没有响应只有错误终态的交换，`fetch` 直接返回那个错误。请求的 `AbortSignal` 和 `cancel()` 都会让挂起的 `fetch` 与正在读的 body 结束。录下的 429、5xx 是普通响应，走 provider 的错误解析和操作重试，不变成回放引擎错误。

## 6. 离线组装

RFC-0041 §6 的表由 provider 包实现：

```rust
// aimux-providers
pub const OFFLINE_API_KEY: &str = "aimux-replay-offline";
pub fn create_offline_provider(name: &str, fetch: FetchFunction) -> Result<Arc<dyn Provider>, AiMuxError>;
/// 录下的 `model.provider`（如 "openai.chat"）属于哪个包：每个包在离线配方旁登记它的模型会报告的 provider id，精确匹配。
pub fn offline_provider_name(model_provider: &str) -> Option<&'static str>;
```

普通 API key 与无鉴权的包和 registry 预设：显式占位 key，不读环境变量，`fetch` 是唯一的传输。包名不从 `modelRef.key` 取：registry 的 key 是宿主起的别名（如 `work`），不一定是包名；也不按 `.` 截断 `provider()`：`offline_provider_name` 查的是各包自己登记的完整 id，查不到时由调用方显式给出。Bedrock / Anthropic-AWS 的 SigV4、Vertex ADC、Azure token resolver 本期没有离线配方，返回 `OfflineConstructionUnavailable`（§9）。鉴权实现不认识回放，回放只替换最内层的 transport。

## 7. ops、工具与绑定

| op | 改动 |
|---|---|
| `recorder.create` | `capturePolicy` 取 `metadata` 或 `payload`，替换现在唯一的 `full` |
| `replay.create` | `ReplaySet::load`，句柄持有集合；格式错误返回 §5.1 的错误 |
| `replay.capabilities` | 从 Future 改为 Current，返回 `{ http: true, webSocket: false, download: false, matchers, pacing }`，关闭 G-c |
| `replay.inspect` / `replay.dryRun` | 按 schema 3 返回 `target`、重算的 `replayability` 与原因 |
| `replay.session`、`replay.offlineModel`（新增） | §4.2：会话句柄与按模型调用组装的离线模型句柄 |
| `replay.offline`（新增） | §4.2；返回 `{ result, consumed, total }`，关闭 G-k |
| `replay.live` | registry 目标改用 `modelRef`（§4.3），删除 C6 移到 ops 的 `rebuild_provider` |

- **错误。** `ReplayError` 进现有的 `recording` 错误域，编码从 106 起按声明顺序排；payload 的 `name` 是 `code()`。codegen 的错误码范围（`tools/aimux-codegen/src/lib.rs:134`）同步扩大。
- **modelRef。** ops 的文本与 decision 处理函数在模型句柄带有唯一的 registry 引用（`ops/registry.rs` 的 `Metadata::Sources`，元素就是 `ModelRef`）时，把这个结构原样交给调用观测接口；有多个引用（wrap、router 组合出来的句柄）时缺省。
- **生成物。** C 头文件、八种绑定的 ops 常量和 DTO、`schema/`、Web 控制台的 TS 类型全部由 `cargo run -p aimux-codegen` 生成，CI 的 `--check` 把关；`contract-tests/dto/Recording/001.json` 由 `UPDATE_DTO_VECTORS=1 cargo test -p aimux-ops` 重写成 schema 3。
- **手写绑定代码。** 只有测试和 README 引用录制与回放（Python `tests/gil_ops.py`、`test_ops_integration.py`；Node `__test__/contract.native.ts`、`native_scenarios.rs`；Go `wire_test.go`；Java、Kotlin 的错误码测试），换成 schema 3 数据和新 op。绑定 API 本身不加新方法：新 op 经已有的通用调用入口可用。
- **aimux-replay。** `--mock` 用 `ReplaySet` + `ReplaySession` + `create_offline_provider`；live 用 `--provider` 或 `modelRef`，都没有就报错；`--dry-run` 打印重算的 replayability。
- **aimux-web。** mock 模式按聊天输入用 `ReplaySet::select(…, Score)` 选录制，用 Sequential 消费，响应标 `approximate`；`/api/replay` 的 provider 来自请求字段或 `modelRef`，Replay 页面把 `target.model.provider` 的第一段预填进可编辑的 provider 输入框，只是界面默认值。
- **依赖回放的 provider 测试。** `aimux-providers/tests/common/replay.rs` 用 wiremock 喂 schema 2 交换，改为用 `ReplaySession` 当 provider 的 `fetch`。`ReplaySession` 在 `aimux-devtools`，而 providers 与 devtools 互不依赖（RFC-0044 §8 第 1 条），所以这些测试搬到 `aimux-ops` 的集成测试（ops 两者都依赖），和 RFC-0046 §5.9 第 9 条（[PR #294](https://github.com/arcships/aimux/pull/294)）对录制、回放类 provider 测试的安排一致。

## 8. 与 RFC-0041、RFC-0044 的关系

| 题目 | RFC-0041（已合并） | RFC-0044 | 本文 |
|---|---|---|---|
| 录制格式 | schema 3 字段与约束（§4） | 不改格式 | 给出完整形状与读取规则（§3） |
| 采集位置 | `transport_leaf`（§4） | Fetch 观察点 + `DiagnosticScope`（C3） | 建在 C3 的观察点上（§4.1，Q1 已定） |
| 调用层事件 | operation/child/attempt 事件（§3） | (seq, attempt, parent) 与交换序号在 C3 的 `DiagnosticScope`（§4.2）；操作开始、结束事件由后续调用观测 RFC 定（§4.1） | 引用 §4.2，不另定；只列操作事件的要求（§4.1） |
| 回放引擎 | `replay_fetch`、匹配、错误码（§3、§7、§8） | 归 `aimux-devtools`（C6） | 接口（§5）；crate 归属不在本文定 |
| 离线组装 | 配方由 provider 包/descriptor 提供（§3、§6） | — | `create_offline_provider`（§6） |
| `rebuild_provider` | 删除（§2、§12） | C6 时暂移到 ops，传输回放后删除 | C6 移入 ops 之后，由 §10 第 3 步随 live 改用 `modelRef` 在 ops 里删除 |
| 治理命令、证据台账 | G4、G5 | — | 不在本期 |

RFC-0044 C6 先把 `rebuild_provider` 移到 ops，§10 第 3 步排在 C6 之后，在 ops 里把它删掉。

## 9. 本期不做与登记差异

- WebSocket 录制与回放：现在被录制的三个操作（generate_text、stream_text、decide）都不走 WebSocket，schema 3 本期不写 `wsSessions`。以后加的是可选字段，不改版本号（RFC-0041 §4）。
- 下载与轮询回放：input 里有 URL 文件或图片的录制，离线回放返回 `ReplayNotPossible`，以保证不联网。
- 流式增量 sanitizer（RFC-0041 §5）：响应分块在缓存到终态后整段判定（§3），含敏感字段、媒体类型未登记或无法判定的整段记 `forbidden`；请求 JSON 体只要含敏感字段，整段记 `forbidden`。
- SigV4、Vertex ADC、Azure token 的离线配方（§6）。
- `Recorded`、`Scaled` 节奏和虚拟时钟：只做 `Immediate`。
- `fixture` 导出策略、`probe` / `diff` / `session` 治理命令、证据台账（RFC-0041 G4、G5）。
- `outcome` 只部分包 `Captured`（§3）。第 3 步落地时在 RFC-0041 §4.1 加一行修订引用本文，与 RFC-0039 §0.1.7、§7.1 的修订同样写法。
- 与对齐文档 §4.3 的差异，第 3 步同步改对齐文档（§4.3 第 875 行附近、§4.4 第 968 行附近、I5 行）：
  - `target.modelRef`：对齐文档 2026-10-13 修订为字符串模型 ID（如 `work:gpt-4o`）。本文改为结构化的 ops `ModelRef`（§3），理由是分隔符可配置、拼接拆不回来（维护者 2026-10-20 评审）。
  - `modelCalls`：对齐文档每个模型调用有 `model { provider, model_id, modality }`、`started_at`、`outcome`。本文保留 `model { provider, modelId }`（`modality` 由 `operation` 决定，不重复记）；不记 `started_at` 和逐调用 `outcome`，时间在各交换的 `timing`，每次调用的结果由它的交换终态和操作的 `outcome` 给出。
- 采集上限：单行 8 MiB、每条录制 base64 后 6 MiB 的 body 预算（§3），64 MiB 在途、JSONL 队列 4096 个事件，沿用 RFC-0041 §4.2 的建议值，没有压测数据。

已有真实录制 `aimux-providers/tests/fixtures/jev_systemone_live.jsonl`（schema 2，2026-10-06 经维护者认可的 live 采集）：按 Q4 转换（§10、§11）。

## 10. 实施计划

| 顺序 | PR | 内容 | 依赖 | 负责 |
|---|---|---|---|---|
| 0 | 本 RFC、RFC-0044 | 维护者接受 | — | eric |
| 1 | RFC-0044 C3 | Fetch 观察点、`DiagnosticScope`；provider-utils 不再读录制上下文 | RFC-0044 | 包拆分 |
| 2 | 调用观测 RFC → RFC-0044 C5、C6 | 调用层事件接口；新建 `aimux-devtools`，录制、回放搬入 | 1 | 包拆分 |
| 3 | schema 3 写入侧与读取 | §3 的类型、读取与结构校验，`ReplaySet`（`load`、`get`、`select`，§5.1），观察者写 schema 3；删除 schema 1/2 读取、`ProviderRecord`、`MockReplayModel`、`MockDecisionReplayModel`（`aimux-core/src/decision_replay.rs:33`），以及 C6 移到 ops 的 `rebuild_provider`；ops `recorder.create` 策略，`replay.create/inspect/dryRun/live` 改用 `ReplaySet`；aimux-web 的录制导入导出（`api/traces.rs:74` 起）改读写 schema 3；aimux-replay 与 Web 的 mock 模式在此 PR 中暂时去掉，并在 PR 描述登记，由 4 补回；在 RFC-0041 §4.1 加 `outcome` 的修订行、按 §9 改对齐文档 §4.3 的 `modelRef` 与 `modelCalls`；生成物与 DTO 向量；PR 描述附 schema 2 fixture 的去向清单（见下） | 2 | 本线程 |
| 4 | 传输回放 | `ReplaySession`、`ReplayFetch`（§5）、§6；ops `replay.session`、`replay.offlineModel`、`replay.offline`、`replay.capabilities`；aimux-replay `--mock` 与 Web mock 模式；依赖回放的 provider 测试在 ops 集成测试里改用 `ReplaySession`（§7），Jev 录制的 mock 回放断言随之恢复；生成物 | 3 | 本线程 |
| 5 | 文档登记 | 关闭 RFC-0039 D12、G-b、G-c、G-k；对齐文档 §9.1 相应各行；§9 的未做项写进已知差距 | 4 | 本线程 |
| 6 | 发版 | 首个写 schema 3 的发布冻结新增必需字段（RFC-0041 §4） | 5 | — |

3、4、5 都要在发版前合入（RFC-0039 D12），每个 PR 只做一个方向，描述先给体系调用栈，再给本 PR 的改动。3 和 4 之间 master 上没有 mock 回放，这是两个未发布提交之间的短暂缺口，而不是过渡代码（Q5 已定分开）。`replay.create` 在第 3 步就用 `ReplaySet`，所以 `ReplaySet` 随第 3 步落地，第 4 步只加消费状态和 Fetch。

第 3 步对现有 schema 2 fixture 的处理：

| fixture | 现在的用途 | 去向 |
|---|---|---|
| `aimux-core/tests/fixtures/master_language_recordings.jsonl` | `master_recording_test.rs` 断言 0.5 写的 schema 2 能读、能 mock | 删除，连同这个测试：schema 2 不再可读（§1 第 1 条）。换成一条内联的 schema 2 行，断言 `UnsupportedRecordingSchema { found: 2 }` |
| `aimux-core/tests/fixtures/master_decision_recordings.jsonl` | 同上，decision；是 Jev 录制的逐字节副本 | 删除，同上 |
| `aimux-providers/tests/fixtures/jev_systemone_live.jsonl` | 真实 live 录制，三处读它：`decision_recording_test.rs:31`、`:253`（录制形状与回放）；`jev_test.rs:112`（经 `tests/common/replay.rs` 的 wiremock 喂给 Jev provider，走 core 校验）；`aimux-core/tests/master_recording_test.rs:186` | 按 Q4 的 (b) 转换成 schema 3，转换脚本列在 PR 描述里。读 `Recording` 的这三个 provider 测试在 C6 随录制类型搬进 devtools 时已移到 `aimux-ops/tests/`（providers 不能依赖 devtools），第 3 步把它们和 `common/replay.rs` 的 `mount_recording` 改读 schema 3 交换，第 4 步换成 `ReplaySession`；`master_recording_test.rs` 的用法随该测试删除 |
| `contract-tests/dto/Recording/001.json` | DTO 向量 | `UPDATE_DTO_VECTORS=1 cargo test -p aimux-ops` 重写成 schema 3（§7） |

## 11. 已定问题

维护者 2026-10-21 的决定（[#325 评论](https://github.com/arcships/aimux/pull/325#issuecomment-6307035442)）：

- **Q1 采集位置。** 采用 RFC-0044 的 Fetch 观察点，即 §4.1；原提案（采集留在 `http.rs:375`）删除。理由是观察点下录制和回放看到同一份请求：`http.rs:375` 交出的请求还没经过 `SigV4Fetch`、`AnthropicAwsFetch`、`BearerTokenFetch`、`UserAgentFetch` 这些装饰层，而回放 Fetch 在装饰层里面，SigV4 的 `x-amz-date`、`x-amz-content-sha256` 两边对不上。
- **Q2 归属。** 按 §10 的顺序：第 2 步先建 `aimux-devtools`，第 3 步再写 schema 3，不先落在临时位置再搬。
- **Q3 调用层事件。** `(seq, attempt, parent)` 与交换序号用 RFC-0044 §4.2 的 `DiagnosticScope`；操作开始、结束事件的接法由调用观测 RFC 定，本文只列要求（§4.1）。
- **Q4 Jev 真实录制。** 用 (b)：确定性的一次性转换，非流式 JSON 响应各记一个分块，`offsetUs` 取录下的延迟，`[REDACTED]` 头去掉值；脚本不进仓库，在第 3 步的 PR 里列出供审核。脱敏头缺值已被 §5.2 排除出比较，转换后的录制可以回放。
- **Q5 3 与 4 是否合并。** 分开（§10 末段）。
