// aimux.dart — Flutter/Dart binding for aimux (dart:ffi, C ABI path).
//
// Calls aimux-ffi's C ABI directly via dart:ffi. No flutter_rust_bridge,
// no codegen step. The native library (libaimux_ffi.so / .dylib / .dll)
// must be on the library path or bundled in the Flutter app.
//
// This is the C ABI path (RFC §3.2) — same as Swift and Kotlin.
//
// Error transport (aimux-error.h): every fallible call returns
// `aimux_error_t *` (NULL = success, result in the trailing out-param).
// Call sites hand the pointer to the decoder in errors.dart — expectAimuxError
// (model calls) / expectRecordingError (recorder) / expectFfiError ([C ABI] calls) —
// which throws AimuxException / RecordingException, or StateError('aimux ffi:
// …') for a C ABI failure. Raw JSON string parameters are pre-validated
// with checkJson; JSON this binding builds itself goes through encodeJson;
// every string crossing the ABI goes through toCString / toCStringOrNull. All
// three reject what the Rust side cannot represent (unpaired surrogates,
// interior NULs) as a FormatException here rather than a StateError there.

// The Flutter tool resolves `dartPluginClass` from the package's main
// library, so the plugin registration class must be visible here.
export 'aimux_plugin.dart';
export 'errors.dart'
    hide
        openAimuxLibrary,
        aimuxFreeString,
        aimuxDropHandle,
        withUtf8,
        toCString,
        toCStringOrNull,
        takeHandle,
        takeString,
        expectAimuxError,
        expectRecordingError,
        expectFfiError,
        checkJson,
        checkPortable,
        encodeJson,
        construct2;

import 'dart:async';
import 'dart:convert';
import 'dart:ffi';
import 'package:ffi/ffi.dart';

import 'errors.dart';

// ─────────────────────────────────────────────────────────────────────────────
// FFI type aliases. `_Err` is `aimux_error_t *` (opaque; NULL = success);
// results come back through the trailing out-param (`uint64_t *out_handle` /
// `char **out_json`). Pointer types are spelled the same in the C and Dart
// signatures, so a single alias serves both.
// ─────────────────────────────────────────────────────────────────────────────

typedef _Err = Pointer<Void>;

typedef _NewC = _Err Function(
    Pointer<Utf8> apiKey, Pointer<Utf8> modelId, Pointer<Uint64> outHandle);
typedef _NewDart = _Err Function(
    Pointer<Utf8> apiKey, Pointer<Utf8> modelId, Pointer<Uint64> outHandle);

// Constructors with a custom base URL (aimux_*_new_with_base).
typedef _NewWithBaseC = _Err Function(Pointer<Utf8> apiKey,
    Pointer<Utf8> modelId, Pointer<Utf8> baseUrl, Pointer<Uint64> outHandle);
typedef _NewWithBaseDart = _Err Function(Pointer<Utf8> apiKey,
    Pointer<Utf8> modelId, Pointer<Utf8> baseUrl, Pointer<Uint64> outHandle);

// Registry provider constructor (aimux_provider_new, RFC-0017 phase 4).
typedef _ProviderNewC = _Err Function(
    Pointer<Utf8> name,
    Pointer<Utf8>? apiKey,
    Pointer<Utf8> modelId,
    Pointer<Utf8>? configJson,
    Pointer<Uint64> outHandle);
typedef _ProviderNewDart = _Err Function(
    Pointer<Utf8> name,
    Pointer<Utf8>? apiKey,
    Pointer<Utf8> modelId,
    Pointer<Utf8>? configJson,
    Pointer<Uint64> outHandle);

// Provider handle constructor (aimux_provider_handle_new, RFC-0027).
typedef _ProviderHandleNewC = _Err Function(Pointer<Utf8> name,
    Pointer<Utf8>? apiKey, Pointer<Utf8>? configJson, Pointer<Uint64> outHandle);
typedef _ProviderHandleNewDart = _Err Function(Pointer<Utf8> name,
    Pointer<Utf8>? apiKey, Pointer<Utf8>? configJson, Pointer<Uint64> outHandle);

