// aimux.dart — Flutter/Dart binding for aimux (dart:ffi, C ABI path).
//
// Calls aimux-ffi's C ABI directly via dart:ffi. No flutter_rust_bridge,
// no codegen step. The native library (libaimux_ffi.so / .dylib / .dll)
// must be on the library path or bundled in the Flutter app.
//
// This is the C ABI path (RFC §3.2) — same as Swift and Kotlin.

// The Flutter tool resolves `dartPluginClass` from the package's main
// library, so the plugin registration class must be visible here.
export 'aimux_plugin.dart';

import 'dart:async';
import 'dart:convert';
import 'dart:ffi';
import 'dart:io';
import 'package:ffi/ffi.dart';

// ─────────────────────────────────────────────────────────────────────────────
// FFI type aliases
// ─────────────────────────────────────────────────────────────────────────────

typedef _OpenaiNewC = Pointer<Utf8> Function(
    Pointer<Utf8> apiKey, Pointer<Utf8> modelId);
typedef _OpenaiNewDart = Pointer<Utf8> Function(
    Pointer<Utf8> apiKey, Pointer<Utf8> modelId);

// Constructors with a custom base URL (aimux_*_new_with_base). The same
// signature covers both OpenAI and Anthropic.
typedef _NewWithBaseC = Pointer<Utf8> Function(
    Pointer<Utf8> apiKey, Pointer<Utf8> modelId, Pointer<Utf8> baseUrl);
typedef _NewWithBaseDart = Pointer<Utf8> Function(
    Pointer<Utf8> apiKey, Pointer<Utf8> modelId, Pointer<Utf8> baseUrl);

// Registry provider constructor (aimux_provider_new, RFC-0017 phase 4).
// name/model_id are required; apiKey (null → provider env var from the
// registry entry) and configJson (null → defaults) are optional.
typedef _ProviderNewC = Pointer<Utf8> Function(Pointer<Utf8> name,
    Pointer<Utf8>? apiKey, Pointer<Utf8> modelId, Pointer<Utf8>? configJson);
typedef _ProviderNewDart = Pointer<Utf8> Function(Pointer<Utf8> name,
    Pointer<Utf8>? apiKey, Pointer<Utf8> modelId, Pointer<Utf8>? configJson);

// Provider handle constructor (aimux_provider_handle_new, RFC-0027). name is
// required; apiKey (null → provider env var from the registry entry) and
// configJson (null → defaults) are optional.
typedef _ProviderHandleNewC = Pointer<Utf8> Function(
    Pointer<Utf8> name, Pointer<Utf8>? apiKey, Pointer<Utf8>? configJson);
typedef _ProviderHandleNewDart = Pointer<Utf8> Function(
    Pointer<Utf8> name, Pointer<Utf8>? apiKey, Pointer<Utf8>? configJson);

// Provider handle list_models (aimux_provider_list_models, RFC-0027).
typedef _ProviderListModelsC = Pointer<Utf8> Function(Uint64 handle);
typedef _ProviderListModelsDart = Pointer<Utf8> Function(int handle);

// Provider handle model (aimux_provider_model, RFC-0027).
typedef _ProviderModelC = Pointer<Utf8> Function(
    Uint64 handle, Pointer<Utf8> modelId);
typedef _ProviderModelDart = Pointer<Utf8> Function(
    int handle, Pointer<Utf8> modelId);

typedef _GetModelSpecsC = Pointer<Utf8> Function(Pointer<Utf8>? sourceUrl);
typedef _GetModelSpecsDart = Pointer<Utf8> Function(Pointer<Utf8>? sourceUrl);

typedef _GenerateTextC = Pointer<Utf8> Function(
    Uint64 handle, Pointer<Utf8> promptJson, Pointer<Utf8>? optsJson);
typedef _GenerateTextDart = Pointer<Utf8> Function(
    int handle, Pointer<Utf8> promptJson, Pointer<Utf8>? optsJson);

typedef _DropHandleC = Void Function(Uint64);
typedef _DropHandleDart = void Function(int);

