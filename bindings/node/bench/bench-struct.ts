/**
 * 维度二·结构化开销：payload 四档曲线
 *
 * 测 SDK 自身的协议转换 / 序列化 / 解析开销。
 * 用差值法：SDK 总延迟 − B0 纯网络延迟 = 结构化开销。
 *
 * 四档 payload：
 *   小  — 1 轮对话，短响应 100 token
 *   中  — 10 轮对话，中等响应 500 token
 *   大  — 长 prompt 4K token，长响应 2K token
 *   工具 — 5 个工具 schema + tool_call 响应
 *
 * 每档每路跑 N 次，对比 aimux 和 AISDK 的斜率。
 */

import { startMockServer } from './mock-server.ts'
import { nativeBinaryPath } from './native.ts'
import { createRequire } from 'node:module'
const require = cR()
function cR() { return createRequire(import.meta.url) }

// raw napi .node — 直接加载原生二进制
const napi = require(nativeBinaryPath()) as {
  openai: (apiKey: string, modelId: string, baseUrl?: string) => Promise<{
    generateText: (prompt: string, opts?: string) => Promise<string>
  }>
}

// ── B0: undici 直调 ──────────────────────────────────────────────────────
async function benchB0(uri: string, body: object): Promise<number> {
  const start = process.hrtime.bigint()
  const resp = await fetch(`${uri}/v1/chat/completions`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', Authorization: 'Bearer test-key' },
    body: JSON.stringify(body),
  })
  await resp.text()
  const end = process.hrtime.bigint()
  return Number(end - start) / 1e6
}

// ── aimux (napi raw API) ───────────────────────────────────────────────────
async function benchAimux(uri: string, prompt: string): Promise<number> {
  const model = await napi.openai('test-key', 'gpt-4o', `${uri}/v1`)
  const promptJson = JSON.stringify(prompt)
  const start = process.hrtime.bigint()
  await model.generateText(promptJson)
  const end = process.hrtime.bigint()
  return Number(end - start) / 1e6
}

// ── AISDK ─────────────────────────────────────────────────────────────────
let aisdkModel: { doGenerate: (opts: unknown) => Promise<unknown> } | null = null

async function initAisdk(uri: string) {
    const { createOpenAI } = await import('@ai-sdk/openai')
  const openai = createOpenAI({ apiKey: 'test-key', baseURL: `${uri}/v1` })
  aisdkModel = openai.chat('gpt-4o') as never
}

