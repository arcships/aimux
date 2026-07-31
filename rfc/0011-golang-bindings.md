# RFC-0011：Golang 绑定

> **状态**：v0.1（PoC 落地：cgo + 静态链接 + 19 个测试全通过）
> **日期**：2026-07-31
> **关联**：[RFC-0001](0001-multilang-bindings.md) 多语言绑定、[aimux-ffi](../aimux-ffi/aimux-ffi.h)

---

## 1. 背景与动机

RFC-0001 §5.3 将 Go 列入"第三梯队（值得做但各有摩擦）"，理由是"CGo 跨边界有损耗；Go 无 async，需 goroutine + channel 转 Rust Stream；SDK 形态下 gRPC sidecar 偏重"。本 RFC 重新评估后，将 Go 升级为**立即执行**的下一个绑定。

### 1.1 为什么现在做 Go

aimux 当前覆盖 6 种语言绑定（Node/Python/Swift/Kotlin/Flutter/C++），**唯独缺 Go**。而 Go 是云原生后端的母语，"在微服务里调 LLM" 的场景体量很大。

2025 下半年 Go 的 AI SDK 生态集中爆发，证明需求真实且在增长：

| 框架 | 来源 | 时间 |
|------|------|------|
| **Eino** | 字节跳动 CloudWeGo | 2025 |
| **ADK-Go** | Google | 2025-11 达 1.0 |
| **Genkit-Go** | Firebase | 2025-11 |
| **LangChainGo** | 社区 | 持续更新 |
| **GoAI** | 社区 | 2026，"22+ LLM providers, 2 dependencies" |

