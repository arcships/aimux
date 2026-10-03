# 全链路 AI SDK 对齐设计附录:全仓库影响面地图

> 12 个模块只读扫描 + 逐模块对抗核验后保留的发现。主文档见 [aisdk-architecture-alignment.md](aisdk-architecture-alignment.md)。本附录中涉及旧数据迁移 / 兼容的建议(如 S5-1 的 schema 2→3 迁移表)已被主文档否决,以主文档为准。

# aimux provider → model 重构：全仓库影响面地图

> 输入是 12 个模块只读扫描后、经对抗核验保留的发现。本文只做重组和归并，不新增证据；file:line 全部取自材料。材料中标为"推断"的结论，在本文中仍标为推断。所有描述采用核验修正后的表述（被修正或部分证伪的原始说法见 §6.2）。

## 0. 编号约定与统计口径

| 前缀 | 模块 | 编号示例 |
|---|---|---|
| `as` | aimux-stream | as:STREAM-1 … as:STREAM-16 |
| `cs` | core-stream | cs:STREAM-1 … cs:STREAM-17 |
| `ct` | core-types | ct:OPT-1、ct:MSG-1、ct:NS-1、ct:CT-1、ct:WIRE-1、ct:TOOL-1、ct:TYPE-1 |
| `cg` | core-generate | cg:GEN-1 … cg:GEN-18 |
| `ce` | core-errors-retry-timeout | ce:RETRY-1、ce:ERR-1、ce:BUG-1、ce:BEHAV-1、ce:STREAM-1、ce:TIMEOUT-1、ce:BIND-1 |
| `mm` | core-multimodal | mm:MM-1 … mm:MM-15 |
| `sub` | core-aimux-subsystems | sub:CORE-1、sub:TRACE-1、sub:SESSION-1、sub:REPLAY-1、sub:LM-1、sub:PROV-1、sub:COMP-1、sub:MOA-1、sub:ID-1 |
| `pu` | provider-utils | pu:PU-1 … pu:PU-14（材料中没有 PU-11） |
| `cv` | providers-convert | cv:CONV-1 … cv:CONV-22 |
| `ffi` | ffi | ffi:FFI-1 … ffi:FFI-15 |
| `bd` | bindings | bd:BIND-1 … bd:BIND-19 |
| `tl` | tools | tl:TOOLS-1 … tl:TOOLS-16 |

- 同名 id 一律带前缀：STREAM-1 同时出现在 as、cs、ce 三个模块，BIND-1 同时出现在 ce 和 bd。
- "漏报补充"原本没有 id，本文按它在材料中出现的顺序编为 `模块:补N`，例如 as:补1。pu:补4（有意差异的风险提示）和 pu:补7（覆盖说明）不是发现，不编入计数。
- 计数口径：主列只计带编号的发现。"独立偏差" = structural-deviation + behavior-deviation。漏报补充单独列一栏，给出"原始条数 / 去重后新增条数"；与其他模块已编号发现同源的复述不算新增，去重是人工判断。

## 1. 总览

### 1.1 按模块统计

| 模块 | redesign-impact（数 / 最高） | 独立偏差（数 / 最高） | bug（数 / 最高） | 编号合计 | 漏报补充（原始 / 新增） | 与 provider 重构的关系 |
|---|---|---|---|---|---|---|
| as aimux-stream | 4 / high | 8 / medium | 4 / high | 16 | 8 / 3 | 本身不带 provider/model 身份；向上传导的是默认上限和错误映射；另有两份复制出来的 provider 流水线 |
| cs core-stream | 3 / high | 13 / high | 1 / medium | 17 | 7 / 2 | stream_text 的 retry 和录制入口直接读 model 配置；StreamPart 同时承担两层语义 |
| ct core-types | 5 / high | 6 / medium | 3 / high | 14 | 7 / 1 | CallOptions 夹带 retry、录制上下文、body_overrides；没有命名空间规则，也没有 supportedUrls |
| cg core-generate | 4 / high | 9 / medium | 5 / medium | 18 | 4 / 1 | retry、config_snapshot、身份字符串三类数据都从这里写入下游 |
| ce core-errors-retry-timeout | 4 / high | 11 / medium | 2 / high | 17 | 6 / 3 | retry_config 贯穿 8 个 model trait 和 provider 设置；缺 LoadApiKey 错误，NoSuchModel 没有生产者 |
| mm core-multimodal | 5 / high | 9 / high | 1 / high | 15 | 6 / 3 | SPI 的 CallOptions 混入了用户层参数；Provider trait 没有多模态工厂 |
| sub core-aimux-subsystems | 5 / high | 6 / high | 8 / medium | 19 | 6 / 4 | 录制、trace、replay、composite 依赖配置快照和身份字符串 |
| pu provider-utils | 3 / high | 6 / high | 4 / medium | 13 | 5 / 4 | 没有 fetch 注入点；录制固化在传输原语里；没有 header 合并原语 |
| cv providers-convert | 3 / high | 12 / high | 7 / high | 22 | 5 / 1 | 按身份字符串分支，命名空间写死；多家 provider 静默丢数据 |
| ffi | 5 / high | 6 / high | 4 / medium | 15 | 5 / 1 | 40 个 provider+model 融合工厂，没有 provider 对象，也没有身份 getter |
| bd bindings | 6 / high | 8 / high | 5 / high | 19 | 8 / 2 | 8 份手写工厂，凭证以位置参数传入；线程和流式模型有问题 |
| tl tools | 4 / high | 3 / medium | 9 / high | 16 | 6 / 4 | 按名字构造 model，按字符串过滤和回放；存在凭证外泄 |
| **合计** | **51** | **97** | **53** | **201** | **73 / 29** | |

### 1.2 被多个模块重复报告的同源问题

同一根因被多个模块各自报告。RFC 应当按根因处理，不要按条目逐个处理。

| 根因 | 报告位置 |
|---|---|
| model 上的 retry_config / provider 级 maxRetries | cs:STREAM-1、ct:OPT-1、ct:补3、cg:GEN-1、cg:补2、ce:RETRY-1/2/3/4、mm:MM-1/2、sub:CORE-2、sub:补3、pu:PU-4、ffi:FFI-4、bd:BIND-8、tl:TOOLS-5、tl:补3 |
| config_snapshot / ProviderRecord | cs:STREAM-2、cg:GEN-2、sub:CORE-1、sub:补2、cv:CONV-21、cv:补3、ffi:FFI-5、tl:TOOLS-5 |
| 身份字符串被精确或子串匹配 | sub:CORE-3、sub:TRACE-3、cg:GEN-8、cg:补3、tl:TOOLS-3、tl:补1、tl:补6、ffi:FFI-5、bd:BIND-9、mm:MM-7 |
| 命名空间写死或未定义 | ct:NS-1、cg:GEN-7、cg:补4、cv:CONV-1/3/9/10、as:STREAM-9、mm:MM-8、cs:STREAM-13 |
| 缺 fetch 注入，录制上下文经 CallOptions 传递 | pu:PU-1、pu:PU-2、ct:OPT-3、sub:CORE-4、sub:REPLAY-1、as:STREAM-10、as:STREAM-12 |
| 凭证作为数据、header 合并缺失 | pu:PU-3、pu:PU-5、ct:OPT-4、ce:ERR-1、ffi:FFI-3、ffi:FFI-6、bd:BIND-2、bd:补6、tl:TOOLS-14 |
| 流错误有两种表示（Err 项和 Error part） | as:STREAM-7、cs:补1、ce:STREAM-1、ce:补3、ffi:FFI-8、ffi:补5 |
| handler 不产出 ParseResult / raw | as:STREAM-8、pu:PU-7、pu:补6 |
| Provider trait 只有 LM，FFI/绑定/工具用融合工厂 | sub:PROV-1、mm:MM-3、ce:ERR-2、ffi:FFI-1/2、bd:BIND-1/11、tl:TOOLS-4 |
| 缺 supportedUrls 和 ai 层下载 | ct:MSG-1、sub:LM-1、cv:CONV-7、cv:CONV-5/6、ct:补1 |
| OpenAI / xAI 默认走 chat | cv:CONV-14、as:补6、bd:BIND-16、bd:补8、tl:TOOLS-10 |
| body_overrides 只在两个 family 生效 | ct:OPT-2、cv:补1、as:补2、cv:CONV-4 |
| 复制流水线（Vertex 与 Google、Vertex-Anthropic 与 Anthropic） | as:STREAM-3/4、as:补1、as:补7、cv:CONV-8/9/17 |
| 工具调用累加器多份实现 | as:STREAM-5、cv:CONV-20 |
| ToolResult 没有类型 | ct:CT-1、ct:补4、ct:补5、cv:CONV-19、cv:补4 |
| 模型生成的文件不进 response_messages | cs:STREAM-9、cg:GEN-6、ct:补2 |
| repair 不受 abort/deadline 约束 | cs:STREAM-7、cg:GEN-4 |
| 缺 ToolChoiceViolation | cs:STREAM-15、cg:GEN-10 |
| embedding 不分块，单值 provider 截断 | ce:BUG-1、mm:MM-4、mm:MM-5、mm:补1 |
| image 不分批 | ce:BEHAV-2、mm:MM-6 |
| 视频 status 重试超出 poll 预算 | ce:BEHAV-1、mm:MM-9 |
| Node ProviderConfig 字段被静默丢弃 | ce:BIND-1、bd:BIND-3 |
| 重放时发送 "[REDACTED]" | sub:REPLAY-2、tl:TOOLS-6 |
| UA 覆盖而非追加 | pu:PU-6、cg:GEN-16、cg:补1 |
| init_proxy 时机 / 全局代理 | pu:PU-8、pu:PU-12、ffi:FFI-12、ffi:补3 |
| 非流式调用无法取消 | ffi:FFI-9、bd:BIND-6、bd:补5 |
| StreamTextResult 没有聚合视图、双层泵 | cs:STREAM-10、bd:BIND-13、bd:BIND-15、bd:BIND-19、bd:补3、bd:补4 |
| StreamPart 线协议（外部标签） | cs:STREAM-3、cs:补6、ct:WIRE-1、ct:补6 |
| 错误形状变更牵连的消费方 | ce:ERR-2、ce:ERR-3、ce:补5、ffi:FFI-6 |

## 2. 跨模块耦合图

### 2.0 总图

```
provider 设置 / XxxConfig（凭证、base_url、profile、retry_config、body_overrides、poll_config）
        │  融合工厂把配置 clone 进每个 model
        ▼
provider-utils ─────► providers（convert / do_stream / 内部重试）─────► model trait
(shared_client、        按身份字符串分支、命名空间写死                  provider()、retry_config()、config_snapshot()
 录制写死、header)            │
        ▲                    ▼
aimux-stream ──► response_handler ──► StreamPart 流 ──► core（stream_text / generate_text / 7 个多模态 op）
                                                         │  prepare_retries、record_provider、record_input、span
          ┌──────────────────────────────────────────────┼──────────────────────────────┐
          ▼                                              ▼                              ▼
 recording（ProviderRecord，schema=2）         trace（TraceLayer scope_key、        composite（Router/MoA，
   └► replay（rebuild_provider / MockReplay）    verdict 子串匹配、TraceFilter）     retry_config=0）
        └► tools（aimux-replay / web replay）    └► tools（cache-probe CLI / web）
                                                         │
                                                         ▼
                        FFI（40 个融合工厂、config_json.max_retries、没有身份 getter）
                                                         │
                                                         ▼
                        bindings（8 份手写工厂、ProviderConfig.maxRetries、6 种语言手写类型镜像）
```

下面按链路展开。每条链路先列涉及的模块和传导路径，再按"跳"列出发现 id。用户点名的 aimux-stream / core 流链路放在最前。

### L1. 流契约：aimux-stream → provider-utils → providers → core → 消费方 → FFI → bindings

涉及模块：aimux-stream、aimux-provider-utils、aimux-providers（全部 do_stream）、aimux-core（generate、recording、moa、openai_output、trace、replay）、aimux-ffi、bindings（node、python、swift、kotlin、java、flutter）。

前提：aimux-stream（sse.rs / ndjson.rs / lines.rs / streaming_tool_call_tracker.rs）本身不带 provider 或 model 身份，泛型只依赖 `S: Stream<Item=Result<Bytes,E>>`，provider 重构本身不会改到它（as:STREAM-12 成立部分、sub:补5、sub:REPLAY-3 核验）。向上传导的是三样东西：它的默认值、provider-utils 对它的错误所做的映射、以及上层没有复用它的地方。

