// Structured e2e tests for the aimux Flutter/Dart binding.
//
// These tests drive the *real* Rust core (via dart:ffi) against a mock
// OpenAI-compatible HTTP server implemented with `dart:io`'s `HttpServer`.
// No real API calls are made.
//
// Run:
//   export PATH="/home/eric8810/.local/dart-sdk/bin:$PATH"
//   export LD_LIBRARY_PATH="/media/eric8810/fast-deliver/code/aimux/target/debug:$LD_LIBRARY_PATH"
//   cd bindings/flutter && dart test test/structured_e2e_test.dart
//
// ## Why the FFI call runs in a worker isolate
//
// `Model.generateText` / `Model.streamText` are *synchronous* FFI calls: they
// block the calling isolate's thread (and thus its event loop) until the Rust
// core finishes. The Rust core issues an HTTP request via reqwest/tokio and
// blocks until it receives a response. If the mock `HttpServer` lived in the
// *same* isolate, the server could never accept/respond to that request while
// the isolate is blocked inside FFI → deadlock.
//
// The mock server therefore runs in the main test isolate (its event loop is
// free), and the blocking FFI call runs in a worker isolate via `Isolate.run`.
// reqwest drives I/O on its own tokio worker threads; the main isolate serves
// the HTTP request and records the request body.

import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:isolate';

import 'package:aimux/aimux.dart';
import 'package:test/test.dart';

// ─────────────────────────────────────────────────────────────────────────────
// Mock server
// ─────────────────────────────────────────────────────────────────────────────

/// A minimal OpenAI-compatible mock server.
///
/// Captures every request (method, path, parsed JSON body) and replies with a
/// caller-configured response for each request, in FIFO order.
class MockOpenAIServer {
  final HttpServer _server;
  final List<Map<String, dynamic>> _responses;
  final List<_RecordedRequest> recorded = [];
  int _nextResponse = 0;

  MockOpenAIServer._(this._server, this._responses) {
    _server.listen(_handleRequest);
  }

  String get baseUrl => 'http://${_server.address.host}:${_server.port}';

  Future<void> close() => _server.close(force: true);

  void _handleRequest(HttpRequest request) async {
    final rawBody = await utf8.decoder.bind(request).join();
    dynamic parsed;
    try {
      parsed = rawBody.isEmpty ? null : jsonDecode(rawBody);
    } catch (_) {
      parsed = rawBody;
    }
    recorded.add(_RecordedRequest(
      method: request.method,
      path: request.uri.path,
      body: parsed,
    ));

    final resp = _nextResponse < _responses.length
        ? _responses[_nextResponse++]
        : _responses.last;
    final response = request.response;
    response.statusCode = HttpStatus.ok;
    if (resp['sse'] == true) {
      response.headers.contentType =
          ContentType.parse('text/event-stream');
      response.headers.set('Cache-Control', 'no-cache');
      response.headers.set('Connection', 'keep-alive');
      response.write(resp['body'] as String);
    } else {
      response.headers.contentType = ContentType.json;
      response.write(resp['body'] as String);
    }
    await response.close();
  }
}

class _RecordedRequest {
  final String method;
  final String path;
  final dynamic body;
  _RecordedRequest({required this.method, required this.path, required this.body});
}

Future<MockOpenAIServer> startMockServer(List<Map<String, dynamic>> responses) async {
  final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
  return MockOpenAIServer._(server, responses);
}

// ─────────────────────────────────────────────────────────────────────────────
// Worker-isolate FFI driver
// ─────────────────────────────────────────────────────────────────────────────

/// Arguments sent into the worker isolate. Must be sendable (plain JSON-like).
class _GenerateArgs {
  final String baseUrl;
  final String apiKey;
  final String modelId;
  final Object prompt; // String or List<Map>
  final Map<String, dynamic>? options;
  _GenerateArgs(this.baseUrl, this.apiKey, this.modelId, this.prompt, this.options);
}

/// Run a non-streaming `generateText` in a worker isolate.
Future<Map<String, dynamic>> runGenerateText(_GenerateArgs args) {
  return Isolate.run(() {
    final model = Model.openai(args.apiKey, args.modelId, baseUrl: args.baseUrl);
    try {
      return model.generateText(args.prompt, args.options);
    } finally {
      model.close();
    }
  });
}

/// Run a streaming `streamText` in a worker isolate, returning all parts.
Future<List<Map<String, dynamic>>> runStreamText(_GenerateArgs args) {
  return Isolate.run(() async {
    final model = Model.openai(args.apiKey, args.modelId, baseUrl: args.baseUrl);
    try {
      // streamText blocks synchronously until the stream completes, then
      // returns an already-closed stream containing the buffered parts.
      final parts = await model.streamText(args.prompt, args.options).toList();
      return parts;
    } finally {
      model.close();
    }
  });
}

// ─────────────────────────────────────────────────────────────────────────────
// Fixture responses (real OpenAI API shapes)
// ─────────────────────────────────────────────────────────────────────────────

