# RFC-0001：多语言绑定（Multi-language Bindings）

> **状态**：DRAFT（草案，待评审）
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

- **Node 绑定**的边际价值需论证——TS 生态已有官方实现。卖点只能是"同一套 Rust 核心 + 边缘/性能场景"。
- **Swift / Kotlin / Flutter** 生态没有官方统一 AI SDK，移动端对二进制体积、性能、离线敏感，Rust 核心有竞争力——**这是多语言化真正的价值区**。
- **Python** AI/ML 开发者基数最大，虽有 LangChain/LlamaIndex，但缺"轻量、统一 provider 接口、不绑架架构"的 SDK，Rust 核心可补位。

**建议**：动机以移动端（Swift/Kotlin/Flutter）+ Python 为主，Node 作为可选。

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
| providers 硬绑 `reqwest` + `tokio` | 拉入整个 Rust async runtime | 移动端会与原生 HTTP 栈重复，增大二进制 |

---

## 3. 推荐方案：混合分层 + FFI 共享核心

不要"一个 Rust 库硬绑四语言"，也不要"各语言各写一遍 provider"。仓库重构为三层：

```
aimux/
├── aimux-core/          # 数据类型 + trait 抽象（FFI 契约层，serde 序列化定义边界）
├── aimux-providers/     # provider HTTP 适配 + 流解析（Rust 参考引擎，高价值复用资产）
├── aimux-stream/        # SSE/NDJSON 原语
├── aimux-ffi/           # 【新增】FFI 边界层：opaque handle + callback 流 + C ABI
└── bindings/            # 【新增】各语言薄绑定
    ├── node/            # napi-rs
    ├── python/          # PyO3 + maturin
    ├── swift/           # Swift Package（swift-bridge 或手写 C FFI）
    ├── kotlin/          # UniFFI 生成 JNI
    └── flutter/         # flutter_rust_bridge
```

### 3.1 核心原则

FFI 边界只传三种东西，绝不传 Rust 的 trait / 泛型 / Stream：

1. **序列化后的 JSON**（数据）
2. **opaque handle**（对象，`u64` 整数 ID）
3. **callback**（流式回调）

各语言绑定负责把这些包装成符合该语言习惯的 API。

---

## 4. FFI 边界设计要点

### 4.1 流的跨边界抽象（头号难题，最值得现在定型）

