import 'dart:async';
import 'dart:convert';
import 'dart:ffi';
import 'dart:isolate';
import 'dart:typed_data';

import 'package:ffi/ffi.dart';

import 'errors.dart';
import 'src/bridge.dart';
import 'src/native.dart';
import 'src/stream_bridge.dart';

export 'aimux_plugin.dart';
export 'errors.dart' show OpsException;

/// One version-checked C ABI library. All blocking work runs in worker isolates.
final class Aimux {
  final String? _libraryPath, _bridgePath;
  final Map<String, dynamic> manifest;
  final NativeApi _native;
  final DartBridge _bridge;
  final Map<int, WeakReference<OpsHandle>> _handles = {};

  Aimux._(this._libraryPath, this._bridgePath, this.manifest)
      : _native = NativeApi(_libraryPath),
        _bridge = DartBridge(_bridgePath);

  static Future<Aimux> open({
    String? libraryPath,
    String? bridgePath,
    Iterable<String> requiredOps = const [],
  }) async {
    final manifest = await execute(
      NativeRequest(libraryPath, 'load'),
    ) as Map<String, dynamic>;
    final names =
        (manifest['ops'] as List).map((op) => (op as Map)['name']).toSet();
    final missing = requiredOps.where((op) => !names.contains(op)).toList();
    if (missing.isNotEmpty)
      throw StateError('Missing required aimux ops: $missing');
    final client = Aimux._(libraryPath, bridgePath, manifest);
    // Resolve the bridge entry points at load, rather than at first use.
    client._bridge.finalizer;
    client._bridge.tokenNew;
    client._bridge.tokenDispose;
    client._bridge.create;
    client._bridge.ack;
    client._bridge.cancel;
    client._bridge.free;
    client._bridge.frameFree;
    client._bridge.pending;
    client._bridge.failure;
    client._bridge.event;
    return client;
  }

  /// Transfers ownership of a decimal handle returned by an op to Dart.
  /// Duplicate live wrappers are rejected; close releases the native resource.
  OpsHandle adopt(String wireHandle) {
    _handles.removeWhere((_, reference) => reference.target == null);
    final id = parseHandle(wireHandle);
    if (_handles[id]?.target case final handle? when !handle.isClosed) {
      throw StateError('Handle already has a live owner');
    }
    final handle = OpsHandle._(this, id);
    _handles[id] = WeakReference(handle);
    return handle;
  }

  Future<Object?> call(
    String op, {
    OpsHandle? handle,
    Object? params = const <String, dynamic>{},
    OpsHandle? abort,
  }) {
    checkString(op);
    return execute(
      NativeRequest(
        _libraryPath,
        'call',
        handle: _id(handle),
        abort: _id(abort),
        op: op,
        params: encodeParams(params),
      ),
    );
  }

  /// Raw frames include exactly one terminal after acceptance. A rejected start
  /// is a stream error; terminal failures remain in EndFrame.error, once.
  Stream<StreamFrame> stream(
    String op, {
    required OpsHandle handle,
    Object? params = const <String, dynamic>{},
  }) {
    checkString(op);
    return _StreamPump(this, _id(handle), op, encodeParams(params)).stream;
  }

  /// Nonblocking native signal; independent of stream credit and worker queues.
  void abort(OpsHandle handle) => _native.ffi.abort(_id(handle));

  Future<void> push(OpsHandle operation, Uint8List bytes) {
    final id = _id(operation);
    if (bytes.length > 1024 * 1024) {
      throw resourceLimit('Audio chunk exceeds 1 MiB');
    }
    return execute(
      NativeRequest(
        _libraryPath,
        'push',
        handle: id,
        bytes: Uint8List.fromList(bytes),
      ),
    ).then((_) {});
  }

  Future<PollResult> next(OpsHandle operation, {int timeoutMs = 0}) {
    if (timeoutMs < 0) throw ArgumentError.value(timeoutMs, 'timeoutMs');
    return execute(
      NativeRequest(
        _libraryPath,
        'poll',
        handle: _id(operation),
        timeout: timeoutMs,
      ),
    ).then((value) {
      final json = value as Map<String, dynamic>;
      return PollResult._(json['state'] as String, json['part']);
    });
  }

  int _id(OpsHandle? handle) {
    if (handle == null) return 0;
    if (!identical(handle._client, this)) {
      throw ArgumentError('Handle belongs to a different Aimux instance');
    }
    if (handle.isClosed) throw StateError('Handle is closed');
    return handle._id;
  }
}

