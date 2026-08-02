# RFC-0016: 对齐 Vercel AI SDK 能力缺口

> **Status**: DRAFT (pending review) — 实施状态与下游 wrapper 影响见 [§7](#7-实施状态截至-2026-08-02)(2026-08-02 更新)
> **Date**: 2026-08-01
> **Scope**: 系统对比 aimux 0.1.2 与 Vercel AI SDK (`@ai-sdk/openai` / `@ai-sdk/openai-compatible` / `@ai-sdk/provider` V4) 的接口与实现,识别能力缺口,并按优先级规划补齐路径
> **Related**: [RFC-0009](0009-request-resilience.md) request resilience(retry/timeout,本 RFC 的 abortSignal/timeout 缺口与之相关),[RFC-0014](0014-logging.md) 统一日志体系(可观测性缺口依赖本 RFC 的 span 树)

---

## 1. 背景与动机

### 1.1 为什么做这次对齐

aimux 定位为 Rust 实现的 Vercel AI SDK 替代品,以统一的 provider 接口服务多语言绑定(Node/Python/Go/Java/Kotlin/Swift/Flutter)。在 0.1.2 修复 tool_result 字段别名和 reasoning_content 回传时,暴露出一个更深层的问题:**部分能力缺口不是单点 bug,而是系统性的接口/暴露不完整**。本 RFC 记录一次全面对齐调查的结果,作为后续补齐的依据。

### 1.2 调查方法

启动三个并行调查,分别覆盖:

1. **OpenAI 兼容 provider 的接口与实现**——对比 `@ai-sdk/openai-compatible` / `@ai-sdk/openai` 的 CallOptions 字段、请求体构建、provider 特化、headers/proxy/fetch、Responses API
2. **流式输出与 StreamPart 类型**——对比 V4 `LanguageModelV4StreamPart` 的 variant 完整性、usage/metadata 透传、abort/cancellation、错误处理、raw passthrough
3. **Node binding 的 API 暴露**——对比 `generateText`/`streamText` 选项、provider 工厂能力、结果对象、abortSignal、maxRetries

调查结论在三个维度上高度一致地收敛到同一批缺口。

### 1.3 关键前提

aimux 的 `openai` 模块**不是**薄封装的 openai-compatible 等价物,而是对标 `@ai-sdk/openai`(官方 OpenAI 包)的完整实现,并作为 Groq/DeepSeek/LiteLLM-proxy 等薄封装的基座。因此:

- 对比 `@ai-sdk/openai-compatible` 时,aimux 在请求体字段上**普遍超出**(支持 `logit_bias`/`parallel_tool_calls`/`store`/`metadata`/`prediction`/`service_tier`/`prompt_cache_*` 等 OpenAI 专属字段,而 openai-compatible 都不支持)。
- 真正的缺口集中在**通用可扩展性**、**几个 V4 标准字段**和 **binding 暴露层**上。

---

## 2. 缺口清单

缺口按性质分两类:
- **binding 暴露缺口**(易补):Rust core 已有能力,但 Node binding 没透出
- **core 架构缺口**(需设计):core 本身缺失,需新增类型/接线

### 2.1 高优先级(实际可用性硬伤)

| # | 缺口 | Vercel | aimux | 性质 | 证据 |
|---|------|--------|-------|------|------|
| H1 | **abortSignal 取消** | ✅ `RequestOptions.abortSignal` | ❌ LLM 路径完全不支持 | core 架构(基础设施已就绪) | `CallOptions` 无 `abort_signal` 字段([options.rs:36-82](../aimux-core/src/options.rs#L36));`do_generate`/`do_stream` 硬编码 `abort_signal: None`([openai/model.rs:369](../aimux-providers/src/openai/model.rs#L369))。但 `AbortSignal` 类型已实现([shared.rs:84-114](../aimux-core/src/shared.rs#L84)),HTTP 层 `send_request` 已用 `tokio::select!` 响应取消([http.rs:324-335](../aimux-provider-utils/src/http.rs#L324)),`HttpRequest.abort_signal` 字段存在([http.rs:161](../aimux-provider-utils/src/http.rs#L161))且被 embedding/image/speech 等 media provider 使用——**唯独语言模型漏了** |
| H2 | **maxRetries 不可配** | ✅ default 2,可设 0 关闭 | ❌ Node 无法关闭/调整 | binding 暴露 | Rust `RetryConfig`(max_retries=2,initial_delay=2s,backoff=2,尊重 retry-after,只重试 is_retryable)已实现并在 HTTP 层生效,但 Node 工厂([lib.rs:165](../bindings/node/src/lib.rs#L165))和 `GenerateTextOptions`([generate.rs:37-55](../aimux-core/src/generate.rs#L37))都没透出。"无法关闭重试"对测试和延迟敏感场景是真实痛点 |
| H3 | **timeout** | ✅ 丰富 `TimeoutConfiguration`(totalMs/stepMs/firstChunkMs/chunkMs/toolMs) | ❌ 无任何超时控制 | core 架构 | 无对应字段或实现 |
| H4 | **多步工具循环(agent)** | ✅ steps/stopWhen/toolResults/prepareStep/activeTools | ❌ 单次调用,不执行工具 | core 架构 | aimux `generate_text`/`stream_text` 是单次调用,只返回 tool_calls,无 tool_results/steps。无自动多步 agent 循环 |

### 2.2 中优先级(影响完整度/易用性)

| # | 缺口 | Vercel | aimux | 性质 | 证据 |
|---|------|--------|-------|------|------|
| M1 | **工厂级 headers/org/project/retry 未透出** | ✅ `createOpenAI({headers, organization, project})` | ❌ 工厂签名只有 `(apiKey, modelId, baseUrl)` | binding 暴露 | Rust `OpenAIConfig` 的 `with_headers`/`with_org_id`/`with_project`/`with_retry_config` 全有([openai/mod.rs:94-167](../aimux-providers/src/openai/mod.rs#L94)),Node napi 工厂([lib.rs:165](../bindings/node/src/lib.rs#L165))只透 3 参。AnthropicConfig 同理 |
| M2 | **includeRawChunks / raw passthrough** | ✅ `LanguageModelV4CallOptions.includeRawChunks` | ❌ variant 已定义但未接线 | core | `StreamPart::Raw` variant 已存在([stream_part.rs:165](../aimux-core/src/stream_part.rs#L165)),但 provider 从不 emit;`CallOptions`/`GenerateTextOptions` 无 `include_raw_chunks` 字段。测试明确注释 "requires an `include_raw_chunks` option not present"([openai_model_test.rs:1522](../aimux-providers/tests/openai_model_test.rs#L1522)) |
| M3 | **logprobs 请求** | ✅ `@ai-sdk/openai` | ❌ 只解析响应,不能请求 | core | aimux 在响应里解析 logprobs([types.rs:25-27](../aimux-providers/src/openai/types.rs#L25)、[model.rs:310-316](../aimux-providers/src/openai/model.rs#L310)),但 `build_request_body` 从不写入 `logprobs`/`top_logprobs`([convert.rs:1098-1329](../aimux-providers/src/openai/convert.rs#L1098))。注:openai-compatible 同样不支持,仅相对 `@ai-sdk/openai` 是缺口 |
| M4 | **通用 providerOptions 透传** | ✅ openai-compatible 把未知 key 透传 | ❌ 只发硬编码白名单 key | core | Vercel openai-compatible 把 `providerOptions[provider]` 下所有未知 key 过滤已知项后直接铺进请求体([openai-compatible-chat-language-model.ts:297-307](../reference/ai/packages/openai-compatible/src/chat/openai-compatible-chat-language-model.ts#L297))。aimux 只发送硬编码白名单,用户无法通过 providerOptions 发送任意新 API 参数 |
| M5 | **transformRequestBody 通用钩子** | ✅ 通用 `transformRequestBody` | ❌ 仅 DeepSeek 封闭枚举 | core | aimux `RequestBodyOverride` 是封闭枚举(仅 `DeepSeek`),([openai/mod.rs:52-55](../aimux-providers/src/openai/mod.rs#L52)),第三方代理/厂商无法自定义请求体变换 |
| M6 | **自定义 fetch / proxy** | ✅ `FetchFunction` + 显式 proxy | ❌ 共享 reqwest client | core 架构 | aimux 用进程级共享 reqwest client([model.rs:33](../aimux-providers/src/openai/model.rs#L33)),无法注入自定义 fetch/中间件;proxy 仅靠 reqwest 默认读取 `HTTP_PROXY`/`HTTPS_PROXY` 环境变量 |
| M7 | **顶层结果聚合**(reasoning/files/sources/responseMessages) | ✅ 顶层便捷字段 | ⚠️ 数据在 `raw` 里 | core | 非流式 reasoning 藏在 `raw.content`(GenerateContent::Reasoning),未提取到顶层;files/sources 同理。无 `responseMessages`(无法直接把结果回填为下一轮消息) |
| M8 | **StreamPart 缺 3 个 variant** | ✅ | ❌ | core | 缺 `tool-approval-request`(provider-executed 工具审批流)、`custom`(provider 专有内容块)、`reasoning-file`(推理产物中的文件);`source` 无法表达 `document` 子类型(缺 mediaType/filename) |
| M9 | **stream-start warnings 被丢弃** | ✅ 透传 unsupported/deprecated 警告 | ❌ 恒为空 `vec![]` | core | `build_request_body_with_warnings_fallible` 计算出 warnings,但 `execute_stream` 丢弃,`StreamStart{warnings:vec![]}` |
| M10 | **usage.raw 未填充** | ✅ `raw: usage` | ❌ 写死 None | core | `convert_usage` 把 `raw` 写死 None([model.rs:111-126](../aimux-providers/src/openai/model.rs#L111)),丢失 provider 原始 usage(额外字段无法透传) |
| M11 | **streamText 高层聚合器** | ✅ `.text`/`.fullStream`/`.toUIMessageStream()`/`.consumeStream()` | ❌ 只 yield 原始 StreamPart | binding | `streamText` 只 `yield` 原始 StreamPart,缺少高层聚合 |
| M12 | **结构化 output / generateObject** | ✅ | ❌ | core | 只有 `response_format` JSON,无类型化 `output` 返回路径 |
| M13 | **生命周期 callbacks** | ✅ onStart/onStepStart/onStepFinish/onEnd/onToolExecution* | ❌ | core | 无生成生命周期回调 |

### 2.3 低优先级

| # | 缺口 | Vercel | aimux | 证据 |
|---|------|--------|-------|------|
| L1 | `response-metadata.timestamp` 恒 None | ✅ 用 `created*1000` 填充 | ❌ | [stream_part.rs](../aimux-core/src/stream_part.rs) |
| L2 | `tool-call.input` 类型为 `Value`(解析后 JSON) vs V4 规范的 `string` | ✅ string | ⚠️ Value | 可能是 aimux 有意设计(Node 侧消费更友好),非确认缺口 |
| L3 | supportsStructuredOutputs / includeUsage 可配置化 | ✅ | ❌ 硬编码 | [openai/mod.rs](../aimux-providers/src/openai/mod.rs) |
| L4 | env 自动读取 apiKey/baseUrl | ✅ `OPENAI_API_KEY`/`OPENAI_BASE_URL` | ❌ Node 强制传参(Rust 有 `from_env()` 但 Node 未用) | [lib.rs:165](../bindings/node/src/lib.rs#L165) |
| L5 | telemetry | ✅ | ❌ | — |
| L6 | toolApproval | ✅ | ❌ | — |
| L7 | queryParams / errorStructure / supportedUrls / metadataExtractor / convertUsage | ✅ | ❌ | [openai-compatible-provider.ts:49-118](../reference/ai/packages/openai-compatible/src/openai-compatible-provider.ts#L49) |

---

## 3. 已对齐的能力(确认无缺口)

为避免重复劳动,记录已经对齐或超出的能力:

### 3.1 CallOptions 字段(完整对齐)
`prompt`、`max_output_tokens`、`temperature`、`top_p`、`top_k`(aimux 比 openai-compatible 更强——后者只发 warning)、`stop_sequences`、`presence_penalty`、`frequency_penalty`、`seed`、`response_format`、`tools`(含 strict)、`tool_choice`、`headers`(per-request)、`provider_options`、`reasoning`(7 个变体与 V4 完全对齐)、`instructions`。

### 3.2 请求体字段(超出 openai-compatible)
aimux 已发送:`model`、`messages`、`stream`+`stream_options`(include_usage)、`top_k`、`max_tokens`/`max_completion_tokens`(reasoning 感知)、`temperature`/`top_p`/`frequency_penalty`/`presence_penalty`(reasoning 剥离)、`stop`、`seed`、`response_format`、`logit_bias`、`user`、`parallel_tool_calls`、`verbosity`、`store`、`metadata`、`prediction`、`prompt_cache_*`、`safety_identifier`、`reasoning_effort`、`service_tier`(含 flex/priority 校验)、`tools`/`tool_choice`、DeepSeek `thinking` override。

### 3.3 usage 解析(颗粒度更细)
`TokenUsage` 字段齐全(noCache/cacheRead/cacheWrite + reasoning/text 拆分),`convert_usage` 正确计算 cached_tokens(含 Moonshot 顶层格式)、cacheWrite、noCache。比 openai-compatible 更完整。

### 3.4 StreamPart 核心类型(基本对齐)
`text-start/delta/end`、`reasoning-start/delta/end`、`tool-input-start/delta/end`、`tool-call`、`tool-result`、`file`、`source`(url)、`finish`、`error`、`stream-start`、`response-metadata`、`raw`(variant 存在)均有定义。覆盖度优于或持平 Vercel。

### 3.5 Responses API(已实现且超出)
aimux 的 `OpenAIResponsesModel`([responses/mod.rs](../aimux-providers/src/openai/responses/mod.rs))已实现 `do_generate`/`do_stream`,支持 input 数组、instructions、store、previous_response_id、reasoning(effort/summary)、response_format、流式主链路 + function_call_arguments + reasoning_summary_text。**openai-compatible 无 Responses API,aimux 在此项超出**。

### 3.6 retry 机制(已实现)
`RetryConfig`(max_retries=2,initial_delay=2s,backoff=2,尊重 retry-after,只重试 is_retryable)已实现并在 HTTP 层生效。缺口仅在 binding 未透出(H2)。

---

## 4. 补齐路径(按优先级)

### 第一批:低风险、高收益(binding 暴露层,core 不动)

可放进下一个 patch 版本(0.1.3),不破坏兼容:

- **M1** 工厂函数加 options 对象:Node `openai()`/`deepseek()`/`anthropic()` 等工厂接收可选 `{ headers, organization, project, maxRetries }`,透传到 Rust 的 `with_headers`/`with_org_id`/`with_project`/`with_retry_config`
- **H2** `GenerateTextOptions` 加 `max_retries` 字段,透传到 per-call RetryConfig 覆盖
- **L4**(顺带)Node 工厂支持 env 自动读取 apiKey/baseUrl

预计改动:`bindings/node/src/lib.rs`(工厂签名)+ `aimux-core/src/generate.rs`(`GenerateTextOptions` 加字段)+ 同步 TS 类型。无 core 行为变化。

### 第二批:中等 core 改动

- **H1** abortSignal 接线:基础设施已就绪,改动集中在 `CallOptions`/`GenerateTextOptions` 加 `abort_signal` 字段 + `do_generate`/`do_stream` 传参 + Node napi 把 JS `AbortSignal` 桥接为 native `AbortSignal`(tokio `CancellationToken`)。其他模型 trait 已有此字段,语言模型只需对齐
- **M2** includeRawChunks 接线:`CallOptions` 加 `include_raw_chunks`,provider do_stream 在 `include_raw_chunks=true` 时 emit `StreamPart::Raw`
- **M9** stream-start warnings 透传:`execute_stream` 把 `build_request_body_with_warnings_fallible` 的 warnings 传入 `StreamStart`
- **M10** usage.raw 填充:`convert_usage` 保留原始 usage JSON

### 第三批:架构性(需设计)

- **H3** timeout:设计 `TimeoutConfiguration`(参考 Vercel 的 totalMs/firstChunkMs/chunkMs)
- **M4/M5** 通用 providerOptions 透传 + transformRequestBody 钩子:从白名单改为"已知项特殊处理 + 未知项透传",并开放 `RequestBodyOverride` 为闭包/trait
- **M6** 自定义 fetch / proxy:设计注入点(reqwest client builder 或 trait 抽象)
- **H4** 多步工具循环:设计 agent 层(steps/stopWhen/toolResults/responseMessages/callbacks),这是最大的架构增量
- **M7/M11/M12/M13** 顶层结果聚合、streamText 聚合器、结构化 output、callbacks:依赖多步循环或独立设计

---

## 5. 与已有 RFC 的关系

- **RFC-0009(request resilience)**:retry 已落地,timeout 是本 RFC H3 的前置;abortSignal(H1)与 retry 正交但同属"请求生命周期控制"
- **RFC-0014(logging)**:可观测性缺口(retry 是否生效、流式断点诊断)依赖本 RFC 的能力补齐后才有意义;abortSignal 的取消事件也需日志埋点

---

## 6. 开放问题

1. **H1 abortSignal 的 napi 桥接**:JS `AbortSignal` → Rust `AbortSignal` 的最佳桥接方式需验证(napi 的 `tokio_rt` feature + `CancellationToken`)。其他模型 trait 已有 `abort_signal` 字段,但 Node 是否已为它们桥接过 signal 需确认。
2. **L2 tool-call.input 类型**:aimux 用 `Value`(解析后 JSON)而非 V4 规范的 `string`。需确认是否回归 V4 的 string 形态,还是保留为 aimux 的有意设计(Node 侧消费更友好)。
3. **M4 通用 providerOptions 透传的安全边界**:从白名单改为透传后,需确保不会把内部字段(如 `prompt_cache_breakpoint` 已在白名单)重复发送。
4. **H4 多步工具循环的范围**:~~是否对标 Vercel `generateText` 的完整多步(stopWhen/maxSteps/prepareStep/activeTools/toolOrder/refineToolInput/repairToolCall),还是先做最小可用版(tools 自动执行 + steps)~~ — 已关闭:H4 明确不做,见 [§7.5](#75-明确不做2026-08-02-决策)

---

## 7. 实施状态(截至 2026-08-02)

> 本节为实施追踪:记录 §2 各缺口的落地状态与下游 wrapper 联动影响,作为后续补齐的核对依据。
> §2 表格保持"起草时缺口"原样,不随实施改动;落地情况一律以本节为准。

### 7.1 已落地(相对本 RFC 起草时)

| # | 缺口 | 落地方式 | 版本 |
|---|------|---------|------|
| H2 | maxRetries 不可配 | `CallOptions.max_retries` + `GenerateTextOptions.max_retries`([options.rs:90](../aimux-core/src/options.rs#L90)、[generate.rs:58](../aimux-core/src/generate.rs#L58));Node `ProviderConfig.max_retries` 透传([lib.rs:207](../bindings/node/src/lib.rs#L207)) | 0.1.3 |
| M1 | 工厂级 headers/org/project/retry | Node 工厂收 `ProviderConfig { baseUrl, headers, organization, project, maxRetries, bodyOverrides }`([lib.rs:168](../bindings/node/src/lib.rs#L168)),透传 `with_headers`/`with_org_id`/`with_project`/`with_retry_config`([lib.rs:189-218](../bindings/node/src/lib.rs#L189));anthropic 同理 | 0.1.3 |
| M4/M5 | providerOptions 透传 / transformRequestBody | **换形态解决**:`RequestBodyOverride` 封闭枚举退役,改 RFC-0017 通用 `body_overrides`(JSON deep-merge,per-call + provider 级 + 全 binding 透出,[convert.rs:1437](../aimux-providers/src/openai/convert.rs#L1437))。任意新 API 参数均可发送;但**不是** Vercel 式"未知 providerOptions key 自动透传"(core `provider_options` 仍白名单读取,[convert.rs:376](../aimux-providers/src/openai/convert.rs#L376)),也**不是** transform 闭包 | 0.1.3 |
| L4 | env 自动读取 | 部分:`provider()`/`deepseek()` 走注册表,api_key 可空读 env([lib.rs:314](../bindings/node/src/lib.rs#L314));`openai()`/`anthropic()` 工厂仍强制传参 | 0.1.3 |
| M9 | warnings 丢弃 | 部分:非流式已透传([model.rs:383](../aimux-providers/src/openai/model.rs#L383));流式仍 `StreamStart{warnings:vec![]}`([model.rs:450](../aimux-providers/src/openai/model.rs#L450)) | 0.1.3 |
| L2 | tool-call.input 类型 | 维持 `Value`(设计选择,Node 侧消费更友好),不回归 V4 string 形态 | — |
| H1 | abortSignal 取消 | `CallOptions`/`GenerateTextOptions` 加 `#[serde(skip)] abort_signal`([options.rs:96](../aimux-core/src/options.rs#L96)、[generate.rs:60](../aimux-core/src/generate.rs#L60));全部 16 个 `LanguageModel` 实现接线到 `HttpRequest.abort_signal`;Node 桥接:JS `AbortSignal` → `AbortBridge`(napi class 持 `Arc<AbortSignal>`,`on_abort` 注册,[lib.rs:40-72](../bindings/node/src/lib.rs#L40)),`generateText`/`streamText` 第三参接收。FFI/C-ABI 无法传运行时句柄,未做 `aimux_cancel`(见 §7.3 类别②)。**2026-08-02 升级**:`AbortSignal` 底层换 `tokio_util::CancellationToken`(事件驱动,覆盖 body 阶段,见 §7.6 R1-R4) | 0.1.6 |
| H3 | timeout | core `TimeoutConfiguration { total_ms, first_chunk_ms, chunk_ms }`([options.rs:32](../aimux-core/src/options.rs#L32));`CallOptions.timeout`/`GenerateTextOptions.timeout`(JSON 可序列化,FFI/Python/Node 自动透传);http 层 `send_timed`/`send_stream_timed`([http.rs:289](../aimux-provider-utils/src/http.rs#L289)):total 覆盖整体(含 retry),流式 `TimeoutBodyStream` 滑动窗口执行 first-chunk/chunk-idle/total;超时 → `AiMuxError::Timeout`(不可重试)。16 个 LLM provider 全部接线。注:`tokio::time::Sleep` 被 drop 会注销 timer——timer 必须存于 stream 内而非 poll 局部变量 | 0.1.6 |

### 7.2 未落地清单(逐条追踪)

**高优先级:**
- ~~**H1** abortSignal~~ — 已落地,见 [§7.1](#71-已落地相对本-rfc-起草时)(2026-08-02)
- ~~**H3** timeout~~ — 已落地,见 [§7.1](#71-已落地相对本-rfc-起草时)(2026-08-02)
- ~~**H4** 多步工具循环~~ — 已移出:明确不做,见 [§7.5](#75-明确不做2026-08-02-决策)

**中优先级:**
- **M2** includeRawChunks — `StreamPart::Raw` 已定义但全仓 0 处 emit;`CallOptions` 无字段
- **M3** logprobs 请求 — 仅响应侧解析([model.rs:363](../aimux-providers/src/openai/model.rs#L363)),请求体不写 `logprobs`/`top_logprobs`
- **M6** 自定义 fetch / proxy — 仍是进程级共享 reqwest client
- **M7** 顶层结果聚合 — `GenerateTextResult` 仅 text/tool_calls([generate.rs:96](../aimux-core/src/generate.rs#L96));reasoning/files/sources 在 `raw.content`,无 `responseMessages`
- **M8** 缺 variant — `tool-approval-request`/`custom`/`reasoning-file` 均无;`source` 无 document 子类型([stream_part.rs:156](../aimux-core/src/stream_part.rs#L156))
- **M10** usage.raw — `convert_usage` 写死 `raw: None`([model.rs:172](../aimux-providers/src/openai/model.rs#L172));流式 raw usage 只进 provider_metadata([model.rs:762](../aimux-providers/src/openai/model.rs#L762))
- **M11** streamText 聚合器 — Node 仍 yield 原始 StreamPart JSON([lib.rs:63](../bindings/node/src/lib.rs#L63));core 仅 `StreamTextResult::text()`
- **M12** 结构化 output / generateObject — 无
- **M13** 生命周期 callbacks — 无

**低优先级:**
- **L1** response-metadata.timestamp 恒 None(非流式 [model.rs:387](../aimux-providers/src/openai/model.rs#L387)、流式 [model.rs:539](../aimux-providers/src/openai/model.rs#L539))
- **L3** supportsStructuredOutputs / includeUsage 仍硬编码
- **L5** telemetry、**L6** toolApproval — 无
- **L7** queryParams / errorStructure / supportedUrls / metadataExtractor / convertUsage — 均未做(注:error structure 有 `DEFAULT_ERROR_STRUCTURE` 常量但写死不可配)

### 7.3 下游 wrapper 影响矩阵

架构事实:所有 binding 走 JSON 边界 —— `aimux-ffi`(C ABI,供 C/Go/Java/Kotlin/Swift/Flutter)与 Node(napi)、Python(pyo3)独立,但都是 options 进 JSON、结果回 JSON([aimux_generate_text(handle, prompt_json, opts_json)](../aimux-ffi/src/lib.rs#L433))。因此**可序列化字段自动透传**,只有**运行时桥接**(回调/取消/宿主函数)必须动 wire 层。

| 类别 | 改动量 | 涉及缺口 |
|------|--------|---------|
| ① 加字段(机械性,每 wrapper 类型层同步;Node/Python JSON 直传可不加字段仅丢类型提示) | 小 | H3、M2、M3(options 字段);M7、M9、M10、L1(结果/StreamPart 字段) |
| ② 动 FFI + 全 wrapper(结构性) | 大 | H1(2026-08-02:**Node 已落地**——`AbortBridge` 类桥接 JS signal;FFI/C-ABI 仍未做 `aimux_cancel(handle)` 类入口,其他语言暂无取消);H4(明确不做,见 §7.5);M13(宿主回调注册);M12(若新增独立入口 `aimux_generate_object` 则全链改动,若复用 generate_text + options 字段则降级为 ①) |
| ③ wrapper 自实现(wire 不动) | 中 | M11(streamText 聚合器,宿主语言便利层);M6(跨语言注入宿主 fetch 不现实,大概率 core 内 reqwest 配置,归 ① 或不涉及) |
| ④ 契约测试同步 | 小 | 所有新增字段需同步 `contract-tests/fixtures/wire-format.json` 及各 binding wire 测试(go `wire_format_test.go`、python `test_wrapper.py`、node `__test__`)。H3 的 `timeout` 字段已入 fixture(2026-08-02) |

### 7.4 文档跟踪状态

- §2/§4 是唯一系统记录缺口的文档;本 RFC 创建后未随实施更新(§2 表格仍把 H2/M1 标 ❌、按 `RequestBodyOverride` 描述 M4/M5),本节起作为修正与追踪入口
- `docs/plan/backlog.md` 仅跟踪 RFC-0017 阶段 2 项,无本 RFC 条目;`docs/api/gaps.md` 是 binding 多模态差距(已闭合),与本 RFC 无关
- 内部审计(QUALITY_REVIEW/REMEDIATION)顺带提到 H1 未接线及 core util.rs `AbortSignal` 缺陷(全仓零调用,建议删除改 `tokio_util::CancellationToken`),未覆盖其余缺口
- 后续实施建议:本表逐条勾销;若进入排期,迁移到 `docs/plan/backlog.md` 分区跟踪

### 7.6 双 agent review(2026-08-02)与修复

提交 1b7229e 后由独立 agent 做了设计 review(glm-5.2)与代码 review(gpt-5.6-sol)。两者独立收敛到同一核心缺口,已全修并提交:

| # | Review 发现 | 严重度 | 修复 |
|---|-------------|--------|------|
| R1 | `TimeoutBodyStream` 超时后不终止,持续 yield `Err(Timeout)`(违反 Stream 契约) | P0(代码) | `done` 终止状态:超时/内部错误/流结束均置位,后续 poll 返回 `None`(fused)。`timeout_stream_is_fused_after_timeout` 测试 |
| R2 | abort 仅覆盖建连/响应头,不覆盖 body(非流式 `resp.bytes()` 与流式 body) | P0(代码)/P1(设计) | 根因是 `AbortSignal` 为 `AtomicBool` + 50ms 轮询(`abort_wait`),非事件驱动。`AbortSignal` 底层换 `tokio_util::CancellationToken`([shared.rs:95](../aimux-core/src/shared.rs#L95)),新增 `cancelled()` future;`send_request` 改 `select!{biased}`(abort 优先);非流式 body 读取包 `select!`;`TimeoutBodyStream` 接 `abort_signal` + 事件驱动 `abort_wait`(存 stream 内,`Notify` waker 跨 poll 存活) |
| R3 | `Instant + Duration::from_millis(u64::MAX)` 可 panic(用户 JSON 可输入) | P1 | `validate_timeout` 入口校验(checked_add,溢出 → `InvalidArgument`);`total_ms=0` 显式立即超时 |
| R4 | abort/timeout 竞争时错误类型不稳定(50ms 轮询) | P1 | `select!{biased}` + `CancellationToken` 事件驱动;`abort_wins_over_total_timeout` 测试 |
| R5 | 错误消息恒为 "stream timeout",无法区分 first-chunk/chunk/total | P2 | `TimeoutKind` 枚举,消息区分 |
| R6 | abort 测试假阳性(只断言 `is_err`、无超时上限) | P2 | 断言 `AiMuxError::Aborted` + join 包 `tokio::time::timeout` + 耗时断言 |
| R7 | Node 多模态 `generate`/`rerank`/`search` 不透出 abort(core 已支持) | P1(设计) | 全部 6 个多模态方法加 `bridge: Option<&AbortBridge>` 参,注入 `opts.abort_signal`([multimodal.rs](../bindings/node/src/multimodal.rs)) |
| R8 | 契约 fixture 缺 `TimeoutConfiguration` 正值 | P2 | `wire-format.json` 补 `timeout_configuration_values` fixture |

新增错误变体 `AiMuxError::Aborted`(error_type "Aborted",不可重试),取代散落的 `Other("request aborted")`。

### 7.7 二轮 review(2026-08-02)与修复

修复提交 e483d5d 后再由同组独立 agent 复验(glm-5.2 设计 / gpt-5.6-sol 代码)。上轮 P0/P1 多数确认已修复(CancellationToken 事件驱动、熔断状态机、Aborted 分类、多模态 bridge);新发现 P1 一个 + P2 若干,已全修:

| # | Review 发现 | 严重度 | 修复 |
|---|-------------|--------|------|
| S1 | 流式 body abort 只在配了 timeout 时生效:`send_stream_timed` 早返回条件只查 timeout 字段、不查 `abort_signal`——"abort 不配 timeout"(最常见用法)丢失 body 取消。`TimeoutBodyStream` 的 no-deadline 分支本就支持 abort,被闸门挡住 | P1(两 agent 独立收敛) | 早返回条件加 `&& request.abort_signal.is_none()`([http.rs:334](../aimux-provider-utils/src/http.rs#L334));`abort_wakes_pending_timeout_stream` 单测覆盖无 timeout + abort 场景并断言 `Aborted` + 熔断 |
| S2 | retry 错误 body(429/5xx/其他)读取与 backoff sleep 期间 abort 不生效(大 `Retry-After` 窗口内无法取消) | P2 | `read_error_body` + backoff `select!{biased; abort, sleep}`([http.rs:600](../aimux-provider-utils/src/http.rs#L600)) |
| S3 | `validate_timeout` 校验基准与 `last_chunk_at + ms` 实际基准不一致,极限值仍可能 panic | P2 | `next_deadline` 全改 `checked_add`;溢出按"当前阶段有配置"→立即超时,否则无 deadline |
| S4 | `abort_wakes_pending_timeout_stream` 丢弃结果、不断言错误类型 | P2 | spawn 返回 item + stream,断言 `Aborted` + 熔断 |
| S5 | `abort_wins_over_total_timeout` 未测真 tie(90ms abort vs 100ms deadline),flaky;timed 层 tie-break 依赖 tokio 内部 poll 顺序 | P2 | 测试注释明示语义范围(严格先到,非同拍 tie);tie 依赖 tokio 实现细节已文档化 |
| S6 | core 依赖过宽:`tokio(full)` + `tokio-util(rt)`;CancellationToken 纯 std 实现、只需 tokio-util 默认 features | P2 | core 删除 tokio 依赖,`tokio-util = "0.7"`(默认 features),core 保持 runtime-agnostic |
| S7 | first_chunk wiremock 测试实际测的是 header 延迟,注释误导 | P2 | 注释修正并指明 body-pending 路径由单测覆盖 |

未采纳(记录):Node 多模态缺 signal→bridge TS 包装层(P2/nit,裸 napi 层已一致,包装层可后续补);`send_timed`/`send_stream_timed` 建连阶段显式 `select!{biased}` 替代 `tokio::time::timeout`(P2,当前内层先 poll 使 Aborted 赢,已文档化依赖)。

### 7.5 明确不做(2026-08-02 决策)

| # | 缺口 | 决策 | 理由 |
|---|------|------|------|
| H4 | 多步工具循环(agent) | ❌ 不做 | 产品决策:不做多步工具循环(steps/stopWhen/工具自动执行)。`generate_text`/`stream_text` 保持单次调用,只返回 `tool_calls`,工具执行与多轮编排由调用方自管(与 aimux 薄绑定、单次调用定位一致) |

§6 开放问题 4(H4 范围)随之关闭。