当前 [generate.rs:101](../aimux-core/src/generate.rs#L101) 直接暴露 `Pin<Box<dyn Stream>>`。FFI 层需把它转成各语言能消费的形式。提供两种模式，让绑定自选：

- **Pull 模式**（轮询）：`stream_next(handle) -> Option<StreamPartJson>`
  - 适合 Dart / Kotlin 的 iterator
- **Push 模式**（回调）：`register_callback(handle, on_part, on_done, on_error)`
  - 适合 JS 的 async iterator / Swift 的 AsyncSequence

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

### 4.3 工具定义从"宏"改成"数据描述"

[aimux-macros](../aimux-macros/src/lib.rs) 的 `#[tool]` 是 Rust 编译期宏，只对 Rust 用户有用。多语言化后，工具定义需有 **语言无关的描述形式**：

- **参数**：JSON Schema
- **执行回调**：注册到 handle

Rust 端保留 `#[tool]` 作为语法糖生成这种描述；其他语言直接用数据构造 + 注册回调。

### 4.4 数据类型加 serde + 固定 wire schema

许多跨边界类型目前没派生 serde（如 `StreamPart` 只有 `Debug`）。FFI 走 JSON 边界要求所有跨边界类型可序列化。建议：

- 所有跨边界类型加 `#[derive(Serialize, Deserialize)]`
- 用版本字段（如 `"specVersion": "v4"`）锁定契约，避免 Rust 端重构时悄悄破坏其他语言

### 4.5 隔离 tokio / reqwest 依赖（移动端关键）

providers 现在硬绑 `reqwest` + `tokio`（见 [aimux-providers/Cargo.toml](../aimux-providers/Cargo.toml)）。建议抽象一个 `HttpTransport` trait，允许注入原生 HTTP 栈（iOS `URLSession`、Android `OkHttp`）。这样移动端可以不拉 Rust HTTP 栈，大幅减小体积并复用系统证书/代理。

> 这是移动端能否接受的决定性因素。

---

## 5. 各语言方案

### 5.1 第一梯队（必须做）

| 语言 | 工具 | 关键点 |
|------|------|--------|
| **Python** | `PyO3` + `maturin` | AI/ML 母语；async generator 映射 Rust Stream；卖点"轻量统一 provider 抽象 + 性能 + 无 GIL 瓶颈的流解析" |
| **Swift** | `swift-bridge` 或手写 C FFI + Swift Package | callback 流 → `AsyncSequence`；iOS 二进制需 `xcframework` |
| **Kotlin** | Mozilla `UniFFI` | 自动生成 JNI，类型映射好；Android 需 `.so` + `.aar`；用 `Closeable` 显式释放 handle |
| **Flutter/Dart** | `flutter_rust_bridge` | 专为 Flutter 设计，async stream 支持最完整；建议作为**第一个绑定**验证流式传输可行性 |

### 5.2 第二梯队（几乎免费，顺手做）

| 语言 | 工具 | 说明 |
|------|------|------|
| **C / C++** | `cbindgen` 生成 `.h` | `aimux-ffi` 本身就是 C ABI，几乎零成本；C++ 包一层 RAII 即可。嵌入式/边缘/游戏引擎场景 |
| **Zig** | 直接 `extern "C"` | 与 C 同路径，生态小但与 Rust 用户群重合 |

### 5.3 第三梯队（值得做但各有摩擦）

| 语言 | 工具 | 摩擦点 |
|------|------|--------|
| **Node.js** | `napi-rs` | 价值需论证（TS 已有官方 SDK）；技术成熟，能直接暴露 Promise/AsyncIterator |
| **Go** | `cgo` + C ABI，或 gRPC sidecar | CGo 跨边界有损耗；Go 无 async，需 goroutine + channel 转 Rust Stream；SDK 形态下 gRPC sidecar 偏重 |
| **Java / Scala** | `UniFFI`（与 Kotlin 共用）或手写 JNI | 企业市场、Spark 内调 LLM；JVM GC 需 `Closeable` 显式释放 |
| **C# / .NET** | `UniFFI` 或 `P/Invoke` | Windows 生态、Unity；`IAsyncEnumerable<T>` 需手动接一层 |

### 5.4 不建议做

Ruby / PHP / Elixir / Perl / Lua —— AI 场景份额低，维护成本 > 收益。

### 5.5 统一建议

不要为每门语言单独设计 FFI。**先把 `aimux-ffi` 的 C ABI + JSON wire schema 锁死**，然后：

| 工具链 | 一次配置覆盖的语言 |
|--------|-------------------|
| `UniFFI` | Kotlin / Java / Swift / C# / Python（部分）|
| `flutter_rust_bridge` | Dart / Flutter |
| `PyO3` + `maturin` | Python（最佳体验，建议独立做）|
| `napi-rs` | Node.js |
| `cbindgen` | C / C++ / Zig |

前期架构投入做好后，新增一门语言往往是"加一个 binding 目录 + CI 构建"的工作量。

---

## 6. Review 期前置改造项

以下改动在单语言状态下也是好设计，不会白做，建议在 review 阶段就推进：

| # | 改造 | 对应章节 | 优先级 |
|---|------|---------|--------|
| 1 | 引入 FFI 友好的"可拉取流"抽象（pull + push 双模式） | §4.1 | 高 |
| 2 | `Box<dyn LanguageModel/Provider>` → opaque handle + 注册表 | §4.2 | 高 |
| 3 | 工具定义从 `#[tool]` 宏改为数据描述 + 回调注册 | §4.3 | 中 |
| 4 | 跨边界类型全量加 `#[derive(Serialize, Deserialize)]` + 版本字段 | §4.4 | 高 |
| 5 | 抽象 `HttpTransport` trait，允许注入原生 HTTP 栈 | §4.5 | 中（移动端关键）|

---

## 7. 落地路线图

| 阶段 | 内容 | 产出 |
|------|------|------|
| **阶段 0（review 期）** | 完成 §6 前置改造 1–5 | 单语言下也更好维护的核心 |
| **阶段 1** | 选 **Flutter**（`flutter_rust_bridge`）做第一个绑定 PoC | 验证流式传输跨边界可行性（async stream 支持最完整）|
| **阶段 2** | `aimux-ffi` C ABI + JSON wire schema 定型；CI 矩阵构建各平台二进制 | `.so`/`.dylib`/`.dll`/`.aar`/`.framework` |
| **阶段 3** | Python（PyO3）、Swift、Kotlin 绑定 | 覆盖第一梯队 |
| **阶段 4** | Node、C/C++ 绑定 + 契约测试 | 覆盖第二梯队 + 一致性保证 |

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
| 1 | **流的跨边界一致性** | Rust Stream 是 lazy pull，JS/Swift 是 push-based async。转换层若有 bug，会丢 chunk、背压失效、内存泄漏 | 专门设计 + 压测；Flutter PoC 优先验证 |
| 2 | **tokio runtime 嵌入** | Rust 核心需在各语言进程里起 tokio runtime，要处理生命周期、线程安全、与各语言事件循环协作 | napi-rs / flutter_rust_bridge 有现成模式；Kotlin/Swift 需手动管理 |
| 3 | **二进制体积** | 移动端对体积敏感，Rust 核心 + tokio + reqwest 可能偏大 | `HttpTransport` 抽象 + `no_std` 友好裁剪 + strip + LTO |
| 4 | **Node 价值不明确** | TS 已有官方 SDK | 先论证卖点（边缘/性能），否则延后 |

---

## 9. 待决策问题（Open Questions）

以下需在评审中明确，决定方案走向：

1. **动机排序**：多语言化的首要目标是移动端（Swift/Kotlin/Flutter）、Python、还是 Node？这决定第一个绑定选谁。
2. **Node 是否做**：若卖点仅为"Rust 核心 + 性能"，是否值得维护成本？
3. **FFI 工具选型**：`UniFFI` 一统多语言，还是各语言用最佳工具（PyO3 / napi-rs / flutter_rust_bridge 各自独立）？前者省心后者体验好。
4. **HTTP 栈策略**：移动端是否必须支持注入原生 HTTP（`URLSession`/`OkHttp`），还是接受 Rust 内置 `reqwest`？前者工作量大但移动端体验好。
5. **流式传输模式**：pull / push 双模式都做，还是先做一种？双模式更通用但工作量大。
6. **工具定义改造时机**：`#[tool]` 宏改造为数据描述，是 review 期就做，还是等第一个绑定落地时再改？

---

## 修订记录

| 日期 | 版本 | 说明 |
|------|------|------|
| 2026-07-26 | DRAFT v0.1 | 初稿，基于代码现状分析与多语言方案讨论 |
