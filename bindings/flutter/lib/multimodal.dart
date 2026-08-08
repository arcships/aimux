// multimodal.dart — Flutter/Dart binding for aimux multimodal models
// (dart:ffi, C ABI path).
//
// Mirrors the Go binding's multimodal.go: 8 modality model types
// (Embedding, Speech, Transcription, Image, Video, Reranking, Search, Files),
// each wrapping a native handle acquired via a provider-specific constructor
// and released via [close]. All cross-boundary data uses JSON strings (base64
// for binary), matching the C ABI wire format.
//
// This is the C ABI path (RFC §3.2) — same pattern as `aimux.dart`. The native
// library (libaimux_ffi.so / .dylib / .dll) must be on the library path or
// bundled in the Flutter app. All calls are synchronous (the C ABI blocks until
// completion).
//
// Engine failures throw [AimuxException] (see errors.dart).

import 'dart:ffi';
import 'package:ffi/ffi.dart';

import 'errors.dart';

// ─────────────────────────────────────────────────────────────────────────────
// FFI type aliases (AimuxError *err trailing out-param)
// ─────────────────────────────────────────────────────────────────────────────

// Constructors taking (api_key, model_id, err) → uint64_t handle.
typedef _NewC = Uint64 Function(
    Pointer<Utf8> apiKey, Pointer<Utf8> modelId, Pointer<AimuxCError> err);
typedef _NewDart = int Function(
    Pointer<Utf8> apiKey, Pointer<Utf8> modelId, Pointer<AimuxCError> err);

// Constructors taking (api_key, model_id, base_url, err) → handle.
typedef _NewWithBaseC = Uint64 Function(Pointer<Utf8> apiKey,
    Pointer<Utf8> modelId, Pointer<Utf8> baseUrl, Pointer<AimuxCError> err);
typedef _NewWithBaseDart = int Function(Pointer<Utf8> apiKey,
    Pointer<Utf8> modelId, Pointer<Utf8> baseUrl, Pointer<AimuxCError> err);

// Files constructors take only (api_key, err) — no model_id.
typedef _FilesNewC = Uint64 Function(
    Pointer<Utf8> apiKey, Pointer<AimuxCError> err);
typedef _FilesNewDart = int Function(
    Pointer<Utf8> apiKey, Pointer<AimuxCError> err);

// Files constructors with a custom base URL: (api_key, base_url, err).
typedef _FilesNewWithBaseC = Uint64 Function(
    Pointer<Utf8> apiKey, Pointer<Utf8> baseUrl, Pointer<AimuxCError> err);
typedef _FilesNewWithBaseDart = int Function(
    Pointer<Utf8> apiKey, Pointer<Utf8> baseUrl, Pointer<AimuxCError> err);

// aimux_embed(handle, values_json, opts_json, err) — opts nullable.
typedef _EmbedC = Pointer<Utf8> Function(Uint64 handle, Pointer<Utf8> valuesJson,
    Pointer<Utf8>? optsJson, Pointer<AimuxCError> err);
typedef _EmbedDart = Pointer<Utf8> Function(int handle, Pointer<Utf8> valuesJson,
    Pointer<Utf8>? optsJson, Pointer<AimuxCError> err);

// Single-arg generate functions: (handle, opts_json, err) → string.
typedef _GenerateOpts1C = Pointer<Utf8> Function(
    Uint64 handle, Pointer<Utf8> optsJson, Pointer<AimuxCError> err);
typedef _GenerateOpts1Dart = Pointer<Utf8> Function(
    int handle, Pointer<Utf8> optsJson, Pointer<AimuxCError> err);

// Three-arg generate functions: (handle, a, b, opts_json, err) → string.
typedef _GenerateOpts3C = Pointer<Utf8> Function(
    Uint64 handle,
    Pointer<Utf8> a,
    Pointer<Utf8> b,
    Pointer<Utf8>? optsJson,
    Pointer<AimuxCError> err);
