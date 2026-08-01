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

import 'dart:convert';
import 'dart:ffi';
import 'dart:io';
import 'package:ffi/ffi.dart';

// ─────────────────────────────────────────────────────────────────────────────
// FFI type aliases
// ─────────────────────────────────────────────────────────────────────────────

// Constructors taking (api_key, model_id) → handle. Shared by every embedding,
// speech, image, transcription, reranking, video, and search provider.
typedef _NewC = Uint64 Function(Pointer<Utf8> apiKey, Pointer<Utf8> modelId);
typedef _NewDart = int Function(Pointer<Utf8> apiKey, Pointer<Utf8> modelId);

// Constructors taking (api_key, model_id, base_url) → handle (the _with_base
// variants). Same signature for every provider.
typedef _NewWithBaseC = Uint64 Function(
    Pointer<Utf8> apiKey, Pointer<Utf8> modelId, Pointer<Utf8> baseUrl);
typedef _NewWithBaseDart = int Function(
    Pointer<Utf8> apiKey, Pointer<Utf8> modelId, Pointer<Utf8> baseUrl);

// Files constructors take only (api_key) — no model_id.
typedef _FilesNewC = Uint64 Function(Pointer<Utf8> apiKey);
typedef _FilesNewDart = int Function(Pointer<Utf8> apiKey);

// Files constructors with a custom base URL: (api_key, base_url).
typedef _FilesNewWithBaseC = Uint64 Function(
    Pointer<Utf8> apiKey, Pointer<Utf8> baseUrl);
typedef _FilesNewWithBaseDart = int Function(
    Pointer<Utf8> apiKey, Pointer<Utf8> baseUrl);

// aimux_embed(handle, values_json, opts_json) — opts nullable.
typedef _EmbedC = Pointer<Utf8> Function(
    Uint64 handle, Pointer<Utf8> valuesJson, Pointer<Utf8>? optsJson);
typedef _EmbedDart = Pointer<Utf8> Function(
    int handle, Pointer<Utf8> valuesJson, Pointer<Utf8>? optsJson);

// Single-arg generate functions: (handle, opts_json) → string.
// Covers speech_generate, image_generate, video_generate, rerank, search.
typedef _GenerateOpts1C = Pointer<Utf8> Function(
    Uint64 handle, Pointer<Utf8> optsJson);
typedef _GenerateOpts1Dart = Pointer<Utf8> Function(
    int handle, Pointer<Utf8> optsJson);

// Three-arg generate functions: (handle, a, b, opts_json) → string, opts
// nullable. Covers transcription_generate and file_upload.
typedef _GenerateOpts3C = Pointer<Utf8> Function(Uint64 handle, Pointer<Utf8> a,
    Pointer<Utf8> b, Pointer<Utf8>? optsJson);
typedef _GenerateOpts3Dart = Pointer<Utf8> Function(int handle, Pointer<Utf8> a,
    Pointer<Utf8> b, Pointer<Utf8>? optsJson);

// Resource management (same as aimux.dart).
typedef _DropHandleC = Void Function(Uint64);
typedef _DropHandleDart = void Function(int);
typedef _FreeStringC = Void Function(Pointer<Utf8>);
typedef _FreeStringDart = void Function(Pointer<Utf8>);

// ─────────────────────────────────────────────────────────────────────────────
// FFI library wrapper
// ─────────────────────────────────────────────────────────────────────────────

final class _MultimodalFFI {
  // Constructors — (api_key, model_id)
  final int Function(Pointer<Utf8>, Pointer<Utf8>) openaiEmbeddingNew;
  final int Function(Pointer<Utf8>, Pointer<Utf8>) cohereEmbeddingNew;
  final int Function(Pointer<Utf8>, Pointer<Utf8>) googleEmbeddingNew;
  final int Function(Pointer<Utf8>, Pointer<Utf8>) openaiSpeechNew;
  final int Function(Pointer<Utf8>, Pointer<Utf8>) openaiImageNew;
  final int Function(Pointer<Utf8>, Pointer<Utf8>) googleImageNew;
  final int Function(Pointer<Utf8>, Pointer<Utf8>) openaiTranscriptionNew;
  final int Function(Pointer<Utf8>, Pointer<Utf8>) cohereRerankingNew;
  final int Function(Pointer<Utf8>, Pointer<Utf8>) googleVideoNew;
  final int Function(Pointer<Utf8>, Pointer<Utf8>) tavilySearchNew;

