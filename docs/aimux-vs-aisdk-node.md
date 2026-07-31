# aimux vs Vercel AI SDK: Node.js Experience Comparison

> **Date**: 2026-07-29
> **Scope**: User experience of the Node.js binding layer, compared based on code facts
> **Comparison baseline**: aimux `bindings/node` (napi-rs + JSON string boundary) vs Vercel AI SDK V5 (`ai` + `@ai-sdk/openai`, native TS + Zod)

---

## 1. Architecture Differences (Root Cause)

| | AI SDK | aimux |
|---|--------|-------|
| Core language | TypeScript (native Node) | Rust core + napi-rs FFI thin wrapper |
| Data boundary | Native JS objects (zero serialization) | JSON strings (serialize/parse on every call) |
| Type system | Zod schema throughout, generic inference | `string` in, `string` out, types lost |
| Tool definition | `tool({ parameters: z.object(...), execute })` | Hand-written JSON Schema object, no execute |
| Tool execution | SDK built-in `execute` + `stopWhen` automatic round-trip | Manual round-trip (second call + ContentPart format) |

**Root cause**: aimux's Node binding is a JSON string thin wrapper over the Rust core — the Rust side uses `serde_json` to do `from_str` (in) + `to_string` (out), and the JS side receives a `string` that must be `JSON.parse`d. This is the cost of cross-language consistency.

---

## 2. Dimension-by-Dimension Comparison

### 2.1 Type Safety (Largest Gap)

**AI SDK** — Zod schema → end-to-end type inference:
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
      parameters: z.object({ location: z.string() }),  // Zod → type inference
      execute: async ({ location }) => fetchWeather(location),  // type safe
    }),
  },
})

result.toolCalls[0].args.location   // ✅ type safe, autocomplete
result.toolResults[0].output        // ✅ inferred from execute return type
```

**aimux** — JSON string boundary, types entirely lost:
```typescript
import { openai } from 'aimux'

const resultJson = await model.generateText(
  JSON.stringify("What's the weather in Tokyo?"),
  JSON.stringify({
    tools: [{
      type: 'function',
      name: 'get_weather',
      description: 'Get weather',
      input_schema: {                     // hand-written JSON Schema, no Zod
        type: 'object',
        properties: { location: { type: 'string' } },
        required: ['location'],
      },
    }],
  })
)
const result = JSON.parse(resultJson)     // any type
result.tool_calls[0].input.location       // ⚠️ any, no autocomplete, no validation
```

| | AI SDK | aimux |
|---|---|---|
| Input parameters | Zod schema, compile-time validation | Hand-written JSON Schema string |
| Return type | Generic inference (`TOOL` → `args`/`output`) | `any` (requires manual parse + assertion) |
| IDE autocomplete | End-to-end | None (unless hand-written `as` assertion) |
| Runtime validation | Zod auto-validates input | None |

### 2.2 Call Boundary (Serialization Overhead)

**AI SDK** — native objects, zero serialization:
```typescript
// Input: pass native objects directly
await generateText({ model, prompt: 'Hello', temperature: 0.7 })
// Output: use native objects directly
console.log(result.text)
```

**aimux** — 3 JSON conversions per call:
```typescript
// Input: stringify prompt + stringify options
await model.generateText(
  JSON.stringify('Hello'),                    // ① JS → JSON string
  JSON.stringify({ temperature: 0.7 })        // ② JS → JSON string
)
// Rust side: serde_json::from_str(prompt)       // ③ JSON string → Rust struct
// Rust side: serde_json::to_string(&result)     // ④ Rust struct → JSON string
const result = JSON.parse(resultJson)          // ⑤ JSON string → JS object
```

A single `generateText` call goes through **5 serialization/deserialization** operations. Streaming is heavier: every `StreamPart` does `serde_json::to_string` (Rust side) + `JSON.parse` (JS side).

### 2.3 Streaming Experience

**AI SDK** — dual stream + typed parts:
```typescript
import { streamText } from 'ai'

const { textStream, fullStream } = streamText({ model, prompt: 'Write a poem' })

// Convenience: text only
for await (const delta of textStream) process.stdout.write(delta)

