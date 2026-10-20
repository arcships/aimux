import Foundation
import XCTest
@testable import Aimux

final class BoundedStreamTests: XCTestCase {
    private enum TestError: Error, Equatable { case failed }

    private final class Counter: @unchecked Sendable {
        private let lock = NSLock()
        private var value = 0
        func increment() { lock.lock(); value += 1; lock.unlock() }
        var count: Int { lock.lock(); defer { lock.unlock() }; return value }
    }

    private func stream(capacity: Int = 1, abort: @escaping @Sendable () -> Void = {}) -> BoundedStream<Int> {
        BoundedStream(capacity: capacity, sizeOf: { _ in 1 }, abort: abort)
    }

    func testFIFOIsLosslessAcrossManyWraps() async throws {
        let aborts = Counter()
        let stream = stream(capacity: 3, abort: { aborts.increment() })
        let producer = stream.producer
        let done = expectation(description: "producer finished")
        DispatchQueue.global().async {
            for value in 0..<2000 {
                XCTAssertEqual(producer.yield(value), .accepted)
            }
            producer.finish()
            done.fulfill()
        }
        var values: [Int] = []
        for try await value in stream { values.append(value) }
        await fulfillment(of: [done], timeout: 5)
        XCTAssertEqual(values, Array(0..<2000))
        XCTAssertEqual(aborts.count, 0)
    }

    func testFullQueueBlocksUntilConsumerMakesSpace() async throws {
        let stream = stream()
        let producer = stream.producer
        XCTAssertEqual(producer.yield(1), .accepted)
        let entered = DispatchSemaphore(value: 0)
        let returned = DispatchSemaphore(value: 0)
        DispatchQueue.global().async {
            entered.signal()
            XCTAssertEqual(producer.yield(2), .accepted)
            returned.signal()
        }
        XCTAssertEqual(entered.wait(timeout: .now() + 2), .success)
        XCTAssertEqual(returned.wait(timeout: .now() + 0.05), .timedOut)
        var iterator = stream.makeAsyncIterator()
        let first = try await iterator.next()
        XCTAssertEqual(first, 1)
        XCTAssertEqual(returned.wait(timeout: .now() + 2), .success)
        producer.finish()
        let second = try await iterator.next()
        let end = try await iterator.next()
        XCTAssertEqual(second, 2)
        XCTAssertNil(end)
    }

    func testTerminalErrorFollowsBufferOnceAndFirstFinishWins() async throws {
        let aborts = Counter()
        let stream = stream(capacity: 2, abort: { aborts.increment() })
        let producer = stream.producer
        producer.yield(1)
        producer.yield(2)
        producer.finish(throwing: TestError.failed)
        producer.finish()
        XCTAssertEqual(producer.yield(3), .terminated)
        var iterator = stream.makeAsyncIterator()
        let first = try await iterator.next()
        let second = try await iterator.next()
        XCTAssertEqual(first, 1)
        XCTAssertEqual(second, 2)
        do {
            _ = try await iterator.next()
            XCTFail("missing terminal failure")
        } catch { XCTAssertEqual(error as? TestError, .failed) }
        let end = try await iterator.next()
        XCTAssertNil(end)
        stream.cancel()
        XCTAssertEqual(aborts.count, 0)
    }

    func testFinishWakesBlockedProducerWithoutDroppingAcceptedElement() async throws {
        let stream = stream()
        let producer = stream.producer
        producer.yield(1)
        let entered = DispatchSemaphore(value: 0)
        let returned = DispatchSemaphore(value: 0)
        DispatchQueue.global().async {
            entered.signal()
            XCTAssertEqual(producer.yield(2), .terminated)
            returned.signal()
        }
        XCTAssertEqual(entered.wait(timeout: .now() + 2), .success)
        XCTAssertEqual(returned.wait(timeout: .now() + 0.05), .timedOut)
        producer.finish(throwing: TestError.failed)
        XCTAssertEqual(returned.wait(timeout: .now() + 2), .success)
        var iterator = stream.makeAsyncIterator()
        let first = try await iterator.next()
        XCTAssertEqual(first, 1)
        do { _ = try await iterator.next(); XCTFail("missing error") }
        catch { XCTAssertEqual(error as? TestError, .failed) }
    }

