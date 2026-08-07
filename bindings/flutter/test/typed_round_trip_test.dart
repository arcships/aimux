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
/// Mirrors the workaround used in `typed_model_test.dart`: `Usage` /
/// `TokenUsage` are `@JsonSerializable()` with the default
/// `explicitToJson: false`, so their generated `toJson()` keeps nested
/// `TokenUsage` instances instead of recursing to plain maps. The only variant
/// affected is `StreamPartFinish` (the only one nesting `@JsonSerializable`
/// types). In the real pipeline the value always crosses a JSON-string
/// boundary (Rust core → JSON → Dart), which flattens it — so we do the same
/// here before re-parsing.
Map<String, dynamic> deepFlatten(Map<String, dynamic> json) =>
    jsonDecode(jsonEncode(json)) as Map<String, dynamic>;

void main() {
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

    test('Source variant', () {
      final original = GenerateContentSource(
        id: 'src_1',
        sourceType: 'url',
        url: 'https://example.com',
        title: 'Example',
        providerMetadata: null,
      );
      final decoded = GenerateContent.fromJson(original.toJson());

      expect(decoded, isA<GenerateContentSource>());
      expect(decoded.tag, 'Source');
      final s = decoded as GenerateContentSource;
      expect(s.id, 'src_1');
      expect(s.sourceType, 'url');
      expect(s.url, 'https://example.com');
      expect(s.title, 'Example');
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

    test('File variant — no filename field', () {
      final original = GenerateContentFile(
        data: FileDataUrl(url: 'https://example.com/img.png'),
        mediaType: 'image/png',
        providerMetadata: null,
      );
      final json = original.toJson();

      // Externally-tagged shape: a single top-level key.
      expect(json.keys, ['File']);
      final payload = json['File'] as Map<String, dynamic>;

      // Contract: `data` + `media_type`, never `filename`.
      expect(payload, contains('data'));
      expect(payload, contains('media_type'));
      expect(payload, isNot(contains('filename')));

      final decoded = GenerateContent.fromJson(json);
      expect(decoded, isA<GenerateContentFile>());
      expect(decoded.tag, 'File');
      final f = decoded as GenerateContentFile;
      expect(f.data, isA<FileDataUrl>());
      expect((f.data as FileDataUrl).url, 'https://example.com/img.png');
      expect(f.mediaType, 'image/png');
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
          StreamPartTextDelta(id: 't1', delta: 'hel', providerMetadata: null);
      final decoded = StreamPart.fromJson(original.toJson());

      expect(decoded, isA<StreamPartTextDelta>());
      expect(decoded.type, 'TextDelta');
      final d = decoded as StreamPartTextDelta;
      expect(d.id, 't1');
      expect(d.delta, 'hel');
    });

    test('TextEnd variant', () {
      final original = StreamPartTextEnd(id: 't1', providerMetadata: null);
      final decoded = StreamPart.fromJson(original.toJson());

      expect(decoded, isA<StreamPartTextEnd>());
      expect(decoded.type, 'TextEnd');
      expect((decoded as StreamPartTextEnd).id, 't1');
    });

    // ── Stream lifecycle ──
    test('StreamStart variant', () {
      final original = StreamPartStreamStart(warnings: [
        {'code': 'deprecation', 'message': 'soon'},
      ]);
      final decoded = StreamPart.fromJson(original.toJson());

      expect(decoded, isA<StreamPartStreamStart>());
      expect(decoded.type, 'StreamStart');
      final s = decoded as StreamPartStreamStart;
      expect(s.warnings, hasLength(1));
      expect(s.warnings.first['code'], 'deprecation');
    });

    test('Finish variant', () {
      final original = StreamPartFinish(
        finishReason: FinishReason(unified: 'stop', raw: 'stop'),
        usage: Usage(
          inputTokens: TokenUsage(total: 10, text: 10),
          outputTokens: TokenUsage(total: 5, text: 5),
        ),
        providerMetadata: null,
      );
      // `Finish` nests `@JsonSerializable` `Usage`/`TokenUsage`, whose
      // generated `toJson()` (explicitToJson: false) does not recurse — so we
      // deep-flatten across the JSON boundary before re-parsing, exactly as the
      // real Rust→Dart pipeline does. See `typed_model_test.dart` for the same
      // note.
      final decoded =
          StreamPart.fromJson(deepFlatten(original.toJson()));

      expect(decoded, isA<StreamPartFinish>());
      expect(decoded.type, 'Finish');
      final f = decoded as StreamPartFinish;
      expect(f.finishReason.unified, 'stop');
      expect(f.finishReason.raw, 'stop');
      expect(f.usage.inputTokens.total, 10);
      expect(f.usage.outputTokens.total, 5);
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

    test('ToolResult variant — uses result, not output', () {
      final original = StreamPartToolResult(
        toolCallId: 'call_1',
        toolName: 'web_search',
        result: {'count': 3},
        isError: false,
        preliminary: true,
        isDynamic: false,
        providerMetadata: null,
      );
      final json = original.toJson();

      final payload = json['ToolResult'] as Map<String, dynamic>;
      expect(payload, contains('result'));
      expect(payload, isNot(contains('output')));

      final decoded = StreamPart.fromJson(json);
      expect(decoded, isA<StreamPartToolResult>());
      expect(decoded.type, 'ToolResult');
      final tr = decoded as StreamPartToolResult;
      expect(tr.toolCallId, 'call_1');
      expect(tr.toolName, 'web_search');
      expect((tr.result as Map<String, dynamic>)['count'], 3);
      expect(tr.isError, false);
      expect(tr.preliminary, true);
      expect(tr.isDynamic, false);
    });

    // ── File ──
    test('File variant — no filename field', () {
      final original = StreamPartFile(
        data: FileDataUrl(url: 'https://example.com/img.png'),
        mediaType: 'image/png',
        providerMetadata: null,
      );
      final json = original.toJson();

      expect(json.keys, ['File']);
      final payload = json['File'] as Map<String, dynamic>;
      expect(payload, contains('data'));
      expect(payload, contains('media_type'));
      expect(payload, isNot(contains('filename')));

      final decoded = StreamPart.fromJson(json);
      expect(decoded, isA<StreamPartFile>());
      expect(decoded.type, 'File');
      final f = decoded as StreamPartFile;
      expect(f.data, isA<FileDataUrl>());
      expect((f.data as FileDataUrl).url, 'https://example.com/img.png');
      expect(f.mediaType, 'image/png');
    });

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
          StreamPartReasoningDelta(id: 'r1', delta: 'thi', providerMetadata: null);
      final decoded = StreamPart.fromJson(original.toJson());

      expect(decoded, isA<StreamPartReasoningDelta>());
      expect(decoded.type, 'ReasoningDelta');
      final d = decoded as StreamPartReasoningDelta;
      expect(d.id, 'r1');
      expect(d.delta, 'thi');
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
    test('ResponseMetadata variant', () {
      final original = StreamPartResponseMetadata(
        id: 'resp_1',
        timestamp: '2024-01-01T00:00:00Z',
        modelId: 'gpt-4o',
      );
      final decoded = StreamPart.fromJson(original.toJson());

      expect(decoded, isA<StreamPartResponseMetadata>());
      expect(decoded.type, 'ResponseMetadata');
      final m = decoded as StreamPartResponseMetadata;
      expect(m.id, 'resp_1');
      expect(m.timestamp, '2024-01-01T00:00:00Z');
      expect(m.modelId, 'gpt-4o');
    });

    test('Source variant', () {
      final original = StreamPartSource(
        id: 'src_1',
        sourceType: 'url',
        url: 'https://example.com',
        title: 'Example',
        providerMetadata: null,
      );
      final decoded = StreamPart.fromJson(original.toJson());

      expect(decoded, isA<StreamPartSource>());
      expect(decoded.type, 'Source');
      final s = decoded as StreamPartSource;
      expect(s.id, 'src_1');
      expect(s.sourceType, 'url');
      expect(s.url, 'https://example.com');
      expect(s.title, 'Example');
    });

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
