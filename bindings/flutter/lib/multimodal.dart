// multimodal.dart — Flutter/Dart binding for aimux multimodal models
// (dart:ffi, C ABI path).
//
// Mirrors the Go binding's multimodal.go: 8 modality model types
// (Embedding, Speech, Transcription, Image, Video, Reranking, Search, Files),
// each wrapping a native handle acquired via a provider-specific constructor
// and released via [close]. All C ABI data uses JSON strings (base64
// for binary), matching the C ABI wire format.
//
// This is the C ABI path (RFC §3.2) — same pattern as `aimux.dart`. The native
// library (libaimux_ffi.so / .dylib / .dll) must be on the library path or
// bundled in the Flutter app. All calls are synchronous (the C ABI blocks until
// completion).
//
// AiMuxError values throw [AimuxException] (see errors.dart).

import 'dart:ffi';
import 'dart:typed_data';
import 'package:ffi/ffi.dart';

import 'errors.dart';

// ─────────────────────────────────────────────────────────────────────────────
// FFI type aliases. `_Err` is `aimux_error_t *` (opaque; NULL = success);
// results come back through the trailing out-param. Pointer types are spelled
// the same in the C and Dart signatures, so a single alias serves both.
// ─────────────────────────────────────────────────────────────────────────────

typedef _Err = Pointer<Void>;

// Constructors taking (api_key, model_id, out_handle).
typedef _NewC = _Err Function(
    Pointer<Utf8> apiKey, Pointer<Utf8> modelId, Pointer<Uint64> outHandle);
typedef _NewDart = _Err Function(
    Pointer<Utf8> apiKey, Pointer<Utf8> modelId, Pointer<Uint64> outHandle);

// Constructors taking (api_key, model_id, base_url, out_handle).
typedef _NewWithBaseC = _Err Function(Pointer<Utf8> apiKey,
    Pointer<Utf8> modelId, Pointer<Utf8> baseUrl, Pointer<Uint64> outHandle);
typedef _NewWithBaseDart = _Err Function(Pointer<Utf8> apiKey,
    Pointer<Utf8> modelId, Pointer<Utf8> baseUrl, Pointer<Uint64> outHandle);

// Files constructors take only (api_key, out_handle) — no model_id.
typedef _FilesNewC = _Err Function(
    Pointer<Utf8> apiKey, Pointer<Uint64> outHandle);
typedef _FilesNewDart = _Err Function(
    Pointer<Utf8> apiKey, Pointer<Uint64> outHandle);

// Files constructors with a custom base URL: (api_key, base_url, out_handle).
typedef _FilesNewWithBaseC = _Err Function(
    Pointer<Utf8> apiKey, Pointer<Utf8> baseUrl, Pointer<Uint64> outHandle);
typedef _FilesNewWithBaseDart = _Err Function(
    Pointer<Utf8> apiKey, Pointer<Utf8> baseUrl, Pointer<Uint64> outHandle);

// aimux_embed(handle, values_json, opts_json, out_json) — opts nullable.
typedef _EmbedC = _Err Function(Uint64 handle, Pointer<Utf8> valuesJson,
    Pointer<Utf8>? optsJson, Pointer<Pointer<Utf8>> outJson);
typedef _EmbedDart = _Err Function(int handle, Pointer<Utf8> valuesJson,
    Pointer<Utf8>? optsJson, Pointer<Pointer<Utf8>> outJson);

// Single-arg generate functions: (handle, opts_json, out_json).
typedef _GenerateOpts1C = _Err Function(
    Uint64 handle, Pointer<Utf8> optsJson, Pointer<Pointer<Utf8>> outJson);
typedef _GenerateOpts1Dart = _Err Function(
    int handle, Pointer<Utf8> optsJson, Pointer<Pointer<Utf8>> outJson);

