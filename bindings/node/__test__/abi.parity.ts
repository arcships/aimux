import test from 'node:test'
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { Runtime, AimuxError } from '../src/native.ts'
import * as ops from '../src/generated/ops.ts'

test('Node dispatch and the C ABI agree on provider, registry, devtools and errors', async () => {
  assert.ok(process.env.AIMUX_FFI_LIBRARY, 'Set AIMUX_FFI_LIBRARY to the built aimux-ffi library')
  const recording = {
    schema: 2, callId: 'node-c-parity', recordedAt: '2026-10-15T00:00:00Z',
    input: { prompt: [{ role: 'user', content: [{ type: 'text', text: 'Hi' }] }], options: {} },
    provider: { providerId: 'openai', provider: 'openai.chat', modelId: 'test-model' },
    exchanges: [], outcome: { status: 'success' }, complete: true, transportClosed: true,
  }
  const vectors = [
    { op: ops.OP_PROVIDER_CREATE, params: { package: 'openai', settings: { apiKey: 'local-test-key' } }, save: 'provider' },
    { handle: '$provider', op: ops.OP_PROVIDER_MODEL, params: { modelId: 'test-model', method: 'languageModel' }, save: 'model' },
    { handle: '$model', op: ops.OP_MODEL_INFO },
    { handle: '$provider', op: ops.OP_PROVIDER_FILES, save: 'files' },
    { op: ops.OP_REGISTRY_CREATE, params: { namespace: 'test', entries: [{ key: 'main', provider: '$provider' }] }, save: 'registry' },
    { handle: '$registry', op: ops.OP_REGISTRY_PROVIDER, params: { key: 'main' }, save: 'lookup' },
    { op: ops.OP_TRACE_CREATE, params: { capacity: 16 }, save: 'trace' },
    { handle: '$trace', op: ops.OP_TRACE_AGGREGATE, params: { filter: {} } },
    { op: ops.OP_SESSION_STORE_CREATE, params: { infer: false }, save: 'sessions' },
    { handle: '$sessions', op: ops.OP_SESSION_STORE_LIST },
    { op: ops.OP_RECORDER_CREATE, params: { backend: { type: 'ring', capacity: 16 }, capturePolicy: 'full' }, save: 'recorder' },
    { handle: '$recorder', op: ops.OP_RECORDER_EXPORT },
    { op: ops.OP_REPLAY_CREATE, params: { recordingsJsonl: JSON.stringify(recording) }, save: 'replay' },
    { handle: '$replay', op: ops.OP_REPLAY_INSPECT },
    { handle: '$replay', op: ops.OP_REPLAY_DRY_RUN },
    { op: 'unknown.op' },
    { handle: '$provider', op: ops.OP_TEXT_GENERATE },
    { op: ops.OP_PROVIDER_CREATE, params: { package: 'does-not-exist' } },
    { op: ops.OP_REPLAY_CREATE, params: { recordingsJsonl: '' } },
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