typedef _GenerateOpts3Dart = Pointer<Utf8> Function(
    int handle,
    Pointer<Utf8> a,
    Pointer<Utf8> b,
    Pointer<Utf8>? optsJson,
    Pointer<AimuxCError> err);

// Resource management (same as aimux.dart).
typedef _DropHandleC = Void Function(Uint64);
typedef _DropHandleDart = void Function(int);

// ─────────────────────────────────────────────────────────────────────────────
// FFI library wrapper (process-wide singleton — dlopen + symbol lookups
// happen once, lazily per symbol)
// ─────────────────────────────────────────────────────────────────────────────

final class _MultimodalFFI {
  final DynamicLibrary _lib = openAimuxLibrary();

  // Constructors — (api_key, model_id, err)
  late final openaiEmbeddingNew =
      _lib.lookupFunction<_NewC, _NewDart>('aimux_openai_embedding_new');
  late final cohereEmbeddingNew =
      _lib.lookupFunction<_NewC, _NewDart>('aimux_cohere_embedding_new');
  late final googleEmbeddingNew =
      _lib.lookupFunction<_NewC, _NewDart>('aimux_google_embedding_new');
  late final openaiSpeechNew =
      _lib.lookupFunction<_NewC, _NewDart>('aimux_openai_speech_new');
  late final openaiImageNew =
      _lib.lookupFunction<_NewC, _NewDart>('aimux_openai_image_new');
  late final googleImageNew =
      _lib.lookupFunction<_NewC, _NewDart>('aimux_google_image_new');
  late final openaiTranscriptionNew =
      _lib.lookupFunction<_NewC, _NewDart>('aimux_openai_transcription_new');
  late final cohereRerankingNew =
      _lib.lookupFunction<_NewC, _NewDart>('aimux_cohere_reranking_new');
  late final googleVideoNew =
      _lib.lookupFunction<_NewC, _NewDart>('aimux_google_video_new');
  late final tavilySearchNew =
      _lib.lookupFunction<_NewC, _NewDart>('aimux_tavily_search_new');

  // Constructors — (api_key, model_id, base_url, err)
  late final openaiEmbeddingNewWithBase = _lib
      .lookupFunction<_NewWithBaseC, _NewWithBaseDart>(
          'aimux_openai_embedding_new_with_base');
  late final cohereEmbeddingNewWithBase = _lib
      .lookupFunction<_NewWithBaseC, _NewWithBaseDart>(
          'aimux_cohere_embedding_new_with_base');
  late final googleEmbeddingNewWithBase = _lib
      .lookupFunction<_NewWithBaseC, _NewWithBaseDart>(
          'aimux_google_embedding_new_with_base');
  late final openaiSpeechNewWithBase = _lib
      .lookupFunction<_NewWithBaseC, _NewWithBaseDart>(
          'aimux_openai_speech_new_with_base');
  late final openaiImageNewWithBase = _lib
      .lookupFunction<_NewWithBaseC, _NewWithBaseDart>(
          'aimux_openai_image_new_with_base');
  late final googleImageNewWithBase = _lib
      .lookupFunction<_NewWithBaseC, _NewWithBaseDart>(
          'aimux_google_image_new_with_base');
  late final openaiTranscriptionNewWithBase = _lib
      .lookupFunction<_NewWithBaseC, _NewWithBaseDart>(
          'aimux_openai_transcription_new_with_base');
  late final cohereRerankingNewWithBase = _lib
      .lookupFunction<_NewWithBaseC, _NewWithBaseDart>(
          'aimux_cohere_reranking_new_with_base');
  late final googleVideoNewWithBase = _lib
      .lookupFunction<_NewWithBaseC, _NewWithBaseDart>(
          'aimux_google_video_new_with_base');
  late final tavilySearchNewWithBase = _lib
      .lookupFunction<_NewWithBaseC, _NewWithBaseDart>(
          'aimux_tavily_search_new_with_base');

