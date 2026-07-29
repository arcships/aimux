// aimux.dart — Flutter/Dart binding for aimux (dart:ffi, C ABI path).
//
// Calls aimux-ffi's C ABI directly via dart:ffi. No flutter_rust_bridge,
// no codegen step. The native library (libaimux_ffi.so / .dylib / .dll)
// must be on the library path or bundled in the Flutter app.
//
// This is the C ABI path (RFC §3.2) — same as Swift and Kotlin.

import 'dart:async';
import 'dart:convert';
import 'dart:ffi';
import 'dart:io';
import 'package:ffi/ffi.dart';

// ─────────────────────────────────────────────────────────────────────────────
// FFI type aliases
// ─────────────────────────────────────────────────────────────────────────────

typedef _OpenaiNewC = Uint64 Function(Pointer<Utf8> apiKey, Pointer<Utf8> modelId);
typedef _OpenaiNewDart = int Function(Pointer<Utf8> apiKey, Pointer<Utf8> modelId);

// Constructors with a custom base URL (aimux_*_new_with_base). The same
// signature covers both OpenAI and Anthropic.
typedef _NewWithBaseC = Uint64 Function(
    Pointer<Utf8> apiKey, Pointer<Utf8> modelId, Pointer<Utf8> baseUrl);
typedef _NewWithBaseDart = int Function(
    Pointer<Utf8> apiKey, Pointer<Utf8> modelId, Pointer<Utf8> baseUrl);

typedef _GenerateTextC = Pointer<Utf8> Function(
    Uint64 handle, Pointer<Utf8> promptJson, Pointer<Utf8>? optsJson);
typedef _GenerateTextDart = Pointer<Utf8> Function(
    int handle, Pointer<Utf8> promptJson, Pointer<Utf8>? optsJson);

typedef _DropHandleC = Void Function(Uint64);
typedef _DropHandleDart = void Function(int);

typedef _FreeStringC = Void Function(Pointer<Utf8>);
typedef _FreeStringDart = void Function(Pointer<Utf8>);

// Stream callback C signatures
typedef _OnPartC = Void Function(Pointer<Utf8>);
typedef _OnDoneC = Void Function();
typedef _OnErrorC = Void Function(Pointer<Utf8>);

// ─────────────────────────────────────────────────────────────────────────────
// FFI library wrapper
// ─────────────────────────────────────────────────────────────────────────────

final class _AimuxFFI {
  final int Function(Pointer<Utf8>, Pointer<Utf8>) openaiNew;
  final int Function(Pointer<Utf8>, Pointer<Utf8>) anthropicNew;
  final int Function(Pointer<Utf8>, Pointer<Utf8>, Pointer<Utf8>) openaiNewWithBase;
  final int Function(Pointer<Utf8>, Pointer<Utf8>, Pointer<Utf8>) anthropicNewWithBase;
  final Pointer<Utf8> Function(int, Pointer<Utf8>, Pointer<Utf8>?) generateText;
  final void Function(int, Pointer<Utf8>, Pointer<Utf8>?,
      Pointer<NativeFunction<_OnPartC>>, Pointer<NativeFunction<_OnDoneC>>,
      Pointer<NativeFunction<_OnErrorC>>) streamText;
  final void Function(int) dropHandle;
  final void Function(Pointer<Utf8>) freeString;

  _AimuxFFI._(
      this.openaiNew,
      this.anthropicNew,
      this.openaiNewWithBase,
      this.anthropicNewWithBase,
      this.generateText,
      this.streamText,
      this.dropHandle,
      this.freeString);

  factory _AimuxFFI() {
    final libName = _platformLibName();
    final dylib = DynamicLibrary.open(libName);

    return _AimuxFFI._(
      dylib.lookupFunction<_OpenaiNewC, _OpenaiNewDart>('aimux_openai_new'),
      dylib.lookupFunction<_OpenaiNewC, _OpenaiNewDart>('aimux_anthropic_new'),
      dylib.lookupFunction<_NewWithBaseC, _NewWithBaseDart>('aimux_openai_new_with_base'),
      dylib.lookupFunction<_NewWithBaseC, _NewWithBaseDart>('aimux_anthropic_new_with_base'),
      dylib.lookupFunction<_GenerateTextC, _GenerateTextDart>('aimux_generate_text'),
      dylib.lookupFunction<
          _StreamTextC,
          _StreamTextDart>('aimux_stream_text'),
      dylib.lookupFunction<_DropHandleC, _DropHandleDart>('aimux_drop_handle'),
      dylib.lookupFunction<_FreeStringC, _FreeStringDart>('aimux_free_string'),
    );
  }

  static String _platformLibName() {
    if (Platform.isLinux) return 'libaimux_ffi.so';
    if (Platform.isMacOS) return 'libaimux_ffi.dylib';
    if (Platform.isWindows) return 'aimux_ffi.dll';
    throw UnsupportedError('Unsupported platform');
  }
}

// Stream ABI type (can't use typedef with Pointer<NativeFunction> inline easily)
typedef _StreamTextC = Void Function(
    Uint64,
    Pointer<Utf8>,
    Pointer<Utf8>?,
    Pointer<NativeFunction<_OnPartC>>,
    Pointer<NativeFunction<_OnDoneC>>,
    Pointer<NativeFunction<_OnErrorC>>);
typedef _StreamTextDart = void Function(
    int,
    Pointer<Utf8>,
    Pointer<Utf8>?,
    Pointer<NativeFunction<_OnPartC>>,
    Pointer<NativeFunction<_OnDoneC>>,
    Pointer<NativeFunction<_OnErrorC>>);

// ─────────────────────────────────────────────────────────────────────────────
// Stream callback trampolines (global — safe because aimux_stream_text
// is synchronous, blocks the caller until done).
// ─────────────────────────────────────────────────────────────────────────────