typedef _InitLoggingC = Void Function(Pointer<Utf8> level);
typedef _InitLoggingDart = void Function(Pointer<Utf8> level);

// Recording + mock replay (RFC-0023). int-returning functions return 0 on
// success or -1 on invalid input; mock_replay_new returns a constructor-style
// JSON string owned by the caller (free with aimux_free_string).
typedef _RecordingDirC = Int32 Function(Pointer<Utf8> dir);
typedef _RecordingDirDart = int Function(Pointer<Utf8> dir);
typedef _RecordingRingC = Int32 Function(Uint64 cap);
typedef _RecordingRingDart = int Function(int cap);
typedef _RecordingNoArgC = Int32 Function();
typedef _RecordingNoArgDart = int Function();
typedef _MockReplayC = Pointer<Utf8> Function(Pointer<Utf8> recordingsJsonl);
typedef _MockReplayDart = Pointer<Utf8> Function(Pointer<Utf8> recordingsJsonl);

typedef _FreeStringC = Void Function(Pointer<Utf8>);
typedef _FreeStringDart = void Function(Pointer<Utf8>);

// Multi-arg constructor C signatures (bedrock/vertex/azure).
typedef _FourStrC = Pointer<Utf8> Function(
    Pointer<Utf8>, Pointer<Utf8>, Pointer<Utf8>, Pointer<Utf8>);
typedef _FourStrDart = Pointer<Utf8> Function(
    Pointer<Utf8>, Pointer<Utf8>, Pointer<Utf8>, Pointer<Utf8>);
typedef _FiveStrC = Pointer<Utf8> Function(
    Pointer<Utf8>, Pointer<Utf8>, Pointer<Utf8>, Pointer<Utf8>, Pointer<Utf8>);
typedef _FiveStrDart = Pointer<Utf8> Function(
    Pointer<Utf8>, Pointer<Utf8>, Pointer<Utf8>, Pointer<Utf8>, Pointer<Utf8>);
// Azure constructor C signatures: 3 required strings + a nullable
// api_version (the C ABI's `const char *api_version` may be NULL).
typedef _FourStrOptC = Pointer<Utf8> Function(
    Pointer<Utf8>, Pointer<Utf8>, Pointer<Utf8>, Pointer<Utf8>?);
typedef _FourStrOptDart = Pointer<Utf8> Function(
    Pointer<Utf8>, Pointer<Utf8>, Pointer<Utf8>, Pointer<Utf8>?);

// Stream callback C signatures
typedef _OnPartC = Void Function(Pointer<Utf8>);
typedef _OnDoneC = Void Function();
typedef _OnErrorC = Void Function(Pointer<Utf8>);

// ─────────────────────────────────────────────────────────────────────────────
// FFI library wrapper
// ─────────────────────────────────────────────────────────────────────────────

final class _AimuxFFI {
  final DynamicLibrary _lib;
  final Pointer<Utf8> Function(Pointer<Utf8>, Pointer<Utf8>) openaiNew;
  final Pointer<Utf8> Function(Pointer<Utf8>, Pointer<Utf8>) anthropicNew;
  final Pointer<Utf8> Function(Pointer<Utf8>, Pointer<Utf8>, Pointer<Utf8>)
      openaiNewWithBase;
  final Pointer<Utf8> Function(Pointer<Utf8>, Pointer<Utf8>, Pointer<Utf8>)
      anthropicNewWithBase;
  final Pointer<Utf8> Function(
      Pointer<Utf8>, Pointer<Utf8>?, Pointer<Utf8>, Pointer<Utf8>?) providerNew;
  final Pointer<Utf8> Function(
      Pointer<Utf8>, Pointer<Utf8>?, Pointer<Utf8>?) providerHandleNew;
  final Pointer<Utf8> Function(int) providerListModels;
  final Pointer<Utf8> Function(int, Pointer<Utf8>) providerModel;
  final Pointer<Utf8> Function(Pointer<Utf8>?) getModelSpecs;
  final Pointer<Utf8> Function(int, Pointer<Utf8>, Pointer<Utf8>?) generateText;
  final void Function(
      int,
      Pointer<Utf8>,
      Pointer<Utf8>?,
      Pointer<NativeFunction<_OnPartC>>,
      Pointer<NativeFunction<_OnDoneC>>,
      Pointer<NativeFunction<_OnErrorC>>) streamText;
  final Pointer<Utf8> Function(int, Pointer<Utf8>, Pointer<Utf8>?)
      generateTextAsOpenAI;
  final void Function(
      int,
      Pointer<Utf8>,
      Pointer<Utf8>?,
      Pointer<NativeFunction<_OnPartC>>,
      Pointer<NativeFunction<_OnDoneC>>,
      Pointer<NativeFunction<_OnErrorC>>) streamTextAsOpenAI;
  final void Function(int) dropHandle;
  final void Function(Pointer<Utf8>) freeString;
  final void Function(Pointer<Utf8>) initLogging;
  final int Function(Pointer<Utf8>) initRecording;
  final int Function(int) initRecordingRing;
  final int Function() recordingStop;
  final int Function() recordingFlush;
  final Pointer<Utf8> Function(Pointer<Utf8>) mockReplayNew;

