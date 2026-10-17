import { createOpenAICompatible } from '@ai-sdk/openai-compatible';
import { API_KEY, JSON_HEADERS, SSE_HEADERS, sse } from '../common.mjs';

const BASE = 'https://api.groq.com/openai/v1';

const completion = {
  id: 'chatcmpl-fixture-1',
  object: 'chat.completion',
  created: 1735689600,
  model: 'llama-3.3-70b-versatile',
  choices: [{ index: 0, message: { role: 'assistant', content: 'Hello! How can I help you today?' }, finish_reason: 'stop' }],
  usage: { prompt_tokens: 12, completion_tokens: 9, total_tokens: 21 },
};

const ok = (url = `${BASE}/chat/completions`, headers = JSON_HEADERS) => ({ url, status: 200, headers, body: completion });

const groq = (mock, extra = {}) => createOpenAICompatible({ name: 'groq', baseURL: BASE, apiKey: API_KEY, fetch: mock, ...extra });
const MODEL = 'llama-3.3-70b-versatile';

const chunk = (delta, finish_reason = null) => ({
  id: 'chatcmpl-fixture-2',
  object: 'chat.completion.chunk',
  created: 1735689600,
  model: MODEL,
  choices: [{ index: 0, delta, finish_reason }],
});

export default [
  {
    name: 'chat-basic',
    build: (mock) => groq(mock).chatModel(MODEL),
    call: (model, run) => run('generateText', { prompt: 'Say hello.' }),
    response: ok(),
  },
  {
    name: 'chat-stream-basic',
    build: (mock) => groq(mock).chatModel(MODEL),
    call: (model, run) => run('streamText', { prompt: 'Say hello.' }),
    response: {
      url: `${BASE}/chat/completions`,
      status: 200,
      headers: SSE_HEADERS,
      body: sse([
        { data: chunk({ role: 'assistant', content: '' }) },
        { data: chunk({ content: 'Hello' }) },
        { data: chunk({ content: ' world' }) },
        { data: chunk({}, 'stop') },
        { data: { id: 'chatcmpl-fixture-2', object: 'chat.completion.chunk', created: 1735689600, model: MODEL, choices: [], usage: { prompt_tokens: 12, completion_tokens: 2, total_tokens: 14 } } },
        'data: [DONE]',
      ]),
    },
  },
  {
    name: 'embedding-basic',
    build: (mock) => groq(mock).embeddingModel('text-embedding-fixture'),
    call: (model, run) => run('embedMany', { values: ['sunny day at the beach', 'rainy day in the city'] }),
    response: {
      url: `${BASE}/embeddings`,
      status: 200,
      headers: JSON_HEADERS,
      body: {
        object: 'list',
        data: [
          { object: 'embedding', index: 0, embedding: [0.125, -0.25, 0.5] },
          { object: 'embedding', index: 1, embedding: [0.0625, 0.375, -0.5] },
        ],
        model: 'text-embedding-fixture',
        usage: { prompt_tokens: 10, total_tokens: 10 },
      },
    },
  },
  {
    name: 'chat-provider-options-namespace',
    build: (mock) => groq(mock).chatModel(MODEL),
    call: (model, run) =>
      run('generateText', {
        prompt: 'Say hello.',
        providerOptions: { groq: { foo: 'bar' }, openaiCompatible: { user: 'u1' } },
      }),
    response: ok(),
    observe: ({ model, request }) => {
      if (model.provider !== 'groq.chat') throw new Error(`expected provider "groq.chat", got "${model.provider}"`);
      return { bodyHasFooAtTopLevel: request.body.foo === 'bar', bodyUser: request.body.user, providerIsGroqChat: true };
    },
  },
  {
    // Extra: unknown fields under the generic `openaiCompatible` key (as opposed to the provider's
    // own name) are NOT forwarded; only the schema-known ones (user, reasoningEffort, ...) are used.
    name: 'chat-provider-options-generic-key-unknown',
    build: (mock) => groq(mock).chatModel(MODEL),
    call: (model, run) =>
      run('generateText', { prompt: 'Say hello.', providerOptions: { openaiCompatible: { user: 'u1', extra_field: 'x' } } }),
    response: ok(),
    observe: ({ request }) => ({ bodyHasExtraField: 'extra_field' in request.body }),
  },
  {
    name: 'chat-no-api-key',
    env: {},
    build: (mock) => createOpenAICompatible({ name: 'local', baseURL: 'http://localhost:1234/v1', fetch: mock }).chatModel('local-model'),
    call: (model, run) => run('generateText', { prompt: 'Say hello.' }),
    response: ok('http://localhost:1234/v1/chat/completions'),
    observe: ({ request }) => {
      const has = Object.keys(request.headers).some((k) => k === 'authorization');
      if (has) throw new Error('unexpected authorization header');
      return { hasAuthorizationHeader: has };
    },
  },
  {
    name: 'chat-query-params-and-headers',
    build: (mock) =>
      groq(mock, { queryParams: { 'api-version': '1' }, headers: { 'x-a': '1', 'X-B': '2' } }).chatModel(MODEL),
    call: (model, run) => run('generateText', { prompt: 'Say hello.', headers: { 'x-b': '3', 'x-c': '4' } }),
    response: ok(`${BASE}/chat/completions?api-version=1`),
    observe: ({ request }) => ({ headerNamesAsSent: request.rawHeaderNames.filter((n) => /^x-/i.test(n)).sort() }),
  },
];
