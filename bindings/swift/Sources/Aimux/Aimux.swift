import CAimuxFFI
import Foundation

/// Presents a generated wire envelope as a Swift error.
public struct AimuxError: Error, Codable, Equatable, Sendable, LocalizedError {
    public let envelope: ErrorEnvelope
    public var kind: String { envelope.kind.rawValue }
    public var code: Int32 { envelope.code }
    public var message: String { envelope.message }
    public var error: JSONValue { envelope.error }
    public var retryAfterMs: UInt64? { envelope.retryAfterMs }
    public var errorDescription: String? { message }
    /// The payload's cause: a nested AI SDK error (with `name`) or an
    /// underlying OS, TLS or parser error as `{message, cause?}`. A retry
    /// error's cause is its last attempt, the AI SDK `lastError`.
    public var cause: JSONValue? {
        if let cause = error["cause"] { return cause }
        guard error["name"] == .string("AI_RetryError") else { return nil }
        return error["errors"]?.arrayValue?.last
    }

    public init(_ envelope: ErrorEnvelope) { self.envelope = envelope }
    public init(from decoder: any Decoder) throws { envelope = try ErrorEnvelope(from: decoder) }
    public func encode(to encoder: any Encoder) throws { try envelope.encode(to: encoder) }

    static func binding(_ message: String, code: Int32 = 206) -> AimuxError {
        AimuxError(ErrorEnvelope(code: code, error: .object(["name": .string("CallbackFailure")]),
                                kind: .binding, message: message))
    }
    static func state(_ message: String, code: Int32 = 304) -> AimuxError {
        AimuxError(ErrorEnvelope(code: code,
                                error: .object(["name": .string(code == 303 ? "ConcurrentConsumer" : code == 302 ? "UnsupportedOpsVersion" : "InvalidState")]),
                                kind: .protocol, message: message))
    }
}

func invariant(_ message: String) -> DecodingError {
    .dataCorrupted(.init(codingPath: [], debugDescription: message))
}

func takeString(_ pointer: UnsafeMutablePointer<CChar>?) throws -> String {
    guard let pointer else { throw invariant("Native success returned no JSON") }
    defer { aimux_free_string(pointer) }
    guard let string = String(validatingCString: pointer) else {
        throw invariant("Native result is not UTF-8")
    }
    return string
}

func takeError(_ pointer: OpaquePointer) -> any Error {
    defer { aimux_error_free(pointer) }
    do {
        let json = try takeString(aimux_error_json(pointer))
        let envelope = try JSONDecoder().decode(AimuxError.self, from: Data(json.utf8))
        guard envelope.code == aimux_error_code(pointer) else {
            throw invariant("Error code does not match envelope")
        }
        return envelope
    } catch { return error }
}

enum Blocking {
    // GCD workers may block in native calls or on channel capacity. Swift's
    // cooperative executor and the main actor never perform those waits.
    static func run<T: Sendable>(_ body: @escaping @Sendable () throws -> T) async throws -> T {
        try await withCheckedThrowingContinuation { continuation in
            DispatchQueue.global(qos: .utility).async {
                do { continuation.resume(returning: try body()) }
                catch { continuation.resume(throwing: error) }
            }
        }
    }
    static func drop(_ id: UInt64) {
        guard id != 0 else { return }
        DispatchQueue.global(qos: .utility).async { aimux_drop(id) }
    }
}

private func nativeManifest() throws -> JSONValue {
    let version = aimux_abi_version()
    guard version >> 16 == AimuxOps.ABI_VERSION >> 16 else {
        throw AimuxError.state("Unsupported ABI version \(version)")
    }
    var output: UnsafeMutablePointer<CChar>?
    if let error = aimux_manifest_json(&output) { throw takeError(error) }
    let value = try JSONDecoder().decode(JSONValue.self, from: Data(takeString(output).utf8))
    try validateManifest(value, version: version)
    return value
}

func validateManifest(_ value: JSONValue, version: UInt32) throws {
    guard value["opsVersion"] == .string(AimuxOps.OPS_VERSION),
          value["abiVersion"] == .integer(Int64(version)),
          value["digest"] == .string(AimuxOps.MANIFEST_DIGEST) else {
        throw AimuxError.state("Incompatible manifest: opsVersion \(String(describing: value["opsVersion"])) (expected \(AimuxOps.OPS_VERSION)), digest \(String(describing: value["digest"])) (expected \(AimuxOps.MANIFEST_DIGEST))", code: 302)
    }
}

private enum RuntimeMetadata {
    // Swift's static initialization serializes this immutable compatibility check.
    static let manifest: Result<JSONValue, any Error> = Result { try nativeManifest() }
}

func verifyRuntime() throws {
    guard aimux_abi_version() >> 16 == AimuxOps.ABI_VERSION >> 16 else {
        throw AimuxError.state("Unsupported ABI version")
    }
    _ = try RuntimeMetadata.manifest.get()
}

