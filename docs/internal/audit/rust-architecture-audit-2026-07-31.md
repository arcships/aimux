# aimux Rust 架构严格审核报告

> 审核日期：2026-07-31  
> 最终审查快照：`7951eda9`（`master`，相对 `origin/master` 领先 1 个提交）  
> 审核范围：主 Rust workspace 的 `aimux-core`、`aimux-provider-utils`、`aimux-stream`、`aimux-providers`、`aimux-ffi`（`aimux-tools` 和 `aimux-macros` 已于 2026-07-31 删除，详见末尾注记），以及根 Cargo 配置；对 workspace 外的 Node/Python Rust 绑定仅审查其边界关系，不审核非 Rust 业务代码。  
> 审核维度：架构整洁度、抽象程度、冗余度、边界稳定性、可扩展性、可验证性。  
> 方法：源码走查、crate 依赖分析、重复结构统计、代表性 provider 抽样、编译/格式/lint/全量测试、独立宏编译复现，并由 5 个并行审查视角交叉复核。  
> 说明：审核进行期间仓库被外部流程更新并提交了 `7951eda9`；本报告的最终检查与结论均以该提交为准，未回退或修改该提交。

---

## 1. 执行摘要

### 1.1 总体结论

**架构方向基本正确，但当前实现尚不具备“整洁、低冗余、稳定公共 API”的成熟度。**

项目已经形成正确的宏观依赖方向：核心契约独立于 provider，HTTP/流式基础设施位于共享层，provider 和 FFI 位于外层。`LanguageModel` 主路径、OpenAI-compatible 协议复用、录播测试体系都具备较好的设计基础。

但规模扩张速度已经超过架构治理能力：

1. `Provider` 统一抽象只覆盖语言模型，其他已公开模态必须绕回具体 provider；
2. `OpenAICompatProfile` 有 3 个字段仅声明、未被运行路径消费，形成“看似可配置、实际无效”的抽象；
3. `AbortSignal` 广泛出现在公共 options 中，但 provider 未消费，取消能力没有贯通；
4. provider 转换路径仍会对可达输入 `panic!`，而 release 配置为 `panic = "abort"`，经 FFI 使用时可直接终止宿主进程；
5. SSE/NDJSON 按网络 chunk 执行 `from_utf8_lossy`，会破坏跨 chunk 的合法多字节 UTF-8；
6. 293 个文件符合完整薄封装骨架，保守归一化仍有 248 个文件、16,965 行落入 11 组结构重复；
7. `aimux-providers/src/lib.rs` 的 737 行中有 650 条机械 `pub mod`/`pub use` 注册语句；
8. 当前质量门禁不闭合：格式化、Clippy、全量测试均未通过。

**发布判断：不建议把当前 HEAD 作为稳定 Rust API/C ABI 版本发布。** 可以继续作为快速演进中的内部/预览版本，但应先完成 P0/P1 整改。

### 1.2 评分

> 分数越高越好；“冗余控制”衡量消除有害重复的能力，而不是重复量本身。

| 维度 | 评分 | 严格判断 |
|---|---:|---|
| 架构整洁度 | **5.4 / 10** | crate 职责与依赖方向清楚；provider 单体、FFI 单文件、公共类型分裂和质量门禁失效明显拉低评分。 |
| 抽象程度 | **5.8 / 10** | LLM 主抽象合理，协议引擎复用方向正确；多模态工厂、取消、错误、metadata 和 profile 抽象没有闭环。 |
| 冗余控制 | **3.2 / 10** | 原生协议差异大多属于必要复杂度，但薄封装、根注册、FFI 调度、响应诊断类型存在大量机械重复。 |
| 可扩展性 | **5.0 / 10** | 新增一个 OpenAI-compatible 厂商很容易；修改全局 provider 契约、公共错误或能力模型则代价极高。 |
| 可验证性 | **5.2 / 10** | 测试体量很大且有 cassette；但 9 个测试 target 失败，宏示例被忽略，FFI/macros/tools 缺少有效测试。 |
| **综合** | **4.9 / 10** | **核心方向可保留，当前需要结构治理而非继续横向铺 provider。** |

### 1.3 规模与量化数据

统计口径：仅主 workspace 七个产品 crate 的 `src/**/*.rs` 与 `tests/**/*.rs`，排除 `target/`、`reference/`、绑定构建产物。

| 指标 | 数值 |
|---|---:|
| 产品源码 Rust 文件 | 433 |
| 产品源码行数 | 68,801 |
| 测试 Rust 文件 | 125 |
| 测试行数 | 74,014 |
| 合计 Rust 行数 | 142,815 |
| `aimux-providers` 源码文件 | 388 |
| `aimux-providers` 源码行数 | 62,025 |
| provider 源码占全部产品源码 | **90.2%** |
| provider 测试文件 / 行数 | 114 / 71,268 |
| `aimux-providers/src/lib.rs` | 737 行 |
| 顶层 `pub mod` / `pub use` | 325 / 325 |
| 符合完整薄封装骨架的文件 | 293 个 / 21,469 行 |
| 保守归一化的结构重复 | 248 个文件 / 16,965 行 / 11 组 |
| 超过 1,000 行的 provider 源文件 | 7 个 |

