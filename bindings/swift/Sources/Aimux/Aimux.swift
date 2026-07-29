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

        // Box the context so C can pass it back through callbacks.
        // We use Unmanaged to pass the pointer through the void* context.
        // For simplicity in this PoC, we use global state since aimux_stream_text
        // is synchronous (blocks until done).

        StreamContext.current = context

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

        // Clear the global context after the synchronous call completes.
        StreamContext.current = nil
    }

    /// Stream text as an AsyncSequence of StreamPart JSON strings.
    ///
    /// Usage:
    /// ```swift
    /// for try await part in model.streamTextAsync(prompt: "...") {
    ///     print(part)
    /// }
    /// ```
    public func streamTextAsync(prompt: String, options: String? = nil) -> AsyncStream<String> {
        AsyncStream { continuation in
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
                    continuation.finish()
                }
            )
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Stream context (trampoline for C callbacks)
// ─────────────────────────────────────────────────────────────────────────────

/// Holds the Swift closures for stream callbacks.
/// Uses a static property because C function pointers can't capture state.
/// This is safe because `aimux_stream_text` is synchronous (blocks the caller).
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

    /// Current active context (set during streamText, cleared after).
    /// Safe because aimux_stream_text is synchronous.
    static var current: StreamContext?
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
