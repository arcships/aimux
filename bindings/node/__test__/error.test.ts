// Error contract tests: every failure surfaced by the binding must be an
// `AimuxError` subclass (instanceof works) carrying `.code` / `.status`.
// Payload fields belong to the one subclass the core fills them for.
// All tests are hermetic — no network beyond 127.0.0.1.

import test from 'ava'
import { createServer, type Server } from 'node:http'
import {
  openai,
  provider,
  mockReplay,
  streamText,
  generateText,
  AimuxError,
  APICallError,
  InvalidArgumentError,
  NoSuchProviderError,
} from '../src/index.ts'

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
  t.is(err.code, 'InvalidArgument')
  t.is(typeof err.status, 'number')
  // Binding-side failures also synthesize an AiMuxError, so errorValue is set.
  t.is(typeof err.errorValue, 'string')
  t.true(Object.hasOwn(JSON.parse(err.errorValue!), 'InvalidArgument'))
})

test('async native failure throws a typed AimuxError subclass', async (t) => {
  const err = (await t.throwsAsync(() =>
    provider('definitely-not-a-provider', null, 'some-model'),
  )) as AimuxError
  t.true(err instanceof AimuxError)
  t.true(err instanceof NoSuchProviderError)
  t.is(err.code, 'NoSuchProvider')
})

test('errorValue carries the raw serde JSON of the core AiMuxError', async (t) => {
  const err = (await t.throwsAsync(() =>
    provider('definitely-not-a-provider', null, 'some-model'),
  )) as AimuxError
  t.is(typeof err.errorValue, 'string')
  t.true(err.errorValue!.includes('NoSuchProvider'))
  const parsed = JSON.parse(err.errorValue!)
  t.deepEqual(Object.keys(parsed), ['NoSuchProvider'])
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
  t.true(err instanceof AimuxError)
  t.is(typeof err.code, 'string')
  t.true(err.code.length > 0)
  t.is(typeof err.status, 'number')
})

test('instanceof narrows to the specific subclass only', async (t) => {
  const err = (await t.throwsAsync(() =>
    provider('definitely-not-a-provider', null, 'some-model'),
  )) as AimuxError
  t.true(err instanceof NoSuchProviderError)
  t.false(err instanceof APICallError)
  t.false(err instanceof InvalidArgumentError)

  // Direct construction carries the given status through.
  const rl = new APICallError('slow down', 429, 1500)
  t.is(rl.status, 429)
  t.is(rl.code, 'ApiCall')
  t.is(rl.name, 'APICallError')
})

test('payload fields live on the subclass the core fills them for', async (t) => {
  const err = (await t.throwsAsync(() =>
    provider('definitely-not-a-provider', null, 'some-model'),
  )) as NoSuchProviderError
  t.true(err instanceof NoSuchProviderError)

  // The registry payload is a field, not something to dig out of errorValue.
  t.is(err.providerId, 'definitely-not-a-provider')

  // ApiCall-only fields are absent here — the core fills them for no other
  // variant, so they must not exist as always-empty properties.
  for (const key of ['retryMs', 'retryable', 'providerCode', 'responseBody', 'requestId']) {
    t.false(key in err, `${key} must not exist on ${err.name}`)
  }

  // …and they do exist on an ApiCall error.
  const call = new APICallError('slow down', 429, 1500, true)
  t.is(call.retryMs, 1500)
  t.true(call.retryable)
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
    const model = await openai('test-key', 'gpt-4o', { baseUrl: url, maxRetries: 0 })
    const err = (await t.throwsAsync(() => generateText(model, 'hi'))) as APICallError
    t.true(err instanceof APICallError)

    t.is(err.status, 429)
    t.is(err.retryMs, 1500)
    t.true(err.retryable)
    t.is(err.providerCode, 'rate_limit_exceeded')
    t.is(err.requestId, 'req_abc123')
    t.is(err.responseBody, body)
    // The provider's text on its own; `message` is the composed form.
    t.is(err.providerMessage, 'slow down')
    t.true(err.message.includes('slow down'))
    t.not(err.message, err.providerMessage)
  } finally {
    await new Promise<void>((resolve) => server.close(() => resolve()))
  }
})
