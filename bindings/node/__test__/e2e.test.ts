// e2e.test.ts — End-to-end provider tests with mock HTTP server.
//
// These tests verify the FULL chain: Node.js → napi-rs → Rust engine →
// HTTP mock server → response parsing → typed result.
//
// The mock responses are taken from real provider API responses
// (same data shape as the Rust e2e_test.rs wiremock mocks).

import test from 'ava'
import { createServer, type Server } from 'node:http'
import { openai, anthropic } from '../index.js'

// ── Mock server helpers ─────────────────────────────────────────────────────

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

// ── OpenAI mock responses (from real API shape) ─────────────────────────────

const OPENAI_CHAT_RESPONSE = JSON.stringify({
  id: 'chatcmpl-test',
  model: 'gpt-4o',
  choices: [{
    message: { role: 'assistant', content: 'Rust is a systems programming language.' },
    finish_reason: 'stop',
  }],
  usage: { prompt_tokens: 10, completion_tokens: 8, total_tokens: 18 },
})

const OPENAI_STREAM_BODY = [
  'data: {"id":"1","model":"gpt-4o","choices":[{"delta":{"content":"Hello"}}]}\n\n',
  'data: {"id":"1","model":"gpt-4o","choices":[{"delta":{"content":" world"}}]}\n\n',
  'data: {"id":"1","model":"gpt-4o","choices":[{"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":5,"completion_tokens":2,"total_tokens":7}}\n\n',
  'data: [DONE]\n\n',
].join('')

// ── Anthropic mock responses ────────────────────────────────────────────────

const ANTHROPIC_MESSAGE_RESPONSE = JSON.stringify({
  id: 'msg_test',
  type: 'message',
  role: 'assistant',
  model: 'claude-3-5-sonnet-20241022',
  content: [{ type: 'text', text: 'Hello from Claude!' }],
  stop_reason: 'end_turn',
  usage: { input_tokens: 10, output_tokens: 5 },
})

const ANTHROPIC_STREAM_BODY = [
  'event: message_start\ndata: {"type":"message_start","message":{"id":"msg_1","model":"claude-3-5-sonnet-20241022","usage":{"input_tokens":10,"output_tokens":0}}}\n\n',
  'event: content_block_start\ndata: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}\n\n',
  'event: content_block_delta\ndata: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}\n\n',
  'event: content_block_delta\ndata: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":" from Claude"}}\n\n',
  'event: content_block_stop\ndata: {"type":"content_block_stop","index":0}\n\n',
  'event: message_delta\ndata: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":5}}\n\n',
  'event: message_stop\ndata: {"type":"message_stop"}\n\n',
].join('')

// ── Tests ───────────────────────────────────────────────────────────────────

test('e2e: OpenAI generateText via mock server', async (t) => {
  const { server, url } = await startMockServer((req, res) => {
    res.writeHead(200, { 'content-type': 'application/json' })
    res.end(OPENAI_CHAT_RESPONSE)
  })

  try {
    const model = await openai('test-key', 'gpt-4o', url)
    const resultJson = await model.generateText(JSON.stringify('What is Rust?'))
    const r = JSON.parse(resultJson)

    t.is(r.text, 'Rust is a systems programming language.')
    t.truthy(r.usage)
    t.truthy(r.finish_reason)
  } finally {
    await closeServer(server)
  }
})

test('e2e: OpenAI streamText via mock server', async (t) => {
  const { server, url } = await startMockServer((req, res) => {
    res.writeHead(200, { 'content-type': 'text/event-stream' })
    res.end(OPENAI_STREAM_BODY)
  })

  try {
    const model = await openai('test-key', 'gpt-4o', url)
    const parts: any[] = []

    for await (const json of await model.streamText(JSON.stringify('Say hello'))) {
      parts.push(JSON.parse(json))
    }

    t.true(parts.length > 0)

    const text = parts
      .filter((p) => p.TextDelta)
      .map((p) => p.TextDelta.delta)
      .join('')

    t.is(text, 'Hello world')
  } finally {
    await closeServer(server)
  }
})

test('e2e: Anthropic generateText via mock server', async (t) => {
  const { server, url } = await startMockServer((req, res) => {
    res.writeHead(200, { 'content-type': 'application/json' })
    res.end(ANTHROPIC_MESSAGE_RESPONSE)
  })

  try {
    const model = await anthropic('test-key', 'claude-3-5-sonnet-20241022', url)
    const resultJson = await model.generateText(JSON.stringify('Hello'))
    const r = JSON.parse(resultJson)

    t.is(r.text, 'Hello from Claude!')
    t.truthy(r.usage)
  } finally {
    await closeServer(server)
  }
})

test('e2e: Anthropic streamText via mock server', async (t) => {
  const { server, url } = await startMockServer((req, res) => {
    res.writeHead(200, { 'content-type': 'text/event-stream' })
    res.end(ANTHROPIC_STREAM_BODY)
  })

  try {
    const model = await anthropic('test-key', 'claude-3-5-sonnet-20241022', url)
    const parts: any[] = []
    for await (const json of await model.streamText(JSON.stringify('Hello'))) {
      parts.push(JSON.parse(json))
    }

    t.true(parts.length > 0)

    const text = parts
      .filter((p) => p.TextDelta)
      .map((p) => p.TextDelta.delta)
      .join('')

    t.is(text, 'Hello from Claude')
  } finally {
    await closeServer(server)
  }
})

test('e2e: OpenAI generateText with options (max_tokens, temperature)', async (t) => {
  let receivedBody: any = null
  const { server, url } = await startMockServer((req, res) => {
    let body = ''
    req.on('data', (chunk: Buffer) => (body += chunk))
    req.on('end', () => {
      receivedBody = JSON.parse(body)
      res.writeHead(200, { 'content-type': 'application/json' })
      res.end(OPENAI_CHAT_RESPONSE)
    })
  })

  try {
    const model = await openai('test-key', 'gpt-4o', url)
    const opts = JSON.stringify({ max_output_tokens: 100, temperature: 0.5 })
    await model.generateText(JSON.stringify('Hello'), opts)

    // Verify the options were passed through to the HTTP request
    t.is(receivedBody.max_tokens, 100)
    t.is(receivedBody.temperature, 0.5)
  } finally {
    await closeServer(server)
  }
})