  // Constructors — (api_key, model_id, base_url)
  final int Function(Pointer<Utf8>, Pointer<Utf8>, Pointer<Utf8>)
      openaiEmbeddingNewWithBase;
  final int Function(Pointer<Utf8>, Pointer<Utf8>, Pointer<Utf8>)
      cohereEmbeddingNewWithBase;
  final int Function(Pointer<Utf8>, Pointer<Utf8>, Pointer<Utf8>)
      googleEmbeddingNewWithBase;
  final int Function(Pointer<Utf8>, Pointer<Utf8>, Pointer<Utf8>)
      openaiSpeechNewWithBase;
  final int Function(Pointer<Utf8>, Pointer<Utf8>, Pointer<Utf8>)
      openaiImageNewWithBase;
  final int Function(Pointer<Utf8>, Pointer<Utf8>, Pointer<Utf8>)
      googleImageNewWithBase;
  final int Function(Pointer<Utf8>, Pointer<Utf8>, Pointer<Utf8>)
      openaiTranscriptionNewWithBase;
  final int Function(Pointer<Utf8>, Pointer<Utf8>, Pointer<Utf8>)
      cohereRerankingNewWithBase;
  final int Function(Pointer<Utf8>, Pointer<Utf8>, Pointer<Utf8>)
      googleVideoNewWithBase;
  final int Function(Pointer<Utf8>, Pointer<Utf8>, Pointer<Utf8>)
      tavilySearchNewWithBase;

  // Files constructors
  final int Function(Pointer<Utf8>) openaiFilesNew;
  final int Function(Pointer<Utf8>, Pointer<Utf8>) openaiFilesNewWithBase;

  // Generate / embed / rerank / search / upload
  final Pointer<Utf8> Function(int, Pointer<Utf8>, Pointer<Utf8>?) embed;
  final Pointer<Utf8> Function(int, Pointer<Utf8>) speechGenerate;
  final Pointer<Utf8> Function(int, Pointer<Utf8>) imageGenerate;
  final Pointer<Utf8> Function(int, Pointer<Utf8>) videoGenerate;
  final Pointer<Utf8> Function(int, Pointer<Utf8>) rerank;
  final Pointer<Utf8> Function(int, Pointer<Utf8>) search;
  final Pointer<Utf8> Function(int, Pointer<Utf8>, Pointer<Utf8>, Pointer<Utf8>?)
      transcriptionGenerate;
  final Pointer<Utf8> Function(int, Pointer<Utf8>, Pointer<Utf8>, Pointer<Utf8>?)
      fileUpload;

  // Resource management
  final void Function(int) dropHandle;
  final void Function(Pointer<Utf8>) freeString;

  _MultimodalFFI._(
      this.openaiEmbeddingNew,
      this.cohereEmbeddingNew,
      this.googleEmbeddingNew,
      this.openaiSpeechNew,
      this.openaiImageNew,
      this.googleImageNew,
      this.openaiTranscriptionNew,
      this.cohereRerankingNew,
      this.googleVideoNew,
      this.tavilySearchNew,
      this.openaiEmbeddingNewWithBase,
      this.cohereEmbeddingNewWithBase,
      this.googleEmbeddingNewWithBase,
      this.openaiSpeechNewWithBase,
      this.openaiImageNewWithBase,
      this.googleImageNewWithBase,
      this.openaiTranscriptionNewWithBase,
      this.cohereRerankingNewWithBase,
      this.googleVideoNewWithBase,
      this.tavilySearchNewWithBase,
      this.openaiFilesNew,
      this.openaiFilesNewWithBase,
      this.embed,
      this.speechGenerate,
      this.imageGenerate,
      this.videoGenerate,
      this.rerank,
      this.search,
      this.transcriptionGenerate,
      this.fileUpload,
      this.dropHandle,
      this.freeString);