1. **aimux-stream 层**
   - 单事件固定上限 1 MiB，调用方无法配置：as:STREAM-1、as:补8
   - 分帧规范差异（单独的 `\r`、CRLF 与 LF 混用、BOM、无冒号字段）：as:STREAM-14
   - 每次 poll 都从头重扫 buffer；放开上限后复杂度变成平方：as:STREAM-15
   - 传输错误被转成 String，cause 丢失：as:STREAM-13
   - NdjsonStream 违反 Stream 契约，且没有调用方：as:STREAM-16
   - StreamingToolCallTracker 是死代码，而且落后于 TS 版本：as:STREAM-5、cv:CONV-20
2. **provider-utils response_handler**
   - 产出 `Result<T>` 而不是 ParseResult，raw 丢失，include_raw_chunks 只有 OpenAI chat 生效：as:STREAM-8、pu:PU-7、pu:补6
   - response_handler.rs:407 把 SseError::FrameTooLarge/Utf8 映射成 JsonParse，分帧错误因此被当作"可恢复帧错误"：ce:补3、as:STREAM-1
   - 输入类型绑定 reqwest::Response（是否必须改，存疑）：as:STREAM-12
3. **providers do_stream**
   - 单帧解析失败时，有的 yield Err，有的 yield Ok(StreamPart::Error)：cs:补1、ce:STREAM-1
   - Err 分支不把 finish 设为 error：as:STREAM-7
   - finish 默认 Stop（AI SDK 默认 'other'）：as:STREAM-6
   - 收到 error 后 break，并把 usage 清零：as:STREAM-11
   - Bedrock 读完整个 body 才解码，吞掉 exception，坏帧静默截断：as:STREAM-2、as:补4
   - Vertex-Claude 出错时不发 Finish：as:补1
   - 各 provider 在 do_stream 里 peek 首个事件，影响 first_chunk 计时：cs:STREAM-14
4. **core stream_text / consume**
   - consume 默认 Stop：as:补5
   - 出现 Error part 或可恢复帧错误后，consume 整体失败：cs:STREAM-4
   - 空流不注入 NoOutputGenerated：cs:STREAM-5
   - abort 表现为 Err，而不是 abort part：cs:STREAM-6
   - repair 不受 abort/deadline 约束：cs:STREAM-7、cg:GEN-4
   - 没有 ToolChoiceViolation：cs:STREAM-15
   - 一个 StreamPart 同时承担 provider 层和用户层：cs:STREAM-3
   - 缺少 V4 部件：cs:STREAM-8
   - 结果是一次性的，没有聚合视图：cs:STREAM-10
   - 建立阶段的错误从 async fn 直接返回，出口不统一：cs:补2
   - pump 使用 unbounded channel：bd:补4
5. **core 内的其他流消费方**
   - RecordingOutcomeStream：signal 路径记 Error，drop 路径记 Cancelled，两者不一致：cs:STREAM-6
   - MoA 吞掉 StreamStart 里的 warnings：sub:MOA-1
   - openai_output：repair 改 id 后输出重复 tool call：cg:GEN-17
   - TraceLayer TTFT 的输出块定义与 core 不一致，计时起点偏晚：cs:STREAM-16、sub:TRACE-4
   - replay 手写 SSE 切分和 OpenAI 状态机：as:STREAM-10、sub:REPLAY-3、sub:补5
6. **FFI**
   - as_openai 路径静默丢弃可恢复错误，StreamPart 路径则转成 Error part：ffi:FFI-8、ce:STREAM-1
   - 流式 ABI 只有 push 加阻塞：bd:BIND-5
   - 非流式调用没有 abort：ffi:FFI-9
   - FFI 在 core 之外又做了一次 abort select：cs:补7
7. **bindings**
   - node / python 再各翻译一遍错误：ffi:补5
   - 外部标签 wire 格式由 ts-rs 导出，是 8 个绑定的共同契约：cs:补6、ct:WIRE-1、ct:补6
   - 第二层 bounded(64) 泵：bd:BIND-19
   - Node 的 for-await 里 break 不会取消底层：bd:BIND-15
   - Swift / Kotlin / Java / Flutter 先缓冲再吐出：bd:BIND-5
   - streamText 没有结果对象：bd:BIND-13

### L2. retry_config / maxRetries：provider 设置 → model trait → user op → composite → provider 内部 → 录制 → FFI → 绑定 → 文档

涉及模块：aimux-provider-utils（retry re-export）、aimux-providers（全部 Config、list_models、files、轮询 provider、replay.rs）、aimux-core（retry、8 个 model trait、generate、7 个多模态 op、router、moa、trace/layer、recording）、aimux-ffi、bindings、tools、docs/ai-sdk-request-pipeline.md。

1. **provider 设置**：ProviderOptions.max_retries、ExternalProviderEntry.max_retries（provider.rs:132/201）；各 XxxConfig 共有 44 处 `retry_config:` 字段、13 处 with_retry_config。见 ce:RETRY-1、ce:RETRY-2、pu:PU-4（retry.rs:3 与 lib.rs:45 对 RetryConfig 的 re-export）。
2. **model trait**：8 个 trait 带 retry_config() 默认方法，providers 中有 33 处覆写（其中包含 LM 实现）；VideoModel::poll_config() 是同一种模式。见 ce:RETRY-1、mm:MM-1、cg:GEN-1、ce:补1。
3. **core user op**：generate.rs:566-570 与 :918-922 调用 prepare_retries(call.max_retries, model.retry_config(), …)；7 个非 LM op 同样如此；per-call 只能覆盖次数，initial_delay/backoff 只能取自 model。SPI 的 CallOptions 上带着 max_retries/timeout。见 cs:STREAM-1、cg:GEN-1、cg:补2、ct:OPT-1、ce:RETRY-4、mm:MM-2、tl:补3。
4. **composite 与装饰器**：Router/MoA 把自身覆写为 0，并按子模型的 retry_config 重试；TraceLayer 转发 retry_config；docs:834 写的是"只有 Core user operation 读取"，与实现不符。见 sub:CORE-2、ce:RETRY-5、ffi:FFI-4、sub:补3。
5. **provider 内部重试**：约 45 个 list_models 实现、3 个 files 实现、8 家轮询 provider 都用 prepare_retries(…, config.retry_config / self.retry_config(), …) 自己重试；list_models 在重试之外只构建一次 headers。见 ce:RETRY-3、mm:MM-2、mm:MM-10、mm:补3、ce:补6、pu:PU-4 核验。
6. **录制与回放**：openai/mod.rs:195 把 max_retries 写进 ProviderRecord.provider_options；aimux-providers/src/replay.rs:83-85 回放时又写回 retry_config。见 ce:RETRY-2、tl:TOOLS-5、tl:补3。
7. **FFI**：config_json 和 register_providers 的 JSON 都公开了 max_retries（lib.rs:1427）。见 ffi:FFI-4。
8. **bindings**：ProviderConfig.maxRetries 出现在 node/go/python/swift；Node 有 8 个原生工厂静默忽略这个字段。见 bd:BIND-8、ce:BIND-1 = bd:BIND-3。
9. **文档**：docs/ai-sdk-request-pipeline.md:89 把"provider 默认 retry"列为有意差异，同时还有 §6.1、§10.1（:834）、:789（composite 默认 0）。这些与重构目标直接冲突。见 cs:STREAM-1、cg:GEN-1、ce:RETRY-1、mm:MM-1。

### L3. config_snapshot / ProviderRecord：model 配置数据 → 快照 → 录制 → 回放重建 → tools

涉及模块：aimux-providers（约 20 个 config_snapshot 实现、replay.rs）、aimux-core（generate、recording、replay、trace/layer）、aimux-provider-utils（http.rs 录制体）、aimux-ffi（mock replay）、bindings（ts-rs 导出的 ProviderRecord.ts）、tools（aimux-replay、aimux-web）。

1. **数据来源**：XxxConfig 里的 base_url、api_key_source、profile、provider_options（其中含 max_retries 和 body_overrides）；约 20 个 config_snapshot 实现。见 sub:CORE-1、cg:GEN-2。
2. **写入点**：generate.rs:575-616 与 :927-968 两段录制和 session 初始化代码逐字重复。见 cg:GEN-2、cs:STREAM-2。
3. **RECORDING_SCHEMA=2 的 ProviderRecord**
   - profile 字段只对 OpenAI 兼容 provider 有意义，其他 provider 在 provider_options 里塞各自的私有配置：cv:CONV-21、cv:补3
   - 响应体截断到 1 MiB 并做有损解码：as:补3、as:STREAM-10 核验
   - Bytes 请求体不做脱敏，JSON 请求体却套用了 error-context 截断规则：pu:PU-9
4. **aimux-providers/src/replay.rs 的 rebuild_provider**
   - 永远重建成 chat，Responses 录制会被重放到 /chat/completions：tl:TOOLS-5、tl:TOOLS-10、bd:BIND-16
   - 缺 base_url 时回落到 api.openai.com：tl:TOOLS-2
   - 把 "[REDACTED]" 当真实 header 恢复：tl:TOOLS-6、sub:REPLAY-2
   - 用裸 provider 名查 registry，provider 字符串一旦变成端点级就会查不到：cg:补3、bd:补7
   - 每次重建都 Box::leak：cv:补2
5. **core 的 MockReplayModel**
   - 只支持 OpenAI chat，解码已与真实 provider 漂移：sub:REPLAY-1
   - 手写 SSE 切分：sub:REPLAY-3、as:STREAM-10
   - 只绑定 recordings[0] 的身份：sub:补1、ffi:FFI-5
   - 没有覆写 config_snapshot，于是录制里得到 minimal 快照：tl:TOOLS-2
6. **旁支 TraceLayer**：scope_key 由 provider、model_id 和 config_snapshot().base_url 组成（trace/layer.rs:166-184），而且每个请求都会调用一次快照。见 sub:TRACE-2、tl:TOOLS-7、ffi:FFI-5、sub:补6。
7. **消费端**：aimux-replay（tl:TOOLS-13）、web 的 replay 与 KeyStore（tl:TOOLS-5、tl:补6）、FFI 的 mock（ffi:FFI-5）、ts-rs 导出的 ProviderRecord.ts。
8. **AI SDK 对照**：没有"配置快照"这一概念；最接近的是 model 级的 WORKFLOW_SERIALIZE（serialize-model-options.ts:26-49），它会同步解析 headers，得到的 headers 里含明文 key。见 sub:补2。

### L4. provider 身份字符串 → 命名空间 → 下游匹配

涉及模块：aimux-providers（全部 provider()、convert、流循环、多模态）、aimux-core（response_messages、openai_output、generate、recording、replay、trace/store、trace/verdict、router）、aimux-ffi、bindings、tools（CLI probe、web cache_probe/traces/calls/replay/settings）。

1. **源头**
   - provider() 的字符串形态不统一：有端点级、点分、短横线、registry 裸名等写法：cv:补5
   - OpenAIProvider::name() 固定返回 "openai"，它产出的 model 却报告 config.provider：ffi:补1
   - 多模态有三种命名空间，Files 写死为 "openai.files"：mm:MM-7
2. **convert 层**
   - `provider == "groq"` 出现 7 处，另有 `contains("azure")`：cv:CONV-1
   - 读取 options 时写死 "openai" / "anthropic" / "bedrock"：cv:CONV-1、cv:CONV-9、cv:CONV-10、ct:NS-1、mm:MM-8
   - Azure Responses 写入 azure 命名空间、读取 openai 命名空间：cv:CONV-3
   - 流循环里 metadata key 写死为 "openai"：as:STREAM-9
3. **core**
   - response_messages 硬编码 ["anthropic","bedrock","amazonBedrock"]：ct:NS-1、cg:GEN-7
   - openai_output 用 pm.get("openai")：cg:GEN-7
   - provider 侧的回退路径同样写死：cg:补4
   - signature / thought_signature 是一等字段：cs:STREAM-13、cv:CONV-19
4. **观测与录制**
   - record_input 和 span 直接写入 model.provider()：cg:GEN-8
   - replay matcher 要求字符串精确相等：sub:CORE-3、cg:GEN-8
   - TraceFilter 精确相等：今天 anthropic / google / xai 已经匹配不上：sub:CORE-3、tl:TOOLS-3
   - verdict 按子串选择审计规格，今天已经错配：sub:TRACE-3 = tl:补1
   - composite 的身份是 "router"：sub:COMP-1