  // Files constructors
  late final openaiFilesNew =
      _lib.lookupFunction<_FilesNewC, _FilesNewDart>('aimux_openai_files_new');
  late final openaiFilesNewWithBase = _lib
      .lookupFunction<_FilesNewWithBaseC, _FilesNewWithBaseDart>(
          'aimux_openai_files_new_with_base');

  // Generate / embed / rerank / search / upload
  late final embed = _lib.lookupFunction<_EmbedC, _EmbedDart>('aimux_embed');
  late final speechGenerate = _lib
      .lookupFunction<_GenerateOpts1C, _GenerateOpts1Dart>(
          'aimux_speech_generate');
  late final imageGenerate = _lib
      .lookupFunction<_GenerateOpts1C, _GenerateOpts1Dart>(
          'aimux_image_generate');
  late final videoGenerate = _lib
      .lookupFunction<_GenerateOpts1C, _GenerateOpts1Dart>(
          'aimux_video_generate');
  late final rerank =
      _lib.lookupFunction<_GenerateOpts1C, _GenerateOpts1Dart>('aimux_rerank');
  late final search =
      _lib.lookupFunction<_GenerateOpts1C, _GenerateOpts1Dart>('aimux_search');
  late final transcriptionGenerate = _lib
      .lookupFunction<_GenerateOpts3C, _GenerateOpts3Dart>(
          'aimux_transcription_generate');
  late final fileUpload = _lib
      .lookupFunction<_GenerateOpts3C, _GenerateOpts3Dart>('aimux_file_upload');

  // Resource management
  late final dropHandle =
      _lib.lookupFunction<_DropHandleC, _DropHandleDart>('aimux_drop_handle');
}

/// Process-wide FFI table. Lazily created on first use; shared by every
/// multimodal model instance.
final _MultimodalFFI _ffi = _MultimodalFFI();

// ─────────────────────────────────────────────────────────────────────────────
// EmbeddingModel
// ─────────────────────────────────────────────────────────────────────────────

/// An embedding model backed by a native handle.
///
/// Call [close] to release the native handle.
class EmbeddingModel {
  final int _handle;
  bool _closed = false;

  EmbeddingModel._(this._handle);

  /// Create an OpenAI embedding model (e.g. text-embedding-3-small).
  ///
  /// Pass [baseUrl] to target an OpenAI-compatible endpoint. When null, the
  /// provider's standard URL is used via `aimux_openai_embedding_new`.
  factory EmbeddingModel.openai(String apiKey, String modelId,
          {String? baseUrl}) =>
      EmbeddingModel._(construct2(apiKey, modelId, baseUrl,
          _ffi.openaiEmbeddingNew, _ffi.openaiEmbeddingNewWithBase));

  /// Create a Cohere embedding model (e.g. embed-english-v3.0).
  factory EmbeddingModel.cohere(String apiKey, String modelId,
          {String? baseUrl}) =>
      EmbeddingModel._(construct2(apiKey, modelId, baseUrl,
          _ffi.cohereEmbeddingNew, _ffi.cohereEmbeddingNewWithBase));

  /// Create a Google embedding model (e.g. gemini-embedding-001).
  factory EmbeddingModel.google(String apiKey, String modelId,
          {String? baseUrl}) =>
      EmbeddingModel._(construct2(apiKey, modelId, baseUrl,
          _ffi.googleEmbeddingNew, _ffi.googleEmbeddingNewWithBase));

  /// Generate embeddings for [valuesJson] (a JSON array of strings).
  ///
  /// [optsJson] — optional serialized EmbeddingCallOptions.
  /// Returns the JSON-serialized EmbeddingResult, or throws [AimuxException].
  String embed(String valuesJson, [String? optsJson]) {
    _checkOpen();
    return withAimuxCError((err) {
      return withUtf8(valuesJson, (valuesPtr) {
        final optsPtr = optsJson != null ? optsJson.toNativeUtf8() : nullptr;
        try {
          final resultPtr = _ffi.embed(_handle, valuesPtr, optsPtr, err);
          return takeString(resultPtr, err);
        } finally {
          if (optsPtr != nullptr) calloc.free(optsPtr);
        }
      });
    });
  }

