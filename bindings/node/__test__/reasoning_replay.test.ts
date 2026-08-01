// reasoning_replay.test.ts — End-to-end regression tests for the two issues
// reported against aimux 0.1.1 when driving OpenAI-compatible thinking models
// (e.g. DeepSeek `deepseek-v4-flash`) in multi-turn tool-call conversations.
//
// Issue 1: a `tool`-role message carrying `ContentPart[]` with a `tool_result`
//   part built from the legacy `output` field was rejected by `ModelPrompt`
//   deserialization ("data did not match any variant of untagged enum
//   ModelPrompt"). The Rust `ContentPart::ToolResult.result` field now accepts
//   `output` as a serde alias, so both shapes round-trip.
//
// Issue 2: thinking models require prior assistant `reasoning_content` to be
//   replayed on later turns. The OpenAI message converter now lifts
//   `ContentPart::Reasoning` parts to a top-level `reasoning_content` string.
//
// These tests use a mock HTTP server (no real API) and assert the outbound
// request body shape that the Rust engine produces.

import test from 'ava'
import { createServer, type Server } from 'node:http'
import { openai } from '../index.js'

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

/// Read the full request body as a parsed JSON object.
function readBody(req: any): Promise<any> {
  return new Promise((resolve) => {
    let body = ''
    req.on('data', (chunk: string) => { body += chunk })
    req.on('end', () => resolve(JSON.parse(body)))
  })
}

const CHAT_RESPONSE = JSON.stringify({
  id: 'chatcmpl-test',
  model: 'deepseek-v4-flash',
  choices: [{
    message: { role: 'assistant', content: 'Done.' },
    finish_reason: 'stop',
  }],
  usage: { prompt_tokens: 10, completion_tokens: 1, total_tokens: 11 },
})

// ── Issue 1: tool role ContentPart[] with the legacy `output` field ─────────

test('issue 1: tool_result with legacy `output` field is accepted and carries tool_call_id', async (t) => {
  let requestBody: any = null
  const { server, url } = await startMockServer(async (req, res) => {
    requestBody = await readBody(req)
    res.writeHead(200, { 'content-type': 'application/json' })
    res.end(CHAT_RESPONSE)
  })

  try {
    const model = await openai('test-key', 'deepseek-v4-flash', url)

    // The exact shape from the user's failing case: a tool message whose
    // ContentPart uses the legacy `output` field (Vercel AI SDK / 0.1.0 TS).
    const messages = [
      { role: 'user', content: 'write a file' },
      {
        role: 'assistant',
        content: [{
          type: 'tool_call',
          tool_call_id: 'tc1',
          tool_name: 'write_file',
          input: { path: '/tmp/test.txt' },
        }],
      },
      {
        role: 'tool',
        content: [{
          type: 'tool_result',
          tool_call_id: 'tc1',
          output: 'Successfully wrote to /tmp/test.txt',
        }],
      },
    ]

    // Before the fix this rejected with "invalid prompt" / ModelPrompt error.
    const resultJson = await model.generateText(JSON.stringify(messages))
    const r = JSON.parse(resultJson)
    t.is(r.text, 'Done.')

    // The outbound request must carry the tool_call_id (the core of issue 1).
    const toolMsg = requestBody.messages[2]
    t.is(toolMsg.role, 'tool')
    t.is(toolMsg.tool_call_id, 'tc1')
    t.is(toolMsg.content, 'Successfully wrote to /tmp/test.txt')
  } finally {
    await closeServer(server)
  }
})

// ── Issue 2: reasoning_content replay on the request side ───────────────────

test('issue 2: assistant reasoning + tool_call lifts reasoning to reasoning_content', async (t) => {
  let requestBody: any = null
  const { server, url } = await startMockServer(async (req, res) => {
    requestBody = await readBody(req)
    res.writeHead(200, { 'content-type': 'application/json' })
    res.end(CHAT_RESPONSE)
  })

  try {
    const model = await openai('test-key', 'deepseek-v4-flash', url)

    // Reproduces the user's table row: assistant
    // [{reasoning},{tool_call}] + tool string. The first turn produced a
    // reasoning_content + tool_call; the second turn must replay both.
    const messages = [
      { role: 'user', content: 'inspect the repo' },
      {
        role: 'assistant',
        content: [
          { type: 'reasoning', text: 'I need to inspect files before answering.', signature: null },
          { type: 'tool_call', tool_call_id: 'call_1', tool_name: 'read_file', input: { path: 'README.md' } },
        ],
      },
      {
        role: 'tool',
        content: 'contents of README.md',
      },
    ]

    const resultJson = await model.generateText(JSON.stringify(messages))
    const r = JSON.parse(resultJson)
    t.is(r.text, 'Done.')

    // The assistant message must carry reasoning_content (the DeepSeek V4
    // thinking-mode requirement) alongside tool_calls.
    const assistant = requestBody.messages[1]
    t.is(assistant.role, 'assistant')
    t.is(assistant.reasoning_content, 'I need to inspect files before answering.')
    t.true(Array.isArray(assistant.tool_calls))
    t.is(assistant.tool_calls[0].id, 'call_1')
    t.is(assistant.tool_calls[0].function.name, 'read_file')
  } finally {
    await closeServer(server)
  }
})

test('issue 2: assistant reasoning + text (no tool calls) emits reasoning_content', async (t) => {
  let requestBody: any = null
  const { server, url } = await startMockServer(async (req, res) => {
    requestBody = await readBody(req)
    res.writeHead(200, { 'content-type': 'application/json' })
    res.end(CHAT_RESPONSE)
  })

  try {
    const model = await openai('test-key', 'deepseek-v4-flash', url)

    const messages = [
      { role: 'user', content: 'What is 2+2?' },
      {
        role: 'assistant',
        content: [
          { type: 'reasoning', text: '2 plus 2 equals 4.', signature: null },
          { type: 'text', text: '4' },
        ],
      },
      { role: 'user', content: 'thanks' },
    ]

    await model.generateText(JSON.stringify(messages))

    const assistant = requestBody.messages[1]
    t.is(assistant.role, 'assistant')
    t.is(assistant.content, '4')
    t.is(assistant.reasoning_content, '2 plus 2 equals 4.')
    t.falsy(assistant.tool_calls, 'no tool_calls key when there are none')
  } finally {
    await closeServer(server)
  }
})

test('issue 2: assistant without reasoning omits reasoning_content', async (t) => {
  let requestBody: any = null
  const { server, url } = await startMockServer(async (req, res) => {
    requestBody = await readBody(req)
    res.writeHead(200, { 'content-type': 'application/json' })
    res.end(CHAT_RESPONSE)
  })

  try {
    const model = await openai('test-key', 'deepseek-v4-flash', url)

    const messages = [
      { role: 'user', content: 'hi' },
      {
        role: 'assistant',
        content: [{
          type: 'tool_call',
          tool_call_id: 'call_1',
          tool_name: 'write_file',
          input: { path: '/tmp/x' },
        }],
      },
    ]

    await model.generateText(JSON.stringify(messages))

    const assistant = requestBody.messages[1]
    t.is(assistant.role, 'assistant')
    t.falsy(assistant.reasoning_content, 'reasoning_content must be omitted when no reasoning part')
  } finally {
    await closeServer(server)
  }
})
