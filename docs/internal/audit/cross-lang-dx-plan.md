# 跨语言开发体验统一方案

> **日期**：2026-07-29
> **范围**：6 绑定层（Node/Python/Swift/Kotlin/Flutter/Rust）开发体验统一
> **结论**：第 1-2 层（类型化边界 + 惯用 API）各语言都可做到，不做 agent loop

---

## 1. 目标层次

| 层次 | 内容 | aimux 是否做 |
|------|------|:---:|
| **第 1 层：类型化边界** | 输入输出从 `string` 变成类型化对象（TS interface / Pydantic model / Swift Codable struct / Kotlin data class / Dart class） | ✅ 做 |
| **第 2 层：惯用 API** | 消除 `JSON.stringify`/`JSON.parse`，wrapper 自动处理，camelCase 命名 | ✅ 做 |
| **第 3 层：工具往返内建** | `execute` + `stopWhen` agent loop | ❌ 不做（交给上层框架） |

**定位**：aimux 是 **provider 统一接入层**，不是 agent 框架。提供类型化 + 惯用的 API，agent loop 由上层（LangChain / Mastra / Vercel AI SDK 等）实现。

---

## 2. 现状 vs 目标

### 现状（JSON 字符串边界）

```typescript
// Node — 现状
const resultJson = await model.generateText(
  JSON.stringify("Hello"),              // string in
  JSON.stringify({ temperature: 0.7 }),  // string in
)
const result = JSON.parse(resultJson)    // any out
```

### 目标（类型化 wrapper）

```typescript
// Node — 目标
const result = await model.generateText("Hello", { temperature: 0.7 })
//    ^? GenerateTextResult (类型化，IDE 补全)
```

---

## 3. 各语言实现方案

### 3.1 Node.js（最简单，已有 ts-rs 类型）

**现状优势**：Rust 的 `ts-rs` 已生成 79 个 `.ts` 类型文件（`aimux-core/bindings/*.ts`），包含完整的 `GenerateTextResult` / `StreamPart` / `Tool` / `ToolChoice` / `GenerateTextOptions` 等。

**实现**：在 `bindings/node/src/` 加一层纯 TS wrapper，复用 ts-rs 类型：

```typescript
// bindings/node/src/index.ts
import { Model as RawModel } from '../index.js'
import type {
  GenerateTextResult, GenerateTextOptions, StreamPart, ModelMessage
} from '../../aimux-core/bindings/'

export class Model {
  private raw: RawModel

  generateText(
    prompt: string | ModelMessage[],
    options?: GenerateTextOptions,
  ): Promise<GenerateTextResult> {
    return this.raw.generateText(JSON.stringify(prompt), options ? JSON.stringify(options) : undefined)
      .then(json => JSON.parse(json) as GenerateTextResult)
  }

  async *streamText(
    prompt: string | ModelMessage[],
    options?: GenerateTextOptions,
  ): AsyncGenerator<StreamPart> {
    const gen = await this.raw.streamText(JSON.stringify(prompt), options ? JSON.stringify(options) : undefined)
    for await (const json of gen) {
      yield JSON.parse(json) as StreamPart
    }
  }
}
```

**工作量**：~1 天（类型已有，只需写 wrapper + 多模态方法）

**类型来源**：`aimux-core/bindings/*.ts`（ts-rs 自动生成，Rust 类型变更后 `cargo test` 自动更新）

### 3.2 Python（Pydantic model）

**现状**：PyO3 绑定用 `#[pyclass]` 但方法是 `string` 进出。

**实现**：加一层 Python wrapper + Pydantic model（手写，或用 `datamodel-code-generator` 从 JSON Schema 生成）：

