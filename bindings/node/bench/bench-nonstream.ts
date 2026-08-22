/**
 * 维度一·速度：非流式单请求对比
 *
 * 三路测量（同一进程、同一 mock server）：
 *   B0 — undici 直调 mock（无 SDK 基线）
 *   aimux — napi → Rust → reqwest → mock
 *   aisdk — @ai-sdk/openai → undici → mock
 *
 * 每路跑 N 次取 P50/P95/P99。
 *
 * 注意：aimux 用 raw napi API（直接 JSON.stringify/parse），
 * 绕过 ts wrapper 层，测的是 Rust 核心 + FFI 的真实开销。
 */

import { startMockServer } from './mock-server.ts'
import { nativeBinaryPath } from './native.ts'
import { createRequire } from 'node:module'
const require = createRequire(import.meta.url)
// raw napi .node — 直接加载原生二进制，绕过 ts wrapper 的循环 re-export
const napi = require(nativeBinaryPath()) as { openai: (apiKey: string, modelId: string, baseUrl?: string) => Promise<{ generateText: (prompt: string, opts?: string) => Promise<string> }> }

// ── B0: undici 直调 ──────────────────────────────────────────────────────
async function benchB0(uri: string): Promise<number> {
  const start = process.hrtime.bigint()
  const resp = await fetch(`${uri}/v1/chat/completions`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', Authorization: 'Bearer test-key' },
    body: JSON.stringify({
      model: 'gpt-4o',
      messages: [{ role: 'user', content: 'Explain Rust ownership in one sentence.' }],
      max_tokens: 50,
    }),
  })
  await resp.text()
  const end = process.hrtime.bigint()
  return Number(end - start) / 1e6 // ms
}

// ── aimux (napi raw API) ───────────────────────────────────────────────────
// raw napi: openai(apiKey, modelId, baseUrl) → Model
//           model.generateText(promptJson, optsJson) → resultJson
async function benchAimux(uri: string): Promise<number> {
  const model = await napi.openai('test-key', 'gpt-4o', `${uri}/v1`)
  const prompt = JSON.stringify('Explain Rust ownership in one sentence.')
  const start = process.hrtime.bigint()
  const result = await model.generateText(prompt)
  const end = process.hrtime.bigint()
  // 消费 result（确保不被优化掉）
  if (!result) throw new Error('empty')
  return Number(end - start) / 1e6
}

// ── AISDK ─────────────────────────────────────────────────────────────────
// 只用 @ai-sdk/openai 的 createOpenAI + model.doGenerate（LanguageModelV1 接口），
// 不依赖 ai 包的 generateText（那层包含 agent loop 等额外逻辑）。
// 这样和 aimux 的 generateText 对等——都是 provider 接入层。
let aisdkModel: { doGenerate: (opts: unknown) => Promise<unknown> } | null = null

async function initAisdk(uri: string) {
  const { createOpenAI } = await import('@ai-sdk/openai')
  const openai = createOpenAI({ apiKey: 'test-key', baseURL: `${uri}/v1` })
  // 用 .chat() 走 chat completions 接口（默认 .responses() 走 Responses API）
  aisdkModel = openai.chat('gpt-4o') as never
}

async function benchAisdk(): Promise<number> {
  if (!aisdkModel) throw new Error('aisdk not init')
  const opts = {
    prompt: [{ role: 'user', content: [{ type: 'text', text: 'Explain Rust ownership in one sentence.' }] }],
    mode: { type: 'regular' },
  }
  const start = process.hrtime.bigint()
  await aisdkModel.doGenerate(opts)
  const end = process.hrtime.bigint()
  return Number(end - start) / 1e6
}

// ── 统计工具 ───────────────────────────────────────────────────────────────
function percentile(sorted: number[], p: number): number {
  const idx = Math.ceil((p / 100) * sorted.length) - 1
  return sorted[Math.max(0, idx)]
}

function stats(samples: number[]) {
  const sorted = [...samples].sort((a, b) => a - b)
  const sum = sorted.reduce((s, v) => s + v, 0)
  return {
    n: sorted.length,
    mean: sum / sorted.length,
    p50: percentile(sorted, 50),
    p95: percentile(sorted, 95),
    p99: percentile(sorted, 99),
    min: sorted[0],
    max: sorted[sorted.length - 1],
  }
}

// ── 主流程 ─────────────────────────────────────────────────────────────────
async function main() {
  const server = await startMockServer()
  const uri = server.uri
  const N = 200
  const WARMUP = 20

  // 初始化 AISDK（createOpenAI 是同步的，但 import 是异步的）
  await initAisdk(uri)

  console.log(`\n╔══════════════════════════════════════════════════════════╗`)
  console.log(`║  非流式单请求对比 (N=${N}, warmup=${WARMUP})              ║`)
  console.log(`║  mock: ${uri.padEnd(48)}║`)
  console.log(`╚══════════════════════════════════════════════════════════╝\n`)

  const results: Record<string, ReturnType<typeof stats>> = {}

  const benches: [string, () => Promise<number>][] = [
    ['B0 undici', () => benchB0(uri)],
    ['aimux', () => benchAimux(uri)],
    ['AISDK', () => benchAisdk()],
  ]

  for (const [name, fn] of benches) {
    process.stdout.write(`  ${name}: warmup... `)
    // 预热
    for (let i = 0; i < WARMUP; i++) {
      await fn()
    }
    process.stdout.write(`running ${N}... `)

    // 计时
    const samples: number[] = []
    for (let i = 0; i < N; i++) {
      samples.push(await fn())
    }
    results[name] = stats(samples)
    process.stdout.write(`done (mean=${results[name].mean.toFixed(2)}ms)\n`)
  }

  // 打印结果
  const fmt = (v: number) => v.toFixed(3).padStart(8)
  console.log('\n┌──────────────┬──────────┬──────────┬──────────┬──────────┬──────────┐')
  console.log('│ SDK          │     mean │      P50 │      P95 │      P99 │      min │')
  console.log('├──────────────┼──────────┼──────────┼──────────┼──────────┼──────────┤')
  for (const [name, s] of Object.entries(results)) {
    console.log(
      `│ ${name.padEnd(12)} │ ${fmt(s.mean)} │ ${fmt(s.p50)} │ ${fmt(s.p95)} │ ${fmt(s.p99)} │ ${fmt(s.min)} │`
    )
  }
  console.log('└──────────────┴──────────┴──────────┴──────────┴──────────┴──────────┘')

  // 差值分析
  const b0 = results['B0 undici'].mean
  const aimux = results['aimux'].mean
  const aisdk = results['AISDK'].mean
  console.log('\n── 差值分析 (ms) ──────────────────────────')
  console.log(`  B0 (无 SDK)     = ${b0.toFixed(3)}`)
  console.log(`  aimux 开销      = ${(aimux - b0).toFixed(3)}  (含 napi FFI)`)
  console.log(`  AISDK 开销      = ${(aisdk - b0).toFixed(3)}`)
  console.log(`  aimux vs AISDK  = ${aimux > aisdk ? '+' : ''}${(aimux - aisdk).toFixed(3)} (${aimux < aisdk ? 'aimux 快' : 'AISDK 快'})`)
  console.log()

  await server.close()
}

main().catch(console.error)
