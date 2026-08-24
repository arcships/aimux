// errors.dart — the two aimux error types + the C-error decoder shared by
// aimux.dart and multimodal.dart.
//
// Transport (aimux-error.h): every fallible C call returns
// `aimux_error_t *` — NULL on success (the result is in the trailing
// out-param), non-NULL on failure (out-param at its sentinel: 0 / NULL). The
// unified code selects AiMuxError (1..13, 15..17), RecordingError (100..105), or a
// C ABI failure (200..206). The last range maps to
// StateError('aimux ffi: …'); Dart does not expose seven additional classes.
// Every field is copied before the error is released with `aimux_error_free`
// exactly once. Errors are not
// handles — never `aimux_drop_handle` them.

import 'dart:convert';
import 'dart:ffi';
import 'dart:io';

import 'package:ffi/ffi.dart';

// ─────────────────────────────────────────────────────────────────────────────
// Error code constants (match aimux_error_code_t in aimux-error.h)
// ─────────────────────────────────────────────────────────────────────────────

/// Machine-readable codes. Values match C `aimux_error_code_t` / Go `Code`.
/// 15 variant codes: 1–13 plus 15–17 (1 is the catch-all; 4 is retired, 14 is
/// reserved). A code outside
/// that set is a header/library mismatch and fails with [StateError], not an
/// error type. Every HTTP-shaped failure
/// arrives as [apiCall], classified
/// by [AimuxException.status] (401 auth, 404 model, 429 rate limit;
/// -1 = no HTTP response was ever observed — a missing API key, an error
/// built without a request, or a transport failure).
abstract final class AimuxErrorCode {
  static const int ok = 0;
  static const int jsonParse = 2;
  static const int invalidResponseData = 3;
  static const int invalidArgument = 5;
  static const int invalidPrompt = 6;
  static const int tokenExpired = 7;
  static const int unsupportedFunctionality = 8;
  static const int noSuchModel = 9;
  static const int noSuchProvider = 10;
  static const int apiCall = 11;
  static const int timeout = 12;
  static const int aborted = 13;
  static const int noSuchTool = 15;
  static const int invalidToolInput = 16;
  static const int toolCallRepair = 17;
  static const int other = 1;

  static const Map<int, String> _text = {
    ok: 'OK',
    jsonParse: 'JsonParse',
    invalidResponseData: 'InvalidResponseData',
    invalidArgument: 'InvalidArgument',
    invalidPrompt: 'InvalidPrompt',
    tokenExpired: 'TokenExpired',
    unsupportedFunctionality: 'UnsupportedFunctionality',
    noSuchModel: 'NoSuchModel',
    noSuchProvider: 'NoSuchProvider',
    apiCall: 'ApiCall',
    timeout: 'Timeout',
    aborted: 'Aborted',
    noSuchTool: 'NoSuchTool',
    invalidToolInput: 'InvalidToolInput',
    toolCallRepair: 'ToolCallRepair',
    other: 'Other',
  };

  /// Core `error_type()` name.
  static String name(int code) => _text[code] ?? 'Code($code)';
}

// ─────────────────────────────────────────────────────────────────────────────
// The decoder (aimux-error.h)
// ─────────────────────────────────────────────────────────────────────────────

/// Decode the `aimux_error_t *` [e] returned by a call that can fail in
/// `AiMuxError` (`[AiMuxError]` in aimux-ffi.h). NULL → returns (success).
/// Codes 1..13 / 15..17 become [AimuxException]; 200..206 become [StateError].
/// The returned error is always freed.
void expectAimuxError(Pointer<Void> e, String context) {
  if (e == nullptr) return;
  final Object failure;
  try {
    final code = _errorCode(e);
    failure = _isFfiCode(code)
        ? _ffiError(e, context)
        : AimuxException._decode(e, context);
  } finally {
    _errorFree(e);
  }
  throw failure;
}

/// Same as [expectAimuxError] for calls that can fail in the recorder
/// (`[RecordingError]`): 100..105 → [RecordingException]; 200..206 →
/// invariant [StateError].
void expectRecordingError(Pointer<Void> e, String context) {
  if (e == nullptr) return;
  final Object failure;
  try {
    final code = _errorCode(e);
    failure = _isFfiCode(code)
        ? _ffiError(e, context)
        : RecordingException._decode(e);
  } finally {
    _errorFree(e);
  }
  throw failure;
}

