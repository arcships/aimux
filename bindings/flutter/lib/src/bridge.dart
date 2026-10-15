import 'dart:ffi';

import 'package:ffi/ffi.dart';

import 'ffi.g.dart';
import 'native.dart';

typedef Notify = Void Function(Pointer<Utf8>);
typedef ReleaseHandle = Void Function(Uint64);

final class DartBridge {
  final DynamicLibrary library;
  DartBridge(String? path) : library = openLibrary(path, bridge: true);

  late final tokenNew = library
      .lookupFunction<
        Pointer<Void> Function(Uint64, Pointer<NativeFunction<ReleaseHandle>>),
        Pointer<Void> Function(int, Pointer<NativeFunction<ReleaseHandle>>)
      >('aimux_dart_token_new');
  late final tokenDispose = library
      .lookupFunction<
        Void Function(Pointer<Void>),
        void Function(Pointer<Void>)
      >('aimux_dart_token_dispose');
  // Keep finalizers reachable for the isolate lifetime, even if a client and
  // all its forgotten handles become unreachable together.
  static final Map<int, NativeFinalizer> _finalizers = {};
  late final finalizer = _finalizers.putIfAbsent(
    library.lookup<NativeFinalizerFunction>('aimux_dart_finalize').address,
    () => NativeFinalizer(
      library.lookup<NativeFinalizerFunction>('aimux_dart_finalize'),
    ),
  );
  late final create = library
      .lookupFunction<
        Pointer<Void> Function(
          Pointer<NativeFunction<Notify>>,
          Pointer<NativeFunction<ReleaseHandle>>,
          Uint64,
        ),
        Pointer<Void> Function(
          Pointer<NativeFunction<Notify>>,
          Pointer<NativeFunction<ReleaseHandle>>,
          int,
        )
      >('aimux_dart_bridge_new');
  late final ack = _voidPointer('aimux_dart_bridge_ack');
  late final cancel = _voidPointer('aimux_dart_bridge_cancel');
  late final free = _voidPointer('aimux_dart_bridge_free');
  late final frameFree = library
      .lookupFunction<
        Void Function(Pointer<Utf8>),
        void Function(Pointer<Utf8>)
      >('aimux_dart_frame_free');
  late final pending = library
      .lookupFunction<
        Int32 Function(Pointer<Void>),
        int Function(Pointer<Void>)
      >('aimux_dart_bridge_pending');
  late final failure = library
      .lookupFunction<
        Int32 Function(Pointer<Void>),
        int Function(Pointer<Void>)
      >('aimux_dart_bridge_failure');
  late final event = library.lookup<NativeFunction<NativeEvent>>(
    'aimux_dart_bridge_event',
  );

  void Function(Pointer<Void>) _voidPointer(String symbol) =>
      library.lookupFunction<
        Void Function(Pointer<Void>),
        void Function(Pointer<Void>)
      >(symbol);
}
