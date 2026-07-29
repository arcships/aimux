import test from 'ava'
import { openai, anthropic, deepseek } from '../index.js'

// These tests verify the native module loads and the API surface works.
// They do NOT make real API calls — they test error handling for invalid keys.

test('native module loads and exports functions', (t) => {
  t.is(typeof openai, 'function')
  t.is(typeof anthropic, 'function')
  t.is(typeof deepseek, 'function')
})

test('openai() creates a model instance with valid key format', async (t) => {
  // Even with a fake key, the provider should construct (the key is only
  // validated on the first API call, not at construction time).
  const model = await openai('sk-test-fake-key', 'gpt-4o-mini')
  t.truthy(model)
  t.is(typeof model.generateText, 'function')
  t.is(typeof model.streamText, 'function')
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
