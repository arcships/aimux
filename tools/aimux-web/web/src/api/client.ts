// API client — typed wrappers for the console endpoints (RFC-0029 §5).

import type { WireCallRequest } from '../types/WireCallRequest'
import type { WireCallResponse } from '../types/WireCallResponse'
import type { Recording } from '../types/Recording'
import type { TraceRecord } from '../types/TraceRecord'
import type { SessionView } from '../types/SessionView'
import type { StreamPart } from '../types/StreamPart'

const API = '/api'

async function parseError(res: Response): Promise<Error> {
  let msg = `HTTP ${res.status}`
  try {
    const j = await res.json()
    if (j.error) msg = String(j.error)
  } catch {
    /* keep fallback */
  }
  return new Error(msg)
}

async function j<T>(res: Response): Promise<T> {
  if (!res.ok) throw await parseError(res)
  return res.json() as Promise<T>
}

// ── SSE ─────────────────────────────────────────────────────────────────────

export interface SSEEvent {
  event: string
  data: string
}

/** POST /api/calls with streaming; yields SSE events (`stream_part` / `meta` / `error`). */
export async function* callStream(
  body: WireCallRequest,
  signal?: AbortSignal,
): AsyncGenerator<SSEEvent, void, unknown> {
  const res = await fetch(`${API}/calls`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
    signal,
  })
  if (!res.ok || !res.body) throw await parseError(res)

  const reader = res.body.getReader()
  const decoder = new TextDecoder()
  let buf = ''
  try {
    for (;;) {
      const { done, value } = await reader.read()
      if (done) break
      buf += decoder.decode(value, { stream: true })
      let idx: number
      while ((idx = buf.indexOf('\n\n')) >= 0) {
        const ev = parseSSE(buf.slice(0, idx))
        buf = buf.slice(idx + 2)
        if (ev) yield ev
      }
    }
    if (buf.trim()) {
      const ev = parseSSE(buf)
      if (ev) yield ev
    }
  } finally {
    reader.releaseLock()
  }
}

function parseSSE(raw: string): SSEEvent | null {
  let event = 'message'
  const data: string[] = []
  for (const line of raw.split('\n')) {
    if (line.startsWith('event:')) event = line.slice(6).trim()
    else if (line.startsWith('data:')) data.push(line.slice(5).trim())
  }
  if (!data.length) return null
  return { event, data: data.join('\n') }
}

// ── typed endpoints ─────────────────────────────────────────────────────────

export const api = {
  health: () => fetch(`${API}/health`).then((r) => r.text()),

  call(body: WireCallRequest): Promise<WireCallResponse> {
    return fetch(`${API}/calls`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    }).then((r) => j<WireCallResponse>(r))
  },

  providers(): Promise<{ providers: string[]; suggested_models: Record<string, string[]> }> {
    return fetch(`${API}/providers`).then((r) => j(r))
  },

  tools(): Promise<{ tools: Array<{ name: string; description?: string | null; parameters: unknown }> }> {
    return fetch(`${API}/tools`).then((r) => j(r))
  },

  executeTool(name: string, input: unknown): Promise<{ tool: string; result: unknown }> {
    return fetch(`${API}/tools/${encodeURIComponent(name)}`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(input),
    }).then((r) => j(r))
  },

  traces(params: { provider?: string; session?: string; status?: string; limit?: number } = {}): Promise<Recording[]> {
    const q = new URLSearchParams()
    if (params.provider) q.set('provider', params.provider)
    if (params.session) q.set('session', params.session)
    if (params.status) q.set('status', params.status)
    if (params.limit) q.set('limit', String(params.limit))
    const qs = q.toString()
    return fetch(`${API}/traces${qs ? `?${qs}` : ''}`).then((r) => j<Recording[]>(r))
  },

  trace(callId: string): Promise<Recording> {
    return fetch(`${API}/traces/${encodeURIComponent(callId)}`).then((r) => j<Recording>(r))
  },

  traceRecords(): Promise<TraceRecord[]> {
    return fetch(`${API}/trace-records`).then((r) => j<TraceRecord[]>(r))
  },

  exportJsonl(): Promise<string> {
    return fetch(`${API}/recordings/export`).then((r) => r.text())
  },

  importJsonl(jsonl: string): Promise<{ imported: number }> {
    return fetch(`${API}/recordings/import`, {
      method: 'POST',
      headers: { 'Content-Type': 'text/plain' },
      body: jsonl,
    }).then((r) => j(r))
  },

  sessions(): Promise<SessionView[]> {
    return fetch(`${API}/sessions`).then((r) => j<SessionView[]>(r))
  },

  sessionDetail(id: string): Promise<{ session_id: string; calls: unknown[]; recordings: Recording[] }> {
    return fetch(`${API}/sessions/${encodeURIComponent(id)}`).then((r) => j(r))
  },

  replay(body: {
    call_id: string
    api_key?: string | null
    overrides?: { messages?: unknown[]; temperature?: number | null; max_output_tokens?: number | null } | null
  }): Promise<WireCallResponse & { call_id: string; tool_calls?: unknown; meta?: unknown }> {
    return fetch(`${API}/replay`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    }).then((r) => j(r))
  },

  mockLoad(jsonl: string): Promise<{ loaded: boolean; provider: string; model: string }> {
    return fetch(`${API}/mock/load`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ jsonl }),
    }).then((r) => j(r))
  },

  cacheProbe(body: {
    provider: string
    model: string
    api_key?: string | null
    base_url?: string | null
    max_requests?: number
    prompt?: string | null
    dry_run?: boolean
  }): Promise<unknown> {
    return fetch(`${API}/cache-probe`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    }).then((r) => j(r))
  },
}

/** Parse a `stream_part` JSON payload into a StreamPart (best-effort). */
export function parseStreamPart(json: string): StreamPart {
  return JSON.parse(json) as StreamPart
}
