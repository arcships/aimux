// Tests for aimux Flutter/Dart binding.
//
// These tests do NOT make real API calls — they verify the API surface
// and error handling. They require the native library (libaimux_ffi.so)
// to be on the library path.
//
// Run: LD_LIBRARY_PATH=../../target/release dart test

import 'dart:ffi';
import 'dart:io';

import 'package:aimux/aimux.dart';
import 'package:aimux/errors.dart'
    show aimuxDropHandle, aimuxFreeString, expectAimuxError, openAimuxLibrary, withUtf8;
import 'package:aimux/multimodal.dart' show EmbeddingModel;
import 'package:ffi/ffi.dart';
import 'package:test/test.dart';

// Raw C signatures (aimux-ffi.h / aimux-error.h) for C ABI tests.
typedef _OpenaiNewC = Pointer<Void> Function(
    Pointer<Utf8>, Pointer<Utf8>, Pointer<Uint64>);
typedef _GenerateTextC = Pointer<Void> Function(
    Uint64, Pointer<Utf8>, Pointer<Utf8>, Pointer<Pointer<Utf8>>);
typedef _GenerateTextDart = Pointer<Void> Function(
    int, Pointer<Utf8>, Pointer<Utf8>, Pointer<Pointer<Utf8>>);
typedef _IntOfPtrC = Int32 Function(Pointer<Void>);
typedef _IntOfPtrDart = int Function(Pointer<Void>);
typedef _StrOfPtr = Pointer<Utf8> Function(Pointer<Void>);

