import test from 'ava'
import { createServer } from 'node:http'
import { readFileSync } from 'node:fs'
import { APICallError, RequestAbortedError, jevDecision, decide } from '../src/index.ts'
import type { DecisionCallOptions } from '../src/index.ts'

const fixture = JSON.parse(readFileSync(new URL('../../../aimux-providers/tests/fixtures/jev_systemone.json', import.meta.url), 'utf8')).response
const options: DecisionCallOptions = {
  state: { message: 'Billed twice' },
  questions: [
    { id: 'needs_human', type: 'boolean', instructions: 'Does this need a human?' },
    { id: 'queue', type: 'choice', instructions: 'Which team?', options: ['billing', 'technical', 'sales'].map(label => ({ label })) },
    { id: 'anger', type: 'score', instructions: 'How angry?', levels: ['Calm', 'Mildly annoyed', 'Frustrated', 'Angry'] },
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
  const model = await jevDecision('test-key', 'jev-1.13', `http://127.0.0.1:${address.port}/v1/systemone`)
  const result = await decide(model, { ...options, headers: { 'Idempotency-Key': 'test-decision-001' } })
  t.is(authorization, 'Bearer test-key')
  t.is(idempotency, 'test-decision-001')
  t.deepEqual(body.state, options.state)
  t.is(body.questions.needs_human.type, 'noul')
  t.deepEqual(body.questions.queue.criteria, { billing: 'billing', technical: 'technical', sales: 'sales' })
  t.deepEqual(body.questions.anger.criteria, ['Calm', 'Mildly annoyed', 'Frustrated', 'Angry'])
  t.deepEqual(result.answers.needs_human, { type: 'boolean', probability_true: 0.89 })
  t.like(result.answers.queue, { type: 'choice', selected: 'billing' })
  t.like(result.answers.anger, { type: 'score', expected_value: 1.89, probabilities: [0, 0.11, 0.89, 0] })
  t.deepEqual(result.response?.body, fixture)
  t.is(result.probability_source, 'native')
})

test('decision abort uses the canonical JavaScript error class', async t => {
  const model = await jevDecision('test-key', 'jev-1.13', 'http://127.0.0.1:1/v1/systemone')
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
  const model = await jevDecision('test-key', 'jev-1.13', `http://127.0.0.1:${address.port}/v1/systemone`)
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
  const model = await jevDecision('test-key', 'jev-1.13', `http://127.0.0.1:${address.port}/v1/systemone`)
  await t.throwsAsync(decide(model, options), { instanceOf: APICallError })
})
