# aimux vs Vercel AI SDK：Node.js 体验对比

> **日期**：2026-07-29
> **范围**：Node.js 绑定层用户体验，基于代码事实对比
> **对比基准**：aimux `bindings/node`（napi-rs + JSON 字符串边界）vs Vercel AI SDK V5（`ai` + `@ai-sdk/openai`，原生 TS + Zod）

---

## 1. 架构差异（根因）

| | AI SDK | aimux |
|---|--------|-------|
| 核心语言 | TypeScript（原生 Node） | Rust 核心 + napi-rs FFI 薄壳 |
| 数据边界 | 原生 JS 对象（零序列化） | JSON 字符串（每次调用 serialize/parse） |
| 类型系统 | Zod schema 贯穿，泛型推断 | `string` 进 `string` 出，类型丢失 |
| Tool 定义 | `tool({ parameters: z.object(...), execute })` | 手写 JSON Schema 对象，无 execute |
| 工具执行 | SDK 内建 `execute` + `stopWhen` 自动往返 | 手动往返（第二次调用 + ContentPart 格式） |

**根因**：aimux 的 Node 绑定是 Rust 核心的 JSON 字符串薄壳——Rust 侧用 `serde_json` 做 `from_str`（入）+ `to_string`（出），JS 侧拿到的是 `string` 必须 `JSON.parse`。这是跨语言一致的代价。

---

## 2. 逐维度对比

### 2.1 类型安全（最大差距）

**AI SDK**——Zod schema → 全链路类型推断：
```typescript
import { generateText, tool } from 'ai'
import { openai } from '@ai-sdk/openai'
import { z } from 'zod'

const result = await generateText({
  model: openai('gpt-4o'),
  prompt: "What's the weather in Tokyo?",
  tools: {
    get_weather: tool({
      description: 'Get weather',
      parameters: z.object({ location: z.string() }),  // Zod → 类型推断
      execute: async ({ location }) => fetchWeather(location),  // 类型安全
    }),
  },
})

result.toolCalls[0].args.location   // ✅ 类型安全，自动补全
result.toolResults[0].output        // ✅ 由 execute 返回类型推断
```

**aimux**——JSON 字符串边界，类型全丢：
```typescript
import { openai } from 'aimux'

const resultJson = await model.generateText(
  JSON.stringify("What's the weather in Tokyo?"),
  JSON.stringify({
    tools: [{
      type: 'function',
      name: 'get_weather',
      description: 'Get weather',
      input_schema: {                     // 手写 JSON Schema，无 Zod
        type: 'object',
        properties: { location: { type: 'string' } },
        required: ['location'],
      },
    }],
  })
)
const result = JSON.parse(resultJson)     // any 类型
result.tool_calls[0].input.location       // ⚠️ any，无补全，无校验
```

| | AI SDK | aimux |
|---|---|---|
| 输入参数 | Zod schema，编译时校验 | 手写 JSON Schema 字符串 |
| 返回类型 | 泛型推断（`TOOL` → `args`/`output`） | `any`（需手动 parse + 断言） |
| IDE 补全 | 全链路 | 无（除非手写 `as` 断言） |
| 运行时校验 | Zod 自动校验输入 | 无 |

### 2.2 调用边界（序列化开销）

**AI SDK**——原生对象，零序列化：
```typescript
// 输入：原生对象直接传
await generateText({ model, prompt: 'Hello', temperature: 0.7 })
// 输出：原生对象直接用
console.log(result.text)
```

**aimux**——每次调用 3 次 JSON 转换：
```typescript
// 输入：stringify prompt + stringify options
await model.generateText(
  JSON.stringify('Hello'),                    // ① JS → JSON string
  JSON.stringify({ temperature: 0.7 })        // ② JS → JSON string
)
// Rust 侧：serde_json::from_str(prompt)       // ③ JSON string → Rust struct
// Rust 侧：serde_json::to_string(&result)     // ④ Rust struct → JSON string
const result = JSON.parse(resultJson)          // ⑤ JSON string → JS object
```

