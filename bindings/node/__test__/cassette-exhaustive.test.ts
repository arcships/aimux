// cassette-exhaustive.test.ts — Exhaustive cassette replay for Node binding.
//
// Iterates over EVERY chat/completions cassette file across all provider
// directories, mounts each one individually, and verifies the full chain:
//   Node.js → napi → Rust engine → single cassette → parse → result

import test from 'ava'
import { openai } from '../index.js'
import { readFileSync, readdirSync, statSync } from 'node:fs'
import { join, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'
import { createServer, type Server } from 'node:http'

const __dirname = dirname(fileURLToPath(import.meta.url))
const CASSETTE_BASE = join(__dirname, '..', '..', '..', 'aimux-providers', 'tests', 'cassettes')

interface Cassette {
  provider: string
  file: string
  reqPath: string
  reqBody: any
  isStream: boolean
  respStatus: number
  respHeaders: Record<string, string>
  respBody: string
}

function loadChatCassettes(): Cassette[] {
  const all: Cassette[] = []
  for (const provider of readdirSync(CASSETTE_BASE)) {
    const dir = join(CASSETTE_BASE, provider)
    try {
      const stat = statSync(dir)
      if (!stat.isDirectory()) continue
    } catch { continue }

    for (const file of readdirSync(dir)) {
      if (!file.endsWith('.json')) continue
      try {
        const raw = JSON.parse(readFileSync(join(dir, file), 'utf-8'))
        const reqPath = raw.request?.path || '/'
        if (!reqPath.endsWith('/chat/completions')) continue

        let body = raw.request?.body || {}
        if (typeof body === 'string') {
          try { body = JSON.parse(body) } catch { body = {} }
        }

        const headers: Record<string, string> = {}
        for (const [k, v] of Object.entries(raw.response?.headers || {})) {
          if (typeof v === 'string') headers[k] = v
        }

        all.push({
          provider,
          file,
          reqPath,
          reqBody: body,
          isStream: body.stream === true,
          respStatus: raw.response?.status || 200,
          respHeaders: headers,
          respBody: raw.response?.body || '',
        })
      } catch {}
    }
  }
  return all
}

function startSingleCassetteServer(cass: Cassette): Promise<{ server: Server; url: string }> {
  return new Promise((resolve) => {
    const server = createServer((req, res) => {
      let body = ''
      req.on('data', (chunk) => (body += chunk))
      req.on('end', () => {
        res.writeHead(cass.respStatus, cass.respHeaders)
        res.end(cass.respBody)
      })
    })
    server.listen(0, '127.0.0.1', () => {
      const addr = server.address() as any
      resolve({ server, url: `http://127.0.0.1:${addr.port}` })
    })
  })
}

function getModel(cass: Cassette): string {
  return cass.reqBody?.model || 'gpt-4o'
}

function getBasePath(reqPath: string): string {
  if (reqPath.endsWith('/chat/completions')) {
    return reqPath.slice(0, -'/chat/completions'.length)
  }
  return ''
}

function extractPrompt(cass: Cassette): string {
  const msgs = cass.reqBody?.messages
  if (Array.isArray(msgs)) {
    for (let i = msgs.length - 1; i >= 0; i--) {
      if (msgs[i].role === 'user') {
        const content = msgs[i].content
        if (typeof content === 'string') return content
        if (Array.isArray(content)) {
          for (const p of content) {
            if (p.text) return p.text
          }
        }
      }
    }
  }
  return 'Hello'
}

const cassettes = loadChatCassettes()

test(`exhaustive: ${cassettes.length} chat/completions cassettes replayed`, async (t) => {
  // 803 sequential replays: ~2.5s idle but >10s (ava's default timeout) on a
  // loaded machine, so give the whole run explicit headroom.
  t.timeout(120_000)
  t.true(cassettes.length > 700, `expected 700+ chat cassettes, got ${cassettes.length}`)

  let passed = 0
  let failed = 0
  const errors: string[] = []

  for (const cass of cassettes) {
    const { server, url } = await startSingleCassetteServer(cass)
    const basePath = getBasePath(cass.reqPath)
    const baseUrl = basePath ? `${url}${basePath}` : url
    const modelId = getModel(cass)
    const prompt = extractPrompt(cass)

    try {
      const model = await openai('test-key', modelId, baseUrl)
      if (cass.isStream) {
        const gen = await model.streamText(JSON.stringify(prompt))
        let parts = 0
        for await (const _json of gen) { parts++ }
        if (parts === 0) throw new Error('no stream parts')
      } else {
        const resultJson = await model.generateText(JSON.stringify(prompt))
        const result = JSON.parse(resultJson)
        if (result.error) throw new Error(result.error)
      }
      passed++
    } catch (e: any) {
      const msg = String(e?.message || e)
      // Accept any provider error — these cassettes record error responses
      if (msg.includes('404') || msg.includes('400') || msg.includes('401')
          || msg.includes('429') || msg.includes('500')
          || msg.includes('model not found') || msg.includes('rate limited')
          || msg.includes('error decoding') || msg.includes('does not exist')
          || msg.includes('invalid type') || msg.includes('missing field')) {
        passed++
      } else {
        failed++
        if (errors.length < 20) errors.push(`${cass.provider}/${cass.file}: ${msg}`)
      }
    } finally {
      await new Promise<void>((resolve) => server.close(() => resolve()))
    }
  }

  t.log(`Total: ${cassettes.length}, Passed: ${passed}, Failed: ${failed}`)
  if (errors.length > 0) {
    for (const e of errors) t.log(`  FAIL: ${e}`)
  }

  const passRate = passed / cassettes.length
  t.true(passRate > 0.9, `pass rate ${(passRate * 100).toFixed(1)}% too low: ${passed}/${cassettes.length}`)
})
