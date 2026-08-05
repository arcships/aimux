// Node.js contract test runner — validates wire format consistency.
//
// Uses the same JSON fixtures as the Rust contract tests.
// Run: node --experimental-strip-types run-node.ts

import { readFileSync } from 'node:fs'
import { join, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'

const __dirname = dirname(fileURLToPath(import.meta.url))
const fixturesPath = join(__dirname, 'fixtures', 'wire-format.json')
const fixtures = JSON.parse(readFileSync(fixturesPath, 'utf-8'))

let passed = 0
let failed = 0

function assert(condition: boolean, name: string, message?: string) {
  if (condition) {
    passed++
    // console.log(`  ✔ ${name}`)
  } else {
    failed++
    console.error(`  ✗ ${name}${message ? ': ' + message : ''}`)
  }
}

// ── ToolChoice wire format ─────────────────────────────────────────────────

function testToolChoice() {
  console.log('ToolChoice:')

  // The native module returns JSON strings; we verify the wire format
  // by checking that the expected JSON parses correctly.
  for (const f of fixtures.filter((f: any) => f.type === 'ToolChoice')) {
    const parsed = JSON.parse(f.json)
    if (f.name === 'tool_choice_auto') {
      assert(parsed === 'auto', f.name, `expected "auto", got ${JSON.stringify(parsed)}`)
    } else if (f.name === 'tool_choice_none') {
      assert(parsed === 'none', f.name, `expected "none", got ${JSON.stringify(parsed)}`)
    } else if (f.name === 'tool_choice_required') {
      assert(parsed === 'required', f.name, `expected "required", got ${JSON.stringify(parsed)}`)
    } else if (f.name === 'tool_choice_tool') {
      assert(parsed.type === 'tool' && parsed.toolName === 'get_weather', f.name,
        `expected {type:"tool",toolName:"get_weather"}, got ${JSON.stringify(parsed)}`)
    }
  }
}

// ── StreamPart wire format ─────────────────────────────────────────────────

function testStreamPart() {
  console.log('StreamPart:')

  for (const f of fixtures.filter((f: any) => f.type === 'StreamPart')) {
    const parsed = JSON.parse(f.json)
    const key = Object.keys(parsed)[0]

    if (f.name === 'stream_part_text_delta') {
      assert(key === 'TextDelta' && parsed.TextDelta.delta === 'Hello', f.name,
        `expected TextDelta with delta "Hello", got ${JSON.stringify(parsed)}`)
    } else if (f.name === 'stream_part_finish') {
      assert(key === 'Finish' && parsed.Finish.usage !== undefined, f.name,
        `expected Finish with usage, got ${JSON.stringify(parsed)}`)
    } else if (f.name === 'stream_part_stream_start') {
      assert(key === 'StreamStart' && Array.isArray(parsed.StreamStart.warnings), f.name,
        `expected StreamStart with warnings array, got ${JSON.stringify(parsed)}`)
    }
  }
}

// ── Role wire format ───────────────────────────────────────────────────────

function testRole() {
  console.log('Role:')

  for (const f of fixtures.filter((f: any) => f.type === 'Role')) {
    const parsed = JSON.parse(f.json)
    if (f.name === 'role_user') {
      assert(parsed === 'user', f.name, `expected "user", got ${JSON.stringify(parsed)}`)
    } else if (f.name === 'role_system') {
      assert(parsed === 'system', f.name, `expected "system", got ${JSON.stringify(parsed)}`)
    }
  }
}

// ── GenerateTextOptions wire format ────────────────────────────────────────

function testGenerateTextOptions() {
  console.log('GenerateTextOptions:')

  for (const f of fixtures.filter((f: any) => f.type === 'GenerateTextOptions')) {
    const parsed = JSON.parse(f.json)
    assert(typeof parsed === 'object' && parsed !== null, f.name, 'expected object')

    if (f.name === 'generate_text_options_default') {
      // All fields should be null
      const allNull = Object.values(parsed).every((v: any) => v === null)
      assert(allNull, f.name, 'all fields should be null for default')
    }

    if (f.name === 'generate_text_options_with_session_id') {
      // RFC-0024: explicit session_id crosses the wire (snake_case).
      assert(
        parsed.session_id === 'sess-1',
        f.name,
        `expected session_id "sess-1", got ${JSON.stringify(parsed.session_id)}`,
      )
    }
  }
}

// ── ModelMessage wire format ───────────────────────────────────────────────

function testModelMessage() {
  console.log('ModelMessage:')

  for (const f of fixtures.filter((f: any) => f.type === 'ModelMessage')) {
    const parsed = JSON.parse(f.json)
    if (f.name === 'model_message_text') {
      assert(parsed.role === 'user' && parsed.content === 'Hello', f.name,
        `expected role:"user", content:"Hello", got ${JSON.stringify(parsed)}`)
    }
  }
}

// ── Native module smoke test ───────────────────────────────────────────────

async function testNativeModule() {
  console.log('Native module:')

  try {
    // The napi-generated index.js uses require() (CommonJS), so use createRequire.
    const { createRequire } = await import('node:module')
    const require = createRequire(import.meta.url)
    const { openai } = require('../bindings/node/index.js')
    assert(typeof openai === 'function', 'native module loads', 'openai is not a function')

    const model = await openai('sk-test-fake-key', 'gpt-4o-mini')
    assert(model !== undefined, 'model creation', 'model is undefined')
    assert(typeof model.generateText === 'function', 'generateText method', 'missing method')
    assert(typeof model.streamText === 'function', 'streamText method', 'missing method')
  } catch (e) {
    assert(false, 'native module', String(e))
  }
}

// ── Run all tests ──────────────────────────────────────────────────────────

async function main() {
  console.log(`\nContract tests (${fixtures.length} fixtures)\n`)

  testToolChoice()
  testStreamPart()
  testRole()
  testGenerateTextOptions()
  testModelMessage()
  await testNativeModule()

  console.log(`\n${passed} passed, ${failed} failed\n`)
  process.exit(failed > 0 ? 1 : 0)
}

main()
