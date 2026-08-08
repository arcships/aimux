// errors.dart — AimuxException hierarchy + AimuxCError (dart:ffi Struct),
// plus the FFI helpers shared by aimux.dart and multimodal.dart.
//
// Maps Rust AiMuxError → C AimuxError (aimux-error.h) → Dart Exception
// subclasses (idiomatic Dart, OpenAI/Anthropic SDK style).
//
// Transport: fallible C ABI calls take a trailing AimuxError *err. Check the
// return value first (0 / NULL / stream 0 = failure); only then read *err via
// [AimuxException.fromC]. On failure the callee allocates `err->message` (a
// NUL-terminated UTF-8 string) which the caller owns and must free via
// `aimux_free_string` — [withAimuxCError] does this automatically. On success
// `*err` is untouched.

import 'dart:ffi';
import 'dart:io';

import 'package:ffi/ffi.dart';

// ─────────────────────────────────────────────────────────────────────────────
// Error code constants (match AimuxErrorCode in aimux-error.h — append-only)
// ─────────────────────────────────────────────────────────────────────────────

/// Machine-readable codes. Values match C `AimuxErrorCode` / Go `Code`.
abstract final class AimuxErrorCode {
  static const int ok = 0;
  static const int unknown = 1;
  static const int provider = 2;
  static const int http = 3;
  static const int json = 4;
  static const int stream = 5;
  static const int tool = 6;
  static const int invalidArgument = 7;
  static const int invalidPrompt = 8;
  static const int rateLimited = 9;
  static const int auth = 10;
  static const int tokenExpired = 11;
  static const int modelNotFound = 12;
  static const int unsupported = 13;
  static const int noSuchModel = 14;
  static const int unknownProvider = 15;
  static const int apiCall = 16;
  static const int timeout = 17;
  static const int aborted = 18;
  static const int other = 19;

  static const Map<int, String> _names = {
    ok: 'OK',
    unknown: 'Unknown',
    provider: 'Provider',
    http: 'Http',
    json: 'Json',
    stream: 'Stream',
    tool: 'Tool',
    invalidArgument: 'InvalidArgument',
    invalidPrompt: 'InvalidPrompt',
    rateLimited: 'RateLimited',
    auth: 'Auth',
    tokenExpired: 'TokenExpired',
    modelNotFound: 'ModelNotFound',
    unsupported: 'Unsupported',
    noSuchModel: 'NoSuchModel',
    unknownProvider: 'UnknownProvider',
    apiCall: 'ApiCall',
    timeout: 'Timeout',
    aborted: 'Aborted',
    other: 'Other',
  };

  /// Core `error_type()` name (e.g. `"Auth"`, `"RateLimited"`).
  static String name(int code) => _names[code] ?? 'Code($code)';
}

// ─────────────────────────────────────────────────────────────────────────────
// C ABI struct (aimux-error.h AimuxError)
// ─────────────────────────────────────────────────────────────────────────────

/// C `AimuxError` layout (40 bytes on 64-bit targets).
///
/// Layout must match `aimux-error.h` / Rust `CAimuxError` (`#[repr(C)]`):
/// `code:i32`, `status:i32`, `retry_ms:i64`, `message:char*`,
/// `error_value:char*`, plus one reserved pointer slot for future ABI
/// extension (always zero).
///
/// On failure the callee allocates [message] and optionally [errorValue]
/// (lossless externally-tagged core `AiMuxError` JSON; NULL for
/// FFI-synthesized failures); the caller owns both and must free each via
/// `aimux_free_string` after reading ([withAimuxCError] handles this). On
/// success the struct is untouched.
final class AimuxCError extends Struct {
  @Int32()
  external int code;

  @Int32()
  external int status;

  @Int64()
  external int retryMs;

  external Pointer<Utf8> message;

  external Pointer<Utf8> errorValue;

  external Pointer<Void> reserved0;
}

/// Reset [err] to OK / no hint / no message (mirrors `aimux_error_clear`).
void clearAimuxCError(Pointer<AimuxCError> err) {
  err.ref.code = AimuxErrorCode.ok;
  err.ref.status = -1;
  err.ref.retryMs = -1;
  err.ref.message = nullptr;
  err.ref.errorValue = nullptr;
  err.ref.reserved0 = nullptr;
}

/// Allocate a cleared [AimuxCError], run [fn], free the engine-allocated
/// message (if any) and the struct itself.
///
/// Typical use:
/// ```dart
/// final handle = withAimuxCError((err) {
///   final h = ffi.openaiNew(key, id, err);
///   if (h == 0) throw AimuxException.fromC(err.ref);
///   return h;
/// });
/// ```
T withAimuxCError<T>(T Function(Pointer<AimuxCError> err) fn) {
  final err = calloc<AimuxCError>();
  try {
    clearAimuxCError(err);
    return fn(err);
  } finally {
    final msg = err.ref.message;
    if (msg != nullptr) aimuxFreeString(msg);
    final ev = err.ref.errorValue;
    if (ev != nullptr) aimuxFreeString(ev);
    calloc.free(err);
  }
}

