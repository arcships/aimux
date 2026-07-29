// cassette-replay.ts — Lightweight cassette replay server for Node tests.
//
// Reads real cassette JSON files (from aimux-providers/tests/cassettes/),
// serves them via a local HTTP server with the same matching logic as
// the Rust `common::replay` module: group by (method, path), score by
// model + stream flag, fall back to first cassette.

import { createServer, type Server, type IncomingMessage, type ServerResponse } from 'node:http'
import { readFileSync, readdirSync } from 'node:fs'
import { join, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'

const __dirname = dirname(fileURLToPath(import.meta.url))
const CASSETTE_BASE = join(__dirname, '..', '..', '..', 'aimux-providers', 'tests', 'cassettes')

interface Cassette {
  scenario: string
  reqPath: string
  reqMethod: string
  reqBody: any
  respStatus: number
  respHeaders: Record<string, string>
  respBody: string
}

function loadCassettes(provider: string): Cassette[] {
  const dir = join(CASSETTE_BASE, provider)
  const files = readdirSync(dir).filter((f) => f.endsWith('.json'))
  const cassettes: Cassette[] = []
  for (const file of files) {
    try {
      const raw = JSON.parse(readFileSync(join(dir, file), 'utf-8'))
      const body = raw.request?.body
      // body can be a JSON object or a string
      let reqBody = body
      if (typeof body === 'string') {
        try { reqBody = JSON.parse(body) } catch { reqBody = null }
      }
      cassettes.push({
        scenario: raw.scenario || file,
        reqPath: raw.request?.path || '/',
        reqMethod: raw.request?.method || 'POST',
        reqBody: reqBody || {},
        respStatus: raw.response?.status || 200,
        respHeaders: raw.response?.headers || {},
        respBody: raw.response?.body || '',
      })
    } catch (e) {
      // skip malformed cassettes
    }
  }
  return cassettes
}

// Score a cassette against the incoming request body (same logic as Rust replay)
function score(cassetteBody: any, reqBody: any): number {
  if (typeof cassetteBody !== 'object' || cassetteBody === null) return 0
  if (typeof reqBody !== 'object' || reqBody === null) return 0
  let s = 0
  for (const [key, val] of Object.entries(cassetteBody)) {
    if (typeof val !== 'string' && typeof val !== 'boolean' && typeof val !== 'number') continue
    if (reqBody[key] === val) {
      if (key === 'model') s += 100
      else if (key === 'stream') s += 10
      else s += 1
    }
  }
  return s
}

function findBestMatch(cassettes: Cassette[], method: string, path: string, reqBody: any): Cassette | null {
  // Group by (method, path)
  const group = cassettes.filter((c) => c.reqMethod === method && c.reqPath === path)
  if (group.length === 0) return null

  // Score and pick best
  let best = group[0]
  let bestScore = score(best.reqBody, reqBody)
  for (let i = 1; i < group.length; i++) {
    const s = score(group[i].reqBody, reqBody)
    if (s > bestScore) {
      bestScore = s
      best = group[i]
    }
  }
  return best
}

export class CassetteServer {
  private server: Server
  private cassettes: Cassette[]
  public url: string

  constructor(provider: string) {
    this.cassettes = loadCassettes(provider)
    if (this.cassettes.length === 0) {
      throw new Error(`No cassettes found for provider: ${provider}`)
    }

    this.server = createServer((req, res) => this.handle(req, res))
    this.url = '' // set in start()
  }

  private handle(req: IncomingMessage, res: ServerResponse) {
    let body = ''
    req.on('data', (chunk) => (body += chunk))
    req.on('end', () => {
      let reqBody: any = null
      try { reqBody = JSON.parse(body) } catch {}

      const match = findBestMatch(this.cassettes, req.method || 'POST', req.url || '/', reqBody)
      if (!match) {
        res.writeHead(404, { 'content-type': 'application/json' })
        res.end(JSON.stringify({ error: `no cassette for ${req.method} ${req.url}` }))
        return
      }

      res.writeHead(match.respStatus, match.respHeaders)
      res.end(match.respBody)
    })
  }

  async start(): Promise<void> {
    return new Promise((resolve) => {
      this.server.listen(0, '127.0.0.1', () => {
        const addr = this.server.address() as any
        this.url = `http://127.0.0.1:${addr.port}`
        resolve()
      })
    })
  }

  async stop(): Promise<void> {
    return new Promise((resolve) => this.server.close(() => resolve()))
  }

  get count(): number {
    return this.cassettes.length
  }
}
