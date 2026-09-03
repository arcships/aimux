// e2e.test.ts — End-to-end provider tests with mock HTTP server.
//
// These tests verify the FULL chain: Node.js → napi-rs → Rust engine →
// HTTP mock server → response parsing → typed result.
//
// The mock responses are taken from real provider API responses
// (same data shape as the Rust e2e_test.rs wiremock mocks).

import test from 'ava'
import { createServer, type Server } from 'node:http'
import { openai, anthropic } from '../src/native.ts'

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

// ── Tool-call parsing ───────────────────────────────────────────────────────

const OPENAI_TOOL_CALL_RESPONSE = JSON.stringify({
  id: 'chatcmpl-tc',
  model: 'gpt-4o',
  choices: [{
    message: {
      role: 'assistant',
      content: null,
      tool_calls: [{
        id: 'call_abc',
        type: 'function',
        function: { name: 'get_weather', arguments: '{"location":"Tokyo"}' },
      }],
    },
    finish_reason: 'tool_calls',
  }],
  usage: { prompt_tokens: 20, completion_tokens: 10, total_tokens: 30 },
})

test('e2e: OpenAI generateText parses tool_calls (structured content)', async (t) => {
  const { server, url } = await startMockServer((req, res) => {
    res.writeHead(200, { 'content-type': 'application/json' })
    res.end(OPENAI_TOOL_CALL_RESPONSE)
  })

  try {
    const model = await openai('test-key', 'gpt-4o', url)
    const opts = JSON.stringify({
      tools: [{
        type: 'function',
        name: 'get_weather',
        description: 'Get weather for a location',
        input_schema: {
          type: 'object',
          properties: { location: { type: 'string' } },
          required: ['location'],
        },
      }],
    })
    const r = JSON.parse(await model.generateText(JSON.stringify("What's the weather in Tokyo?"), opts))

    // Convenience field: tool_calls extracted
    t.is(r.tool_calls.length, 1)
    t.is(r.tool_calls[0].tool_name, 'get_weather')
    t.is(r.tool_calls[0].tool_call_id, 'call_abc')
    t.deepEqual(r.tool_calls[0].input, { location: 'Tokyo' })

    // Structured content: raw.content contains the ToolCall variant
    t.truthy(r.raw)
    t.true(Array.isArray(r.raw.content))
    const tc = r.raw.content.find((c: any) => c.ToolCall)
    t.truthy(tc, 'raw.content contains a ToolCall variant')
    t.is(tc.ToolCall.tool_name, 'get_weather')
    t.is(tc.ToolCall.tool_call_id, 'call_abc')
    // raw content keeps the provider's argument text; the parsed object
    // lives on the top-level toolCalls.
    t.is(tc.ToolCall.input, '{"location":"Tokyo"}')
  } finally {
    await closeServer(server)
  }
})

// ── Multi-role messages ────────────────────────────────────────────────────

test('e2e: OpenAI generateText with multi-role messages (system + user)', async (t) => {
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
    const prompt = JSON.stringify([
      { role: 'system', content: 'You are a helpful assistant.' },
      { role: 'user', content: 'What is Rust?' },
    ])
    const r = JSON.parse(await model.generateText(prompt))

    // The full multi-role message sequence reaches the provider
    t.true(Array.isArray(receivedBody.messages))
    t.is(receivedBody.messages.length, 2)
    t.is(receivedBody.messages[0].role, 'system')
    t.is(receivedBody.messages[0].content, 'You are a helpful assistant.')
    t.is(receivedBody.messages[1].role, 'user')
    t.is(receivedBody.messages[1].content, 'What is Rust?')
    t.truthy(r.text)
  } finally {
    await closeServer(server)
  }
})

// ── ToolChoice ──────────────────────────────────────────────────────────────

test('e2e: OpenAI generateText with tool_choice: required', async (t) => {
  let receivedBody: any = null
  const { server, url } = await startMockServer((req, res) => {
    let body = ''
    req.on('data', (chunk: Buffer) => (body += chunk))
    req.on('end', () => {
      receivedBody = JSON.parse(body)
      res.writeHead(200, { 'content-type': 'application/json' })
      res.end(OPENAI_TOOL_CALL_RESPONSE)
    })
  })

  try {
    const model = await openai('test-key', 'gpt-4o', url)
    const opts = JSON.stringify({
      tools: [{
        type: 'function',
        name: 'get_weather',
        input_schema: { type: 'object', properties: { location: { type: 'string' } } },
      }],
      tool_choice: 'required',
    })
    await model.generateText(JSON.stringify('Hello'), opts)

    // tool_choice reaches the provider request body as "required"
    t.is(receivedBody.tool_choice, 'required')
  } finally {
    await closeServer(server)
  }
})

// ── Streaming tool calls ────────────────────────────────────────────────────