const String plainOpenAIResponse = r'''
{"id":"chatcmpl-test","model":"gpt-4o","choices":[{"message":{"role":"assistant","content":"Rust is a systems programming language."},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":8,"total_tokens":18}}
''';

const String toolCallOpenAIResponse = r'''
{"id":"chatcmpl-tc","model":"gpt-4o","choices":[{"message":{"role":"assistant","content":null,"tool_calls":[{"id":"call_abc","type":"function","function":{"name":"get_weather","arguments":"{\"location\":\"Tokyo\"}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":20,"completion_tokens":10,"total_tokens":30}}
''';

/// Final assistant text emitted after the tool result is fed back in.
const String finalTextOpenAIResponse = r'''
{"id":"chatcmpl-2","model":"gpt-4o","choices":[{"message":{"role":"assistant","content":"The weather in Tokyo is sunny."},"finish_reason":"stop"}],"usage":{"prompt_tokens":30,"completion_tokens":8,"total_tokens":38}}
''';

/// SSE stream containing a tool call: name then arguments delta, then finish.
String buildToolCallSse() {
  final event1 = {
    'id': '1',
    'model': 'gpt-4o',
    'choices': [
      {
        'delta': {
          'role': 'assistant',
          'tool_calls': [
            {
              'index': 0,
              'id': 'call_xyz',
              'type': 'function',
              'function': {'name': 'get_weather', 'arguments': ''},
            }
          ],
        }
      }
    ],
  };
  final event2 = {
    'id': '1',
    'model': 'gpt-4o',
    'choices': [
      {
        'delta': {
          'tool_calls': [
            {
              'index': 0,
              'function': {'arguments': '{"location":"Tokyo"}'},
            }
          ],
        }
      }
    ],
  };
  final event3 = {
    'id': '1',
    'model': 'gpt-4o',
    'choices': [
      {'delta': {}, 'finish_reason': 'tool_calls'},
    ],
    'usage': {
      'prompt_tokens': 5,
      'completion_tokens': 2,
      'total_tokens': 7,
    },
  };
  final sb = StringBuffer()
    ..write('data: ${jsonEncode(event1)}\n\n')
    ..write('data: ${jsonEncode(event2)}\n\n')
    ..write('data: ${jsonEncode(event3)}\n\n')
    ..write('data: [DONE]\n\n');
  return sb.toString();
}

