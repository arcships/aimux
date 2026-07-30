import { startMockServer } from './mock-server.ts'
import { createServer, type IncomingMessage, type ServerResponse } from 'node:http'

// 和 Node bench-constrained 一样的大响应 mock (50KB)
const chunk = 'x'.repeat(1000)
const body = JSON.stringify({
  id: 'chatcmpl-mock', object: 'chat.completion', created: 1700000000, model: 'gpt-4o',
  choices: [{ index: 0, message: { role: 'assistant', content: chunk.repeat(50) }, finish_reason: 'stop' }],
  usage: { prompt_tokens: 5000, completion_tokens: 50000, total_tokens: 55000 },
})

async function main() {
  const server = createServer((req: IncomingMessage, res: ServerResponse) => {
    req.on('data', () => {})
    req.on('end', () => {
      res.writeHead(200, { 'Content-Type': 'application/json' })
      res.end(body)
    })
  })
  server.listen(0, '127.0.0.1', () => {
    const port = (server.address() as { port: number }).port
    console.log('MOCK_URI=http://127.0.0.1:' + port)
  })
  setInterval(() => {}, 1000)
}

main().catch(console.error)
