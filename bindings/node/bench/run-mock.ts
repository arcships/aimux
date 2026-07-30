import { startMockServer } from './mock-server.ts'

async function main() {
  const server = await startMockServer()
  console.log('MOCK_URI=' + server.uri)
  // keep running
  setInterval(() => {}, 1000)
}

main().catch(console.error)