// Full: typed parts
for await (const part of fullStream) {
  switch (part.type) {
    case 'tool-call':        // type safe
      console.log(part.toolName, part.args)
    case 'reasoning':
      console.log(part.textDelta)
    case 'finish':
      console.log(part.usage)
  }
}
```

**aimux** — single stream + manual parse + string matching:
```typescript
for await (const json of await model.streamText(JSON.stringify('Write a poem'))) {
  const part = JSON.parse(json)               // ① parse every time
  if (part.TextDelta) console.log(part.TextDelta.delta)  // ② string match variant name
  if (part.ToolCall) console.log(part.ToolCall.tool_name)  // ③ snake_case (not camelCase)
  if (part.Finish) console.log(part.Finish.usage)
}
// No textStream shortcut
```

| | AI SDK | aimux |
|---|---|---|
| Convenient text stream | `textStream` (zero assembly) | None, manually concatenate `TextDelta.delta` |
| Part type | `part.type` + type inference | External tag (`part.TextDelta`), `any` |
| Field naming | camelCase (`toolName`/`textDelta`) | snake_case (`tool_name`) — non-idiomatic JS |
| Parsing overhead | Zero | One `JSON.parse` per part |

### 2.4 Tool Round-Trip (agent loop)

**AI SDK** — `stopWhen` + `execute` automatic round-trip:
```typescript
const result = await generateText({
  model,
  prompt: "What's the weather in Tokyo?",
  tools: {
    get_weather: tool({
      parameters: z.object({ location: z.string() }),
      execute: async ({ location }) => {
        return { temperature: 22, condition: 'sunny' }  // SDK auto-fills
      },
    }),
  },
  stopWhen: stepCount(5),  // auto loop: tool_call → execute → fill back → call again
})
console.log(result.text)  // "The weather in Tokyo is sunny."
```

**aimux** — 5-step manual round-trip:
```typescript
// ① First call
const r1 = JSON.parse(await model.generateText(
  JSON.stringify("What's the weather in Tokyo?"),
  JSON.stringify({ tools: [{ type: 'function', name: 'get_weather', input_schema: {...} }] })
))
// ② Extract tool call
const call = r1.tool_calls[0]  // { tool_call_id, tool_name, input }
// ③ Manual execution
const weather = { temperature: 22, condition: 'sunny' }
// ④ Manually construct message sequence (must use ContentPart format, not OpenAI wire format)
const messages = [
  { role: 'user', content: "What's the weather in Tokyo?" },
  { role: 'assistant', content: [{ type: 'tool_call', tool_call_id: call.tool_call_id, tool_name: 'get_weather', input: call.input }] },
  { role: 'tool', content: [{ type: 'tool_result', tool_call_id: call.tool_call_id, output: weather }] },
]
// ⑤ Second call
const r2 = JSON.parse(await model.generateText(JSON.stringify(messages), JSON.stringify({ tools: [...] })))
console.log(r2.text)  // "The weather in Tokyo is sunny."
```

| | AI SDK | aimux |
|---|---|---|
| Round-trip automation | `stopWhen: stepCount(5)` | None, manual 5 steps |
| Tool execution | `execute` function built-in | Manual call + manual fill back |
| Message format | SDK handles automatically | Must use ContentPart format (`{type:'tool_call',...}`) |
| Multi-turn loop | Built-in | Hand-written while loop |

### 2.5 Multimodal Calls

**AI SDK** — unified top-level functions:
```typescript
import { embed, generateImage, generateSpeech } from 'ai'

const { embedding } = await embed({ model: openai.embedding('text-embedding-3-small'), value: 'hello' })
const { image } = await generateImage({ model: openai.image('dall-e-3'), prompt: 'a cat' })
const { audio } = await generateSpeech({ model: openai.speech('tts-1'), text: 'Hello', voice: 'alloy' })
```

**aimux** — different class + different signature per modality:
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
| Call consistency | Unified `embed()` / `generateImage()` / `generateSpeech()` | Each class method signature differs |
| Parameters | Native objects | `JSON.stringify(optsObject)` |
| Return | Typed objects | `JSON.parse(string)` → `any` |

### 2.6 Provider Switching (Experience Close)

**Both consistent** — switch provider by changing one line of import:

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
| Provider switching | Change import | Change import | 🟢 Consistent |
| API Key | Can auto-read from environment variables | Must pass explicitly | 🟡 Slightly inconvenient |
| Construction | Synchronous `openai('gpt-4o')` | Asynchronous `await openai(key, model)` | 🟡 Extra await |

---

## 3. Serialization Performance Analysis

### 3.1 Serialization Chain of a Single generateText Call

```
JS call                     napi FFI boundary            Rust core
─────────                   ────────────                ─────────
JSON.stringify(prompt)  ──→  serde_json::from_str  ──→  ModelPrompt
JSON.stringify(opts)    ──→  serde_json::from_str  ──→  CallOptions
                                                     │
                  generate_text (do_generate)        │
                                                     ↓
                  serde_json::to_string  ←──  GenerateTextResult
