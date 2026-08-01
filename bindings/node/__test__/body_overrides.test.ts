// body_overrides.test.ts — Node e2e tests for RFC-0017 phase 1: bodyOverrides
// (JSON deep-merge) and provider factory config (headers/maxRetries/bodyOverrides).
//
// These verify the full chain: JS → napi → Rust → HTTP mock, asserting the
// outbound request body carries the user's overrides.

import test from 'ava'
import { createServer, type Server } from 'node:http'
import { openai, deepseek } from '../index.js'

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

// ── per-call bodyOverrides ───────────────────────────────────────────────────

test('per-call bodyOverrides injects a field into the request body', async (t) => {
  let requestBody: any = null
  const { server, url } = await startMockServer(async (req, res) => {
    requestBody = await readBody(req)
    res.writeHead(200, { 'content-type': 'application/json' })
    res.end(CHAT_RESPONSE)
  })

  try {
    const model = await openai('test-key', 'gpt-4o', url)
    const opts = JSON.stringify({
      body_overrides: { enable_thinking: false },
    })
    await model.generateText(JSON.stringify('Hello'), opts)

    t.is(requestBody.enable_thinking, false)
    t.is(requestBody.model, 'gpt-4o')
  } finally {
    await closeServer(server)
  }
})

test('per-call bodyOverrides overwrites a standard field', async (t) => {
  let requestBody: any = null
  const { server, url } = await startMockServer(async (req, res) => {
    requestBody = await readBody(req)
    res.writeHead(200, { 'content-type': 'application/json' })
    res.end(CHAT_RESPONSE)
  })

  try {
    const model = await openai('test-key', 'gpt-4o', url)
    // temperature set via standard option, overridden by body_overrides
    const opts = JSON.stringify({
      temperature: 0.9,
      body_overrides: { temperature: 0.1 },
    })
    await model.generateText(JSON.stringify('Hello'), opts)

    t.is(requestBody.temperature, 0.1)
  } finally {
    await closeServer(server)
  }
})

test('per-call bodyOverrides null deletes a field', async (t) => {
  let requestBody: any = null
  const { server, url } = await startMockServer(async (req, res) => {
    requestBody = await readBody(req)
    res.writeHead(200, { 'content-type': 'application/json' })
    res.end(CHAT_RESPONSE)
  })

  try {
    const model = await openai('test-key', 'gpt-4o', url)
    const opts = JSON.stringify({
      temperature: 0.5,
      body_overrides: { temperature: null },
    })
    await model.generateText(JSON.stringify('Hello'), opts)

    t.true(requestBody.temperature === undefined, 'temperature should be deleted by null')
  } finally {
    await closeServer(server)
  }
})

// ── provider-level bodyOverrides (factory config) ────────────────────────────

test('factory config bodyOverrides are merged into every request', async (t) => {
  let requestBody: any = null
  const { server, url } = await startMockServer(async (req, res) => {
    requestBody = await readBody(req)
    res.writeHead(200, { 'content-type': 'application/json' })
    res.end(CHAT_RESPONSE)
  })

  try {
    const model = await openai('test-key', 'gpt-4o', {
      baseUrl: url,
      bodyOverrides: '{"X-Relay-Tag":"my-team"}',
    })
    await model.generateText(JSON.stringify('Hello'))

    t.is(requestBody['X-Relay-Tag'], 'my-team')
  } finally {
    await closeServer(server)
  }
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

// ── per-call overrides provider-level ────────────────────────────────────────

test('per-call bodyOverrides take precedence over provider-level', async (t) => {
  let requestBody: any = null
  const { server, url } = await startMockServer(async (req, res) => {
    requestBody = await readBody(req)
    res.writeHead(200, { 'content-type': 'application/json' })
    res.end(CHAT_RESPONSE)
  })

  try {
    const model = await openai('test-key', 'gpt-4o', {
      baseUrl: url,
      bodyOverrides: '{"custom_field":"provider"}',
    })
    const opts = JSON.stringify({
      body_overrides: { custom_field: 'call' },
    })
    await model.generateText(JSON.stringify('Hello'), opts)

    t.is(requestBody.custom_field, 'call', 'per-call should override provider-level')
  } finally {
    await closeServer(server)
  }
})
