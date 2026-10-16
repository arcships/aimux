import CAimuxFFI
import Foundation

public struct StreamLimits: Sendable {
    public let items: Int
    public let bytes: Int
    public let eventBytes: Int
    public static let `default` = StreamLimits(items: 256, bytes: 8 * 1_048_576, eventBytes: 1_048_576)
    private init(items: Int, bytes: Int, eventBytes: Int) {
        self.items = items; self.bytes = bytes; self.eventBytes = eventBytes
    }
    /// Limits may be lowered, never disabled or raised above protocol budgets.
    public init(maxItems: Int, maxBytes: Int, maxEventBytes: Int) throws {
        guard (1...256).contains(maxItems), (1...8 * 1_048_576).contains(maxBytes),
              (1...min(maxBytes, 1_048_576)).contains(maxEventBytes) else {
            throw resourceLimit("Invalid stream limits")
        }
        self.init(items: maxItems, bytes: maxBytes, eventBytes: maxEventBytes)
    }
}

func resourceLimit(_ message: String) -> AimuxError {
    AimuxError(ErrorEnvelope(code: 305, error: .object(["name": .string("ResourceLimit")]),
                            kind: .protocol, message: message))
}

public struct StreamEvent: Equatable, Sendable {
    public let seq: String
    public let event: JSONValue
}

private enum Completion {
    case success
    case failure(any Error)
}

struct BufferedEvent: Sendable {
    let value: StreamEvent
    let bytes: Int
}

/// The callback bridge uses the shared bounded queue for all delivery.
final class StreamChannel: @unchecked Sendable {
    let stream: BoundedStream<BufferedEvent>
    private let lock = NSLock()
    private var releases = 0

    init(limits: StreamLimits, cancellation: Cancellation) {
        stream = BoundedStream(capacity: limits.items, byteCapacity: limits.bytes,
                               maxElementBytes: limits.eventBytes, sizeOf: { $0.bytes },
                               abort: { cancellation.cancel() })
    }
    func send(_ event: StreamEvent, size: Int) {
        stream.producer.yield(BufferedEvent(value: event, bytes: size))
    }
    fileprivate func finish(_ completion: Completion) {
        switch completion {
        case .success: stream.producer.finish()
        case .failure(let error): stream.producer.finish(throwing: error)
        }
    }
    func close() { stream.cancel() }
    func contextReleased() {
        lock.lock(); releases += 1; lock.unlock()
    }
    var snapshot: (items: Int, bytes: Int, highItems: Int, highBytes: Int, releases: Int) {
        let queue = stream.snapshot
        lock.lock(); defer { lock.unlock() }
        return (queue.items, queue.bytes, queue.highItems, queue.highBytes, releases)
    }
}

/// Single-consumer, lossless stream. An iterator owns its lifetime: leaving a
/// for-await loop releases it and aborts even when the sequence is retained.
/// If retaining an iterator yourself, close it explicitly when abandoning it.
public final class AimuxStream: Sendable, AsyncSequence {
    public typealias Element = StreamEvent
    let channel: StreamChannel
    init(_ channel: StreamChannel) { self.channel = channel }
    deinit { channel.close() }
    public func close() { channel.close() }
    public func makeAsyncIterator() -> Iterator {
        Iterator(self)
    }
    public final class Iterator: AsyncIteratorProtocol {
        private let sequence: AimuxStream
        private let iterator: BoundedStream<BufferedEvent>.AsyncIterator
        fileprivate init(_ sequence: AimuxStream) {
            self.sequence = sequence
            iterator = sequence.channel.stream.makeAsyncIterator()
        }
        public func close() { iterator.cancel() }
        public func next() async throws -> StreamEvent? {
            do {
                try Task.checkCancellation()
                var reader = iterator
                let event = try await reader.next()?.value
                try Task.checkCancellation()
                return event
            } catch BoundedStream<BufferedEvent>.Failure.multipleConsumers {
                throw AimuxError.state("Stream already has a consumer", code: 303)
            } catch {
                if Task.isCancelled { close(); throw CancellationError() }
                throw error
            }
        }
    }
}

// The sole continuation is removed before resuming. After acceptance this
// object retains no sequence, so native production cannot keep a consumer alive.
private final class StreamReady: @unchecked Sendable {
    private let lock = NSLock()
    private var continuation: CheckedContinuation<AimuxStream, any Error>?
    private var sequence: AimuxStream?
    init(_ sequence: AimuxStream, _ continuation: CheckedContinuation<AimuxStream, any Error>) {
        self.sequence = sequence; self.continuation = continuation
    }
    func accept() {
        lock.lock()
        let continuation = self.continuation, sequence = self.sequence
        self.continuation = nil; self.sequence = nil
        lock.unlock()
        if let continuation, let sequence { continuation.resume(returning: sequence) }
    }
    func fail(_ error: any Error) {
        lock.lock()
        let continuation = self.continuation
        self.continuation = nil; self.sequence = nil
        lock.unlock()
        continuation?.resume(throwing: error)
    }
}

