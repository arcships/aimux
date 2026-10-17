import { Client, LanguageModel } from '../src/index.ts'

const client = new Client()
const provider = await client.provider({ package: 'openai', settings: { apiKey: process.env.OPENAI_API_KEY } })
const model = await provider.model('gpt-4o-mini', 'languageModel')
try {
  if (!(model instanceof LanguageModel)) throw new Error('Expected a language model')
  for await (const frame of await model.stream('Write a short poem.')) {
    if (frame.type === 'event' && frame.event.type === 'text-delta') process.stdout.write(frame.event.text)
    if (frame.type === 'end' && frame.error) console.error(frame.error)
  }
} finally {
  await model.close()
  await provider.close()
}
