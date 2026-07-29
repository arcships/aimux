// Example: stream text with OpenAI
//
// Run: OPENAI_API_KEY=sk-... node --experimental-strip-types examples/stream-text.ts

import { openai, streamText } from '../index.ts'
import type { StreamPart } from '../index.ts'

async function main() {
  const apiKey = process.env.OPENAI_API_KEY
  if (!apiKey) {
    console.error('Please set OPENAI_API_KEY')
    process.exit(1)
  }

  const model = await openai(apiKey, 'gpt-4o-mini')

  console.log('Streaming:\n')
  for await (const part of streamText(model, 'Write a haiku about Rust.')) {
    // StreamPart is a tagged union — check the type field
    const obj = part as Record<string, unknown>
    const key = Object.keys(obj)[0]
    if (key === 'TextDelta') {
      process.stdout.write((obj[key] as { delta: string }).delta)
    } else if (key === 'Finish') {
      console.log('\n\n[done]')
    }
  }
}

main().catch(console.error)