  /// Release the native handle. Safe to call multiple times.
  void close() {
    if (!_closed) {
      _ffi.dropHandle(_handle);
      _closed = true;
    }
  }

  void _checkOpen() {
    if (_closed) throw StateError('EmbeddingModel has been closed');
  }
}

// ─────────────────────────────────────────────────────────────────────────────
// SpeechModel (TTS)
// ─────────────────────────────────────────────────────────────────────────────

/// A speech (text-to-speech) model backed by a native handle.
///
/// Call [close] to release the native handle.
class SpeechModel {
  final int _handle;
  bool _closed = false;

  SpeechModel._(this._handle);

  /// Create an OpenAI speech (TTS) model.
  factory SpeechModel.openai(String apiKey, String modelId,
          {String? baseUrl}) =>
      SpeechModel._(construct2(apiKey, modelId, baseUrl, _ffi.openaiSpeechNew,
          _ffi.openaiSpeechNewWithBase));

  /// Generate speech audio from [optsJson] (serialized SpeechCallOptions).
  ///
  /// Returns the JSON-serialized SpeechResult, or throws [AimuxException].
  String generate(String optsJson) {
    _checkOpen();
    return withAimuxCError((err) {
      return withUtf8(optsJson, (optsPtr) {
        final resultPtr = _ffi.speechGenerate(_handle, optsPtr, err);
        return takeString(resultPtr, err);
      });
    });
  }

  /// Release the native handle. Safe to call multiple times.
  void close() {
    if (!_closed) {
      _ffi.dropHandle(_handle);
      _closed = true;
    }
  }

  void _checkOpen() {
    if (_closed) throw StateError('SpeechModel has been closed');
  }
}

// ─────────────────────────────────────────────────────────────────────────────
// TranscriptionModel (STT)
// ─────────────────────────────────────────────────────────────────────────────

/// A transcription (speech-to-text) model backed by a native handle.
///
/// Call [close] to release the native handle.
class TranscriptionModel {
  final int _handle;
  bool _closed = false;

  TranscriptionModel._(this._handle);

  /// Create an OpenAI transcription (STT) model.
  factory TranscriptionModel.openai(String apiKey, String modelId,
          {String? baseUrl}) =>
      TranscriptionModel._(construct2(apiKey, modelId, baseUrl,
          _ffi.openaiTranscriptionNew, _ffi.openaiTranscriptionNewWithBase));

  /// Transcribe audio to text.
  ///
  /// [audioBase64] — base64-encoded audio data.
  /// [mediaType] — media type of the audio (e.g. "audio/wav").
  /// [optsJson] — optional serialized TranscriptionCallOptions.
  /// Returns the JSON-serialized TranscriptionResult, or throws [AimuxException].
  String generate(String audioBase64, String mediaType, [String? optsJson]) {
    _checkOpen();
    return withAimuxCError((err) {
      return withUtf8(audioBase64, (audioPtr) {
        return withUtf8(mediaType, (mediaPtr) {
          final optsPtr = optsJson != null ? optsJson.toNativeUtf8() : nullptr;
          try {
            final resultPtr = _ffi.transcriptionGenerate(
                _handle, audioPtr, mediaPtr, optsPtr, err);
            return takeString(resultPtr, err);
          } finally {
            if (optsPtr != nullptr) calloc.free(optsPtr);
          }
        });
      });
    });
  }

  /// Release the native handle. Safe to call multiple times.
  void close() {
    if (!_closed) {
      _ffi.dropHandle(_handle);
      _closed = true;
    }
  }

  void _checkOpen() {
    if (_closed) throw StateError('TranscriptionModel has been closed');
  }
}

// ─────────────────────────────────────────────────────────────────────────────
// ImageModel
// ─────────────────────────────────────────────────────────────────────────────