一次 `generateText` 调用经过 **5 次序列化/反序列化**。流式更重：每个 `StreamPart` 都 `serde_json::to_string`（Rust 侧）+ `JSON.parse`（JS 侧）。

### 2.3 流式体验

**AI SDK**——双流 + 类型化 part：
```typescript
import { streamText } from 'ai'

const { textStream, fullStream } = streamText({ model, prompt: 'Write a poem' })

// 便捷：只要文本
for await (const delta of textStream) process.stdout.write(delta)

// 完整：类型化 part
for await (const part of fullStream) {
  switch (part.type) {
    case 'tool-call':        // 类型安全
      console.log(part.toolName, part.args)
    case 'reasoning':
      console.log(part.textDelta)
    case 'finish':
      console.log(part.usage)
  }
}
```

**aimux**——单流 + 手动 parse + 字符串匹配：
```typescript
for await (const json of await model.streamText(JSON.stringify('Write a poem'))) {
  const part = JSON.parse(json)               // ① 每次都要 parse
  if (part.TextDelta) console.log(part.TextDelta.delta)  // ② 字符串匹配变体名
  if (part.ToolCall) console.log(part.ToolCall.tool_name)  // ③ snake_case（非 camelCase）
  if (part.Finish) console.log(part.Finish.usage)
}
// 没有 textStream 快捷方式
```

| | AI SDK | aimux |
|---|---|---|
| 便捷文本流 | `textStream`（零拼装） | 无，手动拼 `TextDelta.delta` |
| Part 类型 | `part.type` + 类型推断 | 外部标签（`part.TextDelta`），`any` |
| 字段命名 | camelCase（`toolName`/`textDelta`） | snake_case（`tool_name`）—— 非惯用 JS |
| 解析开销 | 零 | 每个 part 一次 `JSON.parse` |

### 2.4 工具往返（agent loop）

**AI SDK**——`stopWhen` + `execute` 自动往返：
```typescript
const result = await generateText({
  model,
  prompt: "What's the weather in Tokyo?",
  tools: {
    get_weather: tool({
      parameters: z.object({ location: z.string() }),
      execute: async ({ location }) => {
        return { temperature: 22, condition: 'sunny' }  // SDK 自动回填
      },
    }),
  },
  stopWhen: stepCount(5),  // 自动循环：tool_call → execute → 回填 → 再调用
})
console.log(result.text)  // "The weather in Tokyo is sunny."
```

**aimux**——5 步手动往返：
```typescript
// ① 第一次调用
const r1 = JSON.parse(await model.generateText(
  JSON.stringify("What's the weather in Tokyo?"),
  JSON.stringify({ tools: [{ type: 'function', name: 'get_weather', input_schema: {...} }] })
))
// ② 提取工具调用
const call = r1.tool_calls[0]  // { tool_call_id, tool_name, input }
// ③ 手动执行
const weather = { temperature: 22, condition: 'sunny' }
// ④ 手动构造消息序列（必须用 ContentPart 格式，非 OpenAI wire 格式）
const messages = [
  { role: 'user', content: "What's the weather in Tokyo?" },
  { role: 'assistant', content: [{ type: 'tool_call', tool_call_id: call.tool_call_id, tool_name: 'get_weather', input: call.input }] },
  { role: 'tool', content: [{ type: 'tool_result', tool_call_id: call.tool_call_id, output: weather }] },
]
// ⑤ 第二次调用
const r2 = JSON.parse(await model.generateText(JSON.stringify(messages), JSON.stringify({ tools: [...] })))
console.log(r2.text)  // "The weather in Tokyo is sunny."
```

