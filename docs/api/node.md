# aimux · Node.js API

> Unified LLM service access layer — one API to access 325 AI providers

Shared reference — parameter tables, result shapes, factory functions, and the
feature coverage matrix — lives in the [API overview](../API.md).

## Quick Start

```bash
npm install @arcships/aimux
```

```typescript
import { openai, generateText } from '@arcships/aimux'

const model = await openai(process.env.OPENAI_API_KEY!, 'gpt-4o')
const result = await generateText(model, 'What is Rust?')
console.log(result.text)
```

## Providers

All 251 built-in OpenAI-compatible providers are registry-backed. Look them up
by name; the `ProviderName` type is a string-literal union generated from
`provider-registry.json`, so your IDE autocompletes and typo'd names fail
type-checking:

```typescript
import { provider, generateText, ProviderName } from '@arcships/aimux'

// 推荐:ProviderName.groq 写法(IDE 补全 + 类型检查)
const model = await provider(ProviderName.groq, undefined, 'llama-3.3-70b')
// 字符串形式同样可用:
const relay = await provider('groq', 'sk-...', 'llama-3.3-70b', {
  baseUrl: 'https://relay.example/v1',
  maxRetries: 0,
})
const result = await generateText(model, 'Hello')
```

`openai` / `anthropic` / `deepseek` factories remain (deepseek is now
registry-backed). For custom providers not in the registry, build from the
base classes with `createProvider`-style config via the base-URL override.

> **Scope:** `provider(name)` covers only the 251 registry OpenAI-compatible
> providers; Anthropic/Google/multimodal/local → typed factories
> (`anthropic(apiKey, model)`); custom endpoints → `baseUrl` override.
> Full list: [providers.md](providers.md).

## Desktop and Electron compatibility

The package ships one Node-API 8 binary per desktop OS and architecture. The
root package selects a platform package at load time, so installers do not
carry binaries for the other five targets.

| OS | Architecture | Native package | Runtime baseline |
|---|---|---|---|
| Windows | x64 | `@arcships/aimux-win32-x64-msvc` | Static MSVC CRT; no Visual C++ Redistributable required |
| Windows | ARM64 | `@arcships/aimux-win32-arm64-msvc` | Static MSVC CRT; no Visual C++ Redistributable required |
| macOS | x64 | `@arcships/aimux-darwin-x64` | Addon deployment target 10.13; system frameworks only |
| macOS | ARM64 | `@arcships/aimux-darwin-arm64` | Addon deployment target 11.0; system frameworks only |
| Linux | x64 | `@arcships/aimux-linux-x64-gnu` | glibc 2.17 or newer |
| Linux | ARM64 | `@arcships/aimux-linux-arm64-gnu` | glibc 2.17 or newer |

The addon uses Node-API rather than Electron's version-specific native ABI, so
it does not require an Electron-specific rebuild. Load it from the main process
or a Node-enabled preload script. Keep npm optional dependencies enabled when
installing, because the native platform package is an optional dependency.

When packaging with ASAR, keep native addons unpacked:

```yaml
asarUnpack:
  - '**/*.node'
```

Linux musl distributions such as Alpine are not included in the six desktop
targets. The GNU/Linux builds use rustls and do not require system OpenSSL.

## Errors

Two aimux error types, one per Rust type; the bridge's own failures are
plain napi errors (see below). `AiMuxError` values throw an **`AimuxError`
subclass hierarchy** (Vercel AI SDK style — `instanceof`, not stringly `code`
checks); the recorder throws its own class:

```text
Error
 └── AimuxError
      ├── APICallError              // provider call/transport failure; status when observed
      ├── JSONParseError / InvalidResponseDataError
      ├── NoSuchToolError / InvalidToolInputError / ToolCallRepairError  // tool-contract errors
      ├── InvalidArgumentError / InvalidPromptError
      ├── TokenExpiredError
      ├── UnsupportedFunctionalityError
      ├── NoSuchModelError / NoSuchProviderError
      ├── TimeoutError
      ├── RequestAbortedError
      └── OtherError

Error                             // the recorder's own failure type — not an AimuxError
 └── RecordingError               // initRecording(): code 'Init' | 'OpenFile' | 'Spawn'; recordingTryFlush(): 'WriterGone' | 'FlushTimeout' | 'Write'
```

