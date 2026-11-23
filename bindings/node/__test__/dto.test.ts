import test from 'node:test'
import assert from 'node:assert/strict'
import { roundTrip } from './dto_vectors.ts'
import type { StreamFrame } from '../src/generated/types.ts'

test('DTO vectors reject unknown and missing discriminators with the type and tag', () => {
  assert.throws(() => roundTrip('StreamFrame', '{"type":"future"}'), /Unknown StreamFrame tag: future/)
  assert.throws(() => roundTrip('ModelMessage', '{}'), /Unknown ModelMessage tag: undefined/)
})

test('DTO vectors preserve decimal handle strings and optional nulls', () => {
  const frame: Extract<StreamFrame, { type: 'end' }> = { type: 'end', seq: '18446744073709551615', outcome: 'success', error: null }
  assert.deepEqual(roundTrip('StreamFrame', JSON.stringify(frame)), frame)
  delete frame.error
  assert.deepEqual(roundTrip('StreamFrame', JSON.stringify(frame)), frame)
  assert.equal(roundTrip('HandleId', '"18446744073709551615"'), '18446744073709551615')
})

test('DTO vectors reject strict unknown keys, bad enums, non-objects, and invalid handles', () => {
  assert.throws(() => roundTrip('OperationNextParams', '{"timeoutMs":50,"extra":true}'), /Unknown field extra/)
  assert.throws(() => roundTrip('OperationNextParams', '"not-an-object"'), /Expected object/)
  assert.throws(() => roundTrip('FinishReasonUnified', '"future-reason"'), /Invalid FinishReasonUnified/)
  assert.throws(() => roundTrip('HandleId', '"18446744073709551616"'), /Expected u64 string/)
  assert.throws(() => roundTrip('HandleId', '123'), /Expected u64 string/)
  assert.throws(() => roundTrip('OperationNextParams', '{"timeoutMs":-1}'), /Expected integer/)
  assert.throws(() => roundTrip('OperationNextParams', '{"timeoutMs":1.00000000000000001}'), /Expected integer/)
  assert.throws(() => roundTrip('OperationNextParams', '{"timeoutMs":18446744073709551616}'), /Expected integer/)
  assert.deepEqual(roundTrip('OperationNextParams', '{"timeoutMs":18446744073709551615}'), { timeoutMs: 18446744073709551616 })
  assert.throws(() => roundTrip('TraceAggregateFilter', '{"sinceUnixMs":9223372036854775808}'), /Expected integer/)
  assert.throws(() => roundTrip('TraceAggregateFilter', '{"sinceUnixMs":-9223372036854775809}'), /Expected integer/)
  assert.throws(() => roundTrip('TraceAggregateFilter', '{"sinceUnixMs":-9223372036854775809e0}'), /Expected integer/)
  assert.throws(() => roundTrip('TraceAggregateFilter', '{"sinceUnixMs":-9223372036854775808.1}'), /Expected integer/)
  assert.throws(() => roundTrip('GenerateTextOptions', `{"temperature":1${'0'.repeat(309)}}`), /Expected number/)
  assert.deepEqual(roundTrip('GenerateTextOptions', '{"temperature":1.00000000000000001}'), { temperature: 1 })
  assert.deepEqual(roundTrip('TraceAggregateFilter', '{"sinceUnixMs":9223372036854775807}'), { sinceUnixMs: 9223372036854775808 })
  assert.deepEqual(roundTrip('TraceAggregateFilter', '{"sinceUnixMs":-9223372036854775808}'), { sinceUnixMs: -9223372036854775808 })
  assert.throws(() => roundTrip('BackendConfig', '{"type":"ring","capacity":1,"extra":true}'), /Unknown field extra/)
  assert.throws(() => roundTrip('StreamFrame', '{"type":"end","seq":"0","outcome":"success","extra":true}'), /Unknown field extra/)
  assert.equal(roundTrip('HandleId', '"0001"'), '0001')
  assert.deepEqual(roundTrip('BackendConfig', '{"type":"ring","capacity":1}'), { type: 'ring', capacity: 1 })
})