Map<String, dynamic> _weatherTool() {
  return {
    'type': 'function',
    'name': 'get_weather',
    'input_schema': {
      'type': 'object',
      'properties': {
        'location': {'type': 'string'},
      },
      'required': ['location'],
    },
  };
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

void main() {
  const apiKey = 'sk-test-mock-key';
  const modelId = 'gpt-4o';

  test('generateText parses tool_calls', () async {
    final server = await startMockServer([
      {'body': toolCallOpenAIResponse},
    ]);
    addTearDown(server.close);

    final result = await runGenerateText(_GenerateArgs(
      server.baseUrl,
      apiKey,
      modelId,
      'What is the weather in Tokyo?',
      {'tools': [_weatherTool()]},
    ));

    // The mock recorded exactly one POST to /chat/completions.
    expect(server.recorded, hasLength(1));
    expect(server.recorded.first.method, 'POST');
    expect(server.recorded.first.path, '/chat/completions');

    // Tool call parsed into the user-facing result.
    expect(result, contains('tool_calls'));
    final toolCalls = result['tool_calls'] as List;
    expect(toolCalls, hasLength(1));
    expect(toolCalls[0]['tool_name'], 'get_weather');
    expect(toolCalls[0]['tool_call_id'], 'call_abc');
    expect((toolCalls[0]['input'] as Map)['location'], 'Tokyo');

    // The raw provider result contains a ToolCall content variant.
    final raw = result['raw'] as Map<String, dynamic>;
    final content = raw['content'] as List;
    final hasToolCallVariant =
        content.any((c) => (c as Map).containsKey('ToolCall'));
    expect(hasToolCallVariant, isTrue);
  });

  test('multi-role messages reach provider', () async {
    final server = await startMockServer([
      {'body': plainOpenAIResponse},
    ]);
    addTearDown(server.close);

    final messages = [
      {'role': 'system', 'content': 'You are a helpful assistant.'},
      {'role': 'user', 'content': 'Hello'},
    ];

    final result = await runGenerateText(_GenerateArgs(
      server.baseUrl,
      apiKey,
      modelId,
      messages,
      null,
    ));

    // The request body sent to the provider contains both messages.
    expect(server.recorded, hasLength(1));
    final sentBody = server.recorded.first.body as Map<String, dynamic>;
    expect(sentBody['model'], modelId);
    final sentMessages = sentBody['messages'] as List;
    expect(sentMessages, hasLength(2));
    expect((sentMessages[0] as Map)['role'], 'system');
    expect((sentMessages[0] as Map)['content'], 'You are a helpful assistant.');
    expect((sentMessages[1] as Map)['role'], 'user');
    expect((sentMessages[1] as Map)['content'], 'Hello');

    // Sanity: generateText still returned text.
    expect(result['text'], 'Rust is a systems programming language.');
  });

  test('tool_choice reaches provider', () async {
    final server = await startMockServer([
      {'body': toolCallOpenAIResponse},
    ]);
    addTearDown(server.close);

    await runGenerateText(_GenerateArgs(
      server.baseUrl,
      apiKey,
      modelId,
      'What is the weather in Tokyo?',
      {
        'tools': [_weatherTool()],
        'tool_choice': 'required',
      },
    ));

    expect(server.recorded, hasLength(1));
    final sentBody = server.recorded.first.body as Map<String, dynamic>;
    // tool_choice is only emitted when tools are present; we pass both.
    expect(sentBody, contains('tools'));
    expect(sentBody['tool_choice'], 'required');
  });

  test('streamText parses tool-call stream parts', () async {
    final server = await startMockServer([
      {'sse': true, 'body': buildToolCallSse()},
    ]);
    addTearDown(server.close);

    final parts = await runStreamText(_GenerateArgs(
      server.baseUrl,
      apiKey,
      modelId,
      'What is the weather in Tokyo?',
      {'tools': [_weatherTool()]},
    ));

    // The request was a streaming POST.
    expect(server.recorded, hasLength(1));
    final sentBody = server.recorded.first.body as Map<String, dynamic>;
    expect(sentBody['stream'], true);

    // The stream delivered tool-call parts.
    expect(parts, isNotEmpty);
    final hasToolInputDelta =
        parts.any((p) => (p as Map).containsKey('ToolInputDelta'));
    final hasToolCall =
        parts.any((p) => (p as Map).containsKey('ToolCall'));
    expect(hasToolInputDelta || hasToolCall, isTrue,
        reason: 'expected a tool-call stream part; got: $parts');

    // The assembled ToolCall should carry the get_weather tool + Tokyo input.
    final toolCallPart = parts
        .cast<Map<String, dynamic>?>()
        .firstWhere((p) => p!.containsKey('ToolCall'));
    final tc = toolCallPart!['ToolCall'] as Map<String, dynamic>;
    expect(tc['tool_name'], 'get_weather');
    expect((tc['input'] as Map)['location'], 'Tokyo');
  });

  // Full tool-call round-trip: two generateText calls with a ToolResult fed
  // back in between.
  //
  // The second call's messages carry the assistant's prior tool call and the
  // tool's result. These are passed in the binding's content-part shape
  // (`content: [{type: 'tool_call' | 'tool_result', ...}]`) — that is what the
  // Rust `ModelMessage` deserializes (a message only has `role` + `content`,
  // where `content` is a string or a list of typed parts). The OpenAI wire
  // shape (`content: null, tool_calls: [...]` at message level) is what the
  // core *emits to the provider*; it is reconstructed from these parts, so the
  // recorded request body still carries `role: tool` + `tool_call_id`.
  test('tool round-trip: generateText → ToolResult → generateText', () async {
    final server = await startMockServer([
      {'body': toolCallOpenAIResponse},
      {'body': finalTextOpenAIResponse},
    ]);
    addTearDown(server.close);

    const userQuestion = "What's the weather in Tokyo?";

    // 1. First call: the model requests a tool call.
    final firstResult = await runGenerateText(_GenerateArgs(
      server.baseUrl,
      apiKey,
      modelId,
      userQuestion,
      {'tools': [_weatherTool()]},
    ));

    expect(firstResult, contains('tool_calls'));
    final toolCalls = firstResult['tool_calls'] as List;
    expect(toolCalls, hasLength(1));
    expect(toolCalls[0]['tool_name'], 'get_weather');
    expect(toolCalls[0]['tool_call_id'], 'call_abc');

    // 2. Build the follow-up messages: original user question, the assistant's
    //    tool call, and the tool result we "executed" and are filling back in.
    final messages = [
      {'role': 'user', 'content': userQuestion},
      {
        'role': 'assistant',
        'content': [
          {
            'type': 'tool_call',
            'tool_call_id': 'call_abc',
            'tool_name': 'get_weather',
            'input': {'location': 'Tokyo'},
          }
        ],
      },
      {
        'role': 'tool',
        'content': [
          {
            'type': 'tool_result',
            'tool_call_id': 'call_abc',
            'output': {'temperature': 22, 'condition': 'sunny'},
          }
        ],
      },
    ];

    // 3. Second call (same tools): the model produces the final text.
    final secondResult = await runGenerateText(_GenerateArgs(
      server.baseUrl,
      apiKey,
      modelId,
      messages,
      {'tools': [_weatherTool()]},
    ));

    expect(secondResult['text'], 'The weather in Tokyo is sunny.');

    // 4. Two requests were recorded.
    expect(server.recorded, hasLength(2));

    // 5. The second request carried all three messages, the last one being the
    //    tool result with the matching tool_call_id.
    final secondBody =
        server.recorded[1].body as Map<String, dynamic>;
    final sentMessages = secondBody['messages'] as List;
    expect(sentMessages, hasLength(3));
    final lastMessage = sentMessages[2] as Map<String, dynamic>;
    expect(lastMessage['role'], 'tool');
    expect(lastMessage['tool_call_id'], 'call_abc');
  });
}
