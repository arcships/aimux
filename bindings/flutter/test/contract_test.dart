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

        // A fixture type with no case here fails rather than being skipped:
        // silent skipping is how a net grows holes.
        switch (type) {
          case 'ToolChoice':
            ToolChoice.fromJson(wire);
          case 'StreamPart':
            StreamPart.fromJson(wire as Map<String, dynamic>);
          case 'GenerateContent':
            GenerateContent.fromJson(wire as Map<String, dynamic>);
          case 'GenerateTextOptions':
            GenerateTextOptions.fromJson(wire as Map<String, dynamic>);
          case 'TimeoutConfiguration':
            TimeoutConfiguration.fromJson(wire as Map<String, dynamic>);
          case 'ModelMessage':
            ModelMessage.fromJson(wire as Map<String, dynamic>);
          case 'Role':
            Role.fromJson(wire as String);
          case 'FinishReasonUnified':
            FinishReasonUnified.fromJson(wire as String);
          case 'ReasoningEffort':
            ReasoningEffort.fromJson(wire as String);
          default:
            fail(
              "fixture '$name' declares type '$type', which has no case in "
              'contract_test.dart — wire it up so the fixture is actually '
              'checked against Dart',
            );
        }
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
  });
}
