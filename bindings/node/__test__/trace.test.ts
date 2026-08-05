// trace.test.ts — RFC-0015 cache probing through the Node binding.
//
// Verifies the full downstream path: `trace()` wraps a model, probed calls
// record fingerprints/verdicts, and the query API returns typed results.
// Uses a mock HTTP server — no real API calls.

import test from 'ava'
import { createServer, type Server } from 'node:http'

import {
  openai,
  generateText,
  type GenerateTextResult,
} from '../src/index.ts'

const MOCK_RESPONSE = JSON.stringify({
  id: 'chatcmpl-trace',
  model: 'gpt-4o-mini',
  choices: [
    {
      message: { role: 'assistant', content: 'ok' },
      finish_reason: 'stop',
    },
  ],
  usage: {
    prompt_tokens: 200,
    completion_tokens: 5,
    total_tokens: 205,
    prompt_tokens_details: { cached_tokens: 128 },
  },
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

test('trace() records calls and query API returns typed results', async (t) => {
  const { server, url } = await startMockServer()
  t.teardown(() => server.close())

  const raw = await openai('sk-test-fake-key', 'gpt-4o-mini', { baseUrl: url })
  const traced = raw.traceAudited(true) // strict mode

  // > 4 KiB user message so block-aligned prefixes actually match.
  const big = 'x'.repeat(5000)
  await generateText(traced, [{ role: 'user', content: big }], { session_id: 'sess-1' })
  await generateText(
    traced,
    [
      { role: 'user', content: big },
      { role: 'assistant', content: 'a1' },
      { role: 'user', content: 'u2' },
    ],
    { session_id: 'sess-1' },
  )

  // Untraced models reject the query API.
  t.throws(() => (raw as any).traceAggregate(), { message: /not traced/ })

  const statsJson = traced.traceAggregate()
  const stats = JSON.parse(statsJson) as any[]
  t.is(stats.length, 1)
  t.is(stats[0].provider, 'openai')
  t.is(stats[0].requests, 2)
  t.truthy(stats[0].reported_hit_rate)
  t.truthy(stats[0].client_upper_bound_hit_rate)
  t.truthy(stats[0].verdict_counts)

  const chain = JSON.parse(traced.traceSessionChain('sess-1')) as any
  t.is(chain.record_ids.length, 2)
  t.truthy(chain.prefix_stability)

  const jsonl = traced.traceExportJsonl()
  const lines = jsonl.trim().split('\n')
  t.is(lines.length, 2, 'one TraceRecord per line')
  const first = JSON.parse(lines[0])
  t.truthy(first.fingerprint.body_hash)
  t.is(first.session_id, 'sess-1')

  traced.traceClear()
  t.is(traced.traceExportJsonl().trim(), '')
})

test('non-traced model still generates normally', async (t) => {
  const { server, url } = await startMockServer()
  t.teardown(() => server.close())

  const model = await openai('sk-test-fake-key', 'gpt-4o-mini', { baseUrl: url })
  const result: GenerateTextResult = await generateText(model, 'hello')
  t.truthy(result.text)
  t.is(result.usage.input_tokens.total, 200)
})