test('GenerateTextResult getters derive ordered values without expanding JSON', async () => {
  const { readFileSync } = await import('node:fs')
  const { attachGenerateTextResult } = await import('../src/generated/text_result.ts')
  const fixture = JSON.parse(readFileSync(new URL(
    '../../../contract-tests/dto/GenerateTextResult/001.json', import.meta.url), 'utf8'))
  assert.throws(() => attachGenerateTextResult({ ...fixture, steps: [] }), TypeError)
  const first = fixture.steps[0]
  const toolCall = (toolCallId: string, dynamic?: boolean) =>
    ({ type: 'tool-call', toolCallId, toolName: 'tool', input: {}, ...(dynamic === undefined ? {} : { dynamic }) })
  const toolResult = (toolCallId: string, dynamic?: boolean) =>
    ({ type: 'tool-result', toolCallId, toolName: 'tool', output: null, ...(dynamic === undefined ? {} : { dynamic }) })
  const file = { base64: 'aGk=', mediaType: 'text/plain' }
  const source = { type: 'source', sourceType: 'url', id: 'source', url: 'https://example.com' }
  const earlyWarning = { type: 'other', message: 'first' }
  const finalWarning = { type: 'other', message: 'second' }
  first.content.push({ type: 'reasoning', text: 'ignored' }, source, toolCall('a'), toolResult('a', false))
  first.warnings = [earlyWarning]
  const last = {
    ...first,
    stepNumber: 1,
    finishReason: 'length',
    rawFinishReason: 'max_tokens',
    content: [
      { type: 'text', text: 'Final ' },
      { type: 'reasoning', text: 'think ' },
      { type: 'reasoning-file', file },
      { type: 'reasoning', text: 'more' },
      { type: 'text', text: 'answer' },
      { type: 'file', file },
      toolCall('b', true), toolResult('b', true), toolCall('c', false), toolResult('c'),
      { type: 'tool-error', toolCallId: 'd', toolName: 'tool', error: 'error' },
    ],
    warnings: [finalWarning],
    request: { body: { prompt: 'final' } },
    response: { ...first.response, messages: [{ role: 'assistant', content: 'Final answer' }] },
    providerMetadata: { last: { key: 1 } },
  }
  fixture.steps.push(last)
  const wire = JSON.stringify(fixture)
  const result = attachGenerateTextResult(fixture)
  assert.equal(result.finalStep, last)
  assert.equal(result.text, 'Final answer')
  assert.equal(result.output, result.text)
  assert.deepEqual(result.content, [...first.content, ...last.content])
  assert.deepEqual(result.reasoning, last.content.slice(1, 4))
  assert.equal(result.reasoningText, 'think more')
  assert.deepEqual(result.files, [file])
  assert.deepEqual(result.sources, [source])
  assert.deepEqual(result.toolCalls.map(part => part.toolCallId), ['a', 'b', 'c'])
  assert.deepEqual(result.staticToolCalls.map(part => part.toolCallId), ['a', 'c'])
  assert.deepEqual(result.dynamicToolCalls.map(part => part.toolCallId), ['b'])
  assert.deepEqual(result.toolResults.map(part => part.toolCallId), ['a', 'b', 'c'])
  assert.deepEqual(result.staticToolResults.map(part => part.toolCallId), ['a', 'c'])
  assert.deepEqual(result.dynamicToolResults.map(part => part.toolCallId), ['b'])
  assert.equal(result.finishReason, 'length')
  assert.equal(result.rawFinishReason, 'max_tokens')
  assert.equal(result.request, last.request)
  assert.equal(result.response, last.response)
  assert.equal(result.providerMetadata, last.providerMetadata)
  assert.equal(result.usage, fixture.totalUsage)
  assert.deepEqual(result.warnings, [earlyWarning, finalWarning])
  assert.deepEqual(result.responseMessages, [...first.response.messages, ...last.response.messages])
  assert.equal(JSON.stringify(result), wire)
  assert.deepEqual({ ...result }, fixture)
  assert.deepEqual(structuredClone(result), JSON.parse(wire))
  assert.deepEqual(Object.keys(result).sort(), ['steps', 'totalUsage'])

  const explicitNull = { ...JSON.parse(wire), output: null }
  const explicitWire = JSON.stringify(explicitNull)
  assert.equal(attachGenerateTextResult(explicitNull).output, null)
  assert.equal(JSON.stringify(explicitNull), explicitWire)

  const notParsed = { ...JSON.parse(wire), outputParsed: false }
  const notParsedWire = JSON.stringify(notParsed)
  const unparsedResult = attachGenerateTextResult(notParsed)
  assert.equal(unparsedResult.output, undefined)
  assert.equal(Object.hasOwn(unparsedResult, 'output'), false)
  assert.equal(unparsedResult.outputParsed, false)
  assert.equal(JSON.stringify(unparsedResult), notParsedWire)

  assert.throws(() => attachGenerateTextResult({ ...JSON.parse(wire), output: null, outputParsed: false }), TypeError)
})
