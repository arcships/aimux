// Aimux.swift — Swift wrapper around the aimux-ffi C ABI.
//
// This is the C ABI path (§3.2). Swift calls the C functions from aimux-ffi
// and wraps them in a Swifty API with ARC-managed handles.

import CAimuxFFI
import Foundation

// ─────────────────────────────────────────────────────────────────────────────
// C AimuxError (aimux-error.h) — distinct from the Swift enum below.
// ─────────────────────────────────────────────────────────────────────────────

/// C `struct AimuxError` imported from `CAimuxFFI` / aimux-error.h.
///
/// Kept as an internal alias so the public Swift type can remain `AimuxError`.
typealias CAimuxError = CAimuxFFI.AimuxError

/// Run a fallible FFI call with a cleared, stack-allocated C error.
///
/// `body` performs the C call and returns `nil` when the C return sentinel
/// indicates failure (NULL result / zero handle / zero rc); this helper then
/// maps the filled `*err` via `AimuxError.fromC` (which frees the C-allocated
/// `message` and `error_value`) and throws it.
func withCError<T>(
    _ body: (UnsafeMutablePointer<CAimuxError>?) -> T?
) throws -> T {
    precondition(MemoryLayout<CAimuxError>.size == 40,
                 "CAimuxError layout mismatch with aimux-error.h")
    var e = CAimuxError()
    aimux_error_clear(&e)
    guard let result = body(&e) else {
        throw AimuxError.fromC(e)
    }
    return result
}

