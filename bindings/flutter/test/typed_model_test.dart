// Typed e2e tests for the aimux Flutter/Dart typed wrapper (`TypedModel`).
//
// Drives the *real* Rust core (via dart:ffi) through the typed
// `GenerateTextOptions` → `GenerateTextResult` boundary against a mock
// OpenAI-compatible HTTP server (same mock as `structured_e2e_test.dart`).
// No real API calls are made.
//
// Run:
//   export PATH="/home/eric8810/.local/dart-sdk/bin:$PATH"
//   export LD_LIBRARY_PATH="/media/eric8810/fast-deliver/code/aimux/target/debug:$LD_LIBRARY_PATH"
//   cd bindings/flutter && dart test test/typed_model_test.dart
//
// ## Why the FFI call runs in a worker isolate
//
// Same reason as `structured_e2e_test.dart`: `Model.generateText` /
// `Model.streamText` are *synchronous* FFI calls that block the calling
// isolate's event loop while the Rust core blocks on an HTTP response. If the
// mock `HttpServer` lived in the same isolate it could never answer →
// deadlock. The mock server runs in the main test isolate; the blocking FFI
// call (here wrapped by `TypedModel`) runs in a worker via `Isolate.run`.
//
// `TypedModel` returns typed class instances, which are not directly sendable
// across isolates. The worker therefore round-trips the result through
// `toJson()` (a sendable `Map`) and the main isolate reconstructs it with
// `GenerateTextResult.fromJson`, exercising both directions.

import 'dart:convert';
import 'dart:io';
import 'dart:isolate';

import 'package:aimux/aimux.dart';
import 'package:aimux/typed_model.dart';
import 'package:aimux/types.dart';
import 'package:test/test.dart';

// ─────────────────────────────────────────────────────────────────────────────
// Mock server (mirrors structured_e2e_test.dart)
// ─────────────────────────────────────────────────────────────────────────────

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
      response.headers.contentType = ContentType.parse('text/event-stream');
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
// Worker-isolate FFI driver (typed)
// ─────────────────────────────────────────────────────────────────────────────

/// Sendable arguments for the worker isolate. Only plain JSON-like fields, so
/// the instance can cross the isolate boundary (same approach as
/// `structured_e2e_test.dart`'s `_GenerateArgs`).
class _TypedArgs {
  final String baseUrl;
  final String apiKey;
  final String modelId;
  final String? prompt; // for generateText (string prompt)
  final List<Map<String, dynamic>>? messages; // for generateTextMessages
  final List<Tool>? tools;
  final ToolChoice? toolChoice;
  final int? maxOutputTokens;
  final double? temperature;
  _TypedArgs({
    required this.baseUrl,
    required this.apiKey,
    required this.modelId,
    this.prompt,
    this.messages,
    this.tools,
    this.toolChoice,
    this.maxOutputTokens,
    this.temperature,
  });
}

/// Run a typed `generateText` in a worker isolate, returning the result as a
/// sendable `Map`. The worker constructs the typed `GenerateTextResult` (so the
/// real wrapper is exercised) then deep-flattens it via a JSON encode/decode
/// round-trip: `toJson()` alone leaves nested `@JsonSerializable` objects as
/// instances (not plain maps), which cannot be re-parsed by `fromJson`.
Future<Map<String, dynamic>> runTypedGenerateText(_TypedArgs args) {
  return Isolate.run(() {
    final model = Model.openai(args.apiKey, args.modelId, baseUrl: args.baseUrl);
    final typed = TypedModel(model);
    try {
      final hasOptions = args.tools != null ||
          args.toolChoice != null ||
          args.maxOutputTokens != null ||
          args.temperature != null;
      final options = hasOptions
          ? GenerateTextOptions(
              tools: args.tools,
              toolChoice: args.toolChoice,
              maxOutputTokens: args.maxOutputTokens,
              temperature: args.temperature,
            )
          : null;
      final result = typed.generateText(args.prompt!, options);
      return jsonDecode(jsonEncode(result)) as Map<String, dynamic>;
    } finally {
      typed.close();
    }
  });
}

/// Run a typed `generateTextMessages` (multi-turn) in a worker isolate.
Future<Map<String, dynamic>> runTypedGenerateTextMessages(_TypedArgs args) {
  return Isolate.run(() {
    final model = Model.openai(args.apiKey, args.modelId, baseUrl: args.baseUrl);
    final typed = TypedModel(model);
    try {
      final messages = args.messages!
          .map((m) => ModelMessage(role: m['role'] as String, content: m['content']!))
          .toList();
      final result = typed.generateTextMessages(messages);
      return jsonDecode(jsonEncode(result)) as Map<String, dynamic>;
    } finally {
      typed.close();
    }
  });
}

