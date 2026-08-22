/**
 * aimux vs AISDK — 直接对比，不设 B0 基线。
 *
 * 同一进程、同一 mock server、同一 prompt，
 * 只看两个 SDK 的端到端延迟差异。
 */

import { startMockServer } from './mock-server.ts'
import { nativeBinaryPath } from './native.ts'
import { createRequire } from 'node:module'
const require = createRequire(import.meta.url)
const napi = require(nativeBinaryPath()) as {
  openai: (apiKey: string, modelId: string, baseUrl?: string) => Promise<{
    generateText: (prompt: string, opts?: string) => Promise<string>
  }>
}

async function main() {
  const server = await startMockServer()
  const uri = server.uri
  const N = 300
  const WARMUP = 30
  const prompt = 'Explain Rust ownership in one sentence.'

  // ── AISDK init ──
    const { createOpenAI } = await import('@ai-sdk/openai')
  const openai = createOpenAI({ apiKey: 'test-key', baseURL: `${uri}/v1` })
  const aisdkModel = openai.chat('gpt-4o') as { doGenerate: (opts: unknown) => Promise<unknown> }

  // ── aimux init ──
  const aimuxModel = await napi.openai('test-key', 'gpt-4o', `${uri}/v1`)

  // ── warmup ──
  for (let i = 0; i < WARMUP; i++) {
    await aimuxModel.generateText(JSON.stringify(prompt))
  }
  const aisdkOpts = { prompt: [{ role: 'user', content: [{ type: 'text', text: prompt }] }], mode: { type: 'regular' } }
  for (let i = 0; i < WARMUP; i++) {
    await aisdkModel.doGenerate(aisdkOpts)
  }

  // ── aimux bench ──
  const aimuxT: number[] = []
  for (let i = 0; i < N; i++) {
    const s = process.hrtime.bigint()
    await aimuxModel.generateText(JSON.stringify(prompt))
    aimuxT.push(Number(process.hrtime.bigint() - s) / 1e6)
  }

  // ── AISDK bench ──
  const aisdkT: number[] = []
  for (let i = 0; i < N; i++) {
    const s = process.hrtime.bigint()
    await aisdkModel.doGenerate(aisdkOpts)
    aisdkT.push(Number(process.hrtime.bigint() - s) / 1e6)
  }

  // ── stats ──
  const pct = (a: number[], p: number) => [...a].sort((x, y) => x - y)[Math.ceil(p / 100 * a.length) - 1]
  const mean = (a: number[]) => a.reduce((s, v) => s + v, 0) / a.length

  const aimuxMean = mean(aimuxT)
  const aisdkMean = mean(aisdkT)

  console.log()
  console.log(`  N=${N}, warmup=${WARMUP}, mock=${uri}`)
  console.log()
  console.log(`  aimux:  mean=${aimuxMean.toFixed(3)}ms  P50=${pct(aimuxT, 50).toFixed(3)}  P95=${pct(aimuxT, 95).toFixed(3)}  P99=${pct(aimuxT, 99).toFixed(3)}`)
  console.log(`  AISDK:  mean=${aisdkMean.toFixed(3)}ms  P50=${pct(aisdkT, 50).toFixed(3)}  P95=${pct(aisdkT, 95).toFixed(3)}  P99=${pct(aisdkT, 99).toFixed(3)}`)
  console.log()
  console.log(`  aimux 比 AISDK 快 ${(aisdkMean / aimuxMean).toFixed(1)}x`)
  console.log(`  每次请求节省 ${(aisdkMean - aimuxMean).toFixed(3)}ms`)
  console.log()

  await server.close()
}

main().catch(console.error)
