# aimux · Node.js API

> Unified LLM service access layer — one API to access AI providers

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

The vendor packages and 281 registry-backed OpenAI-compatible providers are
reachable by string name; see [providers.md](providers.md) for the provider list:

```typescript
import { provider, generateText } from '@arcships/aimux'

const model = await provider('groq', undefined, 'llama-3.3-70b')
const relay = await provider('groq', 'sk-...', 'llama-3.3-70b', {
  baseUrl: 'https://relay.example/v1',
})
const result = await generateText(model, 'Hello')
```

`openai` / `anthropic` / `deepseek` factories remain; DeepSeek uses its own vendor
chat model. Custom endpoints use a base-URL override with a compatible provider.
`openai` and by-name `provider('openai', …)` select the Responses API.

The 3rd argument of every constructor is a base URL string or a
`ProviderConfig` (`baseUrl`, `headers` as a JSON string, `organization`,
`project`, and `params` — a `Record<string, string>` filling a preset's template
parameters, e.g. `{ account_id: '…' }` for `cloudflare_workers_ai`).
`maxRetries` and `bodyOverrides` are kept in the type only so that passing them
throws `InvalidArgumentError`; retry is a per-call option
(`maxRetries` in the call options).

> **Scope:** `provider(name)` reaches vendor packages and the 281 preset rows;
> typed factories remain available. Custom endpoints use the `baseUrl` override.
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
      ├── RetryError                // the retry loop gave up; reason, errors (oldest first), lastError
      ├── JSONParseError / InvalidResponseDataError / ToolError
      ├── JSONParseError / InvalidResponseDataError
      ├── NoSuchToolError / InvalidToolInputError / ToolCallRepairError  // tool-contract errors
      ├── InvalidArgumentError / InvalidPromptError
      ├── LoadAPIKeyError / LoadSettingError   // no API key / required setting: envVar (+ description / settingName)
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
`retryMs` / `url` / `requestBodyValues` / `responseHeaders` / `providerCode` /
`providerMessage` / `responseBody` / `data`; `RetryError` adds `reason`
(`'maxRetriesExceeded'` — every permitted attempt failed with a retryable
error — or `'errorNotRetryable'` — a later attempt failed non-retryably),
`errors` — the per-attempt history, oldest first, each itself an error from
this hierarchy — and `lastError`; `TokenExpiredError` carries `status: 401`;
`NoSuchModelError` adds `modelId` / `modelType`; `NoSuchProviderError`
adds `providerId`, `modelId`, `modelType` and `availableProviders`; and `LoadAPIKeyError` / `LoadSettingError` add `envVar`
(the environment variable consulted) plus `description` / `settingName`. Missing HTTP status and retry hints are absent rather than
represented by `-1`.

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
`TextStreamPart`, not for throws.

## Text Generation

Non-streaming text generation; returns the complete result.

```typescript
const { openai, generateText } = require('@arcships/aimux')

const model = await openai('sk-...', 'gpt-4o', 'https://api.openai.com/v1')
const result = await generateText(model, 'Explain Rust ownership.', {
  maxOutputTokens: 100,
  temperature: 0.7,
  maxRetries: 0,                           // disable retries for this call
  timeout: { totalMs: 30_000, firstChunkMs: 5_000, chunkMs: 2_000 },
})

console.log(result.text)           // generated text
console.log(result.usage)          // token usage
console.log(result.finishReason)   // finish reason
console.log(result.toolCalls)      // tool calls (if any)
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
const toolCallPart = rawContent.find(c => c.type === 'tool-call')
const reasoningPart = rawContent.find(c => c.type === 'reasoning')
```

## Streaming Generation

Returns generated content as a stream, output chunk by chunk.

```typescript
const { openai, streamText } = require('@arcships/aimux')

const model = await openai('sk-...', 'gpt-4o')
for await (const part of streamText(model, 'Write a haiku about Rust.')) {
  if (part.type === 'text-delta') {
    process.stdout.write(part.delta)
  }
  if (part.type === 'finish') {
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
  inputSchema: {
    type: 'object',
    properties: {
      location: { type: 'string', description: 'City name' }
    },
    required: ['location']
  }
}]

const result = await generateText(model, "What's the weather in Tokyo?", { tools })
if (result.toolCalls.length > 0) {
  const call = result.toolCalls[0]
  console.log(call.toolName)     // get_weather
  console.log(call.input)         // { location: "Tokyo" }
}
```