5. **FFI 与绑定**：没有 model 身份 getter。见 ffi:FFI-1、ffi:FFI-5、bd:BIND-9。
6. **tools**：mock map 的 key、KeyStore 的 key、推荐 provider 列表都依赖名字。见 tl:TOOLS-3、tl:补6、tl:TOOLS-12。

### L5. 缺少 fetch / transport 注入 → 录制上下文 → 回放 → 协议适配

涉及模块：aimux-provider-utils（http.rs、ws.rs、post_to_api.rs）、aimux-providers（bedrock、catalogue.rs、全部 HttpRequest 构造点）、aimux-core（options、recording、replay、多模态 op）、aimux-ffi（init_proxy）。

1. **provider-utils**
   - 进程级 shared_client，没有 Fetch trait：pu:PU-1
   - GLOBAL_PROXY 只能设置一次，而且时机不确定：pu:PU-12、ffi:FFI-12、ffi:补3
   - WS 绕过环境变量代理：pu:PU-8
   - WS 用 insert、HTTP 用 append，重复 header 的语义不一致：pu:补5
   - SigV4 在传输层之外签名（条件性问题）：pu:PU-1
   - catalogue.rs 另建了一条传输路径：pu:补2
   - 非流式 exchange 30s 上限写死在内部：ce:补2、pu:补4
2. **录制**
   - 录制写死在 send_request_once 里，ExchangeContext 只有 LM 的 CallOptions 实现：pu:PU-2
   - CallOptions 上挂着 call_id / recording_context / for_step：ct:OPT-3、sub:CORE-4
   - 多模态完全不录制：mm:MM-12
3. **回放**：core 里重写了一份 OpenAI 解码器；录制体被截断且有损。见 sub:REPLAY-1、as:STREAM-10、as:补3。
4. **协议适配**：Bedrock 的 event-stream 解码器是 provider 私有的，而且不是增量解码，没法像 AI SDK 那样在 fetch 层转成 SSE。见 as:STREAM-2、as:STREAM-12。
5. **FFI**：init_proxy 是全局的；没有按 provider 注入 transport 的入口。见 ffi:FFI-12。

### L6. 凭证与 headers：构造期作为数据 → 按请求解析

涉及模块：aimux-provider-utils（api_key.rs、headers.rs、http.rs）、aimux-providers（40 处 load_api_key、anthropic、azure、elevenlabs、codex、provider.rs resolve_key）、aimux-core（error、generate）、aimux-ffi、bindings、tools。

1. **provider-utils**
   - load_api_key 的语义：显式空串会回落到环境变量；错误类型是 InvalidArgument：pu:PU-5、ce:ERR-1
   - 没有 combine / normalize / Resolvable，reqwest 用 append，大小写不同的同名头会重复发送：pu:PU-3、ct:OPT-4
   - UA 是覆盖而不是追加，还冒用了 ai-sdk 品牌：pu:PU-6、cg:GEN-16、cg:补1
2. **providers**
   - 40 处 load_api_key 都在构造期调用，并且传 None：pu:PU-3
   - 已经存在三种按请求解析的写法：Anthropic 闭包、Azure TokenProvider、open_responses Fn：pu:PU-3
   - Azure 的 token 在重试闭包外只取一次：ce:RETRY-3、ce:补4
   - ElevenLabs 的 WS 握手用 model 上存的 api_key：mm:MM-14
   - Codex 用 TokenExpired 丢掉了上下文：ce:ERR-4
   - SSRF 守卫依赖 config.base_url：pu:补1
   - resolve_key 在构造期读取环境变量：bd:补6
3. **core 错误类型**：没有 LoadApiKey / LoadSetting。见 ce:ERR-1、ffi:FFI-6。
4. **FFI**：融合工厂要求 api_key 非 NULL，不能回退到环境变量；Vertex 只接受静态 token；Bedrock 只接受 AK/SK（core 其实支持 session_token）；将来若有凭证回调会受重入约束。见 ffi:FFI-3、ffi:补4。
5. **bindings**：apiKey 是位置参数；Bedrock session token、Vertex express、Azure Entra 都不可达；有 8 个工厂丢弃 headers；多模态工厂只接收 base_url。见 bd:BIND-2、bd:BIND-3、bd:补2。
6. **tools**：构造期就把凭证解析成明文；覆盖 base_url 时会带上已存的 key 发出去。见 tl:TOOLS-14、tl:TOOLS-1、tl:补2。

### L7. 复制出来的流水线：缺少"共享 model 类 + config 注入"

涉及模块：aimux-providers（google、vertex、vertex/anthropic_model、anthropic、anthropic_aws、openai、xai、mistral、bedrock）、aimux-stream（tracker）、aimux-core（replay、generate）、aimux-ffi 与 bindings（mock replay 构造点）。

- Vertex Gemini 复制了 Google 的流水线，已经漂移：流式和非流式都丢 inlineData，thought 路径是潜在问题。见 as:STREAM-3。连带问题：Vertex 丢 warnings（cv:CONV-17），不读 options（cv:CONV-4）。
- Vertex-Anthropic 复制了 Anthropic 的流循环：丢 signature（as:STREAM-4），出错不发 Finish（as:补1），failed handler 用了 google 的（as:补7），丢 betas 和宿主能力差异（cv:CONV-8），身份与 Gemini 相同，都是 "google.vertex"（cv:CONV-9）。
- 工具调用累加器至少有 aimux-stream tracker（死代码）、openai/model.rs、xai/model.rs、replay.rs 几份；mistral 假设每个 delta 就是完整调用。见 as:STREAM-5、cv:CONV-20。
- 一个 OpenAI convert 同时服务 OpenAI、兼容厂商、Groq 三种方言。见 cv:CONV-13、cv:CONV-1。
- Bedrock 解码器私有且非增量。见 as:STREAM-2、as:STREAM-12。
- 录制初始化代码有两份（cg:GEN-2）；mock replay 在 FFI、node、python、core 各有构造点（sub:补1）。
- AI SDK 对照：google-language-model.ts:167-171 由同一个类按 config.provider 参数化；google-vertex-anthropic-provider.ts:207-208 直接复用 AnthropicLanguageModel；amazon-bedrock-anthropic-fetch.ts 在 fetch 层做协议转换。

### L8. Provider trait → 工厂 → registry → FFI → bindings → tools

涉及模块：aimux-core（provider.rs、model_id.rs、error、generate、search_model、composite）、aimux-providers（openai/xai 工厂、provider.rs overlay）、aimux-ffi、bindings、tools（CLI、web）。

1. **core**
   - Provider trait 只有 name / language_model / list_models：sub:PROV-1、mm:MM-3
   - 不支持时返回 UnsupportedFunctionality，而 NoSuchModel 没有生产者：ce:ERR-2、ffi:补2
   - NoSuchProvider 的形状不对：ce:ERR-3
   - ModelId 用 '/' 分隔，且没有使用方：sub:ID-1
   - search_model 的定位未定：mm:MM-15
   - 组合能力只针对 LM：mm:补5
2. **ai 层入口**：stream_text / generate_text 只接受 &dyn LanguageModel；没有 registry、customProvider、wrapLanguageModel，也没有默认 provider。见 cs:STREAM-11、cg:GEN-9、bd:BIND-10。
3. **providers**：OpenAI / xAI 默认走 chat；registry 是全局可变的 overlay。见 cv:CONV-14、as:补6、bd:BIND-16、tl:TOOLS-10、ffi:FFI-12。
4. **FFI**：40 个融合工厂；没有 Google LM，也没有 Responses。见 ffi:FFI-1、ffi:FFI-2。
5. **bindings**
   - 8 份手写工厂：bd:BIND-1
   - 各绑定的能力矩阵不一致：bd:BIND-11
   - 工厂是 async 的，I/O 调用却是同步的：bd:BIND-14
   - ProviderName：bd:BIND-17
   - Node 主入口缺少部分导出：bd:BIND-18
   - 多模态构造面是 N×M：mm:补4、bd:补2
6. **tools**：native! 宏写了两份，provider 列表手写。见 tl:TOOLS-4、tl:TOOLS-12。

### L9. supportedUrls / prompt 标准化 → 各 provider 处理文件部件

涉及模块：aimux-core（language_model、language_model_message、content、generate）、aimux-provider-utils（下载器）、aimux-providers（mistral、google、bedrock、cohere、openai、anthropic、vertex/anthropic）。

- **core**
  - LanguageModel 没有 supported_urls，convert_to_language_model_prompt 不做下载：ct:MSG-1、sub:LM-1、cv:CONV-7
  - 没有 standardizePrompt：ct:MSG-3
  - ContentPart 拆成五个文件变体：ct:CT-2
  - ModelMessage 没有 providerOptions：ct:MSG-2
- **providers**
  - Mistral 把文件部件转成 null：cv:CONV-5
  - Google / Bedrock / Cohere 静默丢弃：cv:CONV-6 = ct:补1
  - OpenAI 报错，Anthropic 原样透传：cv:CONV-7
  - Vertex-Anthropic 应当设 supportedUrls={}，强制下载：cv:CONV-8
  - google/utils.rs:31 的 is_supported_file_url 是死代码：ct:MSG-1 核验

### L10. body_overrides / providerOptions 的传递

涉及模块：aimux-core（options、tool、shared）、aimux-providers（openai、anthropic、google、vertex、mistral、bedrock、responses、provider.rs）、aimux-ffi、bindings。

- **core**
  - CallOptions.body_overrides 的契约是"deep-merge 进 provider 构造的请求体"：ct:OPT-2
  - provider_options 有三种类型表示：ct:TYPE-1
  - 没有 schema 解析：cv:CONV-16
  - tool_choice 不是 Option：ct:补7
- **providers**
  - provider 级 body_overrides 每次请求都被塞回 CallOptions，并克隆整个 prompt：ct:OPT-2
  - 调用级 body_overrides 只有 openai chat 和 anthropic 实现：cv:补1
  - Google / Vertex 不读调用级 providerOptions，也没有任何逃生口：cv:CONV-4、as:补2
  - Mistral / Bedrock 没有接线：cv:CONV-18
  - Responses 不支持 body_overrides：cv:CONV-14
  - 兼容厂商的未知键不会透传：cv:CONV-10
- **FFI 与绑定**：bodyOverrides 只在部分工厂生效；FFI 的 stream_options 占用了 openai 命名空间。见 bd:BIND-3、ffi:FFI-14。

### L11. 多模态：SPI 参数与用户 op 参数没有分层

涉及模块：aimux-core（8 个多模态 model 文件、files_model、error）、aimux-providers（全部多模态实现、8 家轮询 provider）、aimux-provider-utils（ExchangeContext）、aimux-ffi、bindings（multimodal.rs 及各语言 typed 结构）。

- SPI 的 *CallOptions 带 max_retries / timeout / poll，而且直接作为跨语言 wire 类型：mm:MM-2、ce:RETRY-4
- 用户 op 的语义缺失
  - embed 不分块：mm:MM-4、ce:BUG-1、mm:MM-5、mm:补1
  - image 不分批：mm:MM-6、ce:BEHAV-2、mm:补2
  - 视频的 status 重试不受剩余 poll 预算约束：mm:MM-9、ce:BEHAV-1
  - Files 没有 core op，重试放在 provider 内部：mm:MM-10、ce:补6、mm:补6
  - 没有空结果错误，也没有 mediaType：mm:MM-11
  - rerank 不短路空文档，不校验下标：mm:MM-13
  - stream_transcribe 的超时下放给 provider：mm:MM-14
- 观测：多模态没有任何遥测、recording、UA 或中间件。见 mm:MM-12。
- FFI 与绑定：transcription / file_upload 丢弃 opts_json；aimux_embed 的 opts 不带 values 就报错；embed / uploadFile 没有 abort。见 ffi:FFI-10、ffi:FFI-11、bd:BIND-6。

### L12. 内容类型：ToolResult 无类型、厂商专属字段做成一等字段

涉及模块：aimux-core（content、tool、stream_part、result、response_messages、parse_tool_call）、aimux-providers（全部 convert）、bindings（ContentPart JSON）、tools（web wire、agent engine）。

- ToolResult.result 是无类型的 Value，各 provider 只能猜：
  - Anthropic 丢 is_error：ct:CT-1
  - Bedrock 的 execution-denied 分支走不到：ct:补4
  - Anthropic 的 file 子项推断会 400：ct:补5
  - OpenAI / Mistral 把整个对象字符串化：cv:CONV-19、cv:补4
