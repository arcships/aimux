import { jsonSchema, tool } from 'ai';
import { createOpenAI } from '@ai-sdk/openai';
import { API_KEY, ENV_KEY, JSON_HEADERS, SSE_HEADERS, sse } from '../common.mjs';

const CHAT_URL = 'https://api.openai.com/v1/chat/completions';

const chatCompletion = (message, finish_reason = 'stop', extra = {}) => ({
  id: 'chatcmpl-fixture-1',
  object: 'chat.completion',
  created: 1735689600,
  model: 'gpt-4o-2024-08-06',
  choices: [{ index: 0, message, finish_reason }],
  usage: {
    prompt_tokens: 12,
    completion_tokens: 9,
    total_tokens: 21,
    prompt_tokens_details: { cached_tokens: 0 },
    completion_tokens_details: { reasoning_tokens: 0 },
  },
  system_fingerprint: 'fp_fixture',
  ...extra,
});

const chatOk = (content = 'Hello! How can I help you today?') => ({
  url: CHAT_URL,
  status: 200,
  headers: { ...JSON_HEADERS, 'x-request-id': 'req_fixture_1' },
  body: chatCompletion({ role: 'assistant', content }),
});

const provider = (mock, extra = {}) => createOpenAI({ apiKey: API_KEY, fetch: mock, ...extra });

const weatherTool = tool({
  description: 'Get the current weather for a city.',
  inputSchema: jsonSchema({
    type: 'object',
    properties: { city: { type: 'string', description: 'City name' } },
    required: ['city'],
    additionalProperties: false,
  }),
});

const chunk = (delta, finish_reason = null, extra = {}) => ({
  id: 'chatcmpl-fixture-2',
  object: 'chat.completion.chunk',
  created: 1735689600,
  model: 'gpt-4o-2024-08-06',
  system_fingerprint: 'fp_fixture',
  choices: [{ index: 0, delta, finish_reason }],
  ...extra,
});