/// Run a typed `streamText` in a worker isolate, returning the parsed parts.
Future<List<StreamPart>> runTypedStreamText(_TypedArgs args) {
  return Isolate.run(() async {
    final model = Model.openai(args.apiKey, args.modelId, baseUrl: args.baseUrl);
    final typed = TypedModel(model);
    try {
      final options = args.tools != null
          ? GenerateTextOptions(tools: args.tools, toolChoice: args.toolChoice)
          : null;
      // streamText blocks synchronously, then returns an already-closed
      // stream of buffered parts.
      final parts = await typed.streamText(args.prompt!, options).toList();
      return parts;
    } finally {
      typed.close();
    }
  });
}

// ─────────────────────────────────────────────────────────────────────────────
// Fixture responses (real OpenAI API shapes — same as structured_e2e_test.dart)
// ─────────────────────────────────────────────────────────────────────────────

const String plainOpenAIResponse = r'''
{"id":"chatcmpl-test","model":"gpt-4o","choices":[{"message":{"role":"assistant","content":"Rust is a systems programming language."},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":8,"total_tokens":18}}
''';

const String toolCallOpenAIResponse = r'''
{"id":"chatcmpl-tc","model":"gpt-4o","choices":[{"message":{"role":"assistant","content":null,"tool_calls":[{"id":"call_abc","type":"function","function":{"name":"get_weather","arguments":"{\"location\":\"Tokyo\"}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":20,"completion_tokens":10,"total_tokens":30}}
''';

/// SSE stream: a tool-call name → arguments delta → finish.
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

