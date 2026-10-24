import 'dart:convert';
import 'dart:ffi';
import 'dart:io';
import 'dart:isolate';
import 'dart:typed_data';

import 'package:ffi/ffi.dart';

import '../generated/ops.dart' as ops;
import '../errors.dart';
import 'generated/types.dart';
import 'ffi.g.dart';

DynamicLibrary openLibrary(String? path, {bool bridge = false}) {
  if (path != null) return DynamicLibrary.open(path);
  if (Platform.isIOS) return DynamicLibrary.process();
  final name = bridge ? 'aimux_dart' : 'aimux_ffi';
  if (Platform.isAndroid || Platform.isLinux) {
    return DynamicLibrary.open('lib$name.so');
  }
  if (Platform.isMacOS) return DynamicLibrary.open('lib$name.dylib');
  if (Platform.isWindows) return DynamicLibrary.open('$name.dll');
  throw UnsupportedError('Unsupported native platform');
}

// Reject values that dart:ffi would silently truncate or replace.
void checkString(String value, {bool allowNul = false}) {
  final units = value.codeUnits;
  for (var i = 0; i < units.length; i++) {
    final unit = units[i];
    if (unit == 0 && !allowNul)
      throw const FormatException('Interior NUL in native string');
    if (unit >= 0xd800 && unit <= 0xdbff) {
      if (++i >= units.length || units[i] < 0xdc00 || units[i] > 0xdfff) {
        throw const FormatException('Unpaired UTF-16 surrogate');
      }
    } else if (unit >= 0xdc00 && unit <= 0xdfff) {
      throw const FormatException('Unpaired UTF-16 surrogate');
    }
  }
}

void checkJsonValue(Object? value) {
  if (value is String) checkString(value, allowNul: true);
  if (value is List) value.forEach(checkJsonValue);
  if (value is Map) {
    for (final entry in value.entries) {
      if (entry.key is! String)
        throw const FormatException('JSON key is not a string');
      checkJsonValue(entry.key);
      checkJsonValue(entry.value);
    }
  }
}

String encodeParams(Object? value) {
  checkJsonValue(value);
  return jsonEncode(value);
}

int parseHandle(String wire) {
  if (!RegExp(r'^[1-9][0-9]*$').hasMatch(wire)) {
    throw const FormatException('Handle must be a nonzero decimal string');
  }
  final number = BigInt.parse(wire);
  if (number > (BigInt.one << 64) - BigInt.one) {
    throw const FormatException('Handle exceeds uint64');
  }
  return number.toSigned(64).toInt();
}

final class NativeApi {
  final OpsBindings ffi;
  NativeApi(String? path) : ffi = OpsBindings(openLibrary(path));

  String takeString(Pointer<Utf8> value) {
    if (value == nullptr)
      throw const FormatException('Missing native JSON output');
    try {
      return value.toDartString();
    } finally {
      ffi.freeString(value);
    }
  }

  void checkError(Pointer<Void> value) {
    if (value == nullptr) return;
    try {
      throw OpsException.fromJson(
        jsonDecode(takeString(ffi.errorJson(value))) as Map<String, dynamic>,
      );
    } finally {
      ffi.errorFree(value);
    }
  }

  Object? output(Pointer<Void> Function(Pointer<Pointer<Utf8>>) invoke) {
    final out = calloc<Pointer<Utf8>>();
    try {
      checkError(invoke(out));
      final ptr = out.value;
      out.value = nullptr;
      return jsonDecode(takeString(ptr));
    } finally {
      if (out.value != nullptr) ffi.freeString(out.value);
      calloc.free(out);
    }
  }

  Object? call(int handle, String op, String params, int abort) {
    final name = op.toNativeUtf8();
    final json = params.toNativeUtf8();
    try {
      return output((out) => ffi.call(handle, name, json, abort, out));
    } finally {
      calloc.free(name);
      calloc.free(json);
    }
  }
}