- signature / thought_signature 做成一等字段：cs:STREAM-13、ct:NS-1、cg:GEN-7
- 模型生成的文件不进 response_messages，Source / File 丢 provider_metadata：cs:STREAM-9、cg:GEN-6、ct:补2
- 缺 V4 部件（tool-approval-request / custom / reasoning-file）：cs:STREAM-8、ct:CT-2
- 原始 tool 输入在两条路径上类型不同：cs:补5
- 其他：web 的 tool_result 不带 tool_name（tl:TOOLS-11）；input_examples 类型不明确（ct:TOOL-1）；provider tool 不做 schema 校验（cg:GEN-12）

### L13. call_id / session / trace 身份

涉及模块：aimux-core（generate、session、recording、trace/layer、trace/store、moa、router、replay）、tools（aimux-web state、replay）。

- call_id 只在录制开启时生成，session、trace、recording 三方的 id 对不上：sub:SESSION-2
- TraceLayer 位于重试循环内部，同一个 call_id 产生多条记录，store 取的是第一条（失败的那条）：sub:TRACE-1
- 推断出的 session_id 没有写回 CallOptions：sub:SESSION-1
- 没有注册 store 时，推断器仍会写入全局状态：sub:补4
- web 用"全局最新完成的录制"反推 call_id：tl:TOOLS-9
- 回放会写进原会话，而且不产生 trace：tl:补5
- 流式路径没有调用结束事件，response 元数据没有默认值：cs:STREAM-12、cg:GEN-14
- TTFT 口径：cs:STREAM-16、sub:TRACE-4
- composite 的身份与请求体：sub:COMP-1、sub:MOA-2

## 3. 必须随 provider 重构一起改的项

判定标准：不一起改，重构后就会**无法编译**、**能力丢失**或**行为错误**。以下按依赖顺序自底向上排列。与 do_stream 重写"顺手做成本最低"、但不改也不会坏的项，以及防御性的项，放在 §4.1。

### 步骤 0：决策与文档（RFC 层，先于代码）

| 编号 | 改动 | 发现 id | 不改的后果 |
|---|---|---|---|
| S0-1 | 撤销 docs/ai-sdk-request-pipeline.md:89"provider 默认 retry"这一有意差异，同步改 §6.1、§10.1（:834，与实现已不符）和 :789（composite 默认 0） | cs:STREAM-1、cg:GEN-1、ce:RETRY-1、mm:MM-1、pu:PU-4、sub:CORE-2、sub:补3、ffi:FFI-4、bd:BIND-8 | 文档与目标相互矛盾 |
| S0-2 | 定下 provider 字符串规范 `<optionsName>.<endpoint>`；录制、trace、session 中同时保存 provider_id（registry key 或工厂 name）和端点级的 model.provider() | cv:CONV-1、cv:补5、sub:CORE-3、cg:GEN-8、mm:MM-7、ffi:补1、tl:TOOLS-3 | 下游所有匹配点失配（见 L4） |
| S0-3 | 定下命名空间派生规则：`provider_options_name = provider.split('.')[0]`，同时读 canonical key 和 custom key（含 camelCase）；写 metadata 时 canonical 和 custom 都写 | ct:NS-1、cv:CONV-1/3/9/10、as:STREAM-9、cg:GEN-7、mm:MM-8 | 见 S2-5、S4-3 |
| S0-4 | 列出与本重构冲突、需要在 RFC 中改写的既有有意差异：docs:89（retry）；docs:91（first-event peek 影响计时，cs:STREAM-14）；docs:320-328（30s 非流式上限，ce:补2）；docs:158-160（阶段重试原则可以保留，但预算来源必须改，ce:RETRY-3）；RFC-0017 阶段 2（兼容厂商特化改由 bodyOverrides 表达，cv:CONV-10）；RFC-0018（Codex 无状态刷新，ce:ERR-4） | 同左 | 实现与文档继续分叉 |

### 步骤 1：aimux-provider-utils 原语

| 编号 | 改动 | 发现 id | 不改的后果 |
|---|---|---|---|
| S1-1 | 新增 `Fetch` trait，默认实现包一层现有 reqwest client；HttpRequest 增加 fetch 字段，send_one_request 改走注入的 fetch；SigV4 改成 fetch 装饰器，对最终字节签名；WS 另设 connector 注入点；catalogue.rs:410-455 的自建 client 也并入 | pu:PU-1、pu:补2、pu:补5、as:STREAM-12 | 能力丢失：model config 里的 fetch 没有落点，代理、TLS、测试注入、回放都无处可挂 |
| S1-2 | 新增 `HeadersFn` / `Resolvable<T>` / `resolve()`，以及 `combine_headers`（后者覆盖、大小写不敏感）和 `normalize_headers`（统一小写、删除 None）；构建请求时用 insert 替代 append | pu:PU-3、ct:OPT-4 | 行为错误：headers() 闭包与调用级 headers 的合并语义和 AI SDK 不一致；凭证头会重复发送 |
| S1-3 | `load_api_key` 改为：Some(s) 原样返回（包括空串），None 才读环境变量；新增 `load_setting` / `load_optional_setting` | pu:PU-5、ce:ERR-1 | 行为错误（条件性推断）：重构后按请求调用时，显式空串会回落到环境变量，把真实 key 发往用户自定义的 base_url |
| S1-4 | 录制改成 `RecordingFetch` 装饰器；每次调用的 RecordingContext（call_id / attempt / step）通过 task_local 或 `http::Request::extensions()` 传递，由 core 在重试闭包内设置；删掉 HttpRequest.call_id / recording_context，以及 ExchangeContext 里的差异化实现 | pu:PU-2、ct:OPT-3、sub:CORE-4 | 能力丢失：fetch 闭包在构造期就固定了，拿不到每次调用的上下文，录制链路会断 |
| S1-5 | `prepare_retries(max_retries: Option<u32>, abort)`，默认值为常量 2 / 2000ms / ×2；删掉 RetryConfig 的 re-export（retry.rs:3、lib.rs:45） | pu:PU-4、ce:RETRY-1 | 编译失败（配合 S2-1） |
| S1-6 | 新增 `provider_options_name()` helper（`parse_provider_options` 见 §4） | ct:NS-1、cv:CONV-1 | S4-3 缺少统一实现，各处会各自解析 |

### 步骤 2：aimux-core 类型与 trait

| 编号 | 改动 | 发现 id | 不改的后果 |
|---|---|---|---|
| S2-1 | 从 8 个 model trait 中删除 `retry_config()`，从 VideoModel 删除 `poll_config()`；core 的 9 个 op 改用新的 prepare_retries；TraceLayer 不再转发 | ce:RETRY-1、mm:MM-1、cg:GEN-1、cg:补2、cs:STREAM-1、ce:补1、tl:补3 | 编译失败：33 处覆写、9 个 op、composite、TraceLayer；以及 provider 内部约 20 处 `prepare_retries(…, config.retry_config, …)` |
| S2-2 | 拆开"用户 op 参数"（GenerateTextOptions、EmbedOptions 等，包含 max_retries、timeout、poll、max_images_per_call、max_parallel_calls、session）和 SPI 的 CallOptions（严格对齐 V4，只保留 abort_signal 和 headers）；修正 options.rs:31-35 中"对齐 V4 timeout"的错误注释 | ct:OPT-1、ce:RETRY-4、mm:MM-2 | 行为错误：provider 仍能看到并使用 max_retries，违背"maxRetries 只在调用层"；8 家轮询 provider 的预算来源不明 |
| S2-3 | 从 LanguageModel 删除 `config_snapshot()`；ProviderRecord 缩成 `{provider_id, provider, model_id}`，外加可选的可重建描述（方案见 §6.4-2）；把 generate_text 和 stream_text 的"call_id + record_input + record_provider + session"抽成一个共享函数 | sub:CORE-1、cg:GEN-2、cs:STREAM-2 | 编译失败或能力丢失：record_provider、TraceLayer scope_key、rebuild_provider 都失去数据来源 |
| S2-4 | 身份字段按 S0-2 落地：TraceRecord、ProviderRecord、SessionCall 同时保存两种身份；TraceFilter 和 replay matcher 按 provider_id 匹配 | sub:CORE-3、cg:GEN-8 | 行为错误：录制回放、TraceFilter、mock key 全部失配（anthropic / google / xai 今天已经失配） |
| S2-5 | 删除 core 里写死的命名空间（response_messages.rs:31-37 的 extract_reasoning_signature、openai_output.rs:345-351 的 logprobs）；删除 ContentPart::Reasoning.signature、ToolCall.thought_signature、tool::ToolCall.thought_signature，这些信息只通过 provider_options / provider_metadata 传递 | ct:NS-1、cg:GEN-7、cg:补4、cs:STREAM-13 | 行为错误：命名空间一旦按 provider 派生（例如 googleVertex），signature 就取不到，下一轮 thinking 回放失败；groq 等兼容厂商的 logprobs 丢失 |
| S2-6 | LanguageModel 增加 `supported_urls()`，值来自工厂注入的 config 闭包；在 core 的 prompt 转换阶段接入 download（复用 provider-utils 的受控下载器） | ct:MSG-1、sub:LM-1、cv:CONV-7 | 能力丢失：model config 没有地方声明可直传的 URL；Vertex-Anthropic 无法表达"强制下载" |
| S2-7 | Provider trait 对齐 ProviderV4：language_model / embedding_model / image_model 必选，transcription / speech / reranking / files / video / search 可选；不支持时返回 `NoSuchModel{model_type: 枚举}`；list_models 移到扩展 trait，headers 在重试闭包内每次解析 | sub:PROV-1、mm:MM-3、ce:ERR-2、ffi:补2、ce:RETRY-3 | 能力丢失：createXxx(settings) 返回的对象无法提供多模态模型，FFI 和绑定只能继续依赖融合工厂 |
| S2-8 | AiMuxError 新增 `LoadApiKey{env_var,…}` 和 `LoadSetting` 变体；同步修改错误形状的全部消费方：tools/aimux-web api/mod.rs:63 的状态码映射、ffi lib.rs:479-481 与 :722-750 的错误码和 getter、node 和 python 的 error.rs 映射、error_value_golden_test.rs:162-166 | ce:ERR-1、ffi:FFI-6、ce:补5 | 行为错误：key 改为按请求加载后，缺 key 会从调用中以 InvalidArgument 返回，与参数错误混在一起；消费方不改会编译失败或映射错误 |

### 步骤 3：composite 与 trace

| 编号 | 改动 | 发现 id | 不改的后果 |
|---|---|---|---|
| S3-1 | Router/MoA 的重试语义：子模型只使用调用层传下来的 max_retries；子模型最终失败时统一包成不可重试的 RetryError，或者把 per-child 策略写进 composite 自己的 config_json（二选一，见 §6.4-1） | sub:CORE-2、ce:RETRY-5、ffi:FFI-4、cg:GEN-1 | 行为错误（推断）：外层默认重试 2 次，会重跑整个 routing、fallback 或 MoA fanout |
| S3-2 | TraceLayer 不再读 config_snapshot；scope 改为 provider_id + 端点 + 实际请求 origin，或由宿主显式传入；verdict 改为按结构化身份查表，并为每个 AI SDK provider 字符串写单测 | sub:TRACE-2、tl:TOOLS-7、ffi:FFI-5、sub:TRACE-3、tl:补1、sub:补6 | 编译失败；scope 退化后 LCP 历史互相污染；'bedrock.anthropic.messages' 会静默切换到另一套审计规则 |
| S3-3 | composite 在结果中带出实际提供服务的子模型身份（例如 response.model_id 加 provider_metadata['aimux']['servedBy']）；约束 TraceLayer 只包在子模型上 | sub:COMP-1、sub:MOA-2 | 能力丢失：S2-3 之后，composite 的录制拿不到子模型的身份 |

### 步骤 4：aimux-providers

