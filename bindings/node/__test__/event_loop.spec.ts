// Run with npm run test:lifecycle; requires a real native addon.
import test from 'node:test'
import assert from 'node:assert/strict'
import { createServer, type ServerResponse } from 'node:http'
import { once } from 'node:events'
import { performance } from 'node:perf_hooks'
import { setTimeout as delay } from 'node:timers/promises'
import { Worker } from 'node:worker_threads'
import { spawn } from 'node:child_process'
import * as native from '../src/native.ts'

// This adapter is also serialized into the worker and shutdown child process.
function createAdapter(binding: typeof native) {
  const runtime = new binding.Runtime()
  const params = JSON.stringify({ prompt: 'hello', options: { maxRetries: 0 } })
  return {
    async model(url: string) {
      const provider = JSON.parse(await runtime.call('0', 'provider.create', JSON.stringify({
        package: 'deepseek', settings: { apiKey: 'local-test-key', baseUrl: url },
      })))
      const model = JSON.parse(await runtime.call(provider.handle, 'provider.model', JSON.stringify({
        modelId: 'test-model', method: 'languageModel',
      })))
      runtime.drop(provider.handle)
      return model.handle as string
    },
    async call(handle: string) {
      return JSON.parse(await runtime.call(handle, 'text.generate', params))
    },
    stream(handle: string, signal?: AbortSignal) {
      return runtime.stream(handle, 'text.stream', params, signal ? runtime.abort(signal) : undefined)
    },
    part(value: string) {
      const frame = JSON.parse(value)
      if (frame.type === 'event') return frame.event as { type: string; text?: string }
      if (frame.outcome === 'error') throw new Error(JSON.stringify(frame.error))
      return { type: 'end' }
    },
  }
}
const api = createAdapter(native)
const nativeUrl = new URL('../src/native.ts', import.meta.url).href
const LIMIT_MS = 4_000

async function bounded<T>(promise: PromiseLike<T>, label: string, ms = LIMIT_MS): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined
  try {
    return await Promise.race([
      promise,
      new Promise<never>((_, reject) => {
        timer = setTimeout(() => reject(new Error(`${label} exceeded ${ms}ms`)), ms)
      }),
    ])
  } finally {
    clearTimeout(timer)
  }
}

function latch() {
  let resolve!: () => void
  const promise = new Promise<void>((done) => { resolve = done })
  return { promise, resolve }
}

async function fixture(mode: 'call' | 'finite' | 'flood' | 'idle') {
  const started = latch()
  const disconnected = latch()
  let requests = 0
  let sent = 0
  let closed = false
  const producers = new Set<Promise<void>>()
  const server = createServer((req, res) => {
    requests++
    req.resume()
    res.on('close', () => {
      closed = true
      disconnected.resolve()
    })
    const producer = produce(res).catch((error) => {
      if (!res.destroyed) res.destroy(error)
    })
    producers.add(producer)
    void producer.finally(() => producers.delete(producer))
  })
  async function produce(res: ServerResponse) {
    if (mode === 'call') {
      started.resolve()
      await delay(600)
      if (res.destroyed) return
      res.writeHead(200, { 'content-type': 'application/json' })
      res.end(JSON.stringify({
        id: 'local', model: 'test-model',
        choices: [{ message: { role: 'assistant', content: 'ok' }, finish_reason: 'stop' }],
        usage: { prompt_tokens: 1, completion_tokens: 1, total_tokens: 2 },
      }))
      return
    }
    res.writeHead(200, { 'content-type': 'text/event-stream' })
    res.flushHeaders()
    started.resolve()
    if (mode === 'idle') return // Remains open until the native client cancels.
    const count = mode === 'finite' ? 12 : 4096
    for (let i = 0; i < count && !res.destroyed; i++) {
      const content = mode === 'finite' ? `${i},` : 'x'.repeat(32 * 1024)
      const chunk = `data: ${JSON.stringify({
        id: 'local', model: 'test-model', choices: [{ delta: { content } }],
      })}\n\n`
      sent++
      if (!res.write(chunk)) {
        // `close` must also release a producer waiting on socket backpressure.
        await new Promise<void>((resolve) => {
          const done = () => {
            res.off('drain', done)
            res.off('close', done)
            resolve()
          }
          res.once('drain', done)
          res.once('close', done)
        })
      }
      if (mode === 'finite') await delay(2)
    }
    if (!res.destroyed) res.end('data: {"id":"local","model":"test-model","choices":[{"delta":{},"finish_reason":"stop"}]}\n\ndata: [DONE]\n\n')
  }
  server.listen(0, '127.0.0.1')
  await once(server, 'listening')
  const address = server.address()
  if (!address || typeof address === 'string') throw new Error('Expected TCP address')
  return {
    url: `http://127.0.0.1:${address.port}`,
    started: started.promise,
    disconnected: disconnected.promise,
    requests: () => requests,
    sent: () => sent,
    closed: () => closed,
    async close() {
      const closed = new Promise<void>((resolve) => server.close(() => resolve()))
      server.closeAllConnections()
      await bounded(closed, 'fixture close')
      await bounded(Promise.all(producers), 'fixture producers')
    },
  }
}

function worker(body: string, data: unknown) {
  return new Worker(`
    const { parentPort, workerData } = require('node:worker_threads');
    (async () => {
      const api = (${createAdapter.toString()})(await import(${JSON.stringify(nativeUrl)}));
      ${body}
    })().catch(error => { throw error; });
  `, { eval: true, workerData: data, execArgv: ['--experimental-strip-types'] })
}

