import Foundation

/// A single-consumer bridge from synchronous callbacks to async iteration.
///
/// Call `producer.yield` only on a background thread: it blocks while either
/// queue budget is full. `sizeOf` must return the payload's actual byte size.
/// Keep the producer in the callback context until the native call returns.
/// Task cancellation, `cancel()`, or releasing the iterator aborts the producer
/// once. An early `for await` exit releases its iterator even if this sequence
/// is retained. Retaining an iterator explicitly requires explicit cancellation.
public struct BoundedStream<Element: Sendable>: AsyncSequence, Sendable {
    // Swift 6.3 requires this witness; the generic parameter is not enough.
    public typealias Element = Element
    // AsyncSequence.Failure is any Error from finish(throwing:).
    public enum QueueError: Error, Equatable {
        case elementTooLarge
        case invalidElementSize
        case multipleConsumers
    }

    public enum YieldResult: Sendable, Equatable {
        case accepted
        case terminated
    }

    public struct Producer: Sendable {
        fileprivate let state: State

        /// Blocks the callback thread until space is available or the stream
        /// terminates. Oversized payloads finish the stream with a QueueError.
        @discardableResult
        public func yield(_ element: Element) -> YieldResult {
            state.yield(element)
        }

        /// Terminal state has its own slot, so this never waits for capacity.
        /// The first finish wins; its error follows all accepted elements once.
        public func finish(throwing error: (any Error)? = nil) {
            state.finish(error)
        }
    }

    public struct AsyncIterator: AsyncIteratorProtocol {
        fileprivate let lease: Lease

        public func cancel() { if lease.admitted { lease.state.cancel() } }

        public mutating func next() async throws -> Element? {
            // Keep the lease alive throughout suspension, including cancellation.
            defer { withExtendedLifetime(lease) {} }
            guard lease.admitted else { throw QueueError.multipleConsumers }
            return try await withTaskCancellationHandler {
                try await lease.state.next()
            } onCancel: { [state = lease.state] in
                state.cancel()
            }
        }
    }

    private let state: State
    public var producer: Producer { Producer(state: state) }

    public init(
        capacity: Int = 256,
        byteCapacity: Int = 8 * 1024 * 1024,
        maxElementBytes: Int = 1024 * 1024,
        sizeOf: @escaping @Sendable (Element) -> Int,
        abort: @escaping @Sendable () -> Void
    ) {
        precondition((1...256).contains(capacity))
        precondition((1...(8 * 1024 * 1024)).contains(byteCapacity))
        precondition((1...(1024 * 1024)).contains(maxElementBytes))
        state = State(capacity: capacity, byteCapacity: byteCapacity,
                      maxElementBytes: maxElementBytes, sizeOf: sizeOf, abort: abort)
    }

    public func makeAsyncIterator() -> AsyncIterator {
        AsyncIterator(lease: Lease(state: state))
    }

    /// Discards undelivered elements and wakes both sides. Subsequent next is nil.
    public func cancel() { state.cancel() }

    var snapshot: (items: Int, bytes: Int, highItems: Int, highBytes: Int) { state.snapshot }

    fileprivate final class Lease {
        let state: State
        let admitted: Bool

        init(state: State) {
            self.state = state
            admitted = state.admitConsumer()
        }

        deinit {
            if admitted { state.cancel() }
        }
    }