```python
# bindings/python/python/aimux/wrapper.py
from pydantic import BaseModel
from . import openai as _openai, generate_text as _generate_text

class GenerateTextOptions(BaseModel):
    max_output_tokens: int | None = None
    temperature: float | None = None
    tools: list[Tool] | None = None
    tool_choice: ToolChoice | None = None
    # ...

class GenerateTextResult(BaseModel):
    text: str
    tool_calls: list[ToolCall]
    finish_reason: FinishReason
    usage: Usage
    raw: GenerateResult

class Model:
    def __init__(self, raw):
        self._raw = raw

    def generate_text(self, prompt: str | list[ModelMessage], options: GenerateTextOptions | None = None) -> GenerateTextResult:
        prompt_json = json.dumps(prompt)
        opts_json = options.model_dump_json() if options else None
        result_json = self._raw.generate_text(prompt_json, opts_json)
        return GenerateTextResult.model_validate_json(result_json)
```

**工作量**：~1.5 天（Pydantic model 手写，或用 codegen 工具从 ts-rs 的 JSON Schema 生成）

**类型来源**：手写 Pydantic model（Rust 变更后需手动同步）；或 `datamodel-code-generator` 从 JSON Schema 自动生成

### 3.3 Swift（Codable struct）

**现状**：C ABI 返回 JSON 字符串，无类型化结构体。

**实现**：加 Swift `Codable` struct + wrapper：

```swift
// bindings/swift/Sources/Aimux/Types.swift
public struct GenerateTextResult: Codable {
    public let text: String
    public let tool_calls: [ToolCall]
    public let finish_reason: FinishReason
    public let usage: Usage
    public let raw: GenerateResult
}

public struct StreamPart: Codable {
    // ts-rs 外部标签格式 → Codable enum
    public let TextDelta: TextDelta?
    public let ToolCall: ToolCallPart?
    // ...
}

// wrapper
extension Model {
    public func generateText(prompt: String, options: GenerateTextOptions? = nil) throws -> GenerateTextResult {
        let json = try generateText(prompt: prompt, options: options?.jsonString)
        return try JSONDecoder().decode(GenerateTextResult.self, from: json.data(using: .utf8)!)
    }
}
```

**工作量**：~2 天（Swift Codable struct 手写；外部标签 enum 的 Codable 解码需要自定义 init）

**类型来源**：手写 Codable struct（Rust 变更后需手动同步）

### 3.4 Kotlin（data class + kotlinx.serialization）

**现状**：JNA 返回 JSON 字符串，无类型化。

**实现**：加 data class + kotlinx.serialization + wrapper：

```kotlin
// bindings/kotlin/src/main/kotlin/aimux/Types.kt
@Serializable
data class GenerateTextResult(
    val text: String,
    val tool_calls: List<ToolCall>,
    val finish_reason: FinishReason,
    val usage: Usage,
    val raw: GenerateResult,
)

@Serializable
data class GenerateTextOptions(
    val max_output_tokens: Int? = null,
    val temperature: Double? = null,
    val tools: List<Tool>? = null,
    val tool_choice: ToolChoice? = null,
)

// wrapper
class TypedModel(private val raw: Model) {
    fun generateText(prompt: String, options: GenerateTextOptions? = null): GenerateTextResult {
        val promptJson = Json.encodeToString(prompt)
        val optsJson = options?.let { Json.encodeToString(it) }
        val resultJson = raw.generateText(promptJson, optsJson)
        return Json.decodeFromString(resultJson)
    }
}
```

**工作量**：~1.5 天（data class + kotlinx.serialization + wrapper）

**类型来源**：手写 data class（Rust 变更后需手动同步）；或用 `kotlinx.serialization` codegen

### 3.5 Flutter/Dart（class + json_serializable）

**现状**：dart:ffi 返回 JSON 字符串，已用 `Map<String, dynamic>`。

**实现**：加类型化 class + json_serializable + wrapper：

```dart
// bindings/flutter/lib/types.dart
@JsonSerializable()
class GenerateTextResult {
  final String text;
  final List<ToolCall> toolCalls;  // camelCase
  final FinishReason finishReason;
  final Usage usage;
  final GenerateResult raw;

  factory GenerateTextResult.fromJson(Map<String, dynamic> json) => _$GenerateTextResultFromJson(json);
}

// wrapper
class TypedModel {
  final Model _raw;
  GenerateTextResult generateText(String prompt, [GenerateTextOptions? options]) {
    final json = _raw.generateText(prompt, options?.toJson());
    return GenerateTextResult.fromJson(json);
  }
}
```

