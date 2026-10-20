# RFC-0044：crate 拆分与依赖反转

> **Status**: Draft
>
> **Date**: 2026-10-19
>
> **Scope**: 把 [docs/aisdk-architecture-alignment.md](../docs/aisdk-architecture-alignment.md) 第 2 章 D17 的五层目标落成可以逐步合入的计划：`aimux-core` 按职责拆入 `aimux-provider`、`aimux-provider-utils`、`aimux`、`aimux-devtools`；core 内部直接调用的录制、session、trace、下载改成依赖反转；各 crate 的源码和测试目录按上游包组织。`aimux-providers` 内部的厂商目录、测试位置和厂商间复用不在本文范围，见 RFC-0046（[PR #294](https://github.com/arcships/aimux/pull/294)），本文的步骤不依赖它。本文不改变 wire 格式和录制格式；公开行为只有一处变化：删除环境变量 `AIMUX_SESSION_INFER`（§7 登记）。
>
> **Baseline**: `master@5743a40a`（#265 合入后）。上游对照：`vercel/ai` main `fce9210`（目录与文件命名），行为基线仍按对齐文档登记的版本（`ai@7.0.127` 等）。
>
> **Related**: [对齐文档](../docs/aisdk-architecture-alignment.md) §2、§8、§9.1（D3、D4、D6、D17、D27、D28、D30）、[RFC-0036](0036-positioning-and-layered-architecture.md) §3、[RFC-0041](0041-replay-governance.md) §3 / §10、[RFC-0042](0042-v4-type-boundaries.md) §1、[RFC-0043](0043-size-performance-budgets.md)、[#276](https://github.com/arcships/aimux/issues/276)、[#270](https://github.com/arcships/aimux/issues/270)、[#273](https://github.com/arcships/aimux/issues/273)、[#278](https://github.com/arcships/aimux/issues/278)、[PR #268](https://github.com/arcships/aimux/pull/268)、RFC-0046（[PR #294](https://github.com/arcships/aimux/pull/294)，provider 目录、测试位置与厂商间复用）

## 1. 决策摘要

对齐文档已经定了目标分层（D17），但没有说怎么从现在的代码走过去，#276 里也只有“provider-utils 不再依赖 aimux-core”一行。本文补上这部分。

| 编号 | 决定 | 后果 |
|---|---|---|
| L1 | 按 D17 新建 `aimux-provider`、`aimux-devtools`，`aimux-core` 拆空后改名 `aimux`；不留兼容 re-export（对齐文档 §2.1 约束 6、D31） | 每一步在同一个 PR 里改完 ops、FFI、四个工具、Node/Python 绑定（各自独立的 workspace，Python 测试里还有名为 `aimux-core` 的替身包）的全部 import |
| L2 | 先在 core 内部按层拆开混在一起的文件，再切 crate | crate 切分本身变成机械移动，评审只看依赖边 |
| L3 | core 不再直接调用录制、session 存储、trace、具体下载实现；下载改为注入接口（§4）；传输层的调用关联用对齐文档 §3.5 的 `DiagnosticScope`（C3）；录制、session、trace 在调用层的接入方式由调用观测的后续 RFC 定（§4.1） | 去掉 4 个进程全局单例；关闭 RFC-0039 G-a |
| L4 | 下载只保留一份实现：地址校验、重定向逐跳校验在 `aimux-provider-utils`，何时下载、大小上限在 `aimux` 的操作里 | 删除 core 自建的 reqwest client 和第二个重定向循环 |
| L5 | `aimux` 的源码目录按上游 `ai/src/<操作>/` 组织：每个操作一个目录，trait 只在 `aimux-provider` | `generate_video`、`transcribe`、`decide` 等离开 `*_model.rs` |
| L6 | 各 crate 的文件和测试按上游包对照：`aimux-provider-utils` 对 provider-utils，`aimux` 对 `ai/src/<操作>/`，每个模块目录一个 `tests/` 子目录，一个上游测试文件对应其中一个同名 Rust 文件，crate 的 `tests/` 只留跨模块的场景测试；`aimux-providers` 内部另文 | #278 能按文件名对照上游测试 |
| L7 | 一个共享错误枚举放在 `aimux-provider`（对齐文档 §2.2 已定），差异登记 | 每个错误补 `cause` 另开小 PR，不放进本文的拆分步骤 |
| L8 | `aimux-stream` 并入 `aimux-provider-utils`，删除这个 crate；SSE 适配放进对应上游 `parse-json-event-stream.ts` 的文件，`extract_lines` 放进 `extract_lines.rs`，第三方 `sse-stream` 的位置相当于上游的 `eventsource-parser` | 少一个发布包和体积预算；改写对齐文档 D17“aimux-stream 保持独立”，登记（§7） |

## 2. 现状

### 2.1 crate 依赖与体量

| crate | 依赖的 aimux crate | src 行数 | tests/ 行数 | 说明 |
|---|---|---|---|---|
| `aimux-core` | 无 | 35,958 | 7,710 | 规范 trait、类型、操作、retry、组合模型、录制、回放、trace、session 全在这里 |
| `aimux-stream` | 无 | 178 | 850 | 约 120 行 SSE 适配（包在第三方 `sse-stream` 外）加 `extract_lines`；唯一调用方是 provider-utils `response_handler.rs:389`；`aimux-providers` 声明了依赖但没有使用；`extract_lines` 在 workspace 里没有调用方 |
| `aimux-provider-utils` | core、stream | 6,400 | 3,171 | 读 core 的录制上下文、下载守卫 |
| `aimux-providers` | core、stream、provider-utils | 79,776 | 60,279 | 48 个厂商实现（另有 282 个 preset 数据行），126 个集成测试文件 |
| `aimux-ops` | core、providers、provider-utils | 10,934 | 10,319 | 宿主组装层 |
| `aimux-ffi` | ops（dev 依赖 core） | 694 | 426 | |
| `tools/aimux-cli`、`aimux-replay`、`aimux-web` | core、providers | 896 / 422 / 3,084 | — | |
| `tools/aimux-codegen` | ops | 553 | 483 | |
| `bindings/node`、`bindings/python` | core、ops | — | — | 不在 workspace 内 |

目标（对齐文档 §2.1）是 `aimux-provider` 在最底层且不依赖任何 aimux crate；现在最底层是包含运行时的 `aimux-core`，`aimux-provider` 和 `aimux-devtools` 都不存在。crates.io 上 `aimux`、`aimux-provider`、`aimux-devtools` 三个名字目前未被占用（2026-10-19 查询）。

### 2.2 同一个文件里混着两层

| 文件 | 规范层（去 `aimux-provider`） | 调用层（去 `aimux`） |
|---|---|---|
| `result.rs` | `GenerateResult`、`StreamResult`（`LanguageModel` 的返回类型）、`GenerateContent`、`Source`、`ReasoningOutput`、`RawToolApprovalRequest`（约 10 个厂商在用） | `ContentPart`、`TypedToolResult`、`GeneratedFile` 等调用层输出 |
| `stream_part.rs` | `StreamPart` | `TextStreamPart`（它用到 `step_result` 的类型，`stream_part.rs:19`，随它一起走）；`StreamPart::into_text_stream_part`（`stream_part.rs:456-461`，inherent impl，调用方是 `generate.rs:1903` 和 `ai_sdk_json_test.rs:488-502`）改成 `aimux` 里的自由函数，放在 stream_text 旁边：`StreamPart` 在 `aimux-provider`，`aimux` 不能给它写 inherent impl |
| `tool.rs` | `FunctionTool`、`ProviderTool`、`LanguageModelV4ToolChoice`、`RawToolCall`、`ToolResult` | `TypedToolCall`、用户侧 `ToolChoice`、`prepare_tool_choice`（`tool.rs:379`） |
| `types.rs` | `FinishReason`、`FinishReasonUnified`、`Usage`、`InputTokenUsage`、`OutputTokenUsage`、`Warning`、`ProviderMetadata`、`ReasoningEffort`、`ResponseMetadata`、`NamespacedKind` | `LanguageModelUsage` |
| `options.rs` | `CallOptions`、`ResponseFormat` | `options.rs:14` 对 `ToolChoice`、`prepare_tool_choice` 的 re-export 删除 |
| `language_model_message.rs` | V4 prompt 类型 | `convert_to_language_model_prompt`、`download_assets`、`convert_with_downloaded_assets` |
| `download.rs` | `Download`、`DownloadRequest`、`DownloadedFile`、`DownloadFunction`（`TranscriptionCallOptions.download`、`VideoCallOptions.download` 字段的类型，`transcription_model.rs:154`） | `create_download`、`create_default_download_function`、下载实现 |
| 11 个 `*_model.rs` | 各模态 trait、CallOptions、结果（含 `decision_model.rs` 的 trait、类型和校验，`files_model.rs`、`skills_model.rs`） | `embed`、`generate_image`、`rerank`、`search`、`generate_speech`、`transcribe`、`stream_transcribe`、`generate_video`、`decide` / `DecideOptions` |
| `provider.rs` | `Provider`、`ProviderDiscovery`；`list_models` 返回的 `RuntimeModel`（`provider.rs:19`，现在在 `model_catalogue.rs`）一起下移。`Provider::discovery()` 返回 `Option<&dyn ProviderDiscovery>`（`provider.rs:142`），trait 只能和 `Provider` 在同一层，否则 `aimux-provider` 要反过来依赖 `aimux-providers` | — |
| `message.rs`、`content.rs` | `content.rs` 只依赖 shared/types/wire，V4 prompt 用到它，去 `aimux-provider` | 用户输入 `ModelMessage`（`message.rs`）按 RFC-0042 去 `aimux-provider-utils` |

上游对应关系：trait 在 `@ai-sdk/provider` 的 `<模态>-model/v4/`，操作在 `ai/src/<操作>/`（如 `generate-video/`、`transcribe/`、`decide/`），两者从不在同一个文件。

### 2.3 core 直接调用的地方

下面四条调用栈从公开入口一直走到问题所在的行。

**录制：规范层的 CallOptions 一路把录制上下文带进传输层**

```text
aimux_core::generate_text                          aimux-core/src/generate.rs:976
 └ generate_in_operation                           aimux-core/src/generate.rs:1062
   └ recording::recorder()                         aimux-core/src/generate.rs:1101   ← 读全局 RECORDER（recording.rs:605）
   └ RecordingContext::scope(context, …)           aimux-core/src/generate.rs:1153   ← 设 task-local
     └ do_generate_with_logging                    aimux-core/src/generate.rs:2288
       └ model.do_generate(call_options)           aimux-core/src/generate.rs:2294
         └ OpenAI do_generate                      aimux-providers/src/openai/model.rs:166
           └ execute_generate                      aimux-providers/src/openai/model.rs:174 → :221
             └ config.http_request(url, headers, options)
                                                    aimux-providers/src/openai/config.rs:180
               └ HttpRequest::new                  aimux-provider-utils/src/http.rs:168
                 └ options.recording_context()     aimux-provider-utils/src/http.rs:178
                   └ RecordingContext::current()   aimux-provider-utils/src/http.rs:126 → aimux-core/src/recording.rs:695
                                                                                      ← provider-utils 读 core 的 task-local
             └ post_json_to_api                    aimux-providers/src/openai/model.rs:235
               └ record_exchange                   aimux-provider-utils/src/http.rs:647 → :861
                 └ Recorder::record_exchange       aimux-provider-utils/src/http.rs:865 → aimux-core/src/recording.rs:568
                                                                                      ← 传输层直接写录制
```

读全局 recorder 的只有三处：`generate.rs:1101`（generate_text）、`:1621`（stream_text）、`decision_model.rs:662`（decide）。router（10 处）、MoA（6 处）、composite（4 处）读写的是 task-local 的 `RecordingContext`。

**session：每次生成都去读进程全局的 session 存储**

```text
aimux_core::generate_text                          aimux-core/src/generate.rs:976
 └ generate_in_operation                           aimux-core/src/generate.rs:1062
   └ session::resolve_session_id(settings.session_id, prompt)
                                                    aimux-core/src/generate.rs:1130
   └ session::session_store()                      aimux-core/src/generate.rs:1131 → aimux-core/src/session.rs:333
     └ static SESSION_STORE                        aimux-core/src/session.rs:320   ← 全局单例
   └ store.append(session_id, call_id, source)     aimux-core/src/session.rs:131
（stream_text 同样在 generate.rs:1657）
```

`session.rs` 里有三样东西：调用方给的 `session_id`（已经是操作设置）、前缀推断（`SESSION_INFERER`，`session.rs:321`，还读环境变量 `AIMUX_SESSION_INFER`）、会话索引 `SessionStore`（`session.rs:84`）。上游 `generateText` 没有 session，归组属于观测工具。

**下载：video 的真实下载实现在 core，而且和 provider-utils 各有一份**

```text
aimux_core::generate_video                         aimux-core/src/video_model.rs:463
 └ options.download.unwrap_or_else(|| create_download(None))
                                                    aimux-core/src/video_model.rs:535
 └ download.download(url, abort)                   aimux-core/src/video_model.rs:538 → aimux-core/src/download.rs:83
   └ create_download 建的下载函数                    aimux-core/src/download.rs:106
     └ download                                    aimux-core/src/download.rs:222
       └ download_inner                            aimux-core/src/download.rs:239
         └ for redirects in 0..=MAX_DOWNLOAD_REDIRECTS
                                                    aimux-core/src/download.rs:244   ← 第一个重定向循环
           └ reqwest::Client::builder()…           aimux-core/src/download.rs:259   ← core 自己建 HTTP client

provider 下载自家返回的 URL 时：
send_following_redirects                           aimux-provider-utils/src/http.rs:429
 └ aimux_core::download_guard::validate_download_target
                                                    aimux-provider-utils/src/http.rs:444 → aimux-core/src/download_guard.rs:101
                                                                                      ← 反向依赖 core
 └ for redirect_count in 0..=MAX_REDIRECTS         aimux-provider-utils/src/http.rs:453 ← 第二个重定向循环（每跳再校验，:504）
```

`transcribe`（`transcription_model.rs:493`）也走 `create_download`；prompt 中的 URL 走 `create_default_download_function`（`download.rs:185`，由 `generate.rs:1048` 和 ops 的 `handles.rs:136` 调用），底下是同一份实现。上游只有一份：`ai/src/generate-video/generate-video.ts` 决定何时下载（`:136` 默认下载、`:387` 调用），`ai/src/util/download/download.ts:39` 调 `@ai-sdk/provider-utils` 的 `fetchUntrustedUrl`，地址校验和逐跳重定向都在 provider-utils。

**trace 与 replay：core 包一层 model，回放要靠 providers 反向重建**

```text
aimux_ops trace.create                             aimux-ops/src/ops/trace.rs:99
 └ aimux_core::trace::RingTraceStore::with_capacity aimux-ops/src/ops/trace.rs:113 → aimux-core/src/trace/store.rs:395
宿主包模型时                                         aimux-ops/src/ops/handles.rs:212
 └ aimux_core::trace::TraceLayer::new              aimux-core/src/trace/layer.rs:68，:520（impl LanguageModel）

aimux_ops replay.live（registry）                   aimux-ops/src/ops/replay.rs:333
 └ aimux_providers::replay::rebuild_provider       aimux-providers/src/replay.rs:58  ← core 不能依赖 providers，只好放这里
 └ aimux_core::replay::replay_with_model           aimux-ops/src/ops/replay.rs:335
（decide 回放同理：aimux-ops/src/ops/replay.rs:339 → aimux-providers/src/replay.rs:26 rebuild_decision_provider）
```

上游 devtools 是外挂的：`registerTelemetry(DevToolsTelemetry())`（`ai/src/telemetry/telemetry-registry.ts:6`、`devtools/src/integration.ts:113`）加 `devToolsMiddleware()`（`devtools/src/middleware.ts:101`），`ai` 包不认识 devtools。两者都在模型调用层：前者收 telemetry 事件，后者是模型 middleware。上游没有 Fetch 层的观察者，传输层录制是 aimux 的扩展（RFC-0036 §3 的 L0），登记见 §7。上游 devtools 在 `NODE_ENV === 'production'` 时直接抛错（`middleware.ts:102-107`、`integration.ts:116-121`）。

### 2.4 provider 目录与测试

厂商目录、测试文件和厂商之间的引用不在本文范围。本文只关心 crate 之间的边：`aimux-providers` 现在依赖 core，C6 之后不再依赖。

## 3. 目标分层

沿用对齐文档 §2.1 的五层，本文把每个现有模块的去向定下来。

```text
aimux-provider          V4 规范：Provider 与各模态 trait、CallOptions、V4 prompt/content/stream part、共享错误、middleware 规范
   ↑
aimux-provider-utils    SSE 解码（原 aimux-stream）、Fetch/WS、headers、Resolvable、load_*、单次 HTTP helper、下载守卫与逐跳重定向、
                        Fetch 观察点、脱敏、ModelMessage 用户输入、descriptor/catalogue 类型
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
| `aimux-provider` | `abort_signal`、`error`、`language_model`、`language_model_middleware`、`image_model_middleware` 的规范部分、`provider`（`Provider` 和 `ProviderDiscovery`）、11 个 `*_model.rs` 的 trait/选项/结果部分、§2.2 表中规范层一列的类型（含下载接口类型和 `RuntimeModel`）、`content`、`language_model_message` 的 V4 prompt 类型、`options`、`shared`、`timestamp`、`wire` |
| `aimux-provider-utils` | `message`（RFC-0042）、`download_guard`、`recording::is_sensitive_key` 与 `redact_json`（`recording.rs:802`、`:821`，脱敏是传输层和录制共用的）、`model_catalogue` 除 `RuntimeModel` 外的 descriptor 类型（对齐文档 §2.2 “无运行时依赖的注册条目与 descriptor 类型”）、Fetch 观察点交出的请求与响应记录类型（现在的 `HttpExchange` 等，`http.rs:15-16`；录制 schema 本身在 devtools） |
| `aimux` | `generate` 及其测试、`output`、`parse_tool_call`、`json_repair`、`math`（`cosine_similarity`，上游在 `ai`）、`openai_output`、`response_messages`、`step_result`、§2.2 表中调用层一列、`download`（下载策略，传输改用 provider-utils）、`retry`、`timeout`、`operation`、`router`、`moa`、`composite`、`custom_provider`、`wrap_provider`、`wrap_language_model` / `wrap_image_model`、`provider_registry` |
| `aimux-devtools` | `recording`（schema、Recorder 实现、`RingRecorder`）、`replay`（含 `MockReplayModel`）、`decision_replay`（含 `MockDecisionReplayModel`）、`trace/*`、整个 `session`（`resolve_session_id`、前缀推断 `SessionInferer`、`SessionStore`、`list_sessions`、`session_calls`）。ops、CLI、Web 现在用的这几个公开类型由 devtools 导出 |
| `aimux-ops` | `aimux-providers/src/replay.rs` 的 `rebuild_provider` 和 `rebuild_decision_provider`，连同它们的测试（测试调 `create_provider_registry` 并读 core 的录制样例，`replay.rs:90`、`:101`、`:107`）；宿主才有 registry，#270 的传输回放落地后删除（对齐文档 §8.3） |
| 留在 `aimux-providers` | `default_providers.rs`：它只按名字调各厂商工厂、返回 provider 表，不依赖运行时；它唯一调 `create_provider_registry` 的测试随 C2 挪到 aimux 的集成测试 |
| 拆掉 | `util.rs`（只是 re-export 门面）；`lib.rs` 的根 re-export 和 `prelude` 各 crate 按自己的内容重写 |

provider-utils 里已有的 `decision_language_model.rs` 随 decide 的规范类型改 import，位置不变。

### 3.2 aimux 的目录

按上游 `ai/src/` 组织，每个操作一个目录，测试放在该目录的 `tests/` 子目录（§5）：

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

### 3.3 测试归属

1. 测试跟着被测代码走；一个测试同时用到上下两层时，放到较上面那个 crate。
2. 不允许下层 crate 的测试 dev-depend 上层 crate：dev 依赖造成的环会让同一个类型在测试里出现两份。现在 providers、provider-utils 的测试用到 download、operation、openai_output、generate、replay、`RingRecorder`，这些测试搬到 `aimux`、`aimux-devtools` 或 ops 的集成测试。
3. 只测录制、trace、缓存审计的（`cache_probe_test`、`trace_stream_text_test`、`trace_recording_correlation_test`、`master_recording_test`）去 devtools；`operation_settings_test` 按内容拆成 aimux 和 devtools 两份；`ai_sdk_json_test` 按类型族拆；`custom_provider_test` 随 `custom_provider` 去 `aimux`（`ProviderDiscovery` 在 `aimux-provider`，它不需要 providers）。
4. 各 crate 自己的测试辅助（mock model、脚本模型）放在本 crate 的 `test_support` 模块，`#[cfg(any(test, feature = "test-support"))]` 编译：只用 `#[cfg(test)]` 的话，`tests/` 下的集成测试和下游 crate 的测试都看不到它。本 crate 的集成测试经自引用的 dev 依赖（`[dev-dependencies] aimux-x = { path = ".", features = ["test-support"] }`）开启，下游 crate 在自己的 dev 依赖里开启；正常构建不带这个 feature；core 测试里重复 3 次的 `ScriptedModel` 合成一份。providers 的 `tests/common/replay.rs:93` 用到录制类型，C6 前改成用 provider-utils 的观察记录类型。

## 4. 依赖反转

| 现在 | 反转后 | 接口所在 | 实现所在 |
|---|---|---|---|
| provider-utils 读 `RecordingContext::current()` 并直接写 `Recorder`（`http.rs:126`、`:861`） | provider-utils 不再认识录制：在 Fetch 链最内层留观察点，包住用户或默认的叶子 Fetch，鉴权之后、真正发出之前（对齐文档 §3.2 的顺序、D4 的叶子位置），请求、响应、分块、错误都经过它；和 D5 的 `replay_fetch` 是同一个接口（RFC-0041 §3）。exchange 归属哪个调用，用对齐文档 D3、§3.5 已定的传输可见 scope（`DiagnosticScope`，task-local，放在 provider-utils，内容见 §4.2）：`aimux` 的操作在调用模型前建立它，观察点从当前 scope 取调用关联交给观察者；流式响应的 body 在发出时就拿到关联，不依赖之后 poll 所在的 task-local（§3.5）。这对应上游每个 telemetry 事件都带的 `callId`（`ai/src/generate-text/generate-text.ts:669` 生成，`ai/src/telemetry/telemetry.ts:332-337` 的 `executeLanguageModelCall` 让模型调用跑在集成提供的上下文里）。C3 就按这个做，C3 到 C5 之间录制不丢调用关联 | `aimux-provider-utils` | `aimux-devtools` 的 Recorder |
| generate / decide / router / MoA / composite 读全局 `RECORDER` 并自己建 `RecordingContext` | `aimux` 不再认识录制；录制在 `aimux-devtools`，接入方式由调用观测的后续 RFC 定（§4.1） | 后续 RFC | `aimux-devtools` |
| generate 解析 session、读全局 `SESSION_STORE` 并 `append`，再写进录制（`generate.rs:1129-1139`） | generate 不再认识 session：调用方给的 `session_id` 只是操作设置里的一个字符串，原样交给观测层；推断、索引、与录制的关联全部在 devtools（§4.1） | 后续 RFC | `aimux-devtools` |
| 前缀推断是全局 `SESSION_INFERER` + 环境变量 `AIMUX_SESSION_INFER` | 推断器是 devtools 里 session 归组的配置项，宿主开启，默认关闭；删除环境变量开关 | `aimux-devtools` | `aimux-devtools` |
| `TraceLayer` 是 core 里的 model 包装 | 缓存审计移到 devtools；指纹、LCP、真实性判定保留，接入方式由后续 RFC 定（§4.1） | 后续 RFC | `aimux-devtools` |
| `generate_video` / `transcribe` / prompt 下载默认构造 core 自带的 reqwest 下载 | 操作只依赖 `Download` 接口；默认实现调用 provider-utils 的不可信 URL 下载（一份地址校验、一份逐跳重定向） | `aimux` | `aimux-provider-utils` |
| 回放靠 providers 里的 `rebuild_provider` 反向重建 | 宿主（ops/CLI）组装 registry 交给回放；#270 后由传输层回放取代 | `aimux-devtools` | `aimux-ops` |

完成后 `aimux` 里不再有 `RECORDER`、`SESSION_STORE`、`SESSION_INFERER`、`INFER_ENV_CHECKED` 这四个全局量。两个 `CALL_SEQ` 计数器（`recording.rs:771`、`session.rs:406`）和 `TRACE_SEQ`（`trace/layer.rs:14`）随各自模块去 devtools；`operation.rs` 只有 task-local 的操作设置（`CURRENT`，`operation.rs:25`），留在 `aimux`。

不在本文处理、保持现状的全局量：`DEFAULT_TRANSPORT`（provider-utils `fetch.rs:221-281`，上游同样有默认 fetch）、各厂商的默认实例 `DEFAULT`（上游同样有）；日志初始化（`logging.rs` 的 `INIT`、`AIMUX_LOG`、`AIMUX_LOG_BODY`）交给调用观测的后续 RFC。

### 4.1 调用观测留给后续 RFC

trace、日志和 telemetry 怎么统一（倾向 Rig 的 span 方案）单独成文，RFC-0044 合入后另起。对齐文档已经写了一些相关决定（D4、D6、D27，§3.2 的 `DiagnosticScope` 放在 provider-utils，§3.5 的 task-local 诊断作用域），后续 RFC 可能修改它们，届时在那份 RFC 里登记。本文只定两件事：

1. 传输层由本文定：Fetch 观察点和 `DiagnosticScope`（上表第一行，C3）。录制、#270 的传输回放都建在它上面。
2. 调用层（模型调用开始与结束、session 归组、缓存审计）不再由 core 直接调用，最终落在 `aimux-devtools`（§3.1、上表）。它们接到调用上的方式由后续 RFC 定，本文不先造一套之后要换掉的调用层事件接口（对齐文档 D31 不留过渡代码）。
3. 因此 C5、C6 中“session、trace 搬进 devtools”这部分等后续 RFC 定稿再做；C1–C4、C7 不受影响。

### 4.2 `DiagnosticScope` 的内容

只带 `call_id` 不够：录制要知道一次交换属于哪个模型调用的第几次尝试、是这次尝试里的第几次交换、交给哪个观察者（RFC-0048 §3 的 `exchangeId: "<seq>.<attempt>.<index>"`、§4.1 的要求，[PR #325](https://github.com/arcships/aimux/pull/325)）。C3 定下的 scope：

| 字段 | 谁设置 | 用途 |
|---|---|---|
| `call_id` | `aimux` 的操作入口 | 对应上游 telemetry 事件的 `callId` |
| `model_call: { seq, attempt, parent }` | 操作是 seq 0；router 子模型、MoA 的每个 reference 和 aggregator 进入子 scope，取新 seq 并记下 `parent`；retry 每次尝试递增 `attempt`，不换 seq | 区分组合模型的子调用和重试 |
| 交换序号 | 观察点在每个 `(seq, attempt)` 内从 1 递增，scope 里只放计数器 | 同一次尝试里的多次交换（重定向每一跳单独计一次） |
| `observers` | 宿主经操作设置装上（devtools 的 Recorder 是其中一个）；为空时观察点直接透传，不复制 body | 取代全局 `RECORDER`：交换只交给本次操作的观察者 |

seq 和 attempt 在 C3 就由 `aimux` 的 retry 循环和 router、MoA、composite 设置（它们是传输可见的关联，不是调用层事件）；调用开始、结束等调用层事件仍由后续 RFC 定（§4.1）。流式响应在发出时就把这一组值交给观察者，之后 poll 的 task 不需要 scope。

### 4.3 下载归一的对等要求

C4 是重构，不改各条下载路径现在可观察的行为。两份实现现在的差别（master@`c0622ea4`）：

| 项 | 操作的下载（video、transcribe、prompt，`download.rs`） | provider 下载自家 URL（`http.rs:429-520`） | C4 之后 |
|---|---|---|---|
| 代理 | `no_proxy()`，只连校验过的地址（`download.rs:259-263`） | 走默认传输的代理设置（D26，`http.rs:458-466`） | 各自保持：不可信 URL 下载带一个连接策略参数（直连 / 默认传输的代理），操作用直连 |
| User-Agent | `aimux/<版本>`（`download.rs:266-269`） | 请求自带的 UA，跨源只留 UA（`http.rs:515`） | 各自保持 |
| 错误 | `Other`（状态码、大小）、`InvalidArgument`（地址）；RFC-0047（#290）之后带 cause、不带 URL | 状态与重定向错误是 `ApiCall`（`http.rs:400-406`） | 各自保持现有变体和消息 |
| 重定向 | 最多 10 跳，逐跳校验 | 最多 10 跳，逐跳校验，跨源去掉调用方 header | 同一个循环 |
| 大小上限 | 默认 2 GiB | 默认 2 GiB（`read_response_with_size_limit.rs:12`） | 同一个常量 |

上游 `ai/src/util/download/download.ts:33-71` 用 `ai-sdk/<版本>` 加运行时 UA、经 `fetchUntrustedUrl` 下载、失败抛 `DownloadError`（带 url、状态码）。是否把操作下载的 UA 和错误改成上游的样子，见 §9，不在 C4 里顺带改。

## 5. 目录与测试位置

1. `aimux-provider-utils` 的文件对照上游 provider-utils，`aimux` 对照 `ai/src/<操作>/`（§3.2），文件名把 kebab-case 换成 snake_case。
2. 测试照 RFC-0046 §5 的厂商测试目录：每个模块目录一个 `tests/` 子目录，目录的 `mod.rs`（crate 根目录是 `lib.rs`）末尾一行 `#[cfg(test)] mod tests;`；一个上游测试文件对应 `tests/` 下一个与被测源文件同名的 Rust 文件，如 `ai/src/generate-text/generate-text.test.ts` → `aimux/src/generate_text/tests/generate_text.rs`，上游 provider-utils 的 `src/post-to-api.test.ts` → `aimux-provider-utils/src/tests/post_to_api.rs`。直接调内部函数的和走公开 API 的都放这里，不用 `#[path]`，源文件里不写测试；被测的私有函数改成 `pub(super)`。crate 的 `tests/` 只留跨模块的场景测试。
3. `aimux-providers` 内部的目录和测试搬移不在本文范围。

## 6. 实施顺序

每一步一个 PR，按对齐文档 D31：改了哪个符号，就在同一 PR 里改完它在 ops、FFI、工具、Node/Python 绑定中的全部使用方；合入时 workspace 全绿，Node、Python 绑定（各自独立的 workspace）也编译通过；PR 描述附产物体积表（RFC-0043）。新建的 crate 在同一个 PR 里登记 RFC-0043 的体积预算项，否则体积检查报未登记。

| 步骤 | 内容 | 前置 |
|---|---|---|
| C0 | `aimux-stream` 并入 provider-utils（L8）：源码和 850 行测试搬进 provider-utils，删掉 providers 里没用的依赖，删除 crate，README、`release.yml`、`logging.rs:139` 的 target 列表同步。`aimux-stream` 0.5.0 已发布到 crates.io，处理方式和 `aimux-core` 一样（§9）：删除前用它当前的代码发最后一个版本，README 和 crate 文档第一行写明已并入 `aimux-provider-utils`、不再维护，不撤回旧版本；CHANGELOG 记一条 | 无 |
| C1 | core 内部按层拆开混合文件，操作函数搬进按操作分的目录（§2.2、§3.2），不新建 crate | #268 合入（它改了 core 的 36 个文件） |
| C2 | 新建 `aimux-provider`，搬入规范模块；core 改为依赖它。先断掉规范层往上的三条边：`RuntimeModel` 下移、下载接口类型下移、`options.rs:14` 的 re-export 删除（§2.2） | C1 |
| C3 | provider-utils 在 `Fetch` 层留观察点，core 的录制改为观察者；HTTP 层不再读 `RecordingContext`，文本生成和 decide 两条路径（`http.rs:118-128`、`:130-140`）以及写录制（`:861-867`）都改为从 `DiagnosticScope` 取调用关联（§4 第一行、§4.2），retry 设置 attempt，router、MoA、composite 建立子 scope；`ModelMessage`、下载守卫、脱敏、catalogue 类型搬入 provider-utils；provider-utils 不再依赖 core | C2 |
| C4 | 下载归一（§4 下载一行）：video、transcribe、prompt、生成文件都走 provider-utils 的不可信 URL 下载，各路径的代理、UA、错误照 §4.3 保持；删除 core 的 reqwest client | C3 |
| C5 | 等调用观测的后续 RFC：按它把录制、session 归组接到调用上，去掉四个全局量 | C3、后续 RFC |
| C6 | 新建 `aimux-devtools`，搬入录制、回放、trace、session；`rebuild_provider` 移到 ops；providers 不再依赖 core | C5 |
| C7 | `aimux-core` 改名 `aimux`；发布顺序改为 provider → provider-utils → providers → aimux → ops → ffi（C6 之后在 aimux 和 ops 之间加 devtools）；生成物重新生成后检查不含 `aimux_core`（#268 删掉了 `aimux-ops/src/manifest/controls.rs` 和 `schema/ops/type-*.json`，master@`c0622ea4` 的 `schema/` 里已经没有 `aimux_core` 路径；ops 里 30 处 `TypeRef::of::<aimux_core::…>` 是 Rust 路径，随改名一起改）；CLI 的二进制也叫 `aimux`，给它设 `doc = false`，避免和库的文档目录冲突 | C4 |

C5、C6 要等调用观测的后续 RFC（§4.1）；改名 C7 不依赖它们，C4 之后就可以做，`aimux` 里暂时留着的录制、session、trace 在 C5、C6 再搬出。`aimux-providers` 内部的目录整理与 C 系列互不阻塞；它先做的话，C2/C3 改 import 时对着整理好的目录一次改完。

与其他工作的关系：

- #270（schema 3 与传输回放）建在 C3 的 Fetch 观察点和 C6 的 `aimux-devtools` 上。先拆后做，schema 3 直接写进 `aimux-devtools`；反过来做则 schema 3 会先写进 core 再搬一次。
- #278（测试补齐）按 §5 的位置写新测试，避免补完再搬；`aimux-providers` 内部的测试位置另文。
- #273（体积门禁）：本文不给 providers 加按厂商的 cargo feature。

## 7. 文档同步与登记差异

随各步骤 PR 一起改：

- 对齐文档 §2.1–§2.3：补模块去向（§3.1）、下载归属、session 整体归 devtools；§9.1 已知差距中“拆 crate”“OperationRuntime、DiagnosticScope”两行在 C3、C5、C6 完成时删除。
- RFC-0039 G-a（录制器、会话存储、推断状态为进程全局）在 C5 关闭。
- RFC-0042 §1 类型归属不变，按实际文件路径更新引用。
- README 的 crate 表和 `init_recording` 示例（README:198-203）、CONTRIBUTING、`docs/api/`（含 `decision.md:403-434`）、`docs/rfc-0039-implementation.md`、`docs/ai-sdk-request-pipeline.md`（:622-625、:933）、`upgrading-to-ops-abi.md`、CHANGELOG（只加新条目，不改历史）、`release.yml` 发布顺序。
- RFC-0036 §3 的 L0 现在包含 retry 和录制；拆分后 retry 在 `aimux`、录制在 devtools，对应关系照实写，不是简单的一层对一个 crate。
- 对齐文档 §6.5 提到的 `aimux-manifest` crate 不在本文范围，本文不新建它。

登记差异（写入对齐文档 §5）：

- 共享错误枚举在 `aimux-provider`，含上游放在 `ai` 包里的 retry、工具调用类错误。原因：ops 错误封装和绑定只认一个联合类型（RFC-0042）。这与对齐文档 §2.1 约束 1 “`aimux-provider` 不包含 retry”冲突，见 §9。
- session 归组、前缀推断是 aimux 扩展，上游 `generateText` 没有。它们只影响观测，不改变调用本身（推断出的 ID 不进入请求、不影响返回结果，见 `generate.rs:1129-1139`），所以整体放在 devtools；它接到调用上的方式由后续 RFC 定（§4.1）。这改写对齐文档 C8 的 2026-10-13 修订（“沿用现有的 opt-in 前缀推断实现，与 generate 入口一起留在 `aimux`”）和 RFC-0041 中“`aimux` 拥有 SessionInferer 注册接口”一句，在 C5 的 PR 中同步改 C8、§3.6 和 RFC-0041。
- 对齐文档 §3.5 的“关闭观测”表写着关闭观测时 session 和 session 推断仍然执行。session 整体进 devtools 后，没有装 devtools 就没有归组；ops 默认装上 devtools，所以绑定用户行为不变，只有自己组装、不装 devtools 的 Rust 用户没有归组。见 §9。
- 删除环境变量 `AIMUX_SESSION_INFER`：它是公开开关（RFC-0024、Python 绑定 `wrapper.py:920` 的文档），改为 devtools 的配置项，算一次公开行为变化。
- `rebuild_provider` 搬到 ops 是过渡代码（对齐文档 D31、RFC-0041 §12 G3 不鼓励过渡），#270 的传输回放落地后删除。
- 录制在 Fetch 层（RFC-0036 §3 的 L0）：上游 devtools 是 telemetry 集成和模型 middleware（§2.3），没有 Fetch 层观察者。Fetch 观察点和 `DiagnosticScope` 是 aimux 扩展。
- devtools 用于生产：上游 devtools 在 `NODE_ENV === 'production'` 时抛错（`devtools/src/middleware.ts:102-107`、`integration.ts:116-121`），aimux 的录制和回放是产品功能（RFC-0023、RFC-0041），ops 默认装上 devtools，生产环境照常可用。
- `aimux-stream` 不再是独立 crate（L8）：对齐文档 D17、§0.3 和 §2.1（含约束 2）写的“aimux-stream 保持独立的 SSE 解析库、保留自己的解析错误”改为 SSE 解码在 provider-utils 内部，`SseError` 仍在边界转换为共享错误，与上游 `parseJsonEventStream` 包住 `eventsource-parser` 的分法一致。

## 8. 验收

1. 依赖边检查（新增脚本，读 `cargo metadata`，进 CI）：`aimux-provider` 不依赖任何 aimux crate，也不依赖 reqwest、tokio 网络特性；`aimux-provider-utils` 只依赖 `aimux-provider`；workspace 里没有 `aimux-stream`；`aimux-providers` 不依赖 `aimux`、`aimux-devtools`；`aimux` 不依赖 `aimux-providers`、`aimux-devtools`；`aimux-devtools` 不依赖 `aimux-providers`。
2. §4 列出的四个全局量不再存在。`aimux` 里其余的 `static` 可变全局都在允许列表里，每一项注明对应的上游全局（例如照上游做的全局 telemetry 注册：`ai/src/telemetry/telemetry-registry.ts:6`、`ai/src/global.ts:54`）；task-local 的操作设置和 `DiagnosticScope` 不算。
3. workspace 里任何地方都不出现 `aimux_core`（C7 后）。
4. 每个 PR 的测试数量前后相等（`cargo test --workspace -- --list`），生成物重新生成后无 diff。
5. 产物体积不超过 RFC-0043 记录的当前值。

## 9. 开放问题与不采纳

- **`session_id` 的入口。** 本文保留它作为操作设置里的一个字段（绑定已暴露，RFC-0041 §10.2 已落地），`aimux` 只透传。完全照上游可以改成通用的 telemetry metadata，由 devtools 自己读出 session；这会改绑定 API，单独决定。
- **不装 devtools 时要不要 session 归组。** 本文的方案是不要（ops 默认装上）；对齐文档 §3.5 写的是关闭观测也照样归组。
- **重试错误放在哪。** 本文放在 `aimux-provider` 的共享错误里（绑定只认一个联合类型），与对齐文档 §2.1 约束 1 冲突，需要登记或改放 `aimux`。
- **crates.io 上的 `aimux-core`、`aimux-stream`。** 建议删除前各发最后一个版本说明去向（`aimux-core` 改名 `aimux`，`aimux-stream` 并入 `aimux-provider-utils`），不撤回旧版本。
- **操作下载的 UA 和错误（待讨论）。** C4 保持现状（§4.3）。上游下载用 `ai-sdk/<版本>` 加运行时 UA，失败抛带 url 和状态码的 `DownloadError`；照上游改要新增一个错误变体，且和 RFC-0047 “传输错误不带 URL” 冲突，单独决定。
- **不采纳：保留 `aimux-core` 作为 re-export 门面。** 违反对齐文档 §2.1 约束 6，且会让两个路径长期共存。
- **不采纳：一次性大 PR 完成全部拆分。** 改动面覆盖整个 workspace 和两个绑定，无法评审，也无法逐步定位回归。
- 每个错误补 `cause`（错误链）另开小 PR，与本文的搬移互不依赖；若先于 C2 合入，C2 原样搬移。
