/**
 * aimux vs AISDK — 大 payload + 高并发
 *
 * 真实 LLM 场景：
 *   - 上文 200K-500K token（~1-2MB JSON）
 *   - 输出 10K-100K token
 *   - 高并发（多请求同时跑）
 *
 * 测试：
 *   1. 大 payload（500K 上文）— 测序列化/解析优势
 *   2. 并发吞吐（N=10/50/100）— 测连接池/TLS 复用优势
 *   3. CPU 限制下跑（taskset 1 核）— 测 Rust 无 GC 的优势
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

// ── 生成大 payload ────────────────────────────────────────────────────────
function makeContext(approxBytes: number): string {
  // 模拟真实对话上文：多轮 messages
  const turn = 'Explain Rust ownership in detail. ' + 'word '.repeat(50)
  const turns: string[] = []
  let total = 0
  let i = 0
  while (total < approxBytes) {
    turns.push(`Message ${i}: ${turn}`)
    total += turn.length + 20
    i++
  }
  return turns.join('\n')
}

const CTX_500K = makeContext(500_000)   // ~500KB 上文 ≈ 125K token
const CTX_1M = makeContext(1_000_000)   // ~1MB 上文 ≈ 250K token

// ── mock server 返回大响应 ────────────────────────────────────────────────
import { createServer as createHttpServer, type IncomingMessage, type ServerResponse } from 'node:http'

function startBigMockServer(responseSize: number) {
  return new Promise<{ uri: string; close: () => Promise<void> }>((resolve) => {
    // 生成大响应（模拟 10K-100K token 的输出）
    const chunk = 'x'.repeat(1000)
    const body = JSON.stringify({
      id: 'chatcmpl-mock',
      object: 'chat.completion',
      created: 1700000000,
      model: 'gpt-4o',
      choices: [{
        index: 0,
        message: { role: 'assistant', content: chunk.repeat(responseSize / 1000) },
        finish_reason: 'stop',
      }],
      usage: { prompt_tokens: 5000, completion_tokens: responseSize, total_tokens: responseSize + 5000 },
    })
    const server = createHttpServer((req: IncomingMessage, res: ServerResponse) => {
      req.on('data', () => {})
      req.on('end', () => {
        res.writeHead(200, { 'Content-Type': 'application/json' })
        res.end(body)
      })
    })
    server.listen(0, '127.0.0.1', () => {
      const addr = server.address()
      const port = typeof addr === 'object' && addr ? addr.port : 0
      resolve({
        uri: `http://127.0.0.1:${port}`,
        close: () => new Promise<void>((r) => server.close(() => r())),
      })
    })
  })
}

// ── bench 函数 ────────────────────────────────────────────────────────────
async function benchAimux(uri: string, prompt: string): Promise<number> {
  const model = await napi.openai('test-key', 'gpt-4o', `${uri}/v1`)
  const promptJson = JSON.stringify(prompt)
  const s = process.hrtime.bigint()
  await model.generateText(promptJson)
  return Number(process.hrtime.bigint() - s) / 1e6
}

let aisdkModel: { doGenerate: (opts: unknown) => Promise<unknown> } | null = null
async function initAisdk(uri: string) {
    const { createOpenAI } = await import('@ai-sdk/openai')
  const openai = createOpenAI({ apiKey: 'test-key', baseURL: `${uri}/v1` })
  aisdkModel = openai.chat('gpt-4o') as never
}
async function benchAisdk(prompt: string): Promise<number> {
  if (!aisdkModel) throw new Error('not init')
  const opts = { prompt: [{ role: 'user', content: [{ type: 'text', text: prompt }] }], mode: { type: 'regular' } }
  const s = process.hrtime.bigint()
  await aisdkModel.doGenerate(opts)
  return Number(process.hrtime.bigint() - s) / 1e6
}

function pct(a: number[], p: number) { return [...a].sort((x, y) => x - y)[Math.ceil(p / 100 * a.length) - 1] }
function mean(a: number[]) { return a.reduce((s, v) => s + v, 0) / a.length }

// ── 主流程 ────────────────────────────────────────────────────────────────
async function main() {
  // ===== 测试 1: 大 payload 单请求 =====
  console.log('\n═══ 测试 1: 大 payload 单请求 (N=50) ═══\n')

  for (const [label, ctx] of [['500KB (~125K token)', CTX_500K], ['1MB (~250K token)', CTX_1M]] as const) {
    const server = await startBigMockServer(50_000) // 50KB 响应
    const uri = server.uri
    await initAisdk(uri)
    const model = await napi.openai('test-key', 'gpt-4o', `${uri}/v1`)

    // warmup
    for (let i = 0; i < 5; i++) { await model.generateText(JSON.stringify(ctx)) }
    const aisdkOpts = { prompt: [{ role: 'user', content: [{ type: 'text', text: ctx }] }], mode: { type: 'regular' } }
    for (let i = 0; i < 5; i++) { await aisdkModel!.doGenerate(aisdkOpts) }

    const N = 50
    const aimuxT: number[] = []
    const aisdkT: number[] = []
    for (let i = 0; i < N; i++) {
      const s = process.hrtime.bigint()
      await model.generateText(JSON.stringify(ctx))
      aimuxT.push(Number(process.hrtime.bigint() - s) / 1e6)
    }
    for (let i = 0; i < N; i++) {
      const s = process.hrtime.bigint()
      await aisdkModel!.doGenerate(aisdkOpts)
      aisdkT.push(Number(process.hrtime.bigint() - s) / 1e6)
    }

    const am = mean(aimuxT), sm = mean(aisdkT)
    console.log(`  ${label}:`)
    console.log(`    aimux:  ${am.toFixed(1)}ms  P50=${pct(aimuxT, 50).toFixed(1)}  P95=${pct(aimuxT, 95).toFixed(1)}`)
    console.log(`    AISDK:  ${sm.toFixed(1)}ms  P50=${pct(aisdkT, 50).toFixed(1)}  P95=${pct(aisdkT, 95).toFixed(1)}`)
    console.log(`    差值:   ${am < sm ? 'aimux 快' : 'AISDK 快'} ${(sm / am).toFixed(1)}x  (省 ${(sm - am).toFixed(1)}ms)`)
    console.log()
    await server.close()
  }

  // ===== 测试 2: 并发吞吐 =====
  console.log('═══ 测试 2: 并发吞吐 ═══\n')

  const server = await startBigMockServer(10_000) // 10KB 响应
  const uri = server.uri
  await initAisdk(uri)
  const model = await napi.openai('test-key', 'gpt-4o', `${uri}/v1`)
  const prompt = makeContext(50_000) // 50KB 上文
  const aisdkOpts = { prompt: [{ role: 'user', content: [{ type: 'text', text: prompt }] }], mode: { type: 'regular' } }

  for (const concurrency of [1, 10, 50, 100]) {
    // warmup
    await Promise.all(Array.from({ length: Math.min(concurrency, 5) }, () => model.generateText(JSON.stringify(prompt))))

    const total = 100 // 总请求数
    const batches = Math.ceil(total / concurrency)

    // aimux
    const aimuxStart = process.hrtime.bigint()
    for (let b = 0; b < batches; b++) {
      const batch = Math.min(concurrency, total - b * concurrency)
      await Promise.all(Array.from({ length: batch }, () => model.generateText(JSON.stringify(prompt))))
    }
    const aimuxMs = Number(process.hrtime.bigint() - aimuxStart) / 1e6

    // AISDK
    await Promise.all(Array.from({ length: Math.min(concurrency, 5) }, () => aisdkModel!.doGenerate(aisdkOpts)))
    const aisdkStart = process.hrtime.bigint()
    for (let b = 0; b < batches; b++) {
      const batch = Math.min(concurrency, total - b * concurrency)
      await Promise.all(Array.from({ length: batch }, () => aisdkModel!.doGenerate(aisdkOpts)))
    }
    const aisdkMs = Number(process.hrtime.bigint() - aisdkStart) / 1e6

    const aimuxRps = (total / aimuxMs * 1000).toFixed(0)
    const aisdkRps = (total / aisdkMs * 1000).toFixed(0)
    console.log(`  并发=${String(concurrency).padStart(3)}  aimux ${aimuxMs.toFixed(0)}ms (${aimuxRps} rps)  |  AISDK ${aisdkMs.toFixed(0)}ms (${aisdkRps} rps)  |  ${(aisdkMs / aimuxMs).toFixed(1)}x`)
  }
  console.log()
  await server.close()
}

main().catch(console.error)