// ─────────────────────────────────────────────────────────────────────────────
// Shared FFI plumbing (internal — hidden from the package:aimux/aimux.dart
// export; aimux.dart and multimodal.dart both build on these).
// ─────────────────────────────────────────────────────────────────────────────

/// Opens the aimux-ffi native library for the current platform.
///
/// The library ships inside this Flutter plugin package:
///   - Android: `libaimux_ffi.so` per ABI under android/src/main/jniLibs,
///     loaded by name (the Android build system bundles it into the APK).
///   - iOS:     `libaimux_ffi.a` is statically linked into the app binary
///     via ios/aimux.podspec (vendored + force_load), so symbols resolve
///     from the process itself.
///   - Desktop: looked up on the platform library path (dev/test usage).
DynamicLibrary openAimuxLibrary() {
  if (Platform.isAndroid) return DynamicLibrary.open('libaimux_ffi.so');
  if (Platform.isIOS) return DynamicLibrary.process();
  if (Platform.isLinux) return DynamicLibrary.open('libaimux_ffi.so');
  if (Platform.isMacOS) return DynamicLibrary.open('libaimux_ffi.dylib');
  if (Platform.isWindows) return DynamicLibrary.open('aimux_ffi.dll');
  throw StateError('Unsupported platform');
}

/// `aimux_free_string` — frees engine-allocated strings, including
/// `err->message`. Lazily initialized so pure-Dart tests that never produce a
/// native message do not dlopen the library.
final void Function(Pointer<Utf8>) aimuxFreeString = openAimuxLibrary()
    .lookupFunction<Void Function(Pointer<Utf8>), void Function(Pointer<Utf8>)>(
        'aimux_free_string');

/// Run [fn] with a temporary native UTF-8 copy of [s]; always frees it.
T withUtf8<T>(String s, T Function(Pointer<Utf8>) fn) {
  final ptr = s.toNativeUtf8();
  try {
    return fn(ptr);
  } finally {
    calloc.free(ptr);
  }
}

/// Take a constructor `uint64_t` handle; throw [AimuxException] when 0.
int takeHandle(int handle, Pointer<AimuxCError> err) {
  if (handle == 0) throw AimuxException.fromC(err.ref);
  return handle;
}

/// Take an owned C string result; free it; throw [AimuxException] on null.
String takeString(Pointer<Utf8> ptr, Pointer<AimuxCError> err) {
  if (ptr == nullptr) throw AimuxException.fromC(err.ref);
  try {
    return ptr.toDartString();
  } finally {
    aimuxFreeString(ptr);
  }
}

/// Shared constructor for `(api_key, model_id[, base_url])` providers.
int construct2(
  String apiKey,
  String modelId,
  String? baseUrl,
  int Function(Pointer<Utf8>, Pointer<Utf8>, Pointer<AimuxCError>) plain,
  int Function(
          Pointer<Utf8>, Pointer<Utf8>, Pointer<Utf8>, Pointer<AimuxCError>)
      withBase,
) {
  return withAimuxCError((err) {
    return withUtf8(apiKey, (keyPtr) {
      return withUtf8(modelId, (idPtr) {
        if (baseUrl == null) {
          return takeHandle(plain(keyPtr, idPtr, err), err);
        }
        return withUtf8(baseUrl, (basePtr) {
          return takeHandle(withBase(keyPtr, idPtr, basePtr, err), err);
        });
      });
    });
  });
}

// ─────────────────────────────────────────────────────────────────────────────
// Exception hierarchy
// ─────────────────────────────────────────────────────────────────────────────

/// Base class for all aimux engine / binding failures.
///
/// Catch this for any structured failure; use subclasses for specific handling
/// (`on RateLimitedError`, `on AuthenticationError`, …).
///
/// Fields mirror C `AimuxError` / core helpers:
/// - [code]: kind ([AimuxErrorCode])
/// - [status]: HTTP status, or `-1`
/// - [retryMs]: rate-limit hint, or `-1` (`0` = retry now)
/// - [message]: human-readable text
/// - [errorValue]: raw lossless core-error JSON, or `null`
class AimuxException implements Exception {
  /// Human-readable failure text.
  final String message;

  /// Machine-readable kind ([AimuxErrorCode] constants).
  final int code;

  /// HTTP status when known; otherwise `-1`.
  final int status;

  /// Rate-limit hint in ms; `-1` if none; `0` means retry immediately.
  final int retryMs;

