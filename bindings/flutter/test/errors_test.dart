import 'package:aimux/errors.dart';
import 'package:aimux/types.dart';
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
    expect(error.kind, 'protocol');
  });
  test('AI SDK payload and response headers remain unchanged', () {
    final error = OpsException.fromJson({
      'kind': 'aimux',
      'code': 11,
      'message': 'try later',
      'error': {
        'name': 'AI_APICallError',
        'isRetryable': true,
        'statusCode': 429,
        'responseHeaders': {'retry-after': '2.5'},
      },
    });
    expect((error.error as Map)['isRetryable'], isTrue);
    expect((error.error as Map)['responseHeaders'], {'retry-after': '2.5'});
    expect((error.error as Map)['statusCode'], 429);
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
    final source = (error.cause as ErrorCauseBranchSourceError).value;
    expect(source.message, 'tcp connect error');
    expect(source.cause?.message, 'Connection refused');
    expect(
        ((error.error as Map)['cause'] as Map)['message'], 'tcp connect error');
    expect(bindingFailure('x').cause, isNull);
    final nested = OpsException.fromJson({
      'kind': 'aimux',
      'code': 11,
      'message': 'm',
      'error': {
        'name': 'AI_APICallError',
        'url': 'https://example.test/v1',
        'requestBodyValues': <String, dynamic>{},
        'statusCode': 200,
        'message': 'm',
        'isRetryable': false,
        'cause': {
          'name': 'AI_JSONParseError',
          'message': 'Unexpected end of JSON input',
        },
      },
    });
    final parse = (nested.cause as ErrorCauseBranchAiMuxError).value
        as AiMuxErrorAIJSONParseError;
    expect(parse.message, 'Unexpected end of JSON input');
    final retry = OpsException.fromJson({
      'kind': 'aimux',
      'code': 10,
      'message': 'r',
      'error': {
        'name': 'AI_RetryError',
        'message': 'r',
        'reason': 'maxRetriesExceeded',
        'errors': [
          {'name': 'AI_JSONParseError', 'message': 'first'},
          {'name': 'AI_JSONParseError', 'message': 'last'},
        ],
      },
    });
    final last = (retry.cause as ErrorCauseBranchAiMuxError).value
        as AiMuxErrorAIJSONParseError;
    expect(last.message, 'last');
  });
  test('malformed envelope is rejected', () {
    expect(() => OpsException.fromJson({'kind': 'aimux', 'code': 11}),
        throwsA(isA<TypeError>()));
    expect(
      () => OpsException.fromJson({
        'kind': 'binding',
        'code': '203',
        'message': 'bad',
        'error': {},
      }),
      throwsA(isA<TypeError>()),
    );
  });
}
