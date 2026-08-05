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

import {
  AbortBridge,
  sessionCalls as nativeSessionCalls,
  listSessions as nativeListSessions,
} from '../index.js'
import type { Model } from '../index.js'

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
  AiMuxError,
  GenerateResult,
  FunctionTool,
  SessionCall,
  SessionSource,
  SessionView,
} from './types'

// Re-export the raw napi constructors/factories so consumers can do everything
// from a single import: `import { openai, generateText } from 'aimux'`.
// Rust fn names are snake_case; napi-rs exposes them camelCased (like
// `init_logging` → `initLogging`).
export {
  Model,
  StreamTextGenerator,
  AbortBridge,
  initLogging,
  initSessionStore,
  initSessionInfer,
  sessionCalls,
  listSessions,
  openai,
  anthropic,
  deepseek,
  google,
  cohere,
  mistral,
  xai,
  bedrock,
  vertex,
  anthropicAws,
  azure,
  provider,
} from '../index.js'
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
  AiMuxError,
  GenerateResult,
  FunctionTool,
  SessionCall,
  SessionSource,
  SessionView,
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
  const optsJson = options ? JSON.stringify(options) : undefined
  const bridge = signal ? new AbortBridge(signal) : undefined
  const resultJson = await model.generateText(JSON.stringify(prompt), optsJson, bridge)
  return JSON.parse(resultJson) as GenerateTextResult
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
  const optsJson = options ? JSON.stringify(options) : undefined
  const bridge = signal ? new AbortBridge(signal) : undefined
  const gen = await model.streamText(JSON.stringify(prompt), optsJson, bridge)
  for await (const json of gen) {
    yield JSON.parse(json) as StreamPart
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
  return JSON.parse(nativeSessionCalls(sessionId)) as SessionCall[]
}

/**
 * All known sessions (RFC-0024).
 */
export function getSessions(): SessionView[] {
  return JSON.parse(nativeListSessions()) as SessionView[]
}
