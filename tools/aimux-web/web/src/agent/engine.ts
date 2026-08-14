// Agent loop engine — the loop runs in the browser (RFC-0029 §6, decision D1).
//
// Each model call goes through `POST /api/calls` (streaming SSE). The engine
// collects text + tool calls, executes tools via `/api/tools/:name`, appends
// the assistant/tool messages, and repeats until the model stops calling
// tools or `max_steps` is reached. Every call carries the same `session_id`
// so the Traces page groups the whole run into a waterfall.

import { api, callStream, parseStreamPart } from '../api/client'
import type { WireCallRequest } from '../types/WireCallRequest'
import type { StreamPart } from '../types/StreamPart'

/** A chat message as sent to the backend (loose typing: the backend wire
 * schema's `JsonValue` is a recursive union that trips TS2589 inside
 * `ref`/`reactive` unwrapping, so content parts are `any[]` here). */
export type AgentMessage = { role: string; content: any[] }

export interface ToolCallView {
  id: string
  name: string
  input: unknown
  result?: unknown
  is_error?: boolean
  executing?: boolean
}

export interface AgentStepView {
  step: number
  status: 'running' | 'done' | 'error' | 'stopped'
  text: string
  toolCalls: ToolCallView[]
  usage?: unknown
  latencyMs?: number
  error?: string
  meta?: { call_id?: string; session_id?: string | null; step?: number | null; outcome?: string }
}

export interface AgentDef {
  name: string
  system_prompt: string
  provider: string
  model: string
  /** '' or `env:VAR` */
  api_key: string
  base_url?: string
  tools: string[]
  max_steps: number
  temperature: number
  session_id: string
  /** Route calls through the loaded mock model (offline demo/testing). */
  mock: boolean
}

export interface AgentRun {
  messages: AgentMessage[]
  steps: AgentStepView[]
  status: 'idle' | 'running' | 'done' | 'error' | 'stopped'
  abort: () => void
}

/** A fresh, empty run container (reactive in the view). */
export function newRun(): AgentRun {
  return { messages: [], steps: [], status: 'idle', abort: () => {} }
}

export async function runAgent(
  def: AgentDef,
  run: AgentRun,
  toolSchemas: Map<string, { name: string; description?: string | null; parameters: unknown }>,
  userText: string,
): Promise<void> {
  run.messages = []
  if (def.system_prompt.trim()) {
    run.messages.push({ role: 'system', content: [{ type: 'text', text: def.system_prompt }] })
  }
  if (userText.trim()) {
    run.messages.push({ role: 'user', content: [{ type: 'text', text: userText }] })
  }
  run.steps = []
  run.status = 'running'

  let controller: AbortController | null = null
  run.abort = () => controller?.abort()

  try {
    for (let step = 0; step < def.max_steps; step++) {
      if (run.status !== 'running') break
      const view: AgentStepView = {
        step,
        status: 'running',
        text: '',
        toolCalls: [],
        latencyMs: undefined,
      }
      run.steps.push(view)
      const started = Date.now()
      controller = new AbortController()

      const body: WireCallRequest = {
        provider: def.provider,
        model: def.model,
        api_key: def.api_key || null,
        base_url: def.base_url || null,
        stream: true,
        mock: def.mock,
        options: {
          temperature: def.temperature,
          max_output_tokens: 2048,
          tools: def.tools.map((t) => toolSchemas.get(t)).filter(Boolean) as never[],
        },
        session_id: def.session_id,
        step,
        messages: run.messages,
      }

      try {
        for await (const ev of callStream(body, controller.signal)) {
          if (ev.event === 'stream_part') {
            const part: StreamPart = parseStreamPart(ev.data)
            applyPart(view, part)
          } else if (ev.event === 'meta') {
            view.meta = JSON.parse(ev.data)
          } else if (ev.event === 'error') {
            throw new Error(ev.data)
          }
        }
      } catch (e) {
        if (controller.signal.aborted) {
          view.status = 'stopped'
          run.status = 'stopped'
          return
        }
        throw e
      }

      view.latencyMs = Date.now() - started
      view.status = 'done'

      // No tool calls → the agent is done.
      if (!view.toolCalls.length) {
        run.status = 'done'
        return
      }

      // Execute tools and feed results back for the next step.
      const assistantParts: any[] = []
      if (view.text.trim()) assistantParts.push({ type: 'text', text: view.text })
      for (const tc of view.toolCalls) {
        assistantParts.push({
          type: 'tool_call',
          tool_call_id: tc.id,
          tool_name: tc.name,
          input: tc.input,
        })
      }
      run.messages.push({ role: 'assistant', content: assistantParts })

      const toolResults: any[] = []
      for (const tc of view.toolCalls) {
        tc.executing = true
        try {
          const out = await api.executeTool(tc.name, tc.input)
          tc.result = out.result
          tc.is_error = false
          toolResults.push({
            type: 'tool_result',
            tool_call_id: tc.id,
            result: out.result,
            is_error: false,
          })
        } catch (e) {
          tc.result = String(e)
          tc.is_error = true
          toolResults.push({
            type: 'tool_result',
            tool_call_id: tc.id,
            result: String(e),
            is_error: true,
          })
        } finally {
          tc.executing = false
        }
      }
      run.messages.push({ role: 'tool', content: toolResults })
    }
    if (run.status === 'running') run.status = 'done'
  } catch (e) {
    run.status = 'error'
    const last = run.steps[run.steps.length - 1]
    if (last) last.error = String(e)
  }
}

function applyPart(view: AgentStepView, part: StreamPart): void {
  if ('TextDelta' in part) {
    view.text += part.TextDelta.delta
  } else if ('ToolCall' in part) {
    view.toolCalls.push({
      id: part.ToolCall.tool_call_id,
      name: part.ToolCall.tool_name,
      input: part.ToolCall.input,
    })
  } else if ('ToolInputStart' in part) {
    // Streamed tool input — collect into a pending call (rare path).
    view.toolCalls.push({ id: part.ToolInputStart.id, name: part.ToolInputStart.tool_name, input: '' })
  } else if ('ToolInputDelta' in part) {
    const pending = view.toolCalls.find((t) => t.id === part.ToolInputDelta.id)
    if (pending && typeof pending.input === 'string') {
      pending.input += part.ToolInputDelta.delta
      // Best-effort parse of accumulated JSON arguments.
      try {
        pending.input = JSON.parse(pending.input)
      } catch {
        /* keep accumulating */
      }
    }
  } else if ('Finish' in part) {
    view.usage = part.Finish.usage
  }
}
