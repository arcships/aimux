# RFC-0016: 对齐 Vercel AI SDK 能力缺口

> **Status**: DRAFT (pending review)
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
4. **H4 多步工具循环的范围**:是否对标 Vercel `generateText` 的完整多步(stopWhen/maxSteps/prepareStep/activeTools/toolOrder/refineToolInput/repairToolCall),还是先做最小可用版(tools 自动执行 + steps)。
