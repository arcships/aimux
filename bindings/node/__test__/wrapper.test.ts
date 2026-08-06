// wrapper.test.ts — typed wrapper layer over the raw napi binding.
//
// Verifies that `generateText` / `streamText` (from src/index.ts) erase the
// JSON-string boundary: inputs are typed objects and outputs are parsed into
// typed objects (`.text`, `.tool_calls`, `.raw.content`, stream `TextDelta` /
// `ToolCall`). Uses the same mock-HTTP-server pattern as e2e.test.ts — no real
// API calls.

import test from 'ava'
import { createServer, type Server } from 'node:http'

// The wrapper under test. It re-exports the raw `openai`/`anthropic` factories
// and the ts-rs types, so a consumer needs only this one import.
import { openai, anthropic, generateText, streamText } from '../src/index.ts'
import type {
  GenerateTextResult,
  StreamPart,
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

// A typed tool definition + tool_choice, reused across tests.
const weatherTool: Tool = {
  type: 'function',
  name: 'get_weather',
  description: 'Get weather for a location',
  input_schema: {
    type: 'object',
    properties: { location: { type: 'string' } },
    required: ['location'],
  },
}

// ── Tests ───────────────────────────────────────────────────────────────────

test('wrapper: generateText returns a typed object (.text / .usage / .finish_reason / .raw.content)', async (t) => {
  const { server, url } = await startMockServer((req, res) => {
    res.writeHead(200, { 'content-type': 'application/json' })
    res.end(OPENAI_CHAT_RESPONSE)
  })

  try {
    const model = await openai('test-key', 'gpt-4o', url)
    // Typed input (string prompt) + typed output — no manual JSON.stringify/parse.
    const result: GenerateTextResult = await generateText(model, 'What is Rust?')

    // Top-level convenience fields are typed and present.
    t.is(result.text, 'Rust is a systems programming language.')
    t.truthy(result.usage, 'usage is present')
    t.truthy(result.finish_reason, 'finish_reason is present')
    t.true(Array.isArray(result.warnings))

    // raw is the provider-facing GenerateResult; .content is a typed array.
    t.truthy(result.raw, 'raw provider result is present')
    t.true(Array.isArray(result.raw.content), 'raw.content is a typed array')
    const textPart = result.raw.content.find((c) => 'Text' in c)
    t.truthy(textPart, 'raw.content contains a Text content part')
  } finally {
    await closeServer(server)
  }
})

test('wrapper: generateText parses tool_calls + raw.content ToolCall', async (t) => {
  const { server, url } = await startMockServer((req, res) => {
    res.writeHead(200, { 'content-type': 'application/json' })
    res.end(OPENAI_TOOL_CALL_RESPONSE)
  })

  try {
    const model = await openai('test-key', 'gpt-4o', url)
    // Typed options object: tools pass through to the provider.
    const result = await generateText(model, "What's the weather in Tokyo?", {
      tools: [weatherTool],
    })

    // Convenience field: tool_calls extracted and typed.
    t.is(result.tool_calls.length, 1)
    t.is(result.tool_calls[0].tool_name, 'get_weather')
    t.is(result.tool_calls[0].tool_call_id, 'call_abc')
    t.deepEqual(result.tool_calls[0].input, { location: 'Tokyo' })

    // Structured content: raw.content carries the ToolCall variant.
    const tc = result.raw.content.find((c) => 'ToolCall' in c)
    t.truthy(tc, 'raw.content contains a ToolCall content part')
    if (tc && 'ToolCall' in tc) {
      t.is(tc.ToolCall.tool_name, 'get_weather')
      t.is(tc.ToolCall.tool_call_id, 'call_abc')
      t.deepEqual(tc.ToolCall.input, { location: 'Tokyo' })
    }
  } finally {
    await closeServer(server)
  }
})

test('wrapper: generateText passes tool_choice through to the provider', async (t) => {
  const cap = capturingHandler(OPENAI_TOOL_CALL_RESPONSE)
  const { server, url } = await startMockServer(cap.handler)

  try {
    const model = await openai('test-key', 'gpt-4o', url)
    const toolChoice: ToolChoice = 'required'
    await generateText(model, 'Hello', {
      tools: [weatherTool],
      tool_choice: toolChoice,
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
    const model = await openai('test-key', 'gpt-4o', url)
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

test('wrapper: streamText yields typed TextDelta parts', async (t) => {
  const { server, url } = await startMockServer((req, res) => {
    res.writeHead(200, { 'content-type': 'text/event-stream' })
    res.end(OPENAI_STREAM_BODY)
  })

  try {
    const model = await openai('test-key', 'gpt-4o', url)
    const parts: StreamPart[] = []
    // streamText is an async generator of typed StreamParts — no JSON.parse.
    for await (const part of streamText(model, 'Say hello')) {
      parts.push(part)
    }

    t.true(parts.length > 0)

    // Narrow the tagged union by key presence (typed, not `any`).
    const deltas = parts
      .filter((p): p is Extract<StreamPart, { TextDelta: unknown }> => 'TextDelta' in p)
      .map((p) => p.TextDelta.delta)
    t.is(deltas.join(''), 'Hello world')
  } finally {
    await closeServer(server)
  }
})

test('wrapper: streamText emits Raw parts when include_raw_chunks set', async (t) => {
  // RFC-0016 M2: include_raw_chunks surfaces one Raw part per JSON SSE event
  // (parsed payload, before the parsed parts; [DONE] excluded).
  const { server, url } = await startMockServer((req, res) => {
    res.writeHead(200, { 'content-type': 'text/event-stream' })
    res.end(OPENAI_STREAM_BODY)
  })

  try {
    const model = await openai('test-key', 'gpt-4o', url)
    const parts: StreamPart[] = []
    for await (const part of streamText(model, 'Say hello', {
      include_raw_chunks: true,
    })) {
      parts.push(part)
    }

    const rawParts = parts.filter(
      (p): p is Extract<StreamPart, { Raw: unknown }> => 'Raw' in p,
    )
    // OPENAI_STREAM_BODY has 3 JSON events (2 content + 1 usage chunk);
    // the [DONE] sentinel emits no Raw.
    t.is(rawParts.length, 3)
    const firstRaw = rawParts[0].Raw as { raw_value: any }
    t.is(firstRaw.raw_value.choices[0].delta.content, 'Hello')

    // Raw for the first event precedes its TextDelta.
    const firstRawIdx = parts.indexOf(rawParts[0])
    const firstTextIdx = parts.findIndex((p) => 'TextDelta' in p)
    t.true(firstRawIdx < firstTextIdx, 'Raw must precede the parsed parts')
  } finally {
    await closeServer(server)
  }
})

test('wrapper: no Raw parts by default', async (t) => {
  const { server, url } = await startMockServer((req, res) => {
    res.writeHead(200, { 'content-type': 'text/event-stream' })
    res.end(OPENAI_STREAM_BODY)
  })

  try {
    const model = await openai('test-key', 'gpt-4o', url)
    const parts: StreamPart[] = []
    for await (const part of streamText(model, 'Say hello')) {
      parts.push(part)
    }
    t.true(parts.every((p) => !('Raw' in p)), 'no Raw parts expected by default')
  } finally {
    await closeServer(server)
  }
})

test('wrapper: streamText yields typed ToolCall parts', async (t) => {
  const { server, url } = await startMockServer((req, res) => {
    res.writeHead(200, { 'content-type': 'text/event-stream' })
    res.end(OPENAI_STREAM_TOOL_BODY)
  })

  try {
    const model = await openai('test-key', 'gpt-4o', url)
    const parts: StreamPart[] = []
    for await (const part of streamText(model, "What's the weather?", {
      tools: [weatherTool],
    })) {
      parts.push(part)
    }

    // The stream must contain a tool-related part.
    t.true(
      parts.some((p) => 'ToolCall' in p || 'ToolInputStart' in p || 'ToolInputDelta' in p),
      'stream contained a tool-related StreamPart',
    )

    // A complete ToolCall part carries the parsed tool name + input (typed).
    const toolCall = parts.find((p): p is Extract<StreamPart, { ToolCall: unknown }> => 'ToolCall' in p)
    if (toolCall) {
      t.is(toolCall.ToolCall.tool_name, 'get_weather')
      t.deepEqual(toolCall.ToolCall.input, { location: 'Tokyo' })
    }
  } finally {
    await closeServer(server)
  }
})