/// NativeFinalizer uses an allocated token, preserving all 64 bits on 32-bit
/// targets. Explicit close takes ownership and detaches before scheduling drop.
final class OpsHandle implements Finalizable {
  final Aimux _client;
  final Object _detachKey = Object();
  late final Pointer<Void> _token;
  int _id;
  Future<void>? _closing;

  OpsHandle._(this._client, this._id) {
    _token = _client._bridge.tokenNew(
      _id,
      _client._native.ffi.library.lookup<NativeFunction<ReleaseHandle>>(
        'aimux_drop',
      ),
    );
    if (_token == nullptr)
      throw StateError('Could not allocate finalizer token');
    _client._bridge.finalizer.attach(this, _token, detach: _detachKey);
  }

  bool get isClosed => _id == 0;
  String get wireId {
    if (isClosed) throw StateError('Handle is closed');
    return BigInt.from(_id).toUnsigned(64).toString();
  }

  Future<void> close() {
    if (_closing != null) return _closing!;
    final id = _id;
    _id = 0;
    _client._handles.remove(id);
    _client._bridge.finalizer.detach(_detachKey);
    _client._bridge.tokenDispose(_token);
    _closing = execute(NativeRequest(_client._libraryPath, 'drop', handle: id))
        .then((_) {});
    return _closing!;
  }
}

final class PollResult {
  final String state;
  final Object? part;
  const PollResult._(this.state, this.part);
}

sealed class StreamFrame {
  final String seq;
  const StreamFrame(this.seq);
  factory StreamFrame.fromJson(Map<String, dynamic> json) {
    final seq = json['seq'];
    if (seq is! String ||
        seq.length > 20 ||
        !RegExp(r'^(0|[1-9][0-9]*)$').hasMatch(seq) ||
        BigInt.parse(seq) > (BigInt.one << 64) - BigInt.one) {
      throw const FormatException('Invalid stream sequence');
    }
    switch (json['type']) {
      case 'event':
        if (!json.containsKey('event'))
          throw const FormatException('Missing event');
        return EventFrame(seq, json['event']);
      case 'end':
        final outcome = json['outcome'];
        if (!const ['success', 'error', 'cancelled'].contains(outcome)) {
          throw const FormatException('Invalid stream outcome');
        }
        final error = json['error'] == null
            ? null
            : OpsException.fromJson(json['error'] as Map<String, dynamic>);
        if (outcome == 'error' && error == null ||
            outcome == 'success' && error != null) {
          throw const FormatException('Invalid terminal error');
        }
        return EndFrame(seq, outcome as String, error);
      default:
        throw const FormatException('Unknown stream frame');
    }
  }
}

final class EventFrame extends StreamFrame {
  final Object? event;
  const EventFrame(super.seq, this.event);
}

final class EndFrame extends StreamFrame {
  final String outcome;
  final OpsException? error;
  const EndFrame(super.seq, this.outcome, this.error);
}

final class _StreamRequest {
  final String? libraryPath, bridgePath;
  final int handle, abort, bridgeAddress;
  final String op, params;
  const _StreamRequest(
    this.libraryPath,
    this.bridgePath,
    this.handle,
    this.abort,
    this.bridgeAddress,
    this.op,
    this.params,
  );
}

Future<void> _runStream(_StreamRequest request) => Isolate.run(() {
      final api = NativeApi(request.libraryPath);
      final bridge = DartBridge(request.bridgePath);
      final op = request.op.toNativeUtf8();
      final params = request.params.toNativeUtf8();
      try {
        api.checkError(
          api.ffi.stream(
            request.handle,
            op,
            params,
            request.abort,
            bridge.event,
            Pointer<Void>.fromAddress(request.bridgeAddress),
          ),
        );
      } finally {
        calloc.free(op);
        calloc.free(params);
      }
    });

// The listener isolate stays responsive while the C call runs separately.
BlockingStreamProducer _streamProducer(Object? request) =>
    _NativeStreamProducer(request! as _StreamRequest);

final class _NativeStreamProducer implements BlockingStreamProducer {
  final _StreamRequest request;
  _NativeStreamProducer(this.request);

