# RFC-0044：crate 拆分、依赖反转与 provider 目录布局

> **Status**: Draft
>
> **Date**: 2026-10-19
>
> **Scope**: 把 [docs/aisdk-architecture-alignment.md](../docs/aisdk-architecture-alignment.md) 第 2 章 D17 的五层目标落成可以逐步合入的计划：`aimux-core` 按职责拆入 `aimux-provider`、`aimux-provider-utils`、`aimux`、`aimux-devtools`；core 内部直接调用的录制、session、trace、下载改成依赖反转；`aimux-providers` 每个厂商一个目录、文件按上游包对齐、测试放进各自目录。本文不改变公开行为、wire 格式和录制格式。
>
> **Baseline**: `master@5743a40a`（#265 合入后）。上游对照：`vercel/ai` main `fce9210`（目录与文件命名），行为基线仍按对齐文档登记的版本（`ai@7.0.127` 等）。
>
> **Related**: [对齐文档](../docs/aisdk-architecture-alignment.md) §2、§8、§9.1（D3、D4、D6、D17、D27、D28、D30）、[RFC-0036](0036-positioning-and-layered-architecture.md) §3、[RFC-0041](0041-replay-governance.md) §3 / §10、[RFC-0042](0042-v4-type-boundaries.md) §1、[RFC-0043](0043-size-performance-budgets.md)、[#276](https://github.com/arcships/aimux/issues/276)、[#270](https://github.com/arcships/aimux/issues/270)、[#273](https://github.com/arcships/aimux/issues/273)、[#278](https://github.com/arcships/aimux/issues/278)、[PR #268](https://github.com/arcships/aimux/pull/268)

## 1. 决策摘要

对齐文档已经定了目标分层（D17），但没有说怎么从现在的代码走过去，#276 里也只有“provider-utils 不再依赖 aimux-core”一行。本文补上这部分，并新增 provider 目录与测试位置的规则。

| 编号 | 决定 | 后果 |
|---|---|---|
| L1 | 按 D17 新建 `aimux-provider`、`aimux-devtools`，`aimux-core` 拆空后改名 `aimux`；不留兼容 re-export（D17 约束 6、D31） | 每一步在同一个 PR 里改完 ops、FFI、四个工具、Node/Python 绑定的全部 import |
| L2 | 先在 core 内部按层拆开混在一起的文件，再切 crate | crate 切分本身变成机械移动，评审只看依赖边 |
| L3 | core 不再直接调用录制、session 存储、trace、具体下载实现；下载改为注入接口（§4），录制、session、trace 的接入方式由调用观测的后续 RFC 定（§4.1） | 去掉 4 个进程全局单例；关闭 RFC-0039 G-a |
| L4 | 下载只保留一份实现：地址校验、重定向逐跳校验在 `aimux-provider-utils`，何时下载、大小上限在 `aimux` 的操作里 | 删除 core 自建的 reqwest client 和第二个重定向循环 |
| L5 | `aimux` 的源码目录按上游 `ai/src/<操作>/` 组织：每个操作一个目录，trait 只在 `aimux-provider` | `generate_video`、`transcribe`、`decide` 等离开 `*_model.rs` |
| L6 | `aimux-providers` 每个厂商一个目录；目录内文件与上游同名包一一对应（命名规则固定，数量随上游） | 删除“同名 `.rs` + 只有 options.rs 的目录”双形态；新增文件对照表并在 CI 校验 |
| L7 | 单元测试紧挨源文件（`foo.rs` 旁边 `foo_test.rs`，对应上游 `foo.test.ts`）；跨厂商场景测试合成一个集成测试二进制 | `aimux-providers/tests/` 从 126 个测试二进制收成 1 个；#278 能按文件名对照上游测试 |
| L8 | 一个共享错误枚举放在 `aimux-provider`（对齐文档 §2.2 已定），差异登记 | 每个错误补 `cause` 另开小 PR，不放进本文的拆分步骤 |

## 2. 现状

### 2.1 crate 依赖与体量

| crate | 依赖的 aimux crate | src 行数 | tests/ 行数 | 说明 |
|---|---|---|---|---|
| `aimux-core` | 无 | 35,958 | 7,710 | 规范 trait、类型、操作、retry、组合模型、录制、回放、trace、session 全在这里 |
| `aimux-stream` | 无 | 178 | 850 | SSE/行解析，已符合目标 |
| `aimux-provider-utils` | core、stream | 6,400 | 3,171 | 读 core 的录制上下文、下载守卫 |
| `aimux-providers` | core、stream、provider-utils | 79,776 | 60,279 | 81 个条目，126 个集成测试文件 |
| `aimux-ops` | core、providers、provider-utils | 10,934 | 10,319 | 宿主组装层 |
| `aimux-ffi` | ops | 694 | 426 | |
| `tools/aimux-cli`、`aimux-replay`、`aimux-web` | core、providers | 896 / 422 / 3,084 | — | |
| `tools/aimux-codegen` | ops | 553 | 483 | |
| `bindings/node`、`bindings/python` | core、ops | — | — | 不在 workspace 内 |

目标（对齐文档 §2.1）是 `aimux-provider` 在最底层且不依赖任何 aimux crate；现在最底层是包含运行时的 `aimux-core`，`aimux-provider` 和 `aimux-devtools` 都不存在。crates.io 上 `aimux`、`aimux-provider`、`aimux-devtools` 三个名字目前未被占用（2026-10-19 查询）。

### 2.2 同一个文件里混着两层

| 文件 | 规范层（去 `aimux-provider`） | 调用层（去 `aimux`） |
|---|---|---|
| `result.rs` | `GenerateResult`、`StreamResult`、`GenerateContent`、`Source` | `ContentPart`、`TypedToolResult`、`GeneratedFile`、`ReasoningOutput` 等 core 输出 |
| `stream_part.rs` | `StreamPart` | `TextStreamPart` |
| `tool.rs` | `FunctionTool`、`ProviderTool`、`LanguageModelV4ToolChoice` | `TypedToolCall`、用户侧 `ToolChoice` |
| `types.rs` | `FinishReason`、`Usage`、`Warning`、`ProviderMetadata` | `LanguageModelUsage` |
| `language_model_message.rs` | V4 prompt 类型 | `convert_to_language_model_prompt`（:1153）及 URL 下载 |
| 9 个 `*_model.rs` | 各模态 trait、CallOptions、结果 | `embed`、`generate_image`、`rerank`、`search`、`generate_speech`、`transcribe`、`stream_transcribe`、`generate_video`、`decide` |
| `message.rs`、`content.rs` | — | 用户输入 `ModelMessage`，按 RFC-0042 去 `aimux-provider-utils` |

上游对应关系：trait 在 `@ai-sdk/provider` 的 `<模态>-model/v4/`，操作在 `ai/src/<操作>/`（如 `generate-video/`、`transcribe/`、`decide/`），两者从不在同一个文件。

### 2.3 core 直接调用的地方

下面四条调用栈从公开入口一直走到问题所在的行。

**录制：规范层的 CallOptions 一路把录制上下文带进传输层**

```text
aimux_core::generate_text                         aimux-core/src/generate.rs:976
 └ RecordingContext::scope(context, retries…)     aimux-core/src/generate.rs:1153
   └ do_generate_with_logging                     aimux-core/src/generate.rs:2285
     └ model.do_generate(call_options)            aimux-core/src/generate.rs:2294
       └ OpenAI do_generate                       aimux-providers/src/openai/model.rs:166
         └ post_json_to_api                       aimux-providers/src/openai/model.rs:235
           └ HttpRequest::new(url, headers, opts) aimux-provider-utils/src/http.rs:168
             └ opts.recording_context()           aimux-provider-utils/src/http.rs:126
               └ RecordingContext::current()      aimux-core/src/recording.rs:695   ← provider-utils 读 core 的 task-local
           └ record_exchange(...)                 aimux-provider-utils/src/http.rs:647, :861
             └ Recorder::record_exchange          aimux-core/src/recording.rs:542   ← 传输层直接写录制
```

同样的直接调用还在 `decide`（`decision_model.rs:662` 读全局 recorder）、router（10 处）、MoA（6 处）、composite（4 处）。全局 recorder 是 `recording.rs:605` 的 `static RECORDER`。

**session：每次生成都去读进程全局的 session 存储**

```text
aimux_core::generate_text                          aimux-core/src/generate.rs:976
 └ session::resolve_session_id(settings.session_id, prompt)
                                                    aimux-core/src/generate.rs:1130
 └ session::session_store()                         aimux-core/src/session.rs:333
   └ static SESSION_STORE                           aimux-core/src/session.rs:320   ← 全局单例
 └ store.append(session_id, call_id, source)        aimux-core/src/session.rs:131
（stream_text 同样在 generate.rs:1657）
```

`session.rs` 里有三样东西：调用方给的 `session_id`（已经是操作设置）、前缀推断（`SESSION_INFERER`，`session.rs:321`，还读环境变量 `AIMUX_SESSION_INFER`）、会话索引 `SessionStore`（`session.rs:84`）。上游 `generateText` 没有 session，归组属于观测工具。

**下载：video 的真实下载实现在 core，而且和 provider-utils 各有一份**

```text
aimux_core::generate_video                         aimux-core/src/video_model.rs:463
 └ options.download.unwrap_or_else(|| create_download(None))
                                                    aimux-core/src/video_model.rs:532-535
 └ download.download(url, abort)                    aimux-core/src/video_model.rs:538
   └ create_download                                aimux-core/src/download.rs:106
     └ create_default_download_function             aimux-core/src/download.rs:185
       └ for redirects in 0..=MAX_DOWNLOAD_REDIRECTS aimux-core/src/download.rs:244   ← 第一个重定向循环
         └ reqwest::Client::builder()…               aimux-core/src/download.rs:259   ← core 自己建 HTTP client

provider 下载自家返回的 URL 时：
HttpRequest 发送                                    aimux-provider-utils/src/http.rs:429
 └ for redirect_count in 0..=MAX_REDIRECTS          aimux-provider-utils/src/http.rs:453 ← 第二个重定向循环
   └ aimux_core::download_guard::validate_download_target
                                                    aimux-core/src/download_guard.rs:101 ← 反向依赖 core
```

`transcribe`（`transcription_model.rs:493`）、prompt 中的 URL（`language_model_message.rs:13`）、生成文件（`generate.rs:100`）走的也是 core 这一份。上游只有一份：`ai/src/generate-video/generate-video.ts` 决定何时下载（`:136` 默认下载、`:387` 调用），`ai/src/util/download/download.ts:39` 调 `@ai-sdk/provider-utils` 的 `fetchUntrustedUrl`，地址校验和逐跳重定向都在 provider-utils。

**trace 与 replay：core 包一层 model，回放要靠 providers 反向重建**

```text
aimux_ops trace.create                             aimux-ops/src/ops/trace.rs:6
 └ aimux_core::trace::RingTraceStore / TraceLayer  aimux-core/src/trace/layer.rs:68, :520（impl LanguageModel）

aimux_ops replay.live（registry）                   aimux-ops/src/ops/replay.rs:333
 └ aimux_providers::replay::rebuild_provider        aimux-providers/src/replay.rs:58  ← core 不能依赖 providers，只好放这里
   └ aimux_core::replay::replay_with_model          aimux-core/src/replay.rs
```

上游 devtools 是外挂的：`registerTelemetry(DevToolsTelemetry())`（`ai/src/telemetry/telemetry-registry.ts:6`、`devtools/src/integration.ts:113`）加 `devToolsMiddleware()`（`devtools/src/middleware.ts:101`），`ai` 包不认识 devtools。

### 2.4 provider 目录与测试

上游不是“每个包文件数相同”，而是命名规则固定、一个模型类一个文件、测试紧挨源文件。13 个与 aimux 同名的上游包：

| 规则 | 出现的包数 |
|---|---|
| `<x>-provider.ts`、`*-options.ts`、`version.ts`、`index.ts` | 13 / 13 |
| `<x>-error.ts`、`*-api-types.ts` 或 `*-api.ts` | 11 / 13 |
| `<x>-chat-language-model.ts` / `<x>-language-model.ts` | 9 / 13（有文本模型的全部） |
| `convert-to-<x>-*.ts`、`*prepare-tools.ts` | 9 / 13 |
| 其余模态：每种模型一个 `<x>-<模态>-model.ts` + `-options.ts` | 有该模态的全部 |
| 测试 `foo.test.ts` 与 `foo.ts` 同目录 | 13 个包里 6 个全部如此，其余 7 个只有 1–5 个跨文件场景测试例外 |

| 厂商 | 上游源文件 / 测试文件 | aimux 源文件 / 行数 | aimux 测试文件 / 行数（都在 `tests/`） |
|---|---|---|---|
| openai | 80 / 34 | 21 / 11,812 | 13 / 9,231 |
| anthropic | 46 / 15 | 14 / 8,282 | 8 / 2,872 |
| google | 67 / 34 | 12 / 6,588 | 11 / 4,835 |
| amazon-bedrock | 35 / 24 | 10 / 5,059 | 6 / 3,737 |
| xai | 42 / 16 | 9 / 4,596 | 4 / 2,384 |
| mistral | 24 / 12 | 8 / 2,091 | 4 / 1,300 |
| deepseek | 17 / 7 | 9 / 1,857 | 2 / 2,636 |
| cohere | 16 / 5 | 7 / 1,815 | 2 / 742 |
| groq | 18 / 7 | 11 / 1,608 | 2 / 2,735 |
| elevenlabs | 13 / 3 | 2 / 1,231 | 3 / 993 |
| fal | 18 / 6 | 2 / 912 | 3 / 550 |
| luma | 6 / 2 | 2 / 474 | 1 / 190 |
| deepgram | 13 / 3 | 2 / 450 | 1 / 322 |

aimux 现状：

- 21 个厂商是“`<name>.rs` 放全部实现 + `<name>/` 目录里只有一个 `options.rs`”的双形态，其余是带 `mod.rs` 的目录。
- 文件名用通用名（`model.rs`、`convert.rs`、`types.rs`），不对应上游文件，无法按文件比对实现和测试。
- `tests/` 下 126 个文件，每个都是独立的测试二进制，各自链接整个 `aimux-providers`。其中 101 个能按前缀对应到厂商，8 个对应的厂商在 src 里没有同名条目（preset：alibaba、baseten、cerebras、deepinfra、fireworks、moonshotai、openrouter、perplexity），17 个是跨厂商场景（cassette、conformance、e2e 等）。

## 3. 目标分层

沿用对齐文档 §2.1 的五层，本文把每个现有模块的去向定下来。

```text
aimux-provider          V4 规范：Provider 与各模态 trait、CallOptions、V4 prompt/content/stream part、共享错误、middleware 规范
   ↑
aimux-provider-utils ← aimux-stream
                        Fetch/WS、headers、Resolvable、load_*、单次 HTTP helper、下载守卫与逐跳重定向、
                        传输诊断接口、脱敏、ModelMessage 用户输入、descriptor/catalogue 类型
   ↑
aimux-providers         厂商工厂与模型实现、preset、catalogue 摄取
aimux                   → aimux-provider、aimux-provider-utils
                        各操作、prompt 标准化与下载策略、retry/timeout、registry/custom/wrap、
                        operation scope、组合模型
aimux-devtools          → aimux、aimux-provider-utils
                        录制、回放、缓存审计、session 归组（推断 + 索引）；接入方式见 §4.1
aimux-ops / FFI / 绑定 / 工具   宿主组装
```

### 3.1 aimux-core 模块去向

| 去向 | 模块 |
|---|---|
| `aimux-provider` | `abort_signal`、`error`、`language_model`、`language_model_middleware`、`image_model_middleware`、`provider`（只留 `Provider`，`ProviderDiscovery` 作为厂商扩展去 providers，对齐文档 §8.1）、9 个 `*_model.rs` 的 trait/选项/结果部分、`result` / `stream_part` / `tool` / `types` 的规范部分、`language_model_message` 的 V4 prompt 类型、`options`（`CallOptions`、`ResponseFormat`）、`shared`、`timestamp`、`wire` |
| `aimux-provider-utils` | `message`、`content`（RFC-0042）、`download_guard`、`recording::is_sensitive_key` 与 `redact_json`（脱敏是传输层和录制共用的）、`model_catalogue`（descriptor 类型，对齐文档 §2.2 “无运行时依赖的注册条目与 descriptor 类型”） |
| `aimux` | `generate` 及其测试、`output`、`parse_tool_call`、`json_repair`、`openai_output`、`response_messages`、`step_result`、`result` / `stream_part` / `tool` / `types` 的调用层部分、各操作函数、`convert_to_language_model_prompt`、`download`（下载策略，传输改用 provider-utils）、`retry`、`timeout`、`operation`、`router`、`moa`、`composite`、`custom_provider`、`wrap_provider`、`provider_registry` |
| `aimux-devtools` | `recording`（schema、Recorder 实现）、`replay`、`decision_replay`、`trace/*`、整个 `session`（`resolve_session_id`、前缀推断 `SessionInferer`、`SessionStore`、`list_sessions`、`session_calls`） |
| `aimux-ops` | `aimux-providers/src/replay.rs` 的 `rebuild_provider`（宿主才有 registry；#270 的传输回放落地后删除，对齐文档 §8.3）。`aimux-providers/src/default_providers.rs` 留在 providers：它只按名字调各厂商工厂、返回 provider 表，不依赖运行时；唯一调 `create_provider_registry` 的是它的一个测试，随 C2 挪到 aimux 的集成测试 |

### 3.2 aimux 的目录

按上游 `ai/src/` 组织，每个操作一个目录，测试放在同一目录：

```text
aimux/src/
  generate_text/   generate_text.rs  stream_text.rs  step_result.rs  output.rs  parse_tool_call.rs  ...
  generate_object/
  embed/  generate_image/  generate_speech/  transcribe/  generate_video/  rerank/  search/  decide/
  prompt/          convert_to_language_model_prompt.rs  standardize_prompt.rs
  registry/        provider_registry.rs  custom_provider.rs  wrap_provider.rs
  middleware/      （调用层 wrap 函数；middleware 规范在 aimux-provider）
  composite/       router.rs  moa.rs
  util/            download.rs  retry.rs  timeout.rs
```

`aimux-provider` 同理按上游 `provider/src/<模态>-model/` 组织。

## 4. 依赖反转

| 现在 | 反转后 | 接口所在 | 实现所在 |
|---|---|---|---|
| provider-utils 读 `RecordingContext::current()` 并直接写 `Recorder`（`http.rs:126`、`:861`） | provider-utils 定义传输诊断：自己的 task-local 诊断作用域 + 观察者 trait，在 leaf 发出请求、响应、分块、错误事件（D4、RFC-0041 §3） | `aimux-provider-utils` | `aimux-devtools` 的 Recorder |
| generate / decide / router / MoA / composite 读全局 `RECORDER` 并自己建 `RecordingContext` | `aimux` 不再认识录制；录制在 `aimux-devtools`，接入方式由调用观测的后续 RFC 定（§4.1） | 后续 RFC | `aimux-devtools` |
| generate 解析 session、读全局 `SESSION_STORE` 并 `append`，再写进录制（`generate.rs:1129-1139`） | generate 不再认识 session：调用方给的 `session_id` 只是操作设置里的一个字符串，原样交给观测层；推断、索引、与录制的关联全部在 devtools（§4.1） | 后续 RFC | `aimux-devtools` |
| 前缀推断是全局 `SESSION_INFERER` + 环境变量 `AIMUX_SESSION_INFER` | 推断器是 devtools 里 session 归组的配置项，宿主开启，默认关闭；删除环境变量开关 | `aimux-devtools` | `aimux-devtools` |
| `TraceLayer` 是 core 里的 model 包装 | 缓存审计移到 devtools；指纹、LCP、真实性判定保留，接入方式由后续 RFC 定（§4.1） | 后续 RFC | `aimux-devtools` |
| `generate_video` / `transcribe` / prompt 下载默认构造 core 自带的 reqwest 下载 | 操作只依赖 `Download` 接口；默认实现调用 provider-utils 的不可信 URL 下载（一份地址校验、一份逐跳重定向） | `aimux` | `aimux-provider-utils` |
| 回放靠 providers 里的 `rebuild_provider` 反向重建 | 宿主（ops/CLI）组装 registry 交给回放；#270 后由传输层回放取代 | `aimux-devtools` | `aimux-ops` |

完成后 `aimux` 里不再有 `RECORDER`、`SESSION_STORE`、`SESSION_INFERER`、`INFER_ENV_CHECKED` 这四个全局量。operation 序号分配（`CALL_SEQ`）留在 `aimux`，由 operation runtime 持有（D30）。

### 4.1 调用观测留给后续 RFC

trace、日志和 telemetry 怎么统一（倾向 Rig 的 span 方案）单独成文，素材在 `observability-followup.md`，RFC-0044 合入后另起。本文只定两件事：

1. 录制、session 归组、缓存审计不再由 core 直接调用，最终落在 `aimux-devtools`（§3.1、上表）。它们接到调用上的方式由后续 RFC 定，本文不先造一套之后要换掉的事件接口（对齐文档 D31 不留过渡代码）。
2. 因此 C5、C6 中“录制、session、trace 搬进 devtools”这部分等后续 RFC 定稿再做；C1–C4、C7 和 provider 目录整理不受影响。

## 5. provider 目录与文件

### 5.1 规则

1. 每个厂商一个目录 `aimux-providers/src/<vendor>/`，入口 `mod.rs` 只做声明和导出（对应上游 `index.ts`）。删除 `<vendor>.rs` 与同名目录并存的形式。
2. 有上游包的厂商，目录内文件与上游包一一对应，文件名把 kebab-case 换成 snake_case 并去掉重复的厂商前缀：`mistral-chat-language-model.ts` → `mistral/chat_language_model.rs`，`map-mistral-finish-reason.ts` → `mistral/map_finish_reason.rs`，`convert-to-mistral-chat-messages.ts` → `mistral/convert_to_chat_messages.rs`。上游有子目录（如 openai 的 `chat/`、`responses/`、`embedding/`）就照建子目录。
3. 上游用 zod schema 写在模型文件里的响应类型，aimux 放进 `api_types.rs`（上游 11/13 的包本来就有 `*-api-types.ts` / `*-api.ts`）。
4. 上游没有、aimux 独有的文件（`list_models` 发现、SigV4 GET 签名等已登记的扩展）放在 `aimux_ext/` 子目录，文件头注明对应的登记差异；不混进上游同名文件。
5. 没有上游包的厂商（Exa、Tavily、Serper、Jev 等）用同一骨架：`mod.rs`、`provider.rs`、`<模态>_model.rs`、`<模态>_model_options.rs`、`api_types.rs`、`error.rs`。
6. 多个厂商共用的实现（OpenAI-chat 一族的流处理、轮询）留在 `src/shared/`、`src/stream/`，与 RFC-0042 T2 一致。
7. 新增 `aimux-providers/layout.toml`：每行写“aimux 文件 ↔ 上游文件”，没有对应时写登记原因。CI 检查两件事：目录里每个 `.rs` 都在表里；表里列的上游文件在基线版本中存在。基线升级时这张表就是待办清单。

### 5.2 测试位置

1. 单元测试紧挨源文件：`chat_language_model.rs` 末尾 `#[cfg(test)] #[path = "chat_language_model_test.rs"] mod tests;`，测试文件 `chat_language_model_test.rs` 对应上游 `mistral-chat-language-model.test.ts`。测试函数名保留上游 `it(...)` 的描述（snake_case），#278 的覆盖统计直接按文件和名字对照。
2. 上游的 `__fixtures__/`、`__snapshots__/` 对应到厂商目录下的 `fixtures/`、`snapshots/`；大文件（现有 176 MB 的 cassette）留在 crate 根目录的 `fixtures/` 按路径读取，不 `include_str!`。
3. 测试共用的 mock fetch、回放 helper（现 `tests/common/`）移到 `src/test_support/`，`#[cfg(test)]` 编译，不进发布产物。
4. 跨厂商场景测试（cassette、conformance、e2e、数据丢失回归等 17 个文件）和 preset 测试合成一个集成测试二进制 `tests/it/main.rs`，每个原文件变成其中一个模块。
5. 其他 crate 照同样规则：`aimux-provider-utils` 对照上游 provider-utils 文件，`aimux` 对照 `ai/src/<操作>/`。

迁移只搬动测试，不改断言、不删测试；每个 PR 前后用 `cargo test -- --list` 比较测试数量，数量必须相等，PR 描述里附对照。

## 6. 实施顺序

每一步一个 PR，按对齐文档 D31：改了哪个符号，就在同一 PR 里改完它在 ops、FFI、工具、Node/Python 绑定中的全部使用方；合入时 workspace 全绿；PR 描述附产物体积表（RFC-0043）。

| 步骤 | 内容 | 前置 |
|---|---|---|
| P1 | provider 目录与测试整理（§5），按厂商家族分批：OpenAI 一族与 compat、Anthropic、Google/Vertex、Bedrock、其余模态厂商、无上游厂商与 preset | 无；#268 只改 providers 的 2 个文件，可以现在开始 |
| C1 | core 内部按层拆开混合文件，操作函数搬进按操作分的目录（§2.2、§3.2），不新建 crate | #268 合入（它改了 core 的 36 个文件） |
| C2 | 新建 `aimux-provider`，搬入规范模块；core 改为依赖它 | C1 |
| C3 | provider-utils 定义传输诊断接口，core 的录制改为订阅；`ModelMessage`、下载守卫、脱敏、catalogue 类型搬入 provider-utils；日志初始化移到 ops（§4.1 第 3 条）；provider-utils 不再依赖 core | C2 |
| C4 | 下载归一（§4 下载一行）：video、transcribe、prompt、生成文件都走 provider-utils 的不可信 URL 下载；删除 core 的 reqwest client | C3 |
| C5 | 等调用观测的后续 RFC：按它把录制、session 归组接到调用上，去掉四个全局量 | C3、后续 RFC |
| C6 | 新建 `aimux-devtools`，搬入录制、回放、trace、session；`rebuild_provider` 移到 ops；providers 不再依赖 core | C5 |
| C7 | `aimux-core` 改名 `aimux`；发布顺序改为 provider → stream → provider-utils → providers → aimux → devtools → ops → ffi | C6 |

P1 与 C 系列互不阻塞。P1 先做，C2/C3 改 import 时对着已经整理好的目录一次改完。

与其他工作的关系：

- #270（schema 3 与传输回放）建在 C5、C6 的订阅接口上。先拆后做，schema 3 直接写进 `aimux-devtools`；反过来做则 schema 3 会先写进 core 再搬一次。
- #278（测试补齐）按 §5.2 的位置写新测试，避免补完再搬。
- #273（体积门禁）：providers 是否按厂商加 cargo feature 不在本文范围，见 §9。

## 7. 文档同步与登记差异

随各步骤 PR 一起改：

- 对齐文档 §2.1–§2.3：补模块去向（§3.1）、下载归属、session 整体归 devtools；§9.1 已知差距中“拆 crate”“OperationRuntime、DiagnosticScope”两行在 C3、C5、C6 完成时删除。
- RFC-0039 G-a（录制器、会话存储、推断状态为进程全局）在 C5 关闭。
- RFC-0042 §1 类型归属不变，按实际文件路径更新引用。
- RFC-0036 §3 加一句层与 crate 的对应：L0 传输 = provider-utils，L1 协议 = providers，L2 统一视图 = aimux-provider + aimux，L3 治理 = devtools。
- README 的 crate 表、CONTRIBUTING、`docs/api/`、CHANGELOG、`release.yml` 发布顺序。

登记差异（写入对齐文档 §5）：

- 共享错误枚举在 `aimux-provider`，含上游放在 `ai` 包里的 retry、工具调用类错误。原因：ops 错误封装和绑定只认一个联合类型（RFC-0039 D1）。
- session 归组、前缀推断是 aimux 扩展，上游 `generateText` 没有。它们只影响观测，不改变调用本身（推断出的 ID 不进入请求、不影响返回结果，见 `generate.rs:1129-1139`），所以整体放在 devtools，按上游 devtools 的方式以订阅接入。这改写对齐文档 C8 的 2026-10-13 修订（“推断与 generate 入口一起留在 `aimux`”），在 C5 的 PR 中同步改 C8 与 §3.6。
- provider 目录下 `aimux_ext/` 中的文件。

## 8. 验收

1. 依赖边检查（新增脚本，读 `cargo metadata`，进 CI）：`aimux-provider` 不依赖任何 aimux crate，也不依赖 reqwest、tokio 网络特性；`aimux-provider-utils` 只依赖 `aimux-provider`、`aimux-stream`；`aimux-providers` 不依赖 `aimux`、`aimux-devtools`；`aimux` 不依赖 `aimux-providers`、`aimux-devtools`；`aimux-devtools` 不依赖 `aimux-providers`。
2. `aimux` 源码里没有 `static` 可变全局（允许列表只含 operation 序号分配器和常量）。
4. workspace 里任何地方都不出现 `aimux_core`（C7 后）。
5. 每个 PR 的测试数量前后相等（`cargo test --workspace -- --list`），生成物重新生成后无 diff。
6. `layout.toml` 校验通过；`aimux-providers/tests/` 只剩 `it/` 一个二进制。
7. 产物体积不超过 RFC-0043 记录的当前值。

## 9. 开放问题与不采纳

- **按厂商拆 crate 还是加 feature。** 上游每个厂商一个 npm 包。Rust 侧可以一个厂商一个 crate（70 多个 crate，发布与版本管理成本高），也可以一个 `aimux-providers` 加每个厂商一个 cargo feature（体积门禁 #273 受益，绑定按需裁剪）。建议后者，单独决定，不阻塞本文。
- **`session_id` 的入口。** 本文保留它作为操作设置里的一个字段（绑定已暴露，RFC-0041 §10.2 已落地），`aimux` 只透传。完全照上游可以改成通用的 telemetry metadata，由 devtools 自己读出 session；这会改绑定 API，单独决定。
- **不采纳：保留 `aimux-core` 作为 re-export 门面。** 违反 D17 约束 6，且会让两个路径长期共存。
- **不采纳：一次性大 PR 完成全部拆分。** 改动面覆盖整个 workspace 和两个绑定，无法评审，也无法逐步定位回归。
- 每个错误补 `cause`（错误链）另开小 PR，与本文的搬移互不依赖；若先于 C2 合入，C2 原样搬移。