JSON.parse(resultJson)  ←──  String (JSON)
```

**5 serialization operations**: 2 `JSON.stringify` (JS) + 2 `serde_json::from_str` (Rust) + 1 `serde_json::to_string` (Rust) + 1 `JSON.parse` (JS) = actually 6.

### 3.2 Performance Order-of-Magnitude Estimation

| Operation | Typical latency | Notes |
|------|---------|------|
| `JSON.stringify` small object (prompt + opts) | ~1-5 μs | JS V8 native, extremely fast |
| `serde_json::from_str` prompt + opts | ~2-10 μs | Rust, comparable to V8 |
| `serde_json::to_string` result (incl. raw.content) | ~5-20 μs | result may be large (incl. full content array) |
| `JSON.parse` result | ~2-10 μs | V8 native |
| **Total serialization overhead** | **~10-45 μs** | |

**Compared to network request**: a single OpenAI API call typically takes **200-2000 ms** (depending on model and response length).

```
Serialization overhead:    ~0.01-0.05 ms
Network request:           ~200-2000 ms
Proportion:                < 0.025%
```

### 3.3 Serialization Overhead of Streaming

In the streaming scenario, every `StreamPart` requires `to_string` + `JSON.parse`:

```
Per part: serde_json::to_string (~1-5 μs) + JSON.parse (~0.5-2 μs) = ~1.5-7 μs
Typical stream: 100-500 parts
Total overhead: ~0.15-3.5 ms
```

Compared to streaming transmission time (seconds to tens of seconds), **proportion < 0.1%**.

### 3.4 Conclusion: Serialization Loss Is Negligible

**Serialization overhead is entirely negligible in LLM scenarios** — the bottleneck is always network I/O (200ms+), never JSON conversion (0.01-0.05ms). Serialization overhead is **4000-40000×** faster than the network.

The only exception is **ultra-high-frequency small requests** (e.g. batch embedding of large numbers of short texts), but in that scenario the Rust core's `do_embed` processes batches all at once, so the number of serializations does not grow linearly with the number of texts.

---

## 4. How to Maintain Cross-Language Consistency + Close the Experience Gap

### 4.1 Core Contradiction

```
Rust core consistency  ←→  native experience per language
     JSON string boundary         type safety + idiomatic API
```

aimux chose the JSON string boundary — the upside is that 6 bindings share one Rust core, the downside is that every binding's type safety is `any`.

### 4.2 Option A: TS Wrapper Layer (Recommended, Zero Performance Loss)

Add a pure TS wrapper layer on top of `bindings/node`, without changing the Rust side:

```typescript
// bindings/node/src/index.ts (new wrapper)
import { Model as RawModel } from '../index.js'
import type { GenerateTextOptions, GenerateTextResult, StreamPart } from './types.js'

export class Model {
  private raw: RawModel

  async generateText(
    prompt: string | ModelMessage[],
    options?: GenerateTextOptions,    // native object, not JSON string
  ): Promise<GenerateTextResult> {    // typed return, not string
    const promptJson = JSON.stringify(prompt)
    const optsJson = options ? JSON.stringify(options) : undefined
    const resultJson = await this.raw.generateText(promptJson, optsJson)
    return JSON.parse(resultJson) as GenerateTextResult
  }

