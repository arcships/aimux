# RFC-0001：多语言绑定（Multi-language Bindings）

> **状态**：v0.5.1（全部阶段落地；Flutter 修正为 dart:ffi C ABI 路径）
> **作者**：—
> **日期**：2026-07-26
> **关联**：[aimux-core](../aimux-core)、[aimux-providers](../aimux-providers)

---

## 1. 背景与动机

`aimux` 当前是纯 Rust 工作区，对标 Vercel AI SDK（TypeScript 原生）。目标是将核心能力以库的形式提供给 **Node、Swift、Kotlin、Flutter、Python** 等多语言生态，避免在各语言重复实现 provider 适配与流解析。

### 1.1 复用价值在哪里

跨语言重写成本最高、最易错的部分是 **provider 适配逻辑**：

- OpenAI / Anthropic / Bedrock 等的 HTTP 请求构造
- SSE / NDJSON 流式解码（见 [aimux-stream](../aimux-stream/src/lib.rs)）
- 响应到 `StreamPart` 的转换（见 [stream_part.rs](../aimux-core/src/stream_part.rs)）
- 速率限制、重试、退避（见 [aimux-provider-utils/src/retry.rs](../aimux-provider-utils/src/retry.rs)）

这部分量大且稳定，是 Rust 核心真正的复用资产。用户面 API（`generate_text` / `stream_text`）和 provider 面 trait（`do_generate` / `do_stream`）已在 [generate.rs](../aimux-core/src/generate.rs) 中解耦——这是 FFI 化的基础。

### 1.2 战略前提（必须先回答）

> ⚠️ 本项目对标 Vercel AI SDK，而后者本身就是 TypeScript 原生。

- **Node 绑定是旗舰战场**——本项目存在的首要目标就是战胜 Vercel AI SDK。TS 生态虽有官方实现，但 aimux 的卖点不是"另一个 TS SDK"，而是**统一 Rust 核心跨语言复用 + 性能 + 172 provider 覆盖**。Node 作为第一个绑定，直接在 AISDK 主场证明 Rust 核心的竞争力，是整个多语言战略的支点。
- **Python** AI/ML 开发者基数最大，虽有 LangChain/LlamaIndex，但缺"轻量、统一 provider 接口、不绑架架构"的 SDK，Rust 核心可补位——第二优先级。
- **Swift / Kotlin / Flutter** 生态没有官方统一 AI SDK，移动端对二进制体积、性能、离线敏感，Rust 核心有竞争力——第三梯队，价值区但非首要战场。

**建议**：动机以 Node（旗舰，对标 AISDK）+ Python（AI/ML 母语）为主，移动端（Swift/Kotlin/Flutter）作为价值延伸。

---

## 2. 现状分析：当前架构对多语言有多友好

逐个核对核心类型后，结论分两类。

### 2.1 ✅ 天然跨语言友好（数据类型，可 serde 序列化）

- `GenerateContent` / `GenerateResult` / `StreamPart` / `Usage` / `FinishReason` / `Warning`（见 [result.rs](../aimux-core/src/result.rs)、[stream_part.rs](../aimux-core/src/stream_part.rs)、[types.rs](../aimux-core/src/types.rs)）
- `CallOptions` / `GenerateTextOptions` / `ModelMessage` / `ContentPart`（见 [options.rs](../aimux-core/src/options.rs)、[message.rs](../aimux-core/src/message.rs)）
- `FunctionTool` / `ToolCall` / `ToolResult`（见 [tool.rs](../aimux-core/src/tool.rs)）
- `AiMuxError`（enum，可序列化，见 [error.rs](../aimux-core/src/error.rs)）

### 2.2 ⛔ 跨 FFI 边界的拦路虎（Rust 特有抽象）

