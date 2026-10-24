@Timeout(Duration(minutes: 2))
library;

import 'dart:io';
import 'dart:typed_data';

import 'package:aimux/aimux.dart';
import 'package:aimux/decision.dart';
import 'package:aimux/generated/ops.dart' as ops;
import 'package:test/test.dart';

void main() {
  final library = Platform.environment['AIMUX_NATIVE_LIBRARY'];
  final bridge = Platform.environment['AIMUX_DART_BRIDGE'];
  group('real Rust ABI', () {
    late Aimux client;
    final handles = <OpsHandle>[];
    Future<OpsHandle> create(String op,
        {OpsHandle? handle, Object? params = const <String, dynamic>{}}) async {
      final value =
          await client.call(op, handle: handle, params: params) as Map;
      final result = client.adopt(value['handle'] as String);
      handles.add(result);
      return result;
    }

    setUp(() async {
      client = await Aimux.open(
          libraryPath: library,
          bridgePath: bridge,
          requiredOps: [
            ops.OP_RUNTIME_DESCRIBE,
            ops.OP_PROVIDER_CREATE,
            ops.OP_PROVIDER_MODEL,
            ops.OP_DECISION_DECIDE,
            ops.OP_DECISION_CAPABILITIES,
            ops.OP_TEXT_STREAM
          ]);
    });
    tearDown(() async {
      for (final handle in handles.reversed) {
        await handle.close();
      }
      handles.clear();
    });

    test('manifest and runtime.describe', () async {
      expect(client.manifest['handleKinds'], contains('Decision'));
      expect(await client.call(ops.OP_RUNTIME_DESCRIBE), isA<Map>());
    });

    test('provider/model handles retain children after provider close',
        () async {
      final provider = await create(ops.OP_PROVIDER_CREATE, params: {
        'package': 'openai',
        'settings': {'apiKey': 'test'}
      });
      final model = await create(ops.OP_PROVIDER_MODEL,
          handle: provider,
          params: {'method': 'languageModel', 'modelId': 'gpt-test'});
      await provider.close();
      final info = await client.call(ops.OP_MODEL_INFO, handle: model) as Map;
      expect(info['modelId'], 'gpt-test');
      await model.close();
      await model.close();
    });

    test('Decision capabilities use existing provider configuration', () async {
      final provider = await create(ops.OP_PROVIDER_CREATE, params: {
        'package': 'jev',
        'settings': {'apiKey': 'test'}
      });
      final model = await create(ops.OP_PROVIDER_MODEL,
          handle: provider,
          params: {'method': 'decision', 'modelId': 'decision-test'});
      final decision = DecisionModel(client, model);
      final info = await decision.capabilities();
      expect(info['modelId'], 'decision-test');
      await expectLater(
          decision.decide(
              DecideOptions.fromJson({'state': 'test', 'questions': {}})),
          throwsA(isA<OpsException>().having((e) => e.code, 'code', 5)));
    });

    test('wrong-kind binary/poll and protocol errors preserve envelopes',
        () async {
      final abort = await create(ops.OP_ABORT_CREATE);
      final wrongKind = isA<OpsException>()
          .having((e) => e.code, 'code', 203)
          .having((e) => (e.error as Map)['name'], 'name', 'InvalidHandle');
      await expectLater(client.push(abort, Uint8List(0)), throwsA(wrongKind));
      await expectLater(client.next(abort), throwsA(wrongKind));
      expect(() => client.next(abort, timeoutMs: -1), throwsArgumentError);
      await expectLater(client.call('missing.op'),
          throwsA(isA<OpsException>().having((e) => e.code, 'code', 300)));
    });

    test('accepted cancelled stream has one terminal through native callback',
        () async {
      final provider = await create(ops.OP_PROVIDER_CREATE, params: {
        'package': 'openai',
        'settings': {'apiKey': 'test'}
      });
      final model = await create(ops.OP_PROVIDER_MODEL,
          handle: provider,
          params: {'method': 'languageModel', 'modelId': 'gpt-test'});
      final abort = await create(ops.OP_ABORT_CREATE);
      client.abort(abort);
      final frames = await client.stream(ops.OP_TEXT_STREAM,
          handle: model,
          params: {'prompt': 'hello', 'abort': abort.wireId}).toList();
      expect(frames.whereType<StreamFrameEnd>(), hasLength(1));
      expect(
          frames.whereType<StreamFrameEnd>().single.outcome, Outcome.cancelled);
      expect(abort.isClosed, isFalse);
    });

    test('rejected stream returns one error without terminal frames', () async {
      final abort = await create(ops.OP_ABORT_CREATE);
      final frames = <StreamFrame>[];
      final errors = <Object>[];
      await client
          .stream(ops.OP_TEXT_STREAM, handle: abort, params: {'prompt': 'hello'})
          .handleError((Object e) => errors.add(e))
          .forEach(frames.add);
      expect(frames, isEmpty);
      expect(errors.single,
          isA<OpsException>().having((e) => e.code, 'code', 203));
    });
  },
      skip: library == null
          ? 'Set AIMUX_NATIVE_LIBRARY and AIMUX_DART_BRIDGE to the native build'
          : false);
}
