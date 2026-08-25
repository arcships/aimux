import { createRequire } from 'node:module'
const require = createRequire(import.meta.url)

async function main() {
    const { createOpenAI } = await import('@ai-sdk/openai')

  const uri = 'http://127.0.0.1:36131'
  const openai = createOpenAI({ apiKey: 'test-key', baseURL: uri + '/v1' })
  const model = openai.chat('gpt-4o')

  const ctx = 'x '.repeat(100000) // 200KB
  const opts = { prompt: [{role:'user',content:[{type:'text',text:ctx}]}], mode: {type:'regular'} }

  function rss() {
    return Math.round(process.memoryUsage().rss/1024/1024)
  }

  console.log('初始 RSS:', rss(), 'MB')

  // warmup
  for (let i=0;i<10;i++) await (model as any).doGenerate(opts)

  const start = rss()
  for (let i=0;i<2000;i++) await (model as any).doGenerate(opts)
  const end = rss()
  console.log('AISDK @ai-sdk/openai 2000次大prompt:', start, 'MB ->', end, 'MB (+' + (end-start) + 'MB)')

  // 也测一下 openai npm 包（和 Python 用的同一个 SDK）
  const { default: OpenAI } = require('openai')
  const client = new OpenAI({ apiKey: 'test-key', baseURL: uri + '/v1' })

  for (let i=0;i<10;i++) await client.chat.completions.create({ model:'gpt-4o', messages:[{role:'user',content:ctx}], max_tokens:5 })

  const start2 = rss()
  for (let i=0;i<2000;i++) await client.chat.completions.create({ model:'gpt-4o', messages:[{role:'user',content:ctx}], max_tokens:5 })
  const end2 = rss()
  console.log('openai npm SDK 2000次大prompt:', start2, 'MB ->', end2, 'MB (+' + (end2-start2) + 'MB)')
}

main().catch(console.error)
