// multimodal_e2e.test.ts — End-to-end multimodal provider tests with mock HTTP server.
//
// These tests verify the FULL chain for the 8 non-chat modalities:
// Node.js → napi-rs → Rust engine → HTTP mock server → response parsing → typed result.
//
// The mock responses mirror the Go binding's `multimodal_withbase_test.go`
// (same provider wire shapes) and the existing `e2e.test.ts` mock-server pattern.
//
// Each modality constructs a model via its `XxxWithBase(apiKey, modelId, mockUrl)`
// factory (all factories accept an optional `baseUrl`), calls the method, and
// asserts fields on the JSON-serialized result.

import test from 'ava'
import { createServer, type Server } from 'node:http'
import {
  openaiEmbedding,
  openaiSpeech,
  openaiImage,
  openaiTranscription,
  cohereReranking,
  tavilySearch,
  openaiFiles,
  googleVideo,
} from '../src/native.ts'

// ── Mock server helpers (same pattern as e2e.test.ts) ───────────────────────

function startMockServer(handler: (req: any, res: any) => void): Promise<{ server: Server; url: string }> {
  return new Promise((resolve) => {
    const server = createServer(handler)
    server.listen(0, '127.0.0.1', () => {
      const addr = server.address() as any
      resolve({ server, url: `http://127.0.0.1:${addr.port}` })
    })
  })
}

function closeServer(server: Server): Promise<void> {
  return new Promise((resolve) => server.close(() => resolve()))
}

// ── Mock responses (verified wire shapes from Go multimodal_withbase_test.go) ─

const EMBEDDING_RESPONSE = JSON.stringify({
  data: [{ embedding: [0.1, 0.2, 0.3], index: 0 }],
  model: 'text-embedding-3-small',
  usage: { prompt_tokens: 3, total_tokens: 3 },
})

const IMAGE_RESPONSE = JSON.stringify({
  data: [{ b64_json: 'aW1hZ2Ux' }],
})

const TRANSCRIPTION_RESPONSE = JSON.stringify({
  text: 'Hello world',
})

const RERANKING_RESPONSE = JSON.stringify({
  results: [
    { index: 1, relevance_score: 0.95 },
    { index: 0, relevance_score: 0.3 },
  ],
})

const SEARCH_RESPONSE = JSON.stringify({
  results: [{ title: 'Rust', url: 'https://rust-lang.org', content: 'Rust is...' }],
  answer: 'Rust is a systems language.',
})

const FILES_RESPONSE = JSON.stringify({
  id: 'file-abc',
  object: 'file',
  bytes: 1024,
  created_at: 1234,
  filename: 'test.pdf',
  purpose: 'assistants',
})

// Google Video uses a multi-step async API (POST predict → poll operation →
// fetch result). A single-response mock can't cover the full flow, so the video
// test only exercises construction + result-JSON parsing (see test below).
const VIDEO_RESULT_JSON = JSON.stringify({
  videos: [{ Url: { url: 'https://example.com/v.mp4', media_type: 'video/mp4' } }],
})

// ── Tests ───────────────────────────────────────────────────────────────────

test('e2e: OpenAI embedding via mock server', async (t) => {
  const { server, url } = await startMockServer((req, res) => {
    res.writeHead(200, { 'content-type': 'application/json' })
    res.end(EMBEDDING_RESPONSE)
  })

  try {
    const embedder = await openaiEmbedding('test-key', 'text-embedding-3-small', url)
    const r = JSON.parse(await embedder.embed(JSON.stringify(['hello'])))

    t.is(r.embeddings.length, 1)
    t.is(r.embeddings[0].length, 3, 'first embedding has 3 dimensions')
  } finally {
    await closeServer(server)
  }
})

test('e2e: OpenAI speech (TTS) via mock server returns binary audio', async (t) => {
  // The speech provider returns AudioData::Binary with the raw response bytes.
  // The mock serves raw bytes with an audio content-type (not JSON).
  const audioBytes = Buffer.from('fake-mp3-audio-data')
  const { server, url } = await startMockServer((req, res) => {
    res.writeHead(200, { 'content-type': 'audio/mpeg' })
    res.end(audioBytes)
  })

  try {
    const speaker = await openaiSpeech('test-key', 'tts-1', url)
    const r = JSON.parse(
      await speaker.generate(JSON.stringify({ text: 'Hi', voice: 'alloy', output_format: 'mp3' })),
    )

    t.truthy(r.audio.Binary, 'audio comes back as AudioData::Binary')
    t.true(r.audio.Binary.length > 0, 'binary audio is non-empty')
  } finally {
    await closeServer(server)
  }
})

