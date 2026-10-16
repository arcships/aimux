import test from 'node:test'
import assert from 'node:assert/strict'
import { createRequire } from 'node:module'
import { ABI_VERSION, OPS_VERSION, MANIFEST_DIGEST } from '../src/generated/ops.ts'

// Install the test transport before loading either public entrypoint.
const require = createRequire(import.meta.url)
const loader = require.resolve('../index.js')
require.cache[loader] = { id: loader, filename: loader, loaded: true, exports: require('./fixtures/native.cjs') } as never
const api = await import('../src/index.ts')
const rawApi = await import('../src/native.ts')
const { Client, LanguageModel, EmbeddingModel, AimuxError, readable } = api

test('raw and typed preserve the same DTO, null, nested provider options and 64-bit IDs', async () => {
  const client = new Client()
  const provider = await client.provider({ package: 'openai', settings: { apiKey: 'test' } })
  const model = await provider.model('test-model')
  assert.ok(model instanceof LanguageModel)
  assert.ok(BigInt(model.handle) > BigInt(Number.MAX_SAFE_INTEGER))
  const params = { prompt: '你好 🚀', options: { providerOptions: { openai: { nested: [null, { enabled: true }] } }, toolChoice: 'required' as const } }
  const raw = JSON.parse(await client.raw.call(model.handle, 'text.generate', JSON.stringify(params)))
  const typed = await (model as InstanceType<typeof LanguageModel>).generate(params.prompt, params.options)
  assert.deepEqual(typed, raw)
  const embedding = await provider.model('embed', 'embeddingModel')
  assert.ok(embedding instanceof EmbeddingModel)
  await (embedding as InstanceType<typeof EmbeddingModel>).embedMany({ values: ['one', 'two', '三'] })
  await Promise.all([model.close(), embedding.close(), provider.close()])
})

test('error properties and prototype match for both layers, including zero retry hint', async () => {
  const client = new Client()
  for (const call of [() => client.raw.call('0', 'fixture.error', '{}'), () => client.call('0', 'fixture.error')]) {
    await assert.rejects(call, (error: unknown) => {
      assert.ok(error instanceof AimuxError)
      assert.equal(error.kind, 'aimux')
      assert.equal(error.code, 1)
      assert.equal(error.retryAfterMs, 0)
      assert.deepEqual(error.error, { name: 'AI_APICallError', statusCode: 429, isRetryable: true, responseHeaders: { 'retry-after': '0' }, data: { unicode: '你好' } })
      assert.equal('retryable' in error, false)
      assert.equal('type' in error, false)
      return true
    })
  }
})

test('events retain recoverable error data and a unique explicit terminal', async () => {
  const client = new Client()
  const model = await (await client.provider({ package: 'openai' })).model('test')
  const frames = []
  for await (const frame of await client.stream(model.handle, 'text.stream')) frames.push(frame)
  assert.equal(frames.length, 2)
  assert.equal(frames[0].type, 'event')
  assert.equal(frames[0].seq, '9007199254740993')
  assert.deepEqual(frames[1], { type: 'end', seq: '9007199254740994', outcome: 'success' })
})

test('pre-acceptance rejection occurs before an iterator exists', async () => {
  const client = new Client()
  await assert.rejects(client.stream('0', 'text.stream', { reject: true }), (error: unknown) => error instanceof AimuxError && error.code === 301)
})

test('return cancels while next is pending', async () => {
  const client = new Client()
  const abort = client.abort()
  const iterator = await client.stream('0', 'text.stream', { pending: true }, { abort })
  const pending = iterator.next()
  await iterator.return!()
  assert.equal(abort.aborted, true)
  assert.deepEqual(await pending, { done: true, value: undefined })
  assert.equal((await iterator.next()).done, true)
})

test('reader.cancel cancels while a read is pending without prefetch', async () => {
  const client = new Client()
  const abort = client.abort()
  const iterator = await client.stream('0', 'text.stream', { pending: true }, { abort })
  const reader = readable(iterator).getReader()
  const pending = reader.read()
  await new Promise(resolve => setImmediate(resolve))
  await reader.cancel()
  assert.equal(abort.aborted, true)
  assert.equal((await pending).done, true)
})

test('throw preserves the original consumer error and aborts', async () => {
  const client = new Client()
  const abort = client.abort()
  const iterator = await client.stream('0', 'text.stream', {}, { abort })
  const error = new Error('consumer failed')
  await assert.rejects(iterator.throw!(error), caught => caught === error)
  assert.equal(abort.aborted, true)
})