Tool _weatherTool() {
  return Tool.function(FunctionTool(
    name: 'get_weather',
    inputSchema: {
      'type': 'object',
      'properties': {
        'location': {'type': 'string'},
      },
      'required': ['location'],
    },
  ));
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

void main() {
  const apiKey = 'sk-test-mock-key';
  const modelId = 'gpt-4o';

  test('generateText returns a typed GenerateTextResult (text + tool calls)', () async {
    final server = await startMockServer([
      {'body': toolCallOpenAIResponse},
    ]);
    addTearDown(server.close);

    final resultJson = await runTypedGenerateText(_TypedArgs(
      baseUrl: server.baseUrl,
      apiKey: apiKey,
      modelId: modelId,
      prompt: 'What is the weather in Tokyo?',
      tools: [_weatherTool()],
      toolChoice: ToolChoice.required,
      maxOutputTokens: 1024,
      temperature: 0.7,
    ));
    final result = GenerateTextResult.fromJson(resultJson);

    // Typed field access on the result.
    expect(result, isA<GenerateTextResult>());
    expect(result.text, isA<String>());
    expect(result.toolCalls, hasLength(1));
    expect(result.toolCalls[0], isA<ToolCall>());
    expect(result.toolCalls[0].toolName, 'get_weather');
    expect(result.toolCalls[0].toolCallId, 'call_abc');
    expect(result.toolCalls[0].input?['location'], 'Tokyo');
    expect(result.raw, isA<GenerateResult>());

    // finish_reason + usage are typed, not dynamic maps.
    expect(result.finishReason, isA<FinishReason>());
    expect(result.finishReason.unified, 'tool-calls');
    expect(result.finishReason.raw, 'tool_calls');
    expect(result.usage, isA<Usage>());
    expect(result.usage.inputTokens, isA<TokenUsage>());
    expect(result.usage.inputTokens.total, 20);
    expect(result.usage.outputTokens.total, 10);

    // The raw provider result carries a ToolCall content variant.
    final content = result.raw.content;
    expect(content.any((c) => c.tag == 'ToolCall'), isTrue);

    // tools / tool_choice / max_output_tokens / temperature reached provider.
    expect(server.recorded, hasLength(1));
    expect(server.recorded.first.method, 'POST');
    expect(server.recorded.first.path, '/chat/completions');
    final sentBody = server.recorded.first.body as Map<String, dynamic>;
    expect(sentBody, contains('tools'));
    expect(sentBody['tool_choice'], 'required');
    // gpt-4o is a non-reasoning model, so max_output_tokens maps to `max_tokens`.
    expect(sentBody['max_tokens'], 1024);
    expect(sentBody['temperature'], closeTo(0.7, 1e-9));
  });

  test('generateText with a plain prompt returns text', () async {
    final server = await startMockServer([
      {'body': plainOpenAIResponse},
    ]);
    addTearDown(server.close);

    final resultJson = await runTypedGenerateText(_TypedArgs(
      baseUrl: server.baseUrl,
      apiKey: apiKey,
      modelId: modelId,
      prompt: 'What is Rust?',
    ));
    final result = GenerateTextResult.fromJson(resultJson);

    expect(result.text, 'Rust is a systems programming language.');
    expect(result.toolCalls, isEmpty);
    expect(result.finishReason.unified, 'stop');
    expect(result.finishReason.raw, 'stop');
    expect(result.usage.inputTokens.total, 10);
    expect(result.usage.outputTokens.total, 8);
  });

  test('generateTextMessages passes multi-role messages to provider', () async {
    final server = await startMockServer([
      {'body': plainOpenAIResponse},
    ]);
    addTearDown(server.close);

    final resultJson = await runTypedGenerateTextMessages(_TypedArgs(
      baseUrl: server.baseUrl,
      apiKey: apiKey,
      modelId: modelId,
      messages: [
        {'role': 'system', 'content': 'You are a helpful assistant.'},
        {'role': 'user', 'content': 'Hello'},
      ],
    ));
    final result = GenerateTextResult.fromJson(resultJson);

    expect(result.text, 'Rust is a systems programming language.');

    // Both typed messages reached the provider, preserving role + content.
    expect(server.recorded, hasLength(1));
    final sentBody = server.recorded.first.body as Map<String, dynamic>;
    expect(sentBody['model'], modelId);
    final sentMessages = sentBody['messages'] as List;
    expect(sentMessages, hasLength(2));
    expect((sentMessages[0] as Map)['role'], 'system');
    expect((sentMessages[0] as Map)['content'], 'You are a helpful assistant.');
    expect((sentMessages[1] as Map)['role'], 'user');
    expect((sentMessages[1] as Map)['content'], 'Hello');
  });

  test('tool_choice object form reaches provider', () async {
    final server = await startMockServer([
      {'body': toolCallOpenAIResponse},
    ]);
    addTearDown(server.close);

    await runTypedGenerateText(_TypedArgs(
      baseUrl: server.baseUrl,
      apiKey: apiKey,
      modelId: modelId,
      prompt: 'What is the weather in Tokyo?',
      tools: [_weatherTool()],
      toolChoice: ToolChoice.tool('get_weather'),
    ));

    expect(server.recorded, hasLength(1));
    final sentBody = server.recorded.first.body as Map<String, dynamic>;
    expect(sentBody, contains('tools'));
    // The Rust core maps the `ToolChoice::Tool { toolName }` form onto the
    // provider's wire format (OpenAI: {type:function, function:{name}}).
    expect(sentBody['tool_choice'], {
      'type': 'function',
      'function': {'name': 'get_weather'},
    });
  });

  test('streamText yields typed StreamParts with tool-call accessors', () async {
    final server = await startMockServer([
      {'sse': true, 'body': buildToolCallSse()},
    ]);
    addTearDown(server.close);

    final parts = await runTypedStreamText(_TypedArgs(
      baseUrl: server.baseUrl,
      apiKey: apiKey,
      modelId: modelId,
      prompt: 'What is the weather in Tokyo?',
      tools: [_weatherTool()],
    ));

    expect(parts, isNotEmpty);
    // Every part is a single-key tagged union with a non-empty tag.
    expect(parts.every((p) => p.type.isNotEmpty), isTrue);

    // The ToolCall part is accessible via typed getters.
    final toolCall = parts.firstWhere((p) => p.isToolCall);
    expect(toolCall.type, 'ToolCall');
    expect(toolCall.toolName, 'get_weather');
    expect(toolCall.toolCallId, 'call_xyz');
    expect(toolCall.toolInput?['location'], 'Tokyo');

    // A Finish part carries typed usage.
    final finish = parts.firstWhere((p) => p.isFinish);
    expect(finish.type, 'Finish');
    expect(finish.finishUsage, isA<Usage>());
    expect(finish.finishUsage?.inputTokens.total, 5);
    expect(finish.finishUsage?.outputTokens.total, 2);

    // The request was a streaming POST.
    expect(server.recorded, hasLength(1));
    final sentBody = server.recorded.first.body as Map<String, dynamic>;
    expect(sentBody['stream'], true);
  });
}
