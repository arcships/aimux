// Aimux.swift — Swift wrapper around the aimux-ffi C ABI.
//
// This is the C ABI path (§3.2). Swift calls the C functions from aimux-ffi
// and wraps them in a Swifty API with ARC-managed handles.

import CAimuxFFI
import Foundation

// ─────────────────────────────────────────────────────────────────────────────
// C returned errors (aimux-error.h) — distinct from the Swift types below.
//
// Every fallible C function returns `aimux_error_t *` (`OpaquePointer?`):
// NULL = success (result in the trailing out-param), non-NULL = failure. The
// unified code is AiMuxError (1...13, 15...17), RecordingError (100...105), or a C ABI
// failure (200...206). The three `expect*` decoders copy the relevant fields, release
// it with `aimux_error_free` (exactly once) and return the Swift error
// to throw. Errors are not handles: never `aimux_drop_handle` one.
// ─────────────────────────────────────────────────────────────────────────────

/// Copy a caller-owned C string and free it (`aimux_free_string`).
private func takeCString(_ p: UnsafeMutablePointer<CChar>?) -> String? {
    guard let p else { return nil }
    defer { aimux_free_string(p) }
    return String(cString: p)
}

/// The language invariant error: a binding/library contract broke.
func invariant(_ message: String) -> DecodingError {
    .dataCorrupted(.init(codingPath: [], debugDescription: message))
}

/// Decode a returned error from a call that only exposes C ABI failures
/// (NULL / non-UTF-8 argument, dead handle, re-entrant call, ...) — a
/// binding/library invariant, never reachable from a correct binding. Reads
/// `aimux_error_message`, frees `e`, returns
/// `DecodingError.dataCorrupted("aimux ffi: <context>: <message>")`.
func expectFfiError(_ e: OpaquePointer, context: String) -> any Error {
    defer { aimux_error_free(e) }
    let code = aimux_error_code(e)
    guard (200...206).contains(code) else {
        return invariant("aimux ffi: \(context): expected C ABI failure code, got \(code)")
    }
    let message = takeCString(aimux_error_message(e)) ?? "unknown failure"
    return invariant("aimux ffi: \(context): \(message)")
}

/// Decode a returned error from an `[AiMuxError]` call: 1...13 / 15...17 becomes
/// `AimuxError`; 200...206 is decoded by `expectFfiError`. Frees `e` once.
func expectAimuxError(_ e: OpaquePointer, context: String) -> any Error {
    let code = aimux_error_code(e)
    if (200...206).contains(code) {
        return expectFfiError(e, context: context)
    }
    defer { aimux_error_free(e) }
    return AimuxError.fromC(e)
        ?? invariant("aimux ffi: \(context): unknown aimux_error_code_t \(code)")
}

/// Decode a returned error from a `[RecordingError]` call: 100...105 becomes
/// `RecordingError`; 200...206 is a C ABI failure. Frees `e` once.
func expectRecordingError(_ e: OpaquePointer, context: String) -> any Error {
    let code = aimux_error_code(e)
    if (200...206).contains(code) {
        return expectFfiError(e, context: context)
    }
    defer { aimux_error_free(e) }
    return RecordingError.fromC(e)
        ?? invariant("aimux ffi: \(context): unknown aimux_error_code_t \(code)")
}

/// Reject a syntactically invalid raw JSON parameter before it crosses the
/// C ABI, so a malformed-wire-JSON C ABI failure is never produced by
/// this binding. Throws `DecodingError.dataCorrupted` naming `parameter`.
///
/// Optional (`String?`) parameters follow the FFI's "empty means default"
/// rule: `nil` and blank strings pass. Required (`String`) parameters must
/// be valid JSON — an empty string is rejected.
func validateJson(_ json: String?, parameter: String) throws {
    guard let json, !json.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
    try validateJson(json, parameter: parameter)
}

func validateJson(_ json: String, parameter: String) throws {
    do {
        _ = try JSONSerialization.jsonObject(with: Data(json.utf8), options: [.fragmentsAllowed])
    } catch {
        throw DecodingError.dataCorrupted(
            .init(codingPath: [], debugDescription: "\(parameter): invalid JSON: \(error.localizedDescription)")
        )
    }
}

/// Run an `[AiMuxError]` call writing an owned `char*` (JSON) result to
/// `char **out`: copies it into a Swift `String` and frees the C allocation,
/// or throws the decoded error.
func ffiStringCall(
    context: String = #function,
    _ body: (UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>) -> OpaquePointer?
) throws -> String {
    var out: UnsafeMutablePointer<CChar>? = nil
    if let e = body(&out) { throw expectAimuxError(e, context: context) }
    guard let out else { throw invariant("aimux ffi: \(context): success but no result written") }
    defer { aimux_free_string(out) }
    return String(cString: out)
}

// ─────────────────────────────────────────────────────────────────────────────
// Errors
// ─────────────────────────────────────────────────────────────────────────────

