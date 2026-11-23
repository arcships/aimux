// Runs against a real napi build, separately from the mocked transport facade suite.
import test from 'node:test'
import assert from 'node:assert/strict'
import { Worker } from 'node:worker_threads'
import { Runtime, AimuxError } from '../src/native.ts'
import * as ops from '../src/generated/ops.ts'

const source = new URL('../src/native.ts', import.meta.url).href

test('native errors preserve the shared envelope and reject invalid wire JSON', async () => {
  const runtime = new Runtime()
  await assert.rejects(runtime.call('0', 'unknown.op', '{}'), error => error instanceof AimuxError && error.kind === 'protocol' && error.code === 300 && error.error.name === 'InvalidOp')
  await assert.rejects(runtime.call('0', ops.OP_RUNTIME_DESCRIBE, '{'), error => error instanceof AimuxError && error.kind === 'binding' && error.code === 202)
  await assert.rejects(runtime.call('9007199254740993', ops.OP_MODEL_INFO, '{}'), error => error instanceof AimuxError && error.code === 203 && !('actual' in error.error))
})

test('runtime instances in one Env share ownership and native AbortSignal is immediate', async () => {
  const first = new Runtime()
  const second = new Runtime()
  const controller = new AbortController()
  const abort = first.abort(controller.signal)
  await second.call(abort.handle, ops.OP_ABORT_SIGNAL, '{}')
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
        await runtime.call(workerData.handle, workerData.op, '{}')
        parentPort.postMessage({ unexpectedSuccess: true })
      } catch (error) {
        parentPort.postMessage({ kind: error.kind, code: error.code, error: error.error })
      }
    })().catch(error => { throw error })
  `, { eval: true, workerData: { source, handle: abort.handle, op: ops.OP_ABORT_SIGNAL }, execArgv: ['--experimental-strip-types'] })
  const exit = new Promise(resolve => worker.once('exit', resolve))
  const message = await new Promise<Record<string, unknown>>((resolve, reject) => { worker.once('message', resolve); worker.once('error', reject) })
  assert.equal(message.kind, 'binding')
  assert.equal(message.code, 203)
  assert.equal('actual' in (message.error as object), false)
  const timeout = new Promise((_, reject) => { const timer = setTimeout(() => reject(new Error('Worker teardown hung')), 5000); timer.unref() })
  await Promise.race([exit, timeout])
  assert.equal(abort.aborted, false)
})

test('typed lifecycle uses the real provider, registry, files, composite and devtools DTOs', async () => {
  const { Client, LanguageModel } = await import('../src/index.ts')
  const client = new Client()
  const provider = await client.provider({ package: 'openai', settings: { apiKey: 'local-test-key' } })
  const registry = await client.registry({ main: provider }, 'test')
  const resolved = await registry.resolve({ namespace: 'test', key: 'main', method: 'languageModel', modelId: 'test-model' })
  assert.ok(resolved instanceof LanguageModel)
  const raw = await client.call(resolved.handle, ops.OP_MODEL_INFO)
  assert.deepEqual(await resolved.info(), raw)
  const files = await provider.files()
  assert.equal(files.kind, 'Files')
  assert.match(files.handle, /^[1-9][0-9]*$/)
  const router = await client.router({ children: [resolved.handle], strategy: { type: 'rule' } })
  assert.equal(router.kind, 'Language')
  const trace = await client.trace({ capacity: 16, perScopeCapacity: 8 })
  assert.equal(trace.kind, 'TraceStore')
  assert.deepEqual(await trace.aggregate(), await client.call(trace.handle, ops.OP_TRACE_AGGREGATE, { filter: {} }))
  const middleware = await client.middleware({ kind: 'trace', traceStore: trace.handle })
  assert.equal(middleware.kind, 'Middleware')
  assert.match(middleware.handle, /^[1-9][0-9]*$/)
  const wrapped = await resolved.wrap([middleware])
  assert.equal(wrapped.kind, 'Language')
  assert.deepEqual(await wrapped.info(), raw)
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
  await Promise.all([registry, resolved, files, router, trace, middleware, wrapped, recorder, sessions].map(resource => resource.close()))
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
    assert.deepEqual(await replay.inspect(), await client.call(replay.handle, ops.OP_REPLAY_INSPECT))
    assert.deepEqual(await replay.dryRun(), await client.call(replay.handle, ops.OP_REPLAY_DRY_RUN))
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
    await assert.rejects(client.stream(model.handle, ops.OP_TEXT_STREAM, { unexpected: true }), error => error instanceof AimuxError && error.code === 301)
    const returned = await model.stream('hello', {}, { signal: controller.signal })
    const pending = returned.next()
    await returned.return!()
    assert.equal((await pending).done, true)
  } finally { await Promise.all([model.close(), provider.close()]) }
})

test('omitted bodies stay available to trace and recording', async (t) => {
  const { Client } = await import('../src/index.ts')
  const { createServer } = await import('node:http')
  const { once } = await import('node:events')
  const requests: string[] = []
  const server = createServer(async (req, res) => {
    const chunks: Buffer[] = []
    for await (const chunk of req) chunks.push(chunk)
    const body = Buffer.concat(chunks).toString()
    requests.push(body)
    if (JSON.parse(body).stream === true) {
      res.writeHead(200, { 'content-type': 'text/event-stream' })
      res.end('data: ' + JSON.stringify({
        id: 'local', model: 'test-model',
        choices: [{ delta: { content: 'ok' }, finish_reason: 'stop' }],
      }) + '\n\ndata: [DONE]\n\n')
      return
    }
    res.writeHead(200, { 'content-type': 'application/json' })
    res.end(JSON.stringify({
      id: 'local', model: 'test-model',
      choices: [{ message: { role: 'assistant', content: 'ok' }, finish_reason: 'stop' }],
      usage: { prompt_tokens: 1, completion_tokens: 1, total_tokens: 2 },
    }))
  })
  server.listen(0, '127.0.0.1')
  await once(server, 'listening')
  t.after(() => { server.close(); server.closeAllConnections() })
  const address = server.address()
  assert.ok(address && typeof address === 'object')

  const client = new Client()
  const provider = await client.provider({
    package: 'deepseek',
    settings: { apiKey: 'local-test-key', baseUrl: `http://127.0.0.1:${address.port}` },
  })
  const model = await provider.model('test-model', 'languageModel')
  const trace = await client.trace({ capacity: 16 })
  const middleware = await client.middleware({ kind: 'trace', traceStore: trace.handle })
  const wrapped = await model.wrap([middleware])
  const recorder = await client.recorder({ backend: { type: 'ring', capacity: 16 }, capturePolicy: 'full' })
  try {
    await recorder.attach()
    const prompt = 'x'.repeat(1024)
    const compact = await wrapped.generate(prompt)
    assert.equal(compact.text, 'ok')
    assert.equal(compact.output, 'ok')
    assert.equal(Object.hasOwn(compact.request, 'body'), false)
    assert.equal(Object.hasOwn(compact.response, 'body'), false)
    assert.deepEqual(Object.keys(JSON.parse(JSON.stringify(compact))).sort(), ['steps', 'totalUsage'])

    const full = await wrapped.generate(prompt, { include: { requestBody: true, responseBody: true } })
    assert.deepEqual(full.request.body, JSON.parse(requests[1]!))
    assert.equal((full.response.body as { id: string }).id, 'local')

    const streamed = await wrapped.consume({ prompt })
    assert.equal(streamed.text, 'ok')
    assert.equal(streamed.output, 'ok')
    assert.equal(Object.hasOwn(streamed.request, 'body'), false)
    assert.deepEqual(Object.keys(JSON.parse(JSON.stringify(streamed))).sort(), ['steps', 'totalUsage'])
    const completion = await client.call(wrapped.handle, ops.OP_OUTPUT_TO_OPEN_AI, {
      result: compact, metadataNamespace: 'deepseek',
    }) as { choices: Array<{ message: { content: string } }> }
    assert.equal(completion.choices[0]?.message.content, 'ok')

    const traceJsonl = await trace.export() as string
    const records = traceJsonl.trim().split('\n').map(line => JSON.parse(line))
    assert.equal(records.length, 3)
    const fingerprints = [compact, full, streamed].map(result =>
      records.find(record => record.callId === result.finalStep.callId).fingerprint)
    assert.ok(fingerprints.every(fingerprint => fingerprint.lenBytes > 1024))
    assert.equal(fingerprints[0].bodyHash, fingerprints[1].bodyHash)

    await recorder.flush()
    const { jsonl } = await recorder.export() as { jsonl: string }
    const recordings = jsonl.trim().split('\n').map(line => JSON.parse(line))
    assert.equal(recordings.length, 3)
    for (const [result, request] of [[compact, requests[0]], [streamed, requests[2]]] as const) {
      assert.equal(recordings.find(record => record.callId === result.finalStep.callId)
        .exchanges[0].request.body, request)
    }
  } finally {
    await recorder.stop()
    for (const resource of [recorder, wrapped, middleware, trace, model, provider]) resource.close()
  }
})

