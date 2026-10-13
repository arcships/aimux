import test from 'ava'
import { createServer } from 'node:http'
import { readFileSync } from 'node:fs'
import { APICallError, InvalidArgumentError, RequestAbortedError, jevDecision, decide, describeDecisionModel, createProvider } from '../src/index.ts'
import type { DecideOptions } from '../src/index.ts'

const fixture = JSON.parse(readFileSync(new URL('../../../aimux-providers/tests/fixtures/jev_systemone.json', import.meta.url), 'utf8')).response

test('provider handle creates native OpenAI decisions that answer every question', async t => {
  const contract = JSON.parse(readFileSync(new URL('../../../aimux-providers/tests/fixtures/openai_decisions.json', import.meta.url), 'utf8'))
  let captured: any
  const server = createServer((req, res) => {
    t.is(req.url, '/v1/decisions')
    t.is(req.headers.authorization, 'Bearer test-key')
    let data = ''
    req.on('data', chunk => { data += chunk })
    req.on('end', () => {
      captured = JSON.parse(data)
      res.writeHead(200, { 'Content-Type': 'application/json' })
      res.end(JSON.stringify(contract.response))
    })
  })
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve))
  t.teardown(() => server.close())
  const address = server.address() as { port: number }
  const provider = await createProvider('openai', 'test-key', { baseUrl: `http://127.0.0.1:${address.port}/v1` })
  const model = await provider.decisionModel('gpt-6-luna')
  t.deepEqual(describeDecisionModel(model), {
    specification_version: 'v4',
    provider: 'openai',
    model_id: 'gpt-6-luna',
    supported_question_types: ['choice', 'score', 'boolean'],
  })
  const result = await decide(model, contract.options)
  t.deepEqual(captured, contract.request)
  t.deepEqual(Object.keys(result.answers), ['urgent', 'team', 'severity'])
  t.deepEqual(result.answers.urgent, { type: 'boolean', probability: 0.925 })
})
const options: DecideOptions = {
  state: { message: 'Billed twice' },
  questions: {
    is_urgent: { type: 'boolean', instructions: 'The message conveys urgency or time-sensitivity' },
    department: { type: 'choice', instructions: 'Which team should handle this', criteria: { billing: null, technical: null, sales: null } },
    frustration: { type: 'score', instructions: 'How frustrated the customer appears', criteria: ['Calm, just stating facts', 'Frustrated but civil', 'Very angry, strong language'] },
  },
  max_retries: 0,
}

test('missing or null usage counts preserve valid decision answers', async t => {
  let usage: Record<string, number | null> = {}
  const server = createServer((req, res) => {
    req.resume()
    res.writeHead(200, { 'Content-Type': 'application/json' })
    res.end(JSON.stringify({ ...fixture, usage }))
  })
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve))
  t.teardown(() => server.close())
  const address = server.address() as { port: number }
  const model = await jevDecision('test-key', 'jev-latest', `http://127.0.0.1:${address.port}/v1/systemone`)
  const usages: Record<string, number | null>[] = [{}, { input_tokens: null, output_tokens: null }, { input_tokens: 12 }, { output_tokens: 7 }]
  for (const counts of usages) {
    usage = counts
    const result = await decide(model, options)
    t.is(Object.keys(result.answers).length, 3)
    t.is(result.usage.input_tokens, counts.input_tokens ?? null)
    t.is(result.usage.output_tokens, counts.output_tokens ?? null)
    t.is(result.usage.total_tokens, null)
    t.deepEqual((result.response.body as any).usage, counts)
  }
})

test('typed decide maps all question types and preserves response metadata', async t => {
  let body: any
  let authorization: string | undefined
  let idempotency: string | undefined
  const server = createServer((req, res) => {
    authorization = req.headers.authorization
    idempotency = req.headers['idempotency-key'] as string
    let data = ''
    req.on('data', chunk => { data += chunk })
    req.on('end', () => {
      body = JSON.parse(data)
      res.writeHead(200, { 'Content-Type': 'application/json' })
      res.end(JSON.stringify(fixture))
    })
  })
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve))
  t.teardown(() => server.close())
  const address = server.address() as { port: number }
  const model = await jevDecision('test-key', 'jev-latest', `http://127.0.0.1:${address.port}/v1/systemone`)
  const result = await decide(model, { ...options, headers: { 'Idempotency-Key': 'test-decision-001' } })
  t.is(authorization, 'Bearer test-key')
  t.is(idempotency, 'test-decision-001')
  t.deepEqual(body.state, options.state)
  t.is(body.questions.is_urgent.type, 'noul')
  t.deepEqual(body.questions.department.criteria, { billing: null, technical: null, sales: null })
  t.deepEqual(body.questions.frustration.criteria, ['Calm, just stating facts', 'Frustrated but civil', 'Very angry, strong language'])
  t.deepEqual(result.answers.is_urgent, { type: 'boolean', probability: 1.0 })
  t.like(result.answers.department, { type: 'choice', choice: 'technical' })
  t.deepEqual(result.answers.frustration, { type: 'score', score: 1.0, probabilities: { 0: 0, 1: 1.0, 2: 0 } })
  t.deepEqual(result.usage, { input_tokens: 392, output_tokens: 65, total_tokens: 457 })
  t.deepEqual(result.response.body, fixture)
  t.is((result.provider_metadata?.jev as any).probability_source, 'native')
})

test('decision abort uses the canonical JavaScript error class', async t => {
  const model = await jevDecision('test-key', 'jev-latest', 'http://127.0.0.1:1/v1/systemone')
  const controller = new AbortController()
  controller.abort()
  await t.throwsAsync(decide(model, options, controller.signal), { instanceOf: RequestAbortedError })
})