/// Structured aimux failure type (Swift `Error`).
///
/// Maps 1:1 from the 15 core `AiMuxError` variants. Every HTTP-shaped failure
/// is `.apiCall` (`AIMUX_E_API_CALL`). Only aimux-core produces these: a
/// binding-local failure (raw JSON that does not parse, a typed value that
/// fails to encode, library output that fails to decode) surfaces as the
/// native `DecodingError` / `EncodingError`. Nothing a caller can pass in
/// traps: use-after-close throws `DecodingError.dataCorrupted` (deliberately
/// not `AimuxTranscriptionEndedError` — a truncated transcript must not read
/// as a complete one), an empty `router([])` throws C's zero-children failure.
///
/// ```swift
/// do {
///     let result = try model.generateText(prompt: "\"hi\"")
/// } catch let e as AimuxError {
///     print(e.message, e.status, e.retryMs)
///     // Classification is the status field: 429 → rate limited (e.retryMs),
///     // 401 → auth, 404 → model not found.
///     if case .apiCall = e, e.status == 429 { /* back off */ }
///     // Whether a retry is worth it is `e.retryable`, never the status.
///     if e.retryable { /* retry */ }
/// }
/// ```
public enum AimuxError: Error, LocalizedError, CustomStringConvertible, Equatable, Sendable {
    case jsonParse(message: String, status: Int, retryMs: Int64, retryable: Bool)
    case invalidResponseData(message: String, status: Int, retryMs: Int64, retryable: Bool)
    case invalidArgument(message: String, status: Int, retryMs: Int64, retryable: Bool)
    case invalidPrompt(message: String, status: Int, retryMs: Int64, retryable: Bool)
    case tokenExpired(message: String, status: Int, retryMs: Int64, retryable: Bool)
    case unsupportedFunctionality(message: String, status: Int, retryMs: Int64, retryable: Bool)
    case noSuchModel(message: String, status: Int, retryMs: Int64, retryable: Bool, modelId: String, modelType: String)
    case noSuchProvider(message: String, status: Int, retryMs: Int64, retryable: Bool, providerId: String)
    /// Every HTTP-shaped failure: read `status` to classify (401 auth,
    /// 404 model, 429 rate limit). A `nil` status means no HTTP response was
    /// ever observed — a missing API key, an error built without a request, or
    /// a transport failure; read `retryable` to tell those apart, `status`
    /// cannot.
    case apiCall(message: String, status: Int, retryMs: Int64, retryable: Bool, providerCode: String? = nil, providerMessage: String? = nil, requestId: String? = nil, responseBody: String? = nil)
    case timeout(message: String, status: Int, retryMs: Int64, retryable: Bool)
    case aborted(message: String, status: Int, retryMs: Int64, retryable: Bool)
    /// The model called a tool that is not in the supplied tool set.
    case noSuchTool(message: String, status: Int, retryMs: Int64, retryable: Bool, toolName: String, availableTools: [String]?)
    /// The model produced tool arguments that fail to parse or validate.
    case invalidToolInput(message: String, status: Int, retryMs: Int64, retryable: Bool, toolName: String, toolInput: String)
    /// A `repairToolCall` hook itself failed; `originalError` is the error it
    /// was repairing, as externally-tagged wire JSON (the same encoding as
    /// `ToolCall.error`).
    case toolCallRepair(message: String, status: Int, retryMs: Int64, retryable: Bool, originalError: String)
    case other(message: String, status: Int, retryMs: Int64, retryable: Bool)

    // MARK: Accessors

    /// The C-derived payload.
    private var payload: (message: String, status: Int, retryMs: Int64, retryable: Bool) {
        switch self {
        case .jsonParse(let m, let s, let r, let t),
             .invalidResponseData(let m, let s, let r, let t),
             .invalidArgument(let m, let s, let r, let t),
             .invalidPrompt(let m, let s, let r, let t),
             .tokenExpired(let m, let s, let r, let t),
             .unsupportedFunctionality(let m, let s, let r, let t),
             .noSuchModel(let m, let s, let r, let t, _, _),
             .noSuchProvider(let m, let s, let r, let t, _),
             .apiCall(let m, let s, let r, let t, _, _, _, _),
             .timeout(let m, let s, let r, let t),
             .aborted(let m, let s, let r, let t),
             .noSuchTool(let m, let s, let r, let t, _, _),
             .invalidToolInput(let m, let s, let r, let t, _, _),
             .toolCallRepair(let m, let s, let r, let t, _),
             .other(let m, let s, let r, let t):
            return (m, s, r, t)
        }
    }

    /// The C `aimux_error_code_t` (`aimux-error.h`).
    public var code: Int32 {
        let c: aimux_error_code_t
        switch self {
        case .jsonParse: c = AIMUX_E_JSON_PARSE
        case .invalidResponseData: c = AIMUX_E_INVALID_RESPONSE_DATA
        case .invalidArgument: c = AIMUX_E_INVALID_ARGUMENT
        case .invalidPrompt: c = AIMUX_E_INVALID_PROMPT
        case .tokenExpired: c = AIMUX_E_TOKEN_EXPIRED
        case .unsupportedFunctionality: c = AIMUX_E_UNSUPPORTED_FUNCTIONALITY
        case .noSuchModel: c = AIMUX_E_NO_SUCH_MODEL
        case .noSuchProvider: c = AIMUX_E_NO_SUCH_PROVIDER
        case .apiCall: c = AIMUX_E_API_CALL
        case .timeout: c = AIMUX_E_TIMEOUT
        case .aborted: c = AIMUX_E_ABORTED
        case .noSuchTool: c = AIMUX_E_NO_SUCH_TOOL
        case .invalidToolInput: c = AIMUX_E_INVALID_TOOL_INPUT
        case .toolCallRepair: c = AIMUX_E_TOOL_CALL_REPAIR
        case .other: c = AIMUX_E_OTHER
        }
        return Int32(bitPattern: c.rawValue)
    }

    /// Human-readable message (C `message` field).
    public var message: String { payload.message }

    /// HTTP status code, or `nil` when not applicable (C reports `-1`).
    public var status: Int? {
        payload.status >= 0 ? payload.status : nil
    }

    /// Rate-limit retry hint in milliseconds, or `nil` when not applicable
    /// (C reports `-1`). `0` means retry immediately.
    public var retryMs: Int64? {
        payload.retryMs >= 0 ? payload.retryMs : nil
    }

    /// Whether retrying may help — the `AiMuxError` verdict, carried across the C
    /// ABI. Not derivable from `status`: a transport failure (request went
    /// out, connection reset) and a missing API key (request never went out)
    /// both report no status and disagree here.
    public var retryable: Bool { payload.retryable }

    /// `.apiCall` only: the provider's own error code (e.g. `"insufficient_quota"`).
    public var providerCode: String? {
        if case .apiCall(_, _, _, _, let v, _, _, _) = self { return v }
        return nil
    }

    /// `.apiCall` only: the failure's own text without the composed prefix `message` carries (e.g. `"slow down"`).
    public var providerMessage: String? {
        if case .apiCall(_, _, _, _, _, let v, _, _) = self { return v }
        return nil
    }

    /// `.apiCall` only: the provider request id, for support tickets.
    public var requestId: String? {
        if case .apiCall(_, _, _, _, _, _, let v, _) = self { return v }
        return nil
    }

