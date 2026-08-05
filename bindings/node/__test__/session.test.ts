// session.test.ts — RFC-0024 session grouping through the Node binding.
//
// Verifies the full downstream path: `sessionId` in typed options reaches the
// core, calls are grouped in the registered SessionStore, and the query API
// (`getSessionCalls` / `getSessions`) returns typed results. Uses a mock HTTP
// server — no real API calls. The optional inferer is exercised as well.

import test from 'ava'
import { createServer, type Server } from 'node:http'

import {
  openai,
  generateText,
  initSessionStore,
  initSessionInfer,
  getSessionCalls,
  getSessions,
  type SessionCall,
  type SessionView,
} from '../src/index.ts'

const MOCK_RESPONSE = JSON.stringify({
  id: 'chatcmpl-sess',
  model: 'gpt-4o',
  choices: [
    {
      message: { role: 'assistant', content: 'ok' },
      finish_reason: 'stop',
    },
  ],
  usage: { prompt_tokens: 3, completion_tokens: 2, total_tokens: 5 },
})

function startMockServer(): Promise<{ server: Server; url: string }> {
  return new Promise((resolve) => {
    const server = createServer((_req, res) => {
      res.writeHead(200, { 'content-type': 'application/json' })
      res.end(MOCK_RESPONSE)
    })
    server.listen(0, '127.0.0.1', () => {
      const addr = server.address() as any
      resolve({ server, url: `http://127.0.0.1:${addr.port}` })
    })
  })
}

// NOTE: both tests mutate the process-wide session store, so they must run
// serially (ava's default is concurrent).

test.serial('sessionId groups calls and query API returns typed results', async (t) => {
  const { server, url } = await startMockServer()
  t.teardown(() => server.close())

  initSessionStore()
  initSessionInfer(false) // explicit ids only

  const model = await openai('sk-test-fake-key', 'gpt-4o-mini', { baseUrl: url })

  // Two calls in the same session.
  await generateText(model, 'first', { session_id: 'sess-1' })
  await generateText(model, 'second', { session_id: 'sess-1' })

  const calls: SessionCall[] = getSessionCalls('sess-1')
  t.is(calls.length, 2)
  t.is(calls[0].step, 0)
  t.is(calls[1].step, 1)
  t.truthy(calls[0].trace_id)
  t.truthy(calls[0].recorded_at)
  t.not(calls[0].trace_id, calls[1].trace_id)

  // A call without a sessionId (inference off) is not grouped.
  await generateText(model, 'third')
  const sessions: SessionView[] = getSessions()
  t.is(sessions.length, 1)
  t.is(sessions[0].session_id, 'sess-1')
  t.is(sessions[0].source, 'Explicit')

  // Unknown session → empty.
  t.deepEqual(getSessionCalls('nope'), [])

  // Separate explicit session.
  await generateText(model, 'other', { session_id: 'sess-2' })
  t.is(getSessions().length, 2)
  t.is(getSessionCalls('sess-2').length, 1)
})

test.serial('opt-in inferer groups prefix continuations into auto sessions', async (t) => {
  const { server, url } = await startMockServer()
  t.teardown(() => server.close())

  initSessionStore()
  initSessionInfer(true)

  const model = await openai('sk-test-fake-key', 'gpt-4o-mini', { baseUrl: url })

  await generateText(model, 'u1')
  await generateText(model, [{ role: 'user', content: 'u1' }, { role: 'assistant', content: 'a1' }, { role: 'user', content: 'u2' }])

  const sessions = getSessions()
  const autos = sessions.filter((s) => s.session_id.startsWith('auto-'))
  t.is(autos.length, 1, 'prefix continuation stays in one auto session')
  t.is(autos[0].source, 'Inferred')
  t.is(autos[0].calls.length, 2)
})