| | AI SDK | aimux |
|---|---|---|
| 往返自动化 | `stopWhen: stepCount(5)` | 无，手动 5 步 |
| 工具执行 | `execute` 函数内建 | 手动调用 + 手动回填 |
| 消息格式 | SDK 自动处理 | 必须用 ContentPart 格式（`{type:'tool_call',...}`） |
| 多轮循环 | 内建 | 手写 while 循环 |

### 2.5 多模态调用

**AI SDK**——统一顶层函数：
```typescript
import { embed, generateImage, generateSpeech } from 'ai'

const { embedding } = await embed({ model: openai.embedding('text-embedding-3-small'), value: 'hello' })
const { image } = await generateImage({ model: openai.image('dall-e-3'), prompt: 'a cat' })
const { audio } = await generateSpeech({ model: openai.speech('tts-1'), text: 'Hello', voice: 'alloy' })
```

**aimux**——每个模态不同类 + 不同签名：
```typescript
// embedding: embed(valuesJson: string) → string
const embedder = await openaiEmbedding('sk-...', 'text-embedding-3-small')
const { embeddings } = JSON.parse(await embedder.embed(JSON.stringify(['hello'])))

// image: generate(optsJson: string) → string
const imager = await openaiImage('sk-...', 'dall-e-3')
const { images } = JSON.parse(await imager.generate(JSON.stringify({ prompt: 'a cat', n: 1 })))

// speech: generate(optsJson: string) → string
const speaker = await openaiSpeech('sk-...', 'tts-1')
const { audio } = JSON.parse(await speaker.generate(JSON.stringify({ text: 'Hello', voice: 'alloy' })))
```

| | AI SDK | aimux |
|---|---|---|
| 调用一致性 | 统一 `embed()` / `generateImage()` / `generateSpeech()` | 每个类方法签名不同 |
| 参数 | 原生对象 | `JSON.stringify(optsObject)` |
| 返回 | 类型化对象 | `JSON.parse(string)` → `any` |

### 2.6 Provider 切换（体验接近）

**两者一致**——改一行 import 即可切换 provider：

```typescript
// AI SDK
import { openai } from '@ai-sdk/openai'     // → import { anthropic } from '@ai-sdk/anthropic'
const model = openai('gpt-4o')

// aimux
import { openai } from 'aimux'               // → import { anthropic } from 'aimux'
const model = await openai('sk-...', 'gpt-4o')
```

| | AI SDK | aimux |
|---|---|---|
| Provider 切换 | 改 import | 改 import | 🟢 一致 |
| API Key | 可从环境变量自动读 | 必须显式传 | 🟡 略不便 |
| 构造 | 同步 `openai('gpt-4o')` | 异步 `await openai(key, model)` | 🟡 多个 await |

---

## 3. 序列化性能分析

### 3.1 一次 generateText 调用的序列化链路

```
JS 调用                     napi FFI 边界              Rust 核心
─────────                   ────────────              ─────────
JSON.stringify(prompt)  ──→  serde_json::from_str  ──→  ModelPrompt
JSON.stringify(opts)    ──→  serde_json::from_str  ──→  CallOptions
                                                     │
                  generate_text (do_generate)       │
                                                     ↓
                  serde_json::to_string  ←──  GenerateTextResult
JSON.parse(resultJson)  ←──  String (JSON)
```

**5 次序列化操作**：2 次 `JSON.stringify`（JS）+ 2 次 `serde_json::from_str`（Rust）+ 1 次 `serde_json::to_string`（Rust）+ 1 次 `JSON.parse`（JS）= 实际 6 次。

### 3.2 性能量级估算

| 操作 | 典型耗时 | 说明 |
|------|---------|------|
| `JSON.stringify` 小对象（prompt + opts） | ~1-5 μs | JS V8 原生，极快 |
| `serde_json::from_str` prompt + opts | ~2-10 μs | Rust，与 V8 相当 |
| `serde_json::to_string` result（含 raw.content） | ~5-20 μs | result 可能较大（含完整 content 数组） |
| `JSON.parse` result | ~2-10 μs | V8 原生 |
| **总序列化开销** | **~10-45 μs** | |

