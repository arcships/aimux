// Error contract tests: every failure surfaced by the binding must be an
// `AimuxError` subclass (instanceof works) carrying `.code` / `.status` /
// `.retryMs`. All tests are hermetic — no network beyond 127.0.0.1.

import test from 'ava'
import {
  openai,
  provider,
  mockReplay,
  streamText,
  AimuxError,
  APICallError,
  InvalidArgumentError,
  NoSuchProviderError,
} from '../src/index.ts'

test('sync native failure throws a typed AimuxError subclass', (t) => {
  const err = t.throws(() => mockReplay('')) as AimuxError
  t.true(err instanceof AimuxError)
  t.true(err instanceof InvalidArgumentError)
  t.is(err.name, 'InvalidArgumentError')
  t.is(err.code, 'InvalidArgument')
  t.is(typeof err.status, 'number')
  t.is(typeof err.retryMs, 'number')
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
  t.is(typeof err.retryMs, 'number')
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
