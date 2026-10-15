// Runs against a real napi build, separately from the mocked transport facade suite.
import test from 'node:test'
import assert from 'node:assert/strict'
import { Worker } from 'node:worker_threads'
import { fileURLToPath } from 'node:url'
import { Runtime, AimuxError } from '../src/native.ts'

const source = fileURLToPath(new URL('../src/native.ts', import.meta.url))

test('native errors preserve the shared envelope and reject invalid wire JSON', async () => {
  const runtime = new Runtime()
  await assert.rejects(runtime.call('0', 'unknown.op', '{}'), error => error instanceof AimuxError && error.kind === 'protocol' && error.code === 300 && error.error.name === 'InvalidOp')
  await assert.rejects(runtime.call('0', 'runtime.describe', '{'), error => error instanceof AimuxError && error.kind === 'binding' && error.code === 202)
  await assert.rejects(runtime.call('9007199254740993', 'model.info', '{}'), error => error instanceof AimuxError && error.code === 203 && !('actual' in error.error))
})

test('runtime instances in one Env share ownership and native AbortSignal is immediate', async () => {
  const first = new Runtime()
  const second = new Runtime()
  const controller = new AbortController()
  const abort = first.abort(controller.signal)
  await second.call(abort.handle, 'abort.signal', '{}')
  assert.equal(abort.aborted, true)
  const preaborted = new AbortController()
  preaborted.abort()
  assert.equal(first.abort(preaborted.signal).aborted, true)
})

test('worker_threads cannot use a foreign realm handle; environment teardown completes', async () => {
  const runtime = new Runtime()
  const abort = runtime.abort()
  const worker = new Worker(`
    const { parentPort, workerData } = require('node:worker_threads')
    ;(async () => {
      const { Runtime } = await import(workerData.source)
      const runtime = new Runtime()
      try {
        await runtime.call(workerData.handle, 'abort.signal', '{}')
        parentPort.postMessage({ unexpectedSuccess: true })
      } catch (error) {
        parentPort.postMessage({ kind: error.kind, code: error.code, error: error.error })
      }
    })().catch(error => { throw error })
  `, { eval: true, workerData: { source, handle: abort.handle }, execArgv: ['--experimental-strip-types'] })
  const exit = new Promise(resolve => worker.once('exit', resolve))
  const message = await new Promise<Record<string, unknown>>((resolve, reject) => { worker.once('message', resolve); worker.once('error', reject) })
  assert.equal(message.kind, 'binding')
  assert.equal(message.code, 203)
  assert.equal('actual' in (message.error as object), false)
  const timeout = new Promise((_, reject) => { const timer = setTimeout(() => reject(new Error('Worker teardown hung')), 5000); timer.unref() })
  await Promise.race([exit, timeout])
  assert.equal(abort.aborted, false)
})

test('an async native call does not starve an event-loop timer', async () => {
  const runtime = new Runtime()
  let timer = false
  const task = new Promise(resolve => setTimeout(() => { timer = true; resolve(undefined) }, 10))
  for (let i = 0; i < 20; i++) await runtime.call('0', 'runtime.describe', '{}')
  await task
  assert.equal(timer, true)
})