func nativeCall(_ id: UInt64, _ op: String, _ params: String, _ abort: UInt64 = 0) throws -> String {
    try verifyRuntime()
    guard !op.utf8.contains(0) else { throw invariant("Operation contains NUL") }
    var output: UnsafeMutablePointer<CChar>?
    if let error = aimux_call(id, op, params, abort, &output) { throw takeError(error) }
    return try takeString(output)
}

func handleID(_ json: String) throws -> UInt64 {
    let data = Data(json.utf8)
    let decoder = JSONDecoder()
    let handle: HandleId
    if let scalar = try? decoder.decode(HandleId.self, from: data) { handle = scalar }
    else { handle = try decoder.decode(HandleResult.self, from: data).handle }
    let id = handle.value
    guard id != 0, String(id) == handle.rawValue else {
        throw invariant("Expected a nonzero decimal handle string")
    }
    return id
}

// All mutable fields and abort/drop races are protected by this lock. Abort
// itself is the ABI's nonblocking signal, safe to invoke while holding it.
final class Cancellation: @unchecked Sendable {
    private let lock = NSLock()
    private var id: UInt64 = 0
    private var cancelled = false
    private var finished = false
    private let operation: HandleState?
    private weak var channel: StreamChannel?
    init(operation: HandleState? = nil) { self.operation = operation }

    func attach(_ channel: StreamChannel) {
        lock.lock()
        self.channel = channel
        let stop = cancelled
        lock.unlock()
        if stop { channel.close() }
    }
    func install(_ id: UInt64) {
        lock.lock()
        self.id = id
        if cancelled { aimux_abort(id) }
        lock.unlock()
    }
    func cancel() { request(closeConsumer: true) }
    func abortForCallbackFailure() { request(closeConsumer: false) }
    private func request(closeConsumer: Bool) {
        lock.lock()
        let first = !cancelled && !finished
        if first { cancelled = true; if id != 0 { aimux_abort(id) } }
        let channel = self.channel
        lock.unlock()
        if closeConsumer { channel?.close() }
        if first { operation?.cancelOperation() }
    }
    func finish() {
        lock.lock()
        finished = true
        let value = id
        id = 0
        lock.unlock()
        Blocking.drop(value)
    }
    var isCancelled: Bool {
        lock.lock(); defer { lock.unlock() }
        return cancelled
    }
}

// Admission pins the native registry entry until C returns. Close takes away
// admission atomically, cancels admitted work, and queues drop after the last
// pin, never joining on the caller's actor or under this lock.
final class HandleState: @unchecked Sendable {
    private let lock = NSLock()
    private var id: UInt64
    private var closed = false
    private var users = 0
    private var cancellations: [UUID: Cancellation] = [:]
    init(_ id: UInt64) { self.id = id }

    var identifier: UInt64? {
        lock.lock(); defer { lock.unlock() }
        return closed ? nil : id
    }

    func begin(_ cancellation: Cancellation) throws -> (UInt64, UUID) {
        lock.lock(); defer { lock.unlock() }
        guard !closed else { throw AimuxError.state("Handle is closed") }
        let ticket = UUID()
        cancellations[ticket] = cancellation
        users += 1
        return (id, ticket)
    }
    func end(_ ticket: UUID?) {
        lock.lock()
        if let ticket { cancellations.removeValue(forKey: ticket) }
        users -= 1
        let drop = closed && users == 0 ? id : 0
        if drop != 0 { id = 0 }
        lock.unlock()
        Blocking.drop(drop)
    }
    func close() {
        lock.lock()
        guard !closed else { lock.unlock(); return }
        closed = true
        let active = Array(cancellations.values)
        let drop = users == 0 ? id : 0
        if drop != 0 { id = 0 }
        lock.unlock()
        for cancellation in active { cancellation.cancel() }
        Blocking.drop(drop)
    }
    func cancelOperation() {
        lock.lock()
        guard id != 0 else { lock.unlock(); return }
        let value = id
        users += 1
        lock.unlock()
        DispatchQueue.global(qos: .utility).async {
            _ = try? nativeCall(value, AimuxOps.OP_OPERATION_CANCEL, "{}")
            self.end(nil)
        }
    }
}

public enum PollResult: Equatable, Sendable {
    case part(JSONValue)
    case ended
    case timeout
}

/// Owns one nonzero UInt64 C handle. Close is idempotent and safe to race
/// with admission. Use the runtime's manifest for supported operations/kinds.
public final class AimuxHandle: Sendable {
    let state: HandleState
    public init(owning id: UInt64) throws {
        guard id != 0 else { throw AimuxError.state("Zero is global scope, not an owned handle") }
        state = HandleState(id)
    }
    deinit { state.close() }
    public func close() { state.close() }
    /// Use a decimal string of this ID in secondary handle fields. The ID is
    /// borrowed; it becomes unavailable after close and does not transfer ownership.
    public var id: UInt64? { state.identifier }