// Serial execution keeps CPU contention between these timing scenarios out of the measurements.
test('event loop: timers advance with bounded drift during a native call', async (t) => {
  const f = await fixture('call')
  t.after(() => f.close())
  const model = await api.model(f.url)
  const gaps: number[] = []
  let previous = performance.now()
  const timer = setInterval(() => {
    const now = performance.now()
    gaps.push(now - previous)
    previous = now
  }, 10)
  try {
    const result = await bounded(api.call(model), 'generate call')
    gaps.push(performance.now() - previous)
    assert.equal(result.text, 'ok')
    assert.ok(gaps.length >= 15, `Only ${gaps.length} timer samples during the delayed call`)
    assert.ok(Math.max(...gaps) < 250, `Maximum timer gap: ${Math.max(...gaps)}ms`)
  } finally {
    clearInterval(timer)
  }
})

test('stream lifecycle: slow consumer preserves every text delta and terminates', async (t) => {
  const f = await fixture('finite')
  t.after(() => f.close())
  const controller = new AbortController()
  t.after(() => controller.abort())
  const stream = await api.stream(await api.model(f.url), controller.signal)
  let text = ''
  await bounded((async () => {
    for await (const value of stream) {
      const part = api.part(value)
      if (part.type === 'text-delta') text += part.text
      await delay(15)
    }
  })(), 'slow stream')
  assert.equal(text, Array.from({ length: 12 }, (_, i) => `${i},`).join(''))
  assert.ok((await bounded(stream.next(), 'next after completion')).done)
})

test('stream lifecycle: AbortSignal settles a pending next and closes transport', async (t) => {
  const f = await fixture('idle')
  t.after(() => f.close())
  const controller = new AbortController()
  t.after(() => controller.abort())
  const stream = await api.stream(await api.model(f.url), controller.signal)
  // Drain any metadata events until the iterator actually waits for provider data.
  const draining = (async () => {
    try {
      for await (const value of stream) {
        const part = api.part(value)
        if (part.type === 'error') return 'error'
      }
      return 'done'
    } catch (error) {
      if (!(error instanceof Error) || !/abort|cancel/i.test(`${error.name} ${error.message}`)) throw error
      return 'cancelled'
    }
  })()
  void draining.catch(() => {}) // Observe failures even if request startup times out.
  await bounded(f.started, 'stream request')
  controller.abort()
  assert.ok(['error', 'done', 'cancelled'].includes(await bounded(draining, 'aborted next')))
  await bounded(f.disconnected, 'aborted transport')
})

for (const action of ['abort', 'return'] as const) {
  test(`stream lifecycle: ${action} releases an unconsumed full queue`, async (t) => {
    const f = await fixture('flood')
    t.after(() => f.close())
    const controller = new AbortController()
    t.after(() => controller.abort())
    const stream = await api.stream(await api.model(f.url), controller.signal)
    await bounded(f.started, 'unconsumed request')
    await delay(250) // Let the unconsumed bounded bridge fill.
    assert.ok(f.sent() > 64, `Fixture sent only ${f.sent()} chunks`)
    assert.equal(f.closed(), false, 'The producer must still be active before cancellation')
    if (action === 'abort') controller.abort()
    else {
      assert.equal(typeof stream.return, 'function')
      assert.ok((await bounded(stream.return(undefined), 'unconsumed return')).done)
    }
    await bounded(f.disconnected, `${action} closes unconsumed transport`)
  })
}

test('worker realms: a live foreign model is rejected before provider I/O', async (t) => {
  const f = await fixture('call')
  t.after(() => f.close())
  const model = await api.model(f.url)
  const w = worker(`
    parentPort.once('message', async foreign => {
      try {
        await api.call(foreign);
        parentPort.postMessage({ accepted: true });
      } catch (error) {
        parentPort.postMessage({ accepted: false, code: error.code, message: error.message });
      } finally { parentPort.close(); }
    });
    parentPort.postMessage('ready');
  `, null)
  t.after(() => w.terminate())
  await bounded(once(w, 'message'), 'worker ready')
  const reply = once(w, 'message')
  w.postMessage(model)
  const [result] = await bounded(reply, 'foreign handle rejection')
  assert.equal(result.accepted, false)
  assert.equal(result.code, 203)
  assert.equal(f.requests(), 0)
})

for (const mode of ['idle', 'flood'] as const) {
  test(`environment shutdown: outstanding ${mode} stream does not hang cleanup`, async (t) => {
    const f = await fixture(mode)
    t.after(() => f.close())
    // Use a process boundary: a native cleanup deadlock can also hang
    // Worker.terminate(), whereas the parent can kill this disposable process.
    const source = `
      const api = (${createAdapter.toString()})(await import(${JSON.stringify(nativeUrl)}));
      const model = await api.model(${JSON.stringify(f.url)});
      globalThis.liveStream = await api.stream(model);
      // Allow transport startup and queue saturation, then let Node exit naturally.
      await new Promise(resolve => setTimeout(resolve, 250));
    `
    const child = spawn(process.execPath, [
      '--experimental-strip-types', '--input-type=module', '--eval', source,
    ], { stdio: ['ignore', 'ignore', 'pipe'] })
    t.after(() => { child.kill('SIGKILL') })
    let stderr = ''
    child.stderr.on('data', (chunk) => { stderr += chunk.toString() })
    const exited = once(child, 'exit')
    void exited.catch(() => {}) // Request startup is checked before process exit.
    await bounded(f.started, 'child provider request')
    const [code, signal] = await bounded(exited, 'natural environment shutdown', 6_000)
    assert.equal(code, 0, stderr)
    assert.equal(signal, null)
    await bounded(f.disconnected, 'shutdown transport cleanup')
  })
}
