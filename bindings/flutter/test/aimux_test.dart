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

  // T11/T12: initRecordingRing must validate cap up front. A negative Dart int
  // would otherwise be reinterpreted as a huge u64 by the FFI; cap == 0 is
  // rejected by the C ABI. Both are caught in Dart with ArgumentError,
  // matching Kotlin/Java.
  test('initRecordingRing rejects cap <= 0 (T11/T12)', () {
    expect(() => initRecordingRing(0), throwsArgumentError);
    expect(() => initRecordingRing(-1), throwsArgumentError);
    expect(() => initRecordingRing(-9999), throwsArgumentError);
  });

  test('initRecordingRing accepts positive cap', () {
    expect(initRecordingRing(8), 0);
    recordingStop();
  });

  // Omitting cap uses the library default capacity (FFI
  // aimux_init_recording_ring_default) and must not throw.
  test('initRecordingRing with no cap uses library default', () {
    expect(initRecordingRing(), 0);
    recordingStop();
  });

  // T9: Model/ProviderHandle register a NativeFinalizer so a forgotten close()
  // does not leak the native handle. close() detaches the finalizer first,
  // so an explicit close cannot double-free.
  test('Model close is idempotent and detaches the finalizer (T9)', () {
    final model = Model.openai('sk-test-fake-key', 'gpt-4o-mini');
    model.close();
    model.close(); // second close is a no-op (finalizer already detached)
    expect(() => model.generateText('"hi"'), throwsA(isA<StateError>()));
  });

  test('ProviderHandle close is idempotent and detaches the finalizer (T9)', () {
    final p = createProvider('deepseek', 'sk-test-fake-key', null);
    p.close();
    p.close(); // should not throw
    expect(() => p.listModels(), throwsA(isA<StateError>()));
  });
}
