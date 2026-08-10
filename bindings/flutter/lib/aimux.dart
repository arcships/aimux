// aimux.dart — Flutter/Dart binding for aimux (dart:ffi, C ABI path).
//
// Calls aimux-ffi's C ABI directly via dart:ffi. No flutter_rust_bridge,
// no codegen step. The native library (libaimux_ffi.so / .dylib / .dll)
// must be on the library path or bundled in the Flutter app.
//
// This is the C ABI path (RFC §3.2) — same as Swift and Kotlin.
//
// Error transport: fallible calls take a trailing AimuxError *err (see
// errors.dart). Success returns handle > 0 / non-null char* / stream != 0.
// Failure returns 0 / NULL / 0 and fills *err → AimuxException.fromC.

// The Flutter tool resolves `dartPluginClass` from the package's main
// library, so the plugin registration class must be visible here.
export 'aimux_plugin.dart';
export 'errors.dart'
    hide
        openAimuxLibrary,
        aimuxFreeString,
        withUtf8,
        takeHandle,
        takeString,
        construct2;

import 'dart:async';
import 'dart:convert';
import 'dart:ffi';
import 'package:ffi/ffi.dart';

import 'errors.dart';

// ─────────────────────────────────────────────────────────────────────────────
// FFI type aliases (AimuxError *err trailing out-param)
// ─────────────────────────────────────────────────────────────────────────────

typedef _NewC = Uint64 Function(
    Pointer<Utf8> apiKey, Pointer<Utf8> modelId, Pointer<AimuxCError> err);
typedef _NewDart = int Function(
    Pointer<Utf8> apiKey, Pointer<Utf8> modelId, Pointer<AimuxCError> err);

// Constructors with a custom base URL (aimux_*_new_with_base).
typedef _NewWithBaseC = Uint64 Function(Pointer<Utf8> apiKey,
    Pointer<Utf8> modelId, Pointer<Utf8> baseUrl, Pointer<AimuxCError> err);
typedef _NewWithBaseDart = int Function(Pointer<Utf8> apiKey,
    Pointer<Utf8> modelId, Pointer<Utf8> baseUrl, Pointer<AimuxCError> err);

// Registry provider constructor (aimux_provider_new, RFC-0017 phase 4).
typedef _ProviderNewC = Uint64 Function(
    Pointer<Utf8> name,
    Pointer<Utf8>? apiKey,
    Pointer<Utf8> modelId,
    Pointer<Utf8>? configJson,
    Pointer<AimuxCError> err);
typedef _ProviderNewDart = int Function(
    Pointer<Utf8> name,
    Pointer<Utf8>? apiKey,
    Pointer<Utf8> modelId,
    Pointer<Utf8>? configJson,
    Pointer<AimuxCError> err);

// Provider handle constructor (aimux_provider_handle_new, RFC-0027).
typedef _ProviderHandleNewC = Uint64 Function(Pointer<Utf8> name,
    Pointer<Utf8>? apiKey, Pointer<Utf8>? configJson, Pointer<AimuxCError> err);
typedef _ProviderHandleNewDart = int Function(Pointer<Utf8> name,
    Pointer<Utf8>? apiKey, Pointer<Utf8>? configJson, Pointer<AimuxCError> err);

// Provider handle list_models / model (RFC-0027).
typedef _ProviderListModelsC = Pointer<Utf8> Function(
    Uint64 handle, Pointer<AimuxCError> err);
typedef _ProviderListModelsDart = Pointer<Utf8> Function(
    int handle, Pointer<AimuxCError> err);

typedef _ProviderModelC = Uint64 Function(
    Uint64 handle, Pointer<Utf8> modelId, Pointer<AimuxCError> err);
typedef _ProviderModelDart = int Function(
    int handle, Pointer<Utf8> modelId, Pointer<AimuxCError> err);

typedef _GetModelSpecsC = Pointer<Utf8> Function(
    Pointer<Utf8>? sourceUrl, Pointer<AimuxCError> err);
typedef _GetModelSpecsDart = Pointer<Utf8> Function(
    Pointer<Utf8>? sourceUrl, Pointer<AimuxCError> err);

typedef _GenerateTextC = Pointer<Utf8> Function(Uint64 handle,
    Pointer<Utf8> promptJson, Pointer<Utf8>? optsJson, Pointer<AimuxCError> err);
typedef _GenerateTextDart = Pointer<Utf8> Function(int handle,
    Pointer<Utf8> promptJson, Pointer<Utf8>? optsJson, Pointer<AimuxCError> err);

typedef _DropHandleC = Void Function(Uint64);
typedef _DropHandleDart = void Function(int);