/// Same for calls that only expose C ABI failures (`[C ABI]`): message
/// only → invariant [StateError].
void expectFfiError(Pointer<Void> e, String context) {
  if (e == nullptr) return;
  final Object failure;
  try {
    final code = _errorCode(e);
    failure = _isFfiCode(code)
        ? _ffiError(e, context)
        : StateError('aimux ffi: $context: expected C ABI failure code, got $code');
  } finally {
    _errorFree(e);
  }
  throw failure;
}

StateError _ffiError(Pointer<Void> e, String context) =>
    StateError('aimux ffi: $context: ${_errStr(_errorMessage, e) ?? ''}');

bool _isFfiCode(int code) => code >= 200 && code <= 206;

/// Validate a caller-supplied raw JSON string *before* it crosses the C
/// C ABI; throws [FormatException] naming [param] (`source`) on bad
/// syntax. Null passes through (optional parameters); empty/blank is rejected
/// like the FFI does for REQUIRED params, unless [emptyIsDefault] — pass it for
/// every OPTIONAL JSON param (nullable in the Dart signature: `config_json` of
/// provider/router/moa, `opts_json` of embed / file upload / transcription),
/// where the FFI treats NULL/empty as "use defaults".
void checkJson(String? json, String param, {bool emptyIsDefault = false}) {
  if (json == null) return;
  if (json.trim().isEmpty) {
    if (emptyIsDefault) return;
    throw FormatException('$param: invalid JSON: empty', param);
  }
  final Object? value;
  try {
    value = jsonDecode(json);
  } on FormatException catch (e) {
    throw FormatException('$param: ${e.message}', param);
  }
  checkPortable(value, param);
}

/// [jsonEncode] for a caller-supplied value the binding serializes itself
/// (prompts, call options, [ProviderConfig]) — pre-validated with the same
/// rule [checkJson] applies to caller-supplied JSON *text*, because nothing
/// downstream would check it.
String encodeJson(Object? value, String param) {
  checkPortable(value, param);
  return jsonEncode(value);
}

/// Reject a JSON value that `jsonDecode`/`jsonEncode` accept but `serde_json`
/// on the Rust side does not. Divergences (all three verified against
/// serde_json 1.0.151):
///
///  - **Unpaired surrogate.** A Dart `String` is UTF-16 and may hold one;
///    `jsonDecode` accepts it both raw and as a `\uD800` escape, and
///    `jsonEncode` writes it back out as a `\uD800` escape. `serde_json`
///    rejects that escape (`LoneLeadingSurrogateInHexEscape` /
///    `UnexpectedEndOfHexEscape`).
///  - **Non-finite number.** Dart parses `1e999` to `Infinity`; `serde_json`
///    rejects it (`NumberOutOfRange`).
///  - **Nesting past [_maxJsonDepth].**
///
/// All three classify as `Category::Syntax`, which aimux-ffi reports as
/// `FfiError::InvalidWireJson` (code 202), so it reaches Dart through
/// [_ffiError] as an invariant [StateError] — but none of it is
/// an invariant: `text.substring(0, n)` cutting an emoji in half is enough to
/// produce the first. Catch them here, where the parameter still has a name
/// and the failure is a catchable [FormatException].
void checkPortable(Object? value, String param, [int depth = 0]) {
  if (depth >= _maxJsonDepth) {
    throw FormatException(
        '$param: JSON nested deeper than $_maxJsonDepth levels', param);
  }
  if (value is String) {
    _checkSurrogates(value, param);
  } else if (value is double && !value.isFinite) {
    throw FormatException('$param: number out of range: $value', param);
  } else if (value is List) {
    for (final element in value) {
      checkPortable(element, param, depth + 1);
    }
  } else if (value is Map) {
    value.forEach((key, element) {
      checkPortable(key, param, depth + 1);
      checkPortable(element, param, depth + 1);
    });
  }
}

/// Matches `serde_json`'s default recursion limit, and keeps [checkPortable]'s
/// own recursion bounded — `jsonDecode` is iterative and happily returns a
/// 5000-deep document that would overflow the stack here.
// ponytail: hard-coded to serde_json's default. If that ever moves, documents
// between the two limits fall back to today's StateError — nothing valid is
// rejected.
const int _maxJsonDepth = 128;

