// Pure-Dart round trips for the typed `GenerateContent` / `TextStreamPart` unions,
// tool calls, options and results against the generated wire shapes in
// `bindings/node/src/types/`: camelCase fields, `type`-tagged unions with
// kebab-case values, optional fields absent (never null).
import 'dart:convert';

import 'package:aimux/types.dart';
import 'package:test/test.dart';

const _meta = {
  'openai': {'itemId': 'item_1'}
};
// The provider's LanguageModelV4File (`language-model-v4-file.ts`).
const _file = {
  'data': {'type': 'data', 'data': 'aGk='},
  'mediaType': 'image/png',
};
// The AI SDK's GeneratedFile is {base64, mediaType, providerMetadata?}
// (`generate-text/generated-file.ts:10-32`); a stream file part is
// {type, file, providerMetadata?} (`stream-language-model-call.ts:569-590`).
const _generatedFile = {'base64': 'aGk=', 'mediaType': 'image/png'};
// The call-level LanguageModelUsage of a result and of the stream's
// finish-step / finish parts: the AI SDK's (`types/usage.ts:10-67`),
// camelCase, an unknown count absent.
const _lmUsage = {
  'inputTokens': 3,
  'inputTokenDetails': {'cacheReadTokens': 1},
  'outputTokens': 2,
  'outputTokenDetails': <String, dynamic>{},
  'totalTokens': 5,
};