Failures of the binding's own bridge layer (the napi-rs side, never `AiMuxError`)
follow napi-rs: a plain `Error` whose `code` is a napi status name, passed
through unchanged — no aimux class.

| scenario                                            | thrown                                                        |
|-----------------------------------------------------|---------------------------------------------------------------|
| a wire JSON text (`prompt_json`, `opts_json`, …) does not parse | `Error`, `code: 'InvalidArg'`, message `"prompt_json: invalid JSON: …"` |
| closed / invalid native object (`TranscriptionSession`) | `Error`, `code: 'InvalidArg'`, message `"transcription session is closed …"` |
| the binding could not serialize a result           | `Error`, `code: 'GenericFailure'`, message `"serialize result: …"` |
| a bridge invariant broke                            | `Error`, `code: 'GenericFailure'`                             |
| argument type errors                                | napi-rs's own `Error` (`code: 'StringExpected'`, …)           |
| panic                                               | napi's mechanism                                              |

Well-formed JSON that violates the schema, and business validation (empty
model list, `cap === 0`, no recordings, …) stay `InvalidArgumentError`
— that is what the core would say. Both package entrypoints register the
exported JavaScript constructors with the native addon at load time. Rust
constructs that exact class before throwing or rejecting, so `instanceof`
works directly for synchronous calls, promises, and stream/session errors;
`name` is the ordinary JavaScript error name, not a discriminator to parse.

Every `AimuxError` instance has the ordinary `Error` fields. There is no aimux
`code` discriminator and no JSON companion. Payload fields belong to the class
that carries them: `APICallError` adds `retryable` and optional `status` /
`retryMs` / `providerCode` /
`providerMessage` / `responseBody` / `requestId`, `TokenExpiredError` carries
`status: 401`, `NoSuchModelError` adds `modelId` / `modelType`, and
`NoSuchProviderError` adds `providerId`. Missing HTTP status and retry hints are
absent rather than represented by `-1`.

```typescript
import { generateText, AimuxError, APICallError } from '@arcships/aimux'

try {
  await generateText(model, 'hi')
} catch (e) {
  if (e instanceof APICallError) {
    // classify on status (AI SDK APICallError.statusCode):
    if (e.status === 429) {
      // rate limited — e.retryMs
    } else if (e.status === 401) {
      // auth failure
    } else if (e.status === 404) {
      // model not found
    }
  } else if (e instanceof AimuxError) {
    // any AiMuxError failure
  } else if (e instanceof Error && 'code' in e && e.code === 'InvalidArg') {
    // the napi-rs bridge rejected an argument (bad wire JSON, closed session)
  }
}
```

The ts-rs wire type `AiMuxError` is only for payload unions inside
`StreamPart`, not for throws.

## Text Generation

Non-streaming text generation; returns the complete result.

```typescript
const { openai, generateText } = require('@arcships/aimux')

const model = await openai('sk-...', 'gpt-4o', 'https://api.openai.com/v1')
const result = await generateText(model, 'Explain Rust ownership.', {
  max_output_tokens: 100,
  temperature: 0.7,
  max_retries: 0,                          // disable retries for this call
  timeout: { total_ms: 30_000, first_chunk_ms: 5_000, chunk_ms: 2_000 },
})

console.log(result.text)           // generated text
console.log(result.usage)          // token usage
console.log(result.finish_reason)  // finish reason
console.log(result.tool_calls)     // tool calls (if any)
```

Cancellation via `AbortSignal` (4th argument — works for both
`generateText` and `streamText`):

```typescript
const controller = new AbortController()
const result = await generateText(model, 'Explain Rust ownership.', {}, controller.signal)
controller.abort() // cancels an in-flight call; pre-aborted signals fail fast
```

