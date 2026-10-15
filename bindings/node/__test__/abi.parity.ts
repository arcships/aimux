import test from 'node:test'
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { Runtime, AimuxError } from '../src/native.ts'

test('Node dispatch and the C ABI agree on provider, registry, devtools and errors', async () => {
  assert.ok(process.env.AIMUX_FFI_LIBRARY, 'Set AIMUX_FFI_LIBRARY to the built aimux-ffi library')
  const recording = {
    schema: 2, callId: 'node-c-parity', recordedAt: '2026-10-15T00:00:00Z',
    input: { prompt: [{ role: 'user', content: [{ type: 'text', text: 'Hi' }] }], options: {} },
    provider: { providerId: 'openai', provider: 'openai.chat', modelId: 'test-model' },
    exchanges: [], outcome: { status: 'success' }, complete: true, transportClosed: true,
  }
  const vectors = [
    { op: 'provider.create', params: { package: 'openai', settings: { apiKey: 'local-test-key' } }, save: 'provider' },
    { handle: '$provider', op: 'provider.model', params: { modelId: 'test-model', method: 'languageModel' }, save: 'model' },
    { handle: '$model', op: 'model.info' },
    { handle: '$provider', op: 'provider.files', save: 'files' },
    { op: 'registry.create', params: { namespace: 'test', entries: [{ key: 'main', provider: '$provider' }] }, save: 'registry' },
    { handle: '$registry', op: 'registry.provider', params: { key: 'main' }, save: 'lookup' },
    { op: 'trace.create', params: { capacity: 16 }, save: 'trace' },
    { handle: '$trace', op: 'trace.aggregate', params: { filter: {} } },
    { op: 'sessionStore.create', params: { infer: false }, save: 'sessions' },
    { handle: '$sessions', op: 'sessionStore.list' },
    { op: 'recorder.create', params: { backend: { type: 'ring', capacity: 16 }, capturePolicy: 'full' }, save: 'recorder' },
    { handle: '$recorder', op: 'recorder.export' },
    { op: 'replay.create', params: { recordingsJsonl: JSON.stringify(recording) }, save: 'replay' },
    { handle: '$replay', op: 'replay.inspect' },
    { handle: '$replay', op: 'replay.dryRun' },
    { op: 'unknown.op' },
    { handle: '$provider', op: 'text.generate' },
    { op: 'provider.create', params: { package: 'does-not-exist' } },
    { op: 'replay.create', params: { recordingsJsonl: '' } },
  ]
  const python = spawnSync('python3', [fileURLToPath(new URL('./fixtures/ffi.py', import.meta.url)), process.env.AIMUX_FFI_LIBRARY], { input: JSON.stringify(vectors), encoding: 'utf8', timeout: 30_000 })
  assert.equal(python.status, 0, python.stderr)
  const runtime = new Runtime()
  const handles = new Map<string, string>()
  function resolve(value: unknown): unknown {
    if (typeof value === 'string' && value.startsWith('$')) return handles.get(value.slice(1))!
    if (Array.isArray(value)) return value.map(resolve)
    if (value && typeof value === 'object') return Object.fromEntries(Object.entries(value).map(([key, item]) => [key, resolve(item)]))
    return value
  }
  const results = []
  try {
    for (const vector of vectors) {
      try {
        let result = JSON.parse(await runtime.call(resolve(vector.handle ?? '0') as string, vector.op, JSON.stringify(resolve(vector.params ?? {}))))
        if (vector.save) {
          handles.set(vector.save, typeof result === 'string' ? result : result.handle)
          result = typeof result === 'string' ? '$' + vector.save : { ...result, handle: '$' + vector.save }
        }
        results.push({ result })
      } catch (error) {
        assert.ok(error instanceof AimuxError)
        results.push({ error: { kind: error.kind, code: error.code, message: error.message, error: error.error, ...(error.retryAfterMs === undefined ? {} : { retryAfterMs: error.retryAfterMs }) } })
      }
    }
    assert.deepEqual(results, JSON.parse(python.stdout))
  } finally { for (const handle of handles.values()) runtime.drop(handle) }
})