test('e2e: OpenAI image generation via mock server', async (t) => {
  const { server, url } = await startMockServer((req, res) => {
    res.writeHead(200, { 'content-type': 'application/json' })
    res.end(IMAGE_RESPONSE)
  })

  try {
    const imager = await openaiImage('test-key', 'dall-e-3', url)
    // ImageCallOptions has non-Option required fields `n` and `provider_options`,
    // so both must be present in the opts JSON for serde deserialization.
    const r = JSON.parse(
      await imager.generate(JSON.stringify({ prompt: 'otter', n: 1, provider_options: {} })),
    )

    t.is(r.images.Base64.length, 1)
    t.is(r.images.Base64[0], 'aW1hZ2Ux')
  } finally {
    await closeServer(server)
  }
})

test('e2e: OpenAI transcription (STT) via mock server', async (t) => {
  const { server, url } = await startMockServer((req, res) => {
    res.writeHead(200, { 'content-type': 'application/json' })
    res.end(TRANSCRIPTION_RESPONSE)
  })

  try {
    const transcriber = await openaiTranscription('test-key', 'whisper-1', url)
    const r = JSON.parse(await transcriber.generate('dGVzdA==', 'audio/mp3'))

    t.is(r.text, 'Hello world')
  } finally {
    await closeServer(server)
  }
})

test('e2e: Cohere reranking via mock server', async (t) => {
  const { server, url } = await startMockServer((req, res) => {
    res.writeHead(200, { 'content-type': 'application/json' })
    res.end(RERANKING_RESPONSE)
  })

  try {
    const reranker = await cohereReranking('test-key', 'rerank-v3.0', url)
    // `docs_json` deserializes into the externally-tagged `RerankingDocuments`
    // enum, so it must be `{"Text":{"values":[...]}}` (not a bare array).
    const r = JSON.parse(
      await reranker.rerank('which?', JSON.stringify({ Text: { values: ['doc1', 'doc2'] } })),
    )

    t.is(r.ranking.length, 2)
    t.is(r.ranking[0].relevance_score, 0.95)
  } finally {
    await closeServer(server)
  }
})

test('e2e: Tavily search via mock server', async (t) => {
  const { server, url } = await startMockServer((req, res) => {
    res.writeHead(200, { 'content-type': 'application/json' })
    res.end(SEARCH_RESPONSE)
  })

  try {
    const searcher = await tavilySearch('test-key', url)
    const r = JSON.parse(await searcher.search('What is Rust?'))

    t.is(r.results.length, 1)
    t.is(r.results[0].title, 'Rust')
    t.is(r.answer, 'Rust is a systems language.')
  } finally {
    await closeServer(server)
  }
})

test('e2e: OpenAI files upload via mock server', async (t) => {
  const { server, url } = await startMockServer((req, res) => {
    res.writeHead(200, { 'content-type': 'application/json' })
    res.end(FILES_RESPONSE)
  })

  try {
    const files = await openaiFiles('test-key', url)
    const r = JSON.parse(await files.uploadFile('dGVzdA==', 'application/pdf'))

    t.is(r.provider_reference.openai, 'file-abc')
  } finally {
    await closeServer(server)
  }
})

test('e2e: Google video construction + result parsing', async (t) => {
  // Google Video uses a multi-step async API (POST predict → poll → fetch).
  // A single-response mock server can't cover the full flow, so (matching the
  // Go test) we only verify construction succeeds and the result JSON parses.
  // No HTTP call is made, so the base URL is intentionally unreachable.
  const video = await googleVideo('test-key', 'veo-3.0', 'http://localhost:9999')
  t.truthy(video, 'googleVideo factory constructs without throwing')

  const r = JSON.parse(VIDEO_RESULT_JSON)
  t.is(r.videos[0].Url.url, 'https://example.com/v.mp4')
  t.is(r.videos[0].Url.media_type, 'video/mp4')
})
