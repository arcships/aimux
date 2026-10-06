import 'dart:convert';
import 'dart:io';
import 'dart:isolate';
import 'package:aimux/decision.dart';
import 'package:aimux/errors.dart';
import 'package:test/test.dart';

void main() {
  test('official structured contract and capabilities cross the C ABI', () async {
    final fixture = jsonDecode(File('../../contract-tests/fixtures/decision-native.json').readAsStringSync()) as Map;
    final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    final captures = <Map<String, dynamic>>[];
    server.listen((request) async {
      expect(request.uri.path, '/v1/systemone');
      expect(request.headers.value('authorization'), 'Bearer test-key');
      captures.add((jsonDecode(await utf8.decoder.bind(request).join()) as Map).cast<String, dynamic>());
      request.response.headers.contentType = ContentType.json;
      request.response.write(jsonEncode(fixture['response']));
      await request.response.close();
    });
    addTearDown(() => server.close(force: true));
    final endpoint = 'http://127.0.0.1:${server.port}/v1/systemone';
    final options = (fixture['request'] as Map).cast<String, dynamic>();
    // The Rust call blocks; the HTTP server remains on this isolate.
    final result = await Isolate.run(() {
      final model = DecisionModel.jev('test-key', 'jev-latest', endpoint: endpoint);
      try {
        final caps = model.capabilities();
        if (caps['max_choices'] != 255 || caps['rounding']['score_decimals'] != 2) {
          throw StateError('incorrect capabilities');
        }
        return model.decide(options);
      } finally { model.close(); }
    });
    expect(captures.length, 1);
    expect(result['answers']['urgent']['probability_true'], 0.9);
    expect(result['answers']['severity']['levels'], options['questions'][2]['levels']);
    expect(captures.single['questions']['urgent']['criteria'], options['questions'][0]['criteria']);
    final model = DecisionModel.jev('test-key', 'jev-latest');
    expect(() => model.decideJson('{'), throwsFormatException);
    model.close();
    model.close();
    expect(model.capabilities, throwsStateError);
    expect(() => DecisionModel.jev('test', 'jev-latest', probabilitySource: 'unknown'), throwsA(isA<AimuxException>()));
  });
}
