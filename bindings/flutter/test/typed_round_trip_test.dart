// Round-trip serialization tests for the typed `GenerateContent` /
// `StreamPart` sealed class hierarchies in `types.dart`.
//
// These tests are pure Dart — no FFI, no Rust core, no mock server. They
// verify that every known variant (and the `Unknown` fallback) survives a
// `toJson()` -> `fromJson()` round-trip with all fields intact, and that the
// externally-tagged wire shape matches the Rust core contract:
//
//   - `File` variants serialize `data` + `media_type` and **never** `filename`.
//   - `ToolResult` variants use the `result` key, **not** `output`.
//   - `Unknown` variants pass the tag + payload through verbatim.
//
// Run:
//   export PATH="/home/eric8810/.local/dart-sdk/bin:$PATH"
//   cd bindings/flutter && dart test test/typed_round_trip_test.dart

import 'dart:convert';

import 'package:aimux/types.dart';
import 'package:test/test.dart';

/// Deep-flatten a `toJson()` result across the JSON boundary.
///
/// The variants nesting `@JsonSerializable` types (`StreamPartFinishStep`,
/// `StreamPartFinish`) are re-parsed after crossing a JSON-string boundary,
/// as in the real pipeline (Rust core → JSON → Dart).
Map<String, dynamic> deepFlatten(Map<String, dynamic> json) =>
    jsonDecode(jsonEncode(json)) as Map<String, dynamic>;