/// Reject an unpaired UTF-16 surrogate in [s] (not representable in UTF-8).
void _checkSurrogates(String s, String param) {
  for (var i = 0; i < s.length; i++) {
    final unit = s.codeUnitAt(i);
    if (unit < 0xD800 || unit > 0xDFFF) continue;
    // A high surrogate must be followed by a low one; a bare low one is
    // already wrong.
    final low =
        (unit < 0xDC00 && i + 1 < s.length) ? s.codeUnitAt(i + 1) : 0x0000;
    if (low < 0xDC00 || low > 0xDFFF) {
      throw FormatException(
          '$param: unpaired surrogate U+${unit.toRadixString(16).toUpperCase()}'
          ' at index $i is not representable in UTF-8',
          param);
    }
    i++; // consume the low surrogate of the valid pair
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

/// `aimux_free_string` — frees aimux-allocated result strings. Lazily
/// initialized so pure-Dart tests that never produce a native string do not
/// dlopen the library.
final void Function(Pointer<Utf8>) aimuxFreeString = openAimuxLibrary()
    .lookupFunction<Void Function(Pointer<Utf8>), void Function(Pointer<Utf8>)>(
        'aimux_free_string');

/// Lazily opened library shared by the error symbols below.
final DynamicLibrary _errLib = openAimuxLibrary();

/// `aimux_drop_handle` — releases a model / provider handle. Lazy, as above.
/// (Never for errors: those go through `aimux_error_free`.)
final void Function(int) aimuxDropHandle = _errLib
    .lookupFunction<Void Function(Uint64), void Function(int)>(
        'aimux_drop_handle');

// Returned errors (aimux-error.h) are opaque `Pointer<Void>` values.
typedef _I32OfPtrC = Int32 Function(Pointer<Void>);
typedef _I64OfPtrC = Int64 Function(Pointer<Void>);
typedef _StrOfPtrC = Pointer<Utf8> Function(Pointer<Void>);
typedef _IntOfPtr = int Function(Pointer<Void>);
typedef _StrOfPtr = Pointer<Utf8> Function(Pointer<Void>);

_IntOfPtr _i32Getter(String sym) =>
    _errLib.lookupFunction<_I32OfPtrC, _IntOfPtr>(sym);
_StrOfPtr _strGetter(String sym) =>
    _errLib.lookupFunction<_StrOfPtrC, _StrOfPtr>(sym);

final void Function(Pointer<Void>) _errorFree = _errLib.lookupFunction<
    Void Function(Pointer<Void>),
    void Function(Pointer<Void>)>('aimux_error_free');
// `aimux_error_*` getters. Every char* is owned → freed
// via aimuxFreeString by [_errStr].
final _IntOfPtr _errorCode = _i32Getter('aimux_error_code');
final _IntOfPtr _errorRetryable = _i32Getter('aimux_error_retryable');
final _IntOfPtr _errorStatus = _i32Getter('aimux_error_status');
final _IntOfPtr _errorRetryMs =
    _errLib.lookupFunction<_I64OfPtrC, _IntOfPtr>('aimux_error_retry_ms');
final _StrOfPtr _errorMessage = _strGetter('aimux_error_message');
final _StrOfPtr _errorProviderCode = _strGetter('aimux_error_provider_code');
final _StrOfPtr _errorProviderMessage =
    _strGetter('aimux_error_provider_message');
final _StrOfPtr _errorRequestId = _strGetter('aimux_error_request_id');
final _StrOfPtr _errorResponseBody = _strGetter('aimux_error_response_body');
final _StrOfPtr _errorModelId = _strGetter('aimux_error_model_id');
final _StrOfPtr _errorModelType = _strGetter('aimux_error_model_type');
final _StrOfPtr _errorProviderId = _strGetter('aimux_error_provider_id');
final _StrOfPtr _errorToolName = _strGetter('aimux_error_tool_name');
final _StrOfPtr _errorAvailableTools = _strGetter('aimux_error_available_tools');
final _StrOfPtr _errorToolInput = _strGetter('aimux_error_tool_input');
final _StrOfPtr _errorOriginalError = _strGetter('aimux_error_original_error');

/// Read an owned getter string for [error]; frees it; null when absent.
String? _errStr(_StrOfPtr getter, Pointer<Void> error) {
  final p = getter(error);
  if (p == nullptr) return null;
  try {
    return p.toDartString();
  } finally {
    aimuxFreeString(p);
  }
}

/// Run [fn] with a temporary native UTF-8 copy of [s]; always frees it.
T withUtf8<T>(String s, T Function(Pointer<Utf8>) fn) {
  final ptr = toCString(s);
  try {
    return fn(ptr);
  } finally {
    calloc.free(ptr);
  }
}

/// [toCString] for an optional argument: `nullptr` for null (the FFI reads
/// NULL as "absent"). Callers still free a non-NULL result.
Pointer<Utf8> toCStringOrNull(String? s) => s == null ? nullptr : toCString(s);

/// `toNativeUtf8` with the two checks a C string needs and a Dart `String`
/// does not enforce, applied to every string parameter that crosses the ABI
/// (model ids, api keys, base URLs, dir paths, base64 payloads, media types,
/// JSON text):
///
///  - **Interior NUL.** `toNativeUtf8` writes it verbatim, and the C side's
///    `CStr::from_ptr` stops there — the argument is silently truncated at
///    the NUL rather than rejected.
///  - **Unpaired surrogate.** `toNativeUtf8` substitutes U+FFFD (documented in
///    `package:ffi`'s `StringUtf8Pointer.toNativeUtf8`), so the argument is
///    silently corrupted rather than rejected.
///
/// Both are catchable [FormatException]s here instead of silence there.
Pointer<Utf8> toCString(String s) {
  final nul = s.codeUnits.indexOf(0);
  if (nul >= 0) {
    throw FormatException(
        'argument contains a NUL at index $nul; a C string ends there', s, nul);
  }
  _checkSurrogates(s, 'argument');
  return s.toNativeUtf8();
}

/// Run [fn] with a zeroed `uint64_t *out_handle`, [expectAimuxError] the returned
/// error, and hand back the handle.
int takeHandle(
    Pointer<Void> Function(Pointer<Uint64> out) fn, String context) {
  final out = calloc<Uint64>();
  try {
    expectAimuxError(fn(out), context);
    return out.value;
  } finally {
    calloc.free(out);
  }
}

/// Run [fn] with a NULL `char **out_json`, [expectAimuxError] the returned error,
/// copy the owned C string out and free it.
String takeString(
    Pointer<Void> Function(Pointer<Pointer<Utf8>> out) fn, String context) {
  final out = calloc<Pointer<Utf8>>();
  try {
    expectAimuxError(fn(out), context);
    final p = out.value;
    if (p == nullptr) throw StateError('aimux ffi: $context: NULL result');
    try {
      return p.toDartString();
    } finally {
      aimuxFreeString(p);
    }
  } finally {
    calloc.free(out);
  }
}

/// Shared constructor for `(api_key, model_id[, base_url], out_handle)`
/// providers.
int construct2(
  String apiKey,
  String modelId,
  String? baseUrl,
  Pointer<Void> Function(Pointer<Utf8>, Pointer<Utf8>, Pointer<Uint64>) plain,
  Pointer<Void> Function(
          Pointer<Utf8>, Pointer<Utf8>, Pointer<Utf8>, Pointer<Uint64>)
      withBase,
) {
  return withUtf8(apiKey, (keyPtr) {
    return withUtf8(modelId, (idPtr) {
      if (baseUrl == null) {
        return takeHandle((out) => plain(keyPtr, idPtr, out), 'new');
      }
      return withUtf8(baseUrl, (basePtr) {
        return takeHandle(
            (out) => withBase(keyPtr, idPtr, basePtr, out), 'new_with_base');
      });
    });
  });
}

// ─────────────────────────────────────────────────────────────────────────────
// Exception hierarchy
// ─────────────────────────────────────────────────────────────────────────────

/// Base class for aimux AiMuxErrors only.
///
/// Use subclasses for specific handling (`on APICallError`,
/// `on AimuxTimeoutError`, …). Recording failures use the independent
/// [RecordingException]; C ABI failures surface as native Dart
/// errors (see [expectAimuxError]).
/// HTTP-shaped failures are all [APICallError] — branch on [status]
/// (401 auth, 404 model, 429 rate limit).
///
/// Fields mirror the C `aimux_error_*` getters / core helpers:
/// - [code]: error code ([AimuxErrorCode])
/// - [status]: HTTP status, or `-1`
/// - [retryMs]: rate-limit hint, or `-1` (`0` = retry now)
/// - [retryable]: whether retrying may help
/// - [message]: human-readable text
///
/// Per-code payload lives on the carrying subclass only: [APICallError]
/// (`providerCode`/`providerMessage`/`requestId`/`responseBody`), [NoSuchModelError]
/// (`modelId`/`modelType`), [NoSuchProviderError] (`providerId`),
/// [NoSuchToolError] (`toolName`/`availableTools`), [InvalidToolInputError]
/// (`toolName`/`toolInput`), [ToolCallRepairError] (`originalError`).
class AimuxException implements Exception {
  /// Human-readable failure text.
  final String message;

  /// Machine-readable error code ([AimuxErrorCode] constants).
  final int code;

  /// HTTP status when known; otherwise `-1`.
  final int status;

  /// Rate-limit hint in ms; `-1` if none; `0` means retry immediately.
  final int retryMs;

  /// Whether retrying may help — the `AiMuxError` verdict, carried across the C
  /// ABI. Not derivable from [status]: a transport failure (request went out,
  /// connection reset) and a missing API key (request never went out) both
  /// report `status == -1` and disagree here.
  final bool retryable;

  AimuxException(
    this.message, {
    this.code = AimuxErrorCode.other,
    this.status = -1,
    this.retryMs = -1,
    this.retryable = false,
  });

  /// Build the typed subclass from a returned `const aimux_error_t *` [error]
  /// via the `aimux_error_*` getters (payload getters only under the owning
  /// code; getter strings freed here). The caller ([expectAimuxError])
  /// frees it. A code outside 1..13 / 15..17 is a
  /// contract violation and throws [StateError].
  factory AimuxException._decode(Pointer<Void> error, String context) {
    final code = _errorCode(error);
    var message = _errStr(_errorMessage, error) ?? '';
    if (message.isEmpty) message = 'aimux: ${AimuxErrorCode.name(code)}';

    final retryable = _errorRetryable(error) != 0;
    switch (code) {
      case AimuxErrorCode.apiCall:
        return APICallError(message,
            status: _errorStatus(error),
            retryMs: _errorRetryMs(error),
            retryable: retryable,
            providerCode: _errStr(_errorProviderCode, error),
            providerMessage: _errStr(_errorProviderMessage, error),
            requestId: _errStr(_errorRequestId, error),
            responseBody: _errStr(_errorResponseBody, error));
      case AimuxErrorCode.noSuchModel:
        return NoSuchModelError(message,
            retryable: retryable,
            modelId: _errStr(_errorModelId, error) ?? '',
            modelType: _errStr(_errorModelType, error) ?? '');
      case AimuxErrorCode.noSuchProvider:
        return NoSuchProviderError(message,
            retryable: retryable,
            providerId: _errStr(_errorProviderId, error) ?? '');
      case AimuxErrorCode.noSuchTool:
        // The accessor delivers the tool set as a JSON string array, or NULL
        // when no tool set was supplied.
        final tools = _errStr(_errorAvailableTools, error);
        return NoSuchToolError(message,
            retryable: retryable,
            toolName: _errStr(_errorToolName, error) ?? '',
            availableTools:
                tools == null ? null : (jsonDecode(tools) as List).cast<String>());
      case AimuxErrorCode.invalidToolInput:
        return InvalidToolInputError(message,
            retryable: retryable,
            toolName: _errStr(_errorToolName, error) ?? '',
            toolInput: _errStr(_errorToolInput, error) ?? '');
      case AimuxErrorCode.toolCallRepair:
        final original = _errStr(_errorOriginalError, error);
        return ToolCallRepairError(message,
            retryable: retryable,
            originalError: original == null ? null : jsonDecode(original));
      default:
        try {
          return AimuxException.fromCode(code, message, retryable: retryable);
        } on StateError {
          throw StateError('aimux ffi: $context: unknown error code $code');
        }
    }
  }

  /// Dispatch [code] → concrete subclass (defaults for status when `-1`).
  factory AimuxException.fromCode(
    int code,
    String message, {
    int status = -1,
    int retryMs = -1,
    bool retryable = false,
  }) {
    switch (code) {
      case AimuxErrorCode.jsonParse:
        return JSONParseError(message, status: status, retryMs: retryMs, retryable: retryable);
      case AimuxErrorCode.invalidResponseData:
        return InvalidResponseDataError(message, status: status, retryMs: retryMs, retryable: retryable);
      case AimuxErrorCode.invalidArgument:
        return InvalidArgumentError(message, status: status, retryMs: retryMs, retryable: retryable);
      case AimuxErrorCode.invalidPrompt:
        return InvalidPromptError(message, status: status, retryMs: retryMs, retryable: retryable);
      case AimuxErrorCode.tokenExpired:
        return TokenExpiredError(
          message,
          status: status == -1 ? 401 : status,
          retryMs: retryMs,
          retryable: retryable,
        );
      case AimuxErrorCode.unsupportedFunctionality:
        return UnsupportedFunctionalityError(message, status: status, retryMs: retryMs, retryable: retryable);
      case AimuxErrorCode.noSuchModel:
        return NoSuchModelError(message, status: status, retryMs: retryMs, retryable: retryable);
      case AimuxErrorCode.noSuchProvider:
        return NoSuchProviderError(message, status: status, retryMs: retryMs, retryable: retryable);
      case AimuxErrorCode.apiCall:
        return APICallError(message, status: status, retryMs: retryMs, retryable: retryable);
      case AimuxErrorCode.timeout:
        return AimuxTimeoutError(message, status: status, retryMs: retryMs, retryable: retryable);
      case AimuxErrorCode.aborted:
        return RequestAbortedError(message, status: status, retryMs: retryMs, retryable: retryable);
      case AimuxErrorCode.noSuchTool:
        return NoSuchToolError(message, status: status, retryMs: retryMs, retryable: retryable);
      case AimuxErrorCode.invalidToolInput:
        return InvalidToolInputError(message, status: status, retryMs: retryMs, retryable: retryable);
      case AimuxErrorCode.toolCallRepair:
        return ToolCallRepairError(message, status: status, retryMs: retryMs, retryable: retryable);
      case AimuxErrorCode.other:
        return OtherError(message, status: status, retryMs: retryMs, retryable: retryable);
      default:
        throw StateError('Unknown AimuxErrorCode: $code');
    }
  }

  /// Core `error_type()` name for [code].
  String get codeName => AimuxErrorCode.name(code);

  @override
  String toString() => '$runtimeType: $message';
}

// ── Concrete subclasses (one per live AimuxErrorCode) ───────────────────────

/// JSON parse / serialize failure.
class JSONParseError extends AimuxException {
  JSONParseError(super.message, {super.status, super.retryMs, super.retryable})
      : super(code: AimuxErrorCode.jsonParse);
}

/// Invalid / malformed response data (streaming or decode failure).
class InvalidResponseDataError extends AimuxException {
  InvalidResponseDataError(super.message,
      {super.status, super.retryMs, super.retryable})
      : super(code: AimuxErrorCode.invalidResponseData);
}

/// The model called a tool that is not in the supplied tool set.
class NoSuchToolError extends AimuxException {
  /// The tool name the model called.
  final String toolName;

  /// The available tool names, or null when no tool set was supplied.
  final List<String>? availableTools;

  NoSuchToolError(super.message,
      {super.status,
      super.retryMs,
      super.retryable,
      this.toolName = '',
      this.availableTools})
      : super(code: AimuxErrorCode.noSuchTool);
}

/// The model produced tool arguments that fail to parse or validate.
class InvalidToolInputError extends AimuxException {
  /// The tool name the model called.
  final String toolName;

  /// The raw argument text the model produced.
  final String toolInput;

  InvalidToolInputError(super.message,
      {super.status,
      super.retryMs,
      super.retryable,
      this.toolName = '',
      this.toolInput = ''})
      : super(code: AimuxErrorCode.invalidToolInput);
}

/// A `repairToolCall` hook itself failed.
class ToolCallRepairError extends AimuxException {
  /// The original lookup/parse/validation error the hook was repairing,
  /// decoded from its externally-tagged wire JSON (the same shape as
  /// `ToolCall.error`).
  final dynamic originalError;

  ToolCallRepairError(super.message,
      {super.status, super.retryMs, super.retryable, this.originalError})
      : super(code: AimuxErrorCode.toolCallRepair);
}

/// Invalid argument (null args, invalid or expired handles, …).
class InvalidArgumentError extends AimuxException {
  InvalidArgumentError(super.message, {super.status, super.retryMs, super.retryable})
      : super(code: AimuxErrorCode.invalidArgument);
}

/// Invalid prompt.
class InvalidPromptError extends AimuxException {
  InvalidPromptError(super.message, {super.status, super.retryMs, super.retryable})
      : super(code: AimuxErrorCode.invalidPrompt);
}

/// Access token expired.
class TokenExpiredError extends AimuxException {
  TokenExpiredError(super.message, {super.status = 401, super.retryMs, super.retryable})
      : super(code: AimuxErrorCode.tokenExpired);
}

/// Unsupported functionality.
class UnsupportedFunctionalityError extends AimuxException {
  UnsupportedFunctionalityError(super.message,
      {super.status, super.retryMs, super.retryable})
      : super(code: AimuxErrorCode.unsupportedFunctionality);
}

/// No such model in registry / catalogue.
class NoSuchModelError extends AimuxException {
  /// The model id that was asked for.
  final String modelId;

  /// The model type it was asked for as.
  final String modelType;

  NoSuchModelError(super.message,
      {super.status, super.retryMs, super.retryable, this.modelId = '', this.modelType = ''})
      : super(code: AimuxErrorCode.noSuchModel);
}

/// No such provider name.
class NoSuchProviderError extends AimuxException {
  /// The provider id that was asked for.
  final String providerId;

  NoSuchProviderError(super.message,
      {super.status, super.retryMs, super.retryable, this.providerId = ''})
      : super(code: AimuxErrorCode.noSuchProvider);
}

/// Provider API call failed (AI SDK `APICallError` analogue) — every
/// HTTP-shaped failure. [status] is the classification (401 auth, 404 model,
/// 429 rate limit + [retryMs]); `-1` means no HTTP response was ever observed
/// — a missing API key, an error built without a request, or a transport
/// failure. Read [retryable] to decide on a retry; [status] cannot tell those
/// two apart.
class APICallError extends AimuxException {
  /// The provider's own error code, e.g. `insufficient_quota`.
  final String? providerCode;

  /// The failure's own text without the composed prefix [message] carries,
  /// e.g. `slow down`.
  final String? providerMessage;

  /// Provider request id, for support tickets.
  final String? requestId;

  /// Raw response body.
  final String? responseBody;

  APICallError(super.message,
      {super.status,
      super.retryMs,
      super.retryable,
      this.providerCode,
      this.providerMessage,
      this.requestId,
      this.responseBody})
      : super(code: AimuxErrorCode.apiCall);
}

/// Request timed out. (Prefixed to avoid shadowing dart:async
/// `TimeoutError`.)
class AimuxTimeoutError extends AimuxException {
  AimuxTimeoutError(super.message, {super.status, super.retryMs, super.retryable})
      : super(code: AimuxErrorCode.timeout);
}

/// Request aborted (not DOM `AbortError`).
class RequestAbortedError extends AimuxException {
  RequestAbortedError(super.message, {super.status, super.retryMs, super.retryable})
      : super(code: AimuxErrorCode.aborted);
}

/// Unclassified failure (`AIMUX_E_OTHER`).
class OtherError extends AimuxException {
  OtherError(super.message, {super.status, super.retryMs, super.retryable})
      : super(code: AimuxErrorCode.other);
}

// ─────────────────────────────────────────────────────────────────────────────
// Recording errors (aimux-error.h aimux_error_code_t) — independent
// of AimuxException, mirroring Rust's separate `recording::RecordingError`.
// ─────────────────────────────────────────────────────────────────────────────

/// Code of a [RecordingException]. Dart keeps six values independent of the
/// C transport codes 100..105. Only
/// [writerGone], [flushTimeout] and [write] are reachable from a flush.
enum RecordingErrorCode {
  init,
  openFile,
  spawn,
  writerGone,
  flushTimeout,
  write;

  static RecordingErrorCode fromCode(int code) {
    if (code < 100 || code >= 100 + values.length) {
      throw StateError('Unknown AimuxRecordingErrorCode: $code');
    }
    return values[code - 100];
  }
}

/// Recording failure reported by `initRecording()` / `recordingTryFlush()`.
/// NOT an [AimuxException] — the recorder is a separate subsystem with its
/// own closed error set.
class RecordingException implements Exception {
  final RecordingErrorCode code;
  final String message;
  RecordingException(this.code, this.message);

  /// Decode a returned `const aimux_error_t *` [error]. The caller
  /// ([expectRecordingError]) frees it.
  factory RecordingException._decode(Pointer<Void> error) {
    final code = RecordingErrorCode.fromCode(_errorCode(error));
    final message = _errStr(_errorMessage, error) ??
        'aimux: recording ${code.name}';
    return RecordingException(code, message);
  }

  @override
  String toString() => 'RecordingException(${code.name}): $message';
}
