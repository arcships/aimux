// Example: stream text with OpenAI
//
// Run: OPENAI_API_KEY=sk-... node --experimental-strip-types examples/stream-text.ts

import { openai, streamText } from '../src/index.ts'

async function main() {
  const apiKey = process.env.OPENAI_API_KEY
  if (!apiKey) {
    console.error('Please set OPENAI_API_KEY')
    process.exit(1)
  }

  const model = await openai(apiKey, 'gpt-4o-mini')

  console.log('Streaming:\n')
  for await (const part of streamText(model, 'Write a haiku about Rust.')) {
    // TextStreamPart is a tagged union — check the type field
    if (part.type === 'text-delta') {
      process.stdout.write(part.delta)
    } else if (part.type === 'finish') {
      console.log('\n\n[done]')
    }
  }
}

main().catch(console.error)