| 位置 | 问题 | 为什么难 |
|------|------|---------|
| [result.rs:56](../aimux-core/src/result.rs#L56) `Pin<Box<dyn Stream<Item=Result<StreamPart,AiMuxError>>+Send>>` | Stream trait object | **头号难题**。Rust 的 `Stream` 无法直接传给 JS/Swift/Kotlin，各语言异步模型完全不同 |
| [language_model.rs:25](../aimux-core/src/language_model.rs#L25) `LanguageModel` trait + `Box<dyn LanguageModel>` | trait object 动态分发 | FFI 不能传 `&dyn`，需用 opaque handle + 注册表 |
| [generate.rs:154](../aimux-core/src/generate.rs#L154) `impl Into<ModelPrompt>` | 泛型入参 | FFI 只能传具体类型 |
| [aimux-macros](../aimux-macros/src/lib.rs) `#[tool]` proc-macro | Rust 编译期宏 | 无法跨语言，工具定义需改成数据描述 |

> **注**：providers 硬绑 `reqwest` + `tokio` 原列于此表（"移动端会与原生 HTTP 栈重复，增大二进制"）。v0.2 已删除该判断——reqwest+rustls 跨平台够用，不构成 FFI 障碍，详见 §4.5。

---

## 3. 推荐方案：分层 + 双路径绑定

不要"一个 Rust 库硬绑四语言"，也不要"各语言各写一遍 provider"。仓库分为契约层（`aimux-core`）、引擎层（`aimux-providers` 等可复用 Rust 资产）、绑定层（`bindings/*`），引擎与绑定之间有两条缝可选——原生绑定直连引擎，C ABI 绑定经 `aimux-ffi`：

```
aimux/                         # Cargo workspace 只管 Rust crate
├── aimux-core/                # 契约层：数据类型 + trait（全量 serde，语言无关）
├── aimux-stream/              # SSE/NDJSON 解析原语（无 IO）
├── aimux-provider-utils/      # 重试/退避/key 加载（纯逻辑）
├── aimux-providers/           # 172 provider 引擎（硬绑 reqwest+tokio，全部绑定共用，不动）
├── aimux-ffi/                 # 【新增】C ABI 缝：opaque handle + 流 push 回调（C ABI 路径用）
├── aimux-macros/              # #[tool] Rust 语法糖（保留）
├── aimux-tools/               # 工具调用（Rust 侧）
├── Cargo.toml
└── bindings/                  # 各语言薄绑定，各自独立构建系统，不进 cargo workspace
    ├── python/                # PyO3 + maturin   ── 直吃 aimux-providers（原生路径）
    ├── node/                  # napi-rs          ── 直吃 aimux-providers（原生路径）
    ├── flutter/               # dart:ffi         ── 调 aimux-ffi（C ABI 路径，纯 Dart 无 Rust crate）
    ├── swift/                 # module.modulemap ── 调 aimux-ffi（C ABI 路径）
    ├── kotlin/                # JNA              ── 调 aimux-ffi（C ABI 路径）
    └── c/                     # 直接链接 .h      ── 调 aimux-ffi（C ABI 路径）
```

### 3.1 核心原则

FFI 边界只传三种东西，绝不传 Rust 的 trait / 泛型 / Stream：

1. **序列化后的 JSON**（数据）
2. **opaque handle**（对象，`u64` 整数 ID）
3. **callback**（流式回调）

各语言绑定负责把这些包装成符合该语言习惯的 API。

### 3.2 双路径绑定策略（v0.5 修订：Flutter 移至 C ABI 路径）

不要让所有语言都走 `aimux-ffi` 的 C ABI。按语言生态的 Rust 绑定工具成熟度分两条路径：

| 路径 | 适用语言 | 依赖 | 理由 |
|------|---------|------|------|
| **原生绑定** | Python / Node | `aimux-core` + `aimux-providers`，**绕过 `aimux-ffi`** | PyO3 / napi-rs 能直接映射 Rust 类型与 async，DX 最好，少一层间接 |
| **C ABI 绑定** | Swift / Kotlin / Flutter / C/C++ | `aimux-ffi` + 手写 wrapper（dart:ffi / JNA / module.modulemap / 直接链接） | 这些语言没有自包含的 Rust 原生绑定工具，或工具的 codegen 步骤在 aimux 的 JSON 边界下不产生价值 |

> **v0.5 修正**：Flutter 原列于原生路径（flutter_rust_bridge），评估后移至 C ABI 路径。原因：① flutter_rust_bridge 的 `StreamSink` 不公开导出，需 codegen + Flutter SDK 才能编译，不像 PyO3/napi-rs 那样自包含；② aimux 跨边界协议是 JSON（§3.1），frb 的"自动映射 Rust 类型→Dart"优势在 JSON 边界下不成立；③ dart:ffi 直接调 aimux-ffi 的 6 个 C 函数，和 Swift/Kotlin 统一路径，零额外工具链。详见 §5.5 调研记录。

**组织影响**：`bindings/python`、`bindings/node` 的 Cargo.toml 依赖 `aimux-providers`，**不依赖** `aimux-ffi`。`bindings/swift`、`bindings/kotlin`、`bindings/flutter`、`bindings/c` 调用 `aimux-ffi` 的 C ABI。Flutter 绑定纯 Dart，无 Rust crate。

---

## 4. FFI 边界设计要点

### 4.1 流的跨边界抽象（头号难题，最值得现在定型）

当前 [generate.rs:101](../aimux-core/src/generate.rs#L101) 直接暴露 `Pin<Box<dyn Stream>>`。FFI 层需把它转成各语言能消费的形式。

**v0.2.1 结论：C ABI 路径只做 push（回调），不做 pull（轮询）。**

- **Push 模式**（回调，采用）：`register_callback(handle, on_part, on_done, on_error)`
  - Rust 侧 `spawn` 一个 task 在 tokio 里正常 `.next().await`，每拿到一个 chunk 就回调通知外语。外语侧把回调数据塞进 channel/buffer，自己的 AsyncSequence/Flow 从 buffer 里 pull。
  - **为什么必须 push**：C ABI 同步函数（`extern "C"`）无法 `.await` Rust 的 async stream。pull 模式的 `stream_next(handle) -> Option<json>` 要拿到下一个 chunk 只能 block 当前线程等——在 Swift/Kotlin 主线程上会**卡死 UI**，这是硬伤。push 用回调绕开了同步阻塞。
  - 传输端 push，消费端 pull，中间用 channel 解耦——这是跨语言 stream 的事实标准（napi-rs / PyO3 / flutter_rust_bridge 均此模式）。
- ~~**Pull 模式**（轮询）：`stream_next(handle) -> Option<StreamPartJson>`~~ — **v0.2.1 否决**：C ABI 同步函数无法 await async stream，pull 必然退化为阻塞等待，在主线程环境卡死。Swift `AsyncSequence` / Kotlin `Flow` 的消费端虽然是 pull，但那通过 channel/buffer 衔接 push 传输端即可，不需要 FFI 层也做 pull。

> 注：此结论**只影响 `aimux-ffi`（Swift/Kotlin/C 路径，第三梯队）**。Node/Python/Flutter 走原生绑定，napi-rs / PyO3 / flutter_rust_bridge 各自自带 Rust Stream → 该语言 async 的桥接，不经过 push/pull 这层设计。旗舰绑定 Node 不受影响。

`StreamPart` 已是可序列化 enum（见 [stream_part.rs](../aimux-core/src/stream_part.rs)），转成 tagged JSON 即可跨边界。

> ⚠️ 这是整个方案工作量最大、也最值得在 review 期就定型的部分。

### 4.2 Opaque handle + 注册表（替代 trait object）

FFI 不能传 `Box<dyn LanguageModel>`。在 `aimux-ffi` 维护 `Arc<dyn LanguageModel>` 的整数 ID 注册表：

```rust
// aimux-ffi 草案
static REGISTRY: Mutex<HashMap<u64, Arc<dyn LanguageModel>>> = ...;

fn create_openai_model(api_key: &str, model_id: &str) -> u64;   // 返回 handle
fn generate_text(model_handle: u64, prompt_json: &str, opts_json: &str) -> String; // JSON 结果
fn drop_handle(handle: u64);                                     // 析构
```

各语言拿到 `u64` handle 后包装成对象，析构时调 `drop_handle`。JVM/.NET 需用 `Closeable` / `IDisposable` 模式显式释放，避免 native 内存泄漏。

### 4.3 工具定义：已是数据描述，无需改造

> **v0.2.1 修正**：原章节标题"从宏改成数据描述"是错误前提——核对代码后发现数据描述**早已就绪**。

[tool.rs](../aimux-core/src/tool.rs) 的核心类型全部已是语言无关的数据描述，且已派生 serde：

- `FunctionTool`（[tool.rs:10](../aimux-core/src/tool.rs#L10)）：`name` + `input_schema: Value`（JSON Schema）+ 已 `#[derive(Serialize, Deserialize)]` ✅
- `ToolCall` / `ToolResult`（[tool.rs:96](../aimux-core/src/tool.rs#L96) / [:107](../aimux-core/src/tool.rs#L107)）：已 serde ✅

`#[tool]` 宏（[aimux-macros](../aimux-macros/src/lib.rs)）+ `ToolExecutor`（[tool_executor.rs](../aimux-tools/src/tool_executor.rs)）是 **Rust 侧便利工具**，只给 Rust 用户用。aimux 本身**不做 agent loop**（见 [README](../README.md)），不执行工具——工具执行是用户层的事。

跨语言时无需任何改造：外语用户用对象构造 `FunctionTool` 传入，收到 `ToolCall` 后**在外语侧自己执行**，再把 `ToolResult` 喂回下一轮。全链路都是已序列化的数据，不碰 Rust 宏。

~~唯一遗漏：`ToolChoice`（[tool.rs:117](../aimux-core/src/tool.rs#L117)）缺 `Serialize/Deserialize`，补一个 derive 即可（3 分钟），不算改造。~~ — **v0.2.1 已完成**：`ToolChoice` 已补 serde，且 wire format 对齐 AISDK（`"auto"|"none"|"required"|{type:"tool",toolName}`），手写 Serialize/Deserialize + 8 个契约测试（[tool_choice_test.rs](../aimux-core/tests/tool_choice_test.rs)）。这是跨语言 wire schema 对齐的第一个落地样例。

### 4.4 数据类型加 serde + 固定 wire schema

许多跨边界类型目前没派生 serde（如 `StreamPart` 只有 `Debug`）。FFI 走 JSON 边界要求所有跨边界类型可序列化。建议：

- 所有跨边界类型加 `#[derive(Serialize, Deserialize)]`
- 用版本字段（如 `"specVersion": "v4"`）锁定契约，避免 Rust 端重构时悄悄破坏其他语言

### 4.5 ~~隔离 tokio / reqwest 依赖~~ — **v0.2 删除**

~~providers 现在硬绑 `reqwest` + `tokio`。建议抽象一个 `HttpTransport` trait，允许注入原生 HTTP 栈（iOS `URLSession`、Android `OkHttp`）。~~

**删除理由**：reqwest + rustls 本身跨平台，能编到 iOS/Android，无需为移动端单独抽象传输层。所有绑定吃同一个完整引擎，移动端体积问题改用 tokio feature 收窄 + `strip` + LTO 解决，不动架构。真正占体积的是 tokio runtime 而非 reqwest。抽象 `HttpTransport` 会让 172 个 provider 全改签名，工作量大、收益仅落在移动端，得不偿失。

---

## 5. 各语言方案

### 5.1 第一梯队（必须做）

| 语言 | 工具 | 关键点 |
|------|------|--------|
| **Node.js** | `napi-rs` | **旗舰绑定，第一个做**。对标 Vercel AI SDK，在其主场证明 Rust 核心竞争力；napi-rs 直接暴露 Promise/AsyncIterator，DX 不输原生 TS；原生绑定路径，绕过 `aimux-ffi` |
| **Python** | `PyO3` + `maturin` | AI/ML 母语；async generator 映射 Rust Stream；卖点"轻量统一 provider 抽象 + 性能 + 无 GIL 瓶颈的流解析"；第二优先级 |
| **Swift** | module.modulemap + 手写 C FFI | callback 流 → `AsyncSequence`；iOS 二进制需 `xcframework`；C ABI 路径 |
| **Kotlin** | JNA | 自动映射 C ABI；Android 需 `.so` + `.aar`；`Closeable` 显式释放 handle；C ABI 路径 |
| **Flutter/Dart** | `dart:ffi` 手写 | 直接调 aimux-ffi 的 6 个 C 函数；无需 codegen；C ABI 路径（v0.5 从 flutter_rust_bridge 改为 dart:ffi） |

> **v0.2 调整**：首个绑定定为 **Node.js**（napi-rs）。战略目标是战胜 Vercel AI SDK，Node 是主战场，第一个绑定必须在 AISDK 主场证明 Rust 核心。Python 退为第二优先级——验证流式跨边界可行性的任务也由 Node 承担（napi-rs 的 AsyncIterator 同样能验证流式一致性，且直接服务旗舰目标）。

### 5.2 第二梯队（几乎免费，顺手做）

| 语言 | 工具 | 说明 |
|------|------|------|
| **C / C++** | `cbindgen` 生成 `.h` | `aimux-ffi` 本身就是 C ABI，几乎零成本；C++ 包一层 RAII 即可。嵌入式/边缘/游戏引擎场景 |
| **Zig** | 直接 `extern "C"` | 与 C 同路径，生态小但与 Rust 用户群重合 |

### 5.3 第三梯队（值得做但各有摩擦）

| 语言 | 工具 | 摩擦点 |
|------|------|--------|
| **Go** | `cgo` + C ABI，或 gRPC sidecar | CGo 跨边界有损耗；Go 无 async，需 goroutine + channel 转 Rust Stream；SDK 形态下 gRPC sidecar 偏重 |
| **Java / Scala** | `UniFFI`（与 Kotlin 共用）或手写 JNI | 企业市场、Spark 内调 LLM；JVM GC 需 `Closeable` 显式释放 |
| **C# / .NET** | `UniFFI` 或 `P/Invoke` | Windows 生态、Unity；`IAsyncEnumerable<T>` 需手动接一层 |

### 5.4 不建议做

Ruby / PHP / Elixir / Perl / Lua —— AI 场景份额低，维护成本 > 收益。

### 5.5 双路径总结（v0.5 修订：UniFFI 评估后不采用）

不要为每门语言单独设计 FFI。按 §3.2 双路径策略：

| 路径 | 工具链 | 覆盖语言 | 依赖 |
|------|--------|---------|------|
| **原生绑定** | `PyO3` + maturin | Python | `aimux-providers` 直连 |
| **原生绑定** | `napi-rs` | Node.js | `aimux-providers` 直连 |
| **C ABI 绑定** | 手写 wrapper（module.modulemap） | Swift | `aimux-ffi` |
| **C ABI 绑定** | 手写 wrapper（JNA） | Kotlin / Java | `aimux-ffi` |
| **C ABI 绑定** | 手写 wrapper（dart:ffi） | Flutter / Dart | `aimux-ffi` |
| **C ABI 绑定** | 手写 wrapper（直接链接 .h） | C / C++ | `aimux-ffi` |

> ~~`UniFFI`~~ — **v0.5 评估后不采用**。原因：
> 1. aimux-ffi 只有 6 个 C 函数，手写 Swift/Kotlin/Dart 包装各 ~150 行，维护成本极低；UniFFI 的 codegen 杠杆在极窄 C ABI 面下不成立。
> 2. UniFFI 有自己的 FFI 层，不能复用已有的 aimux-ffi C ABI——需从 Rust trait 重新生成 FFI glue，等于让 aimux-ffi 白做。
> 3. UniFFI 的 async/stream 支持仍在改进中（callback interface），不如当前 push callback → channel → AsyncSequence/Sequence 链路成熟。
> 4. 原生绑定路径已覆盖 Python/Node；Swift/Kotlin/Flutter 手写包装足够薄。UniFFI 的"一次定义多语言生成"优势在双路径架构下落空。

> ~~`flutter_rust_bridge`~~ — **v0.5 评估后不采用**。原因：
> 1. `StreamSink` 不公开导出，需 codegen + Flutter SDK 才能编译，不像 PyO3/napi-rs 那样自包含（`cargo build` 即可用）。
> 2. aimux 跨边界协议是 JSON（§3.1），frb 的核心价值"自动映射 Rust 类型→Dart class"在 JSON 边界下不成立。
> 3. dart:ffi 直接调 aimux-ffi 的 6 个 C 函数，和 Swift/Kotlin 统一路径，零额外工具链。
> 4. 同理否决 Rinf（事件信号系统，仍需 codegen）和 membrane（stream-first，为硬件数据流设计，非通用场景）。ffigen（Dart 官方 FFI 生成器）是备选——当 aimux-ffi 膨胀到几十个函数时可引入自动生成。
>
> **重新考虑条件**：若 aimux-ffi 的 C ABI 函数从 6 个膨胀到几十个（如暴露全模态 FFI），手写包装成本上升，届时 UniFFI 才有杠杆。

前期架构投入做好后，新增一门语言往往是"加一个 binding 目录 + CI 构建"的工作量。原生绑定路径新增语言只需写薄包装 + 对接 Rust async；C ABI 路径则需 `aimux-ffi` 的 handle/callback 已覆盖所需能力。

---

## 6. Review 期前置改造项

以下改动在单语言状态下也是好设计，不会白做，建议在 review 阶段就推进：

| # | 改造 | 对应章节 | 优先级 | 状态 |
|---|------|---------|--------|------|
| 1 | `aimux-ffi` 流式 push 回调抽象（仅 C ABI 路径用；~~pull + push 双模式~~ → v0.2.1 改为只做 push） | §4.1 | 低（第三梯队，不阻塞 Node） | ✅ 完成 |
| 2 | `Box<dyn LanguageModel/Provider>` → opaque handle + 注册表 | §4.2 | 高 | ✅ 完成 |
| 3 | ~~工具定义从 `#[tool]` 宏改为数据描述~~ — **已就绪，无需改造**（§4.3）；✅ `ToolChoice` serde 已补且对齐 AISDK wire format（v0.2.1 完成） | §4.3 | ✅ 完成 | ✅ 完成 |
| 4 | 跨边界类型全量加 `#[derive(Serialize, Deserialize)]` + 版本字段；同时加 `ts-rs` 派生，为 Node 绑定自动生成 `.d.ts` | §4.4 / §9-7 | 高 | ✅ 完成 |

> ~~第 5 项「抽象 HttpTransport trait」~~ — **v0.2 删除**，见 §4.5。

---

## 7. 落地路线图

| 阶段 | 内容 | 产出 | 状态 |
|------|------|------|------|
| **阶段 0（review 期）** | 完成 §6 前置改造 1–4 | 单语言下也更好维护的核心 | ✅ 完成 |
| **阶段 1** | 选 **Node.js**（`napi-rs`）做第一个绑定 PoC | 旗舰绑定，在 AISDK 主场验证 Rust 核心竞争力 + 流式跨边界可行性 | ✅ 完成 |
| **阶段 2** | `aimux-ffi` C ABI + JSON wire schema 定型；CI 矩阵构建各平台二进制 | `.so`/`.dylib`/`.dll`/`.aar`/`.framework` | ✅ 完成（头文件 + CI 矩阵 + C/C++ 示例） |
| **阶段 3** | Python（PyO3，原生绑定）、Flutter（原生绑定）、Swift、Kotlin（C ABI 绑定） | 覆盖第二优先级 + 移动端 | ✅ 完成 |
| **阶段 4** | C/C++ 绑定 + 契约测试 | 覆盖第二梯队 + 一致性保证 | ✅ 完成（C/C++ 示例 + 共享 JSON 契约测试框架） |

### 7.1 CI / 发布

- 每个绑定独立发版：`npm` / `PyPI` / SPM / Maven / `pub.dev`
- 核心 Rust crate 发 `crates.io`
- GitHub Actions 矩阵构建各平台产物

### 7.2 契约测试

用同一组 JSON 测试夹具驱动所有语言，确保 provider 行为一致。

---

## 8. 风险

| # | 风险 | 说明 | 缓解 |
|---|------|------|------|
| 1 | **流的跨边界一致性** | Rust Stream 是 lazy pull，JS/Swift 是 push-based async。转换层若有 bug，会丢 chunk、背压失效、内存泄漏 | 专门设计 + 压测；Node PoC 优先验证（napi-rs AsyncIterator） |
| 2 | **tokio runtime 嵌入** | Rust 核心需在各语言进程里起 tokio runtime，要处理生命周期、线程安全、与各语言事件循环协作 | napi-rs / flutter_rust_bridge 有现成模式；Kotlin/Swift 需手动管理 |
| 3 | **二进制体积** | 移动端对体积敏感，Rust 核心 + tokio + reqwest 可能偏大 | tokio feature 收窄（`full`→按需）+ strip + LTO；~~HttpTransport 抽象~~（v0.2 删除）|
| 4 | ~~**Node 价值不明确**~~ — **v0.2 推翻**：Node 是旗舰战场，价值明确（战胜 Vercel AI SDK）。真实风险改为下条 |
| 5 | **旗舰绑定必须 DX 不输原生 TS** | Node 绑定若 API 体验、流式手感、类型完整度不如 Vercel AI SDK，则无法实现"战胜"目标 | 对齐 AISDK 的 TS API 形状；AsyncIterator 流式手感逐项比对；类型定义从 Rust serde 自动生成（ts-rs / specta）|

---

## 9. 待决策问题（Open Questions）

以下需在评审中明确，决定方案走向：

1. ~~**动机排序**：多语言化的首要目标是移动端、Python、还是 Node？~~ — **v0.2 已决**：Node 优先（旗舰，对标 AISDK）→ Python 第二 → 移动端第三。
2. ~~**Node 是否做**~~ — **v0.2 已决**：做，且最高优先级。目标是战胜 Vercel AI SDK，Node 是主战场。
3. **FFI 工具选型**：`UniFFI` 一统多语言，还是各语言用最佳工具（PyO3 / napi-rs / flutter_rust_bridge 各自独立）？前者省心后者体验好。
   - **v0.2 已决**：双路径——Python/Node/Flutter 走原生绑定，Swift/Kotlin/C 走 C ABI（§3.2）。不追求一统。
   - **v0.5 补充：UniFFI 不采用**。aimux-ffi 只有 6 个 C 函数，手写 Swift/Kotlin 包装各 ~150 行，UniFFI 的 codegen 杠杆不成立；且 UniFFI 有自己的 FFI 层无法复用 aimux-ffi。详见 §5.5。
4. ~~**HTTP 栈策略**：移动端是否必须支持注入原生 HTTP~~ — **v0.2 已决**：不抽象，接受 Rust 内置 reqwest（§4.5 已删除）。
5. ~~**流式传输模式**：pull / push 双模式都做，还是先做一种？~~ — **v0.2.1 已决**：只做 push（回调）。C ABI 同步函数无法 await async stream，pull 必然阻塞（主线程卡死）。只影响 aimux-ffi（Swift/Kotlin/C），Node 等原生绑定不受影响（§4.1）。
6. ~~**工具定义改造时机**：`#[tool]` 宏改造为数据描述，是 review 期就做，还是等第一个绑定落地时再改？~~ — **v0.2.1 已决：伪问题，无需改造**。核对代码发现 `FunctionTool`/`ToolCall`/`ToolResult` 已是语言无关数据描述且已 serde（§4.3）。`#[tool]` 宏 + `ToolExecutor` 是 Rust 侧便利工具，aimux 不做 agent loop、不执行工具，外语侧自行执行。仅 `ToolChoice` 缺 serde，补 derive 即可。
7. **Node 绑定的 TS 类型生成**：用 `ts-rs` / `specta` 从 Rust serde 派生自动生成 `.d.ts`，还是手写 TS 类型？
   - **v0.2.1 已决**：自动生成。手写类型迟早与 Rust 核心漂移，对旗舰绑定是硬伤。`ts-rs` 或 `specta` 二选一，在 §6 前置改造阶段为跨边界类型加派生。

---

## 修订记录

| 日期 | 版本 | 说明 |
|------|------|------|
| 2026-07-26 | DRAFT v0.1 | 初稿，基于代码现状分析与多语言方案讨论 |
| 2026-07-28 | v0.2 | 评审修订：删除 HttpTransport 抽象（§4.5/§6-5/§9-4）；确立双路径绑定策略（§3.2/§5.5，原生绑定 vs C ABI） |
| 2026-07-28 | v0.2.1 | 战略修订：Node 升为旗舰绑定、最高优先级（目标是战胜 Vercel AI SDK，§1.2/§5.1/§7/§8/§9）；首个绑定 Python→Node；Node 从第三梯队升至第一梯队；新增风险5（DX 不输原生 TS）；新增待决7（TS 类型自动生成）；流式 C ABI 否决 pull 只做 push（§4.1/§6-1/§9-5，原因：C ABI 同步函数无法 await async stream）；TS类型生成定为自动生成（§9-7 已决）；工具定义修正为"已就绪无需改造"（§4.3/§6-3/§9-6，伪问题，数据描述早已完成）；全部开放问题收敛 |
| 2026-07-28 | v0.3 | **阶段 0 + 阶段 1 落地**：§6 全部前置改造完成（87 个跨边界类型补 serde+ts-rs，80 个 TS 类型文件自动生成；aimux-ffi crate 创建，handle 注册表 + push 流式回调 + 6 个 C ABI 符号导出）；阶段 1 旗舰 Node.js 绑定完成（napi-rs v3 原生绑定，绕过 aimux-ffi 直连 aimux-providers；generateText/streamText PoC + AsyncGenerator 流式 + 6 个测试通过）；全 workspace 编译测试通过 |
| 2026-07-29 | v0.4 | **阶段 2 + 3(Python) + 4 落地**：阶段 2 — aimux-ffi C 头文件（aimux-ffi.h）+ GitHub Actions CI 矩阵（Rust test / Node binding / Python binding / ffi build / contract tests，跨 Linux/macOS/Windows）；阶段 3 — Python 绑定完成（PyO3 原生路径，6/6 测试通过）；阶段 4 — C/C++ 绑定示例（RAII wrapper）+ 契约测试框架（共享 13 个 JSON wire-format 夹具，Rust 9/9 + Node 16/16 双端验证） |
| 2026-07-29 | v0.5 | **阶段 3 移动端绑定全部落地**：Swift（Swift Package + module.modulemap，C ABI 路径，ARC 管理 handle，AsyncSequence 流式）；Kotlin（JNA 包装 C ABI，Closeable 显式释放，Sequence 流式）；Flutter（flutter_rust_bridge v2 原生路径，handle 注册表 + channel 流式）。全部编译通过。CI 矩阵新增 Swift/Kotlin 构建 job。bindings/README.md 汇总 6 语言绑定。RFC-0001 全部阶段完成 |
| 2026-07-29 | v0.5.1 | **Flutter 路径修正**：调研后否决 flutter_rust_bridge（StreamSink 不公开导出需 codegen；JSON 边界下类型映射优势不成立），否决 UniFFI/Rinf/membrane（同理）。Flutter 改为 dart:ffi 手写调 aimux-ffi（C ABI 路径），和 Swift/Kotlin 统一。纯 Dart 无 Rust crate，零额外工具链。§3.2/§5.1/§5.5 同步更新 |
