// provider_config.test.ts — Node e2e tests for the provider factory config
// (headers) and the call-level maxRetries, plus the removed bodyOverrides and
// the provider-level maxRetries, which are rejected instead of ignored.
//
// These verify the full chain: JS → napi → Rust → HTTP mock.

import test from 'ava'
import { createServer, type Server } from 'node:http'
import { openai } from '../src/native.ts'

function startMockServer(handler: (req: any, res: any) => void): Promise<{ server: Server; url: string }> {
  return new Promise((resolve) => {
    const server = createServer(handler)
    server.listen(0, '127.0.0.1', () => {
      const addr = server.address() as any
      resolve({ server, url: `http://127.0.0.1:${addr.port}` })
    })
  })
}

function closeServer(server: Server): Promise<void> {
  return new Promise((resolve) => server.close(() => resolve()))
}

function readBody(req: any): Promise<any> {
  return new Promise((resolve) => {
    let body = ''
    req.on('data', (chunk: string) => { body += chunk })
    req.on('end', () => resolve(JSON.parse(body)))
  })
}

const CHAT_RESPONSE = JSON.stringify({
  id: 'chatcmpl-test',
  model: 'gpt-4o',
  choices: [{
    message: { role: 'assistant', content: 'Done.' },
    finish_reason: 'stop',
  }],
  usage: { prompt_tokens: 10, completion_tokens: 1, total_tokens: 11 },
})

// ── removed: bodyOverrides ───────────────────────────────────────────────────

test('per-call body_overrides no longer reaches the request body', async (t) => {
  let requestBody: any = null
  const { server, url } = await startMockServer(async (req, res) => {
    requestBody = await readBody(req)
    res.writeHead(200, { 'content-type': 'application/json' })
    res.end(CHAT_RESPONSE)
  })

  try {
    const model = await openai('test-key', 'gpt-4o', url)
    const opts = JSON.stringify({
      temperature: 0.9,
      body_overrides: { temperature: 0.1, enable_thinking: false },
    })
    await model.generateText(JSON.stringify('Hello'), opts)

    t.is(requestBody.temperature, 0.9, 'the standard option stands')
    t.true(requestBody.enable_thinking === undefined, 'the override is not applied')
  } finally {
    await closeServer(server)
  }
})

test('factory config bodyOverrides is rejected, not ignored', async (t) => {
  const err = await t.throwsAsync(() =>
    openai('test-key', 'gpt-4o', {
      baseUrl: 'http://127.0.0.1:1',
      bodyOverrides: '{"X-Relay-Tag":"my-team"}',
    }),
  )
  t.true(err instanceof Error)
  t.regex(err!.message, /body_overrides/)
})

test('factory config maxRetries is rejected, not ignored', async (t) => {
  const err = await t.throwsAsync(() =>
    openai('test-key', 'gpt-4o', { baseUrl: 'http://127.0.0.1:1', maxRetries: 0 }),
  )
  t.true(err instanceof Error)
  t.regex(err!.message, /max_retries/)
})

test('factory config headers are sent on every request', async (t) => {
  let headers: any = null
  const { server, url } = await startMockServer(async (req, res) => {
    headers = req.headers
    // consume body
    await readBody(req)
    res.writeHead(200, { 'content-type': 'application/json' })
    res.end(CHAT_RESPONSE)
  })

  try {
    const model = await openai('test-key', 'gpt-4o', {
      baseUrl: url,
      headers: JSON.stringify({ 'X-Custom-Header': 'custom-value' }),
    })
    await model.generateText(JSON.stringify('Hello'))

    t.is(headers['x-custom-header'], 'custom-value')
  } finally {
    await closeServer(server)
  }
})

// ── backward compatibility ───────────────────────────────────────────────────

test('factory accepts bare string baseUrl (backward compatible)', async (t) => {
  const { server, url } = await startMockServer(async (req, res) => {
    await readBody(req)
    res.writeHead(200, { 'content-type': 'application/json' })
    res.end(CHAT_RESPONSE)
  })

  try {
    // 3rd param is a plain string (old API)
    const model = await openai('test-key', 'gpt-4o', url)
    const result = JSON.parse(await model.generateText(JSON.stringify('Hello')))
    t.is(result.text, 'Done.')
  } finally {
    await closeServer(server)
  }
})

// ── maxRetries ────────────────────────────────────────────────────────────────

test('maxRetries: 0 disables retries (single request on 500)', async (t) => {
  let requestCount = 0
  const { server, url } = await startMockServer(async (req, res) => {
    requestCount++
    await readBody(req)
    res.writeHead(500, { 'content-type': 'application/json' })
    res.end(JSON.stringify({ error: { message: 'Internal server error', type: 'server_error' } }))
  })

  try {
    const model = await openai('test-key', 'gpt-4o', url)
    const opts = JSON.stringify({ max_retries: 0 })

    await t.throwsAsync(
      async () => model.generateText(JSON.stringify('Hello'), opts),
      { message: /500|Internal|server_error/i },
    )

    // With retries disabled, exactly 1 request should have been made.
    t.is(requestCount, 1, 'should make exactly 1 request when maxRetries=0')
  } finally {
    await closeServer(server)
  }
})