Multimodal calls (image/speech/video/transcription/rerank/search) accept an
optional `AbortBridge` (wrap a JS `AbortSignal`) as their last argument:

> Parameters, return value, and the `raw.content` variants are documented in
> the [API overview](../API.md#text-generation).

### Structured content (`raw.content`)

```typescript
// access structured content
const result = await generateText(model, "...", { tools })
const rawContent = result.raw.content
const toolCallPart = rawContent.find(c => c.ToolCall)
const reasoningPart = rawContent.find(c => c.Reasoning)
```

## Streaming Generation

Returns generated content as a stream, output chunk by chunk.

```typescript
const { openai, streamText } = require('@arcships/aimux')

const model = await openai('sk-...', 'gpt-4o')
for await (const part of streamText(model, 'Write a haiku about Rust.')) {
  if (part.TextDelta) {
    process.stdout.write(part.TextDelta.delta)
  }
  if (part.Finish) {
    console.log('\n[done]')
  }
}
```

> Stream part variants are documented in the [API overview](../API.md#streaming-generation).

## Tool Calling

Tool definitions are language-agnostic data descriptions (JSON Schema) that require no macros.

### Defining Tools

```typescript
// Node.js — construct the data object directly
const tools = [{
  type: 'function',
  name: 'get_weather',
  description: 'Get current weather',
  input_schema: {
    type: 'object',
    properties: {
      location: { type: 'string', description: 'City name' }
    },
    required: ['location']
  }
}]

const result = await generateText(model, "What's the weather in Tokyo?", { tools })
if (result.tool_calls.length > 0) {
  const call = result.tool_calls[0]
  console.log(call.tool_name)     // get_weather
  console.log(call.input)         // { location: "Tokyo" }
}
```

> The `repair_tool_call` callback is Rust-core-only (it cannot cross the FFI
> boundary); tool calls that stay invalid arrive with `invalid: true` and a
> typed `error` on the tool call.

### Tool Selection Strategy

```typescript
const opts = {
  tools,
  tool_choice: 'auto'        // 'auto' | 'none' | 'required' | { type: 'tool', toolName: 'get_weather' }
}
```

## Multi-Role Messages

`prompt` accepts a message array to implement multi-turn conversation; roles support `system` / `user` / `assistant` / `tool`:

```typescript
// Node.js — multi-turn dialogue + tool round-trip
const result = await generateText(model, [
  { role: 'user', content: "What's the weather in Tokyo?" },
  { role: 'assistant', content: [{
    type: 'tool_call', tool_call_id: 'call_abc',
    tool_name: 'get_weather', input: { location: 'Tokyo' },
  }] },
  { role: 'tool', content: [{
    type: 'tool_result', tool_call_id: 'call_abc', tool_name: 'get_weather',
    result: { temperature: 22, condition: 'sunny' },
  }] },
], { tools })
```

## Vector Embedding

Converts text into a vector representation.

```typescript
const { openaiEmbedding } = require('@arcships/aimux/raw')

const embedder = await openaiEmbedding('sk-...', 'text-embedding-3-small')
const resultJson = await embedder.embed(JSON.stringify(['hello', 'world']))
const result = JSON.parse(resultJson)

console.log(result.embeddings.length)  // 2
console.log(result.embeddings[0].length)  // 1536 (dimension depends on model)
console.log(result.usage.tokens)  // input token count
```

## Speech Synthesis (TTS)

Converts text into speech audio.

```typescript
const { openaiSpeech } = require('@arcships/aimux/raw')
const fs = require('fs')

const speaker = await openaiSpeech('sk-...', 'tts-1')
const resultJson = await speaker.generate(JSON.stringify({
  text: 'Hello world!',
  voice: 'alloy',
  output_format: 'mp3',
}))
const result = JSON.parse(resultJson)

// audio is in result.audio (base64 or binary)
if (result.audio.Base64) {
  fs.writeFileSync('out.mp3', Buffer.from(result.audio.Base64, 'base64'))
}
```

## Speech to Text (STT)

Converts audio into text (non-streaming).

```typescript
const { openaiTranscription } = require('@arcships/aimux/raw')
const fs = require('fs')

const transcriber = await openaiTranscription('sk-...', 'whisper-1')
const audioBase64 = fs.readFileSync('audio.mp3').toString('base64')
const resultJson = await transcriber.generate(audioBase64, 'audio/mp3')
const result = JSON.parse(resultJson)

console.log(result.text)       // transcribed text
console.log(result.segments)   // timestamped segments
console.log(result.language)   // detected language
```

## Image Generation

```typescript
const { openaiImage, AbortBridge } = require('@arcships/aimux/raw')
const fs = require('fs')

const imager = await openaiImage('sk-...', 'dall-e-3')
const resultJson = await imager.generate(JSON.stringify({
  prompt: 'A cute baby sea otter',
  n: 1,
  provider_options: {},
}))
const result = JSON.parse(resultJson)

if (result.images.Base64) {
  fs.writeFileSync('out.png', Buffer.from(result.images.Base64[0], 'base64'))
}
```

Multimodal calls accept an optional `AbortBridge` as their last argument —
wrap the JS `AbortSignal` in one:

```typescript
const controller = new AbortController()
const resultJson = await imager.generate(
  JSON.stringify({ prompt: 'A cute baby sea otter', n: 1, provider_options: {} }),
  new AbortBridge(controller.signal),
)
controller.abort() // cancels the image call
```

## Video Generation

Video generation typically returns a URL (not binary).

```typescript
const { googleVideo } = require('@arcships/aimux/raw')

const videor = await googleVideo('sk-...', 'veo-3.0')
const resultJson = await videor.generate(JSON.stringify({
  prompt: 'A cat playing piano',
  n: 1,
  provider_options: {},
}))
const result = JSON.parse(resultJson)

// result.videos is usually [{ Url: { url, media_type } }]
if (result.videos[0].Url) {
  console.log('Video URL:', result.videos[0].Url.url)
}
```

## Reranking

Reorders a document list by relevance.

```typescript
const { cohereReranking } = require('@arcships/aimux/raw')

const reranker = await cohereReranking('sk-...', 'rerank-v3.0')
const resultJson = await reranker.rerank(
  'What is Rust?',
  // docs_json is the externally-tagged `RerankingDocuments` enum —
  // `{ Object: { values } }` for JSON documents, `{ Text: { values } }` for plain strings
  JSON.stringify({ Object: { values: [
    { text: 'Rust is a systems programming language.' },
    { text: 'Rust is a chemical element.' },
  ] } }),
)
const result = JSON.parse(resultJson)

// result.ranking sorted by relevance (each rank: { index, relevance_score })
result.ranking.forEach(r => console.log(r.index, r.relevance_score))
```

## Search

```typescript
const { tavilySearch } = require('@arcships/aimux/raw')

const searcher = await tavilySearch('tvly-...')
const resultJson = await searcher.search('What is Rust?')
const result = JSON.parse(resultJson)

console.log(result.results[0].title)  // ordered result list
console.log(result.answer)            // provider's summary, if any
```

## File Upload

Uploads a file to the provider and returns a file ID.

```typescript
const { openaiFiles } = require('@arcships/aimux/raw')
const fs = require('fs')

const files = await openaiFiles('sk-...')
const fileBase64 = fs.readFileSync('doc.pdf').toString('base64')
const resultJson = await files.uploadFile(fileBase64, 'application/pdf')
const result = JSON.parse(resultJson)

console.log(result.provider_reference)  // { openai: 'file-xxx' }
```

## API Surface

The `@arcships/aimux` package has two layers:

| Layer | Source | Boundary |
|------|------|------|
| **Native (napi-rs)** | `@arcships/aimux/raw` — `bindings/node/src/native.ts` over the generated native loader | JSON strings in / JSON strings out |
| **Typed wrapper** | `@arcships/aimux` — `bindings/node/src/index.ts` | Typed objects (ts-rs types, re-exported from the package root) |

### Native classes and methods

| Class | Factory functions | Methods |
|------|------|------|
| `Model` | `openai` / `anthropic` / `deepseek` | `generateText(promptJson, optsJson?)`, `streamText(promptJson, optsJson?)` |
| `EmbeddingModel` | `openaiEmbedding` / `cohereEmbedding` / `googleEmbedding` | `embed(valuesJson, optsJson?)` |
| `SpeechModel` | `openaiSpeech` | `generate(optsJson)` |
| `TranscriptionModel` | `openaiTranscription` | `generate(audioBase64, mediaType, optsJson?)` |
| `ImageModel` | `openaiImage` / `googleImage` | `generate(optsJson)` |
| `VideoModel` | `googleVideo` | `generate(optsJson)` |
| `RerankingModel` | `cohereReranking` | `rerank(query, docsJson, optsJson?)` |
| `SearchModel` | `tavilySearch` | `search(query, optsJson?)` |
| `Files` | `openaiFiles(apiKey, baseUrl?)` | `uploadFile(dataBase64, mediaType, optsJson?)` |
| `StreamTextGenerator` | returned by `Model.streamText` | async iterable of `StreamPart` JSON strings |

All factories return a `Promise` and accept an optional `baseUrl` as the last
parameter. All native methods take and return JSON strings — the typed wrapper
(`generateText` / `streamText`) calls them and `JSON.parse`s into the types
below.

## Types

Type declarations are ts-rs generated from the Rust core into
`bindings/node/src/types/*.ts` (single source of truth — the wrapper re-exports
them, not a local copy):

```typescript
import type {
  GenerateTextOptions, GenerateTextResult, StreamPart, ModelMessage,
  Tool, ToolChoice, ToolCall, ToolResult, Usage, FinishReason, Warning,
  Role, MessageContent, ContentPart, ResponseFormat, ReasoningEffort,
  GenerateResult, FunctionTool,
} from '@arcships/aimux'
```

```typescript
// bindings/node/src/types/GenerateTextResult.ts (ts-rs generated)
export type GenerateTextResult = {
  text: string                            // generated text (all Text variants concatenated)
  tool_calls: Array<ToolCall>             // tool call list (extracted from content)
  finish_reason: FinishReason             // finish reason
  usage: Usage                            // token usage
  warnings: Array<Warning>                // warnings
  raw: GenerateResult                     // raw provider result (includes full content)
  reasoning: Array<ReasoningPart>         // reasoning / thinking segments
  reasoning_text: string                  // the reasoning segments concatenated
  sources: Array<SourcePart>              // sources / citations (search-preview models)
  files: Array<FilePart>                  // files generated by the model
  response_messages: Array<ModelMessage>  // assistant messages ready for the next turn
  raw_finish_reason: string | null        // provider's own finish-reason string
  provider_metadata: JsonValue | null     // mirrored from raw.provider_metadata
  response: ResponseMetadata              // mirrored from raw.response (id, timestamp, model_id)
  total_usage: Usage                      // usage across all steps (equals usage in single-step mode)
}
```

`StreamPart` is an external-tagged union of 18 variants (each is a one-key
object — type narrowing via `part.TextDelta` etc. works out of the box):

```typescript
// bindings/node/src/types/StreamPart.ts (variants, abridged)
export type StreamPart =
  | { StreamStart: ... } | { TextStart: ... } | { TextDelta: ... } | { TextEnd: ... }
  | { ToolInputStart: ... } | { ToolInputDelta: ... } | { ToolInputEnd: ... }
  | { ToolCall: ... } | { ToolResult: ... }
  | { ReasoningStart: ... } | { ReasoningDelta: ... } | { ReasoningEnd: ... }
  | { ResponseMetadata: ... } | { Source: ... } | { Finish: ... }
  | { Error: ... } | { Raw: ... } | { File: ... }
```

The full declarations live in `bindings/node/src/types/` — `GenerateTextOptions.ts`,
`ModelMessage.ts`, `Tool.ts`, `ToolChoice.ts`, `ContentPart.ts`,
`GenerateContent.ts`, `GenerateResult.ts`, and the `types/` directory of the
package (140 files).