const OPENAI_STREAM_TOOL_BODY = [
  'data: {"id":"1","model":"gpt-4o","choices":[{"delta":{"role":"assistant","tool_calls":[{"index":0,"id":"call_xyz","type":"function","function":{"name":"get_weather","arguments":""}}]}}]}\n\n',
  'data: {"id":"1","model":"gpt-4o","choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\\"location\\":\\"Tokyo\\"}"}}]}}]}\n\n',
  'data: {"id":"1","model":"gpt-4o","choices":[{"delta":{},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":5,"completion_tokens":2,"total_tokens":7}}\n\n',
  'data: [DONE]\n\n',
].join('')

test('e2e: OpenAI streamText parses tool-call stream parts', async (t) => {
  const { server, url } = await startMockServer((req, res) => {
    res.writeHead(200, { 'content-type': 'text/event-stream' })
    res.end(OPENAI_STREAM_TOOL_BODY)
  })

  try {
    const model = await openai('test-key', 'gpt-4o', url)
    const opts = JSON.stringify({
      tools: [{
        type: 'function',
        name: 'get_weather',
        input_schema: { type: 'object', properties: { location: { type: 'string' } } },
      }],
    })
    const parts: any[] = []
    for await (const json of await model.streamText(JSON.stringify("What's the weather?"), opts)) {
      parts.push(JSON.parse(json))
    }

    // The stream must contain a ToolCall or ToolInputDelta part (not just TextDelta/Finish)
    const hasToolPart = parts.some(
      (p) => p.ToolCall || p.ToolInputDelta || p.ToolInputStart,
    )
    t.true(hasToolPart, 'stream contained a tool-related StreamPart')

    // A complete ToolCall part should carry the parsed tool name + input
    const toolCall = parts.find((p) => p.ToolCall)
    if (toolCall) {
      t.is(toolCall.ToolCall.tool_name, 'get_weather')
    }
  } finally {
    await closeServer(server)
  }
})

// ── Tool-call full round-trip ───────────────────────────────────────────────

test('e2e: OpenAI tool-call full round-trip (ToolCall → ToolResult → final text)', async (t) => {
  // Two calls: the mock returns tool_calls on the first, final text on the second.
  let callCount = 0
  let secondRequestBody: any = null
  const { server, url } = await startMockServer((req, res) => {
    let body = ''
    req.on('data', (chunk: Buffer) => (body += chunk))
    req.on('end', () => {
      callCount++
      if (callCount === 2) secondRequestBody = JSON.parse(body)
      // First call → tool_calls; second call → final text.
      res.writeHead(200, { 'content-type': 'application/json' })
      res.end(callCount === 1 ? OPENAI_TOOL_CALL_RESPONSE : OPENAI_CHAT_RESPONSE)
    })
  })

  try {
    const model = await openai('test-key', 'gpt-4o', url)
    const opts = JSON.stringify({
      tools: [{
        type: 'function',
        name: 'get_weather',
        description: 'Get weather for a location',
        input_schema: {
          type: 'object',
          properties: { location: { type: 'string' } },
          required: ['location'],
        },
      }],
    })

    // Step 1: first call — model requests a tool call.
    const r1 = JSON.parse(await model.generateText(JSON.stringify("What's the weather in Tokyo?"), opts))
    t.is(r1.tool_calls[0].tool_name, 'get_weather')
    t.is(r1.tool_calls[0].tool_call_id, 'call_abc')

    // Step 2: user "executes" the tool, then builds the full conversation:
    //   user → assistant(tool_call) → tool(result)
    // Input uses the engine's ContentPart variants (tool_call / tool_result);
    // the engine converts these to the OpenAI wire format on the outbound request.
    const messages = [
      { role: 'user', content: "What's the weather in Tokyo?" },
      {
        role: 'assistant',
        content: [{
          type: 'tool_call',
          tool_call_id: 'call_abc',
          tool_name: 'get_weather',
          input: { location: 'Tokyo' },
        }],
      },
      {
        role: 'tool',
        content: [{
          type: 'tool_result',
          tool_call_id: 'call_abc',
          result: { temperature: 22, condition: 'sunny' },
        }],
      },
    ]

    // Step 3: second call — model returns final text after the tool result.
    const r2 = JSON.parse(await model.generateText(JSON.stringify(messages), opts))
    t.is(r2.text, 'Rust is a systems programming language.')

    // Step 4: verify the second request carried the full tool round-trip.
    t.true(Array.isArray(secondRequestBody.messages))
    t.is(secondRequestBody.messages.length, 3)
    t.is(secondRequestBody.messages[2].role, 'tool')
    t.is(secondRequestBody.messages[2].tool_call_id, 'call_abc')
    t.true(Array.isArray(secondRequestBody.messages[1].tool_calls))
    t.is(secondRequestBody.messages[1].tool_calls[0].id, 'call_abc')
    t.is(secondRequestBody.messages[1].tool_calls[0].function.name, 'get_weather')
  } finally {
    await closeServer(server)
  }
})
