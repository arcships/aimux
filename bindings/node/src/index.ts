// aimux — Typed wrapper layer for the Node.js (napi-rs) binding.
//
// The native binding speaks JSON strings:
//   generateText(prompt: string, options?: string): Promise<string>
//   streamText(prompt: string, options?: string): Promise<AsyncGenerator<string>>
// Users would otherwise have to JSON.stringify every input and JSON.parse every
// output, with no static types. This wrapper erases that JSON boundary: inputs
// and outputs are typed objects, using the ts-rs generated types from
// `aimux-core/bindings` (the single source of truth — NOT a local copy).
//
// The raw napi API (`index.js` / `index.d.ts`, auto-generated) is untouched.
// This is purely an extra TypeScript layer on top.

import type { Model } from '../index.js'

// Canonical ts-rs generated types. These are type-only imports, so they are
// fully erased at runtime (the wrapper only touches the raw `../index.js`).
import type { GenerateTextOptions } from '../../aimux-core/bindings/GenerateTextOptions.ts'
import type { GenerateTextResult } from '../../aimux-core/bindings/GenerateTextResult.ts'
import type { StreamPart } from '../../aimux-core/bindings/StreamPart.ts'
import type { ModelMessage } from '../../aimux-core/bindings/ModelMessage.ts'
import type { Tool } from '../../aimux-core/bindings/Tool.ts'
import type { ToolChoice } from '../../aimux-core/bindings/ToolChoice.ts'
import type { ToolCall } from '../../aimux-core/bindings/ToolCall.ts'
import type { ToolResult } from '../../aimux-core/bindings/ToolResult.ts'
import type { Usage } from '../../aimux-core/bindings/Usage.ts'
import type { FinishReason } from '../../aimux-core/bindings/FinishReason.ts'
import type { Warning } from '../../aimux-core/bindings/Warning.ts'
import type { Role } from '../../aimux-core/bindings/Role.ts'
import type { MessageContent } from '../../aimux-core/bindings/MessageContent.ts'
import type { ContentPart } from '../../aimux-core/bindings/ContentPart.ts'
import type { ResponseFormat } from '../../aimux-core/bindings/ResponseFormat.ts'
import type { ReasoningEffort } from '../../aimux-core/bindings/ReasoningEffort.ts'
import type { AiMuxError } from '../../aimux-core/bindings/AiMuxError.ts'
import type { GenerateResult } from '../../aimux-core/bindings/GenerateResult.ts'
import type { FunctionTool } from '../../aimux-core/bindings/FunctionTool.ts'

// Re-export the raw napi constructors/factories so consumers can do everything
// from a single import: `import { openai, generateText } from 'aimux'`.
export { Model, StreamTextGenerator, openai, anthropic, deepseek } from '../index.js'

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
): Promise<GenerateTextResult> {
  const optsJson = options ? JSON.stringify(options) : undefined
  const resultJson = await model.generateText(JSON.stringify(prompt), optsJson)
  return JSON.parse(resultJson) as GenerateTextResult
}

/**
 * Stream text from a model. Yields typed {@link StreamPart}s.
 *
 * @param model   - A raw model instance from `openai()`, `anthropic()`, etc.
 * @param prompt  - A plain string or an array of typed chat messages.
 * @param options - Optional typed generation options (tools, tool_choice, …).
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
): AsyncGenerator<StreamPart> {
  const optsJson = options ? JSON.stringify(options) : undefined
  const gen = await model.streamText(JSON.stringify(prompt), optsJson)
  for await (const json of gen) {
    yield JSON.parse(json) as StreamPart
  }
}
