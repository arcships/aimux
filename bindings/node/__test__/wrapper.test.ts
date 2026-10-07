// wrapper.test.ts — typed wrapper layer over the raw napi binding.
//
// Verifies that `generateText` / `streamText` (from src/index.ts) erase the
// JSON-string boundary: inputs are typed objects and outputs are parsed into
// typed objects (`.text`, `.toolCalls`, `.raw.content`, stream `text-delta` /
// `tool-call`). Uses the same mock-HTTP-server pattern as e2e.test.ts — no real
// API calls.

import test from 'ava'
import { createServer, type Server } from 'node:http'

// The wrapper under test. It re-exports the raw `deepseek`/`anthropic` factories
// and the ts-rs types, so a consumer needs only this one import.
import { deepseek, anthropic, generateText, streamText } from '../src/index.ts'
import type {
  GenerateTextResult,
  TextStreamPart,
  ModelMessage,
  Tool,
  ToolChoice,
} from '../src/index.ts'

// ── Mock server helpers (same shape as e2e.test.ts) ────────────────────────

function startMockServer(
  handler: (req: any, res: any) => void,
): Promise<{ server: Server; url: string }> {
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

/** Capture the request body, then respond with `response` and `status`. */
function capturingHandler(
  response: string,
  contentType = 'application/json',
  status = 200,
) {
  let _body: string | null = null
  const handler = (req: any, res: any) => {
    let body = ''
    req.on('data', (chunk: Buffer) => (body += chunk))
    req.on('end', () => {
      _body = body
      res.writeHead(status, { 'content-type': contentType })
      res.end(response)
    })
  }
  return {
    handler,
    get body() {
      if (_body === null) throw new Error('request not received yet')
      return _body
    },
  }
}

// ── Mock responses (same wire shapes as e2e.test.ts) ───────────────────────

const OPENAI_CHAT_RESPONSE = JSON.stringify({
  id: 'chatcmpl-test',
  model: 'gpt-4o',
  choices: [
    {
      message: { role: 'assistant', content: 'Rust is a systems programming language.' },
      finish_reason: 'stop',
    },
  ],
  usage: { prompt_tokens: 10, completion_tokens: 8, total_tokens: 18 },
})

const OPENAI_STREAM_BODY = [
  'data: {"id":"1","model":"gpt-4o","choices":[{"delta":{"content":"Hello"}}]}\n\n',
  'data: {"id":"1","model":"gpt-4o","choices":[{"delta":{"content":" world"}}]}\n\n',
  'data: {"id":"1","model":"gpt-4o","choices":[{"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":5,"completion_tokens":2,"total_tokens":7}}\n\n',
  'data: [DONE]\n\n',
].join('')

const ANTHROPIC_MESSAGE_RESPONSE = JSON.stringify({
  id: 'msg_test',
  type: 'message',
  role: 'assistant',
  model: 'claude-3-5-sonnet-20241022',
  content: [{ type: 'text', text: 'Hello from Claude!' }],
  stop_reason: 'end_turn',
  usage: { input_tokens: 10, output_tokens: 5 },
})

const OPENAI_TOOL_CALL_RESPONSE = JSON.stringify({
  id: 'chatcmpl-tc',
  model: 'gpt-4o',
  choices: [
    {
      message: {
        role: 'assistant',
        content: null,
        tool_calls: [
          {
            id: 'call_abc',
            type: 'function',
            function: { name: 'get_weather', arguments: '{"location":"Tokyo"}' },
          },
        ],
      },
      finish_reason: 'tool_calls',
    },
  ],
  usage: { prompt_tokens: 20, completion_tokens: 10, total_tokens: 30 },
})

const OPENAI_STREAM_TOOL_BODY = [
  'data: {"id":"1","model":"gpt-4o","choices":[{"delta":{"role":"assistant","tool_calls":[{"index":0,"id":"call_xyz","type":"function","function":{"name":"get_weather","arguments":""}}]}}]}\n\n',
  'data: {"id":"1","model":"gpt-4o","choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\\"location\\":\\"Tokyo\\"}"}}]}}]}\n\n',
  'data: {"id":"1","model":"gpt-4o","choices":[{"delta":{},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":5,"completion_tokens":2,"total_tokens":7}}\n\n',
  'data: [DONE]\n\n',
].join('')

// A typed tool definition + toolChoice, reused across tests.
const weatherTool: Tool = {
  type: 'function',
  name: 'get_weather',
  description: 'Get weather for a location',
  inputSchema: {
    type: 'object',
    properties: { location: { type: 'string' } },
    required: ['location'],
  },
}

// ── Tests ───────────────────────────────────────────────────────────────────

test('wrapper: generateText returns a typed object (.text / .usage / .finishReason / .raw.content)', async (t) => {
  const { server, url } = await startMockServer((req, res) => {
    res.writeHead(200, { 'content-type': 'application/json' })
    res.end(OPENAI_CHAT_RESPONSE)
  })

  try {
    const model = await deepseek('test-key', 'gpt-4o', url)
    // Typed input (string prompt) + typed output — no manual JSON.stringify/parse.
    const result: GenerateTextResult = await generateText(model, 'What is Rust?')

    // Top-level convenience fields are typed and present.
    t.is(result.text, 'Rust is a systems programming language.')
    t.truthy(result.usage, 'usage is present')
    t.truthy(result.finishReason, 'finishReason is present')
    t.true(Array.isArray(result.warnings))

    // raw is the provider-facing GenerateResult; .content is a typed array.
    t.truthy(result.raw, 'raw provider result is present')
    t.true(Array.isArray(result.raw.content), 'raw.content is a typed array')
    const textPart = result.raw.content.find((c) => c.type === 'text')
    t.truthy(textPart, 'raw.content contains a text content part')
  } finally {
    await closeServer(server)
  }
})

test('wrapper: generateText parses toolCalls + raw.content tool-call', async (t) => {
  const { server, url } = await startMockServer((req, res) => {
    res.writeHead(200, { 'content-type': 'application/json' })
    res.end(OPENAI_TOOL_CALL_RESPONSE)
  })

  try {
    const model = await deepseek('test-key', 'gpt-4o', url)
    // Typed options object: tools pass through to the provider.
    const result = await generateText(model, "What's the weather in Tokyo?", {
      tools: [weatherTool],
    })

    // Convenience field: toolCalls extracted and typed.
    t.is(result.toolCalls.length, 1)
    t.is(result.toolCalls[0].toolName, 'get_weather')
    t.is(result.toolCalls[0].toolCallId, 'call_abc')
    t.deepEqual(result.toolCalls[0].input, { location: 'Tokyo' })

    // Structured content: raw.content carries the tool-call variant.
    const tc = result.raw.content.find((c) => c.type === 'tool-call')
    t.truthy(tc, 'raw.content contains a tool-call content part')
    if (tc && tc.type === 'tool-call') {
      t.is(tc.toolName, 'get_weather')
      t.is(tc.toolCallId, 'call_abc')
      // raw content keeps the provider's argument text (see e2e.test.ts).
      t.is(tc.input, '{"location":"Tokyo"}')
    }
  } finally {
    await closeServer(server)
  }
})

test('wrapper: generateText passes toolChoice through to the provider', async (t) => {
  const cap = capturingHandler(OPENAI_TOOL_CALL_RESPONSE)
  const { server, url } = await startMockServer(cap.handler)

  try {
    const model = await deepseek('test-key', 'gpt-4o', url)
    const toolChoice: ToolChoice = 'required'
    await generateText(model, 'Hello', {
      tools: [weatherTool],
      toolChoice,
    })

    const received = JSON.parse(cap.body)
    t.is(received.tool_choice, 'required')
  } finally {
    await closeServer(server)
  }
})

test('wrapper: generateText accepts typed multi-role messages', async (t) => {
  const cap = capturingHandler(OPENAI_CHAT_RESPONSE)
  const { server, url } = await startMockServer(cap.handler)

  try {
    const model = await deepseek('test-key', 'gpt-4o', url)
    // Typed ModelMessage[] prompt — no manual JSON.stringify.
    const messages: ModelMessage[] = [
      { role: 'system', content: 'You are a helpful assistant.' },
      { role: 'user', content: 'What is Rust?' },
    ]
    const result = await generateText(model, messages)

    // The full multi-role sequence reaches the provider request body.
    const received = JSON.parse(cap.body)
    t.true(Array.isArray(received.messages))
    t.is(received.messages.length, 2)
    t.is(received.messages[0].role, 'system')
    t.is(received.messages[0].content, 'You are a helpful assistant.')
    t.is(received.messages[1].role, 'user')
    t.is(received.messages[1].content, 'What is Rust?')

    t.truthy(result.text)
  } finally {
    await closeServer(server)
  }
})

test('wrapper: generateText works across providers (Anthropic mock)', async (t) => {
  const { server, url } = await startMockServer((req, res) => {
    res.writeHead(200, { 'content-type': 'application/json' })
    res.end(ANTHROPIC_MESSAGE_RESPONSE)
  })

  try {
    const model = await anthropic('test-key', 'claude-3-5-sonnet-20241022', url)
    const result = await generateText(model, 'Hello')
    t.is(result.text, 'Hello from Claude!')
    t.truthy(result.usage)
  } finally {
    await closeServer(server)
  }
})

test('wrapper: streamText yields typed text-delta parts', async (t) => {
  const { server, url } = await startMockServer((req, res) => {
    res.writeHead(200, { 'content-type': 'text/event-stream' })
    res.end(OPENAI_STREAM_BODY)
  })

  try {
    const model = await deepseek('test-key', 'gpt-4o', url)
    const parts: TextStreamPart[] = []
    // streamText is an async generator of typed StreamParts — no JSON.parse.
    for await (const part of streamText(model, 'Say hello')) {
      parts.push(part)
    }

    t.true(parts.length > 0)

    // Narrow the tagged union by its `type` tag (typed, not `any`).
    const deltas = parts
      .filter((p): p is Extract<TextStreamPart, { type: 'text-delta' }> => p.type === 'text-delta')
      .map((p) => p.delta)
    t.is(deltas.join(''), 'Hello world')
  } finally {
    await closeServer(server)
  }
})

test('wrapper: streamText emits raw parts when includeRawChunks set', async (t) => {
  // RFC-0016 M2: includeRawChunks surfaces one raw part per JSON SSE event
  // (parsed payload, before the parsed parts; [DONE] excluded).
  const { server, url } = await startMockServer((req, res) => {
    res.writeHead(200, { 'content-type': 'text/event-stream' })
    res.end(OPENAI_STREAM_BODY)
  })

  try {
    const model = await deepseek('test-key', 'gpt-4o', url)
    const parts: TextStreamPart[] = []
    for await (const part of streamText(model, 'Say hello', {
      includeRawChunks: true,
    })) {
      parts.push(part)
    }

    const rawParts = parts.filter(
      (p): p is Extract<TextStreamPart, { type: 'raw' }> => p.type === 'raw',
    )
    // OPENAI_STREAM_BODY has 3 JSON events (2 content + 1 usage chunk);
    // the [DONE] sentinel emits no Raw.
    t.is(rawParts.length, 3)
    const firstRaw = rawParts[0].rawValue as any
    t.is(firstRaw.choices[0].delta.content, 'Hello')

    // The raw part for the first event precedes its text-delta.
    const firstRawIdx = parts.indexOf(rawParts[0])
    const firstTextIdx = parts.findIndex((p) => p.type === 'text-delta')
    t.true(firstRawIdx < firstTextIdx, 'raw must precede the parsed parts')
  } finally {
    await closeServer(server)
  }
})

test('wrapper: no raw parts by default', async (t) => {
  const { server, url } = await startMockServer((req, res) => {
    res.writeHead(200, { 'content-type': 'text/event-stream' })
    res.end(OPENAI_STREAM_BODY)
  })

  try {
    const model = await deepseek('test-key', 'gpt-4o', url)
    const parts: TextStreamPart[] = []
    for await (const part of streamText(model, 'Say hello')) {
      parts.push(part)
    }
    t.true(parts.every((p) => p.type !== 'raw'), 'no raw parts expected by default')
  } finally {
    await closeServer(server)
  }
})

test('wrapper: streamText yields typed tool-call parts', async (t) => {
  const { server, url } = await startMockServer((req, res) => {
    res.writeHead(200, { 'content-type': 'text/event-stream' })
    res.end(OPENAI_STREAM_TOOL_BODY)
  })

  try {
    const model = await deepseek('test-key', 'gpt-4o', url)
    const parts: TextStreamPart[] = []
    for await (const part of streamText(model, "What's the weather?", {
      tools: [weatherTool],
    })) {
      parts.push(part)
    }

    // The stream must contain a tool-related part.
    t.true(
      parts.some((p) => ['tool-call', 'tool-input-start', 'tool-input-delta'].includes(p.type)),
      'stream contained a tool-related StreamPart',
    )

    // A complete tool-call part carries the parsed tool name + input (typed).
    const toolCall = parts.find((p): p is Extract<TextStreamPart, { type: 'tool-call' }> => p.type === 'tool-call')
    if (toolCall) {
      t.is(toolCall.toolName, 'get_weather')
      t.deepEqual(toolCall.input, { location: 'Tokyo' })
    }
  } finally {
    await closeServer(server)
  }
})
