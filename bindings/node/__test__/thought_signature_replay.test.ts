// thought_signature_replay.test.ts — Regression tests for issue arcships/aimux#9.
//
// Google Gemini **thinking** models (gemini-2.5-pro, gemini-3.x-flash-thinking)
// attach a `thoughtSignature` to every `functionCall` part in responses, and
// the follow-up turn MUST echo it back verbatim as a sibling of the
// `functionCall` part — otherwise the API rejects the request with HTTP 400
// ("Function call is missing a thought_signature").
//
// aimux previously dropped the signature at parse time (no field in any
// ToolCall type) and did not emit it when rebuilding assistant tool-call
// parts. These tests use a mock HTTP server (no real API) and assert:
//   1. parse direction: `thoughtSignature` from the response reaches the
//      typed `GenerateTextResult.tool_calls[].thought_signature`;
//   2. replay direction: the outbound request carries the signature as a
//      part-level sibling of `functionCall` (not nested inside it).
//
// This mirrors the reasoning_replay.test.ts approach for DeepSeek thinking
// models.

import test from 'ava'
import { createServer, type Server } from 'node:http'
import { google } from '../index.js'

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

const THOUGHT_SIGNATURE =
  'EuIDCt8DARFNMg/aRDRK3THWhBjzltCEy5/VM6ImWLJU8oHmnC75abdcZBMH'

// A minimal Gemini non-streaming response with a functionCall + thoughtSignature.
function geminiToolCallResponse(signature?: string) {
  return JSON.stringify({
    candidates: [{
      content: {
        parts: [{
          functionCall: {
            id: 'call-1',
            name: 'weather',
            args: { location: 'San Francisco' },
          },
          ...(signature ? { thoughtSignature: signature } : {}),
        }],
        role: 'model',
      },
      finishReason: 'STOP',
      index: 0,
    }],
    usageMetadata: { promptTokenCount: 5, candidatesTokenCount: 5, totalTokenCount: 10 },
  })
}

const TEXT_RESPONSE = JSON.stringify({
  candidates: [{
    content: { parts: [{ text: 'Done.' }], role: 'model' },
    finishReason: 'STOP',
    index: 0,
  }],
  usageMetadata: { promptTokenCount: 5, candidatesTokenCount: 5, totalTokenCount: 10 },
})

// ── parse direction: thoughtSignature reaches the typed tool_calls ──────────

test('parse: thoughtSignature from a functionCall part lands on tool_calls[].thought_signature', async (t) => {
  const { server, url } = await startMockServer(async (_req, res) => {
    res.writeHead(200, { 'content-type': 'application/json' })
    res.end(geminiToolCallResponse(THOUGHT_SIGNATURE))
  })

  try {
    const model = await google('test-key', 'gemini-2.5-pro', url)
    const resultJson = await model.generateText(
      JSON.stringify([{ role: 'user', content: 'weather in SF?' }]),
      JSON.stringify({
        tools: [{
          type: 'function',
          name: 'weather',
          description: 'Get the weather',
          input_schema: {
            type: 'object',
            properties: { location: { type: 'string' } },
            required: ['location'],
            additionalProperties: false,
          },
        }],
      }),
    )
    const r = JSON.parse(resultJson)
    t.is(r.tool_calls.length, 1)
    t.is(r.tool_calls[0].tool_call_id, 'call-1')
    t.is(r.tool_calls[0].tool_name, 'weather')
    t.is(r.tool_calls[0].thought_signature, THOUGHT_SIGNATURE)
  } finally {
    await closeServer(server)
  }
})

// ── replay direction: signature echoed back as a part-level sibling ─────────

test('replay: assistant tool_call with thought_signature echoes it back verbatim as a part sibling', async (t) => {
  let requestBody: any = null
  const { server, url } = await startMockServer(async (req, res) => {
    requestBody = await readBody(req)
    res.writeHead(200, { 'content-type': 'application/json' })
    res.end(TEXT_RESPONSE)
  })

  try {
    const model = await google('test-key', 'gemini-2.5-pro', url)

    // Second turn: assistant tool_call (with the captured signature) + the
    // tool result. This is the exact shape that used to fail with HTTP 400.
    const messages = [
      { role: 'user', content: 'weather in SF?' },
      {
        role: 'assistant',
        content: [{
          type: 'tool_call',
          tool_call_id: 'call-1',
          tool_name: 'weather',
          input: { location: 'San Francisco' },
          thought_signature: THOUGHT_SIGNATURE,
        }],
      },
      {
        role: 'tool',
        content: [{
          type: 'tool_result',
          tool_call_id: 'call-1',
          result: { temp: 70 },
        }],
      },
    ]

    const resultJson = await model.generateText(JSON.stringify(messages))
    const r = JSON.parse(resultJson)
    t.is(r.text, 'Done.')

    // The signature must be a SIBLING of `functionCall` on the part.
    const part = requestBody.contents[1].parts[0]
    t.is(part.thoughtSignature, THOUGHT_SIGNATURE)
    t.is(part.functionCall.thoughtSignature, undefined)
    // And the tool result must be present as a functionResponse part.
    t.is(requestBody.contents[2].role, 'user')
    t.is(requestBody.contents[2].parts[0].functionResponse.id, 'call-1')
  } finally {
    await closeServer(server)
  }
})

// ── negative: no signature on the input → no thoughtSignature in the body ───

test('replay: tool_call without signature emits no thoughtSignature field', async (t) => {
  let requestBody: any = null
  const { server, url } = await startMockServer(async (req, res) => {
    requestBody = await readBody(req)
    res.writeHead(200, { 'content-type': 'application/json' })
    res.end(TEXT_RESPONSE)
  })

  try {
    const model = await google('test-key', 'gemini-2.5-pro', url)

    const messages = [
      { role: 'user', content: 'weather in SF?' },
      {
        role: 'assistant',
        content: [{
          type: 'tool_call',
          tool_call_id: 'call-1',
          tool_name: 'weather',
          input: { location: 'San Francisco' },
        }],
      },
      {
        role: 'tool',
        content: [{
          type: 'tool_result',
          tool_call_id: 'call-1',
          result: { temp: 70 },
        }],
      },
    ]

    await model.generateText(JSON.stringify(messages))

    const part = requestBody.contents[1].parts[0]
    t.is(part.thoughtSignature, undefined)
  } finally {
    await closeServer(server)
  }
})
