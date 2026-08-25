/**
 * aimux vs OpenAI 官方 Node SDK — 对等对比
 *
 * 两者抽象层次一致：都是 HTTP + JSON，不做编排/schema 验证/中间件。
 * 之前对比的 @ai-sdk/openai（Vercel AI SDK）多了 zod 验证/中间件/telemetry，
 * 导致 11x 的优势有水分。这里补一个干净的对比。
 */

import { createRequire } from 'node:module'
import { nativeBinaryPath } from './native.ts'
const require = createRequire(import.meta.url)
const napi = require(nativeBinaryPath()) as {
  openai: (apiKey: string, modelId: string, baseUrl?: string) => Promise<{
    generateText: (prompt: string, opts?: string) => Promise<string>
  }>
}

import { startMockServer } from './mock-server.ts'

async function main() {
  const server = await startMockServer()
  const uri = server.uri
  const N = 300
  const WARMUP = 30
  const prompt = 'Explain Rust ownership in one sentence.'

  // OpenAI 官方 Node SDK
  const openaiModule = await import('openai')
  const OpenAI = openaiModule.default ?? (openaiModule as { OpenAI?: unknown }).OpenAI
  const client = new OpenAI({ apiKey: 'test-key', baseURL: `${uri}/v1` })

  // aimux
  const model = await napi.openai('test-key', 'gpt-4o', `${uri}/v1`)
  const promptJson = JSON.stringify(prompt)

  // warmup
  for (let i = 0; i < WARMUP; i++) await model.generateText(promptJson)
  for (let i = 0; i < WARMUP; i++) await client.chat.completions.create({
    model: 'gpt-4o', messages: [{ role: 'user', content: prompt }], max_tokens: 50,
  })

  function rss() { return Math.round(process.memoryUsage().rss / 1024 / 1024) }

  const pct = (a: number[], p: number) => [...a].sort((x, y) => x - y)[Math.ceil(p / 100 * a.length) - 1]
  const mean = (a: number[]) => a.reduce((s, v) => s + v, 0) / a.length

  console.log(`\n  Node.js: aimux vs OpenAI 官方 SDK (N=${N}, warmup=${WARMUP})`)
  console.log(`  mock: ${uri}\n`)

  // aimux
  const rss1Start = rss()
  const aimuxT: number[] = []
  for (let i = 0; i < N; i++) {
    const s = process.hrtime.bigint()
    await model.generateText(promptJson)
    aimuxT.push(Number(process.hrtime.bigint() - s) / 1e6)
  }
  const rss1End = rss()

  // OpenAI SDK
  const rss2Start = rss()
  const openaiT: number[] = []
  for (let i = 0; i < N; i++) {
    const s = process.hrtime.bigint()
    await client.chat.completions.create({
      model: 'gpt-4o', messages: [{ role: 'user', content: prompt }], max_tokens: 50,
    })
    openaiT.push(Number(process.hrtime.bigint() - s) / 1e6)
  }
  const rss2End = rss()

  console.log('  SDK                 mean     P50     P95     P99        RSS')
  console.log('  ' + '─'.repeat(60))
  const aimuxLabel = 'aimux (Rust)'
  const openaiLabel = 'OpenAI Node SDK'
  const rss1Str = rss1Start + '->' + rss1End + 'MB'
  const rss2Str = rss2Start + '->' + rss2End + 'MB'
  console.log('  ' + aimuxLabel.padEnd(20) + ' ' + mean(aimuxT).toFixed(3).padStart(8) + ' ' + pct(aimuxT,50).toFixed(3).padStart(8) + ' ' + pct(aimuxT,95).toFixed(3).padStart(8) + ' ' + pct(aimuxT,99).toFixed(3).padStart(8) + ' ' + rss1Str.padStart(10))
  console.log('  ' + openaiLabel.padEnd(20) + ' ' + mean(openaiT).toFixed(3).padStart(8) + ' ' + pct(openaiT,50).toFixed(3).padStart(8) + ' ' + pct(openaiT,95).toFixed(3).padStart(8) + ' ' + pct(openaiT,99).toFixed(3).padStart(8) + ' ' + rss2Str.padStart(10))

  const am = mean(aimuxT)
  const om = mean(openaiT)
  console.log(`\n  aimux vs OpenAI SDK = ${(om/am).toFixed(1)}x (aimux 快)`)
  console.log(`  RSS 差值: aimux +${rss1End-rss1Start}MB | OpenAI +${rss2End-rss2Start}MB\n`)

  await server.close()
}

main().catch(console.error)