StreamController<Map<String, dynamic>>? _currentController;
String? _currentError;

void _onPart(Pointer<Utf8> jsonPtr) {
  if (_currentController != null && jsonPtr != nullptr) {
    final json = jsonPtr.toDartString();
    _currentController!.add(jsonDecode(json) as Map<String, dynamic>);
  }
}

void _onDone() {
  _currentController?.close();
}

void _onError(Pointer<Utf8> errPtr) {
  if (errPtr != nullptr) {
    _currentError = errPtr.toDartString();
  }
  _currentController?.addError(StateError(_currentError ?? 'stream error'));
  _currentController?.close();
}

// ─────────────────────────────────────────────────────────────────────────────
// Model
// ─────────────────────────────────────────────────────────────────────────────

/// A model instance backed by a Rust `Arc<dyn LanguageModel>`.
///
/// Call [close] to release the native handle.
class Model {
  final int _handle;
  final _AimuxFFI _ffi;
  bool _closed = false;

  Model._(this._handle, this._ffi);

  /// Create an OpenAI model instance.
  ///
  /// Pass [baseUrl] to target an OpenAI-compatible endpoint (Azure, Groq,
  /// a local mock server, etc.). When null, the provider's standard URL is
  /// used via `aimux_openai_new`.
  factory Model.openai(String apiKey, String modelId, {String? baseUrl}) {
    final ffi = _AimuxFFI();
    final h = _withUtf8(apiKey, (keyPtr) {
      return _withUtf8(modelId, (idPtr) {
        if (baseUrl == null) {
          return ffi.openaiNew(keyPtr, idPtr);
        }
        return _withUtf8(baseUrl,
            (basePtr) => ffi.openaiNewWithBase(keyPtr, idPtr, basePtr));
      });
    });
    if (h == 0) throw StateError('Failed to create OpenAI model');
    return Model._(h, ffi);
  }

  /// Create an Anthropic model instance.
  ///
  /// Pass [baseUrl] to target a custom Anthropic-compatible endpoint. When
  /// null, the provider's standard URL is used via `aimux_anthropic_new`.
  factory Model.anthropic(String apiKey, String modelId, {String? baseUrl}) {
    final ffi = _AimuxFFI();
    final h = _withUtf8(apiKey, (keyPtr) {
      return _withUtf8(modelId, (idPtr) {
        if (baseUrl == null) {
          return ffi.anthropicNew(keyPtr, idPtr);
        }
        return _withUtf8(baseUrl,
            (basePtr) => ffi.anthropicNewWithBase(keyPtr, idPtr, basePtr));
      });
    });
    if (h == 0) throw StateError('Failed to create Anthropic model');
    return Model._(h, ffi);
  }

  /// Generate text (non-streaming).
  ///
  /// [prompt] — a string or a list of message maps.
  /// [options] — optional generation options map.
  /// Returns the parsed GenerateTextResult as a Map.
  Map<String, dynamic> generateText(
    Object prompt, [
    Map<String, dynamic>? options,
  ]) {
    _checkOpen();
    final promptJson = _promptToJson(prompt);
    final optsJson = options != null ? jsonEncode(options) : null;

    final resultStr = _withUtf8(promptJson, (promptPtr) {
      final optsPtr = optsJson != null ? optsJson.toNativeUtf8() : nullptr;
      try {
        final resultPtr = _ffi.generateText(_handle, promptPtr, optsPtr);
        if (resultPtr == nullptr) throw StateError('generate_text returned null');
        final result = resultPtr.toDartString();
        _ffi.freeString(resultPtr);
        return result;
      } finally {
        if (optsPtr != nullptr) calloc.free(optsPtr);
      }
    });

    final result = jsonDecode(resultStr) as Map<String, dynamic>;
    if (result.containsKey('error')) {
      throw StateError(result['error'] as String);
    }
    return result;
  }

  /// Stream text from the model.
  ///
  /// Returns a Stream of StreamPart maps. Blocks the current isolate
  /// until the stream completes.
  Stream<Map<String, dynamic>> streamText(
    Object prompt, [
    Map<String, dynamic>? options,
  ]) {
    _checkOpen();
    final promptJson = _promptToJson(prompt);
    final optsJson = options != null ? jsonEncode(options) : null;

    final controller = StreamController<Map<String, dynamic>>();

    _withUtf8(promptJson, (promptPtr) {
      final optsPtr = optsJson != null ? optsJson.toNativeUtf8() : nullptr;
      try {
        final partCb = Pointer.fromFunction<_OnPartC>(_onPart);
        final doneCb = Pointer.fromFunction<_OnDoneC>(_onDone);
        final errCb = Pointer.fromFunction<_OnErrorC>(_onError);

        _currentController = controller;
        _currentError = null;

        _ffi.streamText(_handle, promptPtr, optsPtr, partCb, doneCb, errCb);

        _currentController = null;
      } finally {
        if (optsPtr != nullptr) calloc.free(optsPtr);
      }
    });

    return controller.stream;
  }

  /// Release the native handle. Safe to call multiple times.
  void close() {
    if (!_closed) {
      _ffi.dropHandle(_handle);
      _closed = true;
    }
  }

  void _checkOpen() {
    if (_closed) throw StateError('Model has been closed');
  }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

String _promptToJson(Object prompt) {
  if (prompt is String) return jsonEncode(prompt);
  return jsonEncode({'prompt': prompt});
}

T _withUtf8<T>(String s, T Function(Pointer<Utf8>) fn) {
  final ptr = s.toNativeUtf8();
  try {
    return fn(ptr);
  } finally {
    calloc.free(ptr);
  }
}
