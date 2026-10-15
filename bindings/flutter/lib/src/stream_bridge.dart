import 'dart:async';
import 'dart:convert';
import 'dart:ffi';
import 'dart:io';
import 'dart:isolate';

import 'package:ffi/ffi.dart';

/// A sendable producer constructed in the worker isolate. [run] may block in
/// aimux_stream. Its synchronous native callback calls [StreamBridgeSink.add]
/// for event JSON and [StreamBridgeSink.end] for the terminal frame. Errors
/// returned before native acceptance may be thrown from [run].
///
/// Callback exceptions must be caught inside the FFI callback: call
/// [StreamBridgeSink.fail] and abort native work, without unwinding across C.
abstract class BlockingStreamProducer {
  FutureOr<void> run(StreamBridgeSink sink);
}

typedef StreamProducerFactory = BlockingStreamProducer Function(
  Object? request,
);

/// Lossless single-subscription bridge for the synchronous C stream ABI.
///
/// The factory and request must be isolate-sendable. [abort] runs in the caller
/// isolate and must be a nonblocking aimux_abort wrapper. Cancellation completes
/// only after the worker returns, keeping callback resources alive until then.
/// Each credit reserves [maxEventBytes], so outstanding data never exceeds
/// [credits] items or 8 MiB. Terminal messages do not consume credits.
///
/// Uses a POSIX pipe on the binding's Android/iOS targets (also Linux/macOS).
/// Unlike a ReceivePort, the pipe can wake a synchronous native callback while
/// the worker isolate's event loop is blocked.
class StreamBridge {
  StreamBridge({
    required StreamProducerFactory producer,
    Object? request,
    required void Function() abort,
    int credits = 8,
    int maxEventBytes = 1024 * 1024,
  })  : _producer = producer,
        _request = request,
        _abort = abort,
        _credits = credits,
        _maxEventBytes = maxEventBytes {
    if (credits < 1 || credits > 256) {
      throw ArgumentError.value(credits, 'credits', 'must be in 1..256');
    }
    if (maxEventBytes < 1 ||
        maxEventBytes > 1024 * 1024 ||
        credits * maxEventBytes > 8 * 1024 * 1024) {
      throw ArgumentError.value(
        maxEventBytes,
        'maxEventBytes',
        'must be in 1..1 MiB with total credit reservation <= 8 MiB',
      );
    }
    _controller = StreamController<String>(
      sync: true,
      onListen: _start,
      onPause: () => _paused = true,
      onResume: _resume,
      onCancel: _cancel,
    );
  }

  final StreamProducerFactory _producer;
  final Object? _request;
  final void Function() _abort;
  final int _credits;
  final int _maxEventBytes;
  late final StreamController<String> _controller;
  final Completer<void> _stopped = Completer<void>();
  _CreditPipe? _pipe;
  ReceivePort? _messages;
  bool _paused = false;
  bool _cancelled = false;
  bool _terminal = false;
  bool _abortSent = false;
  int _pendingAcks = 0;

  Stream<String> get stream => _controller.stream;

  /// Completes after worker exit and descriptor cleanup, including cancellation.
  Future<void> get done => _stopped.future;

  Future<void> _start() async {
    try {
      _pipe = _CreditPipe.create();
      _messages = ReceivePort();
      _messages!.listen(_message);
      // Share one mailbox for event, terminal, failure and exit messages so
      // cleanup cannot overtake accepted events.
      await Isolate.spawn<_WorkerRequest>(
        _runWorker,
        _WorkerRequest(
          _producer,
          _request,
          _pipe!.readFd,
          _maxEventBytes,
          _messages!.sendPort,
        ),
        onExit: _messages!.sendPort,
        onError: _messages!.sendPort,
        errorsAreFatal: true,
      );
      if (_stopped.isCompleted) return;
      if (_cancelled) {
        _pipe!.send(0);
      } else {
        for (var i = 0; i < _credits; i++) {
          _pipe!.send(1);
        }
      }
    } catch (error, stack) {
      _finish(error, stack);
      _cleanup();
    }
  }

  void _message(dynamic message) {
    if (message == null) {
      if (!_terminal && !_cancelled) {
        _finish(StateError('Stream worker exited without a terminal frame'));
      }
      _cleanup();
    } else if (message is _WorkerEvent) {
      if (_cancelled || _terminal) return;
      _controller.add(message.json);
      // A synchronous listener may pause or cancel while processing this event.
      if (!_cancelled && !message.terminal) {
        if (_paused) {
          _pendingAcks++;
        } else {
          _pipe!.send(1);
        }
      }
    } else if (message is _WorkerEnd) {
      _finish(
        message.error,
        message.stack == null ? null : StackTrace.fromString(message.stack!),
      );
    } else if (message is List) {
      _finish(RemoteError('${message[0]}', '${message[1]}'));
    }
  }

  void _resume() {
    _paused = false;
    if (_cancelled || _terminal) return;
    while (_pendingAcks > 0) {
      _pendingAcks--;
      _pipe!.send(1);
    }
  }

  Future<void> _cancel() {
    if (!_terminal && !_cancelled) {
      _cancelled = true;
      try {
        if (!_abortSent) {
          _abortSent = true;
          _abort();
        }
      } finally {
        // Independent of credit count: wake a callback blocked in read().
        _pipe?.send(0);
      }
    }
    return done;
  }