// Provider handle list_models / model (RFC-0027).
typedef _ProviderListModelsC = _Err Function(
    Uint64 handle, Pointer<Pointer<Utf8>> outModelsJson);
typedef _ProviderListModelsDart = _Err Function(
    int handle, Pointer<Pointer<Utf8>> outModelsJson);

typedef _ProviderModelC = _Err Function(
    Uint64 handle, Pointer<Utf8> modelId, Pointer<Uint64> outHandle);
typedef _ProviderModelDart = _Err Function(
    int handle, Pointer<Utf8> modelId, Pointer<Uint64> outHandle);

typedef _GetModelSpecsC = _Err Function(
    Pointer<Utf8>? sourceUrl, Pointer<Pointer<Utf8>> outSpecsJson);
typedef _GetModelSpecsDart = _Err Function(
    Pointer<Utf8>? sourceUrl, Pointer<Pointer<Utf8>> outSpecsJson);

typedef _GenerateTextC = _Err Function(Uint64 handle, Pointer<Utf8> promptJson,
    Pointer<Utf8>? optsJson, Pointer<Pointer<Utf8>> outJson);
typedef _GenerateTextDart = _Err Function(int handle, Pointer<Utf8> promptJson,
    Pointer<Utf8>? optsJson, Pointer<Pointer<Utf8>> outJson);

typedef _DropHandleC = Void Function(Uint64);
typedef _DropHandleDart = void Function(int);

// (const char *) → error: aimux_init_logging, aimux_init_recording,
// aimux_register_providers, aimux_init_proxy.
typedef _StrC = _Err Function(Pointer<Utf8>);
typedef _StrDart = _Err Function(Pointer<Utf8>);

// Recording + mock replay (RFC-0023).
typedef _RecordingRingC = _Err Function(Uint64 cap);
typedef _RecordingRingDart = _Err Function(int cap);
typedef _VoidNoArgC = Void Function();
typedef _VoidNoArgDart = void Function();
typedef _RecordingTryFlushC = _Err Function();
typedef _RecordingTryFlushDart = _Err Function();
typedef _MockReplayC = _Err Function(
    Pointer<Utf8> recordingsJsonl, Pointer<Uint64> outHandle);
typedef _MockReplayDart = _Err Function(
    Pointer<Utf8> recordingsJsonl, Pointer<Uint64> outHandle);

// Composite models (RFC-0021 / RFC-0022). Take a pointer to an array of
// handles + a length, plus an optional JSON config (nullptr = defaults).
typedef _RouterNewC = _Err Function(Pointer<Uint64> handles, IntPtr len,
    Pointer<Utf8> configJson, Pointer<Uint64> outHandle);
typedef _RouterNewDart = _Err Function(Pointer<Uint64> handles, int len,
    Pointer<Utf8> configJson, Pointer<Uint64> outHandle);
typedef _MoaNewC = _Err Function(Pointer<Uint64> referenceHandles, IntPtr refLen,
    Uint64 aggregator, Pointer<Utf8> configJson, Pointer<Uint64> outHandle);
typedef _MoaNewDart = _Err Function(Pointer<Uint64> referenceHandles, int refLen,
    int aggregator, Pointer<Utf8> configJson, Pointer<Uint64> outHandle);

// Multi-arg constructor C signatures (bedrock/vertex/azure).
typedef _FourStrC = _Err Function(Pointer<Utf8>, Pointer<Utf8>, Pointer<Utf8>,
    Pointer<Utf8>, Pointer<Uint64>);
typedef _FourStrDart = _Err Function(Pointer<Utf8>, Pointer<Utf8>,
    Pointer<Utf8>, Pointer<Utf8>, Pointer<Uint64>);
typedef _FiveStrC = _Err Function(Pointer<Utf8>, Pointer<Utf8>, Pointer<Utf8>,
    Pointer<Utf8>, Pointer<Utf8>, Pointer<Uint64>);
