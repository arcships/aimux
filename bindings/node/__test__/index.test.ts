import test from 'ava'
import { openai, anthropic, deepseek, provider, initRecordingRing, recordingStop } from '../index.js'

// These tests verify the native module loads and the API surface works.
// They do NOT make real API calls — they test error handling for invalid keys.

test('native module loads and exports functions', (t) => {
  t.is(typeof openai, 'function')
  t.is(typeof anthropic, 'function')
  t.is(typeof deepseek, 'function')
  t.is(typeof provider, 'function')
})

test('openai() creates a model instance with valid key format', async (t) => {
  // Even with a fake key, the provider should construct (the key is only
  // validated on the first API call, not at construction time).
  const model = await openai('sk-test-fake-key', 'gpt-4o-mini')
  t.truthy(model)
  t.is(typeof model.generateText, 'function')
  t.is(typeof model.streamText, 'function')
})

test('provider() creates a registry model instance (RFC-0017 phase 4)', async (t) => {
  const model = await provider('groq', 'sk-test-fake-key', 'llama-3.3-70b')
  t.truthy(model)
  t.is(typeof model.generateText, 'function')
  t.is(typeof model.streamText, 'function')
})

test('provider() with undefined apiKey reads the registry env var', async (t) => {
  // GROQ_API_KEY is unset in CI — the construction must fail with a clear
  // error (missing env key), proving the env-var path is wired.
  await t.throwsAsync(
    () => provider('abacus', undefined, 'm'),
    { message: /api key/i },
  )
})

test('provider() rejects unknown provider names with the available list', async (t) => {
  await t.throwsAsync(
    () => provider('no-such-provider', 'k', 'm'),
    { message: /no-such-provider/ },
  )
})

test('anthropic() creates a model instance', async (t) => {
  const model = await anthropic('sk-ant-test-fake-key', 'claude-3-5-sonnet-20241022')
  t.truthy(model)
  t.is(typeof model.generateText, 'function')
})

test('deepseek() creates a model instance', async (t) => {
  const model = await deepseek('sk-test-fake-key', 'deepseek-chat')
  t.truthy(model)
  t.is(typeof model.streamText, 'function')
})

test('generateText rejects on invalid prompt JSON', async (t) => {
  const model = await openai('sk-test-fake-key', 'gpt-4o-mini')
  // Invalid JSON should produce a napi Error, not a crash
  await t.throwsAsync(
    () => model.generateText('{invalid json}'),
    { message: /invalid prompt/i },
  )
})

test('streamText returns an async generator', async (t) => {
  const model = await openai('sk-test-fake-key', 'gpt-4o-mini')
  const gen = await model.streamText('"hello"')
  // It should have Symbol.asyncIterator
  t.is(typeof gen[Symbol.asyncIterator], 'function')
})

// Omitting cap uses the library default capacity (FFI
// aimux_init_recording_ring_default) and must not throw. Requires the native
// addon to be rebuilt (napi build) with the optional-cap signature.
test('initRecordingRing with no cap uses library default', (t) => {
  t.notThrows(() => initRecordingRing())
  recordingStop()
})
