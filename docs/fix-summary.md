# 审计问题修复总结

> 修复依据：`docs/ffi-audit-report.md`、`docs/lang-coverage-audit.md`  
> 约束遵循：未执行任何 Git 操作；未运行 `cargo build/test`。

## 已修复

### Swift（P0）

- `bindings/swift/Sources/Aimux/Types.swift:876-901`：`GenerateTextResult` 新增 `warnings: [Warning]`，补入 Codable keys 与初始化器；默认空数组保持已有调用兼容。
- `bindings/swift/Sources/Aimux/Aimux.swift:192-207`：raw async stream 改为 `AsyncThrowingStream<String, Error>`，native stream error 以 `AimuxError.streamError` 结束流，不再吞错。
- `bindings/swift/Sources/Aimux/Types.swift:1242-1258`：typed async stream 同步改为 `AsyncThrowingStream<StreamPart, Error>`，解码/native 错误向调用者抛出。
- `bindings/swift/Sources/Aimux/Aimux.swift:144-181,217-243`：把单一全局 `StreamContext.current` 改为 `Thread.current.threadDictionary` 中的 per-thread context，并在同步调用结束时恢复上一层 context；并发线程流互不覆盖。

### Kotlin（P0）

- `bindings/kotlin/src/main/kotlin/aimux/Model.kt:60-76`：handle 改为 `AtomicLong`；`close()` 使用 `getAndSet(0)`，保证 native drop 最多执行一次；调用已关闭模型时明确失败。
- `bindings/kotlin/src/main/kotlin/aimux/TypedModel.kt:146-158`：`StreamPart` 解码失败不再伪装成 `Unknown("<parse-error>")`，改为调用 `onError` 并保留具体解码错误。

### Flutter（P0）

- `bindings/flutter/lib/types.dart:29-71`：新增 `Role`、`FinishReasonUnified`、`ReasoningEffort` 三枚举及 wire value 转换。
- `bindings/flutter/lib/types.dart:568-590`：`GenerateTextResult` 新增 `warnings` 字段，默认空数组。
- `bindings/flutter/lib/types.dart:600-704`：`GenerateTextOptions` 从 4/15 补齐到 15/15，新增 stop sequences、top-p/top-k、presence/frequency penalty、response format、seed、headers、provider options、reasoning、instructions，并补齐 JSON 往返。
- `bindings/flutter/lib/types.g.dart:104-126`：同步生成代码中的 `GenerateTextResult.warnings` 序列化/反序列化。

### C++（P0）

- `bindings/c/example.cpp:13-92`：stream trampoline 使用 thread-local callback 指针，实际转发调用者传入的 `on_part`、`on_done`、`on_error`，不再用内部打印 lambda 忽略参数；不同线程的同步 stream 相互隔离。

### FFI header（P1）

- `aimux-ffi/aimux-ffi.h:169-185`：补齐原先遗漏的 6 个声明：`aimux_cohere_reranking_new`、`aimux_rerank`、`aimux_google_video_new`、`aimux_video_generate`、`aimux_tavily_search_new`、`aimux_search`，并新增 Reranking/Video/Search section。
- `aimux-ffi/aimux-ffi.h:84,132-185`：同步声明新增构造器及全部新增 `_with_base` API。

### FFI 构造器与 base URL（P1）

- `aimux-ffi/src/lib.rs:287-299`：新增 `aimux_deepseek_new`，使用原生 `DeepSeekConfig/DeepSeekProvider`。
- `aimux-ffi/src/lib.rs:474-497`：新增 OpenAI embedding `_with_base`、Cohere embedding、Google embedding 构造器。
- `aimux-ffi/src/lib.rs:546-557`：新增 OpenAI speech `_with_base`。
- `aimux-ffi/src/lib.rs:591-607`：新增 OpenAI image `_with_base` 与 Google image 构造器。
- `aimux-ffi/src/lib.rs:643-654`：新增 OpenAI transcription `_with_base`。
- `aimux-ffi/src/lib.rs:697-706`：新增 OpenAI files `_with_base`。
- `aimux-ffi/src/lib.rs:754-765`：新增 Cohere reranking `_with_base`。
- `aimux-ffi/src/lib.rs:804-815`：新增 Google video `_with_base`。
- `aimux-ffi/src/lib.rs:854-873`：新增 Tavily search `_with_base`。任务文字称“7 个”但列出 8 类；本次按列举完整实现 8 个变体。

## 跳过项

- **Go 多模态方法直接改为 typed result**：跳过。现有公开方法被测试和用户代码按 `(string, error)` 使用，直接修改返回类型是破坏性 API 变更；安全方案应新增 `EmbedTyped`、`GenerateTyped`、`UploadTyped`、`RerankTyped`、`SearchTyped` 等并保留 raw 方法。该项标注“如果时间允许”，本次优先避免仓促扩大 API surface。
- **Flutter 全局 stream controller / isolate 阻塞问题**：审计报告还提出该问题，但委派任务的明确 Flutter 修复列表只要求 warnings、枚举和 options；per-stream NativeCallable/isolate-safe API 需要较大重构，本次未越界修改。
- **FFI transcription/files opts 合并**：审计报告的 I3/I4 建议不在委派任务明确修复清单中，未修改。

## 已执行验证

- `g++ -std=c++17 -fsyntax-only -I../../aimux-ffi example.cpp`：通过。
- 使用 C11 编译仅包含 `aimux-ffi.h` 的最小 translation unit：通过。
- 脚本对比 Rust `extern "C"` 导出与 header 声明：36 个导出、36 个声明，无缺失声明。
- 脚本核对 Flutter `GenerateTextOptions`：15/15 字段均存在。
- 未运行 Cargo（按任务约束）。环境未安装 `rustfmt`、`dart`、`swiftc`、`go`，相应格式化/语言测试无法执行。

## 后续验证建议

1. Rust/FFI：在无并行构建时运行 `cargo fmt --check`、`cargo check -p aimux-ffi`，并为每个新增 constructor 做 handle-create/drop smoke test。
2. Swift：运行 `swift test`；增加两个并发 stream 的 context 隔离测试，以及 raw/typed async stream error 抛出断言。
3. Kotlin：运行 Gradle 测试；用 fake JNA 层断言 double-close 只调用一次 drop，并增加 malformed StreamPart 触发 `onError` 测试。
4. Flutter：运行 `dart format --set-exit-if-changed lib/types.dart lib/types.g.dart`、`dart run build_runner build --delete-conflicting-outputs`、`flutter test`；增加 15 字段 options 和三枚举 round-trip fixture。
5. C/C++：链接实际 `libaimux_ffi` 做 callback forwarding smoke test；并建议将 C ABI 后续扩展为接收 `void *user_data`，消除 trampoline 对同步/同线程合同的依赖。
6. Go：如要完成 typed API，新增 typed 方法而不是破坏现有 raw 方法，并运行 `go test ./...`。
