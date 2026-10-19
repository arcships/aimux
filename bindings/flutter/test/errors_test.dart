import 'package:aimux/errors.dart';
import 'package:test/test.dart';

void main() {
  test('error envelope preserves unknown names, fields and domain', () {
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
    expect(error.toJson(), json);
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
    expect(error.error['isRetryable'], isTrue);
    expect(error.retryAfterMs, 2500);
    expect(error.error['statusCode'], 429);
    expect(error.toJson().containsKey('retryable'), isFalse);
  });
  test('cause is the payload cause chain', () {
    final error = OpsException.fromJson({
      'kind': 'aimux',
      'code': 11,
      'message': 'API call error: Cannot connect',
      'error': {
        'name': 'AI_APICallError',
        'message': 'Cannot connect',
        'cause': {
          'message': 'tcp connect error',
          'cause': {'message': 'Connection refused'},
        },
      },
    });
    expect(error.cause?['message'], 'tcp connect error');
    expect((error.cause?['cause'] as Map)['message'], 'Connection refused');
    expect(bindingFailure('x').cause, isNull);
  });
  test('malformed envelope is rejected', () {
    expect(() => OpsException.fromJson({'code': 11}), throwsFormatException);
    expect(
      () => OpsException.fromJson({
        'kind': 'binding',
        'code': 203,
        'message': 'bad',
        'error': {},
        'retryAfterMs': -1,
      }),
      throwsFormatException,
    );
  });
}