  factory _MultimodalFFI() {
    final dylib = DynamicLibrary.open(_platformLibName());

    return _MultimodalFFI._(
      // (api_key, model_id) constructors
      dylib.lookupFunction<_NewC, _NewDart>('aimux_openai_embedding_new'),
      dylib.lookupFunction<_NewC, _NewDart>('aimux_cohere_embedding_new'),
      dylib.lookupFunction<_NewC, _NewDart>('aimux_google_embedding_new'),
      dylib.lookupFunction<_NewC, _NewDart>('aimux_openai_speech_new'),
      dylib.lookupFunction<_NewC, _NewDart>('aimux_openai_image_new'),
      dylib.lookupFunction<_NewC, _NewDart>('aimux_google_image_new'),
      dylib.lookupFunction<_NewC, _NewDart>('aimux_openai_transcription_new'),
      dylib.lookupFunction<_NewC, _NewDart>('aimux_cohere_reranking_new'),
      dylib.lookupFunction<_NewC, _NewDart>('aimux_google_video_new'),
      dylib.lookupFunction<_NewC, _NewDart>('aimux_tavily_search_new'),
      // (api_key, model_id, base_url) constructors
      dylib.lookupFunction<_NewWithBaseC, _NewWithBaseDart>(
          'aimux_openai_embedding_new_with_base'),
      dylib.lookupFunction<_NewWithBaseC, _NewWithBaseDart>(
          'aimux_cohere_embedding_new_with_base'),
      dylib.lookupFunction<_NewWithBaseC, _NewWithBaseDart>(
          'aimux_google_embedding_new_with_base'),
      dylib.lookupFunction<_NewWithBaseC, _NewWithBaseDart>(
          'aimux_openai_speech_new_with_base'),
      dylib.lookupFunction<_NewWithBaseC, _NewWithBaseDart>(
          'aimux_openai_image_new_with_base'),
      dylib.lookupFunction<_NewWithBaseC, _NewWithBaseDart>(
          'aimux_google_image_new_with_base'),
      dylib.lookupFunction<_NewWithBaseC, _NewWithBaseDart>(
          'aimux_openai_transcription_new_with_base'),
      dylib.lookupFunction<_NewWithBaseC, _NewWithBaseDart>(
          'aimux_cohere_reranking_new_with_base'),
      dylib.lookupFunction<_NewWithBaseC, _NewWithBaseDart>(
          'aimux_google_video_new_with_base'),
      dylib.lookupFunction<_NewWithBaseC, _NewWithBaseDart>(
          'aimux_tavily_search_new_with_base'),
      // Files constructors
      dylib.lookupFunction<_FilesNewC, _FilesNewDart>('aimux_openai_files_new'),
      dylib.lookupFunction<_FilesNewWithBaseC, _FilesNewWithBaseDart>(
          'aimux_openai_files_new_with_base'),
      // Generate / embed / rerank / search / upload
      dylib.lookupFunction<_EmbedC, _EmbedDart>('aimux_embed'),
      dylib.lookupFunction<_GenerateOpts1C, _GenerateOpts1Dart>(
          'aimux_speech_generate'),
      dylib.lookupFunction<_GenerateOpts1C, _GenerateOpts1Dart>(
          'aimux_image_generate'),
      dylib.lookupFunction<_GenerateOpts1C, _GenerateOpts1Dart>(
          'aimux_video_generate'),
      dylib.lookupFunction<_GenerateOpts1C, _GenerateOpts1Dart>('aimux_rerank'),
      dylib.lookupFunction<_GenerateOpts1C, _GenerateOpts1Dart>('aimux_search'),
      dylib.lookupFunction<_GenerateOpts3C, _GenerateOpts3Dart>(
          'aimux_transcription_generate'),
      dylib.lookupFunction<_GenerateOpts3C, _GenerateOpts3Dart>(
          'aimux_file_upload'),
      // Resource management
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

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

// Manage a NUL-terminated C string for the duration of [fn]. Same helper as
// aimux.dart (duplicated here to keep this file self-contained).
T _withUtf8<T>(String s, T Function(Pointer<Utf8>) fn) {
  final ptr = s.toNativeUtf8();
  try {
    return fn(ptr);
  } finally {
    calloc.free(ptr);
  }
}

// Shared "call FFI → copy Pointer → toDartString → free → check error → return
// String" helper. Mirrors Go's callFFIString + extractError: a null pointer is
// an error, an {"error":"..."} envelope is thrown as StateError, everything
// else is returned as the raw JSON string. Non-JSON results are returned
// as-is (defensive — the ABI guarantees JSON results).
String _readResult(_MultimodalFFI ffi, Pointer<Utf8> ptr) {
  if (ptr == nullptr) throw StateError('aimux: FFI call returned null');
  final result = ptr.toDartString();
  ffi.freeString(ptr);
  try {
    final decoded = jsonDecode(result);
    if (decoded is Map<String, dynamic> && decoded.containsKey('error')) {
      throw StateError('aimux: ${decoded['error']}');
    }
  } on FormatException {
    // Not valid JSON — return the raw string (defensive).
  }
  return result;
}

// ─────────────────────────────────────────────────────────────────────────────
// EmbeddingModel
// ─────────────────────────────────────────────────────────────────────────────

/// An embedding model backed by a native handle.
///
/// Call [close] to release the native handle.
class EmbeddingModel {
  final int _handle;
  final _MultimodalFFI _ffi;
  bool _closed = false;

  EmbeddingModel._(this._handle, this._ffi);

  /// Create an OpenAI embedding model (e.g. text-embedding-3-small).
  ///
  /// Pass [baseUrl] to target an OpenAI-compatible endpoint. When null, the
  /// provider's standard URL is used via `aimux_openai_embedding_new`.
  factory EmbeddingModel.openai(String apiKey, String modelId,
      {String? baseUrl}) {
    final ffi = _MultimodalFFI();
    final h = _withUtf8(apiKey, (keyPtr) {
      return _withUtf8(modelId, (idPtr) {
        if (baseUrl == null) {
          return ffi.openaiEmbeddingNew(keyPtr, idPtr);
        }
        return _withUtf8(baseUrl,
            (basePtr) => ffi.openaiEmbeddingNewWithBase(keyPtr, idPtr, basePtr));
      });
    });
    if (h == 0) throw StateError('Failed to create OpenAI embedding model');
    return EmbeddingModel._(h, ffi);
  }

  /// Create a Cohere embedding model (e.g. embed-english-v3.0).
  factory EmbeddingModel.cohere(String apiKey, String modelId,
      {String? baseUrl}) {
    final ffi = _MultimodalFFI();
    final h = _withUtf8(apiKey, (keyPtr) {
      return _withUtf8(modelId, (idPtr) {
        if (baseUrl == null) {
          return ffi.cohereEmbeddingNew(keyPtr, idPtr);
        }
        return _withUtf8(baseUrl,
            (basePtr) => ffi.cohereEmbeddingNewWithBase(keyPtr, idPtr, basePtr));
      });
    });
    if (h == 0) throw StateError('Failed to create Cohere embedding model');
    return EmbeddingModel._(h, ffi);
  }

  /// Create a Google embedding model (e.g. gemini-embedding-001).
  factory EmbeddingModel.google(String apiKey, String modelId,
      {String? baseUrl}) {
    final ffi = _MultimodalFFI();
    final h = _withUtf8(apiKey, (keyPtr) {
      return _withUtf8(modelId, (idPtr) {
        if (baseUrl == null) {
          return ffi.googleEmbeddingNew(keyPtr, idPtr);
        }
        return _withUtf8(baseUrl,
            (basePtr) => ffi.googleEmbeddingNewWithBase(keyPtr, idPtr, basePtr));
      });
    });
    if (h == 0) throw StateError('Failed to create Google embedding model');
    return EmbeddingModel._(h, ffi);
  }

  /// Generate embeddings for [valuesJson] (a JSON array of strings).
  ///
  /// [optsJson] — optional serialized EmbeddingCallOptions.
  /// Returns the JSON-serialized EmbeddingResult, or throws on error.
  String embed(String valuesJson, [String? optsJson]) {
    _checkOpen();
    return _withUtf8(valuesJson, (valuesPtr) {
      final optsPtr = optsJson != null ? optsJson.toNativeUtf8() : nullptr;
      try {
        final resultPtr = _ffi.embed(_handle, valuesPtr, optsPtr);
        return _readResult(_ffi, resultPtr);
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
  final _MultimodalFFI _ffi;
  bool _closed = false;

  SpeechModel._(this._handle, this._ffi);

  /// Create an OpenAI speech (TTS) model.
  factory SpeechModel.openai(String apiKey, String modelId,
      {String? baseUrl}) {
    final ffi = _MultimodalFFI();
    final h = _withUtf8(apiKey, (keyPtr) {
      return _withUtf8(modelId, (idPtr) {
        if (baseUrl == null) {
          return ffi.openaiSpeechNew(keyPtr, idPtr);
        }
        return _withUtf8(baseUrl,
            (basePtr) => ffi.openaiSpeechNewWithBase(keyPtr, idPtr, basePtr));
      });
    });
    if (h == 0) throw StateError('Failed to create OpenAI speech model');
    return SpeechModel._(h, ffi);
  }

  /// Generate speech audio from [optsJson] (serialized SpeechCallOptions).
  ///
  /// Returns the JSON-serialized SpeechResult, or throws on error.
  String generate(String optsJson) {
    _checkOpen();
    return _withUtf8(optsJson, (optsPtr) {
      final resultPtr = _ffi.speechGenerate(_handle, optsPtr);
      return _readResult(_ffi, resultPtr);
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
  final _MultimodalFFI _ffi;
  bool _closed = false;

  TranscriptionModel._(this._handle, this._ffi);

  /// Create an OpenAI transcription (STT) model.
  factory TranscriptionModel.openai(String apiKey, String modelId,
      {String? baseUrl}) {
    final ffi = _MultimodalFFI();
    final h = _withUtf8(apiKey, (keyPtr) {
      return _withUtf8(modelId, (idPtr) {
        if (baseUrl == null) {
          return ffi.openaiTranscriptionNew(keyPtr, idPtr);
        }
        return _withUtf8(baseUrl,
            (basePtr) => ffi.openaiTranscriptionNewWithBase(keyPtr, idPtr, basePtr));
      });
    });
    if (h == 0) throw StateError('Failed to create OpenAI transcription model');
    return TranscriptionModel._(h, ffi);
  }

  /// Transcribe audio to text.
  ///
  /// [audioBase64] — base64-encoded audio data.
  /// [mediaType] — media type of the audio (e.g. "audio/wav").
  /// [optsJson] — optional serialized TranscriptionCallOptions.
  /// Returns the JSON-serialized TranscriptionResult, or throws on error.
  String generate(String audioBase64, String mediaType, [String? optsJson]) {
    _checkOpen();
    return _withUtf8(audioBase64, (audioPtr) {
      return _withUtf8(mediaType, (mediaPtr) {
        final optsPtr = optsJson != null ? optsJson.toNativeUtf8() : nullptr;
        try {
          final resultPtr =
              _ffi.transcriptionGenerate(_handle, audioPtr, mediaPtr, optsPtr);
          return _readResult(_ffi, resultPtr);
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
  final _MultimodalFFI _ffi;
  bool _closed = false;

  ImageModel._(this._handle, this._ffi);

  /// Create an OpenAI image model (e.g. dall-e-3).
  factory ImageModel.openai(String apiKey, String modelId, {String? baseUrl}) {
    final ffi = _MultimodalFFI();
    final h = _withUtf8(apiKey, (keyPtr) {
      return _withUtf8(modelId, (idPtr) {
        if (baseUrl == null) {
          return ffi.openaiImageNew(keyPtr, idPtr);
        }
        return _withUtf8(baseUrl,
            (basePtr) => ffi.openaiImageNewWithBase(keyPtr, idPtr, basePtr));
      });
    });
    if (h == 0) throw StateError('Failed to create OpenAI image model');
    return ImageModel._(h, ffi);
  }

  /// Create a Google image model (e.g. gemini-2.5-flash-image).
  factory ImageModel.google(String apiKey, String modelId, {String? baseUrl}) {
    final ffi = _MultimodalFFI();
    final h = _withUtf8(apiKey, (keyPtr) {
      return _withUtf8(modelId, (idPtr) {
        if (baseUrl == null) {
          return ffi.googleImageNew(keyPtr, idPtr);
        }
        return _withUtf8(baseUrl,
            (basePtr) => ffi.googleImageNewWithBase(keyPtr, idPtr, basePtr));
      });
    });
    if (h == 0) throw StateError('Failed to create Google image model');
    return ImageModel._(h, ffi);
  }

  /// Generate images from [optsJson] (serialized ImageCallOptions).
  ///
  /// Returns the JSON-serialized ImageResult, or throws on error.
  String generate(String optsJson) {
    _checkOpen();
    return _withUtf8(optsJson, (optsPtr) {
      final resultPtr = _ffi.imageGenerate(_handle, optsPtr);
      return _readResult(_ffi, resultPtr);
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
  final _MultimodalFFI _ffi;
  bool _closed = false;

  VideoModel._(this._handle, this._ffi);

  /// Create a Google video model (e.g. veo-3.0).
  factory VideoModel.google(String apiKey, String modelId, {String? baseUrl}) {
    final ffi = _MultimodalFFI();
    final h = _withUtf8(apiKey, (keyPtr) {
      return _withUtf8(modelId, (idPtr) {
        if (baseUrl == null) {
          return ffi.googleVideoNew(keyPtr, idPtr);
        }
        return _withUtf8(baseUrl,
            (basePtr) => ffi.googleVideoNewWithBase(keyPtr, idPtr, basePtr));
      });
    });
    if (h == 0) throw StateError('Failed to create Google video model');
    return VideoModel._(h, ffi);
  }

  /// Generate videos from [optsJson] (serialized VideoCallOptions).
  ///
  /// Returns the JSON-serialized VideoResult, or throws on error.
  String generate(String optsJson) {
    _checkOpen();
    return _withUtf8(optsJson, (optsPtr) {
      final resultPtr = _ffi.videoGenerate(_handle, optsPtr);
      return _readResult(_ffi, resultPtr);
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
  final _MultimodalFFI _ffi;
  bool _closed = false;

  RerankingModel._(this._handle, this._ffi);

  /// Create a Cohere reranking model (e.g. rerank-v3.0).
  factory RerankingModel.cohere(String apiKey, String modelId,
      {String? baseUrl}) {
    final ffi = _MultimodalFFI();
    final h = _withUtf8(apiKey, (keyPtr) {
      return _withUtf8(modelId, (idPtr) {
        if (baseUrl == null) {
          return ffi.cohereRerankingNew(keyPtr, idPtr);
        }
        return _withUtf8(baseUrl,
            (basePtr) => ffi.cohereRerankingNewWithBase(keyPtr, idPtr, basePtr));
      });
    });
    if (h == 0) throw StateError('Failed to create Cohere reranking model');
    return RerankingModel._(h, ffi);
  }

  /// Rerank documents against a query.
  ///
  /// [optsJson] — serialized RerankingCallOptions.
  /// Returns the JSON-serialized RerankingResult, or throws on error.
  String rerank(String optsJson) {
    _checkOpen();
    return _withUtf8(optsJson, (optsPtr) {
      final resultPtr = _ffi.rerank(_handle, optsPtr);
      return _readResult(_ffi, resultPtr);
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
  final _MultimodalFFI _ffi;
  bool _closed = false;

  SearchModel._(this._handle, this._ffi);

  /// Create a Tavily search model. Tavily uses a fixed endpoint, so no model
  /// ID is needed — an empty string is passed (the C ABI ignores it).
  factory SearchModel.tavily(String apiKey, {String? baseUrl}) {
    final ffi = _MultimodalFFI();
    final h = _withUtf8(apiKey, (keyPtr) {
      return _withUtf8('', (idPtr) {
        if (baseUrl == null) {
          return ffi.tavilySearchNew(keyPtr, idPtr);
        }
        return _withUtf8(baseUrl,
            (basePtr) => ffi.tavilySearchNewWithBase(keyPtr, idPtr, basePtr));
      });
    });
    if (h == 0) throw StateError('Failed to create Tavily search model');
    return SearchModel._(h, ffi);
  }

  /// Perform a web search.
  ///
  /// [optsJson] — serialized SearchCallOptions.
  /// Returns the JSON-serialized SearchResult, or throws on error.
  String search(String optsJson) {
    _checkOpen();
    return _withUtf8(optsJson, (optsPtr) {
      final resultPtr = _ffi.search(_handle, optsPtr);
      return _readResult(_ffi, resultPtr);
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
  final _MultimodalFFI _ffi;
  bool _closed = false;

  Files._(this._handle, this._ffi);

  /// Create an OpenAI files manager. Files take only an API key (no model ID).
  factory Files.openai(String apiKey, {String? baseUrl}) {
    final ffi = _MultimodalFFI();
    final h = _withUtf8(apiKey, (keyPtr) {
      if (baseUrl == null) {
        return ffi.openaiFilesNew(keyPtr);
      }
      return _withUtf8(
          baseUrl, (basePtr) => ffi.openaiFilesNewWithBase(keyPtr, basePtr));
    });
    if (h == 0) throw StateError('Failed to create OpenAI files manager');
    return Files._(h, ffi);
  }

  /// Upload a file to the provider.
  ///
  /// [dataBase64] — base64-encoded file data.
  /// [mediaType] — media type of the file (e.g. "application/pdf").
  /// [optsJson] — optional serialized UploadFileCallOptions.
  /// Returns the JSON-serialized UploadFileResult, or throws on error.
  String uploadFile(String dataBase64, String mediaType, [String? optsJson]) {
    _checkOpen();
    return _withUtf8(dataBase64, (dataPtr) {
      return _withUtf8(mediaType, (mediaPtr) {
        final optsPtr = optsJson != null ? optsJson.toNativeUtf8() : nullptr;
        try {
          final resultPtr =
              _ffi.fileUpload(_handle, dataPtr, mediaPtr, optsPtr);
          return _readResult(_ffi, resultPtr);
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
    if (_closed) throw StateError('Files has been closed');
  }
}
