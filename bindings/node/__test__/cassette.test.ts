// cassette.test.ts — Real cassette replay tests for Node binding.
//
// These tests load ACTUAL recorded provider responses from
// aimux-providers/tests/cassettes/ and verify the full chain:
//   Node.js → napi → Rust engine → cassette replay → parse → typed result
//
// Hard asserts only — no catch-all pass. Providers with non-standard paths
// are excluded (not tested here), not faked.

import test from 'ava'
import { openai, anthropic } from '../src/native.ts'
import { CassetteServer } from './cassette-replay.ts'

// ── OpenAI ──────────────────────────────────────────────────────────────────

test('cassette: OpenAI generate', async (t) => {
  const srv = new CassetteServer('openai')
  await srv.start()
  t.true(srv.count > 0, 'should load OpenAI cassettes')

  try {
    const model = await openai('test-key', 'gpt-4o', `${srv.url}/v1`)
    const resultJson = await model.generateText(JSON.stringify('Hello'))
    const result = JSON.parse(resultJson)

    t.false(!!result.error, `unexpected error: ${result.error}`)
    t.is(typeof result.text, 'string', 'should have text field')
    t.true(result.text.length > 0, 'text should be non-empty')
    t.truthy(result.usage, 'should have usage')
    t.truthy(result.finish_reason, 'should have finish_reason')
  } finally {
    await srv.stop()
  }
})

test('cassette: OpenAI stream', async (t) => {
  const srv = new CassetteServer('openai')
  await srv.start()

  try {
    const model = await openai('test-key', 'gpt-4o', `${srv.url}/v1`)
    const gen = await model.streamText(JSON.stringify('Hello'))
    let parts: any[] = []
    for await (const json of gen) {
      parts.push(JSON.parse(json))
    }
    t.true(parts.length > 0, 'should receive stream parts')

    const types = parts.map((p) => Object.keys(p)[0])
    t.true(types.includes('StreamStart'), 'should have StreamStart')
    t.true(types.includes('Finish'), 'should have Finish')
  } finally {
    await srv.stop()
  }
})

// ── DeepSeek ────────────────────────────────────────────────────────────────

test('cassette: DeepSeek generate', async (t) => {
  const srv = new CassetteServer('deepseek')
  await srv.start()
  t.true(srv.count > 0)

  try {
    const model = await openai('test-key', 'deepseek-chat', srv.url)
    const resultJson = await model.generateText(JSON.stringify('Hello'))
    const result = JSON.parse(resultJson)
    t.false(!!result.error, `unexpected error: ${result.error}`)
    t.is(typeof result.text, 'string', 'should have text')
  } finally {
    await srv.stop()
  }
})

test('cassette: DeepSeek usage.raw carries vendor fields', async (t) => {
  // RFC-0016 M10: DeepSeek's vendor-specific usage fields (not part of the
  // typed Usage model) survive in usage.raw.
  const srv = new CassetteServer('deepseek')
  await srv.start()
  t.true(srv.count > 0)

  try {
    const model = await openai('test-key', 'deepseek-chat', srv.url)
    const resultJson = await model.generateText(JSON.stringify('Hello'))
    const result = JSON.parse(resultJson)
    const raw = result.usage?.raw
    t.truthy(raw, 'usage.raw should be populated (RFC-0016 M10)')
    t.is(typeof raw?.prompt_cache_hit_tokens, 'number')
    t.is(typeof raw?.prompt_cache_miss_tokens, 'number')
  } finally {
    await srv.stop()
  }
})

// ── Anthropic ───────────────────────────────────────────────────────────────

test('cassette: Anthropic generate', async (t) => {
  const srv = new CassetteServer('anthropic')
  await srv.start()
  t.true(srv.count > 0)

  try {
    const model = await anthropic('test-key', 'claude-sonnet-4-6', `${srv.url}/v1`)
    const resultJson = await model.generateText(JSON.stringify('Hello'))
    const result = JSON.parse(resultJson)
    t.false(!!result.error, `unexpected error: ${result.error}`)
    t.is(typeof result.text, 'string', 'should have text')
    t.true(result.text.length > 0, 'text should be non-empty')
    t.truthy(result.usage, 'should have usage')
  } finally {
    await srv.stop()
  }
})

test('cassette: Anthropic stream', async (t) => {
  const srv = new CassetteServer('anthropic')
  await srv.start()

  try {
    const model = await anthropic('test-key', 'claude-3-haiku-20240307', `${srv.url}/v1`)
    const gen = await model.streamText(JSON.stringify('Hello'))
    let parts: any[] = []
    for await (const json of gen) {
      parts.push(JSON.parse(json))
    }
    t.true(parts.length > 0, 'should receive stream parts')
  } finally {
    await srv.stop()
  }
})

// ── Groq ────────────────────────────────────────────────────────────────────

test('cassette: Groq generate', async (t) => {
  const srv = new CassetteServer('groq')
  await srv.start()
  t.true(srv.count > 0)

  try {
    const model = await openai('test-key', 'llama-3.3-70b-versatile', `${srv.url}/openai/v1`)
    const resultJson = await model.generateText(JSON.stringify('Hello'))
    const result = JSON.parse(resultJson)
    t.false(!!result.error, `unexpected error: ${result.error}`)
    t.is(typeof result.text, 'string', 'should have text')
  } finally {
    await srv.stop()
  }
})

// ── Mistral ─────────────────────────────────────────────────────────────────

test('cassette: Mistral generate', async (t) => {
  const srv = new CassetteServer('mistral')
  await srv.start()
  t.true(srv.count > 0)

  try {
    const model = await openai('test-key', 'ministral-8b-latest', `${srv.url}/v1`)
    const resultJson = await model.generateText(JSON.stringify('Hello'))
    const result = JSON.parse(resultJson)
    t.false(!!result.error, `unexpected error: ${result.error}`)
    t.is(typeof result.text, 'string', 'should have text')
  } finally {
    await srv.stop()
  }
})

// ── Ollama ──────────────────────────────────────────────────────────────────

test('cassette: Ollama generate', async (t) => {
  const srv = new CassetteServer('ollama')
  await srv.start()
  t.true(srv.count > 0)

  try {
    const model = await openai('test-key', 'qwen3:4b', `${srv.url}/v1`)
    const resultJson = await model.generateText(JSON.stringify('Hello'))
    const result = JSON.parse(resultJson)
    t.false(!!result.error, `unexpected error: ${result.error}`)
    t.is(typeof result.text, 'string', 'should have text')
  } finally {
    await srv.stop()
  }
})

// ── Perplexity ──────────────────────────────────────────────────────────────

test('cassette: Perplexity generate', async (t) => {
  const srv = new CassetteServer('perplexity')
  await srv.start()
  t.true(srv.count > 0)

  try {
    const model = await openai('test-key', 'sonar', srv.url)
    const resultJson = await model.generateText(JSON.stringify('Hello'))
    const result = JSON.parse(resultJson)
    t.false(!!result.error, `unexpected error: ${result.error}`)
    t.is(typeof result.text, 'string', 'should have text')
  } finally {
    await srv.stop()
  }
})