最大的源文件包括：

- `anthropic/convert.rs`：1,532 行；
- `openai/convert.rs`：1,430 行；
- `open_responses.rs`：1,290 行；
- `huggingface/responses.rs`：1,196 行；
- `azure/responses.rs`：1,106 行；
- `openai/responses/convert.rs`：1,088 行；
- `google/convert.rs`：1,083 行。

---

## 2. 当前架构图与依赖评价

根据 Cargo metadata，主 workspace 有 8 个成员（七个产品 crate 加 `scripts/fix_tool`）：

```text
                    ┌──────────────────────┐
                    │     aimux-core       │
                    │ traits / types / API │
                    └──────────▲───────────┘
                               │
              ┌────────────────┼────────────────┐
              │                                 │
┌─────────────┴────────────┐       ┌────────────┴──────────┐
│ aimux-provider-utils     │       │ aimux-tools           │
│ HTTP/retry/error/multipart│      │ ToolSet/Executor      │
└─────────────▲────────────┘       └───────────────────────┘
              │
      ┌───────┴────────┐
      │ aimux-providers│──────────────► aimux-stream
      │ protocols/vendors             SSE/NDJSON/tracker
      └───────▲────────┘
              │
      ┌───────┴────────┐
      │   aimux-ffi    │
      │ C ABI adapter  │
      └────────────────┘
```

### 正确之处