    public func call(_ op: String, params: JSONValue = .object([:])) async throws -> JSONValue {
        try await invoke(state, op, params)
    }
    public func stream(_ op: String, params: JSONValue = .object([:]),
                       limits: StreamLimits = .default) async throws -> AimuxStream {
        try await startStream(state, op, params, limits)
    }
    public func open(_ op: String, params: JSONValue = .object([:])) async throws -> AimuxHandle {
        let value = try await call(op, params: params)
        return try checkedOwnedHandle(value)
    }
    public func push(_ data: Data) async throws {
        guard data.count <= 1_048_576 else { throw resourceLimit("Audio chunk exceeds 1 MiB") }
        let cancellation = Cancellation(operation: state)
        let (id, ticket) = try state.begin(cancellation)
        defer { state.end(ticket); cancellation.finish() }
        try await withTaskCancellationHandler {
            do {
                try Task.checkCancellation()
                try await Blocking.run {
                    try verifyRuntime()
                    try data.withUnsafeBytes { bytes in
                        if let error = aimux_operation_push(id, bytes.bindMemory(to: UInt8.self).baseAddress, bytes.count) {
                            throw takeError(error)
                        }
                    }
                }
                try Task.checkCancellation()
                if cancellation.isCancelled { throw CancellationError() }
            } catch {
                if cancellation.isCancelled { throw CancellationError() }
                throw error
            }
        } onCancel: { cancellation.cancel() }
    }
    public func nextPart(timeoutMs: Int64) async throws -> PollResult {
        let cancellation = Cancellation(operation: state)
        let (id, ticket) = try state.begin(cancellation)
        defer { state.end(ticket); cancellation.finish() }
        return try await withTaskCancellationHandler {
            do {
                try Task.checkCancellation()
                let result: PollResult = try await Blocking.run {
                    try verifyRuntime()
                    var output: UnsafeMutablePointer<CChar>?
                    var status: Int32 = 0
                    if let error = aimux_operation_next_part(id, timeoutMs, &output, &status) { throw takeError(error) }
                    switch status {
                    case 1: return .part(try JSONDecoder().decode(JSONValue.self, from: Data(takeString(output).utf8)))
                    case 2, 3:
                        if let output { aimux_free_string(output); throw invariant("Unexpected poll output") }
                        return status == 2 ? .ended : .timeout
                    default:
                        if let output { aimux_free_string(output) }
                        throw invariant("Unknown poll state \(status)")
                    }
                }
                try Task.checkCancellation()
                if cancellation.isCancelled { throw CancellationError() }
                return result
            } catch {
                if cancellation.isCancelled { throw CancellationError() }
                throw error
            }
        } onCancel: { cancellation.cancel() }
    }
}

func ownedHandle(_ value: JSONValue) throws -> AimuxHandle {
    let json = String(decoding: try JSONEncoder().encode(value), as: UTF8.self)
    return try AimuxHandle(owning: handleID(json))
}

func checkedOwnedHandle(_ value: JSONValue) throws -> AimuxHandle {
    let handle = try ownedHandle(value)
    if Task.isCancelled { handle.close(); throw CancellationError() }
    return handle
}

/// Global scope (G). Every blocking primitive runs on a background worker.
public enum AimuxRuntime {
    public static func manifest() async throws -> JSONValue {
        try await Blocking.run { try nativeManifest() }
    }
    public static func call(_ op: String, params: JSONValue = .object([:])) async throws -> JSONValue {
        try await invoke(nil, op, params)
    }
    public static func open(_ op: String, params: JSONValue = .object([:])) async throws -> AimuxHandle {
        try checkedOwnedHandle(await call(op, params: params))
    }
}

func invoke(_ state: HandleState?, _ op: String, _ params: JSONValue) async throws -> JSONValue {
    let json = String(decoding: try JSONEncoder().encode(params), as: UTF8.self)
    let cancellation = Cancellation(operation: op.hasPrefix("operation.") ? state : nil)
    let admission = try state?.begin(cancellation)
    defer { if let admission { state?.end(admission.1) }; cancellation.finish() }
    return try await withTaskCancellationHandler {
        try Task.checkCancellation()
        let result: JSONValue
        do {
            result = try await Blocking.run {
                try verifyRuntime()
                let abort = try handleID(nativeCall(0, AimuxOps.OP_ABORT_CREATE, "{}"))
                cancellation.install(abort)
                let result = try nativeCall(admission?.0 ?? 0, op, json, abort)
                return try JSONDecoder().decode(JSONValue.self, from: Data(result.utf8))
            }
        } catch {
            try Task.checkCancellation()
            throw error
        }
        return result
    } onCancel: { cancellation.cancel() }
}