test('unparsed tool-call generation sets outputParsed=false and exposes undefined output', async (t) => {
  const { Client } = await import('../src/index.ts')
  const { createServer } = await import('node:http')
  const { once } = await import('node:events')
  const server = createServer((_req, res) => {
    res.writeHead(200, { 'content-type': 'application/json' })
    res.end(JSON.stringify({
      id: 'local-tool', model: 'test-model',
      choices: [{
        message: {
          role: 'assistant',
          content: 'Calling tool',
          tool_calls: [{ id: 'call_1', type: 'function', function: { name: 'ping', arguments: '{}' } }],
        },
        finish_reason: 'tool_calls',
      }],
      usage: { prompt_tokens: 1, completion_tokens: 1, total_tokens: 2 },
    }))
  })
  server.listen(0, '127.0.0.1')
  await once(server, 'listening')
  t.after(() => { server.close(); server.closeAllConnections() })
  const address = server.address()
  assert.ok(address && typeof address === 'object')

  const client = new Client()
  const provider = await client.provider({
    package: 'deepseek',
    settings: { apiKey: 'local-test-key', baseUrl: `http://127.0.0.1:${address.port}` },
  })
  const model = await provider.model('test-model', 'languageModel')
  try {
    const result = await model.generate('call ping', {
      tools: [{ type: 'function', name: 'ping', inputSchema: { type: 'object', properties: {} } }],
    })
    assert.equal(result.finishReason, 'tool-calls')
    assert.equal(result.text, 'Calling tool')
    assert.equal(result.outputParsed, false)
    assert.equal(result.output, undefined)
    assert.equal(Object.hasOwn(result, 'output'), false)
    assert.deepEqual(Object.keys(JSON.parse(JSON.stringify(result))).sort(), ['outputParsed', 'steps', 'totalUsage'])
  } finally {
    await Promise.all([model.close(), provider.close()])
  }
})
