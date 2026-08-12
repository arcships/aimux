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
    test('rate limiting arrives as APICallError with status 429', () {
      final e = AimuxException.fromCode(
        AimuxErrorCode.apiCall,
        'API call error: HTTP 429: slow down',
        status: 429,
        retryMs: 1500,
      );
      expect(e, isA<APICallError>());
      expect(e.message, 'API call error: HTTP 429: slow down');
      expect(e.code, AimuxErrorCode.apiCall);
      expect(e.status, 429);
      expect(e.retryMs, 1500);
      expect(e.toString(), contains('APICallError'));
    });

    test('auth failure arrives as APICallError with status 401', () {
      final e = AimuxException.fromCode(
        AimuxErrorCode.apiCall,
        'API call error: HTTP 401: bad key',
        status: 401,
      );
      expect(e, isA<APICallError>());
      expect(e.status, 401);
      expect(e.codeName, 'ApiCall');
    });

    test('model-not-found arrives as APICallError with status 404', () {
      final e = AimuxException.fromCode(
        AimuxErrorCode.apiCall,
        'API call error: HTTP 404: no such model',
        status: 404,
      );
      expect(e, isA<APICallError>());
      expect(e.status, 404);
    });

    test('transport failure is an APICallError with no status', () {
      final e = AimuxException.fromCode(
        AimuxErrorCode.apiCall,
        'API call error: connection reset',
      );
      expect(e, isA<APICallError>());
      expect(e.status, -1);
      expect(e.retryMs, -1);
    });

    test('TokenExpiredError keeps its 401 contract', () {
      final e = AimuxException.fromCode(AimuxErrorCode.tokenExpired, 'expired');
      expect(e, isA<TokenExpiredError>());
      expect(e.status, 401);
    });

    test('dispatches all known subclasses', () {
      final cases = <int, Type>{
        AimuxErrorCode.unknown: UnknownError,
        AimuxErrorCode.jsonParse: JSONParseError,
        AimuxErrorCode.invalidResponseData: InvalidResponseDataError,
        AimuxErrorCode.tool: ToolError,
        AimuxErrorCode.invalidArgument: InvalidArgumentError,
        AimuxErrorCode.invalidPrompt: InvalidPromptError,
        AimuxErrorCode.tokenExpired: TokenExpiredError,
        AimuxErrorCode.unsupportedFunctionality: UnsupportedFunctionalityError,
        AimuxErrorCode.noSuchModel: NoSuchModelError,
        AimuxErrorCode.noSuchProvider: NoSuchProviderError,
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

    test('429 arrives as APICallError with status, hint and ApiCall JSON', () {
      const json = '{"ApiCall":{"status_code":429,"provider_code":'
          '"rate_limit_exceeded","message":"slow down","retry_after_ms":1500,'
          '"is_retryable":true}}';
      final e = fromFilled((err) {
        err.ref.code = AimuxErrorCode.apiCall;
        err.ref.status = 429;
        err.ref.retryMs = 1500;
        err.ref.retryable = 1;
        err.ref.message = 'API call error: HTTP 429: slow down'.toNativeUtf8();
        err.ref.errorValue = json.toNativeUtf8();
      });
      expect(e, isA<APICallError>());
      expect(e.status, 429);
      expect(e.retryMs, 1500);
      expect(e.retryable, isTrue);
      expect(e.errorValue, json);
    });

    test('401 arrives as APICallError with status 401', () {
      final e = fromFilled((err) {
        err.ref.code = AimuxErrorCode.apiCall;
        err.ref.status = 401;
        err.ref.message =
            'API call error: HTTP 401: invalid api key'.toNativeUtf8();
      });
      expect(e, isA<APICallError>());
      expect(e.status, 401);
      expect(e.retryable, isFalse);
      expect(e.message, contains('HTTP 401'));
    });

    test('NoSuchModel carries its struct payload JSON', () {
      const json =
          '{"NoSuchModel":{"model_id":"gpt-9","model_type":"languageModel"}}';
      final e = fromFilled((err) {
        err.ref.code = AimuxErrorCode.noSuchModel;
        err.ref.message = 'no such model: gpt-9'.toNativeUtf8();
        err.ref.errorValue = json.toNativeUtf8();
      });
      expect(e, isA<NoSuchModelError>());
      expect(e.status, -1);
      expect(e.errorValue, json);
    });

    test('retryable crosses the ABI and status cannot stand in', () {
      // Both report status == -1 and disagree — never infer one from the other.
      final transport = fromFilled((err) {
        err.ref.code = AimuxErrorCode.apiCall;
        err.ref.message = 'API call error: connection reset'.toNativeUtf8();
        err.ref.retryable = 1;
      });
      expect(transport.status, -1);
      expect(transport.retryable, isTrue);

      final noKey = fromFilled((err) {
        err.ref.code = AimuxErrorCode.apiCall;
        err.ref.message = 'API call error: missing api key'.toNativeUtf8();
      });
      expect(noKey.status, -1);
      expect(noKey.retryable, isFalse);
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
    test('covers the code table', () {
      expect(AimuxErrorCode.name(AimuxErrorCode.ok), 'OK');
      expect(AimuxErrorCode.name(AimuxErrorCode.apiCall), 'ApiCall');
      expect(AimuxErrorCode.name(AimuxErrorCode.other), 'Other');
    });

    test('codes are consecutive and named', () {
      expect(AimuxErrorCode.jsonParse, 2);
      expect(AimuxErrorCode.name(AimuxErrorCode.jsonParse), 'JsonParse');
      expect(AimuxErrorCode.invalidResponseData, 3);
      expect(AimuxErrorCode.name(AimuxErrorCode.invalidResponseData),
          'InvalidResponseData');
      expect(AimuxErrorCode.unsupportedFunctionality, 8);
      expect(AimuxErrorCode.name(AimuxErrorCode.unsupportedFunctionality),
          'UnsupportedFunctionality');
      expect(AimuxErrorCode.noSuchProvider, 10);
      expect(AimuxErrorCode.name(AimuxErrorCode.noSuchProvider),
          'NoSuchProvider');
      expect(AimuxErrorCode.other, 14);
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
      expect(err.ref.retryable, 0);
      expect(err.ref.reserved, 0);
      return 42;
    });
    expect(result, 42);
  });
}