  void _finish(Object? error, [StackTrace? stack]) {
    if (_terminal || _cancelled) return;
    _terminal = true;
    if (error != null) _controller.addError(error, stack);
    unawaited(_controller.close());
  }

  void _cleanup() {
    _messages?.close();
    _pipe?.close();
    _pipe = null;
    if (!_stopped.isCompleted) _stopped.complete();
  }
}

/// Synchronous worker-side callback sink. A false [add] result means the
/// consumer cancelled; the native adapter should stop submitting events.
class StreamBridgeSink {
  StreamBridgeSink._(this._port, this._readFd, this._maxEventBytes);

  final SendPort _port;
  final int _readFd;
  final int _maxEventBytes;
  bool _terminal = false;
  bool _cancelled = false;

  bool add(String eventJson) {
    if (_terminal || _cancelled) return false;
    if (utf8.encode(eventJson).length > _maxEventBytes) {
      throw StateError(
        'ResourceLimit: stream event exceeds $_maxEventBytes bytes',
      );
    }
    if (_CreditPipe.receive(_readFd) == 0) {
      _cancelled = true;
      return false;
    }
    _port.send(_WorkerEvent(eventJson));
    return true;
  }

  /// Deliver one terminal wire frame using the independent terminal slot.
  void endFrame(String json) {
    if (_terminal) return;
    if (utf8.encode(json).length > _maxEventBytes) {
      throw StateError(
          'ResourceLimit: terminal frame exceeds $_maxEventBytes bytes');
    }
    _port.send(_WorkerEvent(json, terminal: true));
    end();
  }

  /// Call for end(success/cancelled), or pass the original error envelope for
  /// end(error). A data part named "error" must still be passed to [add].
  void end([Object? error]) {
    if (_terminal) return;
    _terminal = true;
    _port.send(_WorkerEnd(error, null));
  }

  void fail(Object error, [StackTrace? stack]) {
    if (_terminal) return;
    _terminal = true;
    _port.send(_WorkerEnd(error, stack?.toString()));
  }
}

class _WorkerRequest {
  const _WorkerRequest(
    this.factory,
    this.request,
    this.readFd,
    this.maxEventBytes,
    this.port,
  );
  final StreamProducerFactory factory;
  final Object? request;
  final int readFd;
  final int maxEventBytes;
  final SendPort port;
}

class _WorkerEvent {
  const _WorkerEvent(this.json, {this.terminal = false});
  final String json;
  final bool terminal;
}

class _WorkerEnd {
  const _WorkerEnd(this.error, this.stack);
  final Object? error;
  final String? stack;
}

Future<void> _runWorker(_WorkerRequest request) async {
  final sink = StreamBridgeSink._(
    request.port,
    request.readFd,
    request.maxEventBytes,
  );
  try {
    await request.factory(request.request).run(sink);
    if (!sink._terminal) {
      sink.fail(StateError('Native stream returned without a terminal frame'));
    }
  } catch (error, stack) {
    sink.fail(error, stack);
  }
}

// File descriptors belong to the process, so only the main isolate closes
// them, after worker exit. Keeping the read end open also prevents SIGPIPE
// when cancellation races the final callback.
class _CreditPipe {
  _CreditPipe(this.readFd, this.writeFd);
  final int readFd;
  final int writeFd;

  static final DynamicLibrary _lib = Platform.isWindows
      ? throw UnsupportedError('StreamBridge requires POSIX pipes')
      : DynamicLibrary.process();
  static final _pipe = _lib.lookupFunction<Int32 Function(Pointer<Int32>),
      int Function(Pointer<Int32>)>('pipe');
  static final _read = _lib.lookupFunction<
      IntPtr Function(Int32, Pointer<Uint8>, IntPtr),
      int Function(int, Pointer<Uint8>, int)>('read');
  static final _write = _lib.lookupFunction<
      IntPtr Function(Int32, Pointer<Uint8>, IntPtr),
      int Function(int, Pointer<Uint8>, int)>('write');
  static final _close =
      _lib.lookupFunction<Int32 Function(Int32), int Function(int)>('close');
  static final _errno =
      _lib.lookupFunction<Pointer<Int32> Function(), Pointer<Int32> Function()>(
    Platform.isMacOS || Platform.isIOS
        ? '__error'
        : (Platform.isAndroid ? '__errno' : '__errno_location'),
  );

  static _CreditPipe create() {
    final fds = calloc<Int32>(2);
    try {
      if (_pipe(fds) != 0) throw StateError('Cannot create stream credit pipe');
      return _CreditPipe(fds[0], fds[1]);
    } finally {
      calloc.free(fds);
    }
  }

  void send(int token) {
    final byte = calloc<Uint8>();
    try {
      byte.value = token;
      int count;
      do {
        count = _write(writeFd, byte, 1);
      } while (count < 0 && _errno().value == 4); // EINTR
      if (count != 1) throw StateError('Cannot acknowledge stream credit');
    } finally {
      calloc.free(byte);
    }
  }

  static int receive(int fd) {
    final byte = calloc<Uint8>();
    try {
      int count;
      do {
        count = _read(fd, byte, 1);
      } while (count < 0 && _errno().value == 4);
      if (count != 1) throw StateError('Stream credit pipe closed');
      return byte.value;
    } finally {
      calloc.free(byte);
    }
  }

  void close() {
    _close(readFd);
    _close(writeFd);
  }
}