typedef _FiveStrDart = _Err Function(Pointer<Utf8>, Pointer<Utf8>,
    Pointer<Utf8>, Pointer<Utf8>, Pointer<Utf8>, Pointer<Uint64>);
// Azure: 3 required strings + nullable api_version + out_handle.
typedef _FourStrOptC = _Err Function(Pointer<Utf8>, Pointer<Utf8>,
    Pointer<Utf8>, Pointer<Utf8>?, Pointer<Uint64>);
typedef _FourStrOptDart = _Err Function(Pointer<Utf8>, Pointer<Utf8>,
    Pointer<Utf8>, Pointer<Utf8>?, Pointer<Uint64>);

// Stream callbacks: on_part(json, stream_ctx) / on_done(stream_ctx).
// No on_error — a terminal failure returns a non-NULL error (no on_done);
// NULL after on_done is the clean end.
typedef _OnPartC = Void Function(Pointer<Utf8> json, Pointer<Void> streamCtx);
typedef _OnDoneC = Void Function(Pointer<Void> streamCtx);

typedef _StreamTextC = _Err Function(
    Uint64,
    Pointer<Utf8>,
    Pointer<Utf8>?,
    Pointer<NativeFunction<_OnPartC>>,
    Pointer<NativeFunction<_OnDoneC>>,
    Pointer<Void>);