| 编号 | 改动 | 发现 id | 不改的后果 |
|---|---|---|---|
| S4-1 | `createXxx(settings)` 工厂，model config 为 `{provider, url(), headers(), fetch, supportedUrls, transformRequestBody, generateId?}`；key 在 headers() 里 load；Azure token 在重试闭包内解析；ElevenLabs WS 握手也走 headers()；config 里保留 baseURL（SSRF 守卫要用，见 §6.4-3） | pu:PU-3、bd:补6、ce:补4、mm:MM-14、pu:补1、ce:ERR-4（需决策） | 目标本身 |
| S4-2 | OpenAI 兼容：OpenAICompatProfile 改成 config 开关或闭包（include_usage、supports_structured_outputs、convert_usage、metadata_extractor）；拆出独立的 Groq 和 DeepSeek model；execute_stream 不再接收 profile；流中 metadata key 由 provider 派生 | cv:CONV-1、cv:CONV-13、as:STREAM-9、cv:CONV-11、cv:CONV-21 | 行为错误：provider 字符串变成 'groq.chat' 后 7 处 `== "groq"` 全部失效，Groq 退化成 OpenAI 方言（发 stream_options、用 reasoning_content、丢 browser_search） |
| S4-3 | 按 S0-3 读写命名空间：Anthropic 9 处写死、OpenAI 兼容、Azure Responses（读写要一致）、Google/Vertex（googleVertex/vertex/google）、Bedrock（amazonBedrock 优先） | cv:CONV-3、cv:CONV-9、cv:CONV-10、cv:CONV-18、ct:NS-1、cg:补4 | 行为错误：跨 provider 回传 metadata 时读不到 itemId / signature |
| S4-4 | Vertex 复用 Google 的 LanguageModel，按 config.provider 推出 providerOptionsNames；Vertex-Anthropic 复用 anthropic_stream_core，注入 betas、Anthropic failed handler、supportsNativeStructuredOutput=false、supportsStrictTools=false、supportedUrls={}，provider 字符串改为 'googleVertex.anthropic.messages' | as:STREAM-3、as:STREAM-4、as:补1、as:补7、cv:CONV-8、cv:CONV-9、cv:CONV-17 | 目标本身（共享 model 类 + config 注入）；不改就要维护一份以 "google.vertex" 为身份的复制流水线 |
| S4-5 | provider 内部重试：约 45 个 list_models、3 个 files、8 家轮询 provider 改为接收显式预算或使用常量策略（见 §6.4-5）；Runwayml 等 provider 级的轮询默认值移到调用级 poll 或 providerOptions | ce:RETRY-3、mm:MM-2、mm:MM-10、mm:补3、ce:补1、ce:补6、pu:PU-4 | 编译失败：这些位置读 self.retry_config() 或 config.retry_config |
| S4-6 | provider 级 body_overrides 改成工厂设置 `transform_request_body`，放进 config，不再经过 CallOptions；调用级 body_overrides 的去留见 §6.4-7 | ct:OPT-2、cv:补1 | 目标违背：provider 设置每次请求都被塞回 CallOptions，并克隆整个 prompt |
| S4-7 | OpenAI / xAI 的默认 languageModel 改为 responses，同时保留 `.chat` / `.responses`。前提是先补齐 Responses 的 provider tools 映射和 overrides 合并 | cv:CONV-14、as:补6、bd:BIND-16、tl:TOOLS-10 | 默认端点与 AI SDK 相反；如果不先补齐 Responses 就切换，会出现回归（web_search、file_search 等内置工具不可用） |

### 步骤 5：recording / replay

| 编号 | 改动 | 发现 id | 不改的后果 |
|---|---|---|---|
| S5-1 | RECORDING_SCHEMA 升到 3，并提供 2→3 的 provider 字符串迁移表（例如 "openai"→"openai.chat"，"google.vertex" 按 model 拆开）；ProviderRecord 不再保存 max_retries 和 profile，max_retries 只从 input.options 恢复 | sub:CORE-1、sub:CORE-3、cg:GEN-8、ce:RETRY-2、tl:TOOLS-5、cv:CONV-21、cv:补3 | 行为错误：旧录制全部失配；rebuild 按裸名查 registry 失败，返回 Unsupported（cg:补3） |
| S5-2 | rebuild_provider 改为通过 registry 或工厂按 provider_id、model_id、端点种类（chat/responses）重建；不再回落到 api.openai.com；丢弃值为 "[REDACTED]" 的键 | tl:TOOLS-2、tl:TOOLS-5、tl:TOOLS-6、sub:REPLAY-2、cg:补3、bd:补7 | 编译失败（快照字段已删除），或行为错误（Responses 录制被重放到 chat） |

### 步骤 6：ai 层

| 编号 | 改动 | 发现 id | 不改的后果 |
|---|---|---|---|
| S6-1 | 新增 `LanguageModelRef`（Arc<dyn LanguageModel> 或 String）和 `resolve_language_model`；新增 create_provider_registry（分隔符默认 ':'，可配置，出错时返回 NoSuchProvider/NoSuchModel）、custom_provider、set_default_provider、wrap_language_model 加 middleware trait；删除 ModelId | cs:STREAM-11、cg:GEN-9、bd:BIND-10、ce:ERR-3、sub:ID-1 | 目标本身（默认 provider、registry、customProvider、middleware 都在 ai 层） |

### 步骤 7：FFI

| 编号 | 改动 | 发现 id | 不改的后果 |
|---|---|---|---|
| S7-1 | 新增 `aimux_provider_create(kind, settings_json)`、`aimux_provider_model(h, model_type, id)`、`aimux_model_provider` / `aimux_model_id`、`aimux_registry_create` / `aimux_registry_model` / `aimux_set_default_provider`；删除 40 个融合工厂，不保留兼容层；kind 的分发表由 Rust 侧生成，并做一次覆盖对账 | ffi:FFI-1、ffi:FFI-2、ffi:补1 | 编译失败（工厂签名全部失效）；Google LM 和 Responses 仍然不可达 |
| S7-2 | 从 config_json 和 register_providers 中删除 max_retries；需要 per-child 策略的话，放进 aimux_router_new / aimux_moa_new 的 config_json | ffi:FFI-4 | 宿主传入后被静默忽略 |
| S7-3 | 凭证：settings.apiKey 改为可选，缺省时每次请求读环境变量；定义宿主的 headers/credential 回调 ABI，并写明"回调可能在任意 runtime 线程执行，回调内禁止同步调用 aimux_*"；重入守卫改为 `Handle::try_current` | ffi:FFI-3、ffi:补4、bd:BIND-2 | 能力丢失：会过期的凭证（Vertex、Bedrock STS、Codex）无法表达；推断未来的回调在 worker 线程上重入时会 panic |
| S7-4 | 删除 aimux_register_providers，改为 `aimux_provider_create("openai-compatible", …)`；init_proxy 退役，改为按 provider 实例配置传输；如果保留 init_proxy，在 client 已存在时返回错误 | ffi:FFI-12、ffi:补3、pu:PU-12 | 进程级 overlay 互相覆盖；代理静默失效 |
| S7-5 | 新增错误码 LoadApiKey / LoadSetting 和 env_var getter；NoSuchModel 填写 model_type；更新错误码表和 getter 的全部消费方；aimux_mock_replay_new 增加 `{provider, model_id, matcher}` 选项 | ffi:FFI-6、ffi:补2、ce:补5、ffi:FFI-5 | 与 S2-7、S2-8 不一致 |

### 步骤 8：bindings

| 编号 | 改动 | 发现 id | 不改的后果 |
|---|---|---|---|
| S8-1 | 用一份 provider spec 生成各语言的 `createXxx(settings)` 和 provider 对象（`.languageModel`、`.chat`、`.responses`、`.embeddingModel` 等）；原生 FFI/napi/pyo3 只暴露 `create_provider(kind, settings_json)` 加 `handle.model(type, id)` | bd:BIND-1、bd:BIND-11、bd:BIND-16、bd:BIND-17、mm:补4、bd:补2、bd:BIND-3 = ce:BIND-1 | 编译失败：8 份手写工厂的签名全部失效 |
| S8-2 | Resolvable 回调穿过边界：napi 用 ThreadsafeFunction，pyo3 用 GIL 回调，C 用函数指针加 ctx，其余语言各自包装；settings 暴露 sessionToken、credentialProvider、TokenProvider、Vertex apiKey | bd:BIND-2 | 能力丢失：动态凭证不可达 |
| S8-3 | 删除 ProviderConfig.maxRetries；调用级 max_retries 的 None 语义从"provider 默认"改为 2 | bd:BIND-8 | 字段被静默忽略 |
| S8-4 | 所有 model 类暴露只读的 provider 和 modelId；mockReplay 下沉到 FFI/core 的单一实现 | bd:BIND-9、sub:补1 | 宿主无法按身份筛选录制或做迁移映射 |
| S8-5 | 暴露 registry 和 wrapLanguageModel；registerProviders 退役；Model.trace() 改为 middleware | bd:BIND-10 | 继续使用全局可变 overlay，与 S7-4 不一致 |
| S8-6 | 重新生成所有 wire 类型（ProviderRecord、Recording、*CallOptions、StreamPart） | bd:BIND-12、mm:MM-2 | 6 份手写镜像宽松解码，漏改时静默出错 |

### 步骤 9：tools

| 编号 | 改动 | 发现 id | 不改的后果 |
|---|---|---|---|
| S9-1 | CLI 和 web 改用 registry 构造 model；删除两份 native! 宏、NATIVE 列表、前端的 env 映射表和推荐表；--api-key 改为可选 | tl:TOOLS-4、tl:TOOLS-12、tl:TOOLS-14 | 编译失败（native! 宏依赖 XxxConfig::new 加 Provider::new(cfg).model(id) 的形态） |
| S9-2 | 过滤、mock map、KeyStore 都按 provider_id 取值，不再复用 model.provider 字符串 | tl:TOOLS-3、tl:补6 | 行为错误：重构后连 openai 和 registry provider 也会全部失配 |
| S9-3 | 回放通过 registry 重建 model，max_retries 从 GenerateTextOptions 传入 | tl:TOOLS-5、tl:补3 | 编译失败或行为错误 |
| S9-4 | 覆盖 base_url 视为新建一个 provider 实例，不继承已存的凭证 | tl:TOOLS-1 | 保留凭证外泄路径（短期修复见 §5.1） |
| S9-5 | 支持显式选择 chat 或 responses（例如 `openai.chat:gpt-4o`） | tl:TOOLS-10 | S4-7 之后无法再回到 chat |

## 4. 可独立处理的 AI SDK 偏差（不依赖 provider 重构）

### 4.1 应纳入本轮

这些项不改也不会让重构坏掉，但它们要么和 do_stream / convert 的重写在同一批代码上，要么是跨 8 个语言的 wire 破坏，要么是本轮 ABI 整体替换时的防御项，一次改完成本最低。