test('decision abort cancels an in-flight HTTP request', async t => {
  const controller = new AbortController()
  const server = createServer((req, _res) => { req.resume(); controller.abort() })
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve))
  t.teardown(() => { server.closeAllConnections(); server.close() })
  const address = server.address() as { port: number }
  const model = await jevDecision('test-key', 'jev-latest', `http://127.0.0.1:${address.port}/v1/systemone`)
  await t.throwsAsync(decide(model, options, controller.signal), { instanceOf: RequestAbortedError })
})

test('invalid provider answer rejects with HTTP context', async t => {
  const server = createServer((req, res) => {
    req.resume()
    res.writeHead(200, { 'Content-Type': 'application/json' })
    res.end(JSON.stringify({ ...fixture, answers: {} }))
  })
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve))
  t.teardown(() => server.close())
  const address = server.address() as { port: number }
  const model = await jevDecision('test-key', 'jev-latest', `http://127.0.0.1:${address.port}/v1/systemone`)
  await t.throwsAsync(decide(model, options), { instanceOf: APICallError })
})

test('self-hosted probability provenance survives the Node boundary', async t => {
  const server = createServer((req, res) => {
    req.resume()
    res.writeHead(200, { 'Content-Type': 'application/json' })
    res.end(JSON.stringify(fixture))
  })
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve))
  t.teardown(() => server.close())
  const address = server.address() as { port: number }
  for (const source of ['native', 'logit_scoring', 'model_estimate'] as const) {
    const model = await jevDecision('test-key', 'jev-latest', `http://127.0.0.1:${address.port}/v1/systemone`, source)
    const result = await decide(model, options)
    t.is((result.provider_metadata?.jev as any).probability_source, source)
  }
})

test('unknown probability source is rejected at model construction', async t => {
  // Exercise the raw JSON boundary as well as the typed wrapper contract.
  const { jevDecision: rawJevDecision } = await import('../src/native.ts')
  await t.throwsAsync(rawJevDecision('test-key', 'jev-latest', undefined, 'unknown'), { instanceOf: InvalidArgumentError })
})

test('official structured fields and the model description preserve the native contract', async t => {
  const fixture = JSON.parse(readFileSync(new URL('../../../contract-tests/fixtures/decision-native.json', import.meta.url), 'utf8'))
  const requests: any[] = []
  const server = createServer((req, res) => {
    let data = ''
    req.on('data', chunk => { data += chunk })
    req.on('end', () => {
      requests.push(JSON.parse(data))
      res.writeHead(200, { 'Content-Type': 'application/json' })
      res.end(JSON.stringify(fixture.response))
    })
  })
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve))
  t.teardown(() => server.close())
  const address = server.address() as { port: number }
  const model = await jevDecision('test-key', 'jev-latest', `http://127.0.0.1:${address.port}/v1/systemone`)
  t.is(describeDecisionModel(model).provider, 'jev')
  t.is(requests.length, 0)
  const result = await decide(model, fixture.request)
  t.is(requests.length, 1)
  t.deepEqual(requests[0].questions.urgent.criteria, fixture.request.questions.urgent.criteria)
  t.deepEqual(requests[0].questions.department.instructions, fixture.request.questions.department.instructions)
  t.deepEqual(requests[0].questions.severity.criteria, fixture.request.questions.severity.criteria)
  t.deepEqual(result.answers.severity, { type: 'score', score: 0.25, probabilities: { 0: 0.75, 1: 0.25 } })
  t.deepEqual(result.rounding, { probability_decimals: 2, score_decimals: 2 })
})

test('OpenAI images, typed choice values and score descriptions cross the typed boundary', async t => {
  const contract = JSON.parse(readFileSync(new URL('../../../contract-tests/fixtures/decision-openai-full.json', import.meta.url), 'utf8'))
  let captured: unknown
  const server = createServer((req, res) => {
    let data = ''
    req.on('data', chunk => { data += chunk })
    req.on('end', () => {
      captured = JSON.parse(data)
      res.writeHead(200, { 'Content-Type': 'application/json' })
      res.end(JSON.stringify(contract.response))
    })
  })
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve))
  t.teardown(() => server.close())
  const address = server.address() as { port: number }
  const provider = await createProvider('openai', 'test-key', { baseUrl: `http://127.0.0.1:${address.port}/v1` })
  const model = await provider.decisionModel('gpt-6-luna')
  const request: DecideOptions = contract.options
  const result = await decide(model, request)
  t.deepEqual(captured, contract.request)
  t.deepEqual(result.answers.choice, { type: 'choice', choice: 'boolean_true', probabilities: { boolean_true: 0.75, text_true: 0.25 } })
  t.is((result.provider_metadata?.openai as any).answers.choice.value, true)
  t.deepEqual(result.answers.score, { type: 'score', score: 0.75, probabilities: { 0: 0.25, 1: 0.75 } })
})

test('local runtime factory preserves provider identity with no API key', async t => {
  const contract = JSON.parse(readFileSync(new URL('../../../aimux-providers/tests/fixtures/runtime_decisions.json', import.meta.url), 'utf8'))
  const server = createServer((req, res) => {
    t.is(req.url, '/v1/systemone')
    req.resume()
    res.writeHead(200, { 'Content-Type': 'application/json' })
    res.end(JSON.stringify(contract.response))
  })
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve))
  t.teardown(() => server.close())
  const address = server.address() as { port: number }
  const provider = await createProvider('ollama', undefined, { baseUrl: `http://127.0.0.1:${address.port}/v1` })
  const model = await provider.decisionModel('served-decision')
  t.is(describeDecisionModel(model).provider, 'ollama')
  const result = await decide(model, contract.options)
  t.is((result.provider_metadata?.ollama as any).probability_source, 'native')
  t.is(result.response.modelId, 'served-decision')
})