  _AimuxFFI._(
      this._lib,
      this.openaiNew,
      this.anthropicNew,
      this.openaiNewWithBase,
      this.anthropicNewWithBase,
      this.providerNew,
      this.providerHandleNew,
      this.providerListModels,
      this.providerModel,
      this.getModelSpecs,
      this.generateText,
      this.streamText,
      this.generateTextAsOpenAI,
      this.streamTextAsOpenAI,
      this.dropHandle,
      this.freeString,
      this.initLogging,
      this.initRecording,
      this.initRecordingRing,
      this.recordingStop,
      this.recordingFlush,
      this.mockReplayNew);

  factory _AimuxFFI() {
    final dylib = _openLibrary();

    return _AimuxFFI._(
      dylib,
      dylib.lookupFunction<_OpenaiNewC, _OpenaiNewDart>('aimux_openai_new'),
      dylib.lookupFunction<_OpenaiNewC, _OpenaiNewDart>('aimux_anthropic_new'),
      dylib.lookupFunction<_NewWithBaseC, _NewWithBaseDart>(
          'aimux_openai_new_with_base'),
      dylib.lookupFunction<_NewWithBaseC, _NewWithBaseDart>(
          'aimux_anthropic_new_with_base'),
      dylib.lookupFunction<_ProviderNewC, _ProviderNewDart>(
          'aimux_provider_new'),
      dylib.lookupFunction<_ProviderHandleNewC, _ProviderHandleNewDart>(
          'aimux_provider_handle_new'),
      dylib.lookupFunction<_ProviderListModelsC, _ProviderListModelsDart>(
          'aimux_provider_list_models'),
      dylib.lookupFunction<_ProviderModelC, _ProviderModelDart>(
          'aimux_provider_model'),
      dylib.lookupFunction<_GetModelSpecsC, _GetModelSpecsDart>(
          'aimux_get_model_specs'),
      dylib.lookupFunction<_GenerateTextC, _GenerateTextDart>(
          'aimux_generate_text'),
      dylib.lookupFunction<_StreamTextC, _StreamTextDart>('aimux_stream_text'),
      dylib.lookupFunction<_GenerateTextC, _GenerateTextDart>(
          'aimux_generate_text_as_openai'),
      dylib.lookupFunction<_StreamTextC, _StreamTextDart>(
          'aimux_stream_text_as_openai'),
      dylib.lookupFunction<_DropHandleC, _DropHandleDart>('aimux_drop_handle'),
      dylib.lookupFunction<_FreeStringC, _FreeStringDart>('aimux_free_string'),
      dylib.lookupFunction<_InitLoggingC, _InitLoggingDart>('aimux_init_logging'),
      dylib.lookupFunction<_RecordingDirC, _RecordingDirDart>(
          'aimux_init_recording'),
      dylib.lookupFunction<_RecordingRingC, _RecordingRingDart>(
          'aimux_init_recording_ring'),
      dylib.lookupFunction<_RecordingNoArgC, _RecordingNoArgDart>(
          'aimux_recording_stop'),
      dylib.lookupFunction<_RecordingNoArgC, _RecordingNoArgDart>(
          'aimux_recording_flush'),
      dylib.lookupFunction<_MockReplayC, _MockReplayDart>(
          'aimux_mock_replay_new'),
    );
  }