export default [
  {
    name: 'chat-basic',
    build: (mock) => provider(mock).chat('gpt-4o'),
    call: (model, run) => run('generateText', { prompt: 'Say hello.' }),
    response: chatOk(),
  },
  {
    name: 'chat-system-and-settings',
    build: (mock) => provider(mock).chat('gpt-4o'),
    call: (model, run) =>
      run('generateText', {
        system: 'You are a terse assistant.',
        messages: [{ role: 'user', content: 'Name a color.' }],
        temperature: 0.3,
        maxOutputTokens: 64,
        topP: 0.9,
        stopSequences: ['END', '###'],
        seed: 42,
      }),
    response: chatOk('Blue.'),
  },
  {
    name: 'chat-tools',
    build: (mock) => provider(mock).chat('gpt-4o'),
    call: (model, run) =>
      run('generateText', {
        prompt: 'What is the weather in Paris?',
        tools: { get_weather: weatherTool },
        toolChoice: 'required',
      }),
    response: {
      url: CHAT_URL,
      status: 200,
      headers: JSON_HEADERS,
      body: chatCompletion(
        {
          role: 'assistant',
          content: null,
          tool_calls: [{ id: 'call_fixture_1', type: 'function', function: { name: 'get_weather', arguments: '{"city":"Paris"}' } }],
        },
        'tool_calls',
      ),
    },
  },
  {
    name: 'chat-provider-options',
    build: (mock) => provider(mock).chat('gpt-4o'),
    call: (model, run) =>
      run('generateText', {
        prompt: 'What is the weather in Paris?',
        tools: { get_weather: weatherTool },
        providerOptions: { openai: { parallelToolCalls: false, user: 'u1' } },
      }),
    response: chatOk(),
  },
  {
    name: 'chat-headers-merge',
    build: (mock) => provider(mock, { headers: { 'x-a': '1', 'X-B': '2' } }).chat('gpt-4o'),
    call: (model, run) => run('generateText', { prompt: 'Say hello.', headers: { 'x-b': '3', 'x-c': '4' } }),
    response: chatOk(),
    observe: ({ request }) => ({
      headerNamesAsSent: request.rawHeaderNames.filter((n) => /^x-/i.test(n)).sort(),
      xB: request.headers['x-b'],
    }),
  },
  {
    name: 'chat-custom-name',
    build: (mock) => provider(mock, { name: 'proxy' }).chat('gpt-4o'),
    call: (model, run) => run('generateText', { prompt: 'Say hello.' }),
    response: chatOk(),
    observe: ({ model }) => {
      if (model.provider !== 'proxy.chat') throw new Error(`expected provider "proxy.chat", got "${model.provider}"`);
      return { providerIsProxyChat: true };
    },
  },
  {
    name: 'chat-stream-basic',
    build: (mock) => provider(mock).chat('gpt-4o'),
    call: (model, run) => run('streamText', { prompt: 'Say hello.' }),
    response: {
      url: CHAT_URL,
      status: 200,
      headers: SSE_HEADERS,
      body: sse([
        { data: chunk({ role: 'assistant', content: '' }) },
        { data: chunk({ content: 'Hello' }) },
        { data: chunk({ content: ' world' }) },
        { data: chunk({}, 'stop') },
        {
          data: {
            id: 'chatcmpl-fixture-2',
            object: 'chat.completion.chunk',
            created: 1735689600,
            model: 'gpt-4o-2024-08-06',
            choices: [],
            usage: { prompt_tokens: 12, completion_tokens: 2, total_tokens: 14 },
          },
        },
        'data: [DONE]',
      ]),
    },
  },
  {
    name: 'responses-basic',
    build: (mock) => provider(mock).responses('gpt-4o'),
    call: (model, run) => run('generateText', { prompt: 'Say hello.' }),
    response: {
      url: 'https://api.openai.com/v1/responses',
      status: 200,
      headers: JSON_HEADERS,
      body: {
        id: 'resp_fixture_1',
        object: 'response',
        created_at: 1735689600,
        status: 'completed',
        error: null,
        incomplete_details: null,
        model: 'gpt-4o-2024-08-06',
        output: [
          {
            type: 'message',
            id: 'msg_fixture_1',
            status: 'completed',
            role: 'assistant',
            content: [{ type: 'output_text', text: 'Hello! How can I help you today?', annotations: [] }],
          },
        ],
        usage: {
          input_tokens: 12,
          output_tokens: 9,
          total_tokens: 21,
          input_tokens_details: { cached_tokens: 0 },
          output_tokens_details: { reasoning_tokens: 0 },
        },
      },
    },
  },
  {
    name: 'embedding-basic',
    build: (mock) => provider(mock).embedding('text-embedding-3-small'),
    call: (model, run) => run('embedMany', { values: ['sunny day at the beach', 'rainy day in the city'] }),
    response: {
      url: 'https://api.openai.com/v1/embeddings',
      status: 200,
      headers: JSON_HEADERS,
      body: {
        object: 'list',
        data: [
          { object: 'embedding', index: 0, embedding: [0.125, -0.25, 0.5] },
          { object: 'embedding', index: 1, embedding: [0.0625, 0.375, -0.5] },
        ],
        model: 'text-embedding-3-small',
        usage: { prompt_tokens: 10, total_tokens: 10 },
      },
    },
  },
  {
    name: 'missing-api-key',
    env: { OPENAI_API_KEY: undefined },
    build: (mock) => createOpenAI({ fetch: mock }).chat('gpt-4o'),
    call: (model, run) => run('generateText', { prompt: 'Say hello.' }),
    expectError: true,
    expectRequests: 0,
    response: chatOk(),
    observe: ({ requests }) => ({ requestsSent: requests.length }),
  },
  {
    name: 'empty-api-key',
    env: { OPENAI_API_KEY: ENV_KEY },
    // Pins the "empty string does not fall back to env" rule: whatever the SDK does is recorded.
    build: (mock) => createOpenAI({ apiKey: '', fetch: mock }).chat('gpt-4o'),
    call: (model, run) => run('generateText', { prompt: 'Say hello.' }),
    response: chatOk(),
    observe: ({ request }) => ({
      authorizationIsEmptyBearer: request.rawHeaders.authorization?.trim() === 'Bearer',
      authorizationRawLength: request.rawHeaders.authorization?.length,
      usedEnvKey: (request.rawHeaders.authorization ?? '').includes(ENV_KEY),
    }),
  },
];
