// aimux — Typed wrapper layer for the Node.js (napi-rs) binding.
//
// The native binding speaks JSON strings:
//   generateText(prompt: string, options?: string): Promise<string>
//   streamText(prompt: string, options?: string): Promise<AsyncGenerator<string>>
// Users would otherwise have to JSON.stringify every input and JSON.parse every
// output, with no static types. This wrapper erases that JSON boundary: inputs
// and outputs are typed objects, using the ts-rs generated types from
// `./types.ts` (ts-rs exports directly into `./types/` — single source of
// truth in the Rust core, packaged with the npm tarball).
//
// The raw napi API (`index.js` / `index.d.ts`, auto-generated) is untouched.
// This is purely an extra TypeScript layer on top.

import * as native from '../index.js'
import {
  AbortBridge,
  createProvider as rawCreateProvider,
  getModelSpecs as rawGetModelSpecs,
} from '../index.js'
import type { Model, ProviderConfig, ProviderHandle as RawProviderHandle } from '../index.js'

// Canonical ts-rs generated types (local copy, packaged with the npm tarball).
// These are type-only imports, so they are fully erased at runtime (the
// wrapper only touches the raw `../index.js`).
import type {
  GenerateTextOptions,
  GenerateTextResult,
  StreamPart,
  ModelMessage,
  Tool,
  ToolChoice,
  ToolCall,
  ToolResult,
  Usage,
  FinishReason,
  Warning,
  Role,
  MessageContent,
  ContentPart,
  ResponseFormat,
  ReasoningEffort,
  GenerateResult,
  FunctionTool,
  SessionCall,
  SessionSource,
  SessionView,
  ChatCompletion,
  ChatCompletionChunk,

  ModelSpec,
  RuntimeModel,
} from './types'

// Error hierarchy (throw/catch). Wire payload type `AiMuxError` lives under StreamPart only.
export {
  AimuxError,
  APICallError,
  JSONParseError,
  InvalidResponseDataError,
  ToolError,
  InvalidArgumentError,
  InvalidPromptError,
  TokenExpiredError,
  UnsupportedFunctionalityError,
  NoSuchModelError,
  NoSuchProviderError,
  TimeoutError,
  RequestAbortedError,
  OtherError,
  withAimuxError,
} from './error.ts'
import { AimuxError, withAimuxError } from './error.ts'

// Re-export the raw napi constructors/factories so consumers can do everything
// from a single import: `import { openai, generateText } from 'aimux'`.
// Rust fn names are snake_case; napi-rs exposes them camelCased (like
// `init_logging` → `initLogging`).
//
// Infallible items (classes + void initializers) pass through untouched.
export {
  Model,
  StreamTextGenerator,
  AbortBridge,
  initLogging,
  initSessionStore,
  initSessionInfer,
} from '../index.js'

// Fallible raw functions are wrapped so their throws are typed `AimuxError`
// subclasses (`instanceof` works); generic wrappers preserve the signatures.
function wrapSync<A extends unknown[], R>(fn: (...args: A) => R): (...args: A) => R {
  return (...args) => {
    try {
      return fn(...args)
    } catch (e) {
      throw AimuxError.fromNative(e)
    }
  }
}

function wrapAsync<A extends unknown[], R>(
  fn: (...args: A) => Promise<R>,
): (...args: A) => Promise<R> {
  return (...args) => withAimuxError(() => fn(...args))
}

export const sessionCalls = wrapSync(native.sessionCalls)
export const listSessions = wrapSync(native.listSessions)
export const mockReplay = wrapSync(native.mockReplay)
export const openai = wrapAsync(native.openai)
export const anthropic = wrapAsync(native.anthropic)
export const deepseek = wrapAsync(native.deepseek)
export const google = wrapAsync(native.google)
export const cohere = wrapAsync(native.cohere)
export const mistral = wrapAsync(native.mistral)
export const xai = wrapAsync(native.xai)
export const bedrock = wrapAsync(native.bedrock)
export const vertex = wrapAsync(native.vertex)
export const anthropicAws = wrapAsync(native.anthropicAws)
export const azure = wrapAsync(native.azure)
export const provider = wrapAsync(native.provider)
// `ProviderHandle` is the napi class for provider handles (RFC-0027); export
// it as a type so typed-wrapper users can reference it.
export type { ProviderHandle } from '../index.js'
// Both meanings: the `ProviderName` const object (runtime, for `ProviderName.groq`)
// and the derived string-union type. A value export resolves at runtime, so the
// specifier needs the real `.ts` extension for Node's type-stripping test runs;
// tsc rewrites it to `.js` on emit (rewriteRelativeImportExtensions).
export { ProviderName } from './types/ProviderName.ts'

// Public type surface — typed objects, no `any`.
export type {
  GenerateTextOptions,
  GenerateTextResult,
  StreamPart,
  ModelMessage,
  Tool,
  ToolChoice,
  ToolCall,
  ToolResult,
  Usage,
  FinishReason,
  Warning,
  Role,
  MessageContent,
  ContentPart,
  ResponseFormat,
  ReasoningEffort,
  GenerateResult,
  FunctionTool,
  SessionCall,
  SessionSource,
  SessionView,
  ChatCompletion,
  ChatCompletionChunk,

  ModelSpec,
  RuntimeModel,
}