    // Every mutable field is protected by condition. Continuations and abort
    // callbacks run outside that lock, permitting reentrant native termination.
    fileprivate final class State: @unchecked Sendable {
        private struct Slot {
            let element: Element
            let bytes: Int
        }

        private enum Terminal {
            case open
            case finished((any Error)?)
            case cancelled
            case consumed
        }

        private let condition = NSCondition()
        private var slots: [Slot?]
        private var head = 0
        private var count = 0
        private var bytes = 0
        private var highItems = 0
        private var highBytes = 0

        var snapshot: (items: Int, bytes: Int, highItems: Int, highBytes: Int) {
            condition.lock(); defer { condition.unlock() }
            return (count, bytes, highItems, highBytes)
        }
        private var terminal = Terminal.open
        private var consumerAdmitted = false
        private var waiter: CheckedContinuation<Element?, any Error>?
        private let byteCapacity: Int
        private let maxElementBytes: Int
        private let sizeOf: @Sendable (Element) -> Int
        private let abort: @Sendable () -> Void

        init(capacity: Int, byteCapacity: Int, maxElementBytes: Int,
             sizeOf: @escaping @Sendable (Element) -> Int,
             abort: @escaping @Sendable () -> Void) {
            slots = Array(repeating: nil, count: capacity)
            self.byteCapacity = byteCapacity
            self.maxElementBytes = maxElementBytes
            self.sizeOf = sizeOf
            self.abort = abort
        }

        func admitConsumer() -> Bool {
            condition.lock()
            defer { condition.unlock() }
            guard !consumerAdmitted else { return false }
            consumerAdmitted = true
            return true
        }

        func yield(_ element: Element) -> YieldResult {
            let size = sizeOf(element)
            condition.lock()
            while case .open = terminal {
                guard size >= 0, size <= maxElementBytes, size <= byteCapacity else {
                    condition.unlock()
                    finish(size < 0 ? QueueError.invalidElementSize : QueueError.elementTooLarge)
                    return .terminated
                }
                if let waiting = waiter {
                    waiter = nil
                    condition.unlock()
                    waiting.resume(returning: element)
                    return .accepted
                }
                if count < slots.count && size <= byteCapacity - bytes {
                    slots[(head + count) % slots.count] = Slot(element: element, bytes: size)
                    count += 1
                    bytes += size
                    highItems = Swift.max(highItems, count)
                    highBytes = Swift.max(highBytes, bytes)
                    condition.unlock()
                    return .accepted
                }
                condition.wait()
            }
            condition.unlock()
            return .terminated
        }

        func finish(_ error: (any Error)?) {
            condition.lock()
            guard case .open = terminal else {
                condition.unlock()
                return
            }
            terminal = .finished(error)
            let waiting = waiter
            waiter = nil
            if waiting != nil { terminal = .consumed }
            condition.broadcast()
            condition.unlock()
            if let waiting {
                if let error { waiting.resume(throwing: error) }
                else { waiting.resume(returning: nil) }
            }
        }

        func cancel() {
            condition.lock()
            switch terminal {
            case .cancelled, .consumed:
                condition.unlock()
                return
            case .open, .finished:
                break
            }
            // The consumer is abandoning any buffered elements and terminal.
            let previousTerminal = terminal
            let discarded = slots
            terminal = .cancelled
            slots = Array(repeating: nil, count: slots.count)
            count = 0
            bytes = 0
            let waiting = waiter
            waiter = nil
            condition.broadcast()
            condition.unlock()
            waiting?.resume(returning: nil)
            abort()
            // Payload and error destructors may themselves call native code.
            withExtendedLifetime((discarded, previousTerminal)) {}
        }

        func next() async throws -> Element? {
            try await withCheckedThrowingContinuation { continuation in
                condition.lock()
                if count > 0, let slot = slots[head] {
                    slots[head] = nil
                    head = (head + 1) % slots.count
                    count -= 1
                    bytes -= slot.bytes
                    condition.broadcast()
                    condition.unlock()
                    continuation.resume(returning: slot.element)
                    return
                }
                switch terminal {
                case .open:
                    guard waiter == nil else {
                        condition.unlock()
                        continuation.resume(throwing: QueueError.multipleConsumers)
                        return
                    }
                    waiter = continuation
                    condition.unlock()
                case .finished(let error):
                    terminal = .consumed
                    condition.unlock()
                    if let error { continuation.resume(throwing: error) }
                    else { continuation.resume(returning: nil) }
                case .cancelled, .consumed:
                    condition.unlock()
                    continuation.resume(returning: nil)
                }
            }
        }
    }
}