// C callbacks are serialized by aimux_stream. This state is confined to that
// native calling worker; only the synchronized channel/ready/cancellation cross
// threads. No Sendable assertion is needed for the callback context itself.
private final class StreamBridge {
    let channel: StreamChannel
    let ready: StreamReady
    let cancellation: Cancellation
    let limits: StreamLimits
    private var completion: Completion?
    private var failure: (any Error)?
    private var expectedSeq: UInt64 = 0

    init(_ channel: StreamChannel, _ ready: StreamReady, _ cancellation: Cancellation, _ limits: StreamLimits) {
        self.channel = channel; self.ready = ready; self.cancellation = cancellation; self.limits = limits
    }
    deinit { channel.contextReleased() }

    func receive(_ pointer: UnsafePointer<CChar>?) {
        guard failure == nil else { return }
        do {
            guard completion == nil else { throw invariant("Frame after terminal") }
            guard let pointer else { throw invariant("Null frame") }
            // Inspect at most the per-event budget before copying or decoding.
            let length = strnlen(pointer, limits.eventBytes + 1)
            guard length <= limits.eventBytes else { throw resourceLimit("Event exceeds stream budget") }
            let data = Data(bytes: pointer, count: length)
            let frame = try JSONDecoder().decode(StreamFrame.self, from: data)
            let seq: WireUInt64
            switch frame {
            case .event(_, let value): seq = value
            case .end(_, _, let value): seq = value
            }
            guard seq.rawValue == String(expectedSeq) else { throw invariant("Invalid stream sequence") }
            guard expectedSeq < UInt64.max else { throw invariant("Stream sequence exhausted") }
            expectedSeq += 1
            switch frame {
            case .event(let event, _):
                let value = try JSONDecoder().decode(JSONValue.self, from: JSONEncoder().encode(event))
                ready.accept()
                channel.send(StreamEvent(seq: seq.rawValue, event: value), size: length)
            case .end(let error, let outcome, _):
                switch outcome {
                case .success:
                    guard error == nil else { throw invariant("Success terminal has an error") }
                    completion = .success
                case .error:
                    guard let error else { throw invariant("Missing terminal error") }
                    completion = .failure(AimuxError(error))
                case .cancelled: completion = .failure(CancellationError())
                }
                ready.accept()
            }
        } catch {
            // First-failure latch. Never throw across the C callback boundary.
            failure = (error as? AimuxError) ?? AimuxError.binding(String(describing: error))
            cancellation.abortForCallbackFailure()
        }
    }
    func complete(_ nativeError: (any Error)?) {
        let result: Completion
        if let failure { result = .failure(failure) }
        else if let nativeError { result = .failure(nativeError) }
        else { result = completion ?? .failure(AimuxError.binding("Native stream returned without an end frame")) }
        switch result {
        case .success: ready.accept()
        case .failure(let error): ready.fail(error)
        }
        channel.finish(result)
    }
}

private func streamCallback(_ json: UnsafePointer<CChar>?, _ context: UnsafeMutableRawPointer?) {
    guard let context else { return }
    Unmanaged<StreamBridge>.fromOpaque(context).takeUnretainedValue().receive(json)
}

func startStream(_ state: HandleState, _ op: String, _ params: JSONValue, _ limits: StreamLimits) async throws -> AimuxStream {
    guard !op.utf8.contains(0) else { throw invariant("Operation contains NUL") }
    let json = String(decoding: try JSONEncoder().encode(params), as: UTF8.self)
    let cancellation = Cancellation()
    let (id, ticket) = try state.begin(cancellation)
    let channel = StreamChannel(limits: limits, cancellation: cancellation)
    cancellation.attach(channel)
    let stream = try await withTaskCancellationHandler {
        try await withCheckedThrowingContinuation { continuation in
            let ready = StreamReady(AimuxStream(channel), continuation)
            DispatchQueue.global(qos: .utility).async {
                defer { state.end(ticket); cancellation.finish() }
                let bridge = StreamBridge(channel, ready, cancellation, limits)
                let retained = Unmanaged.passRetained(bridge)
                // Native guarantees no callback after return. This is the only
                // retained-context release on every accepted and rejected path.
                defer { retained.release() }
                do {
                    try verifyRuntime()
                    let abort = try handleID(nativeCall(0, "abort.create", "{}"))
                    cancellation.install(abort)
                    let error = aimux_stream(id, op, json, abort, streamCallback, retained.toOpaque())
                    bridge.complete(error.map(takeError))
                } catch { bridge.complete(error) }
            }
        }
    } onCancel: { channel.close() }
    try Task.checkCancellation()
    return stream
}