typedef _InitLoggingC = Void Function(Pointer<Utf8> level);
typedef _InitLoggingDart = void Function(Pointer<Utf8> level);

// Recording + mock replay (RFC-0023). int-returning functions return 0 on
// success or -1 on invalid input; mock_replay_new returns uint64_t handle.
typedef _RecordingDirC = Int32 Function(Pointer<Utf8> dir);
typedef _RecordingDirDart = int Function(Pointer<Utf8> dir);
typedef _RecordingRingC = Int32 Function(Uint64 cap);
typedef _RecordingRingDart = int Function(int cap);
typedef _RecordingNoArgC = Int32 Function();
typedef _RecordingNoArgDart = int Function();
typedef _MockReplayC = Uint64 Function(
    Pointer<Utf8> recordingsJsonl, Pointer<AimuxCError> err);
typedef _MockReplayDart = int Function(
    Pointer<Utf8> recordingsJsonl, Pointer<AimuxCError> err);

// Multi-arg constructor C signatures (bedrock/vertex/azure).
typedef _FourStrC = Uint64 Function(Pointer<Utf8>, Pointer<Utf8>, Pointer<Utf8>,
    Pointer<Utf8>, Pointer<AimuxCError>);
typedef _FourStrDart = int Function(Pointer<Utf8>, Pointer<Utf8>, Pointer<Utf8>,
    Pointer<Utf8>, Pointer<AimuxCError>);
typedef _FiveStrC = Uint64 Function(Pointer<Utf8>, Pointer<Utf8>, Pointer<Utf8>,
    Pointer<Utf8>, Pointer<Utf8>, Pointer<AimuxCError>);
typedef _FiveStrDart = int Function(Pointer<Utf8>, Pointer<Utf8>, Pointer<Utf8>,
    Pointer<Utf8>, Pointer<Utf8>, Pointer<AimuxCError>);
// Azure: 3 required strings + nullable api_version + err.
typedef _FourStrOptC = Uint64 Function(Pointer<Utf8>, Pointer<Utf8>,
    Pointer<Utf8>, Pointer<Utf8>?, Pointer<AimuxCError>);
typedef _FourStrOptDart = int Function(Pointer<Utf8>, Pointer<Utf8>,
    Pointer<Utf8>, Pointer<Utf8>?, Pointer<AimuxCError>);

// Stream callbacks: on_part(json, stream_ctx) / on_done(stream_ctx).
// No on_error — terminal failures return 0 and fill AimuxError *err.
typedef _OnPartC = Void Function(Pointer<Utf8> json, Pointer<Void> streamCtx);
typedef _OnDoneC = Void Function(Pointer<Void> streamCtx);

typedef _StreamTextC = Int32 Function(
    Uint64,
    Pointer<Utf8>,
    Pointer<Utf8>?,
    Pointer<NativeFunction<_OnPartC>>,
    Pointer<NativeFunction<_OnDoneC>>,
    Pointer<Void>,
    Pointer<AimuxCError>);
typedef _StreamTextDart = int Function(
    int,
    Pointer<Utf8>,
    Pointer<Utf8>?,
    Pointer<NativeFunction<_OnPartC>>,
    Pointer<NativeFunction<_OnDoneC>>,
    Pointer<Void>,
    Pointer<AimuxCError>);

// ─────────────────────────────────────────────────────────────────────────────
// FFI library wrapper (process-wide singleton — dlopen + symbol lookups
// happen once, lazily per symbol)
// ─────────────────────────────────────────────────────────────────────────────

final class _AimuxFFI {
  final DynamicLibrary _lib = openAimuxLibrary();

