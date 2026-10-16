import 'package:aimux/errors.dart';
import 'package:test/test.dart';

void main() {
  test(
      'error envelope preserves opaque payload and domain and ignores unknown keys',
      () {
    final json = <String, dynamic>{
      'kind': 'protocol',
      'code': 399,
      'message': 'new error',
      'error': {
        'name': 'FutureError',
        'nested': [1, null],
      },
      'futureField': true,
    };
    final error = OpsException.fromJson(json);
    expect(error.toJson(), Map.of(json)..remove('futureField'));
    expect(error.retryAfterMs, isNull);
    expect(error.kind, 'protocol');
  });
  test('AI SDK payload and optional retry hint remain unchanged', () {
    final error = OpsException.fromJson({
      'kind': 'aimux',
      'code': 11,
      'message': 'try later',
      'retryAfterMs': 2500,
      'error': {
        'name': 'AI_APICallError',
        'isRetryable': true,
        'statusCode': 429,
        'responseHeaders': {'retry-after': '2.5'},
      },
    });
    expect((error.error as Map)['isRetryable'], isTrue);
    expect(error.retryAfterMs, 2500);
    expect((error.error as Map)['statusCode'], 429);
    expect(error.toJson().containsKey('retryable'), isFalse);
  });
  test('malformed envelope is rejected', () {
    expect(() => OpsException.fromJson({'kind': 'aimux', 'code': 11}),
        throwsA(isA<TypeError>()));
    expect(
      () => OpsException.fromJson({
        'kind': 'binding',
        'code': 203,
        'message': 'bad',
        'error': {},
        'retryAfterMs': 'invalid',
      }),
      throwsA(isA<TypeError>()),
    );
  });
}
