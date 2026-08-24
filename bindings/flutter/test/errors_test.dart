// Pure-Dart tests for the AimuxException hierarchy / code mapping.
// Does not require the native library (the C-error path — expectAimuxError via
// the aimux_error_* getters — is covered in aimux_test.dart, which does).

import 'package:aimux/errors.dart';
import 'package:test/test.dart';

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
        AimuxErrorCode.retry: RetryError,
      };
      for (final entry in cases.entries) {
        final e = AimuxException.fromCode(entry.key, 'msg');
        expect(e.runtimeType, entry.value, reason: 'code ${entry.key}');
        expect(e, isA<AimuxException>());
      }
    });

    test('unknown code is rejected with StateError', () {
      // A code outside the published table is an ABI mismatch, not an error
      // kind. 1 is AIMUX_E_OTHER now because Other inherited the old UNKNOWN
      // slot, so it resolves; 15 is the first unassigned value.
      expect(() => AimuxException.fromCode(999, 'future'), throwsStateError);
      expect(() => AimuxException.fromCode(15, 'unused'), throwsStateError);
    });

    test('bare retry code synthesizes a single-attempt RetryError', () {
      final e = AimuxException.fromCode(AimuxErrorCode.retry, 'gave up');
      expect(e, isA<RetryError>());
      final retry = e as RetryError;
      expect(retry.reason, RetryErrorReason.maxRetriesExceeded);
      expect(retry.errors, hasLength(1));
      expect(retry.lastError, isA<OtherError>());
    });
  });

  group('RetryError', () {
    test('keeps the per-attempt history, oldest first', () {
      final e = RetryError(
        'Failed after 2 attempts. Last error: took too long',
        reason: RetryErrorReason.maxRetriesExceeded,
        errors: [
          APICallError('API call error: HTTP 429: slow down',
              status: 429, retryMs: 1500, retryable: true),
          AimuxTimeoutError('took too long'),
        ],
      );
      expect(e.code, AimuxErrorCode.retry);
      expect(e.codeName, 'Retry');
      expect(e.errors, hasLength(2));
      expect(e.errors.first, isA<APICallError>());
      expect(e.lastError, isA<AimuxTimeoutError>());
      // The history is a snapshot, not a mutable list.
      expect(() => e.errors.add(OtherError('x')), throwsUnsupportedError);
    });

    test('reason decodes from the core wire names', () {
      expect(RetryErrorReason.fromWire('maxRetriesExceeded'),
          RetryErrorReason.maxRetriesExceeded);
      expect(RetryErrorReason.fromWire('errorNotRetryable'),
          RetryErrorReason.errorNotRetryable);
      // Defensive default for a missing/unknown wire value.
      expect(
          RetryErrorReason.fromWire(null), RetryErrorReason.maxRetriesExceeded);
    });
  });

  group('APICallError', () {
    test('carries the enriched request/response context', () {
      final e = APICallError(
        'API call error: HTTP 429: slow down',
        status: 429,
        retryMs: 1500,
        retryable: true,
        providerCode: 'rate_limit_exceeded',
        providerMessage: 'slow down',
        url: 'https://api.example',
        requestBodyValues: {'model': 'm'},
        responseHeaders: {'retry-after-ms': '1500'},
        responseBody: '{"error":"slow down"}',
        data: {'type': 'rate_limit'},
      );
      expect(e.retryable, isTrue);
      expect(e.providerCode, 'rate_limit_exceeded');
      expect(e.url, 'https://api.example');
      expect(e.requestBodyValues, {'model': 'm'});
      expect(e.responseHeaders?['retry-after-ms'], '1500');
      expect(e.data, {'type': 'rate_limit'});
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
      // Engine codes are contiguous 1–14.
      expect(AimuxErrorCode.other, 1);
      expect(AimuxErrorCode.aborted, 13);
      expect(AimuxErrorCode.retry, 14);
      expect(AimuxErrorCode.name(AimuxErrorCode.retry), 'Retry');
    });
  });

  test('recording codes outside the Rust enum are rejected', () {
    expect(() => RecordingErrorCode.fromCode(999), throwsStateError);
  });

  // Raw JSON parameters are validated in Dart before the C call: bad syntax
  // is a native FormatException naming the parameter (pure-Dart, no dlopen).
  test('checkJson rejects bad syntax with FormatException naming the param', () {
    expect(() => checkJson('{not json', 'opts_json'), throwsA(
        isA<FormatException>()
            .having((e) => e.source, 'source', 'opts_json')
            .having((e) => e.message, 'message', contains('opts_json'))));
    expect(() => checkJson(null, 'opts_json'), returnsNormally);
    expect(() => checkJson('', 'opts_json'), throwsA(
        isA<FormatException>().having((e) => e.source, 'source', 'opts_json')));
    expect(() => checkJson(' ', 'config_json', emptyIsDefault: true),
        returnsNormally);
    // Optional opts_json (embed / upload / transcription): "" = defaults.
    expect(() => checkJson('', 'opts_json', emptyIsDefault: true),
        returnsNormally);
    expect(() => checkJson('{"a": [1, 2]}', 'opts_json'), returnsNormally);
    expect(() => checkJson('"text"', 'prompt_json'), returnsNormally);
  });

  // A Dart String is UTF-16 and may hold an unpaired surrogate; jsonDecode /
  // jsonEncode both accept one, serde_json does not. Left unchecked it reaches
  // C as FfiError::InvalidWireJson (code 202), which surfaces as an invariant
  // StateError — for input a
  // `text.substring(0, n)` across an emoji produces.
  test('checkJson rejects what serde_json rejects but jsonDecode accepts', () {
    for (final bad in [
      '{"text":"\\ud800"}', // lone leading surrogate escape
      '"\\udc00"', // lone trailing surrogate escape
      '"\\ud800\\ud800"', // leading followed by leading
      '{"\\ud800":1}', // in a key
      '{"temperature":1e999}', // Dart parses to Infinity
    ]) {
      expect(() => checkJson(bad, 'opts_json'),
          throwsA(isA<FormatException>().having(
              (e) => e.message, 'message', contains('opts_json'))),
          reason: bad);
    }
    // Well-formed pairs and finite numbers still pass.
    expect(() => checkJson('"\\ud83d\\ude00"', 'prompt_json'), returnsNormally);
    expect(() => checkJson('{"a":1e-999}', 'opts_json'), returnsNormally);
    // Bounded so the walk cannot outrun the stack on a document jsonDecode
    // (which is iterative) happily returns.
    expect(() => checkJson('[' * 100 + ']' * 100, 'opts_json'), returnsNormally);
    expect(() => checkJson('[' * 5000 + ']' * 5000, 'opts_json'),
        throwsA(isA<FormatException>()));
  });

  // The same rule on the JSON the binding builds itself (prompt / options /
  // ProviderConfig) — nothing downstream would check that.
  test('encodeJson rejects an unpaired surrogate in caller data', () {
    final halfEmoji = '\u{1F600}'.substring(0, 1); // split emoji
    expect(() => encodeJson(halfEmoji, 'prompt'),
        throwsA(isA<FormatException>()
            .having((e) => e.message, 'message', contains('prompt'))));
    expect(() => encodeJson({'system': [halfEmoji]}, 'options'),
        throwsA(isA<FormatException>()));
    expect(() => encodeJson({'a': double.infinity}, 'options'),
        throwsA(isA<FormatException>()));
    expect(encodeJson('\u{1F600}', 'prompt'), '"\u{1F600}"');
    expect(encodeJson({'a': 1}, 'options'), '{"a":1}');
  });
}