**对比网络请求**：一次 OpenAI API 调用通常 **200-2000 ms**（取决于模型和响应长度）。

```
序列化开销:    ~0.01-0.05 ms
网络请求:      ~200-2000 ms
占比:         < 0.025%
```

### 3.3 流式的序列化开销

流式场景每个 `StreamPart` 都要 `to_string` + `JSON.parse`：

```
每个 part: serde_json::to_string (~1-5 μs) + JSON.parse (~0.5-2 μs) = ~1.5-7 μs
典型流: 100-500 个 parts
总开销: ~0.15-3.5 ms
```

对比流式传输时间（几秒到几十秒），**占比 < 0.1%**。

### 3.4 结论：序列化折损可忽略

**序列化开销在 LLM 场景下完全可忽略**——瓶颈永远在网络 I/O（200ms+），不在 JSON 转换（0.01-0.05ms）。序列化开销比网络快 **4000-40000 倍**。

唯一例外是**超高频小请求**（如 batch embedding 大量短文本），但那种场景 Rust 核心的 `do_embed` 一次性批量处理，序列化次数不随文本数量线性增长。

---

## 4. 如何保持跨语言一致 + 缩小体验差距

### 4.1 核心矛盾

```
Rust 核心一致性  ←→  各语言原生体验
     JSON 字符串边界         类型安全 + 惯用 API
```

aimux 选了 JSON 字符串边界——好处是 6 个绑定用同一套 Rust 核心，坏处是每个绑定的类型安全都是 `any`。

### 4.2 方案 A：TS Wrapper 层（推荐，零性能折损）

在 `bindings/node` 上加一层纯 TS wrapper，不改 Rust 侧：

```typescript
// bindings/node/src/index.ts（新 wrapper）
import { Model as RawModel } from '../index.js'
import type { GenerateTextOptions, GenerateTextResult, StreamPart } from './types.js'

export class Model {
  private raw: RawModel

  async generateText(
    prompt: string | ModelMessage[],
    options?: GenerateTextOptions,    // 原生对象，非 JSON 字符串
  ): Promise<GenerateTextResult> {    // 类型化返回，非 string
    const promptJson = JSON.stringify(prompt)
    const optsJson = options ? JSON.stringify(options) : undefined
    const resultJson = await this.raw.generateText(promptJson, optsJson)
    return JSON.parse(resultJson) as GenerateTextResult
  }

  async *streamText(
    prompt: string | ModelMessage[],
    options?: GenerateTextOptions,
  ): AsyncGenerator<StreamPart> {      // 类型化 part，非 string
    const gen = await this.raw.streamText(JSON.stringify(prompt), options ? JSON.stringify(options) : undefined)
    for await (const json of gen) {
      yield JSON.parse(json) as StreamPart
    }
  }
}
```

**类型定义文件**（从 Rust 的 ts-rs 导出生成）：
```typescript
// types.ts（由 aimux-core 的 ts(export) 自动生成）
export interface GenerateTextResult {
  text: string
  tool_calls: ToolCall[]
  finish_reason: FinishReason
  usage: Usage
  raw: GenerateResult
}
export interface StreamPart { TextDelta?: { delta: string }; ToolCall?: {...}; ... }
```

**优势**：
- ✅ 零性能折损（wrapper 只做 stringify/parse，跟用户手动做一样，Rust 侧不变）
- ✅ 类型安全（返回类型化对象）
- ✅ 渐进式（不影响现有 raw API，wrapper 是可选层）
- ✅ Python/Swift/Kotlin 可同样加 wrapper

**劣势**：
- ❌ 不解决 Zod schema 推断（tool 参数仍是 JSON Schema）
- ❌ 不解决 `execute` 自动往返（仍需手动）

### 4.3 方案 B：Zod + execute 层（体验追平 AI SDK，较重）

在 wrapper 层之上再加 Zod + tool execute：