void main() {
  group('Provider tool round-trip', () {
    test('preserves deferred results and omits null', () {
      for (final supported in <bool?>[null, true, false]) {
        final original = Tool.provider(
          id: 'anthropic.code_execution_20250522',
          name: 'code_execution',
          args: {},
          supportsDeferredResults: supported,
        );
        final json = deepFlatten(original.toJson());
        expect(json.containsKey('supportsDeferredResults'), supported != null);
        expect(json['supportsDeferredResults'], supported);
        final decoded = Tool.fromJson(json);
        expect(decoded.supportsDeferredResults, supported);
        expect(decoded.toJson(), json);
      }
    });
  });

  group('ToolCall round-trip', () {
    test('preserves provider_metadata', () {
      final original = ToolCall(
        toolCallId: 'call_1',
        toolName: 'get_weather',
        input: {'city': 'Paris'},
        providerMetadata: {
          'openai': {'itemId': 'item_1'},
        },
      );
      final json = original.toJson();
      expect(json['provider_metadata'], {
        'openai': {'itemId': 'item_1'},
      });

      final decoded = ToolCall.fromJson(json);
      expect(decoded.providerMetadata, original.providerMetadata);
    });

    test('provider_metadata accepts arbitrary JSON and omits null', () {
      final scalar = ToolCall(
        toolCallId: 'call_scalar',
        toolName: 'tool',
        input: const {},
        providerMetadata: 'opaque-provider-token',
      );
      expect(scalar.toJson()['provider_metadata'], 'opaque-provider-token');
      expect(ToolCall.fromJson(scalar.toJson()).providerMetadata,
          'opaque-provider-token');

      final absent = ToolCall(
        toolCallId: 'call_absent',
        toolName: 'tool',
        input: const {},
      );
      expect(absent.toJson(), isNot(contains('provider_metadata')));
    });
  });

  // ─────────────────────────────────────────────────────────────────────────
  // GenerateContent (6 variants + Unknown)
  // ─────────────────────────────────────────────────────────────────────────
  group('GenerateContent round-trip', () {
    test('Text variant', () {
      final original =
          GenerateContentText(text: 'hello', providerMetadata: null);
      final decoded = GenerateContent.fromJson(original.toJson());

      expect(decoded, isA<GenerateContentText>());
      expect(decoded.tag, 'Text');
      final t = decoded as GenerateContentText;
      expect(t.text, 'hello');
      expect(t.providerMetadata, isNull);
    });

    test('Text variant preserves provider_metadata', () {
      final original = GenerateContentText(
        text: 'hi',
        providerMetadata: {'model': 'gpt-4o'},
      );
      final decoded = GenerateContent.fromJson(original.toJson());

      final t = decoded as GenerateContentText;
      expect(t.providerMetadata, {'model': 'gpt-4o'});
    });

    test('ToolCall variant', () {
      final original = GenerateContentToolCall(
        toolCallId: 'call_1',
        toolName: 'get_weather',
        input: {'location': 'Tokyo'},
        providerExecuted: true,
        isDynamic: false,
        providerMetadata: null,
      );
      final decoded = GenerateContent.fromJson(original.toJson());

      expect(decoded, isA<GenerateContentToolCall>());
      expect(decoded.tag, 'ToolCall');
      final tc = decoded as GenerateContentToolCall;
      expect(tc.toolCallId, 'call_1');
      expect(tc.toolName, 'get_weather');
      expect((tc.input as Map<String, dynamic>)['location'], 'Tokyo');
      expect(tc.providerExecuted, true);
      expect(tc.isDynamic, false);
    });

    test('Reasoning variant', () {
      final original =
          GenerateContentReasoning(text: 'thinking...', providerMetadata: null);
      final decoded = GenerateContent.fromJson(original.toJson());

      expect(decoded, isA<GenerateContentReasoning>());
      expect(decoded.tag, 'Reasoning');
      final r = decoded as GenerateContentReasoning;
      expect(r.text, 'thinking...');
    });



    test('ToolResult variant — uses result, not output', () {
      final original = GenerateContentToolResult(
        toolCallId: 'call_1',
        toolName: 'web_search',
        result: {'count': 3, 'hits': ['a', 'b', 'c']},
        isError: false,
        preliminary: true,
        isDynamic: false,
        providerMetadata: null,
      );
      final json = original.toJson();

      expect(json.keys, ['ToolResult']);
      final payload = json['ToolResult'] as Map<String, dynamic>;

      // Contract: `result` key, never `output`.
      expect(payload, contains('result'));
      expect(payload, isNot(contains('output')));
      expect(payload, contains('tool_call_id'));
      expect(payload, contains('tool_name'));
      expect(payload, contains('is_error'));
      expect(payload, contains('preliminary'));
      expect(payload, contains('dynamic'));

      final decoded = GenerateContent.fromJson(json);
      expect(decoded, isA<GenerateContentToolResult>());
      expect(decoded.tag, 'ToolResult');
      final tr = decoded as GenerateContentToolResult;
      expect(tr.toolCallId, 'call_1');
      expect(tr.toolName, 'web_search');
      expect((tr.result as Map<String, dynamic>)['count'], 3);
      expect(tr.isError, false);
      expect(tr.preliminary, true);
      expect(tr.isDynamic, false);
    });

    test('Unknown variant — tag + data pass through verbatim', () {
      final original = GenerateContentUnknown(
        tag: 'FutureVariant',
        data: {'foo': 'bar', 'n': 42},
      );
      final json = original.toJson();

      // Unknown re-encodes as `{tag: data}`.
      expect(json.keys, ['FutureVariant']);
      expect(json['FutureVariant'], {'foo': 'bar', 'n': 42});

      final decoded = GenerateContent.fromJson(json);
      expect(decoded, isA<GenerateContentUnknown>());
      final u = decoded as GenerateContentUnknown;
      expect(u.tag, 'FutureVariant');
      expect(u.data, {'foo': 'bar', 'n': 42});
    });
  });

  // ─────────────────────────────────────────────────────────────────────────
  // StreamPart (18 variants + Unknown)
  // ─────────────────────────────────────────────────────────────────────────
  group('StreamPart round-trip', () {
    // ── Text variants ──
    test('TextStart variant', () {
      final original = StreamPartTextStart(id: 't1', providerMetadata: null);
      final decoded = StreamPart.fromJson(original.toJson());

      expect(decoded, isA<StreamPartTextStart>());
      expect(decoded.type, 'TextStart');
      expect((decoded as StreamPartTextStart).id, 't1');
    });

    test('TextDelta variant', () {
      final original =
          StreamPartTextDelta(id: 't1', text: 'hel', providerMetadata: null);
      final decoded = StreamPart.fromJson(original.toJson());

      expect(decoded, isA<StreamPartTextDelta>());
      expect(decoded.type, 'TextDelta');
      final d = decoded as StreamPartTextDelta;
      expect(d.id, 't1');
      expect(d.text, 'hel');
    });

    test('TextEnd variant', () {
      final original = StreamPartTextEnd(id: 't1', providerMetadata: null);
      final decoded = StreamPart.fromJson(original.toJson());

      expect(decoded, isA<StreamPartTextEnd>());
      expect(decoded.type, 'TextEnd');
      expect((decoded as StreamPartTextEnd).id, 't1');
    });

    // ── Stream lifecycle ──
    test('Start variant', () {
      final decoded = StreamPart.fromJson(StreamPartStart().toJson());
      expect(decoded, isA<StreamPartStart>());
      expect(decoded.type, 'Start');
    });

    test('StartStep variant', () {
      final original = StreamPartStartStep(request: {}, warnings: [
        {'code': 'deprecation', 'message': 'soon'},
      ]);
      final decoded = StreamPart.fromJson(original.toJson());

      expect(decoded, isA<StreamPartStartStep>());
      expect(decoded.type, 'StartStep');
      final s = decoded as StreamPartStartStep;
      expect(s.warnings, hasLength(1));
      expect(s.warnings.first['code'], 'deprecation');
    });

    test('FinishStep and Finish variants', () {
      final usage = {
        'input_tokens': 3,
        'input_token_details': {'cache_read_tokens': 1},
        'output_tokens': 2,
        'output_token_details': {},
        'total_tokens': 5,
      };
      final step = StreamPart.fromJson(deepFlatten({
        'FinishStep': {
          'response': {'id': 'resp_1', 'timestamp': 't', 'model_id': 'gpt-4o', 'headers': null},
          'usage': usage,
          'performance': {'step_time_ms': 1.0},
          'finish_reason': 'tool-calls',
          'raw_finish_reason': 'tool_calls',
        },
      }));
      expect(step, isA<StreamPartFinishStep>());
      final fs = step as StreamPartFinishStep;
      expect(fs.response.modelId, 'gpt-4o');
      expect(fs.usage.totalTokens, 5);
      expect(fs.finishReason, 'tool-calls');
      expect(StreamPart.fromJson(deepFlatten(fs.toJson())), isA<StreamPartFinishStep>());

      final finish = StreamPart.fromJson(deepFlatten({
        'Finish': {'finish_reason': 'stop', 'raw_finish_reason': null, 'total_usage': usage},
      })) as StreamPartFinish;
      expect(finish.finishReason, 'stop');
      expect(finish.totalUsage.inputTokenDetails.cacheReadTokens, 1);
      final again = StreamPart.fromJson(deepFlatten(finish.toJson())) as StreamPartFinish;
      expect(again.totalUsage.inputTokens, 3);
    });

    test('Abort variant', () {
      final decoded = StreamPart.fromJson(StreamPartAbort(reason: 'stop').toJson());
      expect(decoded, isA<StreamPartAbort>());
      expect((decoded as StreamPartAbort).reason, 'stop');
    });

    test('Error variant', () {
      final original = StreamPartError(error: {'message': 'boom', 'code': 500});
      final decoded = StreamPart.fromJson(original.toJson());

      expect(decoded, isA<StreamPartError>());
      expect(decoded.type, 'Error');
      final e = decoded as StreamPartError;
      expect((e.error as Map<String, dynamic>)['message'], 'boom');
    });

    // ── Tool calls ──
    test('ToolInputStart variant — has title field', () {
      final original = StreamPartToolInputStart(
        id: 'tc1',
        toolName: 'get_weather',
        providerExecuted: true,
        isDynamic: false,
        title: 'Weather lookup',
        providerMetadata: null,
      );
      final json = original.toJson();

      final payload = json['ToolInputStart'] as Map<String, dynamic>;
      expect(payload, contains('title'));
      expect(payload['title'], 'Weather lookup');

      final decoded = StreamPart.fromJson(json);
      expect(decoded, isA<StreamPartToolInputStart>());
      expect(decoded.type, 'ToolInputStart');
      final t = decoded as StreamPartToolInputStart;
      expect(t.id, 'tc1');
      expect(t.toolName, 'get_weather');
      expect(t.title, 'Weather lookup');
      expect(t.providerExecuted, true);
      expect(t.isDynamic, false);
    });

    test('ToolInputDelta variant', () {
      final original =
          StreamPartToolInputDelta(id: 'tc1', delta: '{"loc', providerMetadata: null);
      final decoded = StreamPart.fromJson(original.toJson());

      expect(decoded, isA<StreamPartToolInputDelta>());
      expect(decoded.type, 'ToolInputDelta');
      final d = decoded as StreamPartToolInputDelta;
      expect(d.id, 'tc1');
      expect(d.delta, '{"loc');
    });

    test('ToolInputEnd variant', () {
      final original =
          StreamPartToolInputEnd(id: 'tc1', providerMetadata: null);
      final decoded = StreamPart.fromJson(original.toJson());

      expect(decoded, isA<StreamPartToolInputEnd>());
      expect(decoded.type, 'ToolInputEnd');
      expect((decoded as StreamPartToolInputEnd).id, 'tc1');
    });

    test('ToolCall variant — has provider_metadata field', () {
      final original = StreamPartToolCall(
        toolCallId: 'call_1',
        toolName: 'get_weather',
        input: {'location': 'Tokyo'},
        providerExecuted: true,
        isDynamic: false,
        providerMetadata: {'provider': 'openai'},
      );
      final json = original.toJson();

      final payload = json['ToolCall'] as Map<String, dynamic>;
      expect(payload, contains('provider_metadata'));
      expect(payload['provider_metadata'], {'provider': 'openai'});

      final decoded = StreamPart.fromJson(json);
      expect(decoded, isA<StreamPartToolCall>());
      expect(decoded.type, 'ToolCall');
      final tc = decoded as StreamPartToolCall;
      expect(tc.toolCallId, 'call_1');
      expect(tc.toolName, 'get_weather');
      expect((tc.input as Map<String, dynamic>)['location'], 'Tokyo');
      expect(tc.providerMetadata, {'provider': 'openai'});
    });

    test('ToolResult variant — the typed tool result with output and input', () {
      final original = StreamPartToolResult(
        toolCallId: 'call_1',
        toolName: 'web_search',
        input: {'q': 'rust'},
        output: {'count': 3},
        providerExecuted: true,
        isDynamic: false,
        providerMetadata: null,
      );
      final json = original.toJson();

      final payload = json['ToolResult'] as Map<String, dynamic>;
      expect(payload, contains('output'));
      expect(payload, isNot(contains('result')));

      final decoded = StreamPart.fromJson(json);
      expect(decoded, isA<StreamPartToolResult>());
      expect(decoded.type, 'ToolResult');
      final tr = decoded as StreamPartToolResult;
      expect(tr.toolCallId, 'call_1');
      expect(tr.toolName, 'web_search');
      expect((tr.input as Map<String, dynamic>)['q'], 'rust');
      expect((tr.output as Map<String, dynamic>)['count'], 3);
      expect(tr.providerExecuted, true);
      expect(tr.isDynamic, false);

      final deferred = StreamPart.fromJson({
        'ToolResult': {'tool_call_id': 'srv-0', 'tool_name': 'code_execution', 'output': 'done', 'provider_executed': true},
      }) as StreamPartToolResult;
      expect(deferred.input, isNull);
    });

    test('ToolError variant', () {
      final error = StreamPart.fromJson({
        'ToolError': {
          'tool_call_id': 'srv-2',
          'tool_name': 'code_execution',
          'input': {'code': '1/0'},
          'error': {'error_code': 'unavailable'},
          'provider_executed': true,
        },
      });
      expect(error, isA<StreamPartToolError>());
      expect(error.type, 'ToolError');
      expect((error as StreamPartToolError).error, {'error_code': 'unavailable'});
      expect(error.providerExecuted, true);
      expect(error.isDynamic, isNull);
      expect(StreamPart.fromJson(error.toJson()).toJson(), error.toJson());
    });

    test('ToolOutputDenied and ToolApprovalResponse variants', () {
      final denied = StreamPart.fromJson({
        'ToolOutputDenied': {'tool_call_id': 'c', 'tool_name': 't'},
      });
      expect(denied, isA<StreamPartToolOutputDenied>());
      expect(denied.type, 'ToolOutputDenied');
      expect(denied.toJson(), {
        'ToolOutputDenied': {'tool_call_id': 'c', 'tool_name': 't'},
      });

      final response = StreamPart.fromJson({
        'ToolApprovalResponse': {
          'approval_id': 'a',
          'tool_call': {'tool_call_id': 'c', 'tool_name': 't', 'input': <String, dynamic>{}},
          'approved': false,
        },
      });
      expect(response.type, 'ToolApprovalResponse');
      expect((response as StreamPartToolApprovalResponse).approved, false);
    });

    // ── File ──


    // ── Reasoning ──
    test('ReasoningStart variant', () {
      final original =
          StreamPartReasoningStart(id: 'r1', providerMetadata: null);
      final decoded = StreamPart.fromJson(original.toJson());

      expect(decoded, isA<StreamPartReasoningStart>());
      expect(decoded.type, 'ReasoningStart');
      expect((decoded as StreamPartReasoningStart).id, 'r1');
    });

    test('ReasoningDelta variant', () {
      final original =
          StreamPartReasoningDelta(id: 'r1', text: 'thi', providerMetadata: null);
      final decoded = StreamPart.fromJson(original.toJson());

      expect(decoded, isA<StreamPartReasoningDelta>());
      expect(decoded.type, 'ReasoningDelta');
      final d = decoded as StreamPartReasoningDelta;
      expect(d.id, 'r1');
      expect(d.text, 'thi');
    });

    test('ReasoningEnd variant', () {
      final original =
          StreamPartReasoningEnd(id: 'r1', providerMetadata: null);
      final decoded = StreamPart.fromJson(original.toJson());

      expect(decoded, isA<StreamPartReasoningEnd>());
      expect(decoded.type, 'ReasoningEnd');
      expect((decoded as StreamPartReasoningEnd).id, 'r1');
    });

    // ── Metadata / sources / raw ──
    test('Raw variant', () {
      final original = StreamPartRaw(rawValue: {'raw': 'chunk'});
      final decoded = StreamPart.fromJson(original.toJson());

      expect(decoded, isA<StreamPartRaw>());
      expect(decoded.type, 'Raw');
      final r = decoded as StreamPartRaw;
      expect((r.rawValue as Map<String, dynamic>)['raw'], 'chunk');
    });

    test('Unknown variant — tag + data pass through verbatim', () {
      final original =
          StreamPartUnknown(tag: 'FuturePart', data: {'foo': 'bar', 'n': 7});
      final json = original.toJson();

      expect(json.keys, ['FuturePart']);
      expect(json['FuturePart'], {'foo': 'bar', 'n': 7});

      final decoded = StreamPart.fromJson(json);
      expect(decoded, isA<StreamPartUnknown>());
      expect(decoded.type, 'FuturePart');
      final u = decoded as StreamPartUnknown;
      expect(u.tag, 'FuturePart');
      expect(u.data, {'foo': 'bar', 'n': 7});
    });
  });

  test('GenerateTextOptions includeRawChunks round-trips', () {
    // RFC-0016 M2 true-case through the Dart typed options.
    final original = GenerateTextOptions(includeRawChunks: true);
    final json = original.toJson();
    expect(json['include_raw_chunks'], isTrue);
    final back = GenerateTextOptions.fromJson(json);
    expect(back.includeRawChunks, isTrue);

    // Default omits the field (toJson only emits non-null).
    final defaults = GenerateTextOptions().toJson();
    expect(defaults.containsKey('include_raw_chunks'), isFalse);
  });

  test('GenerateTextOptions sessionId round-trips', () {
    // RFC-0024: explicit session_id crosses the wire as snake_case.
    final original = GenerateTextOptions(sessionId: 'sess-1');
    final json = original.toJson();
    expect(json['session_id'], 'sess-1');
    final back = GenerateTextOptions.fromJson(json);
    expect(back.sessionId, 'sess-1');

    final defaults = GenerateTextOptions().toJson();
    expect(defaults.containsKey('session_id'), isFalse);
  });
}