/**
 * A raw napi `Model` instance returned by `openai()` / `anthropic()` / …
 *
 * The wrapper accepts one of these and hides the JSON-string boundary behind
 * typed inputs and outputs. `RawModel` is just an alias for the napi `Model`
 * class instance type — pass the exact object a provider factory gives you.
 */
export type RawModel = Model

/**
 * Generate text (non-streaming). Returns a typed {@link GenerateTextResult}.
 *
 * @param model   - A raw model instance from `openai()`, `anthropic()`, etc.
 * @param prompt  - A plain string or an array of typed chat messages.
 * @param options - Optional typed generation options (tools, tool_choice,
 *                  temperature, response_format, …).
 * @param signal  - Optional `AbortSignal`; aborting it cancels the call.
 *
 * Internally calls the raw
 * `model.generateText(JSON.stringify(prompt), options ? JSON.stringify(options) : undefined)`
 * and `JSON.parse`s the returned JSON into a typed object.
 *
 * @example
 * ```ts
 * import { openai, generateText } from 'aimux'
 * const model = await openai(apiKey, 'gpt-4o')
 * const result = await generateText(model, 'What is Rust?')
 * console.log(result.text, result.usage)
 * ```
 */
export async function generateText(
  model: RawModel,
  prompt: string | ModelMessage[],
  options?: GenerateTextOptions,
  signal?: AbortSignal,
): Promise<GenerateTextResult> {
  return withAimuxError(async () => {
    const optsJson = options ? JSON.stringify(options) : undefined
    const bridge = signal ? new AbortBridge(signal) : undefined
    const resultJson = await model.generateText(JSON.stringify(prompt), optsJson, bridge)
    return JSON.parse(resultJson) as GenerateTextResult
  })
}

/**
 * Stream text from a model. Yields typed {@link StreamPart}s.
 *
 * @param model   - A raw model instance from `openai()`, `anthropic()`, etc.
 * @param prompt  - A plain string or an array of typed chat messages.
 * @param options - Optional typed generation options (tools, tool_choice, …).
 * @param signal  - Optional `AbortSignal`; aborting it cancels the stream.
 *
 * Internally drives the raw `model.streamText(JSON.stringify(prompt), …)`
 * async generator and `JSON.parse`s each JSON-string chunk before yielding it
 * as a typed `StreamPart`.
 *
 * @example
 * ```ts
 * import { openai, streamText } from 'aimux'
 * const model = await openai(apiKey, 'gpt-4o')
 * for await (const part of streamText(model, 'Write a haiku about Rust.')) {
 *   if ('TextDelta' in part) process.stdout.write(part.TextDelta.delta)
 * }
 * ```
 */
export async function* streamText(
  model: RawModel,
  prompt: string | ModelMessage[],
  options?: GenerateTextOptions,
  signal?: AbortSignal,
): AsyncGenerator<StreamPart> {
  try {
    const optsJson = options ? JSON.stringify(options) : undefined
    const bridge = signal ? new AbortBridge(signal) : undefined
    const gen = await model.streamText(JSON.stringify(prompt), optsJson, bridge)
    for await (const json of gen) {
      yield JSON.parse(json) as StreamPart
    }
  } catch (e) {
    throw AimuxError.fromNative(e)
  }
}

/**
 * All calls of a session, ordered by step (RFC-0024). Empty if the session
 * is unknown or no store is registered.
 *
 * `initSessionStore()` / `initSessionInfer(enabled)` (raw napi, re-exported
 * above) must be called first to register the store / opt-in inferer.
 */
export function getSessionCalls(sessionId: string): SessionCall[] {
  try {
    return JSON.parse(native.sessionCalls(sessionId)) as SessionCall[]
  } catch (e) {
    throw AimuxError.fromNative(e)
  }
}

/**
 * All known sessions (RFC-0024).
 */
export function getSessions(): SessionView[] {
  try {
    return JSON.parse(native.listSessions()) as SessionView[]
  } catch (e) {
    throw AimuxError.fromNative(e)
  }
}

/**
 * Generate text and return an OpenAI Chat Completion (non-streaming).
 *
 * Works with **any** provider — the result is always a standard OpenAI
 * `ChatCompletion` object.
 *
 * @example
 * ```ts
 * import { openai, generateTextAsOpenai } from 'aimux'
 * const model = await openai(apiKey, 'gpt-4o')
 * const completion = await generateTextAsOpenai(model, 'What is Rust?')
 * console.log(completion.choices[0].message.content)
 * ```
 */
