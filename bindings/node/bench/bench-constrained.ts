/**
 * aimux vs AISDK — CPU 受限 + 内存对比
 *
 * 真实部署场景：容器化，CPU 受限。
 * Rust 无 GC 的优势在此体现：
 *   - Node GC 停顿在高频请求下累积
 *   - Node 内存随并发增长（对象分配 + GC 压力）
 *   - Rust 零分配路径 + 无 GC 停顿
 *
 * 用 taskset 限制到 1-2 核后跑此脚本：
 *   taskset -c 0-1 npx tsx bench/bench-constrained.ts
 *
 * 或脚本内用 OS 调度限制（需要 root，这里靠外部 taskset）。
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

import { createServer as createHttpServer, type IncomingMessage, type ServerResponse } from 'node:http'

function startBigMockServer(responseSize: number) {
  return new Promise<{ uri: string; close: () => Promise<void> }>((resolve) => {
    const chunk = 'x'.repeat(1000)
    const body = JSON.stringify({
      id: 'chatcmpl-mock', object: 'chat.completion', created: 1700000000, model: 'gpt-4o',
      choices: [{ index: 0, message: { role: 'assistant', content: chunk.repeat(responseSize / 1000) }, finish_reason: 'stop' }],
      usage: { prompt_tokens: 5000, completion_tokens: responseSize, total_tokens: responseSize + 5000 },
    })
    const server = createHttpServer((req: IncomingMessage, res: ServerResponse) => {
      req.on('data', () => {})
      req.on('end', () => { res.writeHead(200, { 'Content-Type': 'application/json' }); res.end(body) })
    })
    server.listen(0, '127.0.0.1', () => {
      const addr = server.address()
      const port = typeof addr === 'object' && addr ? addr.port : 0
      resolve({ uri: `http://127.0.0.1:${port}`, close: () => new Promise<void>((r) => server.close(() => r())) })
    })
  })
}

function makeContext(approxBytes: number): string {
  const turn = 'Explain Rust ownership in detail. ' + 'word '.repeat(50)
  const turns: string[] = []; let total = 0, i = 0
  while (total < approxBytes) { turns.push(`Message ${i}: ${turn}`); total += turn.length + 20; i++ }
  return turns.join('\n')
}

const CTX_200K = makeContext(200_000)

// ── 内存采样 ──────────────────────────────────────────────────────────────
function rssMB(): number {
  const m = process.memoryUsage()
  return Math.round(m.rss / 1024 / 1024)
}

// ── bench ────────────────────────────────────────────────────────────────
let aisdkModel: { doGenerate: (opts: unknown) => Promise<unknown> } | null = null
async function initAisdk(uri: string) {
    const { createOpenAI } = await import('@ai-sdk/openai')
  const openai = createOpenAI({ apiKey: 'test-key', baseURL: `${uri}/v1` })
  aisdkModel = openai.chat('gpt-4o') as never
}

function pct(a: number[], p: number) { return [...a].sort((x, y) => x - y)[Math.ceil(p / 100 * a.length) - 1] }
function mean(a: number[]) { return a.reduce((s, v) => s + v, 0) / a.length }

async function main() {
  const server = await startBigMockServer(50_000) // 50KB 响应
  const uri = server.uri
  await initAisdk(uri)
  const model = await napi.openai('test-key', 'gpt-4o', `${uri}/v1`)

  const prompt = CTX_200K // 200KB 上文
  const aisdkOpts = { prompt: [{ role: 'user', content: [{ type: 'text', text: prompt }] }], mode: { type: 'regular' } }
  const promptJson = JSON.stringify(prompt)

  // warmup
  for (let i = 0; i < 10; i++) { await model.generateText(promptJson) }
  for (let i = 0; i < 10; i++) { await aisdkModel!.doGenerate(aisdkOpts) }
  if (global.gc) { global.gc() } // 强制 GC 获得干净基线

  console.log(`\n═══ CPU 受限 + 内存对比 (200KB 上文, 50KB 响应) ═══`)
  console.log(`  提示：建议用 taskset -c 0-1 npx tsx bench/bench-constrained.ts 限制 2 核\n`)
  console.log(`  CPU 核数: ${require('node:os').cpus().length}`)
  console.log(`  初始 RSS: ${rssMB()} MB\n`)

  // ===== 持续压测：连续 2000 请求，测 GC 停顿 + 内存增长 =====
  console.log('── 持续压测 (2000 请求) ──\n')

  for (const [name, fn] of [
    ['aimux', async () => { await model.generateText(promptJson) }],
    ['AISDK', async () => { await aisdkModel!.doGenerate(aisdkOpts) }],
  ] as const) {
    if (global.gc) global.gc()
    const startRSS = rssMB()
    const latencies: number[] = []
    const rssSamples: number[] = []

    const start = process.hrtime.bigint()
    for (let i = 0; i < 2000; i++) {
      const s = process.hrtime.bigint()
      await fn()
      const ms = Number(process.hrtime.bigint() - s) / 1e6
      latencies.push(ms)
      if (i % 200 === 0) rssSamples.push(rssMB())
    }
    const totalMs = Number(process.hrtime.bigint() - start) / 1e6

    const endRSS = rssMB()
    const m = mean(latencies)
    const p99 = pct(latencies, 99)
    // GC 停顿指标：P99 - P50 的差值（P99 飙高通常是 GC 停顿）
    const p50 = pct(latencies, 50)
    const tailJitter = p99 - p50

    console.log(`  ${name}:`)
    console.log(`    总耗时:     ${totalMs.toFixed(0)}ms  (${(2000 / totalMs * 1000).toFixed(0)} rps)`)
    console.log(`    延迟:       mean=${m.toFixed(2)}ms  P50=${p50.toFixed(2)}  P95=${pct(latencies, 95).toFixed(2)}  P99=${p99.toFixed(2)}`)
    console.log(`    尾部抖动:   ${tailJitter.toFixed(2)}ms (P99-P50, GC 停顿指标)`)
    console.log(`    内存:       ${startRSS}MB → ${endRSS}MB  (增长 ${endRSS - startRSS}MB)`)
    console.log(`    RSS 采样:   ${rssSamples.join(' → ')} MB`)
    console.log()
  }

  // ===== 并发对比（带内存） =====
  console.log('── 并发对比 (200KB 上文, 总 500 请求) ──\n')

  for (const concurrency of [1, 20, 50]) {
    const total = 500
    const batches = Math.ceil(total / concurrency)

    // aimux
    if (global.gc) global.gc()
    const aimuxStartRSS = rssMB()
    const aimuxStart = process.hrtime.bigint()
    for (let b = 0; b < batches; b++) {
      const batch = Math.min(concurrency, total - b * concurrency)
      await Promise.all(Array.from({ length: batch }, () => model.generateText(promptJson)))
    }
    const aimuxMs = Number(process.hrtime.bigint() - aimuxStart) / 1e6
    const aimuxRSS = rssMB()

    // AISDK
    if (global.gc) global.gc()
    const aisdkStartRSS = rssMB()
    const aisdkStart = process.hrtime.bigint()
    for (let b = 0; b < batches; b++) {
      const batch = Math.min(concurrency, total - b * concurrency)
      await Promise.all(Array.from({ length: batch }, () => aisdkModel!.doGenerate(aisdkOpts)))
    }
    const aisdkMs = Number(process.hrtime.bigint() - aisdkStart) / 1e6
    const aisdkRSS = rssMB()

    console.log(`  并发=${String(concurrency).padStart(2)}:`)
    console.log(`    aimux:  ${aimuxMs.toFixed(0)}ms (${(total / aimuxMs * 1000).toFixed(0)} rps)  RSS ${aimuxStartRSS}→${aimuxRSS}MB (+${aimuxRSS - aimuxStartRSS})`)
    console.log(`    AISDK:  ${aisdkMs.toFixed(0)}ms (${(total / aisdkMs * 1000).toFixed(0)} rps)  RSS ${aisdkStartRSS}→${aisdkRSS}MB (+${aisdkRSS - aisdkStartRSS})`)
    console.log(`    差值:   ${aimuxMs < aisdkMs ? 'aimux 快' : 'AISDK 快'} ${(aisdkMs / aimuxMs).toFixed(1)}x  |  内存差 ${aimuxRSS - aisdkRSS}MB`)
    console.log()
  }

  await server.close()
}

main().catch(console.error)
