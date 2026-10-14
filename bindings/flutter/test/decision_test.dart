import 'dart:convert';
import 'dart:io';
import 'dart:isolate';
import 'package:aimux/aimux.dart';
import 'package:test/test.dart';

// Keep the worker closure outside the server's lexical scope so Dart cannot
// capture its unsendable HttpServer through a shared closure context.
Future<Map<String, dynamic>> runDecision(
    String endpoint, Map<String, dynamic> options) => Isolate.run(() {
  final model = DecisionModel.jev('test-key', 'jev-latest', endpoint: endpoint);
  try {
    final description = model.describe();
    if (description['provider'] != 'jev' ||
        (description['supportedQuestionTypes'] as List).length != 3) {
      throw StateError('incorrect description');
    }
    return model.decide(options);
  } finally { model.close(); }
});

Future<Map<String, dynamic>> runOpenAIDecision(
    String baseUrl, Map<String, dynamic> options) => Isolate.run(() {
  final provider = createProvider('openai', 'test-key', ProviderConfig(baseUrl: baseUrl));
  final model = provider.decisionModel('gpt-6-luna');
  try { return model.decide(options); }
  finally { model.close(); provider.close(); }
});

void main() {
  test('OpenAI media and typed values cross the C ABI', () async {
    final fixture = jsonDecode(File('../../contract-tests/fixtures/decision-openai-full.json').readAsStringSync()) as Map;
    final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    final captures = <dynamic>[];
    server.listen((request) async {
      expect(request.uri.path, '/v1/decisions');
      captures.add(jsonDecode(await utf8.decoder.bind(request).join()));
      request.response.headers.contentType = ContentType.json;
      request.response.write(jsonEncode(fixture['response']));
      await request.response.close();
    });
    addTearDown(() => server.close(force: true));
    final options = (fixture['options'] as Map).cast<String, dynamic>();
    final result = await runOpenAIDecision('http://127.0.0.1:${server.port}/v1', options);
    expect(captures, [fixture['request']]);
    expect(result['answers']['choice']['choice'], 'boolean_true');
    expect(result['providerMetadata']['openai']['answers']['choice']['value'], isTrue);
    expect(result['answers']['score']['score'], 0.75);
  });

  test('provider decision model owns its handle', () {
    final provider = createProvider('openai', 'test-key', null);
    final model = provider.decisionModel('gpt-6-luna');
    addTearDown(provider.close);
    addTearDown(model.close);
    provider.close();
    expect(model.describe()['specificationVersion'], 'v4');
    expect(model.describe()['provider'], 'openai');
    expect(model.describe()['modelId'], 'gpt-6-luna');
    expect(() => provider.decisionModel('gpt-6-luna'), throwsStateError);
  });

  test('official structured contract and description cross the C ABI', () async {
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
    final result = await runDecision(endpoint, options);
    expect(captures.length, 1);
    expect(result['answers']['urgent']['probability'], 0.9);
    expect(result['rounding']['scoreDecimals'], 2);
    expect(result['usage'], {'inputTokens': 50, 'outputTokens': 10, 'totalTokens': 60});
    expect(captures.single['questions']['severity']['criteria'], options['questions']['severity']['criteria']);
    expect(captures.single['questions']['urgent']['criteria'], options['questions']['urgent']['criteria']);
    final model = DecisionModel.jev('test-key', 'jev-latest');
    expect(() => model.decideJson('{'), throwsFormatException);
    model.close();
    model.close();
    expect(model.describe, throwsStateError);
    expect(() => DecisionModel.jev('test', 'jev-latest', probabilitySource: 'unknown'), throwsA(isA<AimuxException>()));
  });
}