export async function generateTextAsOpenai(
  model: RawModel,
  prompt: string | ModelMessage[],
  options?: GenerateTextOptions,
  signal?: AbortSignal,
): Promise<ChatCompletion> {
  return withAimuxError(async () => {
    const optsJson = options ? JSON.stringify(options) : undefined
    const bridge = signal ? new AbortBridge(signal) : undefined
    const resultJson = await model.generateTextAsOpenai(
      JSON.stringify(prompt),
      optsJson,
      bridge,
    )
    return JSON.parse(resultJson) as ChatCompletion
  })
}

/**
 * Stream text as OpenAI Chat Completion chunks.
 *
 * Works with **any** provider — yields standard OpenAI `ChatCompletionChunk`
 * objects. Pass `streamOptions` via `providerOptions.openai.stream_options`:
 * `{ include_usage: true, include_reasoning: true }` (both default true).
 *
 * @example
 * ```ts
 * import { openai, streamTextAsOpenai } from 'aimux'
 * const model = await openai(apiKey, 'gpt-4o')
 * for await (const chunk of streamTextAsOpenai(model, 'Write a haiku.')) {
 *   const delta = chunk.choices[0]?.delta?.content
 *   if (delta) process.stdout.write(delta)
 * }
 * ```
 */
export async function* streamTextAsOpenai(
  model: RawModel,
  prompt: string | ModelMessage[],
  options?: GenerateTextOptions,
  signal?: AbortSignal,
): AsyncGenerator<ChatCompletionChunk> {
  try {
    const optsJson = options ? JSON.stringify(options) : undefined
    const bridge = signal ? new AbortBridge(signal) : undefined
    const gen = await model.streamTextAsOpenai(JSON.stringify(prompt), optsJson, bridge)
    for await (const json of gen) {
      yield JSON.parse(json) as ChatCompletionChunk
    }
  } catch (e) {
    throw AimuxError.fromNative(e)
  }
}

// ─────────────────────────────────────────────────────────────────────────────
// Provider handles (RFC-0027) — createProvider / listModels / model
// ─────────────────────────────────────────────────────────────────────────────

/**
 * A typed wrapper around the raw napi `ProviderHandle` class. Created by
 * {@link createProvider}, supports {@link ProviderHandleTyped.listModels} and
 * {@link ProviderHandleTyped.model}.
 *
 * @example
 * ```ts
 * import { createProvider, generateText } from 'aimux'
 * const p = await createProvider('deepseek', apiKey)
 * const models = await p.listModels()
 * const model = await p.model(models[0].id)
 * const result = await generateText(model, 'Hello')
 * ```
 */
export class ProviderHandleTyped {
  private readonly raw: RawProviderHandle

  constructor(raw: RawProviderHandle) {
    this.raw = raw
  }

  /** List models available on this provider (runtime discovery + anya2a spec). */
  async listModels(): Promise<RuntimeModel[]> {
    return withAimuxError(async () => {
      return JSON.parse(await this.raw.listModels()) as RuntimeModel[]
    })
  }

  /** Build a language model from a discovered model id. */
  async model(modelId: string): Promise<RawModel> {
    return withAimuxError(() => this.raw.model(modelId))
  }
}

/**
 * Create a **provider handle** (RFC-0027) for a registry-backed provider.
 *
 * Unlike `provider()` (which binds to a single modelId), this returns a handle
 * that supports `listModels()` (runtime discovery) and `model()`.
 *
 * @param name     - Provider name (e.g. `"deepseek"`, `"groq"`).
 * @param apiKey   - API key; omit/null to read the provider's env var.
 * @param config   - Optional provider config (baseUrl, headers, …).
 *
 * @example
 * ```ts
 * import { createProvider, generateText } from 'aimux'
 * const p = await createProvider('deepseek', process.env.DEEPSEEK_API_KEY)
 * const models = await p.listModels()
 * const model = await p.model(models[0].id)
 * const result = await generateText(model, 'Hello')
 * ```
 */
export async function createProvider(
  name: string,
  apiKey?: string,
  config?: ProviderConfig,
): Promise<ProviderHandleTyped> {
  return withAimuxError(async () => {
    const raw = await rawCreateProvider(name, apiKey ?? null, config ?? null)
    return new ProviderHandleTyped(raw)
  })
}

/**
 * Fetch the community model catalogue (anya2a). Returns a `Catalogue` object
 * with a `lookup(provider, modelId)` method. Thin fetch — no caching; the host
 * decides how to persist/reuse the result.
 *
 * @param sourceUrl - Optional URL override (default = anya2a endpoint).
 *
 * @example
 * ```ts
 * import { createProvider, getModelSpecs, generateText } from 'aimux'
 * const [p, catalogue] = await Promise.all([
 *   createProvider('deepseek', apiKey),
 *   getModelSpecs(),
 * ])
 * const models = await p.listModels()
 * const model = await p.model(models[0].id)
 * const spec = catalogue.specs?.['deepseek']?.[models[0].id] // community portrait
 * ```
 */
export async function getModelSpecs(sourceUrl?: string): Promise<unknown> {
  return withAimuxError(async () => {
    return JSON.parse(await rawGetModelSpecs(sourceUrl ?? null))
  })
}
