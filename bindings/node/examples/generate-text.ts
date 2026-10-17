import { Client, LanguageModel } from '../src/index.ts'

const client = new Client()
const provider = await client.provider({ package: 'openai', settings: { apiKey: process.env.OPENAI_API_KEY } })
const model = await provider.model('gpt-4o-mini', 'languageModel')
try {
  if (!(model instanceof LanguageModel)) throw new Error('Expected a language model')
  const result = await model.generate('Explain what an async iterator does.')
  console.log(result.text)
} finally {
  await model.close()
  await provider.close()
}