  /// Raw lossless core-error JSON (externally-tagged `AiMuxError`), or `null`
  /// when absent (e.g. FFI-synthesized failures). No parsing is done.
  final String? errorValue;

  AimuxException(
    this.message, {
    this.code = AimuxErrorCode.other,
    this.status = -1,
    this.retryMs = -1,
    this.errorValue,
  });

  /// Build the typed subclass from a filled C [AimuxCError].
  ///
  /// Call only after a fallible FFI return indicated failure. If [e].code is
  /// [AimuxErrorCode.ok] (or the message is empty), produces a generic
  /// failure with [AimuxErrorCode.unknown]. Does not free `e.message` or
  /// `e.errorValue` — the enclosing [withAimuxCError] does.
  factory AimuxException.fromC(AimuxCError e) {
    var code = e.code;
    var message = e.message == nullptr ? '' : e.message.toDartString();

    if (code == AimuxErrorCode.ok) {
      code = AimuxErrorCode.unknown;
      if (message.isEmpty) message = 'aimux: operation failed';
    } else if (message.isEmpty) {
      message = 'aimux: ${AimuxErrorCode.name(code)}';
    }

    return AimuxException.fromCode(
      code,
      message,
      status: e.status,
      retryMs: e.retryMs,
      errorValue:
          e.errorValue == nullptr ? null : e.errorValue.toDartString(),
    );
  }

  /// Dispatch [code] → concrete subclass (defaults for status when `-1`).
  factory AimuxException.fromCode(
    int code,
    String message, {
    int status = -1,
    int retryMs = -1,
    String? errorValue,
  }) {
    switch (code) {
      case AimuxErrorCode.provider:
        return ProviderError(message, status: status, retryMs: retryMs, errorValue: errorValue);
      case AimuxErrorCode.http:
        return HttpError(message, status: status, retryMs: retryMs, errorValue: errorValue);
      case AimuxErrorCode.json:
        return JsonError(message, status: status, retryMs: retryMs, errorValue: errorValue);
      case AimuxErrorCode.stream:
        return StreamError(message, status: status, retryMs: retryMs, errorValue: errorValue);
      case AimuxErrorCode.tool:
        return ToolError(message, status: status, retryMs: retryMs, errorValue: errorValue);
      case AimuxErrorCode.invalidArgument:
        return InvalidArgumentError(message, status: status, retryMs: retryMs, errorValue: errorValue);
      case AimuxErrorCode.invalidPrompt:
        return InvalidPromptError(message, status: status, retryMs: retryMs, errorValue: errorValue);
      case AimuxErrorCode.rateLimited:
        return RateLimitedError(
          message,
          status: status == -1 ? 429 : status,
          retryMs: retryMs,
          errorValue: errorValue,
        );
      case AimuxErrorCode.auth:
        return AuthenticationError(
          message,
          status: status == -1 ? 401 : status,
          retryMs: retryMs,
          errorValue: errorValue,
        );
      case AimuxErrorCode.tokenExpired:
        return TokenExpiredError(
          message,
          status: status == -1 ? 401 : status,
          retryMs: retryMs,
          errorValue: errorValue,
        );
      case AimuxErrorCode.modelNotFound:
        return ModelNotFoundError(
          message,
          status: status == -1 ? 404 : status,
          retryMs: retryMs,
          errorValue: errorValue,
        );
      case AimuxErrorCode.unsupported:
        return AimuxUnsupportedError(message, status: status, retryMs: retryMs, errorValue: errorValue);
      case AimuxErrorCode.noSuchModel:
        return NoSuchModelError(message, status: status, retryMs: retryMs, errorValue: errorValue);
      case AimuxErrorCode.unknownProvider:
        return UnknownProviderError(message, status: status, retryMs: retryMs, errorValue: errorValue);
      case AimuxErrorCode.apiCall:
        return APICallError(message, status: status, retryMs: retryMs, errorValue: errorValue);
      case AimuxErrorCode.timeout:
        return AimuxTimeoutError(message, status: status, retryMs: retryMs, errorValue: errorValue);
      case AimuxErrorCode.aborted:
        return RequestAbortedError(message, status: status, retryMs: retryMs, errorValue: errorValue);
      case AimuxErrorCode.other:
        return OtherError(message, status: status, retryMs: retryMs, errorValue: errorValue);
      case AimuxErrorCode.unknown:
        return UnknownError(message, status: status, retryMs: retryMs, errorValue: errorValue);
      default:
        return AimuxException(
          message,
          code: code,
          status: status,
          retryMs: retryMs,
          errorValue: errorValue,
        );
    }
  }

  /// Core `error_type()` name for [code].
  String get codeName => AimuxErrorCode.name(code);