async function benchAisdk(prompt: unknown): Promise<number> {
  if (!aisdkModel) throw new Error('aisdk not init')
  const opts = {
    prompt: Array.isArray(prompt) ? prompt : [{ role: 'user', content: [{ type: 'text', text: prompt }] }],
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
  return { mean: sum / sorted.length, p50: percentile(sorted, 50), p95: percentile(sorted, 95) }
}

// ── Payload 四档 ───────────────────────────────────────────────────────────
const SHORT_TEXT = 'Rust is a systems programming language.'
const LONG_TEXT = Array.from({ length: 4000 }, (_, i) => `word${i}`).join(' ')

interface PayloadDef {
  name: string
  // B0 用的 request body
  b0Body: object
  // aimux 用的 prompt (string)
  aimuxPrompt: string
  // AISDK 用的 prompt
  aisdkPrompt: unknown
}

const payloads: PayloadDef[] = [
  {
    name: '小 (1轮/短)',
    b0Body: { model: 'gpt-4o', messages: [{ role: 'user', content: SHORT_TEXT }], max_tokens: 50 },
    aimuxPrompt: SHORT_TEXT,
    aisdkPrompt: SHORT_TEXT,
  },
  {
    name: '中 (10轮)',
    b0Body: {
      model: 'gpt-4o',
      messages: Array.from({ length: 10 }, (_, i) => ({
        role: i % 2 === 0 ? 'user' : 'assistant',
        content: `Message ${i}: ${SHORT_TEXT}`,
      })),
      max_tokens: 50,
    },
    aimuxPrompt: JSON.stringify(Array.from({ length: 10 }, (_, i) => ({
      role: i % 2 === 0 ? 'user' : 'assistant',
      content: `Message ${i}: ${SHORT_TEXT}`,
    }))),
    aisdkPrompt: Array.from({ length: 10 }, (_, i) => ({
      role: i % 2 === 0 ? 'user' : 'assistant',
      content: [{ type: 'text', text: `Message ${i}: ${SHORT_TEXT}` }],
    })),
  },
  {
    name: '大 (4K/长)',
    b0Body: { model: 'gpt-4o', messages: [{ role: 'user', content: LONG_TEXT }], max_tokens: 50 },
    aimuxPrompt: LONG_TEXT,
    aisdkPrompt: LONG_TEXT,
  },
  {
    name: '工具 (5 tools)',
    b0Body: {
      model: 'gpt-4o',
      messages: [{ role: 'user', content: 'What is the weather?' }],
      tools: Array.from({ length: 5 }, (_, i) => ({
        type: 'function',
        function: {
          name: `get_weather_${i}`,
          description: `Get weather for region ${i}`,
          parameters: { type: 'object', properties: { city: { type: 'string', description: 'City name' } }, required: ['city'] },
        },
      })),
      tool_choice: 'auto',
      max_tokens: 50,
    },
    aimuxPrompt: 'What is the weather?',
    aisdkPrompt: 'What is the weather?',
  },
]

// ── 主流程 ─────────────────────────────────────────────────────────────────
async function main() {
  const server = await startMockServer()
  const uri = server.uri
  const N = 100
  const WARMUP = 10

  await initAisdk(uri)

  console.log(`\n╔══════════════════════════════════════════════════════════╗`)
  console.log(`║  结构化开销 — payload 四档 (N=${N}, warmup=${WARMUP})        ║`)
  console.log(`╚══════════════════════════════════════════════════════════╝\n`)

  const fmt = (v: number) => v.toFixed(3).padStart(8)
  console.log('┌──────────────┬──────────┬──────────┬──────────┬──────────┬──────────┬──────────┐')
  console.log('│ payload      │ B0 mean  │ aimux m  │ AISDK m  │ aimux开  │ AISDK开  │ 差值     │')
  console.log('├──────────────┼──────────┼──────────┼──────────┼──────────┼──────────┼──────────┤')

  for (const p of payloads) {
    // 预热
    for (let i = 0; i < WARMUP; i++) {
      await benchB0(uri, p.b0Body)
      await benchAimux(uri, p.aimuxPrompt)
      await benchAisdk(p.aisdkPrompt)
    }

    const b0: number[] = []
    const aimux: number[] = []
    const aisdk: number[] = []

    for (let i = 0; i < N; i++) {
      b0.push(await benchB0(uri, p.b0Body))
      aimux.push(await benchAimux(uri, p.aimuxPrompt))
      aisdk.push(await benchAisdk(p.aisdkPrompt))
    }

    const b0s = stats(b0)
    const aimuxs = stats(aimux)
    const aisdks = stats(aisdk)
    const aimuxOverhead = aimuxs.mean - b0s.mean
    const aisdkOverhead = aisdks.mean - b0s.mean
    const diff = aimuxs.mean - aisdks.mean

    console.log(
      `│ ${p.name.padEnd(12)} │ ${fmt(b0s.mean)} │ ${fmt(aimuxs.mean)} │ ${fmt(aisdks.mean)} │ ${fmt(aimuxOverhead)} │ ${fmt(aisdkOverhead)} │ ${fmt(diff)} │`
    )
  }
  console.log('└──────────────┴──────────┴──────────┴──────────┴──────────┴──────────┴──────────┘')
  console.log('\n  注：aimux 开销 = aimux - B0，AISDK 开销 = AISDK - B0，差值 = aimux - AISDK')
  console.log('  负值 = 该 SDK 比 B0 快（Rust HTTP 优势大于 SDK 开销）\n')

  await server.close()
}

main().catch(console.error)