  late final openaiNew =
      _lib.lookupFunction<_NewC, _NewDart>('aimux_openai_new');
  late final anthropicNew =
      _lib.lookupFunction<_NewC, _NewDart>('aimux_anthropic_new');
  late final openaiNewWithBase = _lib
      .lookupFunction<_NewWithBaseC, _NewWithBaseDart>(
          'aimux_openai_new_with_base');
  late final anthropicNewWithBase = _lib
      .lookupFunction<_NewWithBaseC, _NewWithBaseDart>(
          'aimux_anthropic_new_with_base');
  late final cohereNew =
      _lib.lookupFunction<_NewC, _NewDart>('aimux_cohere_new');
  late final cohereNewWithBase = _lib
      .lookupFunction<_NewWithBaseC, _NewWithBaseDart>(
          'aimux_cohere_new_with_base');
  late final mistralNew =
      _lib.lookupFunction<_NewC, _NewDart>('aimux_mistral_new');
  late final mistralNewWithBase = _lib
      .lookupFunction<_NewWithBaseC, _NewWithBaseDart>(
          'aimux_mistral_new_with_base');
  late final xaiNew = _lib.lookupFunction<_NewC, _NewDart>('aimux_xai_new');
  late final xaiNewWithBase = _lib
      .lookupFunction<_NewWithBaseC, _NewWithBaseDart>(
          'aimux_xai_new_with_base');
  late final bedrockNew =
      _lib.lookupFunction<_FourStrC, _FourStrDart>('aimux_bedrock_new');
  late final bedrockNewWithBase = _lib
      .lookupFunction<_FiveStrC, _FiveStrDart>('aimux_bedrock_new_with_base');
  late final vertexNew =
      _lib.lookupFunction<_FourStrC, _FourStrDart>('aimux_vertex_new');
  late final vertexNewWithBase = _lib
      .lookupFunction<_FiveStrC, _FiveStrDart>('aimux_vertex_new_with_base');
  late final anthropicAwsNew = _lib
      .lookupFunction<_NewWithBaseC, _NewWithBaseDart>(
          'aimux_anthropic_aws_new');
  late final anthropicAwsNewWithBase = _lib
      .lookupFunction<_FourStrC, _FourStrDart>(
          'aimux_anthropic_aws_new_with_base');
  late final azureNew =
      _lib.lookupFunction<_FourStrOptC, _FourStrOptDart>('aimux_azure_new');
  late final azureNewWithBase = _lib
      .lookupFunction<_FourStrOptC, _FourStrOptDart>(
          'aimux_azure_new_with_base');
  late final providerNew = _lib
      .lookupFunction<_ProviderNewC, _ProviderNewDart>('aimux_provider_new');
  late final providerHandleNew = _lib
      .lookupFunction<_ProviderHandleNewC, _ProviderHandleNewDart>(
          'aimux_provider_handle_new');
  late final providerListModels = _lib
      .lookupFunction<_ProviderListModelsC, _ProviderListModelsDart>(
          'aimux_provider_list_models');
  late final providerModel = _lib
      .lookupFunction<_ProviderModelC, _ProviderModelDart>(
          'aimux_provider_model');
  late final getModelSpecs = _lib
      .lookupFunction<_GetModelSpecsC, _GetModelSpecsDart>(
          'aimux_get_model_specs');
  late final generateText = _lib
      .lookupFunction<_GenerateTextC, _GenerateTextDart>('aimux_generate_text');
  late final streamText =
      _lib.lookupFunction<_StreamTextC, _StreamTextDart>('aimux_stream_text');
  late final generateTextAsOpenAI = _lib
      .lookupFunction<_GenerateTextC, _GenerateTextDart>(
          'aimux_generate_text_as_openai');
  late final streamTextAsOpenAI = _lib
      .lookupFunction<_StreamTextC, _StreamTextDart>(
          'aimux_stream_text_as_openai');
  late final dropHandle =
      _lib.lookupFunction<_DropHandleC, _DropHandleDart>('aimux_drop_handle');
  // NativeFinalizer callback pointer: aimux_drop_handle reinterpreted as
  // Void Function(Pointer<Void>). The handle value is attached as the token
  // (its address == the uint64 handle); on 64-bit the ABI is identical, so
  // the finalizer releases the handle when a Model/ProviderHandle is GC'd
  // without close(). NativeFinalizerFunction is already a NativeFunction<…>,
  // so it is looked up directly (no extra NativeFunction wrapper).
  late final dropHandlePtr =
      _lib.lookup<NativeFinalizerFunction>('aimux_drop_handle');
  late final initLogging = _lib
      .lookupFunction<_InitLoggingC, _InitLoggingDart>('aimux_init_logging');
  late final initRecording = _lib
      .lookupFunction<_RecordingDirC, _RecordingDirDart>('aimux_init_recording');
  late final initRecordingRing = _lib
      .lookupFunction<_RecordingRingC, _RecordingRingDart>(
          'aimux_init_recording_ring');
  late final initRecordingRingDefault = _lib
      .lookupFunction<_RecordingNoArgC, _RecordingNoArgDart>(
          'aimux_init_recording_ring_default');
  late final recordingStop = _lib
      .lookupFunction<_RecordingNoArgC, _RecordingNoArgDart>(
          'aimux_recording_stop');
  late final recordingFlush = _lib
      .lookupFunction<_RecordingNoArgC, _RecordingNoArgDart>(
          'aimux_recording_flush');
  late final mockReplayNew = _lib
      .lookupFunction<_MockReplayC, _MockReplayDart>('aimux_mock_replay_new');
}