  /// Opens the aimux-ffi native library for the current platform.
  ///
  /// The library ships inside this Flutter plugin package:
  ///   - Android: `libaimux_ffi.so` per ABI under android/src/main/jniLibs,
  ///     loaded by name (the Android build system bundles it into the APK).
  ///   - iOS:     `libaimux_ffi.a` is statically linked into the app binary
  ///     via ios/aimux.podspec (vendored + force_load), so symbols resolve
  ///     from the process itself.
  ///   - Desktop: looked up on the platform library path (dev/test usage).
  static DynamicLibrary _openLibrary() {
    if (Platform.isAndroid) return DynamicLibrary.open('libaimux_ffi.so');
    if (Platform.isIOS) return DynamicLibrary.process();
    return DynamicLibrary.open(_platformLibName());
  }

  static String _platformLibName() {
    if (Platform.isLinux) return 'libaimux_ffi.so';
    if (Platform.isMacOS) return 'libaimux_ffi.dylib';
    if (Platform.isWindows) return 'aimux_ffi.dll';
    throw UnsupportedError('Unsupported platform');
  }

  // ── Native protocol constructors (C ABI 0.2.0) ──────────────────────────

  Pointer<Utf8> cohereNew(Pointer<Utf8> key, Pointer<Utf8> model) =>
      _lib.lookupFunction<_OpenaiNewC, _OpenaiNewDart>('aimux_cohere_new')(
          key, model);
  Pointer<Utf8> cohereNewWithBase(
          Pointer<Utf8> key, Pointer<Utf8> model, Pointer<Utf8> base) =>
      _lib.lookupFunction<_NewWithBaseC, _NewWithBaseDart>(
          'aimux_cohere_new_with_base')(key, model, base);
  Pointer<Utf8> mistralNew(Pointer<Utf8> key, Pointer<Utf8> model) =>
      _lib.lookupFunction<_OpenaiNewC, _OpenaiNewDart>('aimux_mistral_new')(
          key, model);
  Pointer<Utf8> mistralNewWithBase(
          Pointer<Utf8> key, Pointer<Utf8> model, Pointer<Utf8> base) =>
      _lib.lookupFunction<_NewWithBaseC, _NewWithBaseDart>(
          'aimux_mistral_new_with_base')(key, model, base);
  Pointer<Utf8> xaiNew(Pointer<Utf8> key, Pointer<Utf8> model) => _lib
      .lookupFunction<_OpenaiNewC, _OpenaiNewDart>('aimux_xai_new')(key, model);
  Pointer<Utf8> xaiNewWithBase(
          Pointer<Utf8> key, Pointer<Utf8> model, Pointer<Utf8> base) =>
      _lib.lookupFunction<_NewWithBaseC, _NewWithBaseDart>(
          'aimux_xai_new_with_base')(key, model, base);
  Pointer<Utf8> bedrockNew(Pointer<Utf8> a, Pointer<Utf8> b, Pointer<Utf8> c,
          Pointer<Utf8> model) =>
      _lib.lookupFunction<_FourStrC, _FourStrDart>('aimux_bedrock_new')(
          a, b, c, model);
  Pointer<Utf8> bedrockNewWithBase(Pointer<Utf8> a, Pointer<Utf8> b,
          Pointer<Utf8> c, Pointer<Utf8> model, Pointer<Utf8> base) =>
      _lib.lookupFunction<_FiveStrC, _FiveStrDart>(
          'aimux_bedrock_new_with_base')(a, b, c, model, base);
  Pointer<Utf8> vertexNew(Pointer<Utf8> a, Pointer<Utf8> b, Pointer<Utf8> c,
          Pointer<Utf8> model) =>
      _lib.lookupFunction<_FourStrC, _FourStrDart>('aimux_vertex_new')(
          a, b, c, model);
  Pointer<Utf8> vertexNewWithBase(Pointer<Utf8> a, Pointer<Utf8> b,
          Pointer<Utf8> c, Pointer<Utf8> model, Pointer<Utf8> base) =>
      _lib.lookupFunction<_FiveStrC, _FiveStrDart>(
          'aimux_vertex_new_with_base')(a, b, c, model, base);
  Pointer<Utf8> anthropicAwsNew(
          Pointer<Utf8> key, Pointer<Utf8> region, Pointer<Utf8> model) =>
      _lib.lookupFunction<_NewWithBaseC, _NewWithBaseDart>(
          'aimux_anthropic_aws_new')(key, region, model);
  Pointer<Utf8> anthropicAwsNewWithBase(Pointer<Utf8> key, Pointer<Utf8> region,
          Pointer<Utf8> model, Pointer<Utf8> base) =>
      _lib.lookupFunction<_FourStrC, _FourStrDart>(
          'aimux_anthropic_aws_new_with_base')(key, region, model, base);
  Pointer<Utf8> azureNew(Pointer<Utf8> key, Pointer<Utf8> resource,
          Pointer<Utf8> deployment, Pointer<Utf8>? version) =>
      _lib.lookupFunction<_FourStrOptC, _FourStrOptDart>('aimux_azure_new')(
          key, resource, deployment, version);
  Pointer<Utf8> azureNewWithBase(Pointer<Utf8> key, Pointer<Utf8> base,
          Pointer<Utf8> deployment, Pointer<Utf8>? version) =>
      _lib.lookupFunction<_FourStrOptC, _FourStrOptDart>(
          'aimux_azure_new_with_base')(key, base, deployment, version);
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

// Parse a constructor's JSON result (`{"handle":<u64>}` on success,
// `{"error":"..."}` on failure), free the pointer, and return the handle.
int _extractHandle(_AimuxFFI ffi, Pointer<Utf8> ptr) {
  if (ptr == nullptr) throw StateError('constructor returned null');
  final result = ptr.toDartString();
  ffi.freeString(ptr);
  final decoded = jsonDecode(result);
  if (decoded is Map<String, dynamic>) {
    final error = decoded['error'];
    if (error is String) throw StateError(error);
    final handle = decoded['handle'];
    if (handle is int) return handle;
  }
  throw StateError('invalid constructor response: $result');
}

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
    final ptr = _withUtf8(apiKey, (keyPtr) {
      return _withUtf8(modelId, (idPtr) {
        if (baseUrl == null) {
          return ffi.openaiNew(keyPtr, idPtr);
        }
        return _withUtf8(baseUrl,
            (basePtr) => ffi.openaiNewWithBase(keyPtr, idPtr, basePtr));
      });
    });
    return Model._(_extractHandle(ffi, ptr), ffi);
  }

  /// Create an Anthropic model instance.
  ///
  /// Pass [baseUrl] to target a custom Anthropic-compatible endpoint. When
  /// null, the provider's standard URL is used via `aimux_anthropic_new`.
  factory Model.anthropic(String apiKey, String modelId, {String? baseUrl}) {
    final ffi = _AimuxFFI();
    final ptr = _withUtf8(apiKey, (keyPtr) {
      return _withUtf8(modelId, (idPtr) {
        if (baseUrl == null) {
          return ffi.anthropicNew(keyPtr, idPtr);
        }
        return _withUtf8(baseUrl,
            (basePtr) => ffi.anthropicNewWithBase(keyPtr, idPtr, basePtr));
      });
    });
    return Model._(_extractHandle(ffi, ptr), ffi);
  }

  /// Create a Cohere model instance.
  factory Model.cohere(String apiKey, String modelId, {String? baseUrl}) {
    final ffi = _AimuxFFI();
    final ptr = _withUtf8(apiKey, (keyPtr) {
      return _withUtf8(modelId, (idPtr) {
        if (baseUrl == null) {
          return ffi.cohereNew(keyPtr, idPtr);
        }
        return _withUtf8(baseUrl,
            (basePtr) => ffi.cohereNewWithBase(keyPtr, idPtr, basePtr));
      });
    });
    return Model._(_extractHandle(ffi, ptr), ffi);
  }

  /// Create a Mistral model instance.
  factory Model.mistral(String apiKey, String modelId, {String? baseUrl}) {
    final ffi = _AimuxFFI();
    final ptr = _withUtf8(apiKey, (keyPtr) {
      return _withUtf8(modelId, (idPtr) {
        if (baseUrl == null) {
          return ffi.mistralNew(keyPtr, idPtr);
        }
        return _withUtf8(baseUrl,
            (basePtr) => ffi.mistralNewWithBase(keyPtr, idPtr, basePtr));
      });
    });
    return Model._(_extractHandle(ffi, ptr), ffi);
  }

  /// Create an xAI model instance.
  factory Model.xai(String apiKey, String modelId, {String? baseUrl}) {
    final ffi = _AimuxFFI();
    final ptr = _withUtf8(apiKey, (keyPtr) {
      return _withUtf8(modelId, (idPtr) {
        if (baseUrl == null) {
          return ffi.xaiNew(keyPtr, idPtr);
        }
        return _withUtf8(
            baseUrl, (basePtr) => ffi.xaiNewWithBase(keyPtr, idPtr, basePtr));
      });
    });
    return Model._(_extractHandle(ffi, ptr), ffi);
  }

  /// Create a Bedrock model instance (AWS SigV4 credentials).
  factory Model.bedrock(
      String accessKeyId, String secretAccessKey, String region, String modelId,
      {String? baseUrl}) {
    final ffi = _AimuxFFI();
    final ptr = _withUtf8(accessKeyId, (aPtr) {
      return _withUtf8(secretAccessKey, (bPtr) {
        return _withUtf8(region, (cPtr) {
          return _withUtf8(modelId, (idPtr) {
            if (baseUrl == null) {
              return ffi.bedrockNew(aPtr, bPtr, cPtr, idPtr);
            }
            return _withUtf8(
                baseUrl,
                (basePtr) =>
                    ffi.bedrockNewWithBase(aPtr, bPtr, cPtr, idPtr, basePtr));
          });
        });
      });
    });
    return Model._(_extractHandle(ffi, ptr), ffi);
  }

  /// Create a Vertex AI model instance (GCP bearer token).
  factory Model.vertex(
      String accessToken, String project, String location, String modelId,
      {String? baseUrl}) {
    final ffi = _AimuxFFI();
    final ptr = _withUtf8(accessToken, (aPtr) {
      return _withUtf8(project, (bPtr) {
        return _withUtf8(location, (cPtr) {
          return _withUtf8(modelId, (idPtr) {
            if (baseUrl == null) {
              return ffi.vertexNew(aPtr, bPtr, cPtr, idPtr);
            }
            return _withUtf8(
                baseUrl,
                (basePtr) =>
                    ffi.vertexNewWithBase(aPtr, bPtr, cPtr, idPtr, basePtr));
          });
        });
      });
    });
    return Model._(_extractHandle(ffi, ptr), ffi);
  }

  /// Create an Anthropic-on-AWS model instance (API key + region).
  factory Model.anthropicAws(String apiKey, String region, String modelId,
      {String? baseUrl}) {
    final ffi = _AimuxFFI();
    final ptr = _withUtf8(apiKey, (keyPtr) {
      return _withUtf8(region, (rPtr) {
        return _withUtf8(modelId, (idPtr) {
          if (baseUrl == null) {
            return ffi.anthropicAwsNew(keyPtr, rPtr, idPtr);
          }
          return _withUtf8(
              baseUrl,
              (basePtr) =>
                  ffi.anthropicAwsNewWithBase(keyPtr, rPtr, idPtr, basePtr));
        });
      });
    });
    return Model._(_extractHandle(ffi, ptr), ffi);
  }

  /// Create an Azure OpenAI model instance (API key + resource name).
  /// The deployment name is passed as [deployment]; [apiVersion] is optional.
  factory Model.azure(String apiKey, String resourceName, String deployment,
      {String? apiVersion, String? baseUrl}) {
    final ffi = _AimuxFFI();
    final ptr = _withUtf8(apiKey, (keyPtr) {
      return _withUtf8(deployment, (dPtr) {
        final vPtr = apiVersion != null ? apiVersion.toNativeUtf8() : nullptr;
        try {
          if (baseUrl == null) {
            return _withUtf8(
                resourceName, (rPtr) => ffi.azureNew(keyPtr, rPtr, dPtr, vPtr));
          }
          return _withUtf8(baseUrl,
              (bPtr) => ffi.azureNewWithBase(keyPtr, bPtr, dPtr, vPtr));
        } finally {
          if (vPtr != nullptr) calloc.free(vPtr);
        }
      });
    });
    return Model._(_extractHandle(ffi, ptr), ffi);
  }

  /// Create a model from the provider registry by name (RFC-0017 phase 4).
  ///
  /// [apiKey] may be null to read the provider's env var from the registry
  /// entry. [configJson] is an optional JSON object of ProviderOptions
  /// (`{"base_url": "...", "headers": {...}, "max_retries": 0, "body_overrides": {...}}`).
  factory Model.provider(String name, String modelId,
      {String? apiKey, String? configJson}) {
    final ffi = _AimuxFFI();
    final ptr = _withUtf8(name, (namePtr) {
      return _withUtf8(modelId, (idPtr) {
        final keyPtr = apiKey != null ? apiKey.toNativeUtf8() : nullptr;
        final cfgPtr = configJson != null ? configJson.toNativeUtf8() : nullptr;
        try {
          return ffi.providerNew(namePtr, keyPtr, idPtr, cfgPtr);
        } finally {
          if (keyPtr != nullptr) calloc.free(keyPtr);
          if (cfgPtr != nullptr) calloc.free(cfgPtr);
        }
      });
    });
    return Model._(_extractHandle(ffi, ptr), ffi);
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
        if (resultPtr == nullptr)
          throw StateError('generate_text returned null');
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

    final resultStr = _withUtf8(promptJson, (promptPtr) {
      final optsPtr = optsJson != null ? optsJson.toNativeUtf8() : nullptr;
      try {
        final resultPtr =
            _ffi.generateTextAsOpenAI(_handle, promptPtr, optsPtr);
        if (resultPtr == nullptr)
          throw StateError('generate_text_as_openai returned null');
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

  /// Stream text from the model with OpenAI Chat Completions output
  /// (RFC-0026).
  ///
  /// Returns a Stream of ChatCompletionChunk maps (OpenAI
  /// `chat.completion.chunk` objects). Blocks the current isolate until the
  /// stream completes. Stream options (`include_usage`, `include_reasoning`)
  /// are passed via `options['provider_options']['openai']['stream_options']`.
  Stream<Map<String, dynamic>> streamTextAsOpenAI(
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

        _ffi.streamTextAsOpenAI(
            _handle, promptPtr, optsPtr, partCb, doneCb, errCb);

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
/// Call [close] to release the native handle.
///
/// ```dart
/// final p = createProvider('deepseek', apiKey);
/// try {
///   final models = await p.listModels();
///   final model = await p.model('deepseek-chat');
///   try {
///     print(model.generateText('Hello'));
///   } finally {
///     model.close();
///   }
/// } finally {
///   p.close();
/// }
/// ```
class ProviderHandle {
  final int _handle;
  final _AimuxFFI _ffi;
  bool _closed = false;

  ProviderHandle._(this._handle, this._ffi);

  /// List models available on this provider (runtime discovery via the
  /// provider's `/models` endpoint), enriched with community knowledge
  /// (anya2a) when available.
  ///
  /// Returns a JSON array string of `ResolvedModel`
  /// (`[{"id":"...","owned_by":"...","created":<u64>,"spec":{...}}, ...]`).
  /// Blocks the calling isolate until the network call completes.
  Future<String> listModels() async {
    _checkOpen();
    final ptr = _ffi.providerListModels(_handle);
    if (ptr == nullptr) throw StateError('provider_list_models returned null');
    final result = ptr.toDartString();
    _ffi.freeString(ptr);
    final decoded = jsonDecode(result);
    if (decoded is Map<String, dynamic>) {
      final error = decoded['error'];
      if (error is String) throw StateError(error);
      throw StateError('invalid provider_list_models response: $result');
    }
    // A JSON array of ResolvedModel on success.
    return result;
  }

  /// Build a language [Model] from a discovered [modelId].
  ///
  /// The returned [Model] is backed by a fresh native handle (separate from
  /// this provider handle) and is usable with `generateText` / `streamText`.
  /// The caller owns the returned [Model] and must [Model.close] it.
  Future<Model> model(String modelId) async {
    _checkOpen();
    final ptr = _withUtf8(modelId,
        (idPtr) => _ffi.providerModel(_handle, idPtr));
    return Model._(_extractHandle(_ffi, ptr), _ffi);
  }

  /// Release the native provider handle. Safe to call multiple times.
  void close() {
    if (!_closed) {
      _ffi.dropHandle(_handle);
      _closed = true;
    }
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
  final ffi = _AimuxFFI();
  final configJson = config?.toJson();
  final ptr = _withUtf8(name, (namePtr) {
    final keyPtr = apiKey != null ? apiKey.toNativeUtf8() : nullptr;
    final cfgPtr = configJson != null ? configJson.toNativeUtf8() : nullptr;
    try {
      return ffi.providerHandleNew(namePtr, keyPtr, cfgPtr);
    } finally {
      if (keyPtr != nullptr) calloc.free(keyPtr);
      if (cfgPtr != nullptr) calloc.free(cfgPtr);
    }
  });
  return ProviderHandle._(_extractHandle(ffi, ptr), ffi);
}

// ─────────────────────────────────────────────────────────────────────────────
// Model specs (RFC-0027) — get_model_specs
// ─────────────────────────────────────────────────────────────────────────────

/// Fetch the community model catalogue (anya2a). Returns a JSON-serialized
/// Catalogue string. Thin fetch — no caching.
String getModelSpecs(String? sourceUrl) {
  Pointer<Utf8>? urlPtr;
  if (sourceUrl != null) {
    urlPtr = sourceUrl.toNativeUtf8();
  }
  final ptr = _ffi.getModelSpecs(urlPtr);
  if (urlPtr != null) calloc.free(urlPtr);
  final result = _extractString(ptr, 'getModelSpecs');
  return result;
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
  final ffi = _AimuxFFI();
  _withUtf8(level.isEmpty ? 'warn' : level, ffi.initLogging);
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
int initRecording(String dir) {
  final ffi = _AimuxFFI();
  return _withUtf8(dir, ffi.initRecording);
}

/// Start in-memory bounded recording (ring recorder, FIFO eviction, dropped
/// count queryable). Returns 0, or -1 when [cap] == 0.
int initRecordingRing(int cap) {
  final ffi = _AimuxFFI();
  return ffi.initRecordingRing(cap);
}

/// Stop recording: the global recorder becomes None. Returns 0.
int recordingStop() {
  final ffi = _AimuxFFI();
  return ffi.recordingStop();
}

/// Flush the global recorder (blocks until JSONL is on disk; no-op for the
/// ring recorder). Returns 0.
int recordingFlush() {
  final ffi = _AimuxFFI();
  return ffi.recordingFlush();
}

/// Create a mock replay model from recorded JSONL (one `Recording` per line).
///
/// Matches recorded inputs and returns the recorded responses — no real API
/// is sent. The returned [Model] works with [Model.generateText] /
/// [Model.streamText]. Call [Model.close] to release the native handle.
Model mockReplay(String recordingsJsonl) {
  final ffi = _AimuxFFI();
  final ptr = _withUtf8(recordingsJsonl, ffi.mockReplayNew);
  return Model._(_extractHandle(ffi, ptr), ffi);
}
