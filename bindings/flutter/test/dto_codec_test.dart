import 'package:aimux/types.dart';
import 'package:test/test.dart';

void main() {
  test('opaque integers and numeric u64 never pass through double', () {
    const source = '{"large":18446744073709551615,"safe":9007199254740993}';
    final value = dtoJsonDecode(source) as Map;
    expect(value['large'], BigInt.parse('18446744073709551615'));
    expect(value['safe'], 9007199254740993);
    expect(dtoJsonEncode(value), source);
    final error = ErrorEnvelope.fromJson({
      'kind': 'aimux',
      'code': 11,
      'message': 'retry',
      'error': null,
      'retryAfterMs': value['large'],
    });
    expect(error.retryAfterMs, value['large']);
    expect((dtoJsonDecode(dtoJsonEncode(error)) as Map)['retryAfterMs'],
        value['large']);
  });

  test('presence distinguishes absent, explicit null, and a value', () {
    expect(RequestInfo().body.isPresent, isFalse);
    expect(RequestInfo.fromJson({'body': null}).body.isPresent, isTrue);
    expect(RequestInfo.fromJson({'body': null}).body.value, isNull);
    expect(RequestInfo(body: const Field.of(null)).toJson(), {'body': null});
  });

  test('constructed DTOs keep immutable snapshots of opaque JSON', () {
    final nested = <Object?>[1, null];
    final body = <String, Object?>{'items': nested};
    final request = RequestInfo(body: Field.of(body));
    nested.add(2);
    body['other'] = true;
    expect(request.toJson(), {
      'body': {
        'items': [1, null]
      }
    });
    expect(() => (request.body.value as Map)['changed'] = true,
        throwsUnsupportedError);
  });

  test('ordered unions select the first matching shape', () {
    expect(DecisionInput.fromJson('text'), isA<DecisionInputBranch1>());
    expect(DecisionInput.fromJson({'key': null}), isA<DecisionInputBranch2>());
    expect(DecisionInput.fromJson([1, null]), isA<DecisionInputBranch3>());
    expect(() => DecisionInput.fromJson(true), throwsFormatException);
  });

  test('tag errors identify the union and unknown tag', () {
    expect(
        () => GenerateContent.fromJson({'type': 'future'}),
        throwsA(isA<FormatException>().having((e) => e.message, 'message',
            allOf(contains('GenerateContent'), contains('future')))));
  });
}