/// Process-wide FFI table. Lazily created on first use; shared by every
/// [Model] / [ProviderHandle] and the top-level functions.
final _AimuxFFI _ffi = _AimuxFFI();

/// Process-wide native finalizer that releases a handle when a [Model] or
/// [ProviderHandle] is garbage-collected without [Model.close] /
/// [ProviderHandle.close] (T9). The token attached is the handle value
/// encoded as a `Pointer` (its address == the uint64 handle), so the
/// `aimux_drop_handle` native callback receives it as a uint64.
final NativeFinalizer _handleFinalizer = NativeFinalizer(_ffi.dropHandlePtr);

// ─────────────────────────────────────────────────────────────────────────────
// Stream callback trampolines (global — safe because aimux_stream_text
// is synchronous, blocks the caller until done).
// ─────────────────────────────────────────────────────────────────────────────

StreamController<Map<String, dynamic>>? _currentController;

void _onPart(Pointer<Utf8> jsonPtr, Pointer<Void> streamCtx) {
  final controller = _currentController;
  if (controller == null || jsonPtr == nullptr) return;
  // Never let an exception escape into the FFI boundary (undefined behavior);
  // surface decode failures on the stream instead.
  try {
    controller.add(
        jsonDecode(jsonPtr.toDartString()) as Map<String, dynamic>);
  } catch (e, st) {
    controller.addError(e, st);
  }
}

void _onDone(Pointer<Void> streamCtx) {
  _currentController?.close();
}

// ─────────────────────────────────────────────────────────────────────────────
// Constructor helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Shared constructor for 4-string providers (bedrock/vertex).
int _construct4(
  String a,
  String b,
  String c,
  String modelId,
  String? baseUrl,
  _FourStrDart plain,
  _FiveStrDart withBase,
) {
  return withAimuxCError((err) {
    return withUtf8(a, (aPtr) {
      return withUtf8(b, (bPtr) {
        return withUtf8(c, (cPtr) {
          return withUtf8(modelId, (idPtr) {
            if (baseUrl == null) {
              return takeHandle(plain(aPtr, bPtr, cPtr, idPtr, err), err);
            }
            return withUtf8(baseUrl, (basePtr) {
              return takeHandle(
                  withBase(aPtr, bPtr, cPtr, idPtr, basePtr, err), err);
            });
          });
        });
      });
    });
  });
}

// ─────────────────────────────────────────────────────────────────────────────
// Model
// ─────────────────────────────────────────────────────────────────────────────

/// A model instance backed by a Rust `Arc<dyn LanguageModel>`.
///
/// Call [close] to release the native handle. Engine failures throw
/// [AimuxException] (or a subclass); using a closed handle throws [StateError].
/// Implements [Finalizable] so a [NativeFinalizer] can release the handle if
/// [close] is forgotten (T9).
class Model implements Finalizable {
  final int _handle;
  bool _closed = false;

  Model._(this._handle) {
    // Register a native finalizer so the handle is released even if close()
    // is never called. The token is the handle value as a Pointer.
    _handleFinalizer.attach(this, Pointer<Void>.fromAddress(_handle));
  }

  /// Create an OpenAI model instance.
  ///
  /// Pass [baseUrl] to target an OpenAI-compatible endpoint (Azure, Groq,
  /// a local mock server, etc.). When null, the provider's standard URL is
  /// used via `aimux_openai_new`.
  factory Model.openai(String apiKey, String modelId, {String? baseUrl}) =>
      Model._(construct2(
          apiKey, modelId, baseUrl, _ffi.openaiNew, _ffi.openaiNewWithBase));

  /// Create an Anthropic model instance.
  ///
  /// Pass [baseUrl] to target a custom Anthropic-compatible endpoint. When
  /// null, the provider's standard URL is used via `aimux_anthropic_new`.
  factory Model.anthropic(String apiKey, String modelId, {String? baseUrl}) =>
      Model._(construct2(apiKey, modelId, baseUrl, _ffi.anthropicNew,
          _ffi.anthropicNewWithBase));

  /// Create a Cohere model instance.
  factory Model.cohere(String apiKey, String modelId, {String? baseUrl}) =>
      Model._(construct2(
          apiKey, modelId, baseUrl, _ffi.cohereNew, _ffi.cohereNewWithBase));

  /// Create a Mistral model instance.
  factory Model.mistral(String apiKey, String modelId, {String? baseUrl}) =>
      Model._(construct2(
          apiKey, modelId, baseUrl, _ffi.mistralNew, _ffi.mistralNewWithBase));

  /// Create an xAI model instance.
  factory Model.xai(String apiKey, String modelId, {String? baseUrl}) =>
      Model._(construct2(
          apiKey, modelId, baseUrl, _ffi.xaiNew, _ffi.xaiNewWithBase));