| 主题 | 发现 id | 建议 | 为什么放本轮 |
|---|---|---|---|
| 流错误统一为单一表示 | as:STREAM-7、as:STREAM-8、pu:PU-7、pu:补6、cs:补1、ce:STREAM-1、ce:补3、ffi:FFI-8、ffi:补5 | handler 产出 `ParseResult<T>{value, raw}`；provider 把单帧失败转成 StreamPart::Error，并设 finish=error；流里的 Err 只用于终止性错误；删除 is_recoverable_stream_error 及 core、FFI、node、python 中的各处分支；统一接上 include_raw_chunks | 每个 provider 的 do_stream 都会被重写 |
| finish 默认值 | as:STREAM-6、as:补5、as:STREAM-11 | provider 与 consume 默认 Other；OpenAI 兼容路径在没有 finish_reason 时发 Error part；收到 error 后继续消费，并保留已累积的 usage | 同上 |
| StreamPart 拆成两层并统一 wire | cs:STREAM-3、cs:STREAM-8、cs:补5、cs:补6、ct:WIRE-1、ct:补6、ct:TYPE-1 | 拆成 LanguageModelStreamPart（对齐 V4，ToolCall.input 为 String，serde tag="type"）和 TextStreamPart；补齐 tool-approval-request、custom、reasoning-file；Source 改为带标签的联合；字节字段用 base64；provider_options/metadata 统一为 SharedProviderOptions | do_stream 重写，加上 schema 3 和 8 个绑定重新生成，只破坏一次 |
| consume / abort / NoOutput 语义 | cs:STREAM-4、cs:STREAM-5、cs:STREAM-6、cs:补7 | Error part 把 finish 设为 Error，并收集到结果里；flush 时注入 NoOutputGenerated（新增专门的变体）；abort 发出 Abort part 后正常结束，RecordingOutcomeStream 映射为 Cancelled；删除 FFI 的第二层 select | 与 TextStreamPart 同一次改动 |
| StreamTextResult 结果对象 | cs:STREAM-10、bd:BIND-13、bd:BIND-19、bd:BIND-15、bd:补3、bd:补4 | 内部 pump 加广播，提供 full_stream / text_stream 和不消费 self 的 finished()；绑定层直接 poll core 流，去掉第二层泵；Node 实现 AsyncGenerator::complete | 绑定层会被整体重写 |
| ToolResult 类型化 | ct:CT-1、cv:CONV-19、cv:补4、ct:补4、ct:补5 | 改成 `ToolResultOutput` 枚举 Text / Json / ExecutionDenied / ErrorText / ErrorJson / Content，各 provider 用穷尽 match | 各 provider 的 convert 会被重写；短期修复见 §5 |
| ContentPart 文件部件收敛，prompt 标准化 | ct:CT-2、ct:MSG-2、ct:MSG-3 | provider 侧统一为 `File{data: FileData, media_type, filename}`；ModelMessage 增加 provider_options；instructions 支持 SystemMessage；增加 standardize_prompt，合并相邻 tool 消息，检查 MissingToolResults | 配合 S2-6 的下载改造 |
| 工具调用 tracker 统一 | as:STREAM-5、cv:CONV-20 | 把 tracker 移到 provider-utils，直接发 StreamPart；DeltaToolCall.index 改为 Option；OpenAI chat、mistral、xai、replay 统一接入；generateId 来自 config | S4-2 拆分 Groq/兼容 model 时，否则会再复制一份 |
| first_chunk 计时起点 | cs:STREAM-14、sub:TRACE-4 | 把 first-event peek 从各 provider 下沉到 core 的统一流式 helper，first_chunk 在 do_stream 返回后才开始计时；TraceLayer 的 started 移到调用之前 | 移除 peek 需要改每个 do_stream |
| UA 分层 | pu:PU-6、cg:GEN-16、cg:补1 | with_user_agent_suffix 改为追加（key 小写）；core 追加 `aimux/<ver>`，provider 追加 `aimux-sdk/<provider>/<ver>`，不再冒用 ai-sdk 品牌 | S1-2 改 header 管线时要一起定层 |
| 回放改为 replay_fetch，录制保存原始字节 | sub:REPLAY-1、as:STREAM-10、sub:REPLAY-3、sub:补5、as:补3、pu:PU-9 | S1-1 之后，MockReplayModel 改成 `create_replay_fetch(recordings, matcher)`，由真实 provider 解码，在 wire 层匹配；录制保存原始字节（base64）并单独设上限；删除 core 里的 OpenAI 解码和手写 SSE | 有了 fetch 注入后顺理成章；否则 Bedrock 二进制流和超过 1 MiB 的 SSE 永远无法回放 |
| provider warnings 契约 | cv:CONV-15、cv:CONV-17、ct:OPT-2 | 各 prepare_tools 对 Tool::Provider 发 unsupported；Mistral / Bedrock / Vertex 把 warnings 贯穿到结果；不支持的 CallOptions 字段（body_overrides、include_raw_chunks）统一发 Warning::Unsupported | convert 重写 |
| 各 provider 的 getArgs 对齐 | cv:CONV-4（bug）、cv:CONV-12、cv:CONV-18、cv:CONV-22、cv:CONV-16 | 在 provider-utils 增加 `parse_provider_options::<T>`；Google 接上 thinkingConfig 和顶层 reasoning 映射；OpenAI 处理 logprobs 形状、推理模型剥离参数、strictJsonSchema；Mistral / Bedrock 接上 options，Bedrock 实现 JSON response tool；finish 映射补 function_call；DeepSeek 的 cacheRead；normalize_openai_json_schema | 与 S4-2、S4-3 改动的是同一批函数 |
| 多模态用户 op 语义 | mm:MM-4、mm:MM-6、mm:MM-9、mm:MM-10、mm:MM-11、mm:MM-13、ce:BEHAV-1、ce:BEHAV-2、mm:补2 | 新增 embed_many（按条数和字节分块、并发、数量校验）；image 复用 video 的分批 helper，加入 is_retryable 和 NoImageGenerated；video 的 do_status 参数收窄，status 重试受剩余预算约束；新增 core 的 upload_file op；NoSpeech / NoTranscript / NoVideo；rerank 短路空文档并校验下标 | S2-2 拆分用户层和 SPI 后才有地方放 |
| 非流式取消与 opts 透传 | ffi:FFI-9、ffi:FFI-10、bd:BIND-6 | 新 ABI 的每个调用入口统一带 abort_handle 参数；transcription 和 file_upload 反序列化 opts_json；Node 的 embed / uploadFile 补上 bridge | S7-1 会整体替换 ABI |
| ABI 版本握手（防御性） | ffi:FFI-7 | 导出 `aimux_abi_version()` 和 `AIMUX_ABI_VERSION` 宏，本次重构升一个 major；各绑定加载库时比对 | 同名符号签名在本轮改变，dart:ffi 按名字查找时新旧不匹配会出现 UB |
| 工厂同步化 | bd:BIND-14 | Node 的工厂和 provider.languageModel() 改为同步；其他语言提供异步 I/O 版本（async throws / suspend / CompletableFuture / 后台 isolate） | 不破坏行为，但对齐 `const model = openai('…')` 的接口定型只有这一次机会 |
| 类型生成流水线 | bd:BIND-12 | schemars 导出 JSON Schema，再生成 Python / Go / Java / Kotlin / Swift / Dart 类型；JNA 声明从 cbindgen 生成；Python 只保留一套 API | 本轮 wire 变化集中在一起 |

### 4.2 可后续单独立 RFC

| 主题 | 发现 id | 建议 |
|---|---|---|
| 多步循环、工具执行、stopWhen、prepareStep、streamRetries | cg:GEN-10、cs:补4、cs:补3、ce:TIMEOUT-1 | 单独 RFC；prepareStep 切换 model 依赖 S6-1；先把"单步"写进有意差异清单 |
| ToolChoiceViolation | cs:STREAM-15、cg:GEN-10 | 短期可以先补：finish 时检查 tool_choice，新增错误变体；它只依赖身份，不依赖配置数据 |
| Output API 与 generate_object | cg:GEN-3（bug）、ce:ERR-5 | 引入 output 参数，强制 JSON responseFormat，做 secure parse 加 jsonschema 校验，失败返回带 text/usage/finish_reason 的 NoObjectGenerated；fix_json 只在显式 repair 时使用 |
| 流式回调、transform、partialOutputStream | cs:STREAM-10（剩余部分） | on_chunk / on_finish / on_error / on_abort、smoothStream 类 transform |
| invalid tool call 与 repair 的细节 | cg:GEN-5、cg:GEN-11、cg:GEN-12 | invalid 调用生成 tool-error 结果；repair 后仍失败时返回修复后调用本身的错误；provider tool 带 input_schema 并做校验 |
| response 元数据与 include | cs:STREAM-12、cg:GEN-14、cg:GEN-15 | 在 ai 层补全 id / timestamp / model_id；流式路径发出 call-end 事件；增加 include.request_body，默认 false |
| 参数校验与错误细节 | ct:OPT-5、ce:ERR-6、ce:RETRY-5（剩余部分）、ce:TIMEOUT-1 | 校验 maxOutputTokens>=1；RetryError 序列化时带 message 和 lastError，并在 try>1 时保留错误历史；非流式调用收到 first_chunk/chunk 时返回 UnsupportedFunctionality |
| SSE 解析器规范化 | as:STREAM-14、as:STREAM-13 | 改成逐行状态机（三种行结束符、去 BOM、无冒号字段）；SseError 透传原始错误作为 cause（as:STREAM-15 必须随 as:STREAM-1 一起修，见 §5） |
| provider-utils 补全 | pu:PU-13、pu:PU-14、pu:补3 | 新增 delete_from_api；JSON error handler 支持 isRetryable；IdGenerator；在工厂里接入 validate_base_url |
| 多模态范围 | mm:MM-12、mm:MM-15、mm:补5 | 决定遥测、中间件、录制是否覆盖多模态；search_model 的 specification_version 改为 aimux 自有标识 |
| ResponseMessageBuilder 按 id 聚合 | cs:STREAM-17（uncertain） | 先确认是否存在交错发送的 provider |
| 绑定层体验 | bd:BIND-17、bd:BIND-18、tl:TOOLS-11 | ProviderName 的语义；Node 主入口导出 ai 层函数；wire 的 ToolResult 增加 tool_name |

## 5. 真实 bug

"重构后消除"列的含义：√ 表示按 §3 做完后自然消失；× 表示需要独立修复；部分 表示只消除一部分。重复报告已合并，写成"a = b"。

### 5.1 安全与凭证泄露（最高优先，建议先于重构单独修）

| id | 严重度 | 问题 | 位置（材料） | 重构后消除 |
|---|---|---|---|---|
| tl:TOOLS-1 | high | aimux-web 绑定非 loopback 地址时，请求带 base_url 覆盖会把已存 key 发给攻击者主机；`env:VAR` 能读出服务进程的任意环境变量 | wire.rs:291-309、calls.rs:35-58、model_builder.rs:29-31 | 部分（S9-4）；短期修复：覆盖 base_url 时必须显式提供凭证，`env:` 限定在 registry 白名单内 |
| tl:补2 | medium（推断） | 没有 Host/Origin 校验；在默认 loopback 下，DNS rebinding 也可能触发 TOOLS-1 | api/mod.rs:22-51 | × |
| tl:TOOLS-2 | medium | mock 录制的 base_url 为 None，重放时用第三方厂商的 key 请求 api.openai.com（链路跨 core replay、providers replay、web） | core replay.rs:434-450、providers replay.rs:59-65、web replay.rs:72-87 | √（S5-2）；短期修复：rebuild 在 base_url 缺失且 provider 不是 openai 时报错 |
| tl:TOOLS-6 = sub:REPLAY-2 | medium | 重放把 "[REDACTED]" 当作 header/body 发出，覆盖 Bearer；今天就会 401 | providers replay.rs:71-73、core replay.rs:997-1001、openai/model.rs:44-52 | 部分：provider 级 headers 改为工厂设置后，只剩调用级问题 |
| pu:PU-9 | medium | 录制中 Bytes 请求体（Bedrock JSON、multipart 二进制）完全不脱敏，原样落盘；JSON 请求体却被 error-context 规则截断 | http.rs:898-905 | 部分（录制改为 fetch 装饰器时一起迁移） |
| pu:PU-8 | medium | WS 绕过环境变量代理；只配置了一部分字段的 init_proxy 会让 HTTPS 直接出网 | http.rs:67-69、:100-109、ws.rs:448-461 | 部分（S1-1、S7-4） |
| ct:OPT-4 ≈ pu:PU-3 | medium | 大小写不同的同名头被重复发送（reqwest append）；anthropic/mod.rs:320-331 已有实例：用户传 `Authorization` 时两个值同时发出 | openai/model.rs:44-53、http.rs:639-646 | √（S1-2） |
| pu:PU-12（+ffi:补3） | low | init_proxy 的返回值不反映是否生效；HTTP 与 WS 的代理配置可能不一致 | http.rs:59-61、:79-98、ffi lib.rs:3243-3258 | √（S7-4） |
| pu:PU-5 | low（重构后变成真实问题） | 显式传入空串 api_key 会回落到环境变量 | api_key.rs:19-23 | √（S1-3，必须先于 headers 闭包落地） |
| pu:PU-1（SigV4 部分） | 条件性 | 调用方 headers 里带 Content-Type 时，传输层又追加一个，导致 SigV4 签名不匹配 | http.rs:649-650、sigv4.rs:82-102 | √（S1-1） |

### 5.2 数据丢失或结果错误（high）