/// An image generation model backed by a native handle.
///
/// Call [close] to release the native handle.
class ImageModel {
  final int _handle;
  bool _closed = false;

  ImageModel._(this._handle);

  /// Create an OpenAI image model (e.g. dall-e-3).
  factory ImageModel.openai(String apiKey, String modelId, {String? baseUrl}) =>
      ImageModel._(construct2(apiKey, modelId, baseUrl, _ffi.openaiImageNew,
          _ffi.openaiImageNewWithBase));

  /// Create a Google image model (e.g. gemini-2.5-flash-image).
  factory ImageModel.google(String apiKey, String modelId, {String? baseUrl}) =>
      ImageModel._(construct2(apiKey, modelId, baseUrl, _ffi.googleImageNew,
          _ffi.googleImageNewWithBase));

  /// Generate images from [optsJson] (serialized ImageCallOptions).
  ///
  /// Returns the JSON-serialized ImageResult, or throws [AimuxException].
  String generate(String optsJson) {
    _checkOpen();
    return withAimuxCError((err) {
      return withUtf8(optsJson, (optsPtr) {
        final resultPtr = _ffi.imageGenerate(_handle, optsPtr, err);
        return takeString(resultPtr, err);
      });
    });
  }

  /// Release the native handle. Safe to call multiple times.
  void close() {
    if (!_closed) {
      _ffi.dropHandle(_handle);
      _closed = true;
    }
  }

  void _checkOpen() {
    if (_closed) throw StateError('ImageModel has been closed');
  }
}

// ─────────────────────────────────────────────────────────────────────────────
// VideoModel
// ─────────────────────────────────────────────────────────────────────────────

/// A video generation model backed by a native handle.
///
/// Call [close] to release the native handle.
class VideoModel {
  final int _handle;
  bool _closed = false;

  VideoModel._(this._handle);

  /// Create a Google video model (e.g. veo-3.0).
  factory VideoModel.google(String apiKey, String modelId, {String? baseUrl}) =>
      VideoModel._(construct2(apiKey, modelId, baseUrl, _ffi.googleVideoNew,
          _ffi.googleVideoNewWithBase));

  /// Generate videos from [optsJson] (serialized VideoCallOptions).
  ///
  /// Returns the JSON-serialized VideoResult, or throws [AimuxException].
  String generate(String optsJson) {
    _checkOpen();
    return withAimuxCError((err) {
      return withUtf8(optsJson, (optsPtr) {
        final resultPtr = _ffi.videoGenerate(_handle, optsPtr, err);
        return takeString(resultPtr, err);
      });
    });
  }

  /// Release the native handle. Safe to call multiple times.
  void close() {
    if (!_closed) {
      _ffi.dropHandle(_handle);
      _closed = true;
    }
  }

  void _checkOpen() {
    if (_closed) throw StateError('VideoModel has been closed');
  }
}

// ─────────────────────────────────────────────────────────────────────────────
// RerankingModel
// ─────────────────────────────────────────────────────────────────────────────

/// A reranking model backed by a native handle.
///
/// Call [close] to release the native handle.
class RerankingModel {
  final int _handle;
  bool _closed = false;

  RerankingModel._(this._handle);

  /// Create a Cohere reranking model (e.g. rerank-v3.0).
  factory RerankingModel.cohere(String apiKey, String modelId,
          {String? baseUrl}) =>
      RerankingModel._(construct2(apiKey, modelId, baseUrl,
          _ffi.cohereRerankingNew, _ffi.cohereRerankingNewWithBase));

  /// Rerank documents against a query.
  ///
  /// [optsJson] — serialized RerankingCallOptions.
  /// Returns the JSON-serialized RerankingResult, or throws [AimuxException].
  String rerank(String optsJson) {
    _checkOpen();
    return withAimuxCError((err) {
      return withUtf8(optsJson, (optsPtr) {
        final resultPtr = _ffi.rerank(_handle, optsPtr, err);
        return takeString(resultPtr, err);
      });
    });
  }