test('typed lifecycle uses the real provider, registry, files, composite and devtools DTOs', async () => {
  const { Client, LanguageModel } = await import('../src/index.ts')
  const client = new Client()
  const provider = await client.provider({ package: 'openai', settings: { apiKey: 'local-test-key' } })
  const registry = await client.registry({ main: provider }, 'test')
  const resolved = await registry.resolve({ namespace: 'test', key: 'main', method: 'languageModel', modelId: 'test-model' })
  assert.ok(resolved instanceof LanguageModel)
  const raw = await client.call(resolved.handle, 'model.info')
  assert.deepEqual(await resolved.info(), raw)
  const files = await provider.files()
  assert.equal(files.kind, 'Files')
  assert.match(files.handle, /^[1-9][0-9]*$/)
  const router = await client.router({ children: [resolved.handle], strategy: { type: 'rule' } })
  assert.equal(router.kind, 'Language')
  const trace = await client.trace({ capacity: 16, perScopeCapacity: 8 })
  assert.equal(trace.kind, 'TraceStore')
  assert.deepEqual(await trace.aggregate(), await client.call(trace.handle, 'trace.aggregate', { filter: {} }))
  await assert.rejects(client.middleware({ kind: 'trace', traceStore: trace.handle }), error => error instanceof AimuxError && error.code === 8)
  const wrapped = await resolved.wrap([])
  assert.equal(wrapped.kind, 'Language')
  const recorder = await client.recorder({ backend: { type: 'ring', capacity: 16 }, capturePolicy: 'full' })
  assert.equal(recorder.kind, 'Recorder')
  await recorder.attach()
  await recorder.flush()
  assert.deepEqual(await recorder.export(), { jsonl: '' })
  await recorder.stop()
  const sessions = await client.sessionStore({ infer: false })
  assert.equal(sessions.kind, 'SessionStore')
  assert.deepEqual(await sessions.list(), [])
  assert.deepEqual(await sessions.calls('missing'), [])
  // Child resources retain their references after the provider is closed.
  await provider.close()
  assert.deepEqual(await resolved.info(), raw)
  await Promise.all([registry, resolved, files, router, trace, wrapped, recorder, sessions].map(resource => resource.close()))
})

test('replay inspection and planning preserve the canonical recording DTO', async () => {
  const { Client } = await import('../src/index.ts')
  const client = new Client()
  const recording = {
    schema: 2, callId: 'node-replay', recordedAt: '2026-10-15T00:00:00Z',
    input: { prompt: [{ role: 'user', content: [{ type: 'text', text: 'Hi' }] }], options: {} },
    provider: { providerId: 'openai', provider: 'openai.chat', modelId: 'test-model' },
    exchanges: [], outcome: { status: 'success' }, complete: true, transportClosed: true,
  }
  const replay = await client.replay({ recordingsJsonl: JSON.stringify(recording) })
  try {
    assert.equal(replay.kind, 'ReplaySession')
    assert.deepEqual(await replay.inspect(), await client.call(replay.handle, 'replay.inspect'))
    assert.deepEqual(await replay.dryRun(), await client.call(replay.handle, 'replay.dryRun'))
    const inspection = await replay.inspect() as { recordings: Array<{ callId: string }> }
    assert.equal(inspection.recordings[0].callId, recording.callId)
    await assert.rejects(replay.capabilities(), error => error instanceof AimuxError && error.code === 8)
  } finally { await replay.close() }
})

test('accepted native streams preserve one terminal and support return during next', async () => {
  const { Client, LanguageModel } = await import('../src/index.ts')
  const client = new Client()
  const provider = await client.provider({ package: 'deepseek', settings: { apiKey: 'local-test-key' } })
  const model = await provider.model('test-model', 'languageModel')
  assert.ok(model instanceof LanguageModel)
  const controller = new AbortController()
  controller.abort()
  try {
    const stream = await model.stream('hello', {}, { signal: controller.signal })
    const first = stream.next()
    const terminal = await first
    assert.equal(terminal.done, false)
    assert.equal(terminal.value.type, 'end')
    assert.equal(terminal.value.outcome, 'cancelled')
    assert.equal(terminal.value.seq, '0')
    assert.equal((await stream.next()).done, true)
    await assert.rejects(client.stream(model.handle, 'text.stream', { unexpected: true }), error => error instanceof AimuxError && error.code === 301)
    const returned = await model.stream('hello', {}, { signal: controller.signal })
    const pending = returned.next()
    await returned.return!()
    assert.equal((await pending).done, true)
  } finally { await Promise.all([model.close(), provider.close()]) }
})