// Keep the isolate closure free of UI objects, FFI pointers and Finalizables.
final class NativeRequest {
  final String? libraryPath;
  final String action;
  final int handle, abort, timeout;
  final String op, params;
  final Uint8List? bytes;
  const NativeRequest(
    this.libraryPath,
    this.action, {
    this.handle = 0,
    this.abort = 0,
    this.timeout = 0,
    this.op = '',
    this.params = '{}',
    this.bytes,
  });
}

Future<Object?> execute(NativeRequest request) =>
    Isolate.run(() => _execute(request));

Object? _execute(NativeRequest r) {
  final api = NativeApi(r.libraryPath);
  switch (r.action) {
    case 'load':
      // Lookup all mandatory primitives before accepting the library.
      [
        api.ffi.call,
        api.ffi.stream,
        api.ffi.operationPush,
        api.ffi.operationNextPart,
        api.ffi.abort,
        api.ffi.drop,
        api.ffi.handleFinalizer,
        api.ffi.freeString,
        api.ffi.errorFree,
        api.ffi.errorCode,
        api.ffi.errorMessage,
        api.ffi.errorJson,
      ];
      final version = api.ffi.abiVersion();
      if (version >> 16 != ops.ABI_VERSION >> 16)
        throw StateError('Unsupported aimux ABI: $version');
      final manifest = api.output(api.ffi.manifestJson) as Map<String, dynamic>;
      validateManifest(version, manifest);
      return manifest;
    case 'call':
      return api.call(r.handle, r.op, r.params, r.abort);
    case 'drop':
      api.ffi.drop(r.handle);
      return null;
    case 'push':
      final bytes = r.bytes!;
      final ptr =
          bytes.isEmpty ? nullptr.cast<Uint8>() : calloc<Uint8>(bytes.length);
      try {
        if (bytes.isNotEmpty) ptr.asTypedList(bytes.length).setAll(0, bytes);
        api.checkError(api.ffi.operationPush(r.handle, ptr, bytes.length));
        return null;
      } finally {
        if (ptr != nullptr) calloc.free(ptr);
      }
    case 'poll':
      final out = calloc<Pointer<Utf8>>();
      final state = calloc<Int32>();
      try {
        api.checkError(
          api.ffi.operationNextPart(r.handle, r.timeout, out, state),
        );
        switch (state.value) {
          case 1:
            final ptr = out.value;
            out.value = nullptr;
            return OperationPollResultPart(
                    part_: TranscriptionStreamPart.fromJson(
                        jsonDecode(api.takeString(ptr))))
                .toJson();
          case 2:
          case 3:
            if (out.value != nullptr)
              throw const FormatException('JSON on empty poll');
            return state.value == 2
                ? OperationPollResultEnded().toJson()
                : OperationPollResultTimeout().toJson();
          default:
            throw FormatException('Invalid poll state: ${state.value}');
        }
      } finally {
        if (out.value != nullptr) api.ffi.freeString(out.value);
        calloc.free(out);
        calloc.free(state);
      }
    default:
      throw StateError('Unknown native executor action');
  }
}

void validateManifest(int version, Map<String, dynamic> manifest) {
  final opsVersion = manifest['opsVersion'];
  if (manifest['abiVersion'] != version ||
      opsVersion != ops.OPS_VERSION ||
      manifest['digest'] != ops.MANIFEST_DIGEST ||
      manifest['manifestRevision'] == null ||
      manifest['ops'] is! List ||
      manifest['handleKinds'] is! List ||
      manifest['errorCodes'] is! List) {
    throw StateError(
      'Incompatible aimux manifest: opsVersion $opsVersion '
      '(expected ${ops.OPS_VERSION}), digest ${manifest['digest']} '
      '(expected ${ops.MANIFEST_DIGEST})',
    );
  }
  final names = <String>{};
  for (final op in manifest['ops'] as List) {
    if (op is! Map ||
        op['name'] is! String ||
        !names.add(op['name'] as String)) {
      throw StateError('Invalid aimux operation descriptor');
    }
  }
}