    /// `.apiCall` only: the raw response body.
    public var responseBody: String? {
        if case .apiCall(_, _, _, _, _, _, _, let v) = self { return v }
        return nil
    }

    /// `.noSuchModel` only: the model id that was asked for.
    public var modelId: String? {
        if case .noSuchModel(_, _, _, _, let v, _) = self { return v }
        return nil
    }

    /// `.noSuchModel` only: the model type it was asked for as.
    public var modelType: String? {
        if case .noSuchModel(_, _, _, _, _, let v) = self { return v }
        return nil
    }

    /// `.noSuchProvider` only: the provider id that was asked for.
    public var providerId: String? {
        if case .noSuchProvider(_, _, _, _, let v) = self { return v }
        return nil
    }

    /// `.noSuchTool` / `.invalidToolInput` only: the tool name the model called.
    public var toolName: String? {
        switch self {
        case .noSuchTool(_, _, _, _, let v, _), .invalidToolInput(_, _, _, _, let v, _):
            return v
        default:
            return nil
        }
    }

    /// `.noSuchTool` only: the available tool names, or `nil` when no tool set
    /// was supplied.
    public var availableTools: [String]? {
        if case .noSuchTool(_, _, _, _, _, let v) = self { return v }
        return nil
    }

    /// `.invalidToolInput` only: the raw argument text the model produced.
    public var toolInput: String? {
        if case .invalidToolInput(_, _, _, _, _, let v) = self { return v }
        return nil
    }

    /// `.toolCallRepair` only: the original lookup/parse/validation error as
    /// externally-tagged wire JSON (the same encoding as `ToolCall.error`).
    public var originalError: String? {
        if case .toolCallRepair(_, _, _, _, let v) = self { return v }
        return nil
    }

    public var description: String { message }

    public var errorDescription: String? {
        message
    }

    // MARK: C mapping

