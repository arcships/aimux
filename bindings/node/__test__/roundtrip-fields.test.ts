// roundtrip-fields.test.ts — reasoning signature & toolName pass-through (#135).
//
// Proves the language-side pipe for the Round 4 fixes, through the FULL
// chain (Node.js → napi-rs → Rust engine → HTTP mock):
//   - anthropic: with thinking enabled, a prompt reasoning part's
//     `signature` is echoed as a thinking block on the wire (input
//     round-trip), and a thinking-block response surfaces `signature` on
//     the result (response visibility, #131).
//   - google: a tool_result's `toolName` reaches the request as
//     functionResponse.name (#127).

import test from 'ava'
import { createServer, type Server } from 'node:http'
import { anthropic, google } from '../src/native.ts'

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

const ANTHROPIC_THINKING_RESPONSE = JSON.stringify({
  id: 'msg_rt',
  type: 'message',
  role: 'assistant',
  model: 'claude-sonnet-4-20250514',
  content: [
    { type: 'thinking', thinking: 'pondering deeply', signature: 'sig-resp-1' },
    { type: 'text', text: 'The answer.' },
  ],
  stop_reason: 'end_turn',
  usage: { input_tokens: 10, output_tokens: 5 },
})

const GEMINI_RESPONSE = JSON.stringify({
  candidates: [
    {
      content: { parts: [{ text: 'ok' }], role: 'model' },
      finishReason: 'STOP',
    },
  ],
  usageMetadata: { promptTokenCount: 3, candidatesTokenCount: 1, totalTokenCount: 4 },
})

test('roundtrip: anthropic echoes prompt reasoning signature and surfaces response signature', async (t) => {
  let receivedBody: any = null
  const { server, url } = await startMockServer((req, res) => {
    let body = ''
    req.on('data', (chunk: Buffer) => (body += chunk))
    req.on('end', () => {
      receivedBody = JSON.parse(body)
      res.writeHead(200, { 'content-type': 'application/json' })
      res.end(ANTHROPIC_THINKING_RESPONSE)
    })
  })

  try {
    const model = await anthropic('test-key', 'claude-sonnet-4-20250514', url)
    const prompt = [
      { role: 'user', content: [{ type: 'text', text: 'hi' }] },
      {
        role: 'assistant',
        content: [
          { type: 'reasoning', text: 'pondering', signature: 'sig-prompt-1' },
          { type: 'text', text: 'answer' },
        ],
      },
      { role: 'user', content: [{ type: 'text', text: 'go on' }] },
    ]
    const opts = JSON.stringify({
      provider_options: {
        anthropic: { thinking: { type: 'enabled', budgetTokens: 1024 } },
      },
    })
    const resultJson = await model.generateText(JSON.stringify(prompt), opts)

    // Input round-trip: the prompt reasoning part is echoed as a thinking
    // block (with its signature) on the wire.
    const blocks = (receivedBody.messages ?? []).flatMap((m: any) => m.content ?? [])
    const thinking = blocks.find((b: any) => b.type === 'thinking')
    t.truthy(thinking, `expected a thinking block on the wire, got ${JSON.stringify(blocks)}`)
    t.is(thinking.signature, 'sig-prompt-1')
    t.is(thinking.thinking, 'pondering')

    // Response visibility: the result carries the response signature so
    // extended-thinking multi-turn can echo it back.
    t.true(
      resultJson.includes('"signature":"sig-resp-1"'),
      `expected the result JSON to carry the response signature, got ${resultJson}`
    )
  } finally {
    await closeServer(server)
  }
})

test('roundtrip: google toolName reaches functionResponse.name', async (t) => {
  let receivedBody: any = null
  const { server, url } = await startMockServer((req, res) => {
    let body = ''
    req.on('data', (chunk: Buffer) => (body += chunk))
    req.on('end', () => {
      receivedBody = JSON.parse(body)
      res.writeHead(200, { 'content-type': 'application/json' })
      res.end(GEMINI_RESPONSE)
    })
  })

  try {
    const model = await google('test-key', 'gemini-2.0-flash', url)
    const prompt = [
      { role: 'user', content: [{ type: 'text', text: 'weather?' }] },
      {
        role: 'assistant',
        content: [
          { type: 'tool_call', tool_call_id: 'call-1', tool_name: 'weather', input: { location: 'SF' } },
        ],
      },
      {
        role: 'tool',
        content: [{ type: 'tool_result', tool_call_id: 'call-1', tool_name: 'weather', result: { temp: 70 } }],
      },
    ]
    await model.generateText(JSON.stringify(prompt))

    const parts = (receivedBody.contents ?? []).flatMap((c: any) => c.parts ?? [])
    const fr = parts.map((p: any) => p.functionResponse).find(Boolean)
    t.truthy(fr, `expected a functionResponse part, got ${JSON.stringify(parts)}`)
    // `name` carries the real tool name (#127), not the opaque call id.
    t.is(fr.name, 'weather')
    t.is(fr.id, 'call-1')
  } finally {
    await closeServer(server)
  }
})