| id | 问题 | 重构后消除 |
|---|---|---|
| as:STREAM-1 | SSE 单事件 1 MiB 上限：大于 1 MiB 的合法事件（Gemini 流式图片、Responses 的 response.completed）会让本应成功的请求以显式 Err 失败，同时丢掉 File / usage / finish。修复时必须一起修 as:STREAM-15 的平方复杂度 | × |
| as:STREAM-2（+as:补4） | Bedrock converse-stream 并非真正流式，内存无上限；exception 帧被吞掉，finish 为 Stop；CRC 错误或截断的帧被当作正常结束；首包超时实际约束整次生成时长 | × |
| as:STREAM-4、as:补1、as:补7 | Vertex-Claude 流式丢失 thinking signature（下一轮 reasoning 被丢弃，多轮工具调用推断会 400）；出错时不发 Finish；错误体用 Google 的 handler 解析 | √（S4-4） |
| ce:BUG-1 = mm:MM-5、mm:补1 | Bedrock Titan/Nova 和 Vertex gemini-embedding-2 传入多个值时只嵌入第一个；Bedrock 传入空 values 时 panic；Vertex 传入空 values 时凭空返回 1 个向量 | ×；短期修复：do_embed 开头断言 1 <= len <= max，core 层对空输入短路 |
| ct:CT-1（+ct:补4、ct:补5、cv:补4） | Anthropic 在 tool 角色路径丢掉 is_error；Bedrock 的 execution-denied 分支永远走不到；Anthropic 把 V4 content 的 file 子项原样发出（推断会 400）；同一个 ToolResult 在不同 provider 上语义不同 | 部分（§4.1 类型化）；短期修复：anthropic/convert.rs:637-644 合并 is_error |
| cv:CONV-2 | 把 top_k 发给官方 OpenAI 和 243 个 profile 为空的兼容厂商 | √（S4-2） |
| cv:CONV-3 | Azure Responses 写入 azure 命名空间、读取 openai 命名空间，itemId / store 的多轮回传断裂 | √（S4-3） |
| cv:CONV-4（+as:补2） | Google / Vertex 完全不读调用级 providerOptions，不映射 reasoning，也没有 body_overrides 逃生口 | ×（§4.1 getArgs） |
| cv:CONV-5 | Mistral 把 FileBase64 / FileUrl / FileReference 转成 JSON null 放进 content | 部分（S2-6）；短期修复：不认识的变体返回错误 |
| cv:CONV-6 = ct:补1 | Google / Bedrock / Cohere 静默丢弃 URL 和 base64 文件部件；Google 在消息变空时还会补一个 `{text:""}` | 部分（S2-6） |
| cv:CONV-8 | Vertex-Anthropic 和 anthropic_aws 丢掉 betas；Vertex 仍收到 url source、strict、原生 output_format | √（S4-4） |
| cv:补1 | 调用级 body_overrides 只有 OpenAI chat 和 Anthropic 实现，其他 provider 静默忽略，违反 language_model.rs:22-23 的 Warning 契约 | 取决于 §6.4-7 的决策 |
| bd:BIND-3 = ce:BIND-1 | Node 的 8 个原生工厂静默丢弃 headers / maxRetries / bodyOverrides / organization / project | √（S8-1）；短期修复：不支持的字段抛 InvalidArgument |
| bd:BIND-4 | Python 所有非流式调用和多模态调用在 block_on 期间持有 GIL | ×（包进 py.allow_threads） |
| bd:BIND-5 | Swift / Kotlin / Java / Flutter 的 Async / Sequence / Stream 包装先完整缓冲再吐出，而且不能取消；Flutter 会阻塞 UI isolate | 部分（S7-1 可以顺带提供 pull 式流 ABI） |
| bd:BIND-7 | TS 的 GenerateTextOptions 所有字段都是必填（T \| null），传部分选项无法通过 tsc | ×（ts-rs optional，加 CI 的 tsc --noEmit） |

### 5.3 medium

| id | 问题 | 重构后消除 |
|---|---|---|
| cs:STREAM-7 = cg:GEN-4 | tool-call repair 不受 abort 和 total/step deadline 约束；generate.rs:750-752 的注释与实际不符（FFI 路径因 biased select 不会卡住） | × |
| cg:GEN-3 | generate_object 无条件先跑 fix_json，不做 schema 校验，不强制 JSON responseFormat，截断的输出会被当成对象返回（finish_reason 仍会带出 Length） | ×（§4.2） |
| cv:CONV-17 | Mistral / Bedrock / Vertex Gemini 的 warnings 恒为空，Vertex 还丢掉了已经算好的 tool warnings | 部分（Vertex 在 S4-4 中消除） |
| sub:TRACE-1 | TraceLayer 位于重试循环内部，同一个 call_id 产生多条记录，store 按 call_id 取第一条（失败的那条），轨迹和链视图因此失真 | × |
| sub:TRACE-3 = tl:补1 | verdict 按子串判定家族：Google AI Studio 落到 generic；Vertex 上的 Claude 被套用 Gemini 规格 | √（S3-2） |
| sub:TRACE-4 | 流式 TTFT 在 do_stream 返回后才开始计时，系统性少算连接和 TTFB | 部分（§4.1 计时起点） |
| sub:SESSION-1 | 推断出的 session_id 没有写回 CallOptions，TraceLayer 看不到 | × |
| sub:SESSION-2 | 录制关闭时不生成 call_id，session / trace / recording 三方的 id 对不上 | 部分（S1-4 后可以无条件生成） |
| sub:补1 | mock 回放只绑定 recordings[0] 的身份，一个 jsonl 里混有多个 provider/model 时，第一组之外的全部静默匹配不上 | 部分（§4.1 replay_fetch） |
| pu:PU-10 | media_type_to_extension 不做映射也不小写：audio/x-wav 得到 x-wav，大写或非 audio 类型会生成带斜杠的文件名 | × |
| ffi:FFI-11 | aimux_embed 的 opts_json 不带 values 时报 InvalidArgument；Go 的 omitempty 一定会触发（推断，未实测） | ×（S7-1 重写时顺带修） |
| bd:BIND-15 | Node 的 streamText 提前 break 不会取消底层流（推断） | 部分（§4.1 结果对象） |
| bd:补1 | Node 的 AbortBridge 不处理已经 aborted 的 signal，还会覆盖用户的 onabort（推断） | × |
| tl:TOOLS-8 | Agent 引擎对 ToolInputStart 和 ToolCall 各 push 一次，同一个工具执行两次，并产生重复的 tool_call_id | × |

### 5.4 low

| id | 问题 | 重构后消除 |
|---|---|---|
| as:STREAM-16 | NdjsonStream 在 inner 已返回 None 后还会再次 poll（违反 Stream 契约）；目前没有调用方 | × |
| cg:GEN-13 | invalid 调用的 best-effort 解析和 generate_object 都没有做 __proto__ 检查 | × |
| cg:GEN-17 | repair 修改 tool_call_id 后，stream_text_as_openai 输出两个 tool call | × |
| cg:GEN-18 | redacted reasoning 在流式和非流式两条路径上的聚合结果不一致 | × |
| cg:补1 = pu:PU-6 | with_user_agent_suffix 是覆盖而不是追加 | √（§4.1 UA） |
| ct:TOOL-1 | input_examples 启发式解包，会误伤 schema 里恰好有 input 字段的合法示例 | × |
| sub:MOA-1、sub:MOA-2 | MoA 丢 warnings；generate 和 stream 两种模式的 request_body 不一致 | × |
| sub:REPLAY-3（≈as:STREAM-10） | mock 流式回放手写 SSE 切分，带 id: / event: 行的块被静默跳过 | √（§4.1 replay_fetch） |
| sub:补4 | 没有 SessionStore 时，推断器仍会写入全局 recent 表 | × |
| ffi:FFI-13 | 转写 session 的用户 abort 转发任务永久泄漏；join 超时时不打告警 | × |
| ffi:FFI-14 | stream_options 的键路径有三种写法，注释说会移除但实际没有 | × |
| ffi:FFI-15 | router / fallback 配置值拼错时静默降级为默认策略 | × |
| cv:补2 | profile_from_registry 每次创建 provider 都 Box::leak | √（profile 退役） |
| tl:TOOLS-9 | 用"全局最新完成的录制"反推 call_id，并发时会错配 | ×（core 结果带出 call_id） |
| tl:TOOLS-12 | /api/providers 推荐了 5 个既不在列表里、也构造不出来的 provider | √（S9-1） |
| tl:TOOLS-13 | aimux-replay 的 --call-id 没有匹配时仍返回成功；dry-run 不做重建 | 部分 |
| tl:TOOLS-15 | NDJSON 导入不是原子操作；imported 列表没有上限 | × |
| tl:TOOLS-16 | 写死 xdg-open，在 macOS / Windows 上静默失败 | × |
| tl:补4 | mock 模式仍要先解析 `env:VAR` 凭证 | 部分 |

## 6. 核验中不确定的发现与待确认点

### 6.1 标为 uncertain 的 4 条

| id | 不确定点 | 需要确认 |
|---|---|---|
| as:STREAM-12 | "handler 必须改输入类型"证据不足：http.rs:736-742 已经演示了用 http::Response 加 Body::wrap_stream 重建 reqwest::Response；拿 anthropic_aws 做对比有误导（它请求的是原生 SSE 端点，不是 Bedrock InvokeModel） | 成立的部分只有：Bedrock 解码器私有且非增量；aimux 没有 Bedrock-Anthropic InvokeModelWithResponseStream 路径。ResponseHandlerInput 是否需要改，取决于 S1-1 的 Fetch 返回类型设计 |
| cs:STREAM-17 | ResponseMessageBuilder 忽略部件 id，如果两个同类块同时处于打开状态，delta 会拼进同一个缓冲区 | 目前是否有 provider 交错发送多个打开中的块。Anthropic content block 和 OpenAI Responses 的 summary part 都是顺序发送 |
| mm:MM-8 | OpenAI 家族的多模态 model 只认 'openai' 命名空间 | 今天的触发路径不存在：registry 拿不到多模态模型，绑定用的是 OpenAIConfig::new。S2-7 / S7-1 之后会变成真实问题，S0-3 必须覆盖多模态 |
| bd:BIND-9 | "mockReplay 全部 miss"已被证伪：它的身份取自 recordings[0]，与自身录制自洽 | 真实影响落在 providers/replay.rs:57-63 的 rebuild（见 S5-2）；绑定层缺少身份 getter 这一点成立 |

### 6.2 已修正或部分证伪的说法（引用时按修正后的表述）

