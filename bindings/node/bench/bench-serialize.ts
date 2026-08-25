/**
 * 序列化瓶颈分析 — 拆解 FFI 边界各环节耗时
 */

import { createRequire } from 'node:module'
import { nativeBinaryPath } from './native.ts'
const require = createRequire(import.meta.url)
const napi = require(nativeBinaryPath()) as {
  openai: (apiKey: string, modelId: string, baseUrl?: string) => Promise<{
    generateText: (prompt: string, opts?: string) => Promise<string>
  }>
}

import { createServer, type ServerResponse } from 'node:http'

async function main() {
  // 生成不同大小的 payload
  const sizes = [
    ['1KB', 1_000],
    ['10KB', 10_000],
    ['100KB', 100_000],
    ['500KB', 500_000],
    ['1MB', 1_000_000],
  ] as const

  // mock server
  const server = createServer((req, res: ServerResponse) => {
    req.on('data', () => {})
    req.on('end', () => {
      res.writeHead(200, { 'Content-Type': 'application/json' })
      res.end(JSON.stringify({
        id: 'mock', object: 'chat.completion', model: 'gpt-4o',
        choices: [{ index: 0, message: { role: 'assistant', content: 'ok' }, finish_reason: 'stop' }],
        usage: { prompt_tokens: 5, completion_tokens: 1, total_tokens: 6 },
      }))
    })
  })

  const { uri, close } = await new Promise<{ uri: string; close: () => Promise<void> }>(resolve => {
    server.listen(0, '127.0.0.1', () => {
      const port = (server.address() as { port: number }).port
      resolve({ uri: `http://127.0.0.1:${port}`, close: () => new Promise(r => server.close(() => r())) })
    })
  })

  const model = await napi.openai('test-key', 'gpt-4o', `${uri}/v1`)

  console.log('\n  payload  | JS string  | JS parse   | napi total | FFI 边界   | Rust+HTTP')
  console.log('  ---------|------------|------------|------------|------------|----------')

  for (const [label, size] of sizes) {
    // 生成 payload
    const chunk = 'x'.repeat(100)
    const prompt = Array.from({ length: Math.ceil(size / 110) }, (_, i) => `Message ${i}: ${chunk}`).join('\n')

    // warmup
    for (let i = 0; i < 5; i++) {
      await model.generateText(JSON.stringify(prompt))
    }

    const N = 30

    // 1. 纯 JS JSON.stringify
    const sT: number[] = []
    for (let i = 0; i < N; i++) {
      const s = process.hrtime.bigint()
      JSON.stringify(prompt)
      sT.push(Number(process.hrtime.bigint() - s) / 1e6)
    }

    // 2. 纯 JS JSON.parse（stringify 的结果）
    const json = JSON.stringify(prompt)
    const pT: number[] = []
    for (let i = 0; i < N; i++) {
      const s = process.hrtime.bigint()
      JSON.parse(json)
      pT.push(Number(process.hrtime.bigint() - s) / 1e6)
    }

    // 3. napi 完整调用（stringify → Rust serde → HTTP → Rust serde → parse）
    const nT: number[] = []
    for (let i = 0; i < N; i++) {
      const s = process.hrtime.bigint()
      await model.generateText(JSON.stringify(prompt))
      nT.push(Number(process.hrtime.bigint() - s) / 1e6)
    }

    const mean = (a: number[]) => a.reduce((s, v) => s + v, 0) / a.length
    const sMean = mean(sT)
    const pMean = mean(pT)
    const nMean = mean(nT)
    const ffi = nMean - sMean - pMean // 减去 JS 侧序列化，剩余是 Rust serde + HTTP

    const fmt = (v: number) => v.toFixed(3).padStart(8)
    console.log(`  ${label.padEnd(8)} | ${fmt(sMean)} | ${fmt(pMean)} | ${fmt(nMean)} | ${fmt(sMean + pMean)} | ${fmt(ffi)}`)
  }
  console.log()
  console.log('  FFI 边界 = JS stringify + JS parse（JSON string 桥的开销）')
  console.log('  Rust+HTTP = napi total - FFI 边界（Rust serde + reqwest HTTP）')
  console.log()

  await close()
}

main().catch(console.error)
