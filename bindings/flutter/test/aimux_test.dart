// Tests for aimux Flutter/Dart binding.
//
// These tests do NOT make real API calls — they verify the API surface
// and error handling. They require the native library (libaimux_ffi.so)
// to be on the library path.
//
// Run: LD_LIBRARY_PATH=../../target/release dart test

import 'package:aimux/aimux.dart';
import 'package:test/test.dart';

void main() {
  test('Model.openai creates an instance', () {
    final model = Model.openai('sk-test-fake-key', 'gpt-4o-mini');
    expect(model, isNotNull);
    model.close();
  });

  test('Model.anthropic creates an instance', () {
    final model = Model.anthropic('sk-ant-test-fake-key', 'claude-3-5-sonnet-20241022');
    expect(model, isNotNull);
    model.close();
  });

  test('generateText rejects invalid prompt JSON', () {
    final model = Model.openai('sk-test-fake-key', 'gpt-4o-mini');
    // Engine invalid-input failures surface as AimuxException (not StateError).
    expect(
      () => model.generateText('{invalid json}'),
      throwsA(isA<AimuxException>()),
    );
    model.close();
  });

  test('model close is idempotent', () {
    final model = Model.openai('sk-test-fake-key', 'gpt-4o-mini');
    model.close();
    model.close(); // should not throw
  });

  test('generateText on closed model throws StateError', () {
    final model = Model.openai('sk-test-fake-key', 'gpt-4o-mini');
    model.close();
    // Local closed-handle errors stay StateError (not AimuxException).
    expect(
      () => model.generateText('"hello"'),
      throwsA(isA<StateError>()),
    );
  });
}