// Three-arg generate functions: (handle, a, b, opts_json, out_json).
typedef _GenerateOpts3C = _Err Function(
    Uint64 handle,
    Pointer<Utf8> a,
    Pointer<Utf8> b,
    Pointer<Utf8>? optsJson,
    Pointer<Pointer<Utf8>> outJson);
typedef _GenerateOpts3Dart = _Err Function(
    int handle,
    Pointer<Utf8> a,
    Pointer<Utf8> b,
    Pointer<Utf8>? optsJson,
    Pointer<Pointer<Utf8>> outJson);

// Transcription streaming sessions (RFC-0028).
typedef _TranscriptionSessionNewC = _Err Function(Uint64 modelHandle,
    Uint64 abortHandle, Pointer<Utf8> optsJson, Pointer<Uint64> outHandle);
typedef _TranscriptionSessionNewDart = _Err Function(int modelHandle,
    int abortHandle, Pointer<Utf8> optsJson, Pointer<Uint64> outHandle);
typedef _TranscriptionPushAudioC = _Err Function(
    Uint64 session, Pointer<Uint8> data, IntPtr len);
typedef _TranscriptionPushAudioDart = _Err Function(
    int session, Pointer<Uint8> data, int len);
typedef _TranscriptionInputDoneC = _Err Function(Uint64 session);
typedef _TranscriptionInputDoneDart = _Err Function(int session);
// out_state is `int32_t *` in the header (holds an
// aimux_transcription_next_part_state_t value), hence Pointer<Int32>.
typedef _TranscriptionNextPartC = _Err Function(Uint64 session,
    Int64 timeoutMs, Pointer<Pointer<Utf8>> outPart, Pointer<Int32> outState);
typedef _TranscriptionNextPartDart = _Err Function(int session,
    int timeoutMs, Pointer<Pointer<Utf8>> outPart, Pointer<Int32> outState);
typedef _VoidU64C = Void Function(Uint64);
typedef _VoidU64Dart = void Function(int);

// aimux_transcription_next_part_state_t values.
const int _nextPartStatePart = 1;
const int _nextPartStateEnded = 2;
const int _nextPartStateTimeout = 3;

// Resource management (same as aimux.dart).
typedef _DropHandleC = Void Function(Uint64);
typedef _DropHandleDart = void Function(int);

// ─────────────────────────────────────────────────────────────────────────────
// FFI library wrapper (process-wide singleton — dlopen + symbol lookups
// happen once, lazily per symbol)
// ─────────────────────────────────────────────────────────────────────────────

final class _MultimodalFFI {
  final DynamicLibrary _lib = openAimuxLibrary();

  // Constructors — (api_key, model_id, out_handle)
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

  // Constructors — (api_key, model_id, base_url, out_handle)
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

