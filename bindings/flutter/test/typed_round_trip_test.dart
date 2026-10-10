// Pure-Dart round trips for the typed `GenerateContent` / `StreamPart` unions,
// tool calls, options and results against the generated wire shapes in
// `bindings/node/src/types/`: camelCase fields, `type`-tagged unions with
// kebab-case values, optional fields absent (never null).
import 'dart:convert';

import 'package:aimux/types.dart';
import 'package:test/test.dart';

const _meta = {
  'openai': {'itemId': 'item_1'}
};
const _file = {
  'data': {'type': 'data', 'data': 'aGk='},
  'mediaType': 'image/png',
};
const _usage = {
  'inputTokens': {'total': 5, 'noCache': 5},
  'outputTokens': {'total': 2},
};
const _finish = {'unified': 'stop', 'raw': 'stop'};

void main() {
  group('ToolCall', () {
    test('round-trips and omits absent optionals', () {
      final json = {
        'toolCallId': 'call_1',
        'toolName': 'get_weather',
        'input': {'city': 'Paris'},
        'providerExecuted': true,
        'dynamic': false,
        'providerMetadata': _meta,
        'invalid': true,
        'error': {'name': 'AI_NoSuchToolError', 'toolName': 'x'},
      };
      expect(ToolCall.fromJson(json).toJson(), json);

      final minimal = ToolCall(toolCallId: 'c', toolName: 't', input: const {});
      expect(minimal.toJson().keys, ['toolCallId', 'toolName', 'input']);
    });

    test('RawToolCall keeps every provider field through copyWith', () {
      final call = RawToolCall.fromJson({
        'toolCallId': 'call-1',
        'toolName': 'weather',
        'input': '{"town":"Tokyo"}',
        'providerExecuted': true,
        'dynamic': true,
        'providerMetadata': _meta,
      });
      expect(call.copyWith(input: '{"city":"Tokyo"}').toJson(), {
        'toolCallId': 'call-1',
        'toolName': 'weather',
        'input': '{"city":"Tokyo"}',
        'providerExecuted': true,
        'dynamic': true,
        'providerMetadata': _meta,
      });
    });
  });

  group('GenerateContent', () {
    final cases = <Map<String, dynamic>>[
      {'type': 'text', 'text': 'hi', 'providerMetadata': _meta},
      {
        'type': 'tool-call',
        'toolCallId': 'c1',
        'toolName': 'w',
        'input': '{"a":1}',
        'dynamic': true,
      },
      {'type': 'source', 'sourceType': 'url', 'id': 's', 'url': 'https://x'},
      {
        'type': 'source',
        'sourceType': 'document',
        'id': 's',
        'mediaType': 'application/pdf',
        'title': 'T',
        'filename': 'f.pdf',
      },
      {'type': 'reasoning', 'text': 'hmm'},
      {'type': 'file', ..._file},
      {'type': 'reasoning-file', ..._file},
      {'type': 'custom', 'kind': 'k'},
      {'type': 'tool-approval-request', 'approvalId': 'a', 'toolCallId': 'c'},
      {
        'type': 'tool-result',
        'toolCallId': 'c1',
        'toolName': 'web_search',
        'result': {'count': 3},
        'isError': false,
        'preliminary': true,
        'dynamic': false,
      },
    ];
    for (final json in cases) {
      test(json['type'] as String, () {
        final c = GenerateContent.fromJson(json);
        expect(c, isNot(isA<GenerateContentUnknown>()));
        expect(c.tag, json['type']);
        expect(c.toJson(), json);
      });
    }

    test('unknown type passes through verbatim', () {
      final json = {'type': 'future', 'n': 42};
      final c = GenerateContent.fromJson(json);
      expect(c, isA<GenerateContentUnknown>());
      expect(c.tag, 'future');
      expect(c.toJson(), json);
    });
  });

  group('StreamPart', () {
    final call = {
      'toolCallId': 'c1',
      'toolName': 'w',
      'input': {'a': 1},
      'invalid': true,
    };
    final cases = <Map<String, dynamic>>[
      {'type': 'text-start', 'id': 't'},
      {'type': 'text-delta', 'id': 't', 'delta': 'hel', 'providerMetadata': _meta},
      {'type': 'text-end', 'id': 't'},
      {
        'type': 'stream-start',
        'warnings': [
          {'type': 'other', 'message': 'm'}
        ]
      },
      {'type': 'finish', 'finishReason': _finish, 'usage': _usage},
      {
        'type': 'finish-step',
        'finishReason': _finish,
        'usage': _usage,
        'response': {'id': 'r', 'modelId': 'gpt-4o'},
      },
      {
        'type': 'error',
        'error': {'name': 'AI_APICallError', 'message': 'boom', 'isRetryable': false},
      },
      {
        'type': 'tool-input-start',
        'id': 'tc',
        'toolName': 'w',
        'providerExecuted': true,
        'dynamic': false,
        'title': 'Weather',
      },
      {'type': 'tool-input-delta', 'id': 'tc', 'delta': '{"lo'},
      {'type': 'tool-input-end', 'id': 'tc'},
      {'type': 'tool-call', ...call},
      {
        'type': 'tool-result',
        'toolCallId': 'c1',
        'toolName': 'w',
        'result': {'count': 3},
      },
      {'type': 'file', ..._file},
      {'type': 'reasoning-file', 'file': _file},
      {'type': 'custom', 'kind': 'k'},
      {
        'type': 'tool-approval-request',
        'approvalId': 'a',
        'toolCall': call,
        'reason': 'r',
      },
      {'type': 'reasoning-start', 'id': 'r1'},
      {'type': 'reasoning-delta', 'id': 'r1', 'delta': 'thi'},
      {'type': 'reasoning-end', 'id': 'r1'},
      {'type': 'source', 'sourceType': 'url', 'id': 's', 'url': 'https://x'},
      {'type': 'raw', 'rawValue': {'raw': 'chunk'}},
    ];
    for (final json in cases) {
      test(json['type'] as String, () {
        final p = StreamPart.fromJson(json);
        expect(p, isNot(isA<StreamPartUnknown>()));
        expect(p.type, json['type']);
        expect(p.toJson(), json);
      });
    }

    test('unknown type passes through verbatim', () {
      final json = {'type': 'future-part', 'n': 7};
      final p = StreamPart.fromJson(json);
      expect(p, isA<StreamPartUnknown>());
      expect(p.type, 'future-part');
      expect(p.toJson(), json);
    });
  });

  group('options and results', () {
    test('GenerateTextOptions uses camelCase keys and omits unset fields', () {
      final options = GenerateTextOptions(
        maxOutputTokens: 10,
        stopSequences: ['x'],
        toolChoice: ToolChoice.tool('w'),
        timeout: TimeoutConfiguration(totalMs: 1000),
        includeRawChunks: true,
        sessionId: 'sess-1',
        tools: [
          Tool.function(FunctionTool(name: 'w', inputSchema: {'type': 'object'})),
        ],
      );
      final json = options.toJson();
      expect(json, {
        'maxOutputTokens': 10,
        'stopSequences': ['x'],
        'toolChoice': {'type': 'tool', 'toolName': 'w'},
        'timeout': {'totalMs': 1000},
        'includeRawChunks': true,
        'sessionId': 'sess-1',
        'tools': [
          {'type': 'function', 'name': 'w', 'inputSchema': {'type': 'object'}}
        ],
      });
      expect(GenerateTextOptions.fromJson(json).toJson(), json);
      expect(GenerateTextOptions().toJson(), isEmpty);
    });

    test('provider tool', () {
      final json = {
        'type': 'provider',
        'id': 'anthropic.web_search_20250305',
        'name': 'web_search',
        'args': {'maxUses': 1},
      };
      expect(Tool.fromJson(json).toJson(), json);
    });

    test('GenerateTextResult round-trips', () {
      final json = jsonDecode(jsonEncode({
        'content': [
          {'type': 'text', 'text': 'hi'}
        ],
        'text': 'hi',
        'toolCalls': [
          {'toolCallId': 'c', 'toolName': 't', 'input': {}}
        ],
        'finishReason': _finish,
        'usage': _usage,
        'warnings': [],
        'raw': {
          'content': [
            {'type': 'text', 'text': 'hi'}
          ],
          'finishReason': _finish,
          'usage': _usage,
          'warnings': [],
        },
        'reasoning': [],
        'reasoningText': '',
        'sources': [],
        'files': [],
        'responseMessages': [
          {'role': 'assistant', 'content': 'hi'}
        ],
        'rawFinishReason': 'stop',
        'request': {},
        'response': {'id': 'r'},
        'totalUsage': _usage,
      })) as Map<String, dynamic>;
      final result = GenerateTextResult.fromJson(json);
      expect(result.finishReason.unified, 'stop');
      expect(result.totalUsage.inputTokens.total, 5);
      expect(result.raw.content.single, isA<GenerateContentText>());
      expect(jsonDecode(jsonEncode(result)), json);
    });
  });
}
