import CAimuxFFI
import Foundation

/// A DecisionModelV4 model using the core JSON contract: DecideOptions in, DecideResult out.
public final class DecisionModel: @unchecked Sendable {
    private let lock = NSLock()
    private var handle: UInt64

    init(handle: UInt64) { self.handle = handle }
    deinit { close() }

    /// Endpoint is a complete POST URL; nil uses the official API.
    public static func jev(apiKey: String, modelId: String, endpoint: String? = nil,
                           probabilitySource: String? = nil) throws -> DecisionModel {
        let handle = try Model.wrapHandle {
            aimux_jev_decision_new_with_probability_source(apiKey, modelId, endpoint, probabilitySource, $0)
        }
        return DecisionModel(handle: handle)
    }

    /// Optional abort handle uses the existing C ABI abort signal lifecycle.
    public func decide(options: String, abortHandle: UInt64 = 0) throws -> String {
        try validateJson(options, parameter: "options")
        let handle = try snapshot()
        return try ffiStringCall { aimux_decide_with_abort(handle, options, abortHandle, $0) }
    }

    /// The model's DecisionModelV4 properties as JSON
    /// (`{specificationVersion, provider, modelId, supportedQuestionTypes}`), without HTTP.
    public func describe() throws -> String {
        let handle = try snapshot()
        return try ffiStringCall { aimux_decision_capabilities(handle, $0) }
    }

    private func snapshot() throws -> UInt64 {
        lock.lock()
        defer { lock.unlock() }
        guard handle != 0 else { throw invariant("DecisionModel is closed") }
        return handle
    }

    /// Idempotent; a request already inside Rust may finish after close.
    public func close() {
        lock.lock()
        let value = handle
        handle = 0
        lock.unlock()
        if value != 0 { aimux_drop_handle(value) }
    }
}