    func testCancelWithFullQueueReleasesProducerAndAbortsOnce() async throws {
        let aborts = Counter()
        let stream = stream(abort: { aborts.increment() })
        let producer = stream.producer
        producer.yield(1)
        let entered = DispatchSemaphore(value: 0)
        let returned = DispatchSemaphore(value: 0)
        DispatchQueue.global().async {
            entered.signal()
            XCTAssertEqual(producer.yield(2), .terminated)
            returned.signal()
        }
        XCTAssertEqual(entered.wait(timeout: .now() + 2), .success)
        XCTAssertEqual(returned.wait(timeout: .now() + 0.05), .timedOut)
        DispatchQueue.concurrentPerform(iterations: 20) { _ in stream.cancel() }
        XCTAssertEqual(returned.wait(timeout: .now() + 2), .success)
        producer.finish()
        var iterator = stream.makeAsyncIterator()
        let end = try await iterator.next()
        XCTAssertNil(end)
        XCTAssertEqual(aborts.count, 1)
    }

    func testTaskCancellationWhileWaitingWakesConsumer() async throws {
        let aborts = Counter()
        let stream = stream(abort: { aborts.increment() })
        let started = expectation(description: "consumer started")
        let task = Task {
            var iterator = stream.makeAsyncIterator()
            started.fulfill()
            let end = try await iterator.next()
            XCTAssertNil(end)
        }
        await fulfillment(of: [started], timeout: 2)
        task.cancel()
        try await task.value
        XCTAssertEqual(aborts.count, 1)
        XCTAssertEqual(stream.producer.yield(1), .terminated)
    }

    func testAlreadyCancelledTaskDoesNotConsumeBufferedElement() async throws {
        let aborts = Counter()
        let stream = stream(abort: { aborts.increment() })
        stream.producer.yield(1)
        let task = Task {
            withUnsafeCurrentTask { $0?.cancel() }
            var iterator = stream.makeAsyncIterator()
            let end = try await iterator.next()
            XCTAssertNil(end)
        }
        try await task.value
        XCTAssertEqual(aborts.count, 1)
    }

    func testEarlyForAwaitExitAbortsWithStreamStillRetained() async throws {
        let aborts = Counter()
        let stream = stream(abort: { aborts.increment() })
        let producer = stream.producer
        let done = expectation(description: "producer unblocked after break")
        DispatchQueue.global().async {
            var value = 0
            while producer.yield(value) == .accepted { value += 1 }
            done.fulfill()
        }
        for try await value in stream {
            XCTAssertEqual(value, 0)
            break
        }
        await fulfillment(of: [done], timeout: 2)
        XCTAssertEqual(aborts.count, 1)
        XCTAssertEqual(producer.yield(99), .terminated)
        withExtendedLifetime(stream) {}
    }

    func testTaskCancellationDuringLoopBodyWithFullQueue() async throws {
        let aborts = Counter()
        let stream = stream(abort: { aborts.increment() })
        let producer = stream.producer
        let consuming = expectation(description: "consumer in loop body")
        let full = expectation(description: "queue refilled")
        let returned = DispatchSemaphore(value: 0)
        let task = Task {
            for try await _ in stream {
                consuming.fulfill()
                try await Task.sleep(nanoseconds: 10_000_000_000)
            }
        }
        DispatchQueue.global().async {
            XCTAssertEqual(producer.yield(1), .accepted)
            XCTAssertEqual(producer.yield(2), .accepted)
            full.fulfill()
            XCTAssertEqual(producer.yield(3), .terminated)
            returned.signal()
        }
        await fulfillment(of: [consuming, full], timeout: 2)
        XCTAssertEqual(returned.wait(timeout: .now() + 0.05), .timedOut)
        task.cancel()
        do { try await task.value; XCTFail("missing cancellation") }
        catch { XCTAssertTrue(error is CancellationError) }
        XCTAssertEqual(returned.wait(timeout: .now() + 2), .success)
        XCTAssertEqual(aborts.count, 1)
    }

    func testFinishResumesWaitingConsumer() async throws {
        for fail in [false, true] {
            let aborts = Counter()
            let stream = stream(abort: { aborts.increment() })
            let started = expectation(description: "consumer started")
            let task = Task {
                var iterator = stream.makeAsyncIterator()
                started.fulfill()
                do {
                    let end = try await iterator.next()
                    XCTAssertNil(end)
                    XCTAssertFalse(fail)
                } catch {
                    XCTAssertTrue(fail)
                    XCTAssertEqual(error as? TestError, .failed)
                }
                let end = try await iterator.next()
                XCTAssertNil(end)
            }
            await fulfillment(of: [started], timeout: 2)
            stream.producer.finish(throwing: fail ? TestError.failed : nil)
            try await task.value
            XCTAssertEqual(aborts.count, 0)
        }
    }