  @override
  String toString() => '$runtimeType: $message';
}

// ── Concrete subclasses (one per AimuxErrorCode 1..19) ──────────────────────

/// Unclassified / unknown failure (C `AIMUX_E_UNKNOWN`).
class UnknownError extends AimuxException {
  UnknownError(super.message, {super.status, super.retryMs, super.errorValue})
      : super(code: AimuxErrorCode.unknown);
}

/// Provider-layer failure.
class ProviderError extends AimuxException {
  ProviderError(super.message, {super.status, super.retryMs, super.errorValue})
      : super(code: AimuxErrorCode.provider);
}

/// HTTP transport failure.
class HttpError extends AimuxException {
  HttpError(super.message, {super.status, super.retryMs, super.errorValue})
      : super(code: AimuxErrorCode.http);
}

/// JSON parse / serialize failure.
class JsonError extends AimuxException {
  JsonError(super.message, {super.status, super.retryMs, super.errorValue})
      : super(code: AimuxErrorCode.json);
}

/// Streaming failure.
class StreamError extends AimuxException {
  StreamError(super.message, {super.status, super.retryMs, super.errorValue})
      : super(code: AimuxErrorCode.stream);
}

/// Tool-related failure.
class ToolError extends AimuxException {
  ToolError(super.message, {super.status, super.retryMs, super.errorValue})
      : super(code: AimuxErrorCode.tool);
}

/// Invalid argument (null args, invalid or expired handles, …).
class InvalidArgumentError extends AimuxException {
  InvalidArgumentError(super.message, {super.status, super.retryMs, super.errorValue})
      : super(code: AimuxErrorCode.invalidArgument);
}

/// Invalid prompt.
class InvalidPromptError extends AimuxException {
  InvalidPromptError(super.message, {super.status, super.retryMs, super.errorValue})
      : super(code: AimuxErrorCode.invalidPrompt);
}

/// Rate limited (HTTP 429). See [AimuxException.retryMs].
class RateLimitedError extends AimuxException {
  RateLimitedError(super.message, {super.status = 429, super.retryMs, super.errorValue})
      : super(code: AimuxErrorCode.rateLimited);
}

/// Auth / bad API key (HTTP 401). Named after OpenAI/Anthropic
/// `AuthenticationError`.
class AuthenticationError extends AimuxException {
  AuthenticationError(super.message, {super.status = 401, super.retryMs, super.errorValue})
      : super(code: AimuxErrorCode.auth);
}

/// Access token expired.
class TokenExpiredError extends AimuxException {
  TokenExpiredError(super.message, {super.status = 401, super.retryMs, super.errorValue})
      : super(code: AimuxErrorCode.tokenExpired);
}

/// Model not found (HTTP 404).
class ModelNotFoundError extends AimuxException {
  ModelNotFoundError(super.message, {super.status = 404, super.retryMs, super.errorValue})
      : super(code: AimuxErrorCode.modelNotFound);
}

/// Unsupported functionality. (Prefixed to avoid shadowing dart:core
/// `UnsupportedError`.)
class AimuxUnsupportedError extends AimuxException {
  AimuxUnsupportedError(super.message, {super.status, super.retryMs, super.errorValue})
      : super(code: AimuxErrorCode.unsupported);
}

/// No such model in registry / catalogue.
class NoSuchModelError extends AimuxException {
  NoSuchModelError(super.message, {super.status, super.retryMs, super.errorValue})
      : super(code: AimuxErrorCode.noSuchModel);
}

/// Unknown provider name.
class UnknownProviderError extends AimuxException {
  UnknownProviderError(super.message, {super.status, super.retryMs, super.errorValue})
      : super(code: AimuxErrorCode.unknownProvider);
}

/// Provider API call failed (AI SDK `APICallError` analogue).
class APICallError extends AimuxException {
  APICallError(super.message, {super.status, super.retryMs, super.errorValue})
      : super(code: AimuxErrorCode.apiCall);
}

/// Request timed out. (Prefixed to avoid shadowing dart:async
/// `TimeoutError`.)
class AimuxTimeoutError extends AimuxException {
  AimuxTimeoutError(super.message, {super.status, super.retryMs, super.errorValue})
      : super(code: AimuxErrorCode.timeout);
}

/// Request aborted (not DOM `AbortError`).
class RequestAbortedError extends AimuxException {
  RequestAbortedError(super.message, {super.status, super.retryMs, super.errorValue})
      : super(code: AimuxErrorCode.aborted);
}

/// Unclassified failure (`AIMUX_E_OTHER`).
class OtherError extends AimuxException {
  OtherError(super.message, {super.status, super.retryMs, super.errorValue})
      : super(code: AimuxErrorCode.other);
}
