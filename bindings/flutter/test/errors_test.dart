// Pure-Dart tests for AimuxException hierarchy + AimuxCError mapping.
// Does not require the native library.

import 'dart:ffi';

import 'package:aimux/errors.dart';
import 'package:ffi/ffi.dart';
import 'package:test/test.dart';

/// Build an [AimuxException] from a manually filled [AimuxCError].
///
/// Allocates the struct (and message, via [fill] using `toNativeUtf8`) with
/// calloc and frees it with calloc — deliberately NOT via [withAimuxCError],
/// whose cleanup path calls the native `aimux_free_string` (absent in
/// pure-Dart CI, and the wrong allocator for test-allocated strings).
AimuxException fromFilled(void Function(Pointer<AimuxCError> err) fill) {
  final err = calloc<AimuxCError>();
  clearAimuxCError(err);
  fill(err);
  try {
    return AimuxException.fromC(err.ref);
  } finally {
    if (err.ref.message != nullptr) calloc.free(err.ref.message);
    if (err.ref.errorValue != nullptr) calloc.free(err.ref.errorValue);
    calloc.free(err);
  }
}

void main() {
  group('AimuxException.fromCode', () {
    test('dispatches RateLimitedError with default status 429', () {
      final e = AimuxException.fromCode(
        AimuxErrorCode.rateLimited,
        'slow down',
        retryMs: 1500,
      );
      expect(e, isA<RateLimitedError>());
      expect(e.message, 'slow down');
      expect(e.code, AimuxErrorCode.rateLimited);
      expect(e.status, 429);
      expect(e.retryMs, 1500);
      expect(e.toString(), contains('RateLimitedError'));
    });

    test('dispatches AuthenticationError with default status 401', () {
      final e = AimuxException.fromCode(AimuxErrorCode.auth, 'bad key');
      expect(e, isA<AuthenticationError>());
      expect(e.status, 401);
      expect(e.codeName, 'Auth');
    });

    test('dispatches all known subclasses', () {
      final cases = <int, Type>{
        AimuxErrorCode.unknown: UnknownError,
        AimuxErrorCode.provider: ProviderError,
        AimuxErrorCode.http: HttpError,
        AimuxErrorCode.json: JsonError,
        AimuxErrorCode.stream: StreamError,
        AimuxErrorCode.tool: ToolError,
        AimuxErrorCode.invalidArgument: InvalidArgumentError,
        AimuxErrorCode.invalidPrompt: InvalidPromptError,
        AimuxErrorCode.rateLimited: RateLimitedError,
        AimuxErrorCode.auth: AuthenticationError,
        AimuxErrorCode.tokenExpired: TokenExpiredError,
        AimuxErrorCode.modelNotFound: ModelNotFoundError,
        AimuxErrorCode.unsupported: AimuxUnsupportedError,
        AimuxErrorCode.noSuchModel: NoSuchModelError,
        AimuxErrorCode.unknownProvider: UnknownProviderError,
        AimuxErrorCode.apiCall: APICallError,
        AimuxErrorCode.timeout: AimuxTimeoutError,
        AimuxErrorCode.aborted: RequestAbortedError,
        AimuxErrorCode.other: OtherError,
      };
      for (final entry in cases.entries) {
        final e = AimuxException.fromCode(entry.key, 'msg');
        expect(e.runtimeType, entry.value, reason: 'code ${entry.key}');
        expect(e, isA<AimuxException>());
      }
    });

    test('unknown numeric code stays AimuxException base', () {
      final e = AimuxException.fromCode(999, 'future');
      expect(e.runtimeType, AimuxException);
      expect(e.code, 999);
      expect(e.codeName, 'Code(999)');
    });
  });

  group('AimuxException.fromC', () {
    test('maps filled AimuxCError to subclass', () {
      final e = fromFilled((err) {
        err.ref.code = AimuxErrorCode.timeout;
        err.ref.message = 'took too long'.toNativeUtf8();
      });
      expect(e, isA<AimuxTimeoutError>());
      expect(e.message, 'took too long');
      expect(e.code, AimuxErrorCode.timeout);
      expect(e.errorValue, isNull);
    });

    test('carries errorValue JSON when present', () {
      const json = '{"Timeout":{"message":"took too long"}}';
      final e = fromFilled((err) {
        err.ref.code = AimuxErrorCode.timeout;
        err.ref.message = 'took too long'.toNativeUtf8();
        err.ref.errorValue = json.toNativeUtf8();
      });
      expect(e, isA<AimuxTimeoutError>());
      expect(e.errorValue, json);
    });

    test('OK code becomes UnknownError with fallback message', () {
      // cleared = OK / null message
      final e = fromFilled((_) {});
      expect(e, isA<UnknownError>());
      expect(e.message, contains('operation failed'));
    });

    test('null message uses code name fallback', () {
      final e = fromFilled((err) {
        err.ref.code = AimuxErrorCode.aborted;
      });
      expect(e, isA<RequestAbortedError>());
      expect(e.message, contains('Aborted'));
    });
  });

  group('AimuxErrorCode.name', () {
    test('covers append-only table', () {
      expect(AimuxErrorCode.name(AimuxErrorCode.ok), 'OK');
      expect(AimuxErrorCode.name(AimuxErrorCode.rateLimited), 'RateLimited');
      expect(AimuxErrorCode.name(AimuxErrorCode.auth), 'Auth');
      expect(AimuxErrorCode.name(AimuxErrorCode.other), 'Other');
    });
  });

  test('AimuxCError matches the 40-byte C layout', () {
    expect(sizeOf<AimuxCError>(), 40);
  });

  test('withAimuxCError clears, runs, and frees', () {
    // Smoke: allocate, read, free without leak/crash. No message is set, so
    // the native aimux_free_string is never touched (pure-Dart safe).
    final result = withAimuxCError((err) {
      expect(err.ref.code, AimuxErrorCode.ok);
      expect(err.ref.status, -1);
      expect(err.ref.retryMs, -1);
      expect(err.ref.message, nullptr);
      expect(err.ref.errorValue, nullptr);
      return 42;
    });
    expect(result, 42);
  });
}
