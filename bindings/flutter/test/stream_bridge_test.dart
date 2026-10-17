import 'dart:async';
import 'dart:io';
import 'dart:isolate';

import 'package:aimux/src/stream_bridge.dart';
import 'package:test/test.dart';

BlockingStreamProducer producer(Object? request) => _Producer(request! as List);

class _Producer implements BlockingStreamProducer {
  _Producer(this.request);
  final List request;

  @override
  void run(StreamBridgeSink sink) {
    final count = request[0] as int;
    final observations = request[1] as SendPort;
    final mode = request[2] as String;
    observations.send('started');
    if (mode == 'startup-error') throw StateError('not accepted');
    for (var i = 0; i < count; i++) {
      if (mode == 'slow') sleep(const Duration(milliseconds: 15));
      if (!sink.add(mode == 'large' ? 'éé' : '$i')) {
        observations.send('cancelled');
        sink.end();
        return;
      }
      observations.send(i);
    }
    if (mode == 'error') {
      sink.end({
        'kind': 'protocol',
        'code': 304,
        'message': 'failure',
        'error': {'name': 'InvalidState'},
      });
      sink.end('duplicate');
      throw StateError('after terminal');
    }
    if (mode != 'missing-end') sink.end();
  }
}

void main() {
  test(
    'ordered lossless delivery and blocking work leaves main responsive',
    () async {
      final observations = ReceivePort()..listen((_) {});
      var heartbeats = 0;
      final timer = Timer.periodic(
        const Duration(milliseconds: 2),
        (_) => heartbeats++,
      );
      final bridge = StreamBridge(
        producer: producer,
        request: [20, observations.sendPort, 'slow'],
        abort: () {},
      );
      final result = await bridge.stream.toList();
      await bridge.done;
      timer.cancel();
      observations.close();
      expect(result, List.generate(20, (i) => '$i'));
      expect(heartbeats, greaterThan(5));
    },
  );

  test('pause exhausts credits, resume preserves every event', () async {
    final observations = ReceivePort();
    final produced = <int>[];
    final filled = Completer<void>();
    observations.listen((value) {
      if (value is int) {
        produced.add(value);
        if (produced.length == 2) filled.complete();
      }
    });
    final bridge = StreamBridge(
      producer: producer,
      request: [30, observations.sendPort, 'normal'],
      abort: () {},
      credits: 2,
    );
    final first = Completer<void>();
    final ended = Completer<void>();
    final values = <String>[];
    late StreamSubscription<String> subscription;
    subscription = bridge.stream.listen((event) {
      values.add(event);
      if (values.length == 1) {
        subscription.pause();
        first.complete();
      }
    }, onDone: ended.complete);
    await first.future;
    await filled.future;
    await Future<void>.delayed(const Duration(milliseconds: 50));
    expect(values, ['0']);
    expect(produced, [0, 1]);
    subscription.resume();
    await ended.future;
    await bridge.done;
    observations.close();
    expect(values, List.generate(30, (i) => '$i'));
  });

  test('cancel with exhausted credits aborts once and wakes worker', () async {
    final observations = ReceivePort();
    final cancelled = Completer<void>();
    observations.listen((value) {
      if (value == 'cancelled') cancelled.complete();
    });
    var aborts = 0;
    final bridge = StreamBridge(
      producer: producer,
      request: [100, observations.sendPort, 'normal'],
      abort: () => aborts++,
      credits: 1,
    );
    final first = Completer<void>();
    late StreamSubscription<String> subscription;
    subscription = bridge.stream.listen((event) {
      subscription.pause();
      first.complete();
    });
    await first.future;
    await Future<void>.delayed(const Duration(milliseconds: 30));
    await subscription.cancel().timeout(const Duration(seconds: 3));
    await subscription.cancel();
    await cancelled.future.timeout(const Duration(seconds: 3));
    await bridge.done;
    observations.close();
    expect(aborts, 1);
  });

  test(
    'terminal error follows accepted events exactly once while paused',
    () async {
      final observations = ReceivePort()..listen((_) {});
      var aborts = 0;
      final bridge = StreamBridge(
        producer: producer,
        request: [1, observations.sendPort, 'error'],
        abort: () => aborts++,
        credits: 1,
      );
      final first = Completer<void>();
      final ended = Completer<void>();
      final events = <String>[];
      final errors = <Object>[];
      late StreamSubscription<String> subscription;
      subscription = bridge.stream.listen(
        (event) {
          events.add(event);
          subscription.pause();
          first.complete();
        },
        onError: (Object error) => errors.add(error),
        onDone: ended.complete,
      );
      await first.future;
      await bridge.done;
      expect(errors, isEmpty);
      subscription.resume();
      await ended.future;
      observations.close();
      expect(events, ['0']);
      expect(errors, hasLength(1));
      expect((errors.single as Map)['code'], 304);
      expect(aborts, 0);
    },
  );

  test('missing terminal is an error rather than successful EOF', () async {
    final observations = ReceivePort()..listen((_) {});
    final bridge = StreamBridge(
      producer: producer,
      request: [0, observations.sendPort, 'missing-end'],
      abort: () {},
    );
    await expectLater(bridge.stream, emitsError(isA<StateError>()));
    await bridge.done;
    observations.close();
  });

  test('UTF-8 byte limit rejects oversized event', () async {
    final observations = ReceivePort()..listen((_) {});
    final bridge = StreamBridge(
      producer: producer,
      request: [1, observations.sendPort, 'large'],
      abort: () {},
      maxEventBytes: 3,
    );
    await expectLater(bridge.stream, emitsError(isA<StateError>()));
    await bridge.done;
    observations.close();
  });

  test(
    'cancellation during isolate startup still wakes and joins worker',
    () async {
      final observations = ReceivePort()..listen((_) {});
      var aborts = 0;
      final bridge = StreamBridge(
        producer: producer,
        request: [100, observations.sendPort, 'normal'],
        abort: () => aborts++,
      );
      final subscription = bridge.stream.listen((_) {});
      await subscription.cancel().timeout(const Duration(seconds: 3));
      await bridge.done;
      observations.close();
      expect(aborts, 1);
    },
  );

  test('pre-acceptance failure is delivered once', () async {
    final observations = ReceivePort()..listen((_) {});
    final bridge = StreamBridge(
      producer: producer,
      request: [0, observations.sendPort, 'startup-error'],
      abort: () {},
    );
    final errors = <Object>[];
    await bridge.stream
        .handleError((Object error) => errors.add(error))
        .drain<void>();
    await bridge.done;
    observations.close();
    expect(errors, hasLength(1));
    expect(errors.single, isA<StateError>());
  });

  test('credit reservations have finite item and byte limits', () {
    for (final credits in [0, 257]) {
      expect(
        () => StreamBridge(producer: producer, abort: () {}, credits: credits),
        throwsArgumentError,
      );
    }
    expect(
      () => StreamBridge(producer: producer, abort: () {}, credits: 9),
      throwsArgumentError,
    );
    expect(
      () => StreamBridge(producer: producer, abort: () {}, maxEventBytes: 0),
      throwsArgumentError,
    );
  });
}