  /// Create a Bedrock model instance (AWS SigV4 credentials).
  factory Model.bedrock(
          String accessKeyId, String secretAccessKey, String region,
          String modelId,
          {String? baseUrl}) =>
      Model._(_construct4(accessKeyId, secretAccessKey, region, modelId,
          baseUrl, _ffi.bedrockNew, _ffi.bedrockNewWithBase));

  /// Create a Vertex AI model instance (GCP bearer token).
  factory Model.vertex(
          String accessToken, String project, String location, String modelId,
          {String? baseUrl}) =>
      Model._(_construct4(accessToken, project, location, modelId, baseUrl,
          _ffi.vertexNew, _ffi.vertexNewWithBase));

  /// Create an Anthropic-on-AWS model instance (API key + region).
  factory Model.anthropicAws(String apiKey, String region, String modelId,
      {String? baseUrl}) {
    final handle = withAimuxCError((err) {
      return withUtf8(apiKey, (keyPtr) {
        return withUtf8(region, (rPtr) {
          return withUtf8(modelId, (idPtr) {
            if (baseUrl == null) {
              return takeHandle(
                  _ffi.anthropicAwsNew(keyPtr, rPtr, idPtr, err), err);
            }
            return withUtf8(baseUrl, (basePtr) {
              return takeHandle(
                  _ffi.anthropicAwsNewWithBase(
                      keyPtr, rPtr, idPtr, basePtr, err),
                  err);
            });
          });
        });
      });
    });
    return Model._(handle);
  }

  /// Create an Azure OpenAI model instance (API key + resource name).
  /// The deployment name is passed as [deployment]; [apiVersion] is optional.
  factory Model.azure(String apiKey, String resourceName, String deployment,
      {String? apiVersion, String? baseUrl}) {
    final handle = withAimuxCError((err) {
      return withUtf8(apiKey, (keyPtr) {
        return withUtf8(deployment, (dPtr) {
          final vPtr =
              apiVersion != null ? apiVersion.toNativeUtf8() : nullptr;
          try {
            if (baseUrl == null) {
              return withUtf8(resourceName, (rPtr) {
                return takeHandle(
                    _ffi.azureNew(keyPtr, rPtr, dPtr, vPtr, err), err);
              });
            }
            return withUtf8(baseUrl, (bPtr) {
              return takeHandle(
                  _ffi.azureNewWithBase(keyPtr, bPtr, dPtr, vPtr, err), err);
            });
          } finally {
            if (vPtr != nullptr) calloc.free(vPtr);
          }
        });
      });
    });
    return Model._(handle);
  }

  /// Create a model from the provider registry by name (RFC-0017 phase 4).
  ///
  /// [apiKey] may be null to read the provider's env var from the registry
  /// entry. [configJson] is an optional JSON object of ProviderOptions
  /// (`{"base_url": "...", "headers": {...}, "max_retries": 0, "body_overrides": {...}}`).
  factory Model.provider(String name, String modelId,
      {String? apiKey, String? configJson}) {
    final handle = withAimuxCError((err) {
      return withUtf8(name, (namePtr) {
        return withUtf8(modelId, (idPtr) {
          final keyPtr = apiKey != null ? apiKey.toNativeUtf8() : nullptr;
          final cfgPtr =
              configJson != null ? configJson.toNativeUtf8() : nullptr;
          try {
            return takeHandle(
                _ffi.providerNew(namePtr, keyPtr, idPtr, cfgPtr, err), err);
          } finally {
            if (keyPtr != nullptr) calloc.free(keyPtr);
            if (cfgPtr != nullptr) calloc.free(cfgPtr);
          }
        });
      });
    });
    return Model._(handle);
  }

  /// Generate text (non-streaming).
  ///
  /// [prompt] — a string or a list of message maps.
  /// [options] — optional generation options map.
  /// Returns the parsed GenerateTextResult as a Map.
  /// Throws [AimuxException] on engine failure.
  Map<String, dynamic> generateText(
    Object prompt, [
    Map<String, dynamic>? options,
  ]) {
    _checkOpen();
    final promptJson = _promptToJson(prompt);
    final optsJson = options != null ? jsonEncode(options) : null;

    final resultStr = withAimuxCError((err) {
      return withUtf8(promptJson, (promptPtr) {
        final optsPtr = optsJson != null ? optsJson.toNativeUtf8() : nullptr;
        try {
          final resultPtr =
              _ffi.generateText(_handle, promptPtr, optsPtr, err);
          return takeString(resultPtr, err);
        } finally {
          if (optsPtr != nullptr) calloc.free(optsPtr);
        }
      });
    });

    return jsonDecode(resultStr) as Map<String, dynamic>;
  }