/// `withCError` for calls returning an owned `char*` (JSON) result: copies it
/// into a Swift `String` and frees the C allocation, or throws on `NULL`.
func ffiStringCall(
    _ body: (UnsafeMutablePointer<CAimuxError>?) -> UnsafeMutablePointer<CChar>?
) throws -> String {
    try withCError { err in
        guard let ptr = body(err) else { return nil }
        defer { aimux_free_string(ptr) }
        return String(cString: ptr)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Errors
// ─────────────────────────────────────────────────────────────────────────────

/// Structured aimux failure type (Swift `Error`).
///
/// Maps 1:1 from the 13 core `AiMuxError` variants, plus binding-local cases
/// (`invalidHandle`, `serializationError`). Every HTTP-shaped failure is
/// `.apiCall` (`AIMUX_E_API_CALL`).
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
    /// Local: model/provider handle is 0 or already released.
    case invalidHandle

    case jsonParse(message: String, status: Int, retryMs: Int64, retryable: Bool, errorValue: String?)
    case invalidResponseData(message: String, status: Int, retryMs: Int64, retryable: Bool, errorValue: String?)
    case tool(message: String, status: Int, retryMs: Int64, retryable: Bool, errorValue: String?)
    case invalidArgument(message: String, status: Int, retryMs: Int64, retryable: Bool, errorValue: String?)
    case invalidPrompt(message: String, status: Int, retryMs: Int64, retryable: Bool, errorValue: String?)
    case tokenExpired(message: String, status: Int, retryMs: Int64, retryable: Bool, errorValue: String?)
    case unsupportedFunctionality(message: String, status: Int, retryMs: Int64, retryable: Bool, errorValue: String?)
    case noSuchModel(message: String, status: Int, retryMs: Int64, retryable: Bool, errorValue: String?)
    case noSuchProvider(message: String, status: Int, retryMs: Int64, retryable: Bool, errorValue: String?)
    /// Every HTTP-shaped failure: read `status` to classify (401 auth,
    /// 404 model, 429 rate limit). A `nil` status means no HTTP response was
    /// ever observed — a missing API key, an error built without a request, or
    /// a transport failure; read `retryable` to tell those apart, `status`
    /// cannot.
    case apiCall(message: String, status: Int, retryMs: Int64, retryable: Bool, errorValue: String?)
    case timeout(message: String, status: Int, retryMs: Int64, retryable: Bool, errorValue: String?)
    case aborted(message: String, status: Int, retryMs: Int64, retryable: Bool, errorValue: String?)
    case other(message: String, status: Int, retryMs: Int64, retryable: Bool, errorValue: String?)
    /// `AIMUX_E_UNKNOWN` or an unexpected/future code.
    case unknown(message: String, status: Int, retryMs: Int64, retryable: Bool, errorValue: String?)

    /// Binding-local encode/decode failure (not produced by the C ABI).
    case serializationError(String)

    // MARK: Accessors

    /// The C-derived payload, or `nil` for the binding-local cases.
    private var payload: (message: String, status: Int, retryMs: Int64, retryable: Bool, errorValue: String?)? {
        switch self {
        case .invalidHandle, .serializationError:
            return nil
        case .jsonParse(let m, let s, let r, let t, let v),
             .invalidResponseData(let m, let s, let r, let t, let v),
             .tool(let m, let s, let r, let t, let v),
             .invalidArgument(let m, let s, let r, let t, let v),
             .invalidPrompt(let m, let s, let r, let t, let v),
             .tokenExpired(let m, let s, let r, let t, let v),
             .unsupportedFunctionality(let m, let s, let r, let t, let v),
             .noSuchModel(let m, let s, let r, let t, let v),
             .noSuchProvider(let m, let s, let r, let t, let v),
             .apiCall(let m, let s, let r, let t, let v),
             .timeout(let m, let s, let r, let t, let v),
             .aborted(let m, let s, let r, let t, let v),
             .other(let m, let s, let r, let t, let v),
             .unknown(let m, let s, let r, let t, let v):
            return (m, s, r, t, v)
        }
    }

    /// Human-readable message (C `message` field or local description).
    public var message: String {
        if let payload { return payload.message }
        if case .serializationError(let msg) = self { return msg }
        return "invalid model handle"
    }

    /// HTTP status code, or `nil` when not applicable (C reports `-1`).
    public var status: Int? {
        guard let payload, payload.status >= 0 else { return nil }
        return payload.status
    }

    /// Rate-limit retry hint in milliseconds, or `nil` when not applicable
    /// (C reports `-1`). `0` means retry immediately.
    public var retryMs: Int64? {
        guard let payload, payload.retryMs >= 0 else { return nil }
        return payload.retryMs
    }

    /// Whether retrying may help — the engine's verdict, carried across the C
    /// ABI. Not derivable from `status`: a transport failure (request went
    /// out, connection reset) and a missing API key (request never went out)
    /// both report no status and disagree here. `false` for the
    /// binding-local cases.
    public var retryable: Bool {
        payload?.retryable ?? false
    }

    /// Raw lossless machine-readable form of the source error: the
    /// externally-tagged JSON of aimux-core's `AiMuxError`, e.g.
    /// `{"ApiCall":{"status_code":429,"retry_after_ms":1500,...}}`.
    /// `nil` for failures synthesized at the FFI boundary (bad argument,
    /// invalid handle) and for the binding-local cases.
    public var errorValue: String? {
        payload?.errorValue
    }

    public var description: String {
        message
    }

    public var errorDescription: String? {
        message
    }

    // MARK: C mapping

    /// Map a filled C `AimuxError` (by value) into the Swift enum.
    ///
    /// Call only after the FFI return sentinel indicates failure. When `code`
    /// is `AIMUX_OK` (caller passed `NULL` err or forgot to check), returns
    /// `.unknown` with a generic message.
    ///
    /// Consumes the C-allocated `message` and `error_value`: each is copied
    /// into a Swift string and freed with `aimux_free_string`. Do not reuse
    /// `e.message` / `e.error_value` after.
    public static func fromC(_ e: CAimuxFFI.AimuxError) -> AimuxError {
        var rawMsg = ""
        if let msgPtr = e.message {
            rawMsg = String(cString: msgPtr)
            aimux_free_string(msgPtr)
        }
        var errorValue: String?
        if let valPtr = e.error_value {
            errorValue = String(cString: valPtr)
            aimux_free_string(valPtr)
        }
        let status = Int(e.status)
        let retryMs = e.retry_ms
        let retryable = e.retryable != 0
        let message = rawMsg.isEmpty ? "aimux: operation failed" : rawMsg

        switch e.code {
        case AIMUX_E_JSON_PARSE:
            return .jsonParse(message: message, status: status, retryMs: retryMs, retryable: retryable, errorValue: errorValue)
        case AIMUX_E_INVALID_RESPONSE_DATA:
            return .invalidResponseData(message: message, status: status, retryMs: retryMs, retryable: retryable, errorValue: errorValue)
        case AIMUX_E_TOOL:
            return .tool(message: message, status: status, retryMs: retryMs, retryable: retryable, errorValue: errorValue)
        case AIMUX_E_INVALID_ARGUMENT:
            return .invalidArgument(message: message, status: status, retryMs: retryMs, retryable: retryable, errorValue: errorValue)
        case AIMUX_E_INVALID_PROMPT:
            return .invalidPrompt(message: message, status: status, retryMs: retryMs, retryable: retryable, errorValue: errorValue)
        case AIMUX_E_TOKEN_EXPIRED:
            // TokenExpired is definitionally an observed 401 (RFC-0018).
            return .tokenExpired(message: message, status: status == -1 ? 401 : status, retryMs: retryMs, retryable: retryable, errorValue: errorValue)
        case AIMUX_E_UNSUPPORTED_FUNCTIONALITY:
            return .unsupportedFunctionality(message: message, status: status, retryMs: retryMs, retryable: retryable, errorValue: errorValue)
        case AIMUX_E_NO_SUCH_MODEL:
            return .noSuchModel(message: message, status: status, retryMs: retryMs, retryable: retryable, errorValue: errorValue)
        case AIMUX_E_NO_SUCH_PROVIDER:
            return .noSuchProvider(message: message, status: status, retryMs: retryMs, retryable: retryable, errorValue: errorValue)
        case AIMUX_E_API_CALL:
            return .apiCall(message: message, status: status, retryMs: retryMs, retryable: retryable, errorValue: errorValue)
        case AIMUX_E_TIMEOUT:
            return .timeout(message: message, status: status, retryMs: retryMs, retryable: retryable, errorValue: errorValue)
        case AIMUX_E_ABORTED:
            return .aborted(message: message, status: status, retryMs: retryMs, retryable: retryable, errorValue: errorValue)
        case AIMUX_E_OTHER:
            return .other(message: message, status: status, retryMs: retryMs, retryable: retryable, errorValue: errorValue)
        default:
            return .unknown(message: message, status: status, retryMs: retryMs, retryable: retryable, errorValue: errorValue)
        }
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

    /// Run a C constructor that returns a `uint64_t` handle and fills
    /// `AimuxError *err` on failure.
    static func wrapHandle(
        _ call: (UnsafeMutablePointer<CAimuxError>?) -> UInt64
    ) throws -> UInt64 {
        try withCError { err in
            let h = call(err)
            return h == 0 ? nil : h
        }
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
        aimux_init_logging(level.isEmpty ? "warn" : level)
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
    public static func initRecording(dir: String) {
        aimux_init_recording(dir)
    }

    /// Start in-memory bounded recording (ring recorder with FIFO eviction).
    ///
    /// Keeps only the most recent `cap` recordings in memory, evicting the
    /// oldest first. Use this when recordings don't need to be persisted to
    /// disk.
    ///
    /// - Parameter cap: Ring capacity. Pass `nil` (the default) to use the
    ///   library default capacity (FFI `aimux_init_recording_ring_default`).
    ///   An explicit cap must be > 0 — the C ABI rejects `0` (returns -1).
    /// - Throws: `AimuxError.invalidArgument` when the C call fails (an explicit
    ///   cap == 0 returns -1). This matches Kotlin/Java (throw) and Flutter
    ///   (surface the C error); the binding no longer silently ignores the
    ///   return code.
    public static func initRecordingRing(cap: UInt64? = nil) throws {
        let rc: Int32
        if let c = cap {
            rc = aimux_init_recording_ring(c)
        } else {
            rc = aimux_init_recording_ring_default()
        }
        if rc < 0 {
            throw AimuxError.invalidArgument(
                message: "aimux: initRecordingRing requires cap > 0 (got \(cap ?? 0))",
                status: -1,
                retryMs: -1,
                retryable: false,
                errorValue: nil
            )
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

    /// Create a mock replay model from recorded JSONL (one `Recording` per
    /// line).
    ///
    /// The returned model answers `generateText` / `streamText` from the
    /// recorded data without sending any real provider request.
    ///
    /// - Parameter recordingsJsonl: The recorded JSONL content.
    /// - Returns: A `Model` backed by the mock replay handle.
    public static func mockReplay(recordingsJsonl: String) throws -> Model {
        let handle = try wrapHandle { aimux_mock_replay_new(recordingsJsonl, $0) }
        return Model(handle: handle)
    }

    // ── Composite models (RFC-0021 / RFC-0022) ──────────────────────────────

    /// Create a RouterModel (RFC-0021) over the given child models. The
    /// returned model routes each call to one child and falls back across the
    /// rest on error (per `configJson`).
    ///
    /// - Parameters:
    ///   - models: child models (must be non-empty).
    ///   - configJson: optional config: `{"router": "rule"|"weighted",
    ///     "weights": [...], "fallback": "on_error"|"none", "provider_name",
    ///     "model_id"}`.
    /// - Returns: a new RouterModel wrapping the children.
    public static func router(
        _ models: [Model],
        configJson: String? = nil
    ) throws -> Model {
        guard !models.isEmpty else {
            throw AimuxError.invalidArgument(
                message: "router: models must be non-empty",
                status: -1,
                retryMs: -1,
                errorValue: nil
            )
        }
        let handles = models.map { $0.handle }
        let handle = try handles.withUnsafeBufferPointer { buf -> UInt64 in
            try wrapHandle { err in
                aimux_router_new(buf.baseAddress, buf.count, configJson, err)
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
        let agg = aggregator.handle
        // Empty references: pass a NULL base address + 0 length.
        let handle: UInt64
        if references.isEmpty {
            handle = try wrapHandle { err in
                aimux_moa_new(nil, 0, agg, configJson, err)
            }
        } else {
            let refHandles = references.map { $0.handle }
            handle = try refHandles.withUnsafeBufferPointer { buf -> UInt64 in
                try wrapHandle { err in
                    aimux_moa_new(buf.baseAddress, buf.count, agg, configJson, err)
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
    /// Unlike `mockReplay`/`wrapHandle`, the C entry point returns an `int`
    /// success code (1 = ok, 0 = failure) rather than a `uint64_t` handle, so
    /// the err check is inlined here instead of routing through `wrapHandle`.
    ///
    /// - Parameter configJSON: Provider registry config JSON.
    /// - Throws: `AimuxError` when the C call fails (rc == 0).
    public static func registerProviders(_ configJSON: String) throws {
        var err = CAimuxError()
        aimux_error_clear(&err)
        let rc = aimux_register_providers(configJSON, &err)
        if rc == 0 { throw AimuxError.fromC(err) }
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
    /// - Throws: `AimuxError` when the C call fails (rc == 0).
    public static func initProxy(_ configJSON: String) throws {
        var err = CAimuxError()
        aimux_error_clear(&err)
        let rc = aimux_init_proxy(configJSON, &err)
        if rc == 0 { throw AimuxError.fromC(err) }
    }

    // ── Provider handles (RFC-0027) ──────────────────────────────────────────

    /// Create a **provider handle** for a registry-backed provider (RFC-0027).
    ///
    /// Unlike `provider()` (which binds to a single modelId), this returns a
    /// `ProviderHandle` that supports `listModels()` and `model()`.
    public static func createProvider(
        name: String, apiKey: String? = nil, configJson: String? = nil
    ) throws -> ProviderHandle {
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
        try ffiStringCall { aimux_generate_text(handle, prompt, options, $0) }
    }

    /// Generate a structured JSON object (M12, RFC-0016).
    ///
    /// Same signature as `generateText`; returns a JSON-serialized
    /// `GenerateObjectResult`. Pass `response_format: { "Json": { ... } }`
    /// via `options` for schema control; the engine applies JSON repair
    /// before parsing.
    ///
    /// - Parameters:
    ///   - prompt: A prompt string or messages array (serialized as JSON).
    ///   - options: Optional GenerateTextOptions (serialized as JSON).
    /// - Returns: The JSON-serialized GenerateObjectResult.
    public func generateObject(prompt: String, options: String? = nil) throws -> String {
        try ffiStringCall { aimux_generate_object(handle, prompt, options, $0) }
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
        try ffiStringCall { aimux_consume_stream_text(handle, prompt, options, $0) }
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
        try ffiStringCall { aimux_generate_text_as_openai(handle, prompt, options, $0) }
    }

    /// Stream text from the model.
    ///
    /// The C ABI returns `int32` success (non-zero) / failure (0) and fills
    /// `AimuxError *err` on failure — there is no C `onError` callback.
    /// Failures are surfaced via `onError` with a structured `AimuxError`.
    ///
    /// - Parameters:
    ///   - prompt: A prompt string (serialized as JSON).
    ///   - options: Optional GenerateTextOptions (serialized as JSON).
    ///   - onPart: Called for each StreamPart (JSON string).
    ///   - onDone: Called when the stream completes normally.
    ///   - onError: Called on stream failure (`AimuxError.fromC`).
    public func streamText(
        prompt: String,
        options: String? = nil,
        onPart: @escaping (String) -> Void,
        onDone: @escaping () -> Void,
        onError: @escaping (AimuxError) -> Void
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
        onError: @escaping (AimuxError) -> Void
    ) {
        stream(aimux_stream_text_as_openai, prompt: prompt, options: options,
               onPart: onPart, onDone: onDone, onError: onError)
    }

    /// C `aimux_stream_text` / `aimux_stream_text_as_openai` signature.
    private typealias StreamFn = (
        UInt64, UnsafePointer<CChar>?, UnsafePointer<CChar>?,
        (@convention(c) (UnsafePointer<CChar>?, UnsafeMutableRawPointer?) -> Void)?,
        (@convention(c) (UnsafeMutableRawPointer?) -> Void)?,
        UnsafeMutableRawPointer?, UnsafeMutablePointer<CAimuxError>?
    ) -> Int32

    /// Shared body of the two closure-based stream methods (they differ only
    /// by C symbol).
    private func stream(
        _ fn: StreamFn,
        prompt: String,
        options: String?,
        onPart: @escaping (String) -> Void,
        onDone: @escaping () -> Void,
        onError: @escaping (AimuxError) -> Void
    ) {
        let context = StreamContext(onPart: onPart, onDone: onDone)
        let unmanaged = Unmanaged.passRetained(context)
        defer { unmanaged.release() }

        do {
            _ = try withCError { err -> Int32? in
                let rc = fn(handle, prompt, options,
                            aimuxStreamOnPart, aimuxStreamOnDone,
                            unmanaged.toOpaque(), err)
                return rc == 0 ? nil : rc
            }
        } catch let e as AimuxError {
            onError(e)
        } catch {
            // withCError only throws AimuxError.
            onError(.unknown(message: "\(error)", status: -1, retryMs: -1, retryable: false,
                             errorValue: nil))
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
        guard handle != 0 else { throw AimuxError.invalidHandle }
        return try ffiStringCall { aimux_provider_list_models(handle, $0) }
    }

    /// Build a language model from a discovered model id.
    public func model(_ modelId: String) throws -> Model {
        guard handle != 0 else { throw AimuxError.invalidHandle }
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
    ///   - options: Optional options dictionary.
    /// - Returns: Parsed GenerateTextResult as a dictionary.
    func generate(prompt: String, options: [String: Any]? = nil) throws -> [String: Any] {
        let promptJson = "\"\(prompt)\""
        let optsJson = options.flatMap { try? JSONSerialization.data(withJSONObject: $0) }
            .flatMap { String(data: $0, encoding: .utf8) }

        let resultJson = try generateText(prompt: promptJson, options: optsJson)

        guard let data = resultJson.data(using: .utf8),
              let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw AimuxError.serializationError("failed to parse result")
        }

        return json
    }
}