> Tool calls that fail lookup, JSON parsing, or schema validation never fail
> generation: they arrive with `invalid: true` and a typed `error` on the tool
> call. Pass `repairToolCall` to fix them up.

### Repairing Invalid Tool Calls

`repairToolCall` mirrors the AI SDK option of the same name. It runs in
JavaScript, after the model call and before the result is decoded, so it can
`await` anything — including another `generateText`:

```typescript
const result = await generateText(model, "What's the weather in Tokyo?", {
  tools,
  repairToolCall: async ({ toolCall, error, inputSchema, messages }) => {
    // toolCall.input is the model's raw argument TEXT, not a parsed object.
    const fixed = await generateText(model, [
      ...messages,
      { role: 'user', content: `Rewrite these arguments to match ${JSON.stringify(inputSchema)}: ${toolCall.input}` },
    ])
    return { ...toolCall, input: fixed.text }   // null → leave the call invalid
  },
})
```

Return `null` to keep the call invalid with its original error. Throwing marks
the repair failed; the call then carries a `ToolCallRepair` error whose cause is
the thrown message, as does a replacement that still does not match the schema.
Each invalid call is repaired at most once, and a call made without a tool set
is never repaired.

Both `generateText` and `streamText` support it — in the stream, the settled
`tool-call` part is replaced, while `tool-input-delta` parts are the provider's raw
text and pass through untouched. `generateTextAsOpenai` supports it too: the
OpenAI shape carries no `invalid` marker, so it repairs the native result and
converts it (`Model.generateTextResultAsOpenai`). `streamTextAsOpenai` does not
reflect repair — its argument deltas are the provider's text, as in the AI SDK.
`RawToolCall` and `ToolCallRepairReply` are exported from the package root.

### Tool Selection Strategy

```typescript
const opts = {
  tools,
  toolChoice: 'auto'         // 'auto' | 'none' | 'required' | { type: 'tool', toolName: 'get_weather' }
}
```

## Multi-Role Messages

`prompt` accepts a message array to implement multi-turn conversation; roles support `system` / `user` / `assistant` / `tool`:

```typescript
// Node.js — multi-turn dialogue + tool round-trip
const result = await generateText(model, [
  { role: 'user', content: "What's the weather in Tokyo?" },
  { role: 'assistant', content: [{
    type: 'tool-call', toolCallId: 'call_abc',
    toolName: 'get_weather', input: { location: 'Tokyo' },
  }] },
  { role: 'tool', content: [{
    type: 'tool-result', toolCallId: 'call_abc', toolName: 'get_weather',
    output: { type: 'json', value: { temperature: 22, condition: 'sunny' } },
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
  outputFormat: 'mp3',
}))
const result = JSON.parse(resultJson)

// result.audio is a base64 string or an array of byte values
const audio = result.audio
fs.writeFileSync('out.mp3', typeof audio === 'string' ? Buffer.from(audio, 'base64') : Buffer.from(audio))
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
  providerOptions: {},
}))
const result = JSON.parse(resultJson)

// result.images is an array of base64 strings, or an array of byte arrays
const image = result.images[0]
fs.writeFileSync('out.png', typeof image === 'string' ? Buffer.from(image, 'base64') : Buffer.from(image))
```

Multimodal calls accept an optional `AbortBridge` as their last argument —
wrap the JS `AbortSignal` in one:

```typescript
const controller = new AbortController()
const resultJson = await imager.generate(
  JSON.stringify({ prompt: 'A cute baby sea otter', n: 1, providerOptions: {} }),
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
  poll: { intervalMs: 1_000, timeoutMs: 120_000 },
  providerOptions: {},
}))
const result = JSON.parse(resultJson)

// result.videos is usually [{ type: 'url', url, mediaType }]
if (result.videos[0].type === 'url') {
  console.log('Video URL:', result.videos[0].url)
}
```