  /// Stream text from the model.
  ///
  /// Returns a Stream of StreamPart maps. Terminal failures throw
  /// [AimuxException] via [Stream.addError] (and do not call on_done).
  ///
  /// NOTE: the underlying `aimux_stream_text` call is synchronous — this
  /// method blocks the calling isolate until the stream completes, so all
  /// parts are effectively buffered and delivered only after completion. Run
  /// it in a worker isolate for UI code. Making this truly incremental is
  /// follow-up work.
  Stream<Map<String, dynamic>> streamText(
    Object prompt, [
    Map<String, dynamic>? options,
  ]) {
    _checkOpen();
    return _stream(_ffi.streamText, prompt, options);
  }

  /// Stream text from the model with OpenAI Chat Completions output
  /// (RFC-0026).
  ///
  /// Returns a Stream of ChatCompletionChunk maps (OpenAI
  /// `chat.completion.chunk` objects). Stream options (`include_usage`,
  /// `include_reasoning`) are passed via
  /// `options['provider_options']['openai']['stream_options']`.
  ///
  /// NOTE: like [streamText], this blocks the calling isolate until the
  /// stream completes and effectively buffers all parts until then.
  Stream<Map<String, dynamic>> streamTextAsOpenAI(
    Object prompt, [
    Map<String, dynamic>? options,
  ]) {
    _checkOpen();
    return _stream(_ffi.streamTextAsOpenAI, prompt, options);
  }

  Stream<Map<String, dynamic>> _stream(
    _StreamTextDart streamFn,
    Object prompt,
    Map<String, dynamic>? options,
  ) {
    final promptJson = _promptToJson(prompt);
    final optsJson = options != null ? jsonEncode(options) : null;

    final controller = StreamController<Map<String, dynamic>>();

    withAimuxCError((err) {
      withUtf8(promptJson, (promptPtr) {
        final optsPtr = optsJson != null ? optsJson.toNativeUtf8() : nullptr;
        try {
          final partCb = Pointer.fromFunction<_OnPartC>(_onPart);
          final doneCb = Pointer.fromFunction<_OnDoneC>(_onDone);

          final int ok;
          _currentController = controller;
          try {
            ok = streamFn(
              _handle,
              promptPtr,
              optsPtr,
              partCb,
              doneCb,
              nullptr,
              err,
            );
          } finally {
            _currentController = null;
          }

          if (ok == 0) {
            // Terminal failure: on_done was not called.
            if (!controller.isClosed) {
              controller.addError(AimuxException.fromC(err.ref));
              controller.close();
            }
          }
        } finally {
          if (optsPtr != nullptr) calloc.free(optsPtr);
        }
      });
    });

    return controller.stream;
  }

  /// Generate text (non-streaming) with OpenAI Chat Completions output
  /// (RFC-0026).
  ///
  /// Same as [generateText], but returns a parsed ChatCompletion map (OpenAI
  /// `chat.completion` object). Works with any provider.
  Map<String, dynamic> generateTextAsOpenAI(
    Object prompt, [
    Map<String, dynamic>? options,
  ]) {
    _checkOpen();
    final promptJson = _promptToJson(prompt);
    final optsJson = options != null ? jsonEncode(options) : null;

    final resultStr = withAimuxCError((err) {
      return withUtf8(promptJson, (promptPtr) {
        final optsPtr = optsJson != null ? optsJson.toNativeUtf8() : nullptr;
        try {
          final resultPtr =
              _ffi.generateTextAsOpenAI(_handle, promptPtr, optsPtr, err);
          return takeString(resultPtr, err);
        } finally {
          if (optsPtr != nullptr) calloc.free(optsPtr);
        }
      });
    });

    return jsonDecode(resultStr) as Map<String, dynamic>;
  }

  /// Release the native handle. Safe to call multiple times.
  ///
  /// Detaches the native finalizer first so the GC cannot double-free the
  /// handle after an explicit close.
  void close() {
    if (_closed) return;
    _closed = true;
    _handleFinalizer.detach(this);
    _ffi.dropHandle(_handle);
  }

  void _checkOpen() {
    if (_closed) throw StateError('Model has been closed');
  }
}

// ─────────────────────────────────────────────────────────────────────────────
// ProviderConfig (RFC-0027)
// ─────────────────────────────────────────────────────────────────────────────

/// Per-provider construction options (RFC-0027), mirroring the Rust
/// `ProviderOptions` wire format. Pass to [createProvider] to override
/// individual fields of the registry entry.
///
/// All fields are optional; null fields are omitted from the serialized JSON
/// so the provider's defaults apply.
class ProviderConfig {
  /// Override the registry base URL.
  final String? baseUrl;

