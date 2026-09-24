import test from 'ava'
import { readFileSync } from 'node:fs'
import { createServer, type Server } from 'node:http'

import { generateText, generateTextAsOpenai, openai, streamText } from '../src/index.ts'
import type { RawToolCall, RepairToolCall, StreamPart, Tool, ToolCallRepairContext } from '../src/index.ts'
import {
  applyToolCallRepair,
  applyToolCallRepairToResult,
  toolCallRepairContext,
} from '../src/native.ts'

function startServer(body: string, contentType = 'application/json') {
  return new Promise<{ server: Server; url: string }>((resolve) => {
    const server = createServer((_req, res) => {
      res.writeHead(200, { 'content-type': contentType })
      res.end(body)
    })
    server.listen(0, '127.0.0.1', () => {
      const address = server.address()
      if (!address || typeof address === 'string') throw new Error('missing server address')
      resolve({ server, url: `http://127.0.0.1:${address.port}` })
    })
  })
}

const weatherTool: Tool = {
  type: 'function',
  name: 'weather',
  input_schema: {
    type: 'object',
    properties: { city: { type: 'string' } },
    required: ['city'],
    additionalProperties: false,
  },
}

const INVALID_TOOL_CALL = JSON.stringify({
  id: 'chatcmpl-repair',
  model: 'gpt-4o',
  choices: [{
    message: {
      role: 'assistant',
      content: null,
      tool_calls: [{
        id: 'call-1',
        type: 'function',
        function: { name: 'weather', arguments: '{"town":"Singapore"}' },
      }],
    },
    finish_reason: 'tool_calls',
  }],
  usage: { prompt_tokens: 5, completion_tokens: 2, total_tokens: 7 },
})

const INVALID_TOOL_CALL_STREAM = [
  'data: {"id":"1","model":"gpt-4o","choices":[{"delta":{"role":"assistant","tool_calls":[{"index":0,"id":"call-1","type":"function","function":{"name":"weather","arguments":""}}]}}]}\n\n',
  'data: {"id":"1","model":"gpt-4o","choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\\"town\\":\\"Singapore\\"}"}}]}}]}\n\n',
  'data: {"id":"1","model":"gpt-4o","choices":[{"delta":{},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":5,"completion_tokens":2,"total_tokens":7}}\n\n',
  'data: [DONE]\n\n',
].join('')

const fixCity = (ctx: ToolCallRepairContext): RawToolCall => ({
  ...ctx.tool_call,
  input: ctx.tool_call.input.replace('"town"', '"city"'),
})

test('repair: generateText repairs the call and transcript', async (t) => {
  const { server, url } = await startServer(INVALID_TOOL_CALL)
  try {
    const model = await openai('test-key', 'gpt-4o', url)
    let rawInput: string | undefined
    const result = await generateText(model, 'weather in Singapore?', {
      tools: [weatherTool],
      repairToolCall: (ctx) => {
        rawInput = ctx.tool_call.input
        return fixCity(ctx)
      },
    })

    t.is(rawInput, '{"town":"Singapore"}')
    t.deepEqual(result.tool_calls[0].input, { city: 'Singapore' })
    const message = result.response_messages[0] as any
    const part = message.content.find((item: any) => item.type === 'tool_call')
    t.deepEqual(part.input, result.tool_calls[0].input)
  } finally {
    await new Promise<void>((resolve) => server.close(() => resolve()))
  }
})

test('repair: generateTextAsOpenai carries the repaired arguments', async (t) => {
  const { server, url } = await startServer(INVALID_TOOL_CALL)
  try {
    const model = await openai('test-key', 'gpt-4o', url)
    const completion = await generateTextAsOpenai(model, 'weather in Singapore?', {
      tools: [weatherTool],
      repairToolCall: fixCity,
    })
    const call = completion.choices[0].message.tool_calls?.[0]
    t.is(call?.id, 'call-1')
    t.deepEqual(JSON.parse(call?.function.arguments ?? ''), { city: 'Singapore' })
    t.is(completion.model, 'gpt-4o')
  } finally {
    await new Promise<void>((resolve) => server.close(() => resolve()))
  }
})

