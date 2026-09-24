// End-to-end repair through [TypedModel] (RFC-0035): the provider returns
// arguments the tool schema rejects, the core marks the call invalid, and the
// hook runs on the Dart side. `tool_call_repair_test.dart` covers the loop
// itself; this file pins its wiring into the typed entry points.
//
// The blocking FFI call runs in a worker isolate so the mock server in this
// isolate can answer (see `typed_model_test.dart`). Everything returned from
// the worker is a plain sendable value.

import 'dart:convert';
import 'dart:isolate';

import 'package:aimux/aimux.dart';
import 'package:aimux/typed_model.dart';
import 'package:aimux/types.dart';
import 'package:test/test.dart';

import 'typed_model_test.dart'
    show buildToolCallSse, startMockServer, toolCallOpenAIResponse;

/// Accepts `city` only, so the canned `{"location":"Tokyo"}` is invalid.
GenerateTextOptions _strictOptions() => GenerateTextOptions(
      tools: [
        Tool.function(FunctionTool(
          name: 'get_weather',
          inputSchema: {
            'type': 'object',
            'properties': {
              'city': {'type': 'string'},
            },
            'required': ['city'],
            'additionalProperties': false,
          },
        )),
      ],
      repairToolCall: _toCity,
    );

RawToolCall? _toCity(ToolCallRepairContext context) =>
    context.toolCall.copyWith(input: '{"city":"Tokyo"}');

void main() {
  test('generateText repairs the call and its transcript part', () async {
    final server = await startMockServer([
      {'body': toolCallOpenAIResponse},
    ]);
    addTearDown(server.close);
    final baseUrl = server.baseUrl;

    final result = await Isolate.run(() {
      final typed = TypedModel(Model.openai('sk-test', 'gpt-4o', baseUrl: baseUrl));
      try {
        return jsonDecode(jsonEncode(typed.generateText('weather?', _strictOptions())))
            as Map<String, dynamic>;
      } finally {
        typed.close();
      }
    });

    final call = (result['tool_calls'] as List).single as Map<String, dynamic>;
    expect(call['invalid'], isNull);
    expect(call['input'], {'city': 'Tokyo'});
    final part = (result['response_messages'] as List)
        .expand((m) => (m as Map<String, dynamic>)['content'] as List)
        .whereType<Map<String, dynamic>>()
        .singleWhere((p) => p['type'] == 'tool_call');
    expect(part['input'], {'city': 'Tokyo'});
  });

  test('generateTextAsOpenAI carries the repaired arguments', () async {
    final server = await startMockServer([
      {'body': toolCallOpenAIResponse},
    ]);
    addTearDown(server.close);
    final baseUrl = server.baseUrl;

    final arguments = await Isolate.run(() {
      final typed = TypedModel(Model.openai('sk-test', 'gpt-4o', baseUrl: baseUrl));
      try {
        final completion = typed.generateTextAsOpenAI('weather?', _strictOptions());
        return completion.choices.first.message.toolCalls!.single.function.arguments;
      } finally {
        typed.close();
      }
    });

    expect(jsonDecode(arguments), {'city': 'Tokyo'});
  });

  test('streamText delivers the repaired ToolCall part', () async {
    final server = await startMockServer([
      {'sse': true, 'body': buildToolCallSse()},
    ]);
    addTearDown(server.close);
    final baseUrl = server.baseUrl;

    final input = await Isolate.run(() async {
      final typed = TypedModel(Model.openai('sk-test', 'gpt-4o', baseUrl: baseUrl));
      try {
        final parts = await typed.streamText('weather?', _strictOptions()).toList();
        final call = parts.whereType<StreamPartToolCall>().single;
        return {'invalid': call.invalid, 'input': call.input};
      } finally {
        typed.close();
      }
    });

    expect(input['invalid'], isNull);
    expect(input['input'], {'city': 'Tokyo'});
  });

  test('RawToolCall keeps every provider field through a repair', () {
    final call = RawToolCall.fromJson({
      'tool_call_id': 'call-1',
      'tool_name': 'weather',
      'input': '{"town":"Tokyo"}',
      'provider_executed': true,
      'dynamic': true,
      'thought_signature': 'sig',
      'provider_metadata': {
        'google': {'x': 1},
      },
    });
    expect(call.copyWith(input: '{"city":"Tokyo"}').toJson(), {
      'tool_call_id': 'call-1',
      'tool_name': 'weather',
      'input': '{"city":"Tokyo"}',
      'provider_executed': true,
      'dynamic': true,
      'thought_signature': 'sig',
      'provider_metadata': {
        'google': {'x': 1},
      },
    });
  });
}
