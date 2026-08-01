// Example: generate text with OpenAI
//
// Run: OPENAI_API_KEY=sk-... node --experimental-strip-types examples/generate-text.ts

import { openai, generateText } from '../src/index.ts'

async function main() {
  const apiKey = process.env.OPENAI_API_KEY
  if (!apiKey) {
    console.error('Please set OPENAI_API_KEY')
    process.exit(1)
  }

  const model = await openai(apiKey, 'gpt-4o-mini')

  const result = await generateText(model, 'Explain Rust ownership in one sentence.')
  console.log('Text:', result.text)
  console.log('Usage:', result.usage)
  console.log('Finish reason:', result.finish_reason)
}

main().catch(console.error)