void main() {
  group('ToolCall', () {
    test('round-trips and omits absent optionals', () {
      final json = {
        'type': 'tool-call',
        'toolCallId': 'call_1',
        'toolName': 'get_weather',
        'input': {'city': 'Paris'},
        'providerExecuted': true,
        'dynamic': false,
        'providerMetadata': _meta,
        'invalid': true,
        'error': {'name': 'AI_NoSuchToolError', 'toolName': 'x'},
      };
      expect(ToolCall.fromJson(json).toJson(), Map.of(json)..remove('type'));

      final minimal = ToolCall(toolCallId: 'c', toolName: 't', input: const {});
      expect(
          minimal.toJson(), {'toolCallId': 'c', 'toolName': 't', 'input': {}});
    });

    // Provider metadata is a JS object upstream (@ai-sdk/provider
    // shared-v4-provider-metadata.ts:24), so its keys keep insertion order
    // (#229 NP2-1); a decoded Dart map keeps them too.
    test('provider namespaces keep insertion order', () {
      const metadata = '{"zeta":{"z":1,"a":2},"alpha":{},"mid":{}}';
      final text =
          '{"type":"tool-call","toolCallId":"call-1","toolName":"weather",'
          '"input":{},"providerMetadata":$metadata}';
      final call = ToolCall.fromJson(jsonDecode(text) as Map<String, dynamic>);
      expect(call.providerMetadata!.keys, ['zeta', 'alpha', 'mid']);
      expect(jsonEncode(call.providerMetadata), metadata);
    });

    // A result's toolCalls and sources are the AI SDK's TypedToolCall and
    // Source, tagged {type: 'tool-call'} (ai generate-text/tool-call.ts:8-14)
    // and {type: 'source', sourceType} (provider language-model-v4-source.ts:8,
    // :36); generate-text-result.ts:73, :78 (#229 NP2-7).
    test('standalone calls and sources use their schema tags', () {
      final source = {
        'type': 'source',
        'sourceType': 'url',
        'id': 's1',
        'url': 'https://example.com',
      };
      expect(Source.fromJson(source).toJson(), Map.of(source)..remove('type'));
      expect(RawToolCall(toolCallId: 'c', toolName: 't', input: '{}').toJson(),
          {'toolCallId': 'c', 'toolName': 't', 'input': '{}'});
      // The tag is a plain string: Dart emits single-value literals as String.
      expect(
          TypedToolCall.fromJson({
            'type': 'tool-call',
            'toolCallId': 'c',
            'toolName': 't',
            'input': {}
          }).toJson()['type'],
          'tool-call');
    });

    test('RawToolCall keeps every provider field when replacing input', () {
      final call = RawToolCall.fromJson({
        'toolCallId': 'call-1',
        'toolName': 'weather',
        'input': '{"town":"Tokyo"}',
        'providerExecuted': true,
        'dynamic': true,
        'providerMetadata': _meta,
      });
      expect(
          RawToolCall.fromJson({...call.toJson(), 'input': '{"city":"Tokyo"}'})
              .toJson(),
          {
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
        expect(c.type, json['type']);
        expect(c.toJson(), json);
      });
    }

    test('unknown type is rejected', () {
      expect(() => GenerateContent.fromJson({'type': 'future', 'n': 42}),
          throwsFormatException);
    });
  });

  group('TextStreamPart', () {
    final call = {
      'type': 'tool-call',
      'toolCallId': 'c1',
      'toolName': 'w',
      'input': {'a': 1},
      'invalid': true,
    };
    final cases = <Map<String, dynamic>>[
      {'type': 'text-start', 'id': 't'},
      {
        'type': 'text-delta',
        'id': 't',
        'text': 'hel',
        'providerMetadata': _meta
      },
      {'type': 'text-end', 'id': 't'},
      {'type': 'start'},
      {
        'type': 'start-step',
        'request': <String, dynamic>{},
        'warnings': [
          {'type': 'other', 'message': 'm'}
        ]
      },
      {
        'type': 'finish-step',
        'response': {
          'id': 'r',
          'modelId': 'gpt-4o',
          'timestamp': '2026-01-01T00:00:00.000Z'
        },
        'usage': _lmUsage,
        'performance': {
          'stepTimeMs': 1.0,
          'responseTimeMs': 1.0,
          'effectiveOutputTokensPerSecond': 2.0,
          'effectiveTotalTokensPerSecond': 5.0,
          'toolExecutionMs': {}
        },
        'finishReason': 'tool-calls',
        'rawFinishReason': 'tool_calls',
      },
      {'type': 'finish', 'finishReason': 'stop', 'totalUsage': _lmUsage},
      {'type': 'abort', 'reason': 'stop'},
      {
        'type': 'error',
        'error': {
          'name': 'AI_APICallError',
          'message': 'boom',
          'url': 'https://example.com',
          'requestBodyValues': null,
          'isRetryable': false
        },
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
        'input': {'q': 'rust'},
        'output': {'count': 3},
        'providerExecuted': true,
      },
      // A deferred result has no input.
      {
        'type': 'tool-result',
        'toolCallId': 'srv-0',
        'toolName': 'code_execution',
        'output': 'done'
      },
      {
        'type': 'tool-error',
        'toolCallId': 'srv-2',
        'toolName': 'code_execution',
        'input': {'code': '1/0'},
        'error': {'error_code': 'unavailable'},
        'providerExecuted': true,
      },
      {'type': 'tool-output-denied', 'toolCallId': 'c', 'toolName': 't'},
      {'type': 'file', 'file': _generatedFile, 'providerMetadata': _meta},
      {'type': 'reasoning-file', 'file': _generatedFile},
      {'type': 'custom', 'kind': 'k'},
      {
        'type': 'tool-approval-request',
        'approvalId': 'a',
        'toolCall': call,
        'reason': 'r',
      },
      {
        'type': 'tool-approval-response',
        'approvalId': 'a',
        'toolCall': call,
        'approved': false,
      },
      {'type': 'reasoning-start', 'id': 'r1'},
      {'type': 'reasoning-delta', 'id': 'r1', 'text': 'thi'},
      {'type': 'reasoning-end', 'id': 'r1'},
      {'type': 'source', 'sourceType': 'url', 'id': 's', 'url': 'https://x'},
      {
        'type': 'raw',
        'rawValue': {'raw': 'chunk'}
      },
    ];
    for (final json in cases) {
      test(json['type'] as String, () {
        final p = TextStreamPart.fromJson(json);
        expect(p.type, json['type']);
        expect(p.toJson(), json);
      });
    }

    test('unknown type is rejected', () {
      expect(() => TextStreamPart.fromJson({'type': 'future-part', 'n': 7}),
          throwsFormatException);
    });
  });

  test('GeneratedFile is base64 and mediaType', () {
    final file = GeneratedFile.fromJson(_generatedFile);
    expect(base64Decode(file.base64.value), [104, 105]);
    expect(file.toJson(), _generatedFile);
    final part =
        TextStreamPart.fromJson({'type': 'file', 'file': _generatedFile});
    expect((part as TextStreamPartFile).file.base64.value, 'aGk=');
  });

  group('options and results', () {
    test('GenerateTextOptions uses camelCase keys and omits unset fields', () {
      final options = GenerateTextOptions.fromJson({
        'maxOutputTokens': 10,
        'stopSequences': ['x'],
        'toolChoice': {'type': 'tool', 'toolName': 'w'},
        'timeout': {'totalMs': 1000},
        'includeRawChunks': true,
        'sessionId': 'sess-1',
        'tools': [
          {
            'type': 'function',
            'name': 'w',
            'inputSchema': {'type': 'object'}
          }
        ],
      });
      final json = options.toJson();
      expect(json, {
        'maxOutputTokens': 10,
        'stopSequences': ['x'],
        'toolChoice': {'type': 'tool', 'toolName': 'w'},
        'timeout': {'totalMs': 1000},
        'includeRawChunks': true,
        'sessionId': 'sess-1',
        'tools': [
          {
            'type': 'function',
            'name': 'w',
            'inputSchema': {'type': 'object'}
          }
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
        'reasoning': [],
        'files': [],
        'sources': [],
        'toolCalls': [
          {'type': 'tool-call', 'toolCallId': 'c', 'toolName': 't', 'input': {}}
        ],
        'toolResults': [],
        'finishReason': 'tool-calls',
        'rawFinishReason': 'tool_calls',
        'usage': _lmUsage,
        'totalUsage': _lmUsage,
        'warnings': [],
        'request': {},
        'response': {
          'id': 'r',
          'modelId': 'gpt-4o',
          'timestamp': '2026-01-01T00:00:00.000Z',
          'messages': []
        },
        'responseMessages': [
          {'role': 'assistant', 'content': 'hi'}
        ],
        'steps': [],
      })) as Map<String, dynamic>;
      final result = GenerateTextResult.fromJson(json);
      expect(result.finishReason, FinishReasonUnified.toolCalls);
      expect(result.usage.totalTokens, 5);
      expect(result.totalUsage.inputTokenDetails.cacheReadTokens, 1);
      expect(result.reasoningText, isNull);
      expect(result.output.isPresent, isFalse);
      expect(result.steps, isEmpty);
      expect(jsonDecode(jsonEncode(result)), json);

      // A parsed JSON null output is written back as null, apart from no
      // output (the public JSON keeps null apart from undefined).
      final parsedNull = Map<String, dynamic>.of(json)..['output'] = null;
      final withNull = GenerateTextResult.fromJson(parsedNull);
      expect(withNull.output.value, isNull);
      expect(withNull.output.isPresent, isTrue);
      expect(jsonDecode(jsonEncode(withNull)), parsedNull);
    });

    test('GenerateObjectResult round-trips', () {
      final json = jsonDecode(jsonEncode({
        'object': {'answer': 42},
        'finishReason': 'stop',
        'usage': _lmUsage,
        'warnings': [],
        'request': {},
        'response': {
          'id': 'r',
          'timestamp': '2026-01-01T00:00:00.000Z',
          'modelId': 'gpt-4o'
        },
      })) as Map<String, dynamic>;
      final result = GenerateObjectResult.fromJson(json);
      expect((result.object as Map<String, dynamic>)['answer'], 42);
      expect(result.response.modelId, 'gpt-4o');
      expect(jsonDecode(jsonEncode(result)), json);
    });
  });
}