void main() {
  // Error transport (aimux-error.h): a failed call returns an opaque
  // `aimux_error_t *`; the decoder (expectAimuxError / expectRecordingError)
  // reads the unified code, copies the fields, frees the returned error once,
  // and throws. Codes 200..206 become StateError('aimux ffi: …').
  group('aimux_error_t → AimuxException / native errors', () {
    final lib = openAimuxLibrary();
    final errorCode = lib.lookupFunction<_IntOfPtrC, _IntOfPtrDart>(
        'aimux_error_code');
    final ffiMessage =
        lib.lookupFunction<_StrOfPtr, _StrOfPtr>('aimux_error_message');
    final openaiNew =
        lib.lookupFunction<_OpenaiNewC, _OpenaiNewC>('aimux_openai_new');
    final generateText = lib.lookupFunction<_GenerateTextC, _GenerateTextDart>(
        'aimux_generate_text');

    test('unknown provider carries the requested provider id', () {
      expect(
        () => Model.provider('definitely-not-a-provider', 'm', apiKey: 'k'),
        throwsA(isA<NoSuchProviderError>()
            .having((e) => e.providerId, 'providerId',
                'definitely-not-a-provider')
            .having((e) => e.status, 'status', -1)
            .having((e) => e.retryable, 'retryable', isFalse)),
      );
    });

    // Raw-FFI misuse a correct binding never commits: a NULL argument is a
    // C ABI failure code 200 maps to StateError.
    test('NULL argument at the raw C ABI → code 200 → StateError',
        () {
      final out = calloc<Uint64>();
      try {
        final e = openaiNew(nullptr, nullptr, out);
        expect(e, isNot(nullptr));
        expect(out.value, 0);
        expect(errorCode(e), 200);
        final m = ffiMessage(e);
        expect(m, isNot(nullptr));
        expect(m.toDartString(), contains('api_key'));
        aimuxFreeString(m);
        expect(
          () => expectAimuxError(e, 'openai_new'), // frees the owner
          throwsA(isA<StateError>()
              .having((e) => e.message, 'message', contains('aimux ffi:'))
              .having((e) => e.message, 'message', contains('api_key'))),
        );
      } finally {
        calloc.free(out);
      }
    });

    test('dead handle at the raw C ABI → StateError', () {
      final out = calloc<Pointer<Utf8>>();
      try {
        expect(
          () => withUtf8('"hi"', (p) {
            final e = generateText(0x7FFFFFFFFFFF, p, nullptr, out);
            expect(errorCode(e), 203);
            expectAimuxError(e, 'generate_text');
          }),
          throwsA(isA<StateError>()
              .having((e) => e.message, 'message', contains('model'))),
        );
        expect(out.value, nullptr);
      } finally {
        calloc.free(out);
      }
    });

    // Bad wire JSON is only reachable when checkJson pre-validation was
    // bypassed (raw C call here); it is a C ABI failure → StateError.
    test('bad wire JSON at the raw C ABI → StateError', () {
      final outHandle = calloc<Uint64>();
      final outJson = calloc<Pointer<Utf8>>();
      try {
        final e = withUtf8('sk-test-fake-key',
            (k) => withUtf8('gpt-4o-mini', (m) => openaiNew(k, m, outHandle)));
        expect(e, nullptr);
        final handle = outHandle.value;
        expect(handle, isNot(0));
        try {
          expect(
            () => withUtf8('{not json', (p) {
              final e = generateText(handle, p, nullptr, outJson);
              expect(errorCode(e), 202);
              expectAimuxError(e, 'generate_text');
            }),
            throwsA(isA<StateError>()
                .having((e) => e.message, 'message', contains('prompt_json'))),
          );
        } finally {
          aimuxDropHandle(handle);
        }
      } finally {
        calloc.free(outHandle);
        calloc.free(outJson);
      }
    });

    test('NULL error is success: expectAimuxError returns', () {
      expect(() => expectAimuxError(nullptr, 'noop'), returnsNormally);
    });
  });

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

  test('raw configJson is validated in Dart before the C call', () {
    // FormatException naming the parameter, not AiMuxError.jsonParse.
    expect(
      () => Model.provider('openai', 'gpt-4o-mini',
          apiKey: 'k', configJson: '{invalid json}'),
      throwsA(isA<FormatException>()
          .having((e) => e.source, 'source', 'config_json')),
    );
  });

  test('registerProviders validates raw config_json in Dart', () {
    expect(
      () => registerProviders('{not json'),
      throwsA(isA<FormatException>()
          .having((e) => e.source, 'source', 'config_json')),
    );
  });

  test('multimodal use-after-close throws StateError', () {
    final m = EmbeddingModel.openai('sk-test-fake-key', 'text-embedding-3-small');
    m.close();
    expect(
      () => m.embed('["a"]'),
      throwsA(isA<StateError>()
          .having((e) => e.message, 'message', contains('closed'))),
    );
  });

  test('model close is idempotent', () {
    final model = Model.openai('sk-test-fake-key', 'gpt-4o-mini');
    model.close();
    model.close(); // should not throw
  });

  test('generateText on closed model throws StateError', () {
    final model = Model.openai('sk-test-fake-key', 'gpt-4o-mini');
    model.close();
    // Use-after-close is guarded locally: native StateError, not AimuxException.
    expect(
      () => model.generateText('"hello"'),
      throwsA(isA<StateError>()
          .having((e) => e.message, 'message', contains('closed'))),
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
    expect(() => initRecordingRing(8), returnsNormally);
    recordingStop();
  });

  // Omitting cap uses the library default capacity (FFI
  // aimux_init_recording_ring_default) and must not throw.
  test('initRecordingRing with no cap uses library default', () {
    expect(() => initRecordingRing(), returnsNormally);
    recordingStop();
  });

  // aimux_recording_try_flush: NULL = data on disk (also when nothing is
  // recording); otherwise an error with the recording view → RecordingException
  // (its own type, not an AimuxException).
  group('recordingTryFlush', () {
    test('returns normally when nothing is recording', () {
      recordingStop();
      expect(recordingTryFlush, returnsNormally);
    });

    test('initRecording reports INIT for an unwritable directory', () {
      // Parent path is a regular file → the recorder cannot be constructed,
      // so init itself fails with code INIT and nothing is installed; a
      // try-flush afterwards is a no-op.
      final blocker = Directory.systemTemp.createTempSync('aimux-flutter-rec');
      final occupied = File('${blocker.path}/occupied')..writeAsStringSync('x');
      try {
        recordingStop();
        expect(
          () => initRecording('${occupied.path}/sub'),
          throwsA(allOf(
              isA<RecordingException>()
                  .having((e) => e.code, 'code', RecordingErrorCode.init),
              isNot(isA<AimuxException>()))),
        );
        expect(recordingTryFlush, returnsNormally);
      } finally {
        recordingStop();
        blocker.deleteSync(recursive: true);
      }
    });
  });

  // T9: Model/ProviderHandle register a NativeFinalizer so a forgotten close()
  // does not leak the native handle. close() detaches the finalizer first,
  // so an explicit close cannot double-free.
  test('Model close is idempotent and detaches the finalizer (T9)', () {
    final model = Model.openai('sk-test-fake-key', 'gpt-4o-mini');
    model.close();
    model.close(); // second close is a no-op (finalizer already detached)
    expect(() => model.generateText('"hi"'), throwsStateError);
  });

  test('ProviderHandle close is idempotent and detaches the finalizer (T9)', () {
    final p = createProvider('deepseek', 'sk-test-fake-key', null);
    p.close();
    p.close(); // should not throw
    expect(() => p.listModels(), throwsStateError);
  });
}