| id | 原说法 → 修正后 |
|---|---|
| as:STREAM-1 | "静默丢失" → 可恢复错误会进入 provider_error，text()/consume() 返回显式 Err；缺陷是合法请求失败并丢数据。事件大小是估算，未实测 |
| as:STREAM-3 | ":656 写死 vertex" → 实际同时写 googleVertex 和 vertex；thought 混入正文目前触发不了（没有下发 thinkingConfig），属于潜在问题；inlineData 在流式和非流式下都会丢 |
| as:STREAM-4 | "下一轮回传没有签名的块" → anthropic/convert.rs:1256-1276 在没有 signature 时直接丢弃该 reasoning part；do_generate 不受影响 |
| as:STREAM-5 | "三份累加器漂移" → core/openai_output.rs 是编码器；另一份 wire 侧累加器在 xai/model.rs:462-469；缺 index 只影响兼容端点 |
| as:STREAM-6 | 差异在 finish 的值（Other 还是 Stop），不在有没有 Finish |
| as:STREAM-7 | "新 wrapper 容易漏"证据不足（TraceLayer 已经正确处理 Err，Router 不检查流项） |
| as:STREAM-8 | 原因不全在 handler 签名：已经用 T=Value 的 provider 也没有发 Raw |
| as:STREAM-10 | "录制的是原始字节" → 录制体经过 1 MiB 截断和 from_utf8_lossy |
| cs:STREAM-1 | initial_delay/backoff 在任何 provider 中都没有配置入口，不存在"整条链移走"；但影响面扩大到 7 个非 LM model 和 TraceLayer |
| cs:STREAM-4 | 真正的传输错误直接返回 Err，这与 AI SDK 的 controller.error 一致；偏差只出现在 Error part 和可恢复帧 Err |
| cs:STREAM-7 | 卡住的问题只出现在 Rust 直接消费者和 node；FFI 路径因为 biased select 不受影响 |
| cs:STREAM-12 | "stream_text_as_openai 用的是请求 model id"被证伪（它先以请求 id 为初值，再被 ResponseMetadata 覆盖，与 AI SDK 一致） |
| ct:MSG-1 | "google/utils.rs 复刻了 isSupportedFileUrl" → 那是死代码；实际情况是静默丢弃，比原描述更严重 |
| ct:NS-1 | AI SDK 的 openai-compatible 在消息和部件级固定读 'openaiCompatible'，只有 thoughtSignature 用派生的 key |
| ct:CT-2 | "FileData 未被使用" → 输出侧有使用，只是 prompt 侧没用 |
| ct:OPT-5 | seed 是 u64，不存在整数校验缺失；只缺 maxOutputTokens>=1 |
| cg:GEN-2 | 降为 medium：数据源不必然消失，工厂可以在创建时捕获快照 |
| cg:GEN-3 | "调用方没有任何信号"不成立：finish_reason 会带出 Length |
| cg:GEN-5 | aimux 本来就不执行工具，合法调用在 response_messages 里同样没有配对结果；400 不是 invalid 调用独有的后果 |
| cg:GEN-7 | provider 侧的回退路径也写死了 anthropic，两层必须一起改 |
| ce:RETRY-2 | "回放时两个来源叠加"不成立（per-call 的 Some(n) 会覆盖 provider 默认值） |
| ce:RETRY-3 | files 上传会传 abort；Vertex 没有 TokenProvider，旧 token 的风险只在 Azure；另外还漏列了 recraft、assemblyai、anthropic files |
| ce:RETRY-5 | "外层会重试 n 次"不成立；"与 AI SDK 规则相反"说过头了；剩下的真实偏差是 try>1 时丢失前几次的错误历史 |
| ce:ERR-1 | 失败时间点从创建前移到调用，是重构时的一个选择，不是必然结果 |
| ce:ERR-4 | "伪造 401"不对；把刷新放进 headers 闭包与 RFC-0018 的无状态设计冲突 |
| ce:ERR-6 | Display 已经拼好了消息；reason 'abort' 在 AI SDK 里没有生产者 |
| mm:MM-1 | 33 处覆写里包含 LM 实现（cohere/azure/xai/codex），不全是多模态 |
| mm:MM-2 | "(1+R)² 次重试"被证伪（retry.rs:163-166 对内层 Retry 直接透传）；拿到 job id 后单独重试 poll 是 docs:158 认可的设计 |
| mm:MM-7 | "registry 的 groq 多模态"今天走不到，是重构之后才会暴露的问题 |
| mm:MM-9 | None 语义的差异是潜在的（7 个 provider 都返回 Some(1)）；每次 exchange 30s 上限是推断 |
| sub:CORE-1 | "重构后没有东西可以快照"说过头了：AI SDK 有 WORKFLOW_SERIALIZE，anthropic 的 config 也保留 baseURL。目标应当是"model 级可序列化钩子加脱敏"，不是简单删除 |
| sub:CORE-3 | 绑定层的 mockReplay 自洽；真正的问题是只绑定第一条录制的身份 |
| sub:TRACE-1 | 录制关闭时不会撞 id，但 step 会被重复计入；问题在 store 按 call_id 取第一条的语义 |
| sub:TRACE-2 | scope 退化不是必然的，取决于 Rust 版 model 是否暴露可序列化的 config 或 url() |
| sub:TRACE-4 | "≈0ms"说过头了；准确说法是系统性少算了连接和 TTFB |
| sub:REPLAY-2 | 不是对齐之后才出现，今天就会 401 |
| sub:REPLAY-3 | 纯 CRLF 分隔会显式返回 Unsupported；只有 id: / event: 开头的块被静默跳过 |
| pu:PU-1 | SigV4 问题只在调用方 headers 带 Content-Type 时出现 |
| pu:PU-2 | LM 主路径目前没有漏录，漏录只是结构性风险；多模态完全不录制这一点成立 |
| pu:PU-5 | 显式空串分支目前是死代码，降为 low |
| pu:PU-13 | "provider 只能绕过去直接用 shared_client"是推断（当前 0 处调用） |
| pu:PU-14 | 这个快照里没有 provider 传 isRetryable，实际影响接近零 |
| cv:CONV-10 | 白名单实为 15 个键加 Groq 的 reasoningFormat |
| cv:CONV-11 | 降为 low：专门的厂商包会显式开启；真正的缺口是没有逐厂商开关 |
| ffi:FFI-3 | 降为 medium：它是未来设计的约束，不是现存缺陷；Bedrock 在 core 支持 session_token，只是 FFI 没暴露 |
| ffi:FFI-7 | 降为 low：Go 的 cgo 有编译期类型检查；Flutter 按名字查找符号才有风险 |
| ffi:FFI-12 | init_proxy 的机制需要修正：在 client 建好之后首次调用会返回 true，但 HTTP 不生效，WS 却生效 |
| bd:BIND-5 | 基于回调的 streamText(onPart:) 在阻塞期间是增量交付的；缓冲的是 Async / Sequence / Stream 这些包装层 |
| bd:BIND-11 | Kotlin 的 TypedModel 可以包装任意 Model；"约 60/76 个不可达"没有逐一核实 |
| bd:BIND-18 | 在 TS 里这是编译错误，不是运行时 undefined；仓库文档里没有那个示例 |
| tl:TOOLS-2 | gen_fixture 的夹具 provider 是 openai，回落到 OpenAI 是正确的，不能作为泄露示例 |
| tl:TOOLS-4 | 编译期就能发现，降为 medium |
| tl:TOOLS-9 | 只有并发时才会错配 |

### 6.3 需要运行时确认的推断性后果

- **上游是否会拒绝**：cv:CONV-2（OpenAI 收到 top_k）、cv:CONV-5（Mistral 收到 null 部件是否 422）、cv:CONV-8（缺 beta 头）、cv:CONV-11（兼容端对 json_schema / stream_options）、cv:CONV-12（logprobs:5）、cv:CONV-13（官方 OpenAI 收到 reasoning_content）、pu:PU-10（x-wav / x-m4a 扩展名）、ct:补7（没有 tools 时收到 tool_choice）、ct:补5（Anthropic 的 file 子项）、as:STREAM-4（Vertex-Claude 多轮 400）、cs:STREAM-9 / cg:GEN-6（丢失图片和签名后下一轮是否被拒）、tl:TOOLS-8（重复 tool_call_id）、tl:TOOLS-11（Gemini functionResponse.name）、pu:PU-6（没有 UA 时 WAF 是否拦截）、ct:OPT-4（上游对重复头的反应）。
- **运行时行为**：mm:MM-5（panic 经 FFI/napi 是否会中止进程；catch_unwind 只覆盖部分入口）、bd:BIND-15 与 bd:补1（napi 的实际行为）、ffi:FFI-11（serde 缺字段，未实测）、bd:BIND-7（其他 *CallOptions.ts 是否有同样问题）、ce:BEHAV-1 / mm:MM-9（超出预算的时长）、ce:RETRY-3（Azure 旧 token）、ce:补2（30s 上限被切断后重新提交，是否重复计费）、ce:补6（上传超时后重试是否产生重复文件）、as:STREAM-1（真实事件大小）、as:STREAM-3（Vertex inlineData 的运行时丢失）、as:STREAM-5（兼容端是否省略 index）、as:STREAM-14（代理对 CRLF / BOM 的实际行为）、sub:TRACE-4（其他 provider 是否也预读首帧）、sub:CORE-2（删除 retry_config 后外层是否重跑 composite）、sub:补6（config_snapshot 在热路径上的性能）、sub:REPLAY-2（小写 authorization 是否与大写同时发出）、tl:补2（DNS rebinding 能否实际利用）、ffi:FFI-3（未来回调在 worker 线程重入）。

### 6.4 扫描阶段提出、已由主文档裁定的设计决策

下列问题在扫描阶段提出；主文档对每一项都已给出结论，本节只保留问题原文并标注落点，不再是待决清单。

1. **composite 的重试语义**：(a) composite 内部管理子模型重试，文档要求调用方对 router 传 0；(b) 按错误分类处理，子模型最终失败时包成不可重试的 RetryError；(c) per-child 策略写进 composite 的 config_json。相关：cs:STREAM-1、sub:CORE-2、ce:RETRY-5、ffi:FFI-4。
   **裁定**：D14 / 主文档 §4.5——删除 model/provider 级 retry，composite 为每个 child 配置预算，最终组合失败不触发整组重跑。
2. **ProviderRecord 的替代**：model 级 serialize 钩子（对齐 WORKFLOW_SERIALIZE，钩子内必须脱敏，因为 AI SDK 同步解析出的 headers 含明文 key），还是由工厂产出 factory spec 挂在 registry 上。钩子不能放在每个请求的热路径上。相关：sub:CORE-1、sub:补2、sub:补6、cg:GEN-2、cv:CONV-21、tl:TOOLS-5。
   **裁定**：D2 / D24 / 主文档 §4.3–§4.4——不移植 `WORKFLOW_SERIALIZE`，不保存可重建的 ProviderRecord；录制 schema 3 记录身份与元数据，live replay 由宿主 registry 与 operation 目标引用驱动。
3. **model config 是否保留 baseURL 数据**：SSRF 守卫需要（pu:补1），trace scope 需要（sub:TRACE-2、tl:TOOLS-7），序列化需要（sub:补2）；AI SDK 的 anthropic 和 openai config 都保留了可选的 baseURL。
   **裁定**：主文档 §1.1 / §2.3——保留在厂商私有 config（model 可以持有 base URL）；禁止的是下游通过 `config_snapshot()` 之类公开接口读取并推导凭证、身份或重建方式。
4. **双身份字段和迁移表**：provider_id 与端点级 provider 在录制、trace、session 中怎么存，schema 3 的迁移映射怎么定。相关：sub:CORE-3、cg:GEN-8、tl:TOOLS-3。
   **裁定**：主文档 §7——模型身份 / 可寻址引用 / 元数据来源三类分开；schema 3 只处理新录制，不设迁移表（§4.3、附：驳回 S5）。
5. **provider 内部阶段重试的预算来源**：调用层下传 PreparedRetries、固定常量，还是 providerOptions 的 pollIntervalMs；files 是否像 AI SDK 的 uploadFile 一样不重试。相关：ce:RETRY-3、mm:MM-2、mm:MM-10、mm:补3、ce:补6。
   **裁定**：主文档 §4.5——poll / download 的阶段重试用固定常量且不重新 submit，不从 providerOptions 读预算；upload 与 refresh 无隐式重跑（§9.3 状态操作行）。
6. **30s 非流式 exchange 上限放在哪里**：放进可覆盖的默认 fetch、call 层，还是 provider settings。它是 docs:320-328 的有意差异，与 fetch 注入冲突。相关：ce:补2、pu:补4。
   **裁定**：主文档 §3.2 / §5——留在 helper，覆盖注入 fetch 后的完整响应处理，streaming 不套用；登记为产品差异。
7. **调用级 body_overrides 去留**：改成中间件（transformParams），还是保留为 aimux 扩展、在 core 统一实现，并对不支持的 provider 发 Warning；RFC-0017 阶段 2 是否撤销。相关：ct:OPT-2、cv:补1、cv:CONV-10。
   **裁定**：D15——全部删除；compat 保留 providerOptions 透传与 `transformRequestBody`，原生包不增加统一请求体改写契约；RFC-0017 阶段 2 撤销。
8. **Codex token 刷新**：放进 headers 闭包（库持有可变凭证），还是维持 RFC-0018 的无状态设计、由宿主持久化。相关：ce:ERR-4。
   **裁定**：主文档 §4.1 Codex 行 / §4.5——维持 RFC-0018 的无状态设计，401 映射 TokenExpired，宿主持久化并在刷新后重建 provider，不自动重试旋转 refresh token。
9. **宿主 Resolvable 回调的线程模型**：回调运行在哪个线程，重入守卫怎么改，异步宿主怎么桥接。相关：ffi:FFI-3、bd:BIND-2。
   **裁定**：主文档 §0.4 Q4 / §6.4——宿主回调全部语言本期不做；`Resolvable` 仅接受 Rust 内建实现。
10. **多模态是否纳入录制、遥测、中间件、registry**。相关：mm:MM-12、pu:PU-2、mm:补5。
   **裁定**：主文档 §3.5 / §5 “观测范围”行——纳入：为 object 和非 LM 模态增加 operation/attempt 事件，录制全模态（§9.3 录制行）。
11. **search_model 的定位**：ProviderV4 的扩展工厂、单独的 provider 类型，还是 tool；以及它的 specification_version 字符串。相关：mm:MM-15。
   **裁定**：D23 / 主文档 §3.1——作为 aimux 的 Provider 扩展工厂（`search_model`）保留，独立搜索服务继续使用 Search 扩展；本文不引入 `specification_version` 字符串。
12. **supportedUrls 在 composite 上的取值**：Router 先路由再取，还是取子模型交集；MoA 取交集。相关：sub:LM-1。
   **裁定**：主文档 §4.2——Router/MoA 默认 `supported_urls={}`，由 ai 层先下载媒体；不计算正则集合交集。
13. **first_chunk 计时起点**：把 peek 下沉到 core，还是写进差异表。相关：cs:STREAM-14。
   **裁定**：主文档 §4.2 / §5 “SSE 预读”行——保留首事件错误预读并登记为产品差异；trace TTFT 从调用 `do_stream` 之前计时。
14. **非 Node 绑定的异步形态与流式 ABI**：是否提供 pull 式的 `aimux_stream_open / next / close`，把线程模型留在 Rust 内部。相关：bd:BIND-5、bd:BIND-14。