  /// Extra headers merged into every request.
  final Map<String, String>? headers;

  /// OpenAI organization ID (`OpenAI-Organization` header).
  final String? organization;

  /// OpenAI project ID (`OpenAI-Project` header).
  final String? project;

  /// Retry count override; `0` disables retries.
  final int? maxRetries;

  /// Request-body overrides (deep-merged into every request). Any
  /// JSON-serializable value (typically a `Map<String, dynamic>`).
  final Object? bodyOverrides;

  const ProviderConfig({
    this.baseUrl,
    this.headers,
    this.organization,
    this.project,
    this.maxRetries,
    this.bodyOverrides,
  });

  /// Serialize to the `ProviderOptions` JSON wire format (snake_case keys,
  /// null fields omitted). Returns null when no field is set, so callers can
  /// pass null to the FFI for defaults.
  String? toJson() {
    final map = <String, dynamic>{};
    if (baseUrl != null) map['base_url'] = baseUrl;
    if (headers != null) map['headers'] = headers;
    if (organization != null) map['organization'] = organization;
    if (project != null) map['project'] = project;
    if (maxRetries != null) map['max_retries'] = maxRetries;
    if (bodyOverrides != null) map['body_overrides'] = bodyOverrides;
    if (map.isEmpty) return null;
    return jsonEncode(map);
  }
}

// ─────────────────────────────────────────────────────────────────────────────
// ProviderHandle (RFC-0027)
// ─────────────────────────────────────────────────────────────────────────────

/// A provider handle (RFC-0027) backed by a Rust `Arc<dyn Provider>`.
///
/// Created by [createProvider]. Unlike [Model.provider] (which binds to a
/// single model id), a provider handle supports runtime model discovery via
/// [listModels] and building a [Model] from a discovered id via [model].
/// Call [close] to release the native handle. Implements [Finalizable] so a
/// [NativeFinalizer] can release the handle if [close] is forgotten (T9).
///
/// ```dart
/// final p = createProvider('deepseek', apiKey);
/// try {
///   final models = p.listModels();
///   final model = p.model('deepseek-chat');
///   try {
///     print(model.generateText('Hello'));
///   } finally {
///     model.close();
///   }
/// } finally {
///   p.close();
/// }
/// ```
class ProviderHandle implements Finalizable {
  final int _handle;
  bool _closed = false;

  ProviderHandle._(this._handle) {
    // Register a native finalizer so the handle is released even if close()
    // is never called. The token is the handle value as a Pointer.
    _handleFinalizer.attach(this, Pointer<Void>.fromAddress(_handle));
  }

  /// List models available on this provider (runtime discovery via the
  /// provider's `/models` endpoint), enriched with community knowledge
  /// (anya2a) when available.
  ///
  /// Returns a JSON array string of `ResolvedModel`
  /// (`[{"id":"...","owned_by":"...","created":<u64>,"spec":{...}}, ...]`).
  /// Synchronous: blocks the calling isolate until the network call completes.
  String listModels() {
    _checkOpen();
    return withAimuxCError((err) {
      final ptr = _ffi.providerListModels(_handle, err);
      return takeString(ptr, err);
    });
  }

  /// Build a language [Model] from a discovered [modelId].
  ///
  /// The returned [Model] is backed by a fresh native handle (separate from
  /// this provider handle) and is usable with `generateText` / `streamText`.
  /// The caller owns the returned [Model] and must [Model.close] it.
  Model model(String modelId) {
    _checkOpen();
    final handle = withAimuxCError((err) {
      return withUtf8(modelId, (idPtr) {
        return takeHandle(_ffi.providerModel(_handle, idPtr, err), err);
      });
    });
    return Model._(handle);
  }

  /// Release the native provider handle. Safe to call multiple times.
  ///
  /// Detaches the native finalizer first so the GC cannot double-free the
  /// handle after an explicit close.
  void close() {
    if (_closed) return;
    _closed = true;
    _handleFinalizer.detach(this);
    _ffi.dropHandle(_handle);
  }

  void _checkOpen() {
    if (_closed) throw StateError('ProviderHandle has been closed');
  }
}

