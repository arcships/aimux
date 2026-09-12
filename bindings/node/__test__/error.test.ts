// Error contract tests: AiMuxError failures are typed `AimuxError` subclasses;
// recorder failures are the independent `RecordingError` hierarchy; binding
// failures are napi-rs's own plain `Error` (napi `code`).
// All tests are hermetic — no network beyond 127.0.0.1.

import test from 'ava'
import { createServer, type Server } from 'node:http'
import {
  openai,
  provider,
  mockReplay,
  streamText,
  AimuxError,
  APICallError,
  RetryError,
  InvalidArgumentError,
  NoSuchProviderError,
  RecordingError,
  initRecording,
  initRecordingRing,
  recordingStop,
  recordingTryFlush,
} from '../src/index.ts'
import * as native from '../src/native.ts'
import { mkdtempSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

function startMockServer(
  handler: (req: any, res: any) => void,
): Promise<{ server: Server; url: string }> {
  return new Promise((resolve) => {
    const server = createServer(handler)
    server.listen(0, '127.0.0.1', () => {
      const addr = server.address() as any
      resolve({ server, url: `http://127.0.0.1:${addr.port}` })
    })
  })
}

test('sync native failure throws a typed AimuxError subclass', (t) => {
  const err = t.throws(() => mockReplay('')) as AimuxError
  t.true(err instanceof AimuxError)
  t.true(err instanceof InvalidArgumentError)
  t.is(err.name, 'InvalidArgumentError')
  t.false('code' in err)
  t.false('status' in err)
})

test('async native failure throws a typed AimuxError subclass', async (t) => {
  const err = (await t.throwsAsync(() =>
    provider('definitely-not-a-provider', null, 'some-model'),
  )) as AimuxError
  t.true(err instanceof AimuxError)
  t.true(err instanceof NoSuchProviderError)
  t.is(err.constructor, NoSuchProviderError)
  t.false('code' in err)
  t.false('status' in err)
})

test('stream error path yields a typed AimuxError', async (t) => {
  // Unreachable port: connection is refused immediately; no retries.
  const model = await openai('test-key', 'gpt-4o', {
    baseUrl: 'http://127.0.0.1:1',
    maxRetries: 0,
  })
  const err = (await t.throwsAsync(async () => {
    for await (const _part of streamText(model, 'hello')) {
      // never yields
    }
  })) as AimuxError
  t.true(err instanceof APICallError)
  t.false('code' in err)
  // No HTTP response arrived, so no sentinel properties are invented.
  t.false('status' in err)
  t.false('retryMs' in err)
})

test('instanceof narrows to the specific subclass only', async (t) => {
  const err = (await t.throwsAsync(() =>
    provider('definitely-not-a-provider', null, 'some-model'),
  )) as AimuxError
  t.true(err instanceof NoSuchProviderError)
  t.false(err instanceof APICallError)
  t.false(err instanceof InvalidArgumentError)
})

test('payload fields live on the subclass the core fills them for', async (t) => {
  const err = (await t.throwsAsync(() =>
    provider('definitely-not-a-provider', null, 'some-model'),
  )) as NoSuchProviderError
  t.true(err instanceof NoSuchProviderError)

  // The registry payload is a typed field; there is no JSON to dig it out of.
  t.is(err.providerId, 'definitely-not-a-provider')

  // ApiCall-only fields are absent here — the core fills them for no other
  // variant, so they must not exist as always-empty properties.
  const apiCallOnly = ['status', 'retryMs', 'providerCode', 'responseBody', 'url', 'requestBodyValues', 'responseHeaders', 'data']
  for (const key of apiCallOnly) {
    t.false(key in err, `${key} must not exist on ${err.name}`)
  }
  t.false('retryable' in err)
})

test('APICallError carries every field the response produced', async (t) => {
  const body = JSON.stringify({
    error: { message: 'slow down', type: 'rate_limit_exceeded' },
  })
  const { server, url } = await startMockServer((_req, res) => {
    res.writeHead(429, {
      'content-type': 'application/json',
      'x-request-id': 'req_abc123',
      'retry-after-ms': '1500',
    })
    res.end(body)
  })
  try {
    const model = await native.openai('test-key', 'gpt-4o', { baseUrl: url, maxRetries: 0 })
    const err = (await t.throwsAsync(() =>
      model.generateText(JSON.stringify('hi')),
    )) as APICallError
    t.true(err instanceof APICallError)
    t.true(err instanceof native.APICallError)
    t.is(err.constructor, APICallError)

    t.is(err.status, 429)
    t.is(err.retryMs, 1500)
    t.true(err.retryable)
    t.is(err.providerCode, 'rate_limit_exceeded')
    t.is(err.responseBody, body)
    // Request context is carried on the error itself.
    t.true(err.url?.startsWith(url), `url should start with ${url}: ${err.url}`)
    t.is((err.requestBodyValues as { model?: string }).model, 'gpt-4o')
    // Sanitized response headers, as sent (retryMs above is derived from them).
    t.is(err.responseHeaders?.['retry-after-ms'], '1500')
    // The provider's text on its own; `message` is the composed form.
    t.is(err.providerMessage, 'slow down')
    t.true(err.message.includes('slow down'))
    t.not(err.message, err.providerMessage)
  } finally {
    await new Promise<void>((resolve) => server.close(() => resolve()))
  }
})

test('exhausted retries throw RetryError with the per-attempt history', async (t) => {
  const body = JSON.stringify({
    error: { message: 'slow down', type: 'rate_limit_exceeded' },
  })
  const { server, url } = await startMockServer((_req, res) => {
    // retry-after-ms 0 keeps the retry loop instant.
    res.writeHead(429, { 'content-type': 'application/json', 'retry-after-ms': '0' })
    res.end(body)
  })
  try {
    const model = await native.openai('test-key', 'gpt-4o', { baseUrl: url, maxRetries: 1 })
    const err = (await t.throwsAsync(() =>
      model.generateText(JSON.stringify('hi')),
    )) as RetryError
    t.true(err instanceof RetryError)
    t.true(err instanceof AimuxError)
    t.false(err instanceof APICallError)
    t.is(err.reason, 'maxRetriesExceeded')

    // Complete per-attempt history, oldest first, each a full typed error.
    t.is(err.errors.length, 2)
    for (const attempt of err.errors) {
      t.true(attempt instanceof APICallError)
      t.is((attempt as APICallError).status, 429)
      t.is((attempt as APICallError).providerCode, 'rate_limit_exceeded')
    }
    t.is(err.lastError, err.errors[1])
    t.regex(err.message, /Failed after 2 attempts/)
    // Retry exhaustion is not itself an API exchange: no ApiCall-only fields.
    t.false('status' in err)
    t.false('retryable' in err)
  } finally {
    await new Promise<void>((resolve) => server.close(() => resolve()))
  }
})

test('recorder init failures are RecordingError; flush is a no-op when none is installed', (t) => {
  // Nothing recording: nothing to flush is a success.
  recordingStop()
  t.notThrows(() => recordingTryFlush())

  // A regular file in the parent position makes recorder initialization fail
  // before a recorder is installed. This is Init, not a later WriterGone.
  const dir = mkdtempSync(join(tmpdir(), 'aimux-node-rec-'))
  const blocker = join(dir, 'occupied')
  writeFileSync(blocker, 'x')
  try {
    const err = t.throws(() => initRecording(join(blocker, 'sub')))
    t.true(err instanceof RecordingError)
    t.is(err.code, 'Init')
    // A recorder failure is not an AiMuxError failure: separate type.
    t.false(err instanceof AimuxError)
    // Nothing was installed: the checked flush still succeeds.
    t.notThrows(() => recordingTryFlush())
  } finally {
    recordingStop()
  }
})

test('typed initRecordingRing keeps core errors in the AimuxError hierarchy', (t) => {
  const err = t.throws(() => initRecordingRing(0))
  t.true(err instanceof InvalidArgumentError)
  t.false(err instanceof RecordingError)
})

test('malformed prompt JSON is a plain napi InvalidArg error, not a core error', async (t) => {
  const model = await openai('test-key', 'gpt-4o', { baseUrl: 'http://127.0.0.1:1', maxRetries: 0 })
  // The typed generateText() always stringifies; hit the raw JSON-text path.
  const err = (await t.throwsAsync(() => model.generateText('{not json'))) as Error & {
    code?: string
  }
  t.is(err.constructor, Error)
  t.is(err.code, 'InvalidArg')
  t.regex(err.message, /prompt_json/)
  t.false(err instanceof AimuxError)
  t.false(err instanceof RecordingError)
})

test('sync wire-JSON failure is napi InvalidArg; business validation stays core-owned', (t) => {
  const err = t.throws(() => mockReplay('{not json')) as Error & { code?: string }
  t.is(err.code, 'InvalidArg')
  t.regex(err.message, /recordings_jsonl/)
  t.false(err instanceof AimuxError)
  // Well-formed JSON that fails the schema is what the core would reject.
  t.true(t.throws(() => mockReplay('{}')) instanceof InvalidArgumentError)
})