test('AbortSignal is wired before invocation, including pre-aborted signals', async () => {
  const client = new Client()
  const controller = new AbortController()
  controller.abort()
  await assert.rejects(client.call('0', 'runtime.describe', {}, { signal: controller.signal }), error => error instanceof AimuxError && error.kind === 'aimux')
  const second = new AbortController()
  const abort = client.abort(second.signal)
  second.abort()
  assert.equal(abort.aborted, true)
})

test('clients in one environment share handles and Abort objects', async () => {
  const first = new Client()
  const second = new Client()
  const provider = await first.provider({ package: 'openai' })
  const registry = await second.registry({ shared: provider })
  assert.equal((await registry.provider('shared')).kind, 'Provider')
  await second.call('0', 'runtime.describe', {}, { abort: first.abort() })
  await Promise.all([registry.close(), provider.close()])
})

test('close is idempotent and retires the handle before awaiting native drop', async () => {
  const client = new Client()
  const provider = await client.provider({ package: 'openai' })
  const close = provider.close()
  assert.throws(() => provider.handle, error => error instanceof AimuxError && error.code === 203)
  await Promise.all([close, provider.close()])
})

test('devtools, registry and binary operation use the same raw dispatch', async () => {
  const client = new Client()
  const provider = await client.provider({ package: 'openai' })
  const registry = await client.registry({ main: provider })
  assert.equal((await registry.provider('main')).kind, 'Provider')
  const trace = await client.trace({ capacity: 64 })
  assert.deepEqual(await trace.sessionChain('s'), { op: 'trace.sessionChain', params: { sessionId: 's' } })
  const recorder = await client.recorder({ kind: 'ring' })
  assert.deepEqual(await recorder.flush(), { op: 'recorder.flush', params: {} })
  const transcription = await provider.model('asr', 'transcriptionModel') as InstanceType<typeof api.TranscriptionModel>
  const operation = await transcription.start({})
  await operation.push(new Uint8Array([0, 255, 128]))
  const events = operation.events()
  assert.equal((await events.next()).done, true)
  await events.return!()
})

test('manifest handshake pins every generated value and reports both sides', () => {
  const compatible = { abiVersion: ABI_VERSION, opsVersion: OPS_VERSION, digest: MANIFEST_DIGEST } as rawApi.Manifest
  rawApi.verifyManifest(compatible)
  for (const [field, received] of [['abiVersion', 0], ['opsVersion', '1.1'], ['digest', 'outdated'], ['digest', undefined]] as const) {
    assert.throws(() => rawApi.verifyManifest({ ...compatible, [field]: received }), (error: unknown) => {
      assert.ok(error instanceof AimuxError)
      assert.equal(error.code, 302)
      assert.ok(error.message.includes(String(compatible[field])))
      assert.ok(error.message.includes(String(received)))
      return true
    })
  }
})

test('typed serialization failures stay in the binding error domain', async () => {
  const client = new Client()
  const cyclic: Record<string, unknown> = {}
  cyclic.self = cyclic
  await assert.rejects(client.call('0', 'runtime.configure', cyclic), error => error instanceof AimuxError && error.kind === 'binding' && error.code === 202)
})

test('raw and typed expose matching error/cancelled terminal frames as DTOs', async () => {
  const client = new Client()
  for (const outcome of ['error', 'cancelled']) {
    const envelope = { kind: 'aimux', code: 1, message: 'failed', error: { name: 'AI_APICallError', isRetryable: false } }
    const values = [{ type: 'end', seq: '1', outcome, ...(outcome === 'error' ? { error: envelope } : {}) }]
    const raw = []
    for await (const frame of await client.raw.stream('0', 'text.stream', JSON.stringify({ values }))) raw.push(JSON.parse(frame))
    const typed = []
    for await (const frame of await client.stream('0', 'text.stream', { values })) typed.push(frame)
    assert.deepEqual(typed, raw)
    assert.deepEqual(typed, values)
  }
})

test('host callbacks are rejected instead of silently disappearing from settings', async () => {
  const client = new Client()
  await assert.rejects(client.provider({ package: 'openai', settings: { fetch: async () => {} } }), error => error instanceof AimuxError && error.kind === 'binding' && error.code === 202)
})