  @override
  Future<void> run(StreamBridgeSink sink) async {
    final api = NativeApi(request.libraryPath);
    final bridge = DartBridge(request.bridgePath);
    Pointer<Void> context = nullptr;
    Object? failure;
    StackTrace? failureStack;
    String? terminal;
    BigInt? lastSeq;
    bool cancelled = false;
    final drained = Completer<void>();
    bool producerDone = false;
    void receive(Pointer<Utf8> pointer) {
      try {
        if (failure != null || cancelled) return;
        final json = pointer.toDartString();
        final frame = StreamFrame.fromJson(
          jsonDecode(json) as Map<String, dynamic>,
        );
        final seq = BigInt.parse(frame.seq);
        if (terminal != null || lastSeq != null && seq <= lastSeq!) {
          throw const FormatException('Invalid stream frame order');
        }
        lastSeq = seq;
        if (frame is EndFrame) {
          terminal = json;
        } else if (!sink.add(json)) {
          cancelled = true;
          bridge.cancel(context);
        }
      } catch (error, stack) {
        failure ??= bindingFailure('Could not deliver native stream: $error');
        failureStack ??= stack;
        bridge.cancel(context);
      } finally {
        bridge.frameFree(pointer);
        bridge.ack(context);
        if (producerDone && !drained.isCompleted) drained.complete();
      }
    }

    final listener = NativeCallable<Notify>.listener(receive);
    try {
      context = bridge.create(
        listener.nativeFunction,
        api.ffi.library.lookup<NativeFunction<ReleaseHandle>>('aimux_abort'),
        request.abort,
      );
      if (context == nullptr)
        throw StateError('Could not allocate stream bridge');
      try {
        await _runStream(_StreamRequest(
          request.libraryPath,
          request.bridgePath,
          request.handle,
          request.abort,
          context.address,
          request.op,
          request.params,
        ));
      } finally {
        producerDone = true;
        if (bridge.pending(context) != 0) await drained.future;
      }
      final code = bridge.failure(context);
      if (code != 0) {
        failure ??= code == 305
            ? resourceLimit('Stream frame exceeds 1 MiB')
            : bindingFailure('Native stream bridge could not copy frame');
      }
      if (failure != null)
        Error.throwWithStackTrace(failure!, failureStack ?? StackTrace.current);
      if (terminal != null) {
        sink.endFrame(terminal!);
      } else if (cancelled) {
        sink.end();
      } else {
        throw bindingFailure('Native stream ended without terminal');
      }
    } finally {
      if (context != nullptr) bridge.free(context);
      listener.close();
    }
  }
}

final class _StreamPump {
  final Aimux client;
  final int handle;
  final String op, params;
  _StreamPump(this.client, this.handle, this.op, this.params);

  Stream<StreamFrame> get stream {
    OpsHandle? abort;
    StreamBridge? bridge;
    bool cancelled = false;
    late final StreamController<StreamFrame> controller;
    StreamSubscription<String>? subscription;
    final finished = Completer<void>();
    Future<void> start() async {
      try {
        final requestParams = jsonDecode(params);
        final suppliedAbort =
            requestParams is Map ? requestParams['abort'] : null;
        late final int abortId;
        if (suppliedAbort != null) {
          if (suppliedAbort is! String) {
            throw const FormatException(
                'Abort must be a decimal handle string');
          }
          abortId = parseHandle(suppliedAbort);
        } else {
          final result =
              await client.call('abort.create') as Map<String, dynamic>;
          abort = client.adopt(result['handle'] as String);
          abortId = client._id(abort);
        }
        if (cancelled) return;
        bridge = StreamBridge(
          producer: _streamProducer,
          request: _StreamRequest(client._libraryPath, client._bridgePath,
              handle, abortId, 0, op, params),
          abort: () => client._native.ffi.abort(abortId),
          credits: 1,
        );
        subscription = bridge!.stream.listen(
          (json) => controller.add(
              StreamFrame.fromJson(jsonDecode(json) as Map<String, dynamic>)),
          onError: (Object error, StackTrace stack) =>
              controller.addError(error, stack),
          onDone: () => unawaited(controller.close()),
        );
        if (controller.isPaused) subscription!.pause();
        await bridge!.done;
      } catch (error, stack) {
        if (!cancelled) {
          controller.addError(error, stack);
          unawaited(controller.close());
        }
      } finally {
        try {
          await abort?.close();
        } finally {
          if (!finished.isCompleted) finished.complete();
        }
      }
    }

    controller = StreamController<StreamFrame>(
      sync: true,
      onListen: () => unawaited(start()),
      onPause: () => subscription?.pause(),
      onResume: () => subscription?.resume(),
      onCancel: () async {
        cancelled = true;
        await subscription?.cancel();
        await finished.future;
      },
    );
    return controller.stream;
  }
}
