# 类型化 Wrapper 层审计报告

> **日期**：2026-07-29
> **审计员**：独立 agent（fork 全上下文）
> **范围**：5 个绑定层（Node/Python/Swift/Kotlin/Dart）新增的类型化 wrapper 源码 + 测试 + Rust 核心类型变更
> **方法**：逐文件读取源码，逐类型对比字段，验证序列化路径、测试链路、向后兼容

---

## 1. 审计范围

| 层 | wrapper 源码 | 测试文件 | 测试数 |
|----|---------|---------|:---:|
| Node | `bindings/node/src/index.ts` | `bindings/node/__test__/wrapper.test.ts` | 7 |
| Python | `bindings/python/python/aimux/wrapper.py` | `bindings/python/tests/test_wrapper.py` | 11 |
| Swift | `bindings/swift/Sources/Aimux/Types.swift` | `bindings/swift/Tests/AimuxTests/WrapperTests.swift` | 9 |
| Kotlin | `bindings/kotlin/src/main/kotlin/aimux/Types.kt` + `TypedModel.kt` | `bindings/kotlin/src/test/kotlin/aimux/TypedModelTest.kt` | 4 |
| Dart | `bindings/flutter/lib/types.dart` + `typed_model.dart` | `bindings/flutter/test/typed_model_test.dart` | 5 |

Rust 核心类型变更：`ToolCall`/`ToolResult`/`StreamPart`/`GenerateContent` 新增 `provider_executed`/`dynamic`/`is_error`/`preliminary`/`provider_metadata` 字段；`Usage` 新增 `raw` 字段。ts-rs 79 个 .ts 类型已重新生成。

---

## 2. 逐层审计结果

### 2.1 Node

