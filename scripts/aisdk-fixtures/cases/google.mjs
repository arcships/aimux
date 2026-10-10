import { jsonSchema, tool } from 'ai';
import { createGoogleGenerativeAI } from '@ai-sdk/google';
import { API_KEY, JSON_HEADERS, SSE_HEADERS, sse } from '../common.mjs';

const BASE = 'https://generativelanguage.googleapis.com/v1beta';
const MODEL = 'gemini-2.5-flash';
const provider = (mock, extra = {}) => createGoogleGenerativeAI({ apiKey: API_KEY, fetch: mock, ...extra });

const candidates = (parts, finishReason = 'STOP') => [{ content: { role: 'model', parts }, finishReason, index: 0 }];
const usageMetadata = { promptTokenCount: 4, candidatesTokenCount: 3, totalTokenCount: 7 };

const ok = (parts = [{ text: 'Hello! How can I help you today?' }]) => ({
  url: `${BASE}/models/${MODEL}:generateContent`,
  status: 200,
  headers: JSON_HEADERS,
  body: { candidates: candidates(parts), usageMetadata, modelVersion: MODEL, responseId: 'resp-fixture-1' },
});

const weatherTool = tool({
  description: 'Get the current weather for a city.',
  inputSchema: jsonSchema({
    type: 'object',
    properties: { city: { type: 'string', description: 'City name' } },
    required: ['city'],
    additionalProperties: false,
  }),
});

export default [
  {
    name: 'generate-basic',
    build: (mock) => provider(mock)(MODEL),
    call: (model, run) => run('generateText', { prompt: 'Say hello.' }),
    response: ok(),
  },
  {
    name: 'generate-stream-basic',
    build: (mock) => provider(mock)(MODEL),
    call: (model, run) => run('streamText', { prompt: 'Say hello.' }),
    response: {
      url: `${BASE}/models/${MODEL}:streamGenerateContent?alt=sse`,
      status: 200,
      headers: SSE_HEADERS,
      body: sse([
        { data: { candidates: candidates([{ text: 'Hello' }], undefined), modelVersion: MODEL, responseId: 'resp-fixture-2' } },
        { data: { candidates: candidates([{ text: ' world' }]), usageMetadata: { promptTokenCount: 4, candidatesTokenCount: 2, totalTokenCount: 6 }, modelVersion: MODEL, responseId: 'resp-fixture-2' } },
      ]),
    },
  },
  {
    name: 'generate-tools',
    build: (mock) => provider(mock)(MODEL),
    call: (model, run) =>
      run('generateText', {
        prompt: 'What is the weather in Paris?',
        tools: { get_weather: weatherTool },
        toolChoice: 'required',
      }),
    response: ok([{ functionCall: { name: 'get_weather', args: { city: 'Paris' } } }]),
  },
  {
    name: 'generate-provider-options',
    build: (mock) => provider(mock)(MODEL),
    call: (model, run) =>
      run('generateText', {
        prompt: 'Say hello.',
        providerOptions: {
          google: {
            thinkingConfig: { thinkingBudget: 1024, includeThoughts: true },
            safetySettings: [{ category: 'HARM_CATEGORY_HATE_SPEECH', threshold: 'BLOCK_ONLY_HIGH' }],
          },
        },
      }),
    response: ok(),
  },
  {
    name: 'embedding-basic',
    build: (mock) => provider(mock).embedding('gemini-embedding-001'),
    call: (model, run) => run('embedMany', { values: ['sunny day at the beach', 'rainy day in the city'] }),
    response: {
      url: `${BASE}/models/gemini-embedding-001:batchEmbedContents`,
      status: 200,
      headers: JSON_HEADERS,
      body: { embeddings: [{ values: [0.125, -0.25, 0.5] }, { values: [0.0625, 0.375, -0.5] }] },
    },
  },
];
