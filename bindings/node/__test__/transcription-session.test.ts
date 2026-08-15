// transcription-session.test.ts — RFC-0028 session behavior through the Node binding.
//
// Node twin of bindings/python/tests/test_transcription_session.py (issue #118
// requires one binding-behavior group per language). Same three cases against
// a fake realtime WebSocket server:
//
// - `nextPart(timeoutMs)` rejects with a `TimeoutError` whose retry verdict is
//   *determinable* (`retryable` is false), and the session stays live.
// - A server-sent realtime `error` event propagates verbatim as an `Err` part
//   (the provider's own message, decidable retry verdict), then the channel
//   ends normally.
// - A connect failure rejects through the session channel on the first
//   `nextPart` (the session API never silently hangs), and the following
//   `nextPart` resolves `null` (channel closed).
//
// No `ws` dependency exists in this package, so the fake server hand-rolls the
// HTTP Upgrade handshake (SHA-1 accept via node:crypto) and the server→client
// text-frame codec (<126-byte payloads), mirroring the python side's raw-socket
// fake. Client frames are masked and simply drained.

import test from 'ava'
import { createHash } from 'node:crypto'
import { createServer, type Server } from 'node:http'
import type { Duplex } from 'node:stream'

import { openaiTranscription, startTranscriptionSession } from '../index.js'
import { AimuxError, APICallError, TimeoutError } from '../src/index.ts'

const WS_GUID = '258EAFA5-E914-47DA-95CA-C5AB0DC85B11'

// ── Minimal server-side WebSocket codec (frames < 126 bytes) ────────────────

function textFrame(payload: string): Buffer {
  const data = Buffer.from(payload, 'utf8')
  if (data.length >= 126) throw new Error('extend the codec for long payloads')
  return Buffer.concat([Buffer.from([0x81, data.length]), data])
}

interface ScriptStep {
  /** Milliseconds after the handshake completes. */
  delayMs: number
  /** Text-frame payload (a realtime event JSON). */
  json: string
}

interface FakeRealtimeServer {
  server: Server
  url: string
}

/**
 * Fake realtime WS server playing `script` (relative to handshake completion).
 * After the script it holds the socket open — the client drives the teardown,
 * so a silent gap is never an accidental disconnect.
 */
function startFakeRealtimeServer(script: ScriptStep[]): Promise<FakeRealtimeServer> {
  return new Promise((resolve) => {
    const sockets = new Set<Duplex>()
    const server = createServer()
    server.on('upgrade', (req, socket) => {
      const key = req.headers['sec-websocket-key']
      if (typeof key !== 'string') {
        socket.destroy()
        return
      }
      const accept = createHash('sha1').update(key + WS_GUID).digest('base64')
      socket.write(
        'HTTP/1.1 101 Switching Protocols\r\n' +
          'Upgrade: websocket\r\n' +
          'Connection: Upgrade\r\n' +
          `Sec-WebSocket-Accept: ${accept}\r\n` +
          '\r\n',
      )
      sockets.add(socket)
      // Drain client frames (masked; content irrelevant to the script).
      socket.on('data', () => {})
      socket.on('close', () => sockets.delete(socket))
      socket.on('error', () => sockets.delete(socket))
      for (const step of script) {
        setTimeout(() => {
          if (sockets.has(socket)) socket.write(textFrame(step.json))
        }, step.delayMs)
      }
    })
    server.listen(0, '127.0.0.1', () => {
      const addr = server.address() as { port: number }
      resolve({ server, url: `http://127.0.0.1:${addr.port}` })
    })
    ;(server as unknown as { __sockets: Set<Duplex> }).__sockets = sockets
  })
}

function stopFakeRealtimeServer(server: Server): Promise<void> {
  const sockets = (server as unknown as { __sockets: Set<Duplex> }).__sockets
  for (const socket of sockets) socket.destroy()
  return new Promise((resolve) => server.close(() => resolve()))
}

// ── Realtime event fixtures (same shapes as the python twin) ────────────────

const SESSION_CREATED = '{"type":"session.created"}'
const ERROR_EVENT = JSON.stringify({
  type: 'error',
  error: { message: 'insufficient quota for realtime transcription' },
})
const DELTA_EVENT = JSON.stringify({
  type: 'conversation.item.input_audio_transcription.delta',
  item_id: 'i',
  delta: 'he',
})

// Serial: the timing-sensitive scripts must not race each other.

test.serial('nextPart timeout is determinable for retries and the session survives', async (t) => {
  // session.created at 50ms (no part mapping), delta at 900ms: the 250ms
  // nextPart window must time out in between and the session must still
  // deliver the later delta.
  const { server, url } = await startFakeRealtimeServer([
    { delayMs: 50, json: SESSION_CREATED },
    { delayMs: 900, json: DELTA_EVENT },
  ])
  t.teardown(() => stopFakeRealtimeServer(server))

  const model = await openaiTranscription('k', 'gpt-realtime-whisper', url)
  const session = await startTranscriptionSession(model, null)
  try {
    const first = JSON.parse((await session.nextPart(3000)) as string)
    t.truthy('StreamStart' in first.Ok)

    const thrown = await t.throwsAsync(session.nextPart(250))
    const err = AimuxError.fromNative(thrown as Error)
    // Timeout is a spent budget, not a transport failure — the retry verdict
    // must be decidable (and be "don't retry").
    t.true(err instanceof TimeoutError)
    t.regex(err.message, /no transcription part within timeout/)
    t.is(err.retryable, false)

    // The session stayed live across the timeout.
    const late = JSON.parse((await session.nextPart(5000)) as string)
    t.truthy('TranscriptDelta' in late.Ok)
  } finally {
    session.close()
  }
})

test.serial('server error event propagates verbatim as an Err part', async (t) => {
  // NOTE on contract: in-stream errors (a server `error` event) are delivered
  // as a serialized `{"Err": {...}}` *part* — only connect failures and
  // nextPart timeouts reject (the session channel returns Err items).
  const { server, url } = await startFakeRealtimeServer([
    { delayMs: 50, json: ERROR_EVENT },
  ])
  t.teardown(() => stopFakeRealtimeServer(server))

  const model = await openaiTranscription('k', 'gpt-realtime-whisper', url)
  const session = await startTranscriptionSession(model, null)
  try {
    const first = JSON.parse((await session.nextPart(5000)) as string)
    t.truthy('StreamStart' in first.Ok)

    const errPart = JSON.parse((await session.nextPart(5000)) as string)
    const api = errPart.Err.ApiCall
    // The provider's own message, verbatim, not transport noise.
    t.is(api.message, 'insufficient quota for realtime transcription')
    // The retry verdict is decidable from the part.
    t.is(api.is_retryable, false)
    t.is(api.status_code, null)

    // The error terminated the stream: the channel ends normally.
    t.is(await session.nextPart(3000), null)
  } finally {
    session.close()
  }
})

test.serial('connect failure surfaces through the channel', async (t) => {
  // Port 1 on loopback: connection refused immediately.
  const model = await openaiTranscription('k', 'gpt-realtime-whisper', 'http://127.0.0.1:1')
  const session = await startTranscriptionSession(model, null)
  try {
    const thrown = await t.throwsAsync(session.nextPart(5000))
    const err = AimuxError.fromNative(thrown as Error)
    t.true(err instanceof APICallError)
    t.regex(err.message, /websocket connect failed/)
    // The session terminated with the connect error (channel closed).
    t.is(await session.nextPart(3000), null)
  } finally {
    session.close()
  }
})