| 检查项 | 结论 | 证据 |
|--------|:----:|------|
| 类型完整性 | ✅ | 直接 import `aimux-core/bindings/*.ts`（ts-rs 自动生成），零手写类型 → 与 Rust 核心完全同步 |
| 序列化正确性 | ✅ | `JSON.stringify(prompt)` → raw `model.generateText` → `JSON.parse(resultJson) as GenerateTextResult`（[src/index.ts:99-101](bindings/node/src/index.ts#L99)） |
| 测试真实性 | ✅ | 7 个测试全用 `node:http` mock server，验证完整链路；`capturingHandler` 捕获请求体验证到达 provider |
| 向后兼容 | ✅ | raw napi API（`index.js`/`index.d.ts`）零改动；root `index.ts` 改为 `export * from './src/index.ts'` |
| 类型安全 | ✅ | 返回 `GenerateTextResult`（ts-rs 类型），`result.tool_calls[0].tool_name` 有 IDE 补全 |

**亮点**：Node 是唯一使用 ts-rs 自动生成类型的绑定——Rust 类型变更后 `cargo test -p aimux-core` 自动同步，零手工维护。

### 2.2 Python

| 检查项 | 结论 | 证据 |
|--------|:----:|------|
| 类型完整性 | ✅ | 全部 Rust 类型镜像为 Pydantic model（795 行），含 `Usage.raw`（[:106](bindings/python/python/aimux/wrapper.py#L106)）、`ToolCall.provider_executed`/`dynamic`（[:130-131](bindings/python/python/aimux/wrapper.py#L130)）、`GenerateContent::ToolResult` 全字段（[:247-255](bindings/python/python/aimux/wrapper.py#L247)） |
| 序列化正确性 | ✅ | `_prompt_to_json` 处理 string/messages；`_opts_to_json` 用 `exclude_none=True, by_alias=True`；外部标签 enum 用 before-validator + wrap-serializer round-trip（[:144-162](bindings/python/python/aimux/wrapper.py#L144)） |
| 测试真实性 | ✅ | 11 个测试用 `MockServer`/`RecordingMockServer`/`SequencedMockServer`，验证请求体到达 + 响应解析 |
| 向后兼容 | ✅ | raw PyO3 API 保留；wrapper 是独立模块 `aimux.wrapper` |
| 类型安全 | ✅ | 返回 `GenerateTextResult`（Pydantic model），`.text`/`.tool_calls[0].tool_name` 有补全 |

**亮点**：外部标签 enum（`StreamPart`/`GenerateContent`/`Warning`）用 `RootModel` + discriminated union + before-validator + wrap-serializer 精确 round-trip；`ToolChoice.toolName` 的 camelCase 用自定义 serializer 处理。

### 2.3 Swift

| 检查项 | 结论 | 证据 |
|--------|:----:|------|
| 类型完整性 | ✅ | 1125 行，全部 Rust 类型镜像为 `Codable` struct/enum；`Usage.raw`（[:171](bindings/swift/Sources/Aimux/Types.swift#L171)）、`ToolCall.providerExecuted`/`dynamic`（[:628](bindings/swift/Sources/Aimux/Types.swift#L628)）、`StreamPart` 17 个变体全含新字段 |
| 序列化正确性 | ✅ | `JSONEncoder`/`JSONDecoder` + `CodingKeys` 映射 snake_case → camelCase；外部标签 enum 手写 `init(from:)`/`encode(to:)`（[:886-1028](bindings/swift/Sources/Aimux/Types.swift#L886)） |
| 测试真实性 | ✅ | 9 个测试用 `MockHTTPServer`（POSIX socket），验证完整链路 + 请求体 |
| 向后兼容 | ✅ | raw API（`Aimux.swift`）零改动；typed 方法在 `Model` extension 里（[:1030](bindings/swift/Sources/Aimux/Types.swift#L1030)） |
| 类型安全 | ✅ | 返回 `GenerateTextResult`（Codable struct），`.text`/`.toolCalls[0].toolName` 有补全 |

**亮点**：`StreamPart` 外部标签 enum 用手写 `init(from:)` 解码 `{"TextDelta": {id, delta}}` 格式，包含全部 17 个变体 + `Unknown` 兜底；`JSONValue` 类型处理任意 JSON。

### 2.4 Kotlin

| 检查项 | 结论 | 证据 |
|--------|:----:|------|
| 类型完整性 | ✅ | 627 行 data class + sealed class；`Usage.raw`（[Types.kt:114](bindings/kotlin/src/main/kotlin/aimux/Types.kt#L114)）、`ToolCall.providerExecuted`/`dynamic`（[:161-162](bindings/kotlin/src/main/kotlin/aimux/Types.kt#L161)）、`StreamPart` 含全部变体 + 新字段（[:457-527](bindings/kotlin/src/main/kotlin/aimux/Types.kt#L457)） |
| 序列化正确性 | ✅ | `AimuxJson`（`ignoreUnknownKeys=true, explicitNulls=false, encodeDefaults=false`）；`StreamPart` 手写 `KSerializer` 处理外部标签（[:530](bindings/kotlin/src/main/kotlin/aimux/Types.kt#L530) `Unknown` 兜底） |
| 测试真实性 | ✅ | 4 个测试用 `MockProviderServer`，验证 `.text`/`.toolCalls`/`.raw.content` + 请求体 |
| 向后兼容 | ✅ | raw `Model`（JNA）零改动；`MockProviderServer` 从 `private` 改 `internal`（仅可见性） |
| 类型安全 | ✅ | 返回 `GenerateTextResult`（data class），`.text`/`.toolCalls[0].toolName` 有补全 |

**亮点**：`TypedModel` 有 `companion` 工厂（`openai`/`anthropic`）+ `of(model)` 包装现有 model；`decodeResult` 主动检测 `{"error":"..."}` 并抛 `AimuxException`；`streamTextSequence` 用独立线程避 JNA callback 死锁。

### 2.5 Dart

| 检查项 | 结论 | 证据 |
|--------|:----:|------|
| 类型完整性 | ⚠️ | `Usage` 缺 `raw` 字段（[types.dart:56-66](bindings/flutter/lib/types.dart#L56) 只有 `inputTokens`/`outputTokens`）；`Tool`/`ToolChoice` 未建独立 class，用 `List<Map>`/`Object?` 透传（[types.dart:168](bindings/flutter/lib/types.dart#L168)） |
| 序列化正确性 | ✅ | `@JsonSerializable()` + `@JsonKey(name: 'snake_case')`；`GenerateTextResult.fromJson`/`toJson` 正确 |
| 测试真实性 | ✅ | 5 个测试用 `MockOpenAIServer` + `Isolate.run` 避死锁，验证 `.text`/`.toolCalls[0].toolName`/`.finishReason.unified`/`.usage.inputTokens.total` |
| 向后兼容 | ✅ | raw `Model`（dart:ffi）零改动 |
| 类型安全 | ✅ | 返回 `GenerateTextResult`（typed class），但 `raw` 是 `Map<String, dynamic>` 非 `GenerateResult` |

---

## 3. 发现的问题

### 严重问题：无

### 中等问题

| # | 问题 | 位置 | 说明 |
|---|------|------|------|
| M1 | ~~**Dart `Usage` 缺 `raw` 字段**~~ ✅ 已修 | [types.dart:56-66](bindings/flutter/lib/types.dart#L56) | 2026-07-29 已补 `raw` 字段。 |
| M2 | ~~**Dart `Tool`/`ToolChoice` 未类型化**~~ ✅ 已修 | [types.dart](bindings/flutter/lib/types.dart) | 2026-07-29 已建 `Tool`/`FunctionTool`/`ToolChoice` 类型化 class。 |

### 轻微问题

| # | 问题 | 位置 | 说明 |
|---|------|------|------|
| L1 | ~~**Dart `GenerateTextResult.raw` 是 `Map` 而非 `GenerateResult` 类型**~~ ✅ 已修 | [types.dart](bindings/flutter/lib/types.dart) | 2026-07-29 已建 `GenerateResult`/`GenerateContent`/`ResponseMetadata` 类型。 |
| L2 | **Kotlin `ToolResult` struct 缺 `tool_name` 字段** | [tool.rs:120-131](aimux-core/src/tool.rs#L120) vs [Types.kt](bindings/kotlin/src/main/kotlin/aimux/Types.kt) | Rust `tool::ToolResult` 结构体本身不含 `tool_name`（只在 `GenerateContent::ToolResult` 变体里有）。这不是 wrapper 的 bug——是 Rust 核心结构体的设计。 |
| L3 | **Node wrapper `seed` 字段是 `bigint`** | [GenerateTextOptions.ts](aimux-core/bindings/GenerateTextOptions.ts) | `JSON.stringify` 对 `bigint` 会抛异常。如果用户传 `seed` 会 crash。这是 ts-rs 类型映射的固有限制（Rust `u64` → TS `bigint`）。 |
| L4 | **Python `AiMuxError` 用 `Any`** | [wrapper.py:83](bindings/python/python/aimux/wrapper.py#L83) | `AiMuxError` 是 Rust 混合 newtype/struct 外部标签 enum，Python 侧用 `Any` 而非 discriminated union。`StreamPart.Error.error` 因此是 `Any`。 |

---

## 4. 通过项清单

### 类型完整性
- ✅ Node：ts-rs 自动生成，与 Rust 核心 100% 同步（含 `provider_executed`/`dynamic`/`is_error`/`preliminary`/`raw` 新字段）
- ✅ Python：795 行 Pydantic model 完整镜像全部 Rust 类型，含新字段
- ✅ Swift：1125 行 Codable struct/enum 完整镜像，`StreamPart` 17 变体全含新字段
- ✅ Kotlin：627 行 data class + sealed class 完整镜像，`StreamPart` 含 `Unknown` 兜底
- ⚠️ Dart：缺 `Usage.raw`；`Tool`/`ToolChoice` 未类型化

### 序列化正确性
- ✅ 所有 5 语言：wrapper 输入 `stringify`/`dumps`/`encode` → Rust `serde_json::from_str` → 输出 `to_string` → wrapper `parse`/`decode`/`fromJson`，路径完整
- ✅ 外部标签 enum（`StreamPart`/`GenerateContent`/`Warning`）：Node 用 ts-rs 原生外部标签；Python 用 before-validator + wrap-serializer；Swift 手写 `init(from:)`；Kotlin 手写 `KSerializer`
- ✅ `ToolChoice` 的 `toolName` camelCase：Python 自定义 serializer；Swift 手写 encode；Kotlin 自定义 serializer

### 测试真实性（无假绿）
- ✅ 所有 36 个 wrapper 测试通过真实 mock HTTP server 执行完整链路
- ✅ 多角色/ToolChoice 测试验证请求体到达 provider
- ✅ 工具调用测试验证 `.tool_calls` + `.raw.content` 双路径

### 向后兼容
- ✅ 全部 5 语言：raw API（JSON 字符串边界）保留不变
- ✅ 现有测试未回归（Node 23/23, Python 37/37, Swift 23/23, Kotlin 9/9, Dart 15/15）

### 类型安全
- ✅ 所有 5 语言：wrapper 返回值是类型化对象（TS interface / Pydantic model / Codable struct / data class / Dart class）
- ✅ 关键字段（`text`/`tool_calls`/`raw.content`/`StreamPart` 变体）在 4/5 语言中类型化（Dart 的 `raw` 是 `Map`）

---

## 5. 跨语言一致性评价

| 维度 | Node | Python | Swift | Kotlin | Dart |
|------|:----:|:------:|:-----:|:------:|:----:|
| 类型来源 | ts-rs 自动 | 手写 Pydantic | 手写 Codable | 手写 data class | 手写 + json_serializable |
| 类型同步 | ✅ 全自动 | ❌ 手动 | ❌ 手动 | ❌ 手动 | ❌ 手动 |
| `Usage.raw` | ✅ | ✅ | ✅ | ✅ | ❌ 缺 |
| `Tool` 类型化 | ✅ | ✅ | ✅ | ✅ | ❌ `Map` |
| `ToolChoice` 类型化 | ✅ | ✅ | ✅ | ✅ | ❌ `Object?` |
| `raw` 类型化 | ✅ `GenerateResult` | ✅ `GenerateResult` | ✅ `GenerateResult` | ✅ `GenerateResult` | ❌ `Map` |
| `StreamPart` 类型化 | ✅ 17 变体 | ✅ 17 变体 | ✅ 17 变体 | ✅ 17 变体 + `Unknown` | ⚠️ raw Map + 访问器 |
| 测试数 | 7 | 11 | 9 | 4 | 5 |

**评价**：Node/Python/Swift/Kotlin 四者体验高度一致——类型化对象进出、类型化 `raw`、类型化 `StreamPart`。Dart 是唯一的短板：`Usage.raw` 缺失、`Tool`/`ToolChoice`/`raw` 未类型化。建议优先补 Dart 的这三个缺口。

---

## 6. 总结

**整体评价：通过。** 5 个绑定层都成功消除了 JSON 字符串边界，36 个 wrapper 测试全部为真实链路测试，raw API 全部保留不变。类型完整性方面，Node/Python/Swift/Kotlin 四者与 Rust 核心高度一致；Dart 有 2 个中等缺口（`Usage.raw` 缺失、`Tool`/`ToolChoice` 未类型化）。

**核心价值**：用户从 `JSON.stringify`/`JSON.parse` + `any` 的体验，提升到了类型化对象 + IDE 补全——零序列化性能折损（序列化操作不变，只是从用户代码移到 wrapper 内部）。

**建议**：
1. 补 Dart `Usage.raw` 字段（M1）
2. 补 Dart `Tool`/`ToolChoice` 类型化 class（M2）
3. 补 Dart `GenerateTextResult.raw` 为 `GenerateResult` 类型（L1）
4. 考虑 Node `seed: bigint` 的 `JSON.stringify` 问题（L3）——可能需要 wrapper 做 `Number(seed)` 转换

---

## 修订记录

| 日期 | 版本 | 说明 |
|------|------|------|
| 2026-07-29 | v0.1 | 初稿，独立 agent 逐文件审计 5 绑定层 wrapper 源码 + 测试 |
| 2026-07-29 | v0.2 | M1/M2/L1 已修复：Dart 补 `Usage.raw` + `Tool`/`ToolChoice`/`GenerateResult`/`GenerateContent` 类型化 class。5 语言 wrapper 现已全部类型化一致。 |
