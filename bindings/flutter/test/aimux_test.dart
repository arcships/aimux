@Timeout(Duration(minutes: 2))
library;

import 'dart:async';
import 'dart:ffi';
import 'dart:io';
import 'dart:isolate';
import 'dart:typed_data';

import 'package:aimux/aimux.dart';
import 'package:aimux/multimodal.dart';
import 'package:aimux/src/bridge.dart';
import 'package:aimux/src/native.dart';
import 'package:test/test.dart';

String fixture([String name = 'ops_fixture']) =>
    '${Directory.current.path}/.dart_tool/native/$name.${Platform.isMacOS ? 'dylib' : 'so'}';

void main() {
  late Aimux client;
  late OpsHandle handle;

  setUp(() async {
    client = await Aimux.open(libraryPath: fixture(), bridgePath: fixture());
    final result = await client.call('test.handle') as Map;
    handle = client.adopt(result['handle'] as String);
    await client.call('test.reset');
  });
  tearDown(() => handle.close());

  Future<Map> stats() async =>
      await client.call('test.stats', handle: handle) as Map;

  test('load validates ABI, manifest and required ops', () async {
    expect(client.manifest['opsVersion'], '1.0');
    await expectLater(
      Aimux.open(libraryPath: fixture('missing_symbol'), bridgePath: fixture()),
      throwsArgumentError,
    );
    await expectLater(
      Aimux.open(
          libraryPath: fixture('missing_finalizer'), bridgePath: fixture()),
      throwsArgumentError,
    );
    await expectLater(
      Aimux.open(libraryPath: fixture('bad_abi'), bridgePath: fixture()),
      throwsStateError,
    );
    await expectLater(
      Aimux.open(libraryPath: fixture('bad_manifest'), bridgePath: fixture()),
      throwsStateError,
    );
    await expectLater(
      Aimux.open(
        libraryPath: fixture(),
        bridgePath: fixture(),
        requiredOps: ['missing.op'],
      ),
      throwsStateError,
    );
    await Aimux.open(
      libraryPath: fixture(),
      bridgePath: fixture(),
      requiredOps: ['runtime.describe', 'text.stream'],
    );
  });

  test('blocking call leaves the caller isolate responsive', () async {
    var ticks = 0;
    final timer = Timer.periodic(
      const Duration(milliseconds: 5),
      (_) => ticks++,
    );
    try {
      expect(await client.call('test.sleep'), {'ok': true});
      expect(ticks, greaterThan(0));
    } finally {
      timer.cancel();
    }
  });

  test(
    'opaque error keeps unknown payload fields and is delivered once',
    () async {
      await expectLater(
        client.call('test.fail'),
        throwsA(
          isA<OpsException>().having((e) => e.code, 'code', 203).having(
            (e) => e.error['futureField'],
            'futureField',
            [1, 2],
          ),
        ),
      );
    },
  );

  test('parameters preserve open JSON, Unicode and escaped NUL', () async {
    final params = {
      'providerOptions': {
        'future': ['中文', '\u0000', null],
      },
    };
    expect(await client.call('test.echo', params: params), params);
    expect(() => client.call('bad\u0000op'), throwsFormatException);
    expect(
      () => client.call('test.echo', params: String.fromCharCode(0xd800)),
      throwsFormatException,
    );
  });

  test('uint64 handles round trip without a pointer-sized token', () async {
    final high = client.adopt('18446744073709551615');
    expect(high.wireId, '18446744073709551615');
    await high.close();
    expect(() => client.adopt('18446744073709551616'), throwsFormatException);
    expect(() => client.adopt('0'), throwsFormatException);
    expect(() => client.adopt('01'), throwsFormatException);
  });

  test(
    'close detaches once, is idempotent and rejects later admission',
    () async {
      final id = handle.wireId;
      expect(() => client.adopt(id), throwsStateError);
      final first = handle.close();
      final second = handle.close();
      expect(identical(first, second), isTrue);
      expect(handle.isClosed, isTrue);
      expect(() => client.call('test.echo', handle: handle), throwsStateError);
      await first;
      // Read the native drop counter without acquiring ownership a second time.
      final native = NativeApi(fixture());
      final json = native.call(parseHandle(id), 'test.stats', '{}', 0) as Map;
      expect(json['drops'], 1);
    },
  );

  test(
    'typed token trampoline queues cleanup off the finalizer thread',
    () async {
      final bridge = DartBridge(fixture());
      final native = NativeApi(fixture());
      final token = bridge.tokenNew(
        4000,
        native.ffi.library.lookup<NativeFunction<ReleaseHandle>>('aimux_drop'),
      );
      final finalize = bridge.library.lookupFunction<
          Void Function(Pointer<Void>),
          void Function(Pointer<Void>)>('aimux_dart_finalize');
      finalize(token);
      Map result = {};
      for (var i = 0; i < 100; i++) {
        result = native.call(4000, 'test.stats', '{}', 0) as Map;
        if (result['drops'] == 1) break;
        await Future<void>.delayed(const Duration(milliseconds: 5));
      }
      expect(result['drops'], 1);
    },
  );

  for (final mode in ['leak', 'close']) {
    test('NativeFinalizer group shutdown versus explicit $mode releases once',
        () async {
      final ready = ReceivePort();
      final exited = ReceivePort();
      final errors = ReceivePort();
      final failure = Completer<void>();
      final errorSubscription = errors.listen((error) {
        if (!failure.isCompleted) failure.completeError(StateError('$error'));
      });
      try {
        final idFuture = ready.first;
        final exitFuture = exited.first;
        await Isolate.spawnUri(
            Uri.file(
                '${Directory.current.path}/test/native/finalizer_isolate.dart'),
            [fixture(), mode],
            ready.sendPort,
            onExit: exited.sendPort,
            onError: errors.sendPort);
        final id = await Future.any([idFuture, failure.future]) as String;
        await Future.any([exitFuture, failure.future]);
        final native = NativeApi(fixture());
        Map result = {};
        for (var i = 0; i < 100; i++) {
          result = native.call(parseHandle(id), 'test.stats', '{}', 0) as Map;
          if (result['drops'] == 1) break;
          await Future<void>.delayed(const Duration(milliseconds: 5));
        }
        expect(result['drops'], 1);
        await Future<void>.delayed(const Duration(milliseconds: 25));
        expect(
            (native.call(parseHandle(id), 'test.stats', '{}', 0)
                as Map)['drops'],
            1);
      } finally {
        ready.close();
        exited.close();
        await errorSubscription.cancel();
        errors.close();
      }
    },
        skip: const bool.fromEnvironment('dart.library.ui')
            ? 'spawnUri isolate groups require the standalone Dart VM'
            : false);
  }

  test(
    'native worker thread delivers lossless ordered frames and one terminal',
    () async {
      final frames =
          await client.stream('test.stream', handle: handle).toList();
      final events = frames.whereType<EventFrame>().toList();
      expect(events.length, 1000);
      expect(
        events.map((f) => (f.event as Map)['text']),
        List.generate(1000, (i) => '$i'),
      );
      expect(frames.whereType<EndFrame>().single.outcome, 'success');
      expect((await stats())['completed'], 1);
    },
  );

  test('pause withholds credit, resume is lossless', () async {
    final first = Completer<void>();
    final done = Completer<void>();
    final frames = <StreamFrame>[];
    late StreamSubscription<StreamFrame> sub;
    sub = client.stream('test.stream', handle: handle).listen(
      (frame) {
        frames.add(frame);
        if (frames.length == 1) {
          sub.pause();
          first.complete();
        }
      },
      onError: done.completeError,
      onDone: done.complete,
    );
    await first.future;
    await Future<void>.delayed(const Duration(milliseconds: 50));
    final before = (await stats())['produced'];
    await Future<void>.delayed(const Duration(milliseconds: 50));
    expect((await stats())['produced'], before);
    // One transport credit plus the next C callback, waiting for credit.
    expect(before, 2);
    sub.resume();
    await done.future;
    expect(frames.whereType<EventFrame>().length, 1000);
    expect(frames.whereType<EndFrame>().length, 1);
  });

  test('cancel wakes producer while credit is exhausted', () async {
    final first = Completer<void>();
    late StreamSubscription<StreamFrame> sub;
    sub = client.stream('test.stream', handle: handle).listen((_) {
      sub.pause();
      first.complete();
    });
    await first.future;
    await sub.cancel().timeout(const Duration(seconds: 5));
    expect((await stats())['completed'], 1);
  });

  test(
    'cancel before native startup finishes cleans up without producing',
    () async {
      final sub = client.stream('test.stream', handle: handle).listen((_) {});
      await sub.cancel().timeout(const Duration(seconds: 5));
      expect((await stats())['produced'], 0);
    },
  );

  test(
    'each subscription has an independent abort and credit bridge',
    () async {
      final counts = await Future.wait(
        List.generate(
          3,
          (_) async =>
              (await client.stream('test.stream', handle: handle).toList())
                  .length,
        ),
      );
      expect(counts, [1001, 1001, 1001]);
    },
  );

  test('pre-acceptance failure has no frame and one error', () async {
    final frames = <StreamFrame>[];
    final errors = <Object>[];
    await client.stream('test.reject', handle: handle).handleError((Object e) {
      errors.add(e);
    }).forEach(frames.add);
    expect(frames, isEmpty);
    expect(
      errors.single,
      isA<OpsException>().having((e) => e.code, 'code', 203),
    );
  });

  test('accepted runtime failure is only in the single EndFrame', () async {
    final frames =
        await client.stream('test.terminalError', handle: handle).toList();
    final end = frames.whereType<EndFrame>().single;
    expect(end.outcome, 'error');
    expect(end.error!.code, 11);
    expect(frames.length, 1001);
  });

  for (final entry in {
    'test.badFrame': 206,
    'test.noTerminal': 206,
    'test.oversize': 305,
  }.entries) {
    test('${entry.key} reports one failure and frees the bridge', () async {
      final errors = <Object>[];
      await client.stream(entry.key, handle: handle).handleError((Object e) {
        errors.add(e);
      }).drain<void>();
      expect(
        errors.single,
        isA<OpsException>().having((e) => e.code, 'code', entry.value),
      );
    });
  }

  test('operation push and typed poll states use the new primitives', () async {
    final operation = TranscriptionOperation(client, handle);
    await operation.push(Uint8List.fromList([1, 2, 3]));
    await operation.push(Uint8List(0));
    expect((await operation.next(timeoutMs: 1)).part, {'type': 'transcript'});
    expect((await operation.next(timeoutMs: 2)).state, 'ended');
    expect((await operation.next()).state, 'timeout');
    await expectLater(
      operation.next(timeoutMs: 99),
      throwsA(isA<OpsException>()),
    );
    expect(
      () => operation.push(Uint8List(1024 * 1024 + 1)),
      throwsA(isA<OpsException>().having((e) => e.code, 'code', 305)),
    );
  });
}