- `aimux-core` 没有反向依赖 transport 或 provider，符合依赖倒置；
- provider 不直接构造 `reqwest::Client`，共享 HTTP 边界以纯数据 `HttpRequest`/`HttpResponse` 隔离传输实现，见 [`aimux-provider-utils/src/http.rs#L124-L180`](../aimux-provider-utils/src/http.rs#L124-L180)；
- 非流式和流式 client 分离，流式不设全程固定超时，且只在建连阶段 retry，避免中途重试产生重复 token，见 [`aimux-provider-utils/src/http.rs#L90-L107`](../aimux-provider-utils/src/http.rs#L90-L107)、[`aimux-provider-utils/src/http.rs#L214-L238`](../aimux-provider-utils/src/http.rs#L214-L238)；
- LLM 用户入口与 provider-facing trait 分层明确，见 [`aimux-core/src/generate.rs#L159-L258`](../aimux-core/src/generate.rs#L159-L258)、[`aimux-core/src/language_model.rs#L24-L41`](../aimux-core/src/language_model.rs#L24-L41)；
- 用户消息到标准化 provider prompt 的转换集中在单点，见 [`aimux-core/src/language_model_message.rs#L32-L63`](../aimux-core/src/language_model_message.rs#L32-L63)；
- `OpenAICompatProfile` 以数据描述协议差异的方向是正确的，见 [`aimux-providers/src/openai/mod.rs#L30-L91`](../aimux-providers/src/openai/mod.rs#L30-L91)。

### 核心结构问题

依赖图没有环，不代表边界已经完整。当前主要问题是：**core 的契约面扩展到了多模态，但 provider 工厂、用户 façade、取消、错误和诊断结构仍停留在 LLM 第一阶段；providers 与 FFI 则通过复制和具体类型绕过缺失抽象。**

---

## 3. 发现汇总

| ID | 严重度 | 维度 | 发现 |
|---|---|---|---|
| P0-01 | 阻断 | 抽象/正确性 | `#[tool]` 宏与文档承诺不一致，文档示例无法编译，Schema 自动生成未实现。 |
| P0-02 | 阻断 | 正确性/边界 | provider 转换对可达输入执行 `panic!`；release `panic=abort` 会终止 FFI 宿主。 |
| P0-03 | 阻断 | 正确性/流式 | SSE/NDJSON 按 chunk lossy 解码，合法多字节 UTF-8 跨 chunk 时被静默破坏。 |
| P0-04 | 阻断 | 可验证性 | 当前全量测试 9 个 target、12 个测试失败；格式与 Clippy 门禁也失败。 |
| P1-01 | 高 | 抽象 | `Provider` 仅生产 `LanguageModel`，与 core 已公开的多模态 trait 不匹配。 |
| P1-02 | 高 | 抽象 | 非 LLM trait 被标为 provider-facing，却缺少用户 façade，只能直接调用 `do_*`。 |
| P1-03 | 高 | 抽象/正确性 | `AbortSignal` 是公共 API 外观，但没有贯通到 provider/HTTP。 |
| P1-04 | 高 | 抽象 | `OpenAICompatProfile` 三个字段未被运行路径消费，能力描述不可信。 |
| P1-05 | 高 | 冗余/规模 | 单一 provider crate 默认编译并公开 325 个模块，无 feature/bundle 边界。 |
| P1-06 | 高 | 冗余 | 293 个薄封装骨架和 650 条根注册语句构成主要有害重复。 |
| P1-07 | 高 | 错误模型 | HTTP 层过早决定 provider 语义，且调用步骤未补充上下文，导致契约回归和测试失败。 |
| P1-08 | 高 | 边界/安全 | HTTP header 非法时静默丢弃；multipart header 参数未验证或转义。 |
| P1-09 | 高 | FFI | FFI 893 行单文件同步 `block_on`，回调重入可死锁，panic/注册表边界缺少护栏。 |
| P1-10 | 高 | 流式/资源 | SSE、NDJSON、tool-call tracker 的远端驱动缓冲/索引无上限。 |
| P2-01 | 中 | 类型整洁度 | provider metadata、request/response info、provider options 有多套不一致形状。 |
| P2-02 | 中 | 错误模型 | `AiMuxError` 主要只保留 String，错误源、status、provider code 和上下文丢失。 |
| P2-03 | 中 | 模块职责 | 多个 1,000+ 行 convert/responses 文件职责过宽；Anthropic 与 AWS stream loop 重复。 |
| P2-04 | 中 | 工具层 | `ToolSet` 顺序不稳定且重复名静默覆盖；`execute_all` 无并发上限。 |
| P2-05 | 中 | API 可信度 | `validate_base_url` 实际只检查非空；`StreamingToolCallTracker::with_generate_id` 是无效 API。 |
| P3-01 | 低 | 工程治理 | repository 仍为占位 URL、toolchain 浮动 stable、Cargo.lock 被忽略、绑定不在主 workspace 门禁内。 |

---

## 4. 阻断级发现（P0）

### P0-01：`#[tool]` 宏的公开契约不可用

**证据**

- 文档示例声明普通具名参数并返回 `String`，见 [`aimux-macros/src/lib.rs#L11-L25`](../aimux-macros/src/lib.rs#L11-L25)；
- 展开代码固定调用 `fn_name(args.clone()).await`，其中 `args` 是 `serde_json::Value`，并假定返回值是 `Result`，见 [`aimux-macros/src/lib.rs#L39-L57`](../aimux-macros/src/lib.rs#L39-L57)；
- attribute 的 description 被解析后丢弃，见 [`aimux-macros/src/lib.rs#L33-L35`](../aimux-macros/src/lib.rs#L33-L35)；
- 代码没有解析函数参数并生成 JSON Schema，与文档第 25 行直接矛盾；
- 唯一 doctest 被标为 `ignore`，因此 `cargo test -p aimux-macros --doc` 显示 0 个实际执行测试；
- 将文档示例放入独立临时 crate 后，`cargo check` 稳定复现 `E0061`（函数实参数量错误）和 `E0308`（把 `String` 当作 `Result`）。

**影响**

这是公共 API 功能断裂，不是单纯代码风格问题。`aimux-core`、`aimux-tools`、`aimux-macros` 三套工具概念没有形成“定义 → Schema → 注册 → 执行”的闭环。

**整改**

二选一，不应继续保留当前模糊状态：

1. **最小方案**：正式限定为 `async fn(Value) -> Result<Value, E>`，宏在编译期拒绝其他签名，并修正文档；
2. **完整方案**：解析 `FnArg`，为具名参数生成反序列化、调用实参、结果序列化和 JSON Schema。

必须增加 `trybuild` 编译通过/失败测试，并取消忽略文档示例。

---

### P0-02：provider 转换中的 panic 会在 release/FFI 中升级为进程终止

**证据**

- OpenAI content part 转换将可失败函数用 `unwrap_or_else(panic!)` 解包，见 [`aimux-providers/src/openai/convert.rs#L784-L878`](../aimux-providers/src/openai/convert.rs#L784-L878)；
- Anthropic 对不支持的媒体类型、不可探测 subtype、缺失 provider reference 直接 panic，见 [`aimux-providers/src/anthropic/convert.rs#L530-L622`](../aimux-providers/src/anthropic/convert.rs#L530-L622)、[`aimux-providers/src/anthropic/convert.rs#L676-L690`](../aimux-providers/src/anthropic/convert.rs#L676-L690)、[`aimux-providers/src/anthropic/convert.rs#L747-L761`](../aimux-providers/src/anthropic/convert.rs#L747-L761)；
- 根 release profile 明确设置 `panic = "abort"`，见 [`Cargo.toml#L21-L31`](../Cargo.toml#L21-L31)；
- provider 开发规范要求未知响应不得 panic，见 [`rfc/0006-provider-development.md#L85-L93`](../rfc/0006-provider-development.md#L85-L93)。

**影响**

这些输入来自用户消息的 media type、文件引用或 provider 数据，属于正常错误边界。debug 下是 panic，release 下是 abort；经 Swift/Kotlin/Flutter/C 等 FFI 宿主调用时，错误无法转换为 `AiMuxError`，而是直接终止整个进程。

**整改**

- 将所有 prompt/request conversion 设计为 `Result<Converted, AiMuxError>`；
- 用 `InvalidArgument`/`Unsupported` 返回不支持的媒体类型和缺失 reference；
- 在 FFI 层增加 panic containment 只能作为最后防线，不能替代 provider 层移除 panic；当 release 使用 `panic=abort` 时，`catch_unwind` 本身也无法兜底，应重新评估是否对 FFI 产物全局启用 abort。

---

### P0-03：SSE/NDJSON 在合法 UTF-8 跨 chunk 时会静默改写文本

**证据**

- SSE 对每个网络 chunk 单独调用 `String::from_utf8_lossy`，见 [`aimux-stream/src/sse.rs#L89-L99`](../aimux-stream/src/sse.rs#L89-L99)；
- NDJSON 使用相同模式，见 [`aimux-stream/src/ndjson.rs#L72-L80`](../aimux-stream/src/ndjson.rs#L72-L80)；
- 两个错误 enum 都保留了 `Utf8` 变体，但该路径实际不可达，见 [`aimux-stream/src/sse.rs#L10-L16`](../aimux-stream/src/sse.rs#L10-L16)、[`aimux-stream/src/ndjson.rs#L11-L19`](../aimux-stream/src/ndjson.rs#L11-L19)。

**影响**

TCP/HTTP chunk 边界与 UTF-8 字符边界无关。一个合法的中文、emoji 或其他多字节字符可能被拆到两个 chunk；分别 lossy 解码会在两侧产生 U+FFFD，永久破坏文本、JSON 字符串或工具参数。这对中文项目尤其不能接受。

**整改**

- 内部缓冲改为 `Vec<u8>`/`BytesMut`；
- 先按字节寻找完整帧/行分隔符，再对完整帧执行严格 UTF-8 解码；
- 无效 UTF-8 返回现有 `Utf8` 错误，不得 silent replacement；
- 增加“在每一个字节位置拆分中文/emoji”的参数化测试，以及非法 UTF-8 测试。

---

### P0-04：当前质量门禁不闭合

最终快照 `7951eda9` 的检查结果：

| 命令 | 结果 |
|---|---|
| `cargo check --workspace --all-targets` | **通过**，但存在 warning。 |
| `cargo fmt --all -- --check` | **失败**：63 个文件、181 个 diff hunk。 |
| `cargo clippy --workspace --lib -- -D warnings` | **失败**：FFI 2 个未使用参数与 3 个 `collapsible_if`。 |
| `cargo clippy --workspace --all-targets -- -D warnings` | **失败**：另有 core contract test 与 retry test lint。 |
| `cargo test --workspace --no-fail-fast` | **失败**：9 个 test target、12 个测试失败。 |
| `cargo test -p aimux-macros --doc` | 命令成功，但唯一示例被忽略；0 个宏文档测试实际执行。 |
| 独立编译宏文档示例 | **失败**：`E0061`、`E0308`。 |

失败 target：

- `google_files_test`：2；
- `google_model_test`：3；
- `google_pse_test`：1；
- `linkup_test`：1；
- `parallel_ai_test`：1；
- `recraft_test`：1；
- `searxng_test`：1；
- `vertex_anthropic_test`：1；
- `xai_test`：1。

主要回归集中在共享 HTTP 错误映射：403 被统一映射为 `Auth`，5xx 被映射为 `ApiCall`，而 provider 契约测试仍期待 `Provider`；Google file 的多步骤调用还丢失了“初始化失败/上传失败”的操作上下文，见 [`aimux-providers/tests/google_files_test.rs#L389-L465`](../aimux-providers/tests/google_files_test.rs#L389-L465)。

**判断**

测试数量大是优点，但“测试存在”不等于“架构受保护”。当前共享层改动没有完成跨 provider 契约迁移，CI 门禁也没有阻止失败状态进入 HEAD。

---

## 5. 高优先级发现（P1）

### P1-01：`Provider` 名义上是统一工厂，实际上只统一 LLM

`Provider` 只有 `language_model`，见 [`aimux-core/src/provider.rs#L6-L15`](../aimux-core/src/provider.rs#L6-L15)。但 core 已公开 embedding、image、speech、transcription、reranking、search、video、files 等能力，见 [`aimux-core/src/lib.rs#L44-L76`](../aimux-core/src/lib.rs#L44-L76)。具体 provider 只能通过固有方法提供这些模型，例如 OpenAI 的多模态工厂位于 [`aimux-providers/src/openai/mod.rs#L192-L233`](../aimux-providers/src/openai/mod.rs#L192-L233)。

**后果**：持有 `dyn Provider` 的上层只能动态选择语言模型；FFI 和绑定必须依赖具体 provider 类型，造成能力发现、fallback、registry 与配置化选择旁路化。

**建议**：将现有 trait 语义明确为 `LanguageModelProvider`，并引入按能力实现的工厂 trait，如 `EmbeddingProvider`、`ImageProvider`。不要把所有方法塞入单个胖 trait，也不要使用 `Any` 或 JSON 万能模型。

---

### P1-02：用户 façade 只覆盖文本，其余 trait 的“provider-facing”声明不成立

文本有 `generate_text`/`stream_text` façade，负责 prompt 标准化和 option 转换，见 [`aimux-core/src/generate.rs#L141-L258`](../aimux-core/src/generate.rs#L141-L258)。但 embedding/image/speech 等 trait 一方面宣称用户不直接调用 `do_*`，另一方面 core 没有提供替代入口，例如 [`aimux-core/src/embedding_model.rs#L98-L137`](../aimux-core/src/embedding_model.rs#L98-L137)、[`aimux-core/src/image_model.rs#L170-L196`](../aimux-core/src/image_model.rs#L170-L196)。

**建议**：为每种模态增加轻量 façade，统一默认值、输入验证、取消、telemetry/request/response 诊断。若项目明确只做 provider spec 层，则删除“不应直接调用”的表述并重命名方法，不能维持双重语义。

---

### P1-03：`AbortSignal` 没有贯通，是“幻影能力”

`AbortSignal` 文档要求 provider 轮询或接入 `tokio::select!`，见 [`aimux-core/src/shared.rs#L84-L113`](../aimux-core/src/shared.rs#L84-L113)。多个 call options 暴露该字段，例如 [`aimux-core/src/image_model.rs#L106-L115`](../aimux-core/src/image_model.rs#L106-L115)。但对 `aimux-providers/src/**/*.rs` 的定向检索没有任何 `abort_signal` 消费点，`HttpRequest` 也不承载取消信息。

**影响**：用户调用 `abort()` 后，请求、轮询和流仍会继续；视频、文件上传、流式转写等长操作尤其危险。

**建议**：在 transport 层定义统一取消语义，覆盖发起前、等待响应、读取 body、轮询与流消费；完成前将字段标记为实验性或明确“不生效”，不能作为正式能力宣传。

---

### P1-04：`OpenAICompatProfile` 有三个无效字段

`supports_tools`、`supports_response_format`、`stream_usage_key` 定义于 [`aimux-providers/src/openai/mod.rs#L35-L48`](../aimux-providers/src/openai/mod.rs#L35-L48)，但定向检索表明它们只出现在定义和构造函数中，未被 request/stream 执行路径读取。仓库 RFC 也明确记录尚未接入，见 [`rfc/0002-provider-improvements.md#L42-L46`](../rfc/0002-provider-improvements.md#L42-L46)。

**影响**：profile 不是可信的 capability model。某 provider 即使标记不支持 tools/response format，调用仍可能被发送或静默误处理；`stream_usage_key` 不能真正驱动解析。

**建议**：每个公开 profile 字段必须满足“运行路径消费 + 默认行为回归测试 + 差异行为测试”。暂时无法实现的字段应移除或显式标为未实现，不能保留装饰性抽象。

---

### P1-05/P1-06：provider 单体与薄封装重复已经失控

**证据**

- `aimux-providers/src/lib.rs` 显式声明并重导出 325 个模块，见 [`aimux-providers/src/lib.rs#L7-L194`](../aimux-providers/src/lib.rs#L7-L194) 及后续注册区；
- `aimux-providers/Cargo.toml` 没有 provider features，所有模块默认进入同一 crate，见 [`aimux-providers/Cargo.toml#L1-L38`](../aimux-providers/Cargo.toml#L1-L38)；
- 293 个文件符合完整薄封装骨架，共 21,469 行；
- 保守归一化（移除注释并归一化类型名/字符串）后，仍有 248 个文件、16,965 行落入 11 组同构结构；
- profile 构造调用中 `full()` 284 次、`deepseek()` 2 次、`groq()` 2 次，说明大多数 wrapper 仅承载静态元数据；
- 典型 wrapper 的真正差异只有 URL、环境变量、provider 名称和 profile，见 [`aimux-providers/src/ai21.rs#L13-L62`](../aimux-providers/src/ai21.rs#L13-L62)。

**影响**

- 新增厂商容易，但公共构造、鉴权、header、profile 或 Provider trait 一旦变化，就要批量修改数百文件；
- 所有用户即使只用一个厂商，也要编译/分析巨大的公共 crate；LTO 只能缩小最终二进制，不能降低开发构建、IDE、lint、API 审查成本；
- 手工根注册有双点维护和导出漂移风险。

**建议架构**

1. 用一个受 schema 校验的 manifest（Rust const table/TOML/YAML）描述 `id/display/base_url/env/profile/extra_headers`；
2. 通过受测 declarative macro 或生成脚本生成 wrapper 与根导出；生成产物和手写实现物理分区；
3. 增加 Cargo feature 或拆 crate：协议引擎、生成 wrapper、复杂原生 provider、curated bundle 分开；
4. 不要把复杂原生 provider 强行数据化，代码生成只适用于同构 wrapper。

---

### P1-07：共享 HTTP 层越过了 transport 边界，过早决定 provider 错误语义

HTTP 层在 429/5xx/其他状态上直接创建 `RateLimited`、`ApiCall` 或 provider error，见 [`aimux-provider-utils/src/http.rs#L240-L308`](../aimux-provider-utils/src/http.rs#L240-L308)；通用 parser 又把 401/403 统一映射为 `Auth`，见 [`aimux-provider-utils/src/response.rs#L19-L68`](../aimux-provider-utils/src/response.rs#L19-L68)。

当前 12 个失败测试多数正是这次统一分类与 provider 既有契约不一致。Google Files 的两阶段请求使用 `?` 直接传播共享错误，未附加“init/upload”上下文，见 [`aimux-providers/src/google/files.rs#L186-L225`](../aimux-providers/src/google/files.rs#L186-L225)。

**根因**：transport 同时承担网络重试、HTTP 状态分类、provider 语义映射和错误文本组织，职责过多。

**建议**

- transport 返回结构化 `HttpFailure { status, headers, body, source, retry_hint }`；
- retry 依据结构字段决定，不依据字符串化的高层 `AiMuxError`；
- provider adapter 再把失败映射为 `Auth`、`ModelNotFound`、provider-specific code，并在多步骤操作中补充阶段上下文；
- 迁移应先统一契约测试，再改共享层，不能让 9 个 target 长期处于失败状态。

---

### P1-08：header 与 multipart 边界违反 fail-fast

- HTTP header 的 name/value 转换失败时被静默跳过，见 [`aimux-provider-utils/src/http.rs#L311-L336`](../aimux-provider-utils/src/http.rs#L311-L336)；鉴权或签名 header 失效时，调用者只会看到远端错误；
- multipart 的 field name、filename、media type 原样插入 MIME header，见 [`aimux-provider-utils/src/multipart.rs#L29-L53`](../aimux-provider-utils/src/multipart.rs#L29-L53)。包含引号、CR/LF 或 NUL 时可破坏报文结构。

**建议**：`HttpRequest` 构造期校验 header 并返回 `InvalidArgument`；multipart 拒绝控制字符并正确转义 quoted-string，优先采用成熟 multipart builder。

---

### P1-09：FFI 是同步、集中、缺少故障隔离的全模态总线

- `aimux-ffi/src/lib.rs` 已达 893 行；registry、runtime、字符串所有权、provider 工厂、全部模态执行都在一个文件；
- 文档明确承认同步 `block_on` 和回调重入会死锁，见 [`aimux-ffi/src/lib.rs#L17-L23`](../aimux-ffi/src/lib.rs#L17-L23)；
- 流回调在 `runtime().block_on` 内直接执行用户代码，见 [`aimux-ffi/src/lib.rs#L398-L424`](../aimux-ffi/src/lib.rs#L398-L424)；
- registry mutex poison 使用 `expect`，见 [`aimux-ffi/src/lib.rs#L75-L115`](../aimux-ffi/src/lib.rs#L75-L115)；
- 流 part 序列化失败降级成 `{}`，见 [`aimux-ffi/src/lib.rs#L403-L412`](../aimux-ffi/src/lib.rs#L403-L412)；
- crate 当前没有 Rust 单元/集成测试。

**建议**

- ABI 保持稳定，但内部拆为 `registry`、`runtime`、`wire`、`ffi/{language,embedding,...}`；
- 抽取一致的 JSON decode/run/encode/error helper；
- 长操作改为 `start -> request_id`、事件回调/队列、`cancel` 的异步任务模型；
- 禁止在 runtime 的 `block_on` 调用栈中执行任意用户回调；
- 增加空指针、错 handle、double drop、并发 get/drop、回调重入、panic containment 和 ABI symbol snapshot 测试。

---

### P1-10：远端可驱动无界内存增长

- SSE 在没有事件分隔符时无限扩展 `buffer`，见 [`aimux-stream/src/sse.rs#L63-L102`](../aimux-stream/src/sse.rs#L63-L102)；
- NDJSON 在没有换行时无限扩展 `buffer`，见 [`aimux-stream/src/ndjson.rs#L53-L93`](../aimux-stream/src/ndjson.rs#L53-L93)；
- tool-call tracker 直接按远端 `index + 1` 扩容 `Vec`，见 [`aimux-stream/src/streaming_tool_call_tracker.rs#L222-L235`](../aimux-stream/src/streaming_tool_call_tracker.rs#L222-L235)、[`aimux-stream/src/streaming_tool_call_tracker.rs#L308-L317`](../aimux-stream/src/streaming_tool_call_tracker.rs#L308-L317)。

**建议**：为 SSE event、NDJSON line、累计 stream bytes、tool call 数量与 index 设置可配置硬上限，超限返回结构化错误；不要让远端数字直接决定本地稠密分配。

---

## 6. 中低优先级发现

### P2-01：共享数据结构没有真正共享

1. LLM 使用自由 `serde_json::Value` 形式的 `ProviderMetadata`，非 LLM 使用 `HashMap<String, Value>` 的 `SharedProviderMetadata`，见 [`aimux-core/src/shared.rs#L28-L42`](../aimux-core/src/shared.rs#L28-L42)；
2. `shared.rs` 已定义 `RequestInfo`/`ResponseInfo`，见 [`aimux-core/src/shared.rs#L237-L265`](../aimux-core/src/shared.rs#L237-L265)，但各模态仍重复定义不同 response/request 类型；
3. `provider_options` 在部分模态是 `Option<Map>`，在 image/video 是必填空 map；LLM 又直接写 `Option<HashMap<...>>`，见 [`aimux-core/src/options.rs#L73-L81`](../aimux-core/src/options.rs#L73-L81)；
4. `ModelId` 被定义并导出，但主项目没有真实使用点，见 [`aimux-core/src/model_id.rs#L7-L45`](../aimux-core/src/model_id.rs#L7-L45)。

**建议**：统一 provider-scoped metadata 新类型与 request/response envelope；明确集合字段使用空集合还是 `Option` 的全仓规则。不要为了去重合并各模态执行方法，只抽取横切数据。

### P2-02：错误类型过度字符串化

`AiMuxError` 大多数变体只有 `String`，没有 status、provider、model、provider code、response body 或 `source`，见 [`aimux-core/src/error.rs#L7-L52`](../aimux-core/src/error.rs#L7-L52)。`serde_json::Error` 也立即降为字符串，见 [`aimux-core/src/error.rs#L54-L58`](../aimux-core/src/error.rs#L54-L58)。`ModelNotFound` 与 `NoSuchModel` 并存，语义边界不清晰。

建议引入结构化 transport/provider 错误并保留错误链；`is_retryable` 应依据 status/code/阶段，而不是粗粒度变体。

### P2-03：复杂文件职责过宽，存在可验证的局部重复

`openai/convert.rs` 与 `anthropic/convert.rs` 均超过 1,400 行，混合消息、工具、文件、request、response、stream 与 provider 特例。Anthropic 标准与 Anthropic AWS 的 streaming loop 也高度重复，主要差异只是鉴权/签名和 body 发送方式，见 [`aimux-providers/src/anthropic/model.rs#L204-L506`](../aimux-providers/src/anthropic/model.rs#L204-L506)、[`aimux-providers/src/anthropic_aws/model.rs#L222-L455`](../aimux-providers/src/anthropic_aws/model.rs#L222-L455)。

建议按 `request/messages/files/tools/response/stream` 拆内部模块；Anthropic 共用协议 event reducer，transport/auth 保持可注入。不要强行跨 Anthropic/Cohere/Mistral 抽象一个万能状态机，因为协议事件语义并不相同。

### P2-04：工具集合和执行器缺少确定性与资源上限

- `ToolSet` 使用 `HashMap`，`to_vec()` 顺序不稳定，重复注册静默覆盖，见 [`aimux-tools/src/tool_set.rs#L7-L35`](../aimux-tools/src/tool_set.rs#L7-L35)；
- `execute_all` 直接 `join_all`，没有并发上限，见 [`aimux-tools/src/tool_executor.rs#L49-L52`](../aimux-tools/src/tool_executor.rs#L49-L52)；
- `JsonSchemaBuilder` 是公共类型，但 crate root 没有重导出，工具层 API 边界不完整，见 [`aimux-tools/src/lib.rs#L5-L10`](../aimux-tools/src/lib.rs#L5-L10)。

建议使用 insertion-ordered map；`register` 返回替换结果或拒绝重名；增加有默认上限的批量执行 API。

### P2-05：若干 API 名称与真实行为不符

- `validate_base_url` 只拒绝空字符串并移除末尾 `/`，不解析 scheme/host，见 [`aimux-provider-utils/src/url.rs#L19-L27`](../aimux-provider-utils/src/url.rs#L19-L27)；
- `with_generate_id` 可配置但字段被标记为 dead code，实际缺失 ID 直接返回 `MissingId`，见 [`aimux-stream/src/streaming_tool_call_tracker.rs#L156-L192`](../aimux-stream/src/streaming_tool_call_tracker.rs#L156-L192)、[`aimux-stream/src/streaming_tool_call_tracker.rs#L284-L289`](../aimux-stream/src/streaming_tool_call_tracker.rs#L284-L289)。

这类“声明了但不生效”的 API 会系统性降低调用者信任。应实现或删除，不能只靠注释解释。

### P3-01：workspace/release 治理不完整

- 根 `repository` 仍是占位地址，见 [`Cargo.toml#L13-L19`](../Cargo.toml#L13-L19)；
- `rust-version = "1.85"`，但 toolchain 使用浮动 `stable`，见 [`rust-toolchain.toml#L1-L3`](../rust-toolchain.toml#L1-L3)；本次实际检查使用 Rust/Cargo 1.97.1，不能证明 MSRV 1.85；
- `Cargo.lock` 被忽略，见 [`.gitignore#L1-L3`](../.gitignore#L1-L3)，CI 解析图不可复现；
- Node/Python Rust bindings 通过各自 `[workspace]` 排除在主 workspace 外，因此根 `cargo test --workspace` 不代表全部 Rust 交付物；
- 空的 `scripts/fix_tool` 被列为正式 workspace member，根 members 列表格式也不整洁，见 [`Cargo.toml#L1-L10`](../Cargo.toml#L1-L10)。

---

## 7. 哪些不是问题

严格审核不等于一律要求抽象或去重。以下设计应保留：

1. **每种模态拥有独立 trait**：Language/Image/Speech/Search 的输入输出天然不同，不应合并为 `dyn Model<Value, Value>`；
2. **OpenAI-compatible 共享引擎**：这是正确的协议级复用，问题在 wrapper 声明方式和无效 profile 字段，不在共享本身；
3. **复杂原生 provider 保留独立转换**：Google、Bedrock、Anthropic 等协议差异是真实复杂度，不应为了 DRY 强行塞进 OpenAI 模型；
4. **HTTP client 全局共享**：连接池与 TLS session 复用是正确方向，不应退回每个 provider `Client::new()`；
5. **FFI opaque handle 模型**：`u64 -> Arc<dyn Trait>` 适合稳定 C ABI；问题是注册表生命周期、同步执行、panic 和单文件集中；
6. **大量测试代码本身不是冗余**：provider 测试行数超过源码是可接受的，真正问题是契约测试未统一、失败未被门禁拦住，以及同构 fixture 是否可数据驱动；
7. **少量显式 wrapper 类型可有 DX 价值**：用户友好的 `GroqProvider` 等名称可以保留，但实现应由 manifest/macro 生成，而不是复制数百份源码。

---

## 8. 推荐目标架构

```text
aimux-core
├── model traits（各模态独立）
├── capability factory traits
├── shared error / metadata / request / response envelope
└── user façades（generate/embed/image/speech/...）

transport
├── typed request/response/failure
├── retry policy
├── cancellation
├── frame/size limits
└── reqwest backend

protocol-engines
├── openai-compatible
├── anthropic
├── google
├── bedrock
└── ...

provider-catalog
├── validated manifest
├── generated thin wrappers
└── curated feature bundles

native-providers
└── 只存放真实协议差异和模态专用实现

ffi/bindings
├── async task + cancel
├── registry/runtime/wire 分层
└── 各模态薄 ABI façade
```

核心原则：

- **按协议复用，不按厂商名称复制；**
- **按能力组合，不建立胖 Provider；**
- **transport 只描述 HTTP 事实，provider 决定业务错误语义；**
- **所有公开配置必须被执行路径消费；**
- **远端输入永远不能触发 panic 或无界分配；**
- **生成代码必须有单一声明源、确定性输出和 CI 校验。**

---

## 9. 分阶段整改计划

### 阶段 0：恢复可信基线（立即，P0）

1. 修复 9 个失败 test target，先明确共享错误分类的正式契约；
2. 运行并通过 `cargo fmt --all -- --check`；
3. 运行并通过 `cargo clippy --workspace --all-targets -- -D warnings`；
4. 修复或临时下线 `#[tool]`，让文档示例成为实际编译测试；
5. 将 provider 输入转换中的 panic 全部改为 `Result`；
6. 修复 SSE/NDJSON 的字节缓冲与 UTF-8 边界。

**验收**：四项质量命令全部绿色；宏示例不再 ignore；合法 UTF-8 任意切 chunk 均保持原文；可达用户输入不导致 panic。

### 阶段 1：收紧公共语义（1–3 周，P1）

1. `Provider` 更名/拆为 capability factory traits；
2. 为非 LLM 模态补齐用户 façade；
3. 贯通 cancellation；
4. 让 profile 字段全部生效，或删除无效字段；
5. transport 使用结构化失败，provider 补充语义和阶段上下文；
6. 修复 header/multipart/URL 校验；
7. 给 stream frame、tool call 数量和 index 加上限。

### 阶段 2：控制规模与冗余（3–6 周，P1/P2）

1. 建立 provider manifest 和受测生成流程；
2. 将同构 wrapper 与手写复杂 provider 分区；
3. 引入 feature/bundle 或拆 crate，允许按协议/厂商编译；
4. 拆分 1,000+ 行 convert/responses 文件；
5. 提取 Anthropic/AWS 共享 event reducer；
6. FFI 内部按 registry/runtime/wire/modality 拆分并抽取通用执行模板。

### 阶段 3：统一类型与持续治理（6–10 周，P2/P3）

1. 统一 provider metadata、request/response info、provider options 空值策略；
2. 结构化 `AiMuxError` 并保留 source；
3. bindings 纳入明确 CI 矩阵；
4. CI 同时测试 MSRV 与 stable，修正 repository 和 lockfile 策略；
5. 增加依赖/许可/漏洞检查，以及架构规则检查（core 禁止依赖 transport/provider、provider 禁止直接使用 reqwest）。

---

## 10. 最终判断

aimux 不是“架构失败”，而是一个**成功扩张后尚未完成第二阶段治理**的项目：

- 第一阶段已经证明统一协议接入和多 provider 覆盖可行；
- 第二阶段必须把能力模型、错误边界、取消、生成式 wrapper、FFI 和质量门禁收敛起来；
- 在此之前继续增加 provider，只会进一步放大 325 模块单体、机械注册、无效 profile 和契约迁移成本。

**最优先的工作不是再接入更多厂商，而是恢复绿色基线、消除 panic/流式损坏、修复工具宏，并把薄封装迁移到声明式 provider catalog。**

---

## 附录：后续删除

2026-07-31：基于本报告的发现（P0-01 `#[tool]` 宏不可用、P2-04 ToolSet/Executor 问题），确认 `aimux-tools` 和 `aimux-macros` 两个 crate 在项目内零消费、零引用，且与"不做 agent loop"的项目定位矛盾。已将两个 crate 及其相关文档引用全部删除。
