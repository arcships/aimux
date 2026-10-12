import { jsonSchema, tool } from 'ai';
import { createAnthropic } from '@ai-sdk/anthropic';
import { API_KEY, JSON_HEADERS, SSE_HEADERS, sse } from '../common.mjs';

const URL_ = 'https://api.anthropic.com/v1/messages';
const MODEL = 'claude-sonnet-4-5';
const provider = (mock, extra = {}) => createAnthropic({ apiKey: API_KEY, fetch: mock, ...extra });

const message = (content, stop_reason = 'end_turn') => ({
  id: 'msg_fixture_1',
  type: 'message',
  role: 'assistant',
  model: MODEL,
  content,
  stop_reason,
  stop_sequence: null,
  usage: { input_tokens: 12, output_tokens: 9 },
});

const ok = (content = [{ type: 'text', text: 'Hello! How can I help you today?' }], stop_reason) => ({
  url: URL_,
  status: 200,
  headers: { ...JSON_HEADERS, 'request-id': 'req_fixture_1' },
  body: message(content, stop_reason),
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
    name: 'messages-basic',
    build: (mock) => provider(mock)(MODEL),
    call: (model, run) => run('generateText', { prompt: 'Say hello.' }),
    response: ok(),
  },
  {
    name: 'messages-system-tools',
    build: (mock) => provider(mock)(MODEL),
    call: (model, run) =>
      run('generateText', {
        system: 'You are a weather assistant.',
        prompt: 'What is the weather in Paris?',
        tools: { get_weather: weatherTool },
        toolChoice: 'required',
        maxOutputTokens: 256,
      }),
    response: ok([{ type: 'tool_use', id: 'toolu_fixture_1', name: 'get_weather', input: { city: 'Paris' } }], 'tool_use'),
  },
  {
    name: 'messages-stream-basic',
    build: (mock) => provider(mock)(MODEL),
    call: (model, run) => run('streamText', { prompt: 'Say hello.' }),
    response: {
      url: URL_,
      status: 200,
      headers: SSE_HEADERS,
      body: sse([
        { event: 'message_start', data: { type: 'message_start', message: { id: 'msg_fixture_2', type: 'message', role: 'assistant', model: MODEL, content: [], stop_reason: null, stop_sequence: null, usage: { input_tokens: 12, output_tokens: 1 } } } },
        { event: 'content_block_start', data: { type: 'content_block_start', index: 0, content_block: { type: 'text', text: '' } } },
        { event: 'ping', data: { type: 'ping' } },
        { event: 'content_block_delta', data: { type: 'content_block_delta', index: 0, delta: { type: 'text_delta', text: 'Hello' } } },
        { event: 'content_block_delta', data: { type: 'content_block_delta', index: 0, delta: { type: 'text_delta', text: ' world' } } },
        { event: 'content_block_stop', data: { type: 'content_block_stop', index: 0 } },
        { event: 'message_delta', data: { type: 'message_delta', delta: { stop_reason: 'end_turn', stop_sequence: null }, usage: { output_tokens: 2 } } },
        { event: 'message_stop', data: { type: 'message_stop' } },
      ]),
    },
  },
  {
    // Documented option: extended thinking via providerOptions.anthropic.thinking.
    name: 'messages-provider-options',
    build: (mock) => provider(mock)(MODEL),
    call: (model, run) =>
      run('generateText', {
        prompt: 'What is 17 * 23?',
        maxOutputTokens: 4096,
        providerOptions: { anthropic: { thinking: { type: 'enabled', budgetTokens: 2000 } } },
      }),
    response: ok([
      { type: 'thinking', thinking: '17 * 23 = 391.', signature: 'sig_fixture' },
      { type: 'text', text: '391' },
    ]),
  },
  {
    // Which key wins when the same option is given under `anthropic` and under the custom provider name?
    name: 'messages-custom-name',
    build: (mock) => provider(mock, { name: 'myproxy' })(MODEL),
    call: (model, run) =>
      run('generateText', {
        prompt: 'Say hello.',
        providerOptions: {
          anthropic: { metadata: { userId: 'from-anthropic-key' } },
          myproxy: { metadata: { userId: 'from-myproxy-key' } },
        },
      }),
    response: ok(),
    observe: ({ model, request, result }) => ({
      bodyMetadataUserId: request.body.metadata?.user_id,
      modelProvider: model.provider,
      providerMetadataKeys: Object.keys(result.providerMetadata ?? {}).sort(),
    }),
  },
];