```typescript
export async function generateText(opts: {
  model: Model
  prompt: string
  tools?: Record<string, Tool>
  stopWhen?: StopCondition
}): Promise<Result> { ... }

interface Tool {
  parameters: z.ZodType          // Zod schema
  execute?: (args) => Promise<unknown>
}
```

**优势**：
- ✅ 完全追平 AI SDK 体验（Zod 推断 + execute + stopWhen）

**劣势**：
- ❌ 引入 Zod 依赖
- ❌ `execute` + `stopWhen` 意味着 wrapper 要实现 agent loop（循环调用 Rust 的 generateText）——逻辑变重
- ❌ 每种语言都要重新实现这套（Rust 核心帮不上忙）

### 4.4 方案 C：Rust 侧加 `generate` 函数（跨语言一致）

在 Rust 核心加一个不压扁结构化 content 的 `generate` 函数 + 工具往返循环：

```rust
// aimux-core/src/generate.rs
pub async fn generate(
    model: &dyn LanguageModel,
    prompt: impl Into<ModelPrompt>,
    options: GenerateOptions,    // 含 tools + tool execute callback
) -> Result<GenerateResult, AiMuxError>  // 返回完整 content，不压扁
```

各绑定调用 `generate` 而非 `generate_text`，拿到完整 `GenerateResult.content`。

**优势**：
- ✅ 跨语言一致（Rust 核心实现往返逻辑，各绑定共享）
- ✅ 结构化 content 一等公民

**劣势**：
- ❌ tool execute 回调在跨 FFI 边界时复杂（Rust 回调 JS 函数）
- ❌ 改动较大

### 4.5 推荐路径

| 阶段 | 方案 | 投入 | 收益 |
|------|------|------|------|
| **阶段 1** | 方案 A（TS wrapper + 类型定义） | 小 | 类型安全 + 惯用 API，零性能折损 |
| **阶段 2** | 方案 B 的 Zod 部分 | 中 | tool 参数 Zod 校验 + 推断 |
| **阶段 3** | 方案 B 的 execute + stopWhen | 中大 | agent loop 内建（追平 AI SDK） |
| **可选** | 方案 C（Rust 侧 generate） | 大 | 跨语言一致的往返逻辑 |

**关键判断**：序列化性能**不是问题**（< 0.025% 占比），不需要为此改 Rust 侧。体验差距在**类型安全和 API 习惯**层面，用纯 TS wrapper 就能解决大部分——不付性能代价，只付 wrapper 维护代价。

---

## 5. 总结

| 维度 | AI SDK | aimux | 差距 | 可修复性 |
|------|--------|-------|------|---------|
| 类型安全 | Zod 全推断 | `any` | 🔴 严重 | TS wrapper 解决（阶段 1） |
| 序列化开销 | 零 | 6 次/调用 | 🟢 可忽略（< 0.025%） | 不需修 |
| 流式体验 | `textStream` + 类型化 part | 手动 parse + 字符串匹配 | 🟡 中 | TS wrapper 解决 |
| 工具往返 | `stopWhen` + `execute` 自动 | 手动 5 步 | 🔴 严重 | 需阶段 2-3 |
| 多模态 | 统一函数 | 不同类 + 不同签名 | 🟡 中 | TS wrapper 可统一 |
| Provider 切换 | 改 import | 改 import | 🟢 无 | — |
| 字段命名 | camelCase | snake_case | 🟡 小 | wrapper 做 camelCase 映射 |

**核心结论**：aimux 的体验差距不在性能（序列化开销可忽略），而在**类型安全和 API 习惯**。推荐用 TS wrapper 层（方案 A）解决——零性能折损，渐进式，不影响 6 语言一致性。

---

## 修订记录

| 日期 | 版本 | 说明 |
|------|------|------|
| 2026-07-29 | v0.1 | 初稿，基于 Node 绑定源码 + AI SDK V5 源码对比 |
