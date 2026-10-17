// Test double for the JSON transport only; this does not validate aimux-ops.
const ops = require('../../src/generated/ops.ts')
let nextHandle = 9007199254740993n
const handles = new Map()
const envelope = (kind, code, name, message, fields = {}) => Object.assign(new Error(message), {
  kind, code, error: { name, ...fields },
})
class Runtime {
  constructor() { this.owner = 1 }
  manifestJson() { return JSON.stringify({ abiVersion: ops.ABI_VERSION, opsVersion: ops.OPS_VERSION, digest: ops.MANIFEST_DIGEST, manifestRevision: 'fixture', ops: [], handleKinds: [], errorCodes: [] }) }
  manifestDigest() { return ops.MANIFEST_DIGEST }
  create(kind) {
    const handle = String(nextHandle++)
    handles.set(handle, { owner: this.owner, kind })
    return { handle, kind }
  }
  check(id) {
    if (id === '0') return
    if (handles.get(id)?.owner !== this.owner) throw envelope('binding', 203, 'InvalidHandle', 'Invalid handle', { expected: [] })
  }
  dropHandle(id) { if (handles.get(id)?.owner === this.owner) handles.delete(id) }
  async call(id, op, json, abort) {
    const params = JSON.parse(json)
    if (op === ops.OP_HANDLE_DROP) {
      if (handles.get(id)?.owner === this.owner) handles.delete(id)
      return '{}'
    }
    this.check(id)
    if (abort) this.check(abort.handle)
    if (abort?.aborted) throw envelope('aimux', 18, 'AI_AbortError', 'Aborted')
    switch (op) {
      case ops.OP_PROVIDER_CREATE: case ops.OP_PROVIDER_DEFAULT: case ops.OP_PROVIDER_CUSTOM: case ops.OP_REGISTRY_PROVIDER: return JSON.stringify(this.create('Provider'))
      case ops.OP_REGISTRY_CREATE: for (const id of params.entries) this.check(id.provider); return JSON.stringify(this.create('Registry'))
      case ops.OP_PROVIDER_MODEL: return JSON.stringify(this.create(({ embeddingModel: 'Embedding', imageModel: 'Image', speechModel: 'Speech', transcriptionModel: 'Transcription', rerankingModel: 'Reranking', videoModel: 'Video', searchModel: 'Search', decision: 'Decision' })[params.method] || 'Language'))
      case ops.OP_TRACE_CREATE: return JSON.stringify(this.create('TraceStore').handle)
      case ops.OP_RECORDER_CREATE: return JSON.stringify({ handle: this.create('Recorder').handle })
      case ops.OP_SESSION_STORE_CREATE: return JSON.stringify({ handle: this.create('SessionStore').handle })
      case ops.OP_REPLAY_CREATE: return JSON.stringify({ handle: this.create('ReplaySession').handle })
      case ops.OP_MIDDLEWARE_CREATE: return JSON.stringify(this.create('Middleware'))
      case ops.OP_TRANSCRIPTION_START: return JSON.stringify(this.create('Operation').handle)
      case ops.OP_TEXT_GENERATE: return JSON.stringify({ text: params.prompt, nested: params.options, raw: null })
      case ops.OP_OPERATION_NEXT: return JSON.stringify({ type: 'ended' })
      case 'fixture.error': throw Object.assign(envelope('aimux', 1, 'AI_APICallError', 'rate limit', { statusCode: 429, isRetryable: true, responseHeaders: { 'retry-after': '0' }, data: { unicode: '你好' } }), { retryAfterMs: 0 })
      default: return JSON.stringify({ op, params })
    }
  }
  async stream(id, op, json, abort) {
    this.check(id)
    const params = JSON.parse(json)
    if (params.reject) throw envelope('protocol', 301, 'InvalidParams', 'Rejected before acceptance')
    let index = 0
    let closed = false
    let resolvePending
    const values = params.values || [
      { type: 'event', seq: '9007199254740993', event: { type: 'error', error: { name: 'AI_APICallError' } } },
      { type: 'end', seq: '9007199254740994', outcome: 'success' },
    ]
    const iterator = {
      next: async () => {
        if (closed) return { done: true, value: undefined }
        if (params.pending) return new Promise(resolve => { resolvePending = resolve })
        if (index === values.length) return { done: true, value: undefined }
        return { done: false, value: JSON.stringify(values[index++]) }
      },
      return: async () => { closed = true; if (abort) abort.signal(); resolvePending?.({ done: true, value: undefined }); return { done: true, value: undefined } },
      throw: async error => { await iterator.return(); throw error },
      [Symbol.asyncIterator]() { return this },
    }
    return { [Symbol.asyncIterator]() { return iterator } }
  }
  async operationPush(id, data) { this.check(id); this.lastPush = new Uint8Array(data) }
}
class Abort {
  constructor(runtime, signal) {
    this.handle = runtime.create('Abort').handle
    this.aborted = signal?.aborted || false
    signal?.addEventListener('abort', () => this.signal(), { once: true })
  }
  signal() { if (this.handle !== "0") this.aborted = true }
  close() { this.handle = "0" }
}
module.exports = { Runtime, Abort }