/// Create a **provider handle** (RFC-0027) for a registry-backed provider.
///
/// Unlike [Model.provider] (which binds to a single `modelId`), this returns a
/// [ProviderHandle] supporting runtime model discovery
/// ([ProviderHandle.listModels]) and building a [Model] from a discovered id
/// ([ProviderHandle.model]).
///
/// [name] is a registry provider name (e.g. `"deepseek"`, `"groq"`). [apiKey]
/// may be null to read the provider's env var from the registry entry.
/// [config] overrides individual fields of the registry entry.
ProviderHandle createProvider(
    String name, String? apiKey, ProviderConfig? config) {
  final configJson = config?.toJson();
  final handle = withAimuxCError((err) {
    return withUtf8(name, (namePtr) {
      final keyPtr = apiKey != null ? apiKey.toNativeUtf8() : nullptr;
      final cfgPtr = configJson != null ? configJson.toNativeUtf8() : nullptr;
      try {
        return takeHandle(
            _ffi.providerHandleNew(namePtr, keyPtr, cfgPtr, err), err);
      } finally {
        if (keyPtr != nullptr) calloc.free(keyPtr);
        if (cfgPtr != nullptr) calloc.free(cfgPtr);
      }
    });
  });
  return ProviderHandle._(handle);
}

// ─────────────────────────────────────────────────────────────────────────────
// Model specs (RFC-0027) — get_model_specs
// ─────────────────────────────────────────────────────────────────────────────

/// Fetch the community model catalogue (anya2a). Returns a JSON-serialized
/// Catalogue string. Thin fetch — no caching.
String getModelSpecs(String? sourceUrl) {
  return withAimuxCError((err) {
    final urlPtr = sourceUrl != null ? sourceUrl.toNativeUtf8() : nullptr;
    try {
      final ptr = _ffi.getModelSpecs(urlPtr, err);
      return takeString(ptr, err);
    } finally {
      if (urlPtr != nullptr) calloc.free(urlPtr);
    }
  });
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

String _promptToJson(Object prompt) {
  if (prompt is String) return jsonEncode(prompt);
  return jsonEncode({'prompt': prompt});
}

/// Initialize the global logger (RFC-0014).
///
/// Idempotent — safe to call any number of times from any thread; only the
/// first call has an effect. If the host already registered its own
/// `tracing` subscriber, this is a no-op (aimux never overrides a consumer's
/// logger).
///
/// [level]: "off" | "error" | "warn" | "info" | "debug" | "trace"; empty
/// defaults to "warn". The `AIMUX_LOG` / `AIMUX_LOG_LEVEL` environment
/// variables take precedence when set. Logs go to stderr.
void initLogging(String level) {
  withUtf8(level.isEmpty ? 'warn' : level, _ffi.initLogging);
}

// ─────────────────────────────────────────────────────────────────────────────
// Recording + mock replay (RFC-0023)
//
// Recording is opt-in and global (not tied to a Model instance). All
// int-returning functions return 0 on success, or -1 on invalid input.
// ─────────────────────────────────────────────────────────────────────────────

/// Start recording: complete Recording JSONL is written to
/// `{dir}/recordings.jsonl` (dir is auto-created). Calling again replaces the
/// recorder. Returns 0, or -1 when [dir] is null/empty.
int initRecording(String dir) => withUtf8(dir, _ffi.initRecording);

/// Start in-memory bounded recording (ring recorder, FIFO eviction, dropped
/// count queryable).
///
/// [cap] is optional: omit it (or pass null) to use the library default
/// capacity (FFI `aimux_init_recording_ring_default`). When provided, [cap]
/// must be positive: the C ABI rejects `0` (returns -1), and a negative Dart
/// int would be reinterpreted as a huge u64 by the FFI. This binding validates
/// up front and throws [ArgumentError] for `cap <= 0`, matching Kotlin/Java.
/// Returns 0 on success.
int initRecordingRing([int? cap]) {
  if (cap == null) {
    return _ffi.initRecordingRingDefault();
  }
  if (cap <= 0) {
    throw ArgumentError.value(cap, 'cap', 'must be positive');
  }
  return _ffi.initRecordingRing(cap);
}

/// Stop recording: the global recorder becomes None. Returns 0.
int recordingStop() => _ffi.recordingStop();

/// Flush the global recorder (blocks until JSONL is on disk; no-op for the
/// ring recorder). Returns 0.
int recordingFlush() => _ffi.recordingFlush();

/// Create a mock replay model from recorded JSONL (one `Recording` per line).
///
/// Matches recorded inputs and returns the recorded responses — no real API
/// is sent. The returned [Model] works with [Model.generateText] /
/// [Model.streamText]. Call [Model.close] to release the native handle.
Model mockReplay(String recordingsJsonl) {
  final handle = withAimuxCError((err) {
    return withUtf8(recordingsJsonl, (jsonlPtr) {
      return takeHandle(_ffi.mockReplayNew(jsonlPtr, err), err);
    });
  });
  return Model._(handle);
}