typedef _StreamTextDart = _Err Function(
    int,
    Pointer<Utf8>,
    Pointer<Utf8>?,
    Pointer<NativeFunction<_OnPartC>>,
    Pointer<NativeFunction<_OnDoneC>>,
    Pointer<Void>);

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
  late final generateObject = _lib
      .lookupFunction<_GenerateTextC, _GenerateTextDart>('aimux_generate_object');
  late final consumeStreamText = _lib.lookupFunction<_GenerateTextC, _GenerateTextDart>(
      'aimux_consume_stream_text');
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
  late final initLogging =
      _lib.lookupFunction<_StrC, _StrDart>('aimux_init_logging');
  late final initRecording =
      _lib.lookupFunction<_StrC, _StrDart>('aimux_init_recording');
  late final initRecordingRing = _lib
      .lookupFunction<_RecordingRingC, _RecordingRingDart>(
          'aimux_init_recording_ring');
  late final initRecordingRingDefault = _lib
      .lookupFunction<_VoidNoArgC, _VoidNoArgDart>(
          'aimux_init_recording_ring_default');
  late final recordingStop = _lib
      .lookupFunction<_VoidNoArgC, _VoidNoArgDart>('aimux_recording_stop');
  late final recordingFlush = _lib
      .lookupFunction<_VoidNoArgC, _VoidNoArgDart>('aimux_recording_flush');
  late final recordingTryFlush = _lib
      .lookupFunction<_RecordingTryFlushC, _RecordingTryFlushDart>(
          'aimux_recording_try_flush');
  late final mockReplayNew = _lib
      .lookupFunction<_MockReplayC, _MockReplayDart>('aimux_mock_replay_new');
  late final registerProviders =
      _lib.lookupFunction<_StrC, _StrDart>('aimux_register_providers');
  late final initProxy =
      _lib.lookupFunction<_StrC, _StrDart>('aimux_init_proxy');
  late final routerNew = _lib
      .lookupFunction<_RouterNewC, _RouterNewDart>('aimux_router_new');
  late final moaNew =
      _lib.lookupFunction<_MoaNewC, _MoaNewDart>('aimux_moa_new');
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
  // Never let an exception escape into the C ABI (undefined behavior);
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
  return withUtf8(a, (aPtr) {
    return withUtf8(b, (bPtr) {
      return withUtf8(c, (cPtr) {
        return withUtf8(modelId, (idPtr) {
          if (baseUrl == null) {
            return takeHandle(
                (out) => plain(aPtr, bPtr, cPtr, idPtr, out), 'new');
          }
          return withUtf8(baseUrl, (basePtr) {
            return takeHandle(
                (out) => withBase(aPtr, bPtr, cPtr, idPtr, basePtr, out),
                'new_with_base');
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
/// Call [close] to release the native handle. AiMuxError values throw
/// [AimuxException] (or a subclass); using a closed handle throws
/// [StateError].
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

  /// Handle read for composite-model factories (router/moa). Throws if closed.
  int _requireHandle() {
    if (_closed) throw StateError('Model is closed');
    return _handle;
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
    final handle = withUtf8(apiKey, (keyPtr) {
      return withUtf8(region, (rPtr) {
        return withUtf8(modelId, (idPtr) {
          if (baseUrl == null) {
            return takeHandle(
                (out) => _ffi.anthropicAwsNew(keyPtr, rPtr, idPtr, out),
                'anthropic_aws_new');
          }
          return withUtf8(baseUrl, (basePtr) {
            return takeHandle(
                (out) => _ffi.anthropicAwsNewWithBase(
                    keyPtr, rPtr, idPtr, basePtr, out),
                'anthropic_aws_new_with_base');
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
    final handle = withUtf8(apiKey, (keyPtr) {
      return withUtf8(deployment, (dPtr) {
        final vPtr = toCStringOrNull(apiVersion);
        try {
          if (baseUrl == null) {
            return withUtf8(resourceName, (rPtr) {
              return takeHandle(
                  (out) => _ffi.azureNew(keyPtr, rPtr, dPtr, vPtr, out),
                  'azure_new');
            });
          }
          return withUtf8(baseUrl, (bPtr) {
            return takeHandle(
                (out) => _ffi.azureNewWithBase(keyPtr, bPtr, dPtr, vPtr, out),
                'azure_new_with_base');
          });
        } finally {
          if (vPtr != nullptr) calloc.free(vPtr);
        }
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
    checkJson(configJson, 'config_json', emptyIsDefault: true);
    final handle = withUtf8(name, (namePtr) {
      return withUtf8(modelId, (idPtr) {
        final keyPtr = toCStringOrNull(apiKey);
        final cfgPtr = toCStringOrNull(configJson);
        try {
          return takeHandle(
              (out) => _ffi.providerNew(namePtr, keyPtr, idPtr, cfgPtr, out),
              'provider_new');
        } finally {
          if (keyPtr != nullptr) calloc.free(keyPtr);
          if (cfgPtr != nullptr) calloc.free(cfgPtr);
        }
      });
    });
    return Model._(handle);
  }

  /// Generate text (non-streaming).
  ///
  /// [prompt] — a string or a list of message maps.
  /// [options] — optional generation options map.
  /// Returns the parsed GenerateTextResult as a Map.
  /// Throws [AimuxException] on AiMuxError failure.
  Map<String, dynamic> generateText(
    Object prompt, [
    Map<String, dynamic>? options,
  ]) =>
      _callJson(_ffi.generateText, 'generate_text', prompt, options);

  /// Generate a structured JSON object (M12, RFC-0016).
  ///
  /// Same signature as [generateText]; returns the parsed
  /// GenerateObjectResult as a Map. Pass `response_format: { "Json": { ... } }`
  /// via [options] for schema control; aimux-core applies JSON repair before
  /// parsing.
  ///
  /// [prompt] — a string or a list of message maps.
  /// [options] — optional generation options map.
  /// Throws [AimuxException] on AiMuxError failure.
  Map<String, dynamic> generateObject(
    Object prompt, [
    Map<String, dynamic>? options,
  ]) =>
      _callJson(_ffi.generateObject, 'generate_object', prompt, options);

  /// Consume a stream to completion and return the aggregated result
  /// (M11, RFC-0016). Synchronous (blocks until the stream finishes).
  ///
  /// Same signature as [generateText]; returns the parsed
  /// StreamTextResultAggregated as a Map.
  ///
  /// [prompt] — a string or a list of message maps.
  /// [options] — optional generation options map.
  /// Throws [AimuxException] on AiMuxError failure.
  Map<String, dynamic> consumeStreamText(
    Object prompt, [
    Map<String, dynamic>? options,
  ]) =>
      _callJson(
          _ffi.consumeStreamText, 'consume_stream_text', prompt, options);

  /// Generate text (non-streaming) with OpenAI Chat Completions output
  /// (RFC-0026).
  ///
  /// Same as [generateText], but returns a parsed ChatCompletion map (OpenAI
  /// `chat.completion` object). Works with any provider.
  Map<String, dynamic> generateTextAsOpenAI(
    Object prompt, [
    Map<String, dynamic>? options,
  ]) =>
      _callJson(_ffi.generateTextAsOpenAI, 'generate_text_as_openai', prompt,
          options);

  /// Shared body of the four `(handle, prompt_json, opts_json, out_json)`
  /// calls: encode, call, [expectAimuxError] (via [takeString]), decode.
  Map<String, dynamic> _callJson(
    _GenerateTextDart fn,
    String context,
    Object prompt,
    Map<String, dynamic>? options,
  ) {
    _checkOpen();
    final promptJson = _promptToJson(prompt);
    final optsJson = options != null ? encodeJson(options, 'options') : null;

    final resultStr = withUtf8(promptJson, (promptPtr) {
      final optsPtr = toCStringOrNull(optsJson);
      try {
        return takeString(
            (out) => fn(_handle, promptPtr, optsPtr, out), context);
      } finally {
        if (optsPtr != nullptr) calloc.free(optsPtr);
      }
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
    return _stream(_ffi.streamText, 'stream_text', prompt, options);
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
    return _stream(
        _ffi.streamTextAsOpenAI, 'stream_text_as_openai', prompt, options);
  }

  Stream<Map<String, dynamic>> _stream(
    _StreamTextDart streamFn,
    String context,
    Object prompt,
    Map<String, dynamic>? options,
  ) {
    final promptJson = _promptToJson(prompt);
    final optsJson = options != null ? encodeJson(options, 'options') : null;

    final controller = StreamController<Map<String, dynamic>>();

    withUtf8(promptJson, (promptPtr) {
      final optsPtr = toCStringOrNull(optsJson);
      try {
        final partCb = Pointer.fromFunction<_OnPartC>(_onPart);
        final doneCb = Pointer.fromFunction<_OnDoneC>(_onDone);

        final Pointer<Void> e;
        _currentController = controller;
        try {
          e = streamFn(_handle, promptPtr, optsPtr, partCb, doneCb, nullptr);
        } finally {
          _currentController = null;
        }

        // NULL after on_done = clean end. Non-NULL = terminal failure, on_done
        // was not called: expectAimuxError throws AimuxException or the native
        // StateError for a C ABI failure; either way the error must not be
        // swallowed and the controller must not stay open.
        try {
          expectAimuxError(e, context);
        } catch (failure) {
          if (controller.isClosed) rethrow;
          controller
            ..addError(failure)
            ..close();
        }
      } finally {
        if (optsPtr != nullptr) calloc.free(optsPtr);
      }
    });

    return controller.stream;
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
    if (_closed) throw StateError('Model is closed');
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
    return encodeJson(map, 'config_json');
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
    return takeString(
        (out) => _ffi.providerListModels(_handle, out), 'provider_list_models');
  }

  /// Build a language [Model] from a discovered [modelId].
  ///
  /// The returned [Model] is backed by a fresh native handle (separate from
  /// this provider handle) and is usable with `generateText` / `streamText`.
  /// The caller owns the returned [Model] and must [Model.close] it.
  Model model(String modelId) {
    _checkOpen();
    final handle = withUtf8(modelId, (idPtr) {
      return takeHandle(
          (out) => _ffi.providerModel(_handle, idPtr, out), 'provider_model');
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
    if (_closed) throw StateError('ProviderHandle is closed');
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
  final handle = withUtf8(name, (namePtr) {
    final keyPtr = toCStringOrNull(apiKey);
    final cfgPtr = toCStringOrNull(configJson);
    try {
      return takeHandle(
          (out) => _ffi.providerHandleNew(namePtr, keyPtr, cfgPtr, out),
          'provider_handle_new');
    } finally {
      if (keyPtr != nullptr) calloc.free(keyPtr);
      if (cfgPtr != nullptr) calloc.free(cfgPtr);
    }
  });
  return ProviderHandle._(handle);
}

// ─────────────────────────────────────────────────────────────────────────────
// Model specs (RFC-0027) — get_model_specs
// ─────────────────────────────────────────────────────────────────────────────

/// Fetch the community model catalogue (anya2a). Returns a JSON-serialized
/// Catalogue string. Thin fetch — no caching.
String getModelSpecs(String? sourceUrl) {
  final urlPtr = toCStringOrNull(sourceUrl);
  try {
    return takeString(
        (out) => _ffi.getModelSpecs(urlPtr, out), 'get_model_specs');
  } finally {
    if (urlPtr != nullptr) calloc.free(urlPtr);
  }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

String _promptToJson(Object prompt) {
  if (prompt is String) return encodeJson(prompt, 'prompt');
  return encodeJson({'prompt': prompt}, 'prompt');
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
  withUtf8(level.isEmpty ? 'warn' : level,
      (p) => expectFfiError(_ffi.initLogging(p), 'init_logging'));
}

// ─────────────────────────────────────────────────────────────────────────────
// Recording + mock replay (RFC-0023)
//
// Recording is opt-in and global (not tied to a Model instance).
// ─────────────────────────────────────────────────────────────────────────────

/// Start recording: complete Recording JSONL is written to
/// `{dir}/recordings.jsonl` (dir is auto-created). Calling again replaces the
/// recorder. Throws [RecordingException] with [RecordingException.code] one of
/// [RecordingErrorCode.init] (dir could not be created, also for an empty
/// [dir]), [RecordingErrorCode.openFile] or [RecordingErrorCode.spawn] when
/// the recorder cannot be constructed; on failure the previous recorder (if
/// any) stays in place.
void initRecording(String dir) {
  withUtf8(dir, (p) => expectRecordingError(_ffi.initRecording(p), 'init_recording'));
}

/// Start in-memory bounded recording (ring recorder, FIFO eviction, dropped
/// count queryable).
///
/// [cap] is optional: omit it (or pass null) to use the library default
/// capacity (FFI `aimux_init_recording_ring_default`). When provided, [cap]
/// must be positive: the C ABI rejects `0`, and a negative Dart int would be
/// reinterpreted as a huge u64 by the FFI. This binding validates up front and
/// throws [ArgumentError] for `cap <= 0`, matching Kotlin/Java.
void initRecordingRing([int? cap]) {
  if (cap == null) {
    _ffi.initRecordingRingDefault();
    return;
  }
  if (cap <= 0) {
    throw ArgumentError.value(cap, 'cap', 'must be positive');
  }
  expectAimuxError(_ffi.initRecordingRing(cap), 'init_recording_ring');
}

/// Stop recording: the global recorder becomes None.
void recordingStop() => _ffi.recordingStop();

/// Flush the global recorder (blocks until JSONL is on disk; no-op for the
/// ring recorder). Never reports — see [recordingTryFlush].
void recordingFlush() => _ffi.recordingFlush();

/// Flush the global recorder and report failure: throws [RecordingException]
/// with [RecordingException.code] one of [RecordingErrorCode.writerGone],
/// [RecordingErrorCode.flushTimeout] or [RecordingErrorCode.write] (not an
/// [AimuxException] — recording errors are a separate type). Returns normally
/// when data is on disk or nothing is recording. The legacy [recordingFlush]
/// stays and never reports.
void recordingTryFlush() {
  expectRecordingError(_ffi.recordingTryFlush(), 'recording_try_flush');
}

/// Create a mock replay model from recorded JSONL (one `Recording` per line).
///
/// Matches recorded inputs and returns the recorded responses — no real API
/// is sent. The returned [Model] works with [Model.generateText] /
/// [Model.streamText]. Call [Model.close] to release the native handle.
Model mockReplay(String recordingsJsonl) {
  // Mirror the FFI: blank lines are skipped, every other line is one JSON doc.
  for (final line in LineSplitter.split(recordingsJsonl)) {
    if (line.trim().isNotEmpty) checkJson(line, 'recordings_jsonl');
  }
  final handle = withUtf8(recordingsJsonl, (jsonlPtr) {
    return takeHandle(
        (out) => _ffi.mockReplayNew(jsonlPtr, out), 'mock_replay_new');
  });
  return Model._(handle);
}

/// Register external OpenAI-compatible providers from a JSON config string
/// (RFC-0020).
///
/// `configJSON` is `{ "providers": [ { "name", "base_url", ... } ] }`. Entries
/// override same-named built-ins or add new ones. Like `initRecording`, this
/// mutates process-global registry state. A well-formed document the registry
/// rejects throws [InvalidArgumentError].
void registerProviders(String configJSON) {
  checkJson(configJSON, 'config_json');
  withUtf8(configJSON,
      (p) => expectAimuxError(_ffi.registerProviders(p), 'register_providers'));
}

/// Set the global proxy configuration (M6, RFC-0016). Must be called before
/// the first `generateText` / `streamText` call; a no-op if the shared HTTP
/// client is already initialised.
///
/// `configJSON` shape: `{ "http_url", "https_url", "all_url", "no_proxy" }`
/// (all fields optional). Wrong shape throws [InvalidArgumentError].
void initProxy(String configJSON) {
  checkJson(configJSON, 'config_json');
  withUtf8(configJSON, (p) => expectAimuxError(_ffi.initProxy(p), 'init_proxy'));
}

/// Create a RouterModel (RFC-0021) over the given child models. The returned
/// model routes each call to one child and falls back across the rest on error
/// (per `configJson`).
///
/// `configJson` (optional): `{"router": "rule"|"weighted", "weights": [...],
/// "fallback": "on_error"|"none", "provider_name", "model_id"}`.
Model router(List<Model> models, {String? configJson}) {
  if (models.isEmpty) {
    throw ArgumentError('router: models must be non-empty');
  }
  checkJson(configJson, 'config_json', emptyIsDefault: true);
  final handles = models.map((m) => m._requireHandle()).toList();
  final ptr = calloc<Uint64>(handles.length);
  final int handle;
  try {
    ptr.asTypedList(handles.length).setAll(0, handles);
    Pointer<Utf8> cfgPtr = nullptr;
    if (configJson != null) {
      cfgPtr = toCString(configJson);
    }
    try {
      handle = takeHandle(
          (out) => _ffi.routerNew(ptr, handles.length, cfgPtr, out),
          'router_new');
    } finally {
      if (cfgPtr != nullptr) {
        calloc.free(cfgPtr);
      }
    }
  } finally {
    calloc.free(ptr);
  }
  return Model._(handle);
}

/// Create a MoaModel (RFC-0022) over reference models + one aggregator.
/// References fan out in parallel, then the aggregator synthesizes a final
/// answer.
///
/// `references` may be empty (runs aggregator only). `configJson` (optional) is
/// a serialized MoaConfig.
Model moa(List<Model> references, Model aggregator, {String? configJson}) {
  checkJson(configJson, 'config_json', emptyIsDefault: true);
  final aggregatorHandle = aggregator._requireHandle();
  // Allocate the reference-handle array (NULL when there are no references).
  final refPtr = references.isEmpty
      ? Pointer<Uint64>.fromAddress(0)
      : calloc<Uint64>(references.length);
  final shouldFree = !references.isEmpty;
  final int handle;
  try {
    if (shouldFree) {
      final handles = references.map((m) => m._requireHandle()).toList();
      refPtr.asTypedList(references.length).setAll(0, handles);
    }
    Pointer<Utf8> cfgPtr = nullptr;
    if (configJson != null) {
      cfgPtr = toCString(configJson);
    }
    try {
      handle = takeHandle(
          (out) => _ffi.moaNew(
              refPtr, references.length, aggregatorHandle, cfgPtr, out),
          'moa_new');
    } finally {
      if (cfgPtr != nullptr) {
        calloc.free(cfgPtr);
      }
    }
  } finally {
    if (shouldFree) {
      calloc.free(refPtr);
    }
  }
  return Model._(handle);
}
