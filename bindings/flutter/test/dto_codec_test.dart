import 'dart:convert';
import 'package:aimux/types.dart';
import 'package:test/test.dart';

void main() {
  test('native integers keep signed 64-bit precision', () {
    const source = '{"safe":9007199254740993,"max":9223372036854775807}';
    final value = jsonDecode(source) as Map;
    expect(value['safe'], 9007199254740993);
    expect(value['max'], 9223372036854775807);
    expect(jsonEncode(value), source);
    final error = ErrorEnvelope.fromJson({
      'kind': 'aimux',
      'code': 11,
      'message': 'retry',
      'error': null,
      'retryAfterMs': value['max'],
    });
    expect(error.retryAfterMs, value['max']);
    expect(
        (jsonDecode(jsonEncode(error)) as Map)['retryAfterMs'], value['max']);
  });

  test('presence distinguishes absent, explicit null, and a value', () {
    expect(RequestInfo().body.isPresent, isFalse);
    expect(RequestInfo.fromJson({'body': null}).body.isPresent, isTrue);
    expect(RequestInfo.fromJson({'body': null}).body.value, isNull);
    expect(RequestInfo(body: const Field.of(null)).toJson(), {'body': null});
  });

  test('ordinary optional nulls are omitted and unknown keys are ignored', () {
    expect(
        RequestInfo.fromJson({'headers': null, 'future': true}).toJson(), {});
    expect(GenerateTextOptions.fromJson({'stopSequences': []}).toJson(),
        {'stopSequences': []});
    expect(
        ErrorEnvelope.fromJson({
          'kind': 'aimux',
          'code': 11,
          'message': 'retry',
          'error': null,
        }).toJson(),
        {'kind': 'aimux', 'code': 11, 'message': 'retry', 'error': null});
  });

  test('ordered unions select the first matching shape', () {
    expect(DecisionInput.fromJson('text'), isA<DecisionInputBranchText>());
    expect(DecisionInput.fromJson({'key': null}), isA<DecisionInputBranchMap>());
    expect(DecisionInput.fromJson([1, null]), isA<DecisionInputBranchList>());
    expect(() => DecisionInput.fromJson(true), throwsA(isA<TypeError>()));
  });

  test('tag errors identify the union and unknown tag', () {
    expect(
        () => GenerateContent.fromJson({'type': 'future'}),
        throwsA(isA<FormatException>().having((e) => e.message, 'message',
            allOf(contains('GenerateContent'), contains('future')))));
  });
}