    func testByteBudgetBlocksBeforeCountBudget() async throws {
        let stream = BoundedStream<String>(capacity: 4, byteCapacity: 5,
                                           maxElementBytes: 5, sizeOf: { $0.utf8.count }, abort: {})
        let producer = stream.producer
        producer.yield("abc")
        let returned = DispatchSemaphore(value: 0)
        DispatchQueue.global().async {
            XCTAssertEqual(producer.yield("def"), .accepted)
            returned.signal()
        }
        XCTAssertEqual(returned.wait(timeout: .now() + 0.05), .timedOut)
        var iterator = stream.makeAsyncIterator()
        let first = try await iterator.next()
        XCTAssertEqual(first, "abc")
        XCTAssertEqual(returned.wait(timeout: .now() + 2), .success)
        producer.finish()
        let second = try await iterator.next()
        XCTAssertEqual(second, "def")
        let end = try await iterator.next()
        XCTAssertNil(end)
    }

    func testOversizedElementFailsAfterAcceptedBuffer() async throws {
        let stream = BoundedStream<String>(byteCapacity: 8, maxElementBytes: 3,
                                           sizeOf: { $0.utf8.count }, abort: {})
        let producer = stream.producer
        XCTAssertEqual(producer.yield("ok"), .accepted)
        XCTAssertEqual(producer.yield("large"), .terminated)
        var iterator = stream.makeAsyncIterator()
        let first = try await iterator.next()
        XCTAssertEqual(first, "ok")
        do { _ = try await iterator.next(); XCTFail("missing size failure") }
        catch { XCTAssertEqual(error as? BoundedStream<String>.QueueError, .elementTooLarge) }
        let end = try await iterator.next()
        XCTAssertNil(end)
    }

    func testInvalidSizeFailsAndCannotWaitForeverOnByteBudget() async throws {
        for size in [-1, 3] {
            let stream = BoundedStream<Int>(byteCapacity: 2, maxElementBytes: 4,
                                            sizeOf: { _ in size }, abort: {})
            XCTAssertEqual(stream.producer.yield(1), .terminated)
            var iterator = stream.makeAsyncIterator()
            do { _ = try await iterator.next(); XCTFail("missing size failure") }
            catch {
                XCTAssertEqual(error as? BoundedStream<Int>.QueueError,
                               size < 0 ? .invalidElementSize : .elementTooLarge)
            }
        }
    }

    func testSecondIteratorCannotCancelFirstIterator() async throws {
        let aborts = Counter()
        let stream = stream(abort: { aborts.increment() })
        var first = stream.makeAsyncIterator()
        do {
            var second = stream.makeAsyncIterator()
            second.cancel()
            do { _ = try await second.next(); XCTFail("second consumer accepted") }
            catch { XCTAssertEqual(error as? BoundedStream<Int>.QueueError, .multipleConsumers) }
        }
        XCTAssertEqual(aborts.count, 0)
        stream.producer.yield(1)
        stream.producer.finish()
        let value = try await first.next()
        let end = try await first.next()
        XCTAssertEqual(value, 1)
        XCTAssertNil(end)
    }

    func testAbortCanReenterProducerFinish() async throws {
        // The callback runs outside the condition lock, like native abort which
        // may synchronously produce a final callback.
        final class Context: @unchecked Sendable {
            var producer: BoundedStream<Int>.Producer?
        }
        let context = Context()
        let aborts = Counter()
        let stream = stream(abort: {
            aborts.increment()
            context.producer?.finish()
        })
        context.producer = stream.producer
        stream.cancel()
        stream.cancel()
        XCTAssertEqual(aborts.count, 1)
        context.producer = nil
    }

    func testOptionalElementIsNotMistakenForEnd() async throws {
        let stream = BoundedStream<Int?>(sizeOf: { _ in 1 }, abort: {})
        stream.producer.yield(nil)
        stream.producer.finish()
        var iterator = stream.makeAsyncIterator()
        let value = try await iterator.next()
        XCTAssertNotNil(value as Any?)
        XCTAssertNil(value!)
        let end = try await iterator.next()
        XCTAssertNil(end as Any?)
    }
}