The package root exports the generated `VideoCallOptions` and
`VideoPollOptions` types; both poll fields are milliseconds.

## Reranking

Reorders a document list by relevance.

```typescript
const { cohereReranking } = require('@arcships/aimux/raw')

const reranker = await cohereReranking('sk-...', 'rerank-v3.0')
const resultJson = await reranker.rerank(
  'What is Rust?',
  // docs_json is the `type`-tagged `RerankingDocuments` enum —
  // `{ type: 'object', values }` for JSON documents, `{ type: 'text', values }` for plain strings
  JSON.stringify({ type: 'object', values: [
    { text: 'Rust is a systems programming language.' },
    { text: 'Rust is a chemical element.' },
  ] }),
)
const result = JSON.parse(resultJson)

// result.ranking sorted by relevance (each rank: { index, relevanceScore })
result.ranking.forEach(r => console.log(r.index, r.relevanceScore))
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

console.log(result.providerReference)  // { openai: 'file-xxx' }
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
| `StreamTextGenerator` | returned by `Model.streamText` | async iterable of `TextStreamPart` JSON strings |

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
  GenerateTextOptions, GenerateTextResult, TextStreamPart, ModelMessage,
  Tool, ToolChoice, ToolCall, ToolResult, Usage, FinishReason, Warning,
  Role, MessageContent, ContentPart, ResponseFormat, ReasoningEffort,
  GenerateResult, FunctionTool,
} from '@arcships/aimux'
```

```typescript
// bindings/node/src/types/GenerateTextResult.ts (ts-rs generated)
export type GenerateTextResult = {
  text: string                            // generated text (all text parts concatenated)
  toolCalls: Array<ToolCall>              // tool call list (extracted from content)
  finishReason: FinishReason              // finish reason
  usage: Usage                            // token usage
  warnings: Array<Warning>                // warnings
  raw: GenerateResult                     // raw provider result (includes full content)
  reasoning: Array<ReasoningPart>         // reasoning / thinking segments
  reasoningText: string                   // the reasoning segments concatenated
  sources: Array<Source>                  // sources / citations (search-preview models)
  files: Array<GeneratedFile>             // files generated by the model
  responseMessages: Array<ModelMessage>   // assistant messages ready for the next turn
  rawFinishReason?: string                // provider's own finish-reason string
  providerMetadata?: Record<string, Record<string, JsonValue>>  // mirrored from raw.providerMetadata
  request: RequestInfo                    // sent request body
  response: ResponseInfo                  // id, timestamp, modelId, headers and body
  totalUsage: Usage                       // usage across all steps (equals usage in single-step mode)
}
```

`streamText` emits `TextStreamPart`, a union tagged by `type` (type narrowing via
`part.type === 'text-delta'` etc. works out of the box):

```typescript
// bindings/node/src/types/TextStreamPart.ts (variants, abridged)
export type TextStreamPart =
  | { type: 'stream-start', ... } | { type: 'text-start', ... } | { type: 'text-delta', ... } | { type: 'text-end', ... }
  | { type: 'tool-input-start', ... } | { type: 'tool-input-delta', ... } | { type: 'tool-input-end', ... }
  | { type: 'tool-call', ... } | { type: 'tool-result', ... }
  | { type: 'reasoning-start', ... } | { type: 'reasoning-delta', ... } | { type: 'reasoning-end', ... }
  | { type: 'source', ... } | { type: 'finish', ... } | { type: 'finish-step', ... }
  | { type: 'error', ... } | { type: 'raw', ... } | { type: 'file', ... }
  | { type: 'reasoning-file', ... } | { type: 'custom', ... } | { type: 'tool-approval-request', ... }
```

Provider-layer `StreamPart` also has `ResponseMetadata`; the call layer
consumes those events internally.

The full declarations live in `bindings/node/src/types/` — `GenerateTextOptions.ts`,
`ModelMessage.ts`, `Tool.ts`, `ToolChoice.ts`, `ContentPart.ts`,
`GenerateContent.ts`, `GenerateResult.ts`, and the `types/` directory of the
package (140 files).