  /// Release the native handle. Safe to call multiple times.
  void close() {
    if (!_closed) {
      _ffi.dropHandle(_handle);
      _closed = true;
    }
  }

  void _checkOpen() {
    if (_closed) throw StateError('RerankingModel has been closed');
  }
}

// ─────────────────────────────────────────────────────────────────────────────
// SearchModel
// ─────────────────────────────────────────────────────────────────────────────

/// A web search model backed by a native handle.
///
/// Call [close] to release the native handle.
class SearchModel {
  final int _handle;
  bool _closed = false;

  SearchModel._(this._handle);

  /// Create a Tavily search model. Tavily uses a fixed endpoint, so no model
  /// ID is needed — an empty string is passed (the C ABI ignores it).
  factory SearchModel.tavily(String apiKey, {String? baseUrl}) =>
      SearchModel._(construct2(apiKey, '', baseUrl, _ffi.tavilySearchNew,
          _ffi.tavilySearchNewWithBase));

  /// Perform a web search.
  ///
  /// [optsJson] — serialized SearchCallOptions.
  /// Returns the JSON-serialized SearchResult, or throws [AimuxException].
  String search(String optsJson) {
    _checkOpen();
    return withAimuxCError((err) {
      return withUtf8(optsJson, (optsPtr) {
        final resultPtr = _ffi.search(_handle, optsPtr, err);
        return takeString(resultPtr, err);
      });
    });
  }

  /// Release the native handle. Safe to call multiple times.
  void close() {
    if (!_closed) {
      _ffi.dropHandle(_handle);
      _closed = true;
    }
  }

  void _checkOpen() {
    if (_closed) throw StateError('SearchModel has been closed');
  }
}

// ─────────────────────────────────────────────────────────────────────────────
// Files
// ─────────────────────────────────────────────────────────────────────────────

/// A files manager (provider file uploads) backed by a native handle.
///
/// Call [close] to release the native handle.
class Files {
  final int _handle;
  bool _closed = false;

  Files._(this._handle);

  /// Create an OpenAI files manager. Files take only an API key (no model ID).
  factory Files.openai(String apiKey, {String? baseUrl}) {
    final handle = withAimuxCError((err) {
      return withUtf8(apiKey, (keyPtr) {
        if (baseUrl == null) {
          return takeHandle(_ffi.openaiFilesNew(keyPtr, err), err);
        }
        return withUtf8(baseUrl, (basePtr) {
          return takeHandle(
              _ffi.openaiFilesNewWithBase(keyPtr, basePtr, err), err);
        });
      });
    });
    return Files._(handle);
  }

  /// Upload a file to the provider.
  ///
  /// [dataBase64] — base64-encoded file data.
  /// [mediaType] — media type of the file (e.g. "application/pdf").
  /// [optsJson] — optional serialized UploadFileCallOptions.
  /// Returns the JSON-serialized UploadFileResult, or throws [AimuxException].
  String uploadFile(String dataBase64, String mediaType, [String? optsJson]) {
    _checkOpen();
    return withAimuxCError((err) {
      return withUtf8(dataBase64, (dataPtr) {
        return withUtf8(mediaType, (mediaPtr) {
          final optsPtr = optsJson != null ? optsJson.toNativeUtf8() : nullptr;
          try {
            final resultPtr =
                _ffi.fileUpload(_handle, dataPtr, mediaPtr, optsPtr, err);
            return takeString(resultPtr, err);
          } finally {
            if (optsPtr != nullptr) calloc.free(optsPtr);
          }
        });
      });
    });
  }

  /// Release the native handle. Safe to call multiple times.
  void close() {
    if (!_closed) {
      _ffi.dropHandle(_handle);
      _closed = true;
    }
  }

  void _checkOpen() {
    if (_closed) throw StateError('Files has been closed');
  }
}