  async *streamText(
    prompt: string | ModelMessage[],
    options?: GenerateTextOptions,
  ): AsyncGenerator<StreamPart> {      // typed part, not string
    const gen = await this.raw.streamText(JSON.stringify(prompt), options ? JSON.stringify(options) : undefined)
    for await (const json of gen) {
      yield JSON.parse(json) as StreamPart
    }
  }
}
```

**Type definition file** (generated from Rust's ts-rs export):
```typescript
// types.ts (auto-generated by aimux-core's ts(export))
export interface GenerateTextResult {
  text: string
  tool_calls: ToolCall[]
  finish_reason: FinishReason
  usage: Usage
  raw: GenerateResult
}
export interface StreamPart { TextDelta?: { delta: string }; ToolCall?: {...}; ... }
```

**Advantages**:
- ✅ Zero performance loss (wrapper only does stringify/parse, same as what the user does manually, Rust side unchanged)
- ✅ Type safe (returns typed objects)
- ✅ Incremental (does not affect existing raw API, wrapper is an optional layer)
- ✅ Python/Swift/Kotlin can add a wrapper the same way

**Disadvantages**:
- ❌ Does not solve Zod schema inference (tool parameters remain JSON Schema)
- ❌ Does not solve `execute` automatic round-trip (still manual)

### 4.3 Option B: Zod + execute Layer (Experience on par with AI SDK, heavier)

Add Zod + tool execute on top of the wrapper layer:

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

**Advantages**:
- ✅ Fully matches AI SDK experience (Zod inference + execute + stopWhen)

**Disadvantages**:
- ❌ Introduces a Zod dependency
- ❌ `execute` + `stopWhen` means the wrapper must implement an agent loop (repeatedly calling Rust's generateText) — logic becomes heavier
- ❌ Each language must reimplement this set (the Rust core cannot help)

### 4.4 Option C: Add a `generate` Function on the Rust Side (Cross-Language Consistency)

Add a `generate` function that does not flatten structured content + a tool round-trip loop in the Rust core:

```rust
// aimux-core/src/generate.rs
pub async fn generate(
    model: &dyn LanguageModel,
    prompt: impl Into<ModelPrompt>,
    options: GenerateOptions,    // includes tools + tool execute callback
) -> Result<GenerateResult, AiMuxError>  // returns full content, not flattened
```

Each binding calls `generate` instead of `generate_text` and gets the full `GenerateResult.content`.

**Advantages**:
- ✅ Cross-language consistent (Rust core implements round-trip logic, shared by all bindings)
- ✅ Structured content as first-class citizen

**Disadvantages**:
- ❌ tool execute callback is complex across the FFI boundary (Rust calling back into JS functions)
- ❌ Larger change

### 4.5 Recommended Path

| Phase | Option | Investment | Benefit |
|------|------|------|------|
| **Phase 1** | Option A (TS wrapper + type definitions) | Small | Type safety + idiomatic API, zero performance loss |
| **Phase 2** | Zod portion of Option B | Medium | Tool parameter Zod validation + inference |
| **Phase 3** | execute + stopWhen of Option B | Medium-large | Built-in agent loop (on par with AI SDK) |
| **Optional** | Option C (Rust-side generate) | Large | Cross-language consistent round-trip logic |

**Key judgment**: serialization performance **is not an issue** (< 0.025% proportion), no need to change the Rust side for it. The experience gap is at the **type safety and API idiom** level, which a pure TS wrapper can resolve for the most part — no performance cost, only wrapper maintenance cost.

---

## 5. Summary

| Dimension | AI SDK | aimux | Gap | Fixability |
|------|--------|-------|------|---------|
| Type safety | Zod full inference | `any` | 🔴 Severe | Solved by TS wrapper (Phase 1) |
| Serialization overhead | Zero | 6 per call | 🟢 Negligible (< 0.025%) | No fix needed |
| Streaming experience | `textStream` + typed parts | Manual parse + string matching | 🟡 Medium | Solved by TS wrapper |
| Tool round-trip | `stopWhen` + `execute` automatic | Manual 5 steps | 🔴 Severe | Requires Phase 2-3 |
| Multimodal | Unified functions | Different classes + different signatures | 🟡 Medium | TS wrapper can unify |
| Provider switching | Change import | Change import | 🟢 None | — |
| Field naming | camelCase | snake_case | 🟡 Small | Wrapper does camelCase mapping |

**Core conclusion**: aimux's experience gap is not in performance (serialization overhead is negligible), but in **type safety and API idiom**. Recommended to resolve with a TS wrapper layer (Option A) — zero performance loss, incremental, does not affect 6-language consistency.

---

## Revision History

| Date | Version | Notes |
|------|------|------|
| 2026-07-29 | v0.1 | Initial draft, based on Node binding source code + AI SDK V5 source code comparison |