**工作量**：~1.5 天（class + json_serializable codegen + wrapper）

**类型来源**：`json_serializable` 从 class 生成；class 手写（Rust 变更后需手动同步）

---

## 4. 各语言难度对比

| 语言 | 类型系统 | Schema 工具 | Rust→类型 codegen | 第 1-2 层工作量 | 字段命名转换 |
|------|---------|-----------|:---:|:---:|:---:|
| **Node** | TS 强类型 + 泛型 | Zod（可选） | ✅ ts-rs 已生成 79 个 .ts | ~1 天 | 无（ts-rs 用 snake_case，wrapper 可映射） |
| **Python** | 类型注解 + Pydantic | Pydantic | ❌ 需手写或 codegen | ~1.5 天 | 无（Pydantic 可配 alias） |
| **Dart** | 强类型 + null safety | json_serializable | ❌ 需手写 | ~1.5 天 | 需映射（`@JsonKey(name:)`） |
| **Kotlin** | data class + null safety | kotlinx.serialization | ❌ 需手写 | ~1.5 天 | 需映射（`@SerialName`） |
| **Swift** | Codable + 强类型 | 无 Zod 等价 | ❌ 需手写 | ~2 天 | 需映射（`CodingKeys` enum） |

**结论**：各语言都能做到，Node 最简单（类型已生成），Swift 最复杂（Codable enum + 外部标签解码）。

---

## 5. 序列化性能（不变）

wrapper 只是**把用户手动做的 stringify/parse 移到 wrapper 内部**，序列化次数不变：

| 步骤 | 现状（用户手动） | 目标（wrapper 自动） | 次数变化 |
|------|:---:|:---:|:---:|
| JS → JSON string | 用户 `JSON.stringify` | wrapper `JSON.stringify` | 0 |
| Rust parse | `serde_json::from_str` | `serde_json::from_str` | 0 |
| Rust serialize | `serde_json::to_string` | `serde_json::to_string` | 0 |
| JSON → JS | 用户 `JSON.parse` | wrapper `JSON.parse` | 0 |

**零性能折损**。序列化开销仍是 < 0.025%（对比网络请求 200ms+）。

---

## 6. 类型同步策略

Rust 核心类型变更后，各语言类型如何同步：

| 语言 | 同步方式 | 自动化程度 |
|------|---------|:---:|
| **Node** | `cargo test -p aimux-core` 自动重新生成 .ts 文件 | ✅ 全自动 |
| **Python** | 手动同步 Pydantic model，或 `datamodel-code-generator` 从 JSON Schema 生成 | 🟡 半自动 |
| **Dart** | 手写 class + `dart run build_runner` 生成 `.g.dart` | 🟡 半自动 |
| **Kotlin** | 手写 data class | ❌ 手动 |
| **Swift** | 手写 Codable struct | ❌ 手动 |

**Node 是唯一全自动同步的**（ts-rs）。其他语言有 codegen 选项但需额外配置。

---

## 7. 实现优先级

| 优先级 | 语言 | 理由 |
|:---:|------|------|
| P0 | Node | ts-rs 类型已生成，工作量最小，验证 wrapper 模式 |
| P1 | Python | Pydantic 生态成熟，用户量大 |
| P2 | Dart | Flutter 用户需要类型安全 |
| P3 | Kotlin | JNA + kotlinx.serialization |
| P4 | Swift | Codable enum 最复杂，可最后 |

---

## 8. 不做 agent loop 的理由

| 理由 | 说明 |
|------|------|
| **定位** | aimux 是 provider 统一接入层，不是 agent 框架 |
| **复杂度** | `execute` 回调跨 FFI 边界，每种语言机制不同，6×重复实现 |
| **生态** | LangChain / Mastra / Vercel AI SDK 等上层框架已有 agent loop |
| **一致性** | 不做 agent loop = 各语言 wrapper 复杂度一致（都是 1-2 天） |

---

## 修订记录

| 日期 | 版本 | 说明 |
|------|------|------|
| 2026-07-29 | v0.1 | 初稿，基于各绑定源码 + ts-rs 导出现状分析 |