test('repair: streamText repairs ToolCall and preserves deltas', async (t) => {
  const { server, url } = await startServer(INVALID_TOOL_CALL_STREAM, 'text/event-stream')
  try {
    const model = await openai('test-key', 'gpt-4o', url)
    const parts: StreamPart[] = []
    for await (const part of streamText(model, 'weather in Singapore?', {
      tools: [weatherTool],
      repairToolCall: fixCity,
    })) parts.push(part)

    const call = parts.find((p): p is Extract<StreamPart, { ToolCall: unknown }> => 'ToolCall' in p)
    t.deepEqual(call?.ToolCall.input, { city: 'Singapore' })
    const deltas = parts
      .filter((p): p is Extract<StreamPart, { ToolInputDelta: unknown }> => 'ToolInputDelta' in p)
      .map((p) => p.ToolInputDelta.delta)
    t.is(deltas.join(''), '{"town":"Singapore"}')
  } finally {
    await new Promise<void>((resolve) => server.close(() => resolve()))
  }
})

test('repair: shared contract fixture', (t) => {
  const fixture = JSON.parse(readFileSync(
    new URL('../../../contract-tests/fixtures/tool-call-repair.json', import.meta.url),
    'utf8',
  )) as { cases: any[] }
  t.true(fixture.cases.length > 0)

  const run = (c: any): string => {
    const opts = c.input.opts === null ? undefined : JSON.stringify(c.input.opts)
    switch (c.function) {
      case 'tool_call_repair_context':
        return toolCallRepairContext(JSON.stringify(c.input.tool_call), JSON.stringify(c.input.prompt), opts)
      case 'apply_tool_call_repair':
        return applyToolCallRepair(JSON.stringify(c.input.tool_call), opts, JSON.stringify(c.input.reply))
      case 'apply_tool_call_repair_to_result':
        return applyToolCallRepairToResult(
          JSON.stringify(c.input.result), opts, c.input.tool_call_id, JSON.stringify(c.input.reply),
        )
      default:
        throw new Error(`unknown fixture function ${c.function}`)
    }
  }

  for (const c of fixture.cases) {
    if (c.expected_error) {
      const error = t.throws(() => run(c), undefined, c.name)
      t.is(error?.constructor.name, `${c.expected_error}Error`, c.name)
    } else {
      t.deepEqual(JSON.parse(run(c)), c.expected, c.name)
    }
  }
})

test('repair: throwing, null, valid, and no-tool-set branches', async (t) => {
  let calls = 0
  const generate = async (body: string, repair: RepairToolCall, tools?: Tool[]) => {
    const { server, url } = await startServer(body)
    try {
      const model = await openai('test-key', 'gpt-4o', url)
      return await generateText(model, 'weather in Singapore?', {
        tools,
        repairToolCall: (ctx) => {
          calls += 1
          return repair(ctx)
        },
      })
    } finally {
      await new Promise<void>((resolve) => server.close(() => resolve()))
    }
  }

  // Throwing hook: invalid, wrapped in ToolCallRepair over the original error.
  const thrown = await generate(INVALID_TOOL_CALL, () => {
    throw new Error('repair model unavailable')
  }, [weatherTool])
  t.is(calls, 1)
  t.is(thrown.tool_calls[0].invalid, true)
  const failure = (thrown.tool_calls[0].error as any).ToolCallRepair
  t.deepEqual(failure.cause, { Other: 'Error: repair model unavailable' })
  t.true('InvalidToolInput' in failure.original_error)

  // Hook returns null: invalid with the ORIGINAL error, no repair wrapper.
  const unchanged = await generate(INVALID_TOOL_CALL, () => null, [weatherTool])
  t.is(calls, 2)
  t.is(unchanged.tool_calls[0].invalid, true)
  t.deepEqual(Object.keys(unchanged.tool_calls[0].error as any), ['InvalidToolInput'])

  // A valid call never reaches the hook.
  const valid = await generate(INVALID_TOOL_CALL.replace('town', 'city'), fixCity, [weatherTool])
  t.is(calls, 2)
  t.falsy(valid.tool_calls[0].invalid)
  t.deepEqual(valid.tool_calls[0].input, { city: 'Singapore' })

  // No tool set: not repairable, the hook is skipped, the call stays invalid.
  const untooled = await generate(INVALID_TOOL_CALL, fixCity)
  t.is(calls, 2)
  t.is(untooled.tool_calls[0].invalid, true)
  t.deepEqual(Object.keys(untooled.tool_calls[0].error as any), ['NoSuchTool'])
})
