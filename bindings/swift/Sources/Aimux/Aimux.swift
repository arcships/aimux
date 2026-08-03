// Aimux.swift — Swift wrapper around the aimux-ffi C ABI.
//
// This is the C ABI path (§3.2). Swift calls the C functions from aimux-ffi
// and wraps them in a Swifty API with ARC-managed handles.

import CAimuxFFI
import Foundation

// ─────────────────────────────────────────────────────────────────────────────
// Errors
// ─────────────────────────────────────────────────────────────────────────────

public enum AimuxError: Error, CustomStringConvertible {
    case invalidHandle
    case invalidPrompt
    case invalidOptions(String)
    case providerError(String)
    case streamError(String)
    case serializationError(String)

    public var description: String {
        switch self {
        case .invalidHandle: return "invalid model handle"
        case .invalidPrompt: return "invalid prompt JSON"
        case .invalidOptions(let msg): return "invalid options: \(msg)"
        case .providerError(let msg): return "provider error: \(msg)"
        case .streamError(let msg): return "stream error: \(msg)"
        case .serializationError(let msg): return "serialization error: \(msg)"
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

    /// Private initializer — use `Model.openai()` etc. to create.
    private init(handle: UInt64) {
        self.handle = handle
    }

    deinit {
        if handle != 0 {
            aimux_drop_handle(handle)
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
        let handle = aimux_openai_new(apiKey, modelId)
        guard handle != 0 else {
            throw AimuxError.invalidHandle
        }
        return Model(handle: handle)
    }

    /// Create an Anthropic model instance.
    public static func anthropic(apiKey: String, modelId: String) throws -> Model {
        let handle = aimux_anthropic_new(apiKey, modelId)
        guard handle != 0 else {
            throw AimuxError.invalidHandle
        }
        return Model(handle: handle)
    }

    /// Create an OpenAI model instance with a custom base URL.
    ///
    /// An empty `baseUrl` falls back to the provider's standard URL
    /// (see `aimux_openai_new_with_base`).
    public static func openai(apiKey: String, modelId: String, baseUrl: String) throws -> Model {
        let handle = aimux_openai_new_with_base(apiKey, modelId, baseUrl)
        guard handle != 0 else {
            throw AimuxError.invalidHandle
        }
        return Model(handle: handle)
    }

    /// Create an Anthropic model instance with a custom base URL.
    public static func anthropic(apiKey: String, modelId: String, baseUrl: String) throws -> Model {
        let handle = aimux_anthropic_new_with_base(apiKey, modelId, baseUrl)
        guard handle != 0 else {
            throw AimuxError.invalidHandle
        }
        return Model(handle: handle)
    }

    /// Create a Cohere model instance.
    public static func cohere(apiKey: String, modelId: String) throws -> Model {
        let handle = aimux_cohere_new(apiKey, modelId)
        guard handle != 0 else {
            throw AimuxError.invalidHandle
        }
        return Model(handle: handle)
    }

    /// Create a Cohere model instance with a custom base URL.
    public static func cohere(apiKey: String, modelId: String, baseUrl: String) throws -> Model {
        let handle = aimux_cohere_new_with_base(apiKey, modelId, baseUrl)
        guard handle != 0 else {
            throw AimuxError.invalidHandle
        }
        return Model(handle: handle)
    }

    /// Create a Mistral model instance.
    public static func mistral(apiKey: String, modelId: String) throws -> Model {
        let handle = aimux_mistral_new(apiKey, modelId)
        guard handle != 0 else {
            throw AimuxError.invalidHandle
        }
        return Model(handle: handle)
    }

    /// Create a Mistral model instance with a custom base URL.
    public static func mistral(apiKey: String, modelId: String, baseUrl: String) throws -> Model {
        let handle = aimux_mistral_new_with_base(apiKey, modelId, baseUrl)
        guard handle != 0 else {
            throw AimuxError.invalidHandle
        }
        return Model(handle: handle)
    }

    /// Create an xAI model instance.
    public static func xai(apiKey: String, modelId: String) throws -> Model {
        let handle = aimux_xai_new(apiKey, modelId)
        guard handle != 0 else {
            throw AimuxError.invalidHandle
        }
        return Model(handle: handle)
    }

    /// Create an xAI model instance with a custom base URL.
    public static func xai(apiKey: String, modelId: String, baseUrl: String) throws -> Model {
        let handle = aimux_xai_new_with_base(apiKey, modelId, baseUrl)
        guard handle != 0 else {
            throw AimuxError.invalidHandle
        }
        return Model(handle: handle)
    }

    /// Create a Bedrock model instance (AWS SigV4 credentials).
    public static func bedrock(
        accessKeyId: String, secretAccessKey: String, region: String, modelId: String
    ) throws -> Model {
        let handle = aimux_bedrock_new(accessKeyId, secretAccessKey, region, modelId)
        guard handle != 0 else {
            throw AimuxError.invalidHandle
        }
        return Model(handle: handle)
    }

    /// Create a Bedrock model instance with a custom base URL.
    public static func bedrock(
        accessKeyId: String, secretAccessKey: String, region: String, modelId: String, baseUrl: String
    ) throws -> Model {
        let handle = aimux_bedrock_new_with_base(accessKeyId, secretAccessKey, region, modelId, baseUrl)
        guard handle != 0 else {
            throw AimuxError.invalidHandle
        }
        return Model(handle: handle)
    }

    /// Create a Vertex AI model instance (GCP bearer token).
    public static func vertex(
        accessToken: String, project: String, location: String, modelId: String
    ) throws -> Model {
        let handle = aimux_vertex_new(accessToken, project, location, modelId)
        guard handle != 0 else {
            throw AimuxError.invalidHandle
        }
        return Model(handle: handle)
    }

    /// Create a Vertex AI model instance with a custom base URL.
    public static func vertex(
        accessToken: String, project: String, location: String, modelId: String, baseUrl: String
    ) throws -> Model {
        let handle = aimux_vertex_new_with_base(accessToken, project, location, modelId, baseUrl)
        guard handle != 0 else {
            throw AimuxError.invalidHandle
        }
        return Model(handle: handle)
    }

    /// Create an Anthropic-on-AWS model instance (API key + region).
    public static func anthropicAws(apiKey: String, region: String, modelId: String) throws -> Model {
        let handle = aimux_anthropic_aws_new(apiKey, region, modelId)
        guard handle != 0 else {
            throw AimuxError.invalidHandle
        }
        return Model(handle: handle)
    }

    /// Create an Anthropic-on-AWS model instance with a custom base URL.
    public static func anthropicAws(
        apiKey: String, region: String, modelId: String, baseUrl: String
    ) throws -> Model {
        let handle = aimux_anthropic_aws_new_with_base(apiKey, region, modelId, baseUrl)
        guard handle != 0 else {
            throw AimuxError.invalidHandle
        }
        return Model(handle: handle)
    }

    /// Create an Azure OpenAI model instance (API key + resource name).
    /// The deployment name is passed as `modelId`; `apiVersion` is optional.
    public static func azure(
        apiKey: String, resourceName: String, deployment: String, apiVersion: String? = nil
    ) throws -> Model {
        let handle = aimux_azure_new(apiKey, resourceName, deployment, apiVersion)
        guard handle != 0 else {
            throw AimuxError.invalidHandle
        }
        return Model(handle: handle)
    }

    /// Create an Azure OpenAI model instance with a custom base URL.
    public static func azureWithBase(
        apiKey: String, baseUrl: String, deployment: String, apiVersion: String? = nil
    ) throws -> Model {
        let handle = aimux_azure_new_with_base(apiKey, baseUrl, deployment, apiVersion)
        guard handle != 0 else {
            throw AimuxError.invalidHandle
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
    public static func provider(name: String, apiKey: String? = nil, modelId: String, configJson: String? = nil) throws -> Model {
        let handle = aimux_provider_new(name, apiKey, modelId, configJson)
        guard handle != 0 else {
            throw AimuxError.invalidHandle
        }
        return Model(handle: handle)
    }

    // ── Generation ─────────────────────────────────────────────────────────

    /// Generate text (non-streaming).
    ///
    /// - Parameters:
    ///   - prompt: A prompt string or messages array (serialized as JSON).
    ///   - options: Optional GenerateTextOptions (serialized as JSON).
    /// - Returns: The JSON-serialized GenerateTextResult.
    public func generateText(prompt: String, options: String? = nil) throws -> String {
        let resultPtr = aimux_generate_text(
            handle,
            prompt,
            options
        )

        guard let ptr = resultPtr else {
            throw AimuxError.serializationError("generate_text returned null")
        }

        let result = String(cString: ptr)
        aimux_free_string(ptr)

        // Check for error in the result JSON
        if let data = result.data(using: .utf8),
           let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
           let error = json["error"] as? String {
            throw AimuxError.providerError(error)
        }

        return result
    }

    /// Stream text from the model.
    ///
    /// - Parameters:
    ///   - prompt: A prompt string (serialized as JSON).
    ///   - options: Optional GenerateTextOptions (serialized as JSON).
    ///   - onPart: Called for each StreamPart (JSON string).
    ///   - onDone: Called when the stream completes normally.
    ///   - onError: Called on a stream error.
    public func streamText(
        prompt: String,
        options: String? = nil,
        onPart: @escaping (String) -> Void,
        onDone: @escaping () -> Void,
        onError: @escaping (String) -> Void
    ) {
        // The C callbacks are simple function pointers. We use a trampoline
        // through a context object because C function pointers can't capture
        // Swift closures.

        let context = StreamContext(
            onPart: onPart,
            onDone: onDone,
            onError: onError
        )

        // The C ABI has no user-data parameter, so keep the context in the
        // invoking thread's dictionary. aimux_stream_text is synchronous and
        // invokes callbacks on that same thread, while concurrent streams on
        // other threads receive independent contexts.
        let previousContext = StreamContext.current
        StreamContext.current = context
        defer { StreamContext.current = previousContext }

        aimux_stream_text(
            handle,
            prompt,
            options,
            { jsonPtr in
                if let ptr = jsonPtr {
                    let json = String(cString: ptr)
                    StreamContext.current?.onPart(json)
                }
            },
            {
                StreamContext.current?.onDone()
            },
            { errPtr in
                if let ptr = errPtr {
                    let err = String(cString: ptr)
                    StreamContext.current?.onError(err)
                }
            }
        )
    }

    /// Stream text as an AsyncSequence of StreamPart JSON strings.
    ///
    /// Usage:
    /// ```swift
    /// for try await part in model.streamTextAsync(prompt: "...") {
    ///     print(part)
    /// }
    /// ```
    public func streamTextAsync(prompt: String, options: String? = nil) -> AsyncThrowingStream<String, Error> {
        AsyncThrowingStream { continuation in
            self.streamText(
                prompt: prompt,
                options: options,
                onPart: { part in
                    continuation.yield(part)
                },
                onDone: {
                    continuation.finish()
                },
                onError: { err in
                    continuation.finish(throwing: AimuxError.streamError(err))
                }
            )
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Stream context (trampoline for C callbacks)
// ─────────────────────────────────────────────────────────────────────────────

/// Holds the Swift closures for stream callbacks.
/// The current context is stored per thread because the C ABI cannot pass
/// user-data through its callback function pointers.
private final class StreamContext {
    let onPart: (String) -> Void
    let onDone: () -> Void
    let onError: (String) -> Void

    init(onPart: @escaping (String) -> Void,
         onDone: @escaping () -> Void,
         onError: @escaping (String) -> Void) {
        self.onPart = onPart
        self.onDone = onDone
        self.onError = onError
    }

    private static let threadKey = "org.aimux.swift.stream-context"

    /// Current active context for the invoking thread.
    static var current: StreamContext? {
        get { Thread.current.threadDictionary[threadKey] as? StreamContext }
        set {
            if let newValue {
                Thread.current.threadDictionary[threadKey] = newValue
            } else {
                Thread.current.threadDictionary.removeObject(forKey: threadKey)
            }
        }
    }
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
