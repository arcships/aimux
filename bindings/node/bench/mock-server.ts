/**
 * 本地 mock server — 返回固定 JSON / SSE 响应。
 *
 * 抹平网络 RTT 和 LLM 生成时长，剩下的差值就是两个 SDK 自身的
 * 协议转换 / 序列化 / 流解析开销。
 *
 * 用法：先 start()，拿到 uri 后传给 bench 脚本，跑完 stop()。
 */

import { createServer, type IncomingMessage, type ServerResponse } from 'node:http'
import { randomBytes } from 'node:crypto'

const MOCK_RESPONSE = JSON.stringify({
  id: 'chatcmpl-mock',
  object: 'chat.completion',
  created: 1700000000,
  model: 'gpt-4o',
  choices: [
    {
      index: 0,
      message: { role: 'assistant', content: 'Rust is a systems programming language focused on safety, speed, and concurrency.' },
      finish_reason: 'stop',
    },
  ],
  usage: { prompt_tokens: 5, completion_tokens: 15, total_tokens: 20 },
})

/** 固定的 SSE 流（3 个 data chunk + done） */
const SSE_CHUNKS = [
  'data: {"id":"chatcmpl-mock","object":"chat.completion.chunk","created":1700000000,"model":"gpt-4o","choices":[{"index":0,"delta":{"role":"assistant","content":"Rust"},"finish_reason":null}]}\n\n',
  'data: {"id":"chatcmpl-mock","object":"chat.completion.chunk","created":1700000000,"model":"gpt-4o","choices":[{"index":0,"delta":{"content":" is a systems"},"finish_reason":null}]}\n\n',
  'data: {"id":"chatcmpl-mock","object":"chat.completion.chunk","created":1700000000,"model":"gpt-4o","choices":[{"index":0,"delta":{"content":" programming language."},"finish_reason":null}]}\n\n',
  'data: {"id":"chatcmpl-mock","object":"chat.completion.chunk","created":1700000000,"model":"gpt-4o","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":5,"completion_tokens":8,"total_tokens":13}}\n\n',
  'data: [DONE]\n\n',
].join('')

export interface MockServer {
  uri: string
  close(): Promise<void>
}

/** 启动一个本地 mock server，返回其 uri。 */
export async function startMockServer(): Promise<MockServer> {
  return new Promise((resolve) => {
    const server = createServer((req: IncomingMessage, res: ServerResponse) => {
      // 收集 request body（虽然不用，但要消费掉）
      req.on('data', () => {})
      req.on('end', () => {
        if (req.url?.includes('/stream')) {
          res.writeHead(200, {
            'Content-Type': 'text/event-stream',
            'Cache-Control': 'no-cache',
            Connection: 'keep-alive',
          })
          res.end(SSE_CHUNKS)
        } else {
          res.writeHead(200, { 'Content-Type': 'application/json' })
          res.end(MOCK_RESPONSE)
        }
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

/** 生成一个随机 payload（模拟大 prompt）的简单文本。 */
export function makePayload(approxTokens: number): string {
  const words = ['Rust', 'safety', 'speed', 'concurrency', 'ownership', 'borrow', 'lifetime', 'trait', 'async', 'await']
  const lines: string[] = []
  for (let i = 0; i < approxTokens; i++) {
    lines.push(`${words[i % words.length]} ${i}`)
  }
  return lines.join(' ')
}