其中 GoAI 的卖点（"轻量、统一 provider 接口、不多绑架架构"）与 aimux 几乎一致——说明 aimux 的定位在 Go 生态有真实空白。关键信号：[Zep 的调研](https://blog.getzep.com/agentic-development-in-go/) 指出 Go 里"大多数团队跳过框架"——这正是 aimux "只做 provider 统一、不做编排" 的定位缺口。

### 1.2 RFC-0001 §5.3 的三条"摩擦点"重新评估

| 原担忧 | 重新评估 | 结论 |
|--------|---------|------|
| "CGo 跨边界有损耗" | cgo 调用开销 ~200ns/次，对网络 IO 的 LLM 调用可忽略 | ❌ 不构成问题 |
| "Go 无 async，需 goroutine + channel 转 Rust Stream" | aimux-ffi 的 `aimux_stream_text` 本就是 **push-callback 同步阻塞**模型（RFC-0001 §4.1），专门为"没有 Rust 兼容 async 的语言"设计。Go 的 CSP 模型是它最自然的映射：goroutine 调阻塞 FFI，callback 往 channel 写，消费端 `for part := range ch` | ❌ 反而是最适合的 |
| "gRPC sidecar 偏重" | 本方案不采用 gRPC sidecar，走 cgo + 静态链接，单 binary | ❌ 不采用，问题消失 |

三条摩擦点全部化解。

---

## 2. 技术路径：cgo + 静态链接 aimux-ffi

### 2.1 路径选择

Go 没有 PyO3/napi-rs 级别的原生 Rust 绑定工具，只能走 C ABI 路径（与 Swift/Kotlin/Flutter/C++ 一致，见 RFC-0001 §3.2）：

```
Go 代码 ──cgo──→ aimux-ffi (libaimux_ffi.a 静态链接) ──→ aimux-providers
```

| 选项 | 评估 | 采用 |
|------|------|:----:|
| **cgo + 静态链接 `.a`** | 单 binary、零额外进程、与 C/C++ 绑定同路径 | ✅ |
| cgo + 动态链接 `.so` | 破坏 Go"单 binary"习惯，需随包分发 `.so` | ❌ |
| gRPC sidecar | 多一个进程、部署复杂、延迟多一跳 | ❌ |
| 纯 Go 重写 provider | 违背"复用 Rust 核心"战略，放弃 172 provider | ❌ |

### 2.2 单 binary 可行性（已实测）

aimux-ffi 的 [Cargo.toml](../aimux-ffi/Cargo.toml#L11) 已声明 `crate-type = ["cdylib", "staticlib", "rlib"]`，即 `libaimux_ffi.a` 静态库已产出。cgo 链 `.a` 后，Rust 核心被静态编进 Go 二进制。

**实测数据**（release profile 优化后，见 [Cargo.toml](../Cargo.toml) `[profile.release]`）：

| 产物 | 体积 |
|------|------|
| `libaimux_ffi.a`（静态库，含全部中间符号，未去重） | 82 MB |
| `libaimux_ffi.so`（动态库，已 strip） | 3.5 MB |
| **静态链进可执行文件（stripped，真实增量）** | **7.5 MB** |

链接器对 `.a` 做符号解析 + 死代码消除，真实增量只有 7.5MB（不是 `.a` 的 82MB）。预估最终 Go binary ≈ 12-13MB（Rust 核心 7.5MB + Go 自身 ~5MB），Go 生态正常范围。

`ldd` 验证：静态链后的可执行文件**只依赖 OS 自带基础库**：

```
linux-vdso.so.1
libm.so.6          ← glibc 自带
libc.so.6          ← glibc 自带
libgcc_s.so.1      ← gcc 运行时，系统自带
/ld-linux-x86-64.so.2
```

**不需要随包分发任何 `.so/.dll/.dylib`**，单 binary 成立。

### 2.3 跨平台

| 平台 | 静态链接 | 结果 |
|------|---------|------|
| **Linux** | 链 `.a`，配 `x86_64-unknown-linux-musl` + musl-gcc | 全静态 ELF，`ldd` 显示 `not a dynamic executable` |
| **macOS** | 链 `.a` | 一个二进制 + 依赖系统自带 libSystem，实质单文件 |
| **Windows** | 链 `.a` | 一个 .exe + 依赖系统自带 ucrt，实质单文件 |

唯一构建期要求：C 工具链（cgo 本来就需要）+ rustls 的 ring（含汇编）。这是构建环境要求，不影响分发产物。

---

## 3. 流式映射：push callback → channel

RFC-0001 §4.1 已定型 C ABI 路径只做 **push（回调）**，不做 pull。Go 的 CSP 模型是这个设计最自然的消费者：

```
[aimux-ffi]                [cgo]                    [Go 用户代码]
aimux_stream_text  →  C 回调 on_part(json)  →  channel<- part  →  for part := range ch
   (阻塞)              (同步，在 cgo 线程)         (goroutine)        (消费端)
```

- **生产端（push）**：cgo 在独立 goroutine 里调阻塞的 `C.aimux_stream_text`，C 回调把每个 StreamPart JSON 往 Go channel 写
- **消费端（pull）**：Go 用户用 `for part := range ch` 消费，符合 Go 习惯
- 中间用 buffered channel 解耦，避免背压

对比其他 C ABI 绑定的流式封装：

| 绑定 | 传输端 | 消费端 |
|------|--------|--------|
| Kotlin | JNA callback → `LinkedBlockingQueue` | `Sequence` |
| Swift | C callback → `AsyncStream` continuation | `AsyncSequence` |
| Flutter | dart:ffi callback → `StreamController` | `Stream` |
| **Go** | **cgo callback → channel** | **`for range`** |

Go 的实现反而最简洁——channel 是语言一等公民，不需要 `LinkedBlockingQueue` 或 `AsyncStream` 这层包装。

---

## 4. API 形状

对齐其他 C ABI 绑定（Kotlin/Swift/Flutter），保持 API 形状一致：

```go
package aimux

// OpenAI model, 静态链接 Rust 核心
model := aimux.OpenAI("sk-...", "gpt-4o")
defer model.Close()

// 非流式生成
result := model.GenerateText(`"What is Rust?"`)
// result 是 JSON 字符串：{"text":"...","usage":{...}}

// 自定义 base URL（Ollama / OpenRouter / 本地代理）
model := aimux.OpenAIWithBase("sk-...", "gpt-4o", "http://localhost:11434")

// 流式生成
stream := model.StreamText(`"Write a haiku"`)
for part := range stream {
    // part 是 StreamPart JSON：{"TextDelta":{"delta":"..."}}
    fmt.Println(part)
}
// stream 结束自动 close；若有错误，stream.Err() 返回
```

- `Model` 实现 `io.Closer`（对标 Kotlin 的 `Closeable` / Swift 的 `deinit`）
- `GenerateText` / `StreamText` 入参/出参都是 JSON 字符串（wire format 与其他绑定一致，见 [aimux-ffi.h](../aimux-ffi/aimux-ffi.h#L24)）
- 流式返回一个 `chan string`（或自定义 `Stream` 类型，封装 error 传递）

---

## 5. 目录结构

对齐其他 C ABI 绑定的组织方式：

```
bindings/go/
├── go.mod                    # module github.com/aimux/aimux-go
├── aimux.go                  # cgo 声明 + Model 类型 + Generate/Stream
├── stream.go                 # 流式 channel 封装
├── types.go                  # JSON wire types（可选，参考 Kotlin Types.kt）
├── aimux_test.go             # 单元测试
├── stream_test.go
├── examples/
│   └── generate/main.go      # 最小示例
└── README.md                 # 构建/使用说明（或复用 bindings/README.md）
```

预估代码量：wrapper ~250 行（与 Kotlin [Model.kt](../bindings/kotlin/src/main/kotlin/aimux/Model.kt) 201 行同量级，cgo 声明略多）。

---

## 6. cgo 实现要点

### 6.1 静态链接 `.a`

cgo 通过 `CFLAGS` / `LDFLAGS` 链接 `libaimux_ffi.a`：

```go
// #cgo CFLAGS: -I${SRCDIR}/../../aimux-ffi
// #cgo LDFLAGS: -L${SRCDIR}/../../target/release -laimux_ffi -lpthread -ldl -lm
// #include "aimux-ffi.h"
import "C"
```

构建前提：`cargo build -p aimux-ffi --release` 已产出 `target/release/libaimux_ffi.a`。

### 6.2 内存所有权

遵循 [aimux-ffi.h](../aimux-ffi/aimux-ffi.h#L10) 的契约：

- `aimux_generate_text` 返回的 `char*` **由 caller 释放**——Go 侧调 `C.aimux_free_string`，用 `defer` 保证释放
- `aimux_stream_text` 回调收到的 `const char*` **仅在回调期间有效**——回调内必须同步 `C.GoString` 拷贝出来，再往 channel 写

### 6.3 并发

- [aimux-ffi.h:20](../aimux-ffi/aimux-ffi.h#L20) 明确：所有 FFI 函数同步阻塞，回调在同一线程执行，**不可在回调内重入 FFI**
- Go 侧：`GenerateText` / `StreamText` 在独立 goroutine 调阻塞 FFI，不阻塞调用方的 goroutine（`StreamText` 立即返回 channel）
- tokio runtime 由 aimux-ffi 内部管理，Go 不感知

---

## 7. 发布策略

### 7.1 单 binary 分发

Go 绑定的最大优势：**直接 `go build` 产出单 binary**，用户无需安装 Rust 工具链。

- 库形态：发布为 Go module（`go.mod`），但需预编译的 `libaimux_ffi.a`（cgo 编译期需要）
- 应用形态：用户 `go build` 直接产出包含 Rust 核心的单 binary

### 7.2 跨平台构建

GitHub Actions 矩阵（对齐现有 [CI](../.github)）：

| 平台 | target | 链接方式 |
|------|--------|---------|
| Linux x86_64 | `x86_64-unknown-linux-musl` | 全静态 |
| Linux aarch64 | `aarch64-unknown-linux-musl` | 全静态 |
| macOS x86_64 | `x86_64-apple-darwin` | 动态链 libSystem |
| macOS aarch64 | `aarch64-apple-darwin` | 动态链 libSystem |
| Windows x86_64 | `x86_64-pc-windows-msvc` | 动态链 ucrt |

### 7.3 契约测试

复用 [contract-tests/fixtures/wire-format.json](../contract-tests/fixtures/wire-format.json) 共享 JSON 夹具，确保 Go 与其他 6 种语言 wire format 一致。

---

## 8. 风险

| # | 风险 | 说明 | 缓解 |
|---|------|------|------|
| 1 | **cgo 编译期需 `.a`** | 用户 `go get` 时若本地无 `libaimux_ffi.a` 则编译失败 | CI 预编译各平台 `.a` 并随 module 分发；或提供 `go generate` 脚本自动 `cargo build` |
| 2 | **cgo 调用开销** | ~200ns/次 | 对网络 IO 的 LLM 调用可忽略 |
| 3 | **回调线程安全** | cgo callback 在 C 线程执行，往 Go channel 写需 `runtime.cgocall` | Go runtime 保证 channel 跨 goroutine 安全；回调内只做 `C.GoString` + channel send |
| 4 | **二进制体积** | Rust 核心 7.5MB | 已用 release profile 优化（LTO + panic=abort + strip + opt-level=z），12MB → 7.5MB；Go 生态正常范围 |

---

## 9. 落地路线图

| 阶段 | 内容 | 状态 |
|------|------|------|
| **阶段 0** | release profile 优化（LTO/panic=abort/strip/opt-level=z） | ✅ 完成（commit c167273a，12MB→7.5MB） |
| **阶段 1** | 本 RFC 评审通过 | ✅ 完成 |
| **阶段 2** | `bindings/go/` PoC：cgo 声明 + Model + GenerateText + StreamText | ✅ 完成 |
| **阶段 3** | 契约测试（共享 wire-format 夹具） | ✅ 完成 |
| **阶段 4** | CI 矩阵（跨平台 `.a` 构建 + Go test） | ⏳ 待做 |
| **阶段 5** | 文档同步（bindings/README.md / 主 README.md / docs/API.md） | ✅ 完成 |

---

## 修订记录

| 日期 | 版本 | 说明 |
|------|------|------|
| 2026-07-31 | DRAFT v0.1 | 初稿：Go 绑定设计，升级 RFC-0001 §5.3 第三梯队为立即执行；cgo + 静态链接路径；push callback → channel 流式映射；单 binary 实测验证（7.5MB） |
| 2026-07-31 | v0.1 | **PoC 落地**：`bindings/go/` 完整实现（aimux.go cgo 声明 + Model + Generate/Stream；types.go typed JSON 类型；19 个测试全通过：7 单元 + 6 E2E + 6 契约子测试）。单 binary 实测 8.7MB（strip 后），静态链接 `libaimux_ffi.a`，零额外文件依赖 |
