/**
 * 同一个 Rust 核心，Node napi vs Python PyO3 的 FFI 开销对比
 *
 * 两者都打同一个 Node mock server，同一个 prompt，
 * 只测 FFI 边界 + Rust 核心，排除 HTTP 差异。
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

  const model = await napi.openai('test-key', 'gpt-4o', `${uri}/v1`)
  const promptJson = JSON.stringify(prompt)

  // warmup
  for (let i = 0; i < WARMUP; i++) await model.generateText(promptJson)

  // bench
  const samples: number[] = []
  for (let i = 0; i < N; i++) {
    const s = process.hrtime.bigint()
    await model.generateText(promptJson)
    samples.push(Number(process.hrtime.bigint() - s) / 1e6)
  }

  const sorted = [...samples].sort((a, b) => a - b)
  const mean = samples.reduce((s, v) => s + v, 0) / samples.length
  const pct = (p: number) => sorted[Math.ceil(p / 100 * sorted.length) - 1]

  console.log(`\n  Node napi aimux (N=${N})`)
  console.log(`    mean=${mean.toFixed(3)}ms P50=${pct(50).toFixed(3)} P95=${pct(95).toFixed(3)} P99=${pct(99).toFixed(3)}`)
  console.log(`    (Python bench 报告 mean=0.084ms，对比这个数字)\n`)

  await server.close()
}

main().catch(console.error)
