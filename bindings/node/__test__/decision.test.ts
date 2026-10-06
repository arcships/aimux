import test from 'ava'
import { createServer } from 'node:http'
import { readFileSync } from 'node:fs'
import { APICallError, InvalidArgumentError, RequestAbortedError, jevDecision, decide } from '../src/index.ts'
import type { DecisionCallOptions } from '../src/index.ts'

const fixture = JSON.parse(readFileSync(new URL('../../../aimux-providers/tests/fixtures/jev_systemone.json', import.meta.url), 'utf8')).response
const options: DecisionCallOptions = {
  state: { message: 'Billed twice' },
  questions: [
    { id: 'is_urgent', type: 'boolean', instructions: 'The message conveys urgency or time-sensitivity' },
    { id: 'department', type: 'choice', instructions: 'Which team should handle this', options: ['billing', 'technical', 'sales'].map(label => ({ label })) },
    { id: 'frustration', type: 'score', instructions: 'How frustrated the customer appears', levels: ['Calm, just stating facts', 'Frustrated but civil', 'Very angry, strong language'] },
  ],
  max_retries: 0,
}

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
  t.deepEqual(result.answers.is_urgent, { type: 'boolean', probability_true: 1.0 })
  t.like(result.answers.department, { type: 'choice', selected: 'technical' })
  t.like(result.answers.frustration, { type: 'score', expected_value: 1.0, probabilities: [0, 1.0, 0] })
  t.deepEqual(result.response?.body, fixture)
  t.is(result.probability_source, 'native')
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
    t.is((await decide(model, options)).probability_source, source)
  }
})

test('unknown probability source is rejected at model construction', async t => {
  // Exercise the raw JSON boundary as well as the typed wrapper contract.
  const { jevDecision: rawJevDecision } = await import('../src/native.ts')
  await t.throwsAsync(rawJevDecision('test-key', 'jev-latest', undefined, 'unknown'), { instanceOf: InvalidArgumentError })
})