  // Transcription streaming sessions (RFC-0028).
  late final transcriptionSessionNew = _lib.lookupFunction<
      _TranscriptionSessionNewC,
      _TranscriptionSessionNewDart>('aimux_transcription_session_new');
  late final transcriptionPushAudio = _lib.lookupFunction<
      _TranscriptionPushAudioC,
      _TranscriptionPushAudioDart>('aimux_transcription_push_audio');
  late final transcriptionInputDone = _lib.lookupFunction<
      _TranscriptionInputDoneC,
      _TranscriptionInputDoneDart>('aimux_transcription_input_done');
  late final transcriptionNextPart = _lib.lookupFunction<
      _TranscriptionNextPartC,
      _TranscriptionNextPartDart>('aimux_transcription_next_part');
  late final transcriptionSessionDrop = _lib.lookupFunction<
      _VoidU64C,
      _VoidU64Dart>('aimux_transcription_session_drop');
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
    checkJson(valuesJson, 'values_json');
    checkJson(optsJson, 'opts_json', emptyIsDefault: true);
    return withUtf8(valuesJson, (valuesPtr) {
      final optsPtr = toCStringOrNull(optsJson);
      try {
        return takeString(
            (out) => _ffi.embed(_handle, valuesPtr, optsPtr, out), 'embed');
      } finally {
        if (optsPtr != nullptr) calloc.free(optsPtr);
      }
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
    if (_closed) throw StateError('EmbeddingModel is closed');
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
    checkJson(optsJson, 'opts_json');
    return withUtf8(optsJson, (optsPtr) {
      return takeString(
          (out) => _ffi.speechGenerate(_handle, optsPtr, out), 'speech_generate');
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
    if (_closed) throw StateError('SpeechModel is closed');
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
    checkJson(optsJson, 'opts_json', emptyIsDefault: true);
    return withUtf8(audioBase64, (audioPtr) {
      return withUtf8(mediaType, (mediaPtr) {
        final optsPtr = toCStringOrNull(optsJson);
        try {
          return takeString(
              (out) => _ffi.transcriptionGenerate(
                  _handle, audioPtr, mediaPtr, optsPtr, out),
              'transcription_generate');
        } finally {
          if (optsPtr != nullptr) calloc.free(optsPtr);
        }
      });
    });
  }

  /// Start a streaming transcription session (RFC-0028) on this model.
  /// Requires a model that supports streaming (realtime models such as
  /// gpt-realtime-whisper).
  ///
  /// [optsJson] — optional session options JSON
  /// (`{"input_audio_format": {...}, "provider_options", "headers",
  /// "include_raw_chunks"}`).
  /// [abortHandle] — optional abort handle; firing it aborts the session.
  TranscriptionSession startStream({String? optsJson, int abortHandle = 0}) {
    _checkOpen();
    checkJson(optsJson, 'opts_json', emptyIsDefault: true);
    final optsPtr = toCStringOrNull(optsJson);
    final int handle;
    try {
      handle = takeHandle(
          (out) =>
              _ffi.transcriptionSessionNew(_handle, abortHandle, optsPtr, out),
          'transcription_session_new');
    } finally {
      if (optsPtr != nullptr) calloc.free(optsPtr);
    }
    return TranscriptionSession._(handle);
  }

  /// Release the native handle. Safe to call multiple times.
  void close() {
    if (!_closed) {
      _ffi.dropHandle(_handle);
      _closed = true;
    }
  }

  void _checkOpen() {
    if (_closed) throw StateError('TranscriptionModel is closed');
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
    checkJson(optsJson, 'opts_json');
    return withUtf8(optsJson, (optsPtr) {
      return takeString(
          (out) => _ffi.imageGenerate(_handle, optsPtr, out), 'image_generate');
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
    if (_closed) throw StateError('ImageModel is closed');
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
    checkJson(optsJson, 'opts_json');
    return withUtf8(optsJson, (optsPtr) {
      return takeString(
          (out) => _ffi.videoGenerate(_handle, optsPtr, out), 'video_generate');
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
    if (_closed) throw StateError('VideoModel is closed');
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
    checkJson(optsJson, 'opts_json');
    return withUtf8(optsJson, (optsPtr) {
      return takeString(
          (out) => _ffi.rerank(_handle, optsPtr, out), 'rerank');
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
    if (_closed) throw StateError('RerankingModel is closed');
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
    checkJson(optsJson, 'opts_json');
    return withUtf8(optsJson, (optsPtr) {
      return takeString(
          (out) => _ffi.search(_handle, optsPtr, out), 'search');
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
    if (_closed) throw StateError('SearchModel is closed');
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
    final handle = withUtf8(apiKey, (keyPtr) {
      if (baseUrl == null) {
        return takeHandle(
            (out) => _ffi.openaiFilesNew(keyPtr, out), 'openai_files_new');
      }
      return withUtf8(baseUrl, (basePtr) {
        return takeHandle(
            (out) => _ffi.openaiFilesNewWithBase(keyPtr, basePtr, out),
            'openai_files_new_with_base');
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
    checkJson(optsJson, 'opts_json', emptyIsDefault: true);
    return withUtf8(dataBase64, (dataPtr) {
      return withUtf8(mediaType, (mediaPtr) {
        final optsPtr = toCStringOrNull(optsJson);
        try {
          return takeString(
              (out) =>
                  _ffi.fileUpload(_handle, dataPtr, mediaPtr, optsPtr, out),
              'file_upload');
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
    if (_closed) throw StateError('Files is closed');
  }
}

// ─────────────────────────────────────────────────────────────────────────────
// TranscriptionSession (RFC-0028 streaming)
// ─────────────────────────────────────────────────────────────────────────────

/// Thrown by [TranscriptionSession.nextPart] when the stream ended normally
/// (a `Finish` part was delivered earlier).
class AimuxTranscriptionEndedException implements Exception {
  @override
  String toString() => 'AimuxTranscriptionEndedException: stream ended';
}

/// Thrown by [TranscriptionSession.nextPart] when no part arrived within the
/// timeout. The session stays live — call again.
class AimuxTranscriptionTimeoutException implements Exception {
  @override
  String toString() => 'AimuxTranscriptionTimeoutException: part timeout';
}

/// A live streaming-transcription session (RFC-0028): push audio chunks with
/// [pushAudio], mark end-of-audio with [inputDone], then pull transcription
/// parts (JSON `TranscriptionStreamPart`s) with [nextPart].
///
/// Call [close] to terminate (aborts the driver; idempotent).
class TranscriptionSession {
  final int _handle;
  bool _closed = false;

  TranscriptionSession._(this._handle);

  void _checkOpen() {
    if (_closed) throw StateError('TranscriptionSession is closed');
  }

  /// Push one binary audio chunk. Blocks while the internal channel is full
  /// (backpressure propagation).
  void pushAudio(Uint8List audio) {
    _checkOpen();
    final ptr = audio.isEmpty
        ? Pointer<Uint8>.fromAddress(0)
        : calloc<Uint8>(audio.length);
    final allocated = audio.isNotEmpty;
    try {
      if (allocated) {
        ptr.asTypedList(audio.length).setAll(0, audio);
      }
      expectAimuxError(_ffi.transcriptionPushAudio(_handle, ptr, audio.length),
          'transcription_push_audio');
    } finally {
      if (allocated) calloc.free(ptr);
    }
  }

  /// Signal end-of-audio (idempotent).
  void inputDone() {
    _checkOpen();
    expectFfiError(
        _ffi.transcriptionInputDone(_handle), 'transcription_input_done');
  }

  /// Pull the next transcription part (JSON string).
  ///
  /// Throws [AimuxTranscriptionEndedException] when the stream finished
  /// normally, [AimuxTranscriptionTimeoutException] when no part arrived in
  /// time (a poll state, not an error — the session stays live; call again).
  /// `timeoutMs`: >0 wait at most; 0 immediate poll; negative = wait
  /// indefinitely.
  String nextPart(int timeoutMs) {
    _checkOpen();
    final outPart = calloc<Pointer<Utf8>>();
    final outState = calloc<Int32>();
    try {
      expectAimuxError(
          _ffi.transcriptionNextPart(_handle, timeoutMs, outPart, outState),
          'transcription_next_part');
      switch (outState.value) {
        case _nextPartStatePart:
          final p = outPart.value;
          if (p == nullptr) {
            throw StateError('aimux ffi: transcription_next_part: NULL part');
          }
          try {
            return p.toDartString();
          } finally {
            aimuxFreeString(p);
          }
        case _nextPartStateEnded:
          throw AimuxTranscriptionEndedException();
        case _nextPartStateTimeout:
          throw AimuxTranscriptionTimeoutException();
        default:
          throw StateError(
              'aimux ffi: transcription_next_part: unknown state ${outState.value}');
      }
    } finally {
      calloc.free(outPart);
      calloc.free(outState);
    }
  }

  /// Terminate and release the session (aborts the driver; idempotent).
  void close() {
    if (!_closed) {
      _ffi.transcriptionSessionDrop(_handle);
      _closed = true;
    }
  }
}
