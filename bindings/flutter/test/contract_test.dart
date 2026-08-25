// Contract tests — validate the Dart binding against the shared wire-format
// fixtures in `contract-tests/fixtures/wire-format.json`, the same file the
// Rust, Go, Java, Kotlin, Python, Node and Swift contract tests read.
//
// Rust is the wire authority: it produces the JSON and the bindings consume it,
// so the direction that matters is DECODING — every fixture must decode into
// the Dart type it claims to describe. Encoding is checked as a Dart round-trip
// (decode → encode → decode) rather than by byte-comparing against the fixture,
// because Dart omits null fields that Rust emits explicitly; a byte comparison
// would fail on that alone and say nothing about type correctness.
//
// Pure serialization — no native library is loaded.

import 'dart:convert';
import 'dart:io';

import 'package:test/test.dart';
import 'package:aimux/types.dart';

/// The fixture lives at the repo root; the test's working directory depends on
/// how the suite is invoked, so try the usual spots (mirrors the Java runner).
File _fixtureFile() {
  const candidates = [
    '../../contract-tests/fixtures/wire-format.json', // cwd = bindings/flutter
    'contract-tests/fixtures/wire-format.json', // cwd = repo root
    '../contract-tests/fixtures/wire-format.json',
  ];
  for (final path in candidates) {
    final file = File(path);
    if (file.existsSync()) return file;
  }
  throw StateError('cannot find wire-format.json; tried $candidates');
}

List<Map<String, dynamic>> _loadFixtures() =>
    (jsonDecode(_fixtureFile().readAsStringSync()) as List<dynamic>)
        .cast<Map<String, dynamic>>();

void main() {
  group('shared wire-format fixtures', () {
    test('every fixture decodes into its Dart type', () {
      final fixtures = _loadFixtures();
      expect(fixtures, isNotEmpty, reason: 'no fixtures loaded');

      for (final fixture in fixtures) {
        final name = fixture['name'] as String;
        final type = fixture['type'] as String;
        final wire = jsonDecode(fixture['json'] as String);

        // Encoding a decoded fixture and decoding that again must produce the
        // same encoding. Comparing encodings rather than objects is deliberate:
        // these generated classes do not implement value equality, so `==`
        // would compare identities and pass no matter what. The generated
        // toJson() is shallow — nested models stay live objects — so each
        // encoding is normalized through jsonEncode/jsonDecode, which invokes
        // nested toJson() recursively.
        Object? norm(Object? o) => jsonDecode(jsonEncode(o));
        final Object? first;
        final Object? second;

        // A fixture type with no case here fails rather than being skipped:
        // silent skipping is how a net grows holes.
        switch (type) {
          case 'ToolChoice':
            first = norm(ToolChoice.fromJson(wire).toJson());
            second = norm(ToolChoice.fromJson(first).toJson());
          case 'StreamPart':
            first = norm(StreamPart.fromJson(wire as Map<String, dynamic>).toJson());
            second = norm(StreamPart.fromJson(first as Map<String, dynamic>).toJson());
          case 'GenerateContent':
            first = norm(GenerateContent.fromJson(wire as Map<String, dynamic>).toJson());
            second = norm(GenerateContent.fromJson(first as Map<String, dynamic>)
                    .toJson());
          case 'GenerateTextOptions':
            first = norm(GenerateTextOptions.fromJson(wire as Map<String, dynamic>)
                .toJson());
            second = norm(GenerateTextOptions.fromJson(first as Map<String, dynamic>)
                    .toJson());
          case 'TimeoutConfiguration':
            first = norm(TimeoutConfiguration.fromJson(wire as Map<String, dynamic>)
                .toJson());
            second = norm(TimeoutConfiguration.fromJson(first as Map<String, dynamic>)
                    .toJson());
          case 'ModelMessage':
            first = norm(ModelMessage.fromJson(wire as Map<String, dynamic>).toJson());
            second = norm(ModelMessage.fromJson(first as Map<String, dynamic>).toJson());
          case 'Role':
            first = norm(Role.fromJson(wire as String).toJson());
            second = norm(Role.fromJson(first as String).toJson());
          case 'FinishReasonUnified':
            first = norm(FinishReasonUnified.fromJson(wire as String).toJson());
            second = norm(FinishReasonUnified.fromJson(first as String).toJson());
          case 'ReasoningEffort':
            first = norm(ReasoningEffort.fromJson(wire as String).toJson());
            second = norm(ReasoningEffort.fromJson(first as String).toJson());
          default:
            fail(
              "fixture '$name' declares type '$type', which has no case in "
              'contract_test.dart — wire it up so the fixture is actually '
              'checked against Dart',
            );
        }

        expect(
          second,
          equals(first),
          reason:
              "fixture '$name' does not survive a Dart encode/decode round-trip",
        );
      }
    });

    test('numeric fields keep their declared precision', () {
      // `top_k` is fractional on purpose: Rust is Option<f64> (matching the
      // upstream AI SDK's `topK?: number`), so a binding that declares it an
      // integer cannot hold this value. Kotlin and Java both declared it Long
      // until this fixture was added.
      final fixture = _loadFixtures().firstWhere(
        (f) => f['name'] == 'generate_text_options_numeric_types',
        orElse: () => throw StateError(
          "fixture 'generate_text_options_numeric_types' is missing",
        ),
      );
      final opts = GenerateTextOptions.fromJson(
        jsonDecode(fixture['json'] as String) as Map<String, dynamic>,
      );

      expect(opts.topK, 40.5,
          reason: 'top_k must keep its fraction — an int would truncate to 40');
      expect(opts.frequencyPenalty, -0.5, reason: 'penalties must stay signed');
      expect(opts.temperature, 0.7);
      expect(opts.topP, 0.95);
      expect(opts.maxOutputTokens, 256);
      expect(opts.seed, 42);
      expect(opts.maxRetries, 3);
    });

    test('provider-executed transcript uses typed content parts', () {
      final fixture = _loadFixtures().firstWhere(
        (f) => f['name'] == 'model_message_provider_executed_tool_transcript',
      );
      final message = jsonDecode(fixture['json'] as String) as Map<String, dynamic>;
      final content = (message['content'] as List<dynamic>)
          .cast<Map<String, dynamic>>();
      final call = ContentPart.fromJson(content[0]) as ContentPartToolCall;
      final result = ContentPart.fromJson(content[1]) as ContentPartToolResult;

      expect(call.providerExecuted, true);
      expect(call.toolName, 'search');
      expect(result.toolName, 'search');
      expect(result.isError, false);
      expect(result.preliminary, true);
      expect(result.isDynamic, true);
    });
  });
}