    /// Map a borrowed `AiMuxError` view into the Swift enum. Reads
    /// `code`/`message`/`retryable`/`status`/`retry_ms`, then only the payload
    /// getters owned by that code (each owned string is copied and freed with
    /// `aimux_free_string`). Returns `nil` for a code outside
    /// `aimux_error_code_t` (header/library mismatch — the caller throws the
    /// invariant error). Does not release the owner; `expectAimuxError` does.
    static func fromC(_ h: OpaquePointer) -> AimuxError? {
        let raw = aimux_error_code(h)
        let code = aimux_error_code_t(UInt32(bitPattern: raw))
        let rawMsg = takeCString(aimux_error_message(h)) ?? ""
        let retryable = aimux_error_retryable(h) != 0
        // status / retry_ms are API_CALL payload (-1 under any other code),
        // which is exactly what the enum's Int/Int64 fields expect.
        let status = Int(aimux_error_status(h))
        let retryMs = aimux_error_retry_ms(h)
        let message = rawMsg.isEmpty ? "aimux: operation failed" : rawMsg

        switch code {
        case AIMUX_E_JSON_PARSE:
            return .jsonParse(message: message, status: status, retryMs: retryMs, retryable: retryable)
        case AIMUX_E_INVALID_RESPONSE_DATA:
            return .invalidResponseData(message: message, status: status, retryMs: retryMs, retryable: retryable)
        case AIMUX_E_INVALID_ARGUMENT:
            return .invalidArgument(message: message, status: status, retryMs: retryMs, retryable: retryable)
        case AIMUX_E_INVALID_PROMPT:
            return .invalidPrompt(message: message, status: status, retryMs: retryMs, retryable: retryable)
        case AIMUX_E_TOKEN_EXPIRED:
            // TokenExpired is definitionally an observed 401 (RFC-0018).
            return .tokenExpired(message: message, status: status == -1 ? 401 : status, retryMs: retryMs, retryable: retryable)
        case AIMUX_E_UNSUPPORTED_FUNCTIONALITY:
            return .unsupportedFunctionality(message: message, status: status, retryMs: retryMs, retryable: retryable)
        case AIMUX_E_NO_SUCH_MODEL:
            return .noSuchModel(message: message, status: status, retryMs: retryMs, retryable: retryable,
                                modelId: takeCString(aimux_error_model_id(h)) ?? "",
                                modelType: takeCString(aimux_error_model_type(h)) ?? "")
        case AIMUX_E_NO_SUCH_PROVIDER:
            return .noSuchProvider(message: message, status: status, retryMs: retryMs, retryable: retryable,
                                   providerId: takeCString(aimux_error_provider_id(h)) ?? "")
        case AIMUX_E_API_CALL:
            return .apiCall(message: message, status: status, retryMs: retryMs, retryable: retryable,
                            providerCode: takeCString(aimux_error_provider_code(h)),
                            providerMessage: takeCString(aimux_error_provider_message(h)),
                            requestId: takeCString(aimux_error_request_id(h)),
                            responseBody: takeCString(aimux_error_response_body(h)))
        case AIMUX_E_TIMEOUT:
            return .timeout(message: message, status: status, retryMs: retryMs, retryable: retryable)
        case AIMUX_E_ABORTED:
            return .aborted(message: message, status: status, retryMs: retryMs, retryable: retryable)
        case AIMUX_E_NO_SUCH_TOOL:
            // The accessor's JSON string array (or NULL) → [String]?; a decode
            // failure would be an FFI contract break, treated as "absent".
            let tools = takeCString(aimux_error_available_tools(h))
                .flatMap { try? JSONDecoder().decode([String].self, from: Data($0.utf8)) }
            return .noSuchTool(message: message, status: status, retryMs: retryMs, retryable: retryable,
                               toolName: takeCString(aimux_error_tool_name(h)) ?? "",
                               availableTools: tools)
        case AIMUX_E_INVALID_TOOL_INPUT:
            return .invalidToolInput(message: message, status: status, retryMs: retryMs, retryable: retryable,
                                     toolName: takeCString(aimux_error_tool_name(h)) ?? "",
                                     toolInput: takeCString(aimux_error_tool_input(h)) ?? "")
        case AIMUX_E_TOOL_CALL_REPAIR:
            return .toolCallRepair(message: message, status: status, retryMs: retryMs, retryable: retryable,
                                   originalError: takeCString(aimux_error_original_error(h)) ?? "")
        case AIMUX_E_OTHER:
            return .other(message: message, status: status, retryMs: retryMs, retryable: retryable)
        default:
            return nil
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// RecordingError — mirrors aimux-core `recording::RecordingError`, a type
// unrelated to `AimuxError` (separate range in the unified C code enum).
// ─────────────────────────────────────────────────────────────────────────────

/// Recorder failure reported by `Model.recordingTryFlush()`. Independent of
/// `AimuxError`: shares only `Error`.
public struct RecordingError: Error, LocalizedError, CustomStringConvertible, Equatable, Sendable {
    /// Mirrors `aimux_error_code_t` (aimux-error.h) in declaration order.
    /// Only `writerGone` / `flushTimeout` / `write` are reachable from a flush.
    public enum Code: Equatable, Sendable {
        case initFailed, openFile, spawn, writerGone, flushTimeout, write
    }

    public let code: Code
    public let message: String

    public init(code: Code, message: String) {
        self.code = code
        self.message = message
    }

    public var description: String { "\(message) (recording, \(code))" }
    public var errorDescription: String? { message }

    static func code(fromC rawCode: Int32) -> Code? {
        switch aimux_error_code_t(UInt32(bitPattern: rawCode)) {
        case AIMUX_E_RECORDING_INIT: return .initFailed
        case AIMUX_E_RECORDING_OPEN_FILE: return .openFile
        case AIMUX_E_RECORDING_SPAWN: return .spawn
        case AIMUX_E_RECORDING_WRITER_GONE: return .writerGone
        case AIMUX_E_RECORDING_FLUSH_TIMEOUT: return .flushTimeout
        case AIMUX_E_RECORDING_WRITE: return .write
        default: return nil
        }
    }

    /// Map a returned error through its code and message getters. Frees the
    /// owned message string; `nil` for a code outside `aimux_error_code_t`.
    /// `expectRecordingError` frees the returned error.
    static func fromC(_ h: OpaquePointer) -> RecordingError? {
        guard let code = code(fromC: aimux_error_code(h)) else { return nil }
        let message = takeCString(aimux_error_message(h)) ?? ""
        return RecordingError(code: code, message: message.isEmpty ? "aimux: recording error" : message)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Model — ARC-managed wrapper around a C ABI handle.
// ─────────────────────────────────────────────────────────────────────────────

/// A model instance backed by a Rust `Arc<dyn LanguageModel>`.
///
/// The C handle is automatically released when this object is deallocated.
public final class Model: @unchecked Sendable {

    // The opaque handle from aimux-ffi. 0 means invalid/freed.
    private var handle: UInt64

    /// File-private initializer — use `Model.openai()` etc. to create.
    fileprivate init(handle: UInt64) {
        self.handle = handle
    }

    deinit {
        if handle != 0 {
            aimux_drop_handle(handle)
        }
    }

    /// Run a C constructor writing a handle to `uint64_t *out_handle`; throws
    /// the returned error decoded by `expecting` (`expectAimuxError` for `[AiMuxError]`
    /// constructors, `expectFfiError` for `[C ABI]` ones).
    static func wrapHandle(
        expecting decode: (OpaquePointer, String) -> any Error = expectAimuxError,
        context: String = #function,
        _ call: (UnsafeMutablePointer<UInt64>) -> OpaquePointer?
    ) throws -> UInt64 {
        var h: UInt64 = 0
        if let e = call(&h) { throw decode(e, context) }
        return h
    }

    // ── Provider constructors ──────────────────────────────────────────────

    /// Initialize the global logger (RFC-0014).
    ///
    /// Idempotent — safe to call any number of times from any thread; only the
    /// first call has an effect. If the host already registered its own
    /// `tracing` subscriber, this is a no-op (aimux never overrides a
    /// consumer's logger).
    ///
    /// - Parameter level: "off" | "error" | "warn" | "info" | "debug" |
    ///   "trace"; empty defaults to "warn". The `AIMUX_LOG` / `AIMUX_LOG_LEVEL`
    ///   environment variables take precedence when set. Logs go to stderr.
    public static func initLogging(level: String) {
        // [C ABI]: a Swift String is always valid UTF-8, so this cannot fail.
        if let e = aimux_init_logging(level.isEmpty ? "warn" : level) {
            preconditionFailure("\(expectFfiError(e, context: "initLogging"))")
        }
    }

    /// Create an OpenAI model instance.
    public static func openai(apiKey: String, modelId: String) throws -> Model {
        let handle = try wrapHandle { aimux_openai_new(apiKey, modelId, $0) }
        return Model(handle: handle)
    }

    /// Create an Anthropic model instance.
    public static func anthropic(apiKey: String, modelId: String) throws -> Model {
        let handle = try wrapHandle { aimux_anthropic_new(apiKey, modelId, $0) }
        return Model(handle: handle)
    }

    /// Create an OpenAI model instance with a custom base URL.
    ///
    /// An empty `baseUrl` falls back to the provider's standard URL
    /// (see `aimux_openai_new_with_base`).
    public static func openai(apiKey: String, modelId: String, baseUrl: String) throws -> Model {
        let handle = try wrapHandle { aimux_openai_new_with_base(apiKey, modelId, baseUrl, $0) }
        return Model(handle: handle)
    }

    /// Create an Anthropic model instance with a custom base URL.
    public static func anthropic(apiKey: String, modelId: String, baseUrl: String) throws -> Model {
        let handle = try wrapHandle { aimux_anthropic_new_with_base(apiKey, modelId, baseUrl, $0) }
        return Model(handle: handle)
    }

    /// Create a Cohere model instance.
    public static func cohere(apiKey: String, modelId: String) throws -> Model {
        let handle = try wrapHandle { aimux_cohere_new(apiKey, modelId, $0) }
        return Model(handle: handle)
    }

    /// Create a Cohere model instance with a custom base URL.
    public static func cohere(apiKey: String, modelId: String, baseUrl: String) throws -> Model {
        let handle = try wrapHandle { aimux_cohere_new_with_base(apiKey, modelId, baseUrl, $0) }
        return Model(handle: handle)
    }

    /// Create a Mistral model instance.
    public static func mistral(apiKey: String, modelId: String) throws -> Model {
        let handle = try wrapHandle { aimux_mistral_new(apiKey, modelId, $0) }
        return Model(handle: handle)
    }

    /// Create a Mistral model instance with a custom base URL.
    public static func mistral(apiKey: String, modelId: String, baseUrl: String) throws -> Model {
        let handle = try wrapHandle { aimux_mistral_new_with_base(apiKey, modelId, baseUrl, $0) }
        return Model(handle: handle)
    }

    /// Create an xAI model instance.
    public static func xai(apiKey: String, modelId: String) throws -> Model {
        let handle = try wrapHandle { aimux_xai_new(apiKey, modelId, $0) }
        return Model(handle: handle)
    }

    /// Create an xAI model instance with a custom base URL.
    public static func xai(apiKey: String, modelId: String, baseUrl: String) throws -> Model {
        let handle = try wrapHandle { aimux_xai_new_with_base(apiKey, modelId, baseUrl, $0) }
        return Model(handle: handle)
    }

    /// Create a Bedrock model instance (AWS SigV4 credentials).
    public static func bedrock(
        accessKeyId: String, secretAccessKey: String, region: String, modelId: String
    ) throws -> Model {
        let handle = try wrapHandle {
            aimux_bedrock_new(accessKeyId, secretAccessKey, region, modelId, $0)
        }
        return Model(handle: handle)
    }

    /// Create a Bedrock model instance with a custom base URL.
    public static func bedrock(
        accessKeyId: String, secretAccessKey: String, region: String, modelId: String, baseUrl: String
    ) throws -> Model {
        let handle = try wrapHandle {
            aimux_bedrock_new_with_base(accessKeyId, secretAccessKey, region, modelId, baseUrl, $0)
        }
        return Model(handle: handle)
    }

    /// Create a Vertex AI model instance (GCP bearer token).
    public static func vertex(
        accessToken: String, project: String, location: String, modelId: String
    ) throws -> Model {
        let handle = try wrapHandle {
            aimux_vertex_new(accessToken, project, location, modelId, $0)
        }
        return Model(handle: handle)
    }

    /// Create a Vertex AI model instance with a custom base URL.
    public static func vertex(
        accessToken: String, project: String, location: String, modelId: String, baseUrl: String
    ) throws -> Model {
        let handle = try wrapHandle {
            aimux_vertex_new_with_base(accessToken, project, location, modelId, baseUrl, $0)
        }
        return Model(handle: handle)
    }

    /// Create an Anthropic-on-AWS model instance (API key + region).
    public static func anthropicAws(apiKey: String, region: String, modelId: String) throws -> Model {
        let handle = try wrapHandle { aimux_anthropic_aws_new(apiKey, region, modelId, $0) }
        return Model(handle: handle)
    }

    /// Create an Anthropic-on-AWS model instance with a custom base URL.
    public static func anthropicAws(
        apiKey: String, region: String, modelId: String, baseUrl: String
    ) throws -> Model {
        let handle = try wrapHandle {
            aimux_anthropic_aws_new_with_base(apiKey, region, modelId, baseUrl, $0)
        }
        return Model(handle: handle)
    }

    /// Create an Azure OpenAI model instance (API key + resource name).
    /// The deployment name is passed as `modelId`; `apiVersion` is optional.
    public static func azure(
        apiKey: String, resourceName: String, deployment: String, apiVersion: String? = nil
    ) throws -> Model {
        let handle = try wrapHandle {
            aimux_azure_new(apiKey, resourceName, deployment, apiVersion, $0)
        }
        return Model(handle: handle)
    }

    /// Create an Azure OpenAI model instance with a custom base URL.
    public static func azureWithBase(
        apiKey: String, baseUrl: String, deployment: String, apiVersion: String? = nil
    ) throws -> Model {
        let handle = try wrapHandle {
            aimux_azure_new_with_base(apiKey, baseUrl, deployment, apiVersion, $0)
        }
        return Model(handle: handle)
    }

    /// Create a model from the provider registry by name (RFC-0017 phase 4).
    ///
    /// - Parameters:
    ///   - name: Registry provider name (e.g. `"deepseek"`, `"groq"`).
    ///   - apiKey: API key, or `nil` to read the provider's env var from the
    ///     registry entry.
    ///   - modelId: Model id.
    ///   - configJson: Optional JSON object of `ProviderOptions`
    ///     (`{"base_url": "...", "headers": {...}, "max_retries": 0,
    ///     "body_overrides": {...}}`); `nil` for defaults.
    public static func provider(
        name: String, apiKey: String? = nil, modelId: String, configJson: String? = nil
    ) throws -> Model {
        try validateJson(configJson, parameter: "configJson")
        let handle = try wrapHandle {
            aimux_provider_new(name, apiKey, modelId, configJson, $0)
        }
        return Model(handle: handle)
    }

    // ── Recording + mock replay (RFC-0023) ─────────────────────────────────

    /// Start recording: complete `Recording` entries are written as JSONL to
    /// `{dir}/recordings.jsonl` (the directory is auto-created).
    ///
    /// Recording is opt-in; calling again (with a different dir) replaces the
    /// active recorder. Like `initLogging`, this controls a global recorder and
    /// is not tied to a specific model instance.
    ///
    /// - Parameter dir: Directory that will hold `recordings.jsonl`.
    /// - Throws: `RecordingError` (`.initFailed` — dir could not be created,
    ///   `.openFile`, `.spawn`) when the recorder cannot be constructed. On
    ///   failure the previous recorder (if any) stays in place.
    public static func initRecording(dir: String) throws {
        if let e = aimux_init_recording(dir) { throw expectRecordingError(e, context: "initRecording") }
    }

    /// Start in-memory bounded recording (ring recorder with FIFO eviction).
    ///
    /// Keeps only the most recent `cap` recordings in memory, evicting the
    /// oldest first. Use this when recordings don't need to be persisted to
    /// disk.
    ///
    /// - Parameter cap: Ring capacity. Pass `nil` (the default) to use the
    ///   library default capacity (FFI `aimux_init_recording_ring_default`);
    ///   `nil` cannot fail.
    /// - Throws: `AimuxError.invalidArgument` for `cap: 0` — aimux-core
    ///   decides, so a Swift caller sees exactly what a C caller sees
    ///   (`AIMUX_E_INVALID_ARGUMENT`, "cap: must be > 0").
    public static func initRecordingRing(cap: UInt64? = nil) throws {
        guard let cap else { return aimux_init_recording_ring_default() }
        if let e = aimux_init_recording_ring(cap) {
            throw expectAimuxError(e, context: "initRecordingRing")
        }
    }

    /// Stop recording: the global recorder becomes `None`.
    public static func recordingStop() {
        aimux_recording_stop()
    }

    /// Flush the global recorder, blocking until the JSONL is on disk.
    ///
    /// No-op for the in-memory ring recorder.
    public static func recordingFlush() {
        aimux_recording_flush()
    }

    /// Checked flush: like `recordingFlush()` but throws `RecordingError`
    /// (`.writerGone` / `.flushTimeout` / `.write`) when the JSONL cannot be
    /// confirmed on disk. Returns normally when nothing is recording. The
    /// legacy `recordingFlush()` stays and never reports.
    public static func recordingTryFlush() throws {
        if let e = aimux_recording_try_flush() { throw expectRecordingError(e, context: "recordingTryFlush") }
    }

    /// Create a mock replay model from recorded JSONL (one `Recording` per
    /// line).
    ///
    /// The returned model answers `generateText` / `streamText` from the
    /// recorded data without sending any real provider request.
    ///
    /// - Parameter recordingsJsonl: The recorded JSONL content.
    /// - Returns: A `Model` backed by the mock replay handle.
    public static func mockReplay(recordingsJsonl: String) throws -> Model {
        for line in recordingsJsonl.split(whereSeparator: \.isNewline)
        where !line.allSatisfy(\.isWhitespace) {
            try validateJson(String(line), parameter: "recordingsJsonl")
        }
        let handle = try wrapHandle { aimux_mock_replay_new(recordingsJsonl, $0) }
        return Model(handle: handle)
    }

    // ── Composite models (RFC-0021 / RFC-0022) ──────────────────────────────

    /// Create a RouterModel (RFC-0021) over the given child models. The
    /// returned model routes each call to one child and falls back across the
    /// rest on error (per `configJson`).
    ///
    /// - Parameters:
    ///   - models: child models (must be non-empty — an empty array throws,
    ///     it does not trap: the array is as likely to come from a `.filter`
    ///     as from a literal, so C's zero-children failure surfaces instead).
    ///   - configJson: optional config: `{"router": "rule"|"weighted",
    ///     "weights": [...], "fallback": "on_error"|"none", "provider_name",
    ///     "model_id"}`.
    /// - Returns: a new RouterModel wrapping the children.
    public static func router(
        _ models: [Model],
        configJson: String? = nil
    ) throws -> Model {
        try validateJson(configJson, parameter: "configJson")
        let handles = models.map { $0.handle }
        let handle = try handles.withUnsafeBufferPointer { buf -> UInt64 in
            try wrapHandle { out in
                aimux_router_new(buf.baseAddress, buf.count, configJson, out)
            }
        }
        return Model(handle: handle)
    }

    /// Create a MoaModel (RFC-0022) over reference models + one aggregator.
    /// References fan out in parallel, then the aggregator synthesizes a final
    /// answer.
    ///
    /// - Parameters:
    ///   - references: reference models (may be empty — runs aggregator only).
    ///   - aggregator: the aggregator model.
    ///   - configJson: optional MoaConfig.
    /// - Returns: a new MoaModel.
    public static func moa(
        references: [Model],
        aggregator: Model,
        configJson: String? = nil
    ) throws -> Model {
        try validateJson(configJson, parameter: "configJson")
        let agg = aggregator.handle
        // Empty references: pass a NULL base address + 0 length.
        let handle: UInt64
        if references.isEmpty {
            handle = try wrapHandle { out in
                aimux_moa_new(nil, 0, agg, configJson, out)
            }
        } else {
            let refHandles = references.map { $0.handle }
            handle = try refHandles.withUnsafeBufferPointer { buf -> UInt64 in
                try wrapHandle { out in
                    aimux_moa_new(buf.baseAddress, buf.count, agg, configJson, out)
                }
            }
        }
        return Model(handle: handle)
    }

    /// Register external OpenAI-compatible providers from a JSON config string
    /// (RFC-0020).
    ///
    /// `configJSON` is `{ "providers": [ { "name", "base_url", ... } ] }`.
    /// Entries override same-named built-ins or add new ones. Like
    /// `initRecording`, this mutates process-global registry state.
    ///
    /// - Parameter configJSON: Provider registry config JSON.
    /// - Throws: `AimuxError` (`.invalidArgument`) when the registry rejects
    ///   the document.
    public static func registerProviders(_ configJSON: String) throws {
        try validateJson(configJSON, parameter: "configJSON")
        if let e = aimux_register_providers(configJSON) { throw expectAimuxError(e, context: "registerProviders") }
    }

    /// Set the global proxy configuration (M6, RFC-0016). Must be called before
    /// the first `generateText` / `streamText` call; a no-op (returns without
    /// error) if the shared HTTP client is already initialised.
    ///
    /// `configJSON` shape: `{ "http_url", "https_url", "all_url", "no_proxy" }`
    /// (all fields optional; omitting all is equivalent to relying on the
    /// `HTTP_PROXY` / `HTTPS_PROXY` / `ALL_PROXY` / `NO_PROXY` env vars).
    ///
    /// - Parameter configJSON: ProxyConfig JSON.
    /// - Throws: `AimuxError` (`.invalidArgument`) for a wrong-shape config.
    public static func initProxy(_ configJSON: String) throws {
        try validateJson(configJSON, parameter: "configJSON")
        if let e = aimux_init_proxy(configJSON) { throw expectAimuxError(e, context: "initProxy") }
    }

    // ── Provider handles (RFC-0027) ──────────────────────────────────────────

    /// Create a **provider handle** for a registry-backed provider (RFC-0027).
    ///
    /// Unlike `provider()` (which binds to a single modelId), this returns a
    /// `ProviderHandle` that supports `listModels()` and `model()`.
    public static func createProvider(
        name: String, apiKey: String? = nil, configJson: String? = nil
    ) throws -> ProviderHandle {
        try validateJson(configJson, parameter: "configJson")
        let handle = try wrapHandle {
            aimux_provider_handle_new(name, apiKey, configJson, $0)
        }
        return ProviderHandle(handle: handle)
    }

    // ── Generation ─────────────────────────────────────────────────────────

    /// Generate text (non-streaming).
    ///
    /// - Parameters:
    ///   - prompt: A prompt string or messages array (serialized as JSON).
    ///   - options: Optional GenerateTextOptions (serialized as JSON).
    /// - Returns: The JSON-serialized GenerateTextResult.
    public func generateText(prompt: String, options: String? = nil) throws -> String {
        try validateJson(prompt, parameter: "prompt")
        try validateJson(options, parameter: "options")
        return try ffiStringCall { aimux_generate_text(handle, prompt, options, $0) }
    }

    /// Generate a structured JSON object (M12, RFC-0016).
    ///
    /// Same signature as `generateText`; returns a JSON-serialized
    /// `GenerateObjectResult`. Pass `response_format: { "Json": { ... } }`
    /// via `options` for schema control; aimux-core applies JSON repair
    /// before parsing.
    ///
    /// - Parameters:
    ///   - prompt: A prompt string or messages array (serialized as JSON).
    ///   - options: Optional GenerateTextOptions (serialized as JSON).
    /// - Returns: The JSON-serialized GenerateObjectResult.
    public func generateObject(prompt: String, options: String? = nil) throws -> String {
        try validateJson(prompt, parameter: "prompt")
        try validateJson(options, parameter: "options")
        return try ffiStringCall { aimux_generate_object(handle, prompt, options, $0) }
    }

    /// Consume a stream to completion and return the aggregated result
    /// (M11, RFC-0016). Synchronous (blocks until the stream finishes).
    ///
    /// Same signature as `generateText`; returns a JSON-serialized
    /// `StreamTextResultAggregated`.
    ///
    /// - Parameters:
    ///   - prompt: A prompt string or messages array (serialized as JSON).
    ///   - options: Optional GenerateTextOptions (serialized as JSON).
    /// - Returns: The JSON-serialized StreamTextResultAggregated.
    public func consumeStreamText(prompt: String, options: String? = nil) throws -> String {
        try validateJson(prompt, parameter: "prompt")
        try validateJson(options, parameter: "options")
        return try ffiStringCall { aimux_consume_stream_text(handle, prompt, options, $0) }
    }

    /// Generate text (non-streaming) with OpenAI Chat Completions output.
    ///
    /// Same as `generateText`, but returns a serialized ChatCompletion
    /// (OpenAI "chat.completion" object). Works with any provider (RFC-0026).
    ///
    /// - Parameters:
    ///   - prompt: A prompt string or messages array (serialized as JSON).
    ///   - options: Optional GenerateTextOptions (serialized as JSON).
    /// - Returns: The JSON-serialized ChatCompletion.
    public func generateTextAsOpenAI(prompt: String, options: String? = nil) throws -> String {
        try validateJson(prompt, parameter: "prompt")
        try validateJson(options, parameter: "options")
        return try ffiStringCall { aimux_generate_text_as_openai(handle, prompt, options, $0) }
    }

    /// Stream text from the model.
    ///
    /// The C ABI returns NULL after `onDone`, or a returned error on failure
    /// (no `onDone`, no C `onError` callback). Failures are surfaced via
    /// `onError` (`AimuxError` / `DecodingError`).
    ///
    /// - Parameters:
    ///   - prompt: A prompt string (serialized as JSON).
    ///   - options: Optional GenerateTextOptions (serialized as JSON).
    ///   - onPart: Called for each StreamPart (JSON string).
    ///   - onDone: Called when the stream completes normally.
    ///   - onError: Called on stream failure with the decoded error unchanged
    ///     (`AimuxError`, or `DecodingError.dataCorrupted` for unparseable
    ///     `prompt` / `options` JSON, rejected before the C call).
    public func streamText(
        prompt: String,
        options: String? = nil,
        onPart: @escaping (String) -> Void,
        onDone: @escaping () -> Void,
        onError: @escaping (any Error) -> Void
    ) {
        stream(aimux_stream_text, prompt: prompt, options: options,
               onPart: onPart, onDone: onDone, onError: onError)
    }

    /// Stream text from the model with OpenAI Chat Completions output.
    ///
    /// Same as `streamText`, but each `onPart` receives a serialized
    /// ChatCompletionChunk (OpenAI "chat.completion.chunk" object). Works with
    /// any provider (RFC-0026). Stream options (`include_usage`,
    /// `include_reasoning`) are passed via `options` →
    /// `providerOptions.openai.stream_options`.
    public func streamTextAsOpenAI(
        prompt: String,
        options: String? = nil,
        onPart: @escaping (String) -> Void,
        onDone: @escaping () -> Void,
        onError: @escaping (any Error) -> Void
    ) {
        stream(aimux_stream_text_as_openai, prompt: prompt, options: options,
               onPart: onPart, onDone: onDone, onError: onError)
    }

    /// C `aimux_stream_text` / `aimux_stream_text_as_openai` signature.
    private typealias StreamFn = (
        UInt64, UnsafePointer<CChar>?, UnsafePointer<CChar>?,
        (@convention(c) (UnsafePointer<CChar>?, UnsafeMutableRawPointer?) -> Void)?,
        (@convention(c) (UnsafeMutableRawPointer?) -> Void)?,
        UnsafeMutableRawPointer?
    ) -> OpaquePointer?

    /// Shared body of the two closure-based stream methods (they differ only
    /// by C symbol).
    private func stream(
        _ fn: StreamFn,
        prompt: String,
        options: String?,
        onPart: @escaping (String) -> Void,
        onDone: @escaping () -> Void,
        onError: @escaping (any Error) -> Void
    ) {
        let context = StreamContext(onPart: onPart, onDone: onDone)
        let unmanaged = Unmanaged.passRetained(context)
        defer { unmanaged.release() }

        do {
            try validateJson(prompt, parameter: "prompt")
            try validateJson(options, parameter: "options")
            if let e = fn(handle, prompt, options,
                          aimuxStreamOnPart, aimuxStreamOnDone,
                          unmanaged.toOpaque()) {
                throw expectAimuxError(e, context: "streamText")
            }
        } catch {
            // AimuxError / DecodingError (bad raw JSON, C ABI invariant)
            // pass through unchanged.
            onError(error)
        }
    }

    /// Stream text as an AsyncSequence of StreamPart JSON strings.
    ///
    /// Usage:
    /// ```swift
    /// for try await part in model.streamTextAsync(prompt: "...") {
    ///     print(part)
    /// }
    /// ```
    public func streamTextAsync(
        prompt: String, options: String? = nil
    ) -> AsyncThrowingStream<String, Error> {
        AsyncThrowingStream { continuation in
            self.streamText(
                prompt: prompt, options: options,
                onPart: { continuation.yield($0) },
                onDone: { continuation.finish() },
                onError: { continuation.finish(throwing: $0) }
            )
        }
    }

    /// Stream text with OpenAI Chat Completions output as an AsyncSequence of
    /// ChatCompletionChunk JSON strings (RFC-0026).
    public func streamTextAsOpenAIAsync(
        prompt: String, options: String? = nil
    ) -> AsyncThrowingStream<String, Error> {
        AsyncThrowingStream { continuation in
            self.streamTextAsOpenAI(
                prompt: prompt, options: options,
                onPart: { continuation.yield($0) },
                onDone: { continuation.finish() },
                onError: { continuation.finish(throwing: $0) }
            )
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// ProviderHandle (RFC-0027) — provider handle for listModels / model
// ─────────────────────────────────────────────────────────────────────────────

/// A provider handle — created by `Model.createProvider`, supports `listModels()`
/// (runtime discovery) and `model()` (build a model from a discovered id).
public final class ProviderHandle: @unchecked Sendable {

    private var handle: UInt64

    fileprivate init(handle: UInt64) {
        self.handle = handle
    }

    deinit {
        if handle != 0 {
            aimux_drop_handle(handle)
        }
    }

    /// List models available on this provider (runtime discovery + anya2a spec).
    /// Returns a JSON array of ResolvedModel.
    public func listModels() throws -> String {
        return try ffiStringCall { aimux_provider_list_models(handle, $0) }
    }

    /// Build a language model from a discovered model id.
    public func model(_ modelId: String) throws -> Model {
        let h = try Model.wrapHandle { aimux_provider_model(handle, modelId, $0) }
        return Model(handle: h)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Model specs (RFC-0027) — get_model_specs
// ─────────────────────────────────────────────────────────────────────────────

/// Fetch the community model catalogue (anya2a). Returns a JSON-serialized
/// Catalogue string. Thin fetch — no caching.
///
/// - Parameter sourceUrl: Optional URL override (nil = default endpoint).
public func getModelSpecs(sourceUrl: String? = nil) throws -> String {
    try ffiStringCall { aimux_get_model_specs(sourceUrl, $0) }
}

// ─────────────────────────────────────────────────────────────────────────────
// Stream context + C trampolines
// ─────────────────────────────────────────────────────────────────────────────

/// Holds the Swift closures for stream callbacks.
/// Passed through the C ABI `stream_ctx` void* (not thread-local).
private final class StreamContext {
    let onPart: (String) -> Void
    let onDone: () -> Void

    init(onPart: @escaping (String) -> Void, onDone: @escaping () -> Void) {
        self.onPart = onPart
        self.onDone = onDone
    }
}

/// C-compatible trampoline: `on_part(const char *json, void *stream_ctx)`.
private func aimuxStreamOnPart(
    _ jsonPtr: UnsafePointer<CChar>?,
    _ ctx: UnsafeMutableRawPointer?
) {
    guard let ctx else { return }
    let context = Unmanaged<StreamContext>.fromOpaque(ctx).takeUnretainedValue()
    if let jsonPtr {
        context.onPart(String(cString: jsonPtr))
    }
}

/// C-compatible trampoline: `on_done(void *stream_ctx)`.
private func aimuxStreamOnDone(_ ctx: UnsafeMutableRawPointer?) {
    guard let ctx else { return }
    Unmanaged<StreamContext>.fromOpaque(ctx).takeUnretainedValue().onDone()
}

// ─────────────────────────────────────────────────────────────────────────────
// Convenience: JSON helpers
// ─────────────────────────────────────────────────────────────────────────────

public extension Model {
    /// Generate text with a simple string prompt, returning a parsed result.
    ///
    /// - Parameters:
    ///   - prompt: A plain text prompt.
    ///   - options: Optional options dictionary. Must hold only JSON-legal
    ///   values; anything else throws `DecodingError.dataCorrupted`.
    /// - Returns: Parsed GenerateTextResult as a dictionary.
    func generate(prompt: String, options: [String: Any]? = nil) throws -> [String: Any] {
        // Both force-unwraps are JSON UTF-8 bytes → String, which cannot fail.
        let promptJson = String(data: try JSONEncoder().encode(prompt), encoding: .utf8)!
        let optsJson = try options.map { opts -> String in
            // `data(withJSONObject:)` raises an *uncatchable* ObjC exception
            // (not a Swift error) on a non-JSON value, e.g. a Date; the type
            // is `[String: Any]`, so only this check keeps that unreachable.
            guard JSONSerialization.isValidJSONObject(opts) else {
                throw DecodingError.dataCorrupted(.init(codingPath: [],
                    debugDescription: "options: not a JSON object"))
            }
            return String(data: try JSONSerialization.data(withJSONObject: opts), encoding: .utf8)!
        }
        let resultJson = try generateText(prompt: promptJson, options: optsJson)
        guard let json = try JSONSerialization.jsonObject(with: Data(resultJson.utf8)) as? [String: Any] else {
            throw DecodingError.typeMismatch([String: Any].self,
                .init(codingPath: [], debugDescription: "generate_text result is not a JSON object"))
        }
        return json
    }
}
