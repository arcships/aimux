import Foundation
@testable import Aimux
#if canImport(CAimuxTestABI)
import CAimuxTestABI
#endif
#if !AIMUX_STANDALONE
import Testing
#endif

struct CheckFailure: Error, CustomStringConvertible {
    let description: String
}
func require(_ condition: Bool, _ message: String = #function,
             file: String = #fileID, line: Int = #line) throws {
    if !condition { throw CheckFailure(description: "\(file):\(line): \(message)") }
}
#if canImport(CAimuxTestABI)
func eventually(_ predicate: @escaping @Sendable () -> Bool) async throws {
    for _ in 0..<2000 {
        if predicate() { return }
        try await Task.sleep(nanoseconds: 1_000_000)
    }
    throw CheckFailure(description: "Timed out waiting for native cleanup/state")
}
func newHandle() throws -> (AimuxHandle, UInt64) {
    let id = aimux_test_new_handle()
    return (try AimuxHandle(owning: id), id)
}
func smallLimits() throws -> StreamLimits {
    try StreamLimits(maxItems: 2, maxBytes: 256, maxEventBytes: 256)
}

#if !AIMUX_STANDALONE
@Suite(.serialized)
#endif
struct ContractTests {
#if !AIMUX_STANDALONE
    @Test
#endif
    static func manifestAndHandles() async throws {
        let manifest = try await AimuxRuntime.manifest()
        guard case .object(let fields) = manifest else { throw CheckFailure(description: "Missing manifest") }
        try require(fields["opsVersion"] == .string("1.0"))
        let handle = try await AimuxRuntime.open("provider.create")
        try require(try await handle.call("echo", params: .string("你好 🌍")) == .string("你好 🌍"))
        handle.close()
        do { _ = try await handle.call("echo"); throw CheckFailure(description: "Closed handle admitted") }
        catch let error as AimuxError { try require(error.code == 304) }
        try await eventually { aimux_test_strings() == 0 && aimux_test_errors() == 0 }
    }
#if !AIMUX_STANDALONE
    @Test
#endif
    static func versionMismatch() async throws {
        aimux_test_version(2 << 16)
        defer { aimux_test_version(1 << 16) }
        do { _ = try await AimuxRuntime.manifest(); throw CheckFailure(description: "Accepted wrong ABI") }
        catch let error as AimuxError { try require(error.code == 304) }
    }
#if !AIMUX_STANDALONE
    @Test
#endif
    @MainActor static func mainActorHeartbeat() async throws {
        let mainCalls = aimux_test_main_calls()
        let request = Task { try await AimuxRuntime.call("slow") }
        try await eventually { aimux_test_active() == 1 }
        let start = Date()
        try await Task.sleep(nanoseconds: 5_000_000)
        try require(Date().timeIntervalSince(start) < 0.1, "Main actor heartbeat was blocked")
        _ = try await request.value
        try require(aimux_test_main_calls() == mainCalls, "Native blocking work ran on main thread")
    }
#if !AIMUX_STANDALONE
    @Test
#endif
    static func boundedLosslessStream() async throws {
        let (handle, _) = try newHandle()
        defer { handle.close() }
        let stream = try await handle.stream("many", limits: smallLimits())
        try await eventually { stream.channel.snapshot.items == 2 }
        var count = 0
        for try await event in stream {
            try require(event.seq == String(count), "Lost or reordered event")
            guard case .object(let payload) = event.event else { throw CheckFailure(description: "Wrong payload") }
            try require(payload["text"] == .string("你好 🌍"))
            count += 1
        }
        try require(count == 10000, "Stream dropped events")
        try require(stream.channel.snapshot.highItems <= 2)
        try require(stream.channel.snapshot.highBytes <= 256)
        try await eventually { stream.channel.snapshot.releases == 1 }
    }
#if !AIMUX_STANDALONE
    @Test
#endif
    static func byteBudgetAndTerminalFullQueue() async throws {
        let (handle, _) = try newHandle()
        defer { handle.close() }
        let limits = try StreamLimits(maxItems: 256, maxBytes: 128, maxEventBytes: 128)
        let stream = try await handle.stream("four", limits: limits)
        try await eventually { stream.channel.snapshot.items == 1 }
        var count = 0
        for try await _ in stream { count += 1 }
        try require(count == 4 && stream.channel.snapshot.highItems == 1)
        try require(stream.channel.snapshot.highBytes <= 128)
        let terminalLimits = try StreamLimits(maxItems: 1, maxBytes: 1024, maxEventBytes: 1024)
        let terminal = try await handle.stream("oneError", limits: terminalLimits)
        try await eventually { aimux_test_active() == 0 && terminal.channel.snapshot.releases == 1 }
        try require(terminal.channel.snapshot.items == 1, "Terminal used data queue capacity")
        let iterator = terminal.makeAsyncIterator()
        try require(try await iterator.next()?.seq == "0")
        do { _ = try await iterator.next(); throw CheckFailure(description: "Terminal lost behind full queue") }
        catch let error as AimuxError { try require(error.code == 1) }
        try require(try await iterator.next() == nil)
    }
#if !AIMUX_STANDALONE
    @Test
#endif
    static func cancelBeforeAcceptanceAndWhileWaiting() async throws {
        let (handle, _) = try newHandle()
        defer { handle.close() }
        let starting = Task { try await handle.stream("connecting") }
        try await eventually { aimux_test_active() == 1 }
        starting.cancel()
        do { _ = try await starting.value; throw CheckFailure(description: "Startup ignored cancel") }
        catch is CancellationError {}
        let stream = try await handle.stream("idle")
        let consumer = Task {
            for try await _ in stream {}
        }
        try await eventually { stream.channel.snapshot.items == 0 }
        consumer.cancel()
        do { try await consumer.value; throw CheckFailure(description: "Waiting reader ignored cancel") }
        catch is CancellationError {}
        try await eventually { stream.channel.snapshot.releases == 1 && aimux_test_active() == 0 }
    }
#if !AIMUX_STANDALONE
    @Test
#endif
    static func earlyLoopExit() async throws {
        let (handle, _) = try newHandle()
        defer { handle.close() }
        let stream = try await handle.stream("many", limits: smallLimits())
        let before = aimux_test_aborts()
        for try await _ in stream { break }
        // Keep the sequence alive: the temporary iterator must cancel on break.
        try await eventually { stream.channel.snapshot.releases == 1 && aimux_test_active() == 0 }
        try require(aimux_test_aborts() > before)
        try require(stream.channel.snapshot.items == 0)
    }
#if !AIMUX_STANDALONE
    @Test
#endif
    static func cancelFullQueue() async throws {
        let (handle, _) = try newHandle()
        defer { handle.close() }
        let stream = try await handle.stream("many", limits: smallLimits())
        try await eventually { stream.channel.snapshot.items == 2 }
        let consumer = Task {
            let iterator = stream.makeAsyncIterator()
            do { try await Task.sleep(nanoseconds: 10_000_000_000) } catch {}
            return try await iterator.next()
        }
        consumer.cancel()
        do { _ = try await consumer.value; throw CheckFailure(description: "Cancellation ignored") }
        catch is CancellationError {}
        try await eventually { stream.channel.snapshot.releases == 1 && aimux_test_active() == 0 }
        try require(stream.channel.snapshot.items == 0)
    }
#if !AIMUX_STANDALONE
    @Test
#endif
    static func closeFullQueue() async throws {
        let (handle, id) = try newHandle()
        let stream = try await handle.stream("many", limits: smallLimits())
        try await eventually { stream.channel.snapshot.items == 2 }
        DispatchQueue.concurrentPerform(iterations: 100) { _ in handle.close() }
        try await eventually { stream.channel.snapshot.releases == 1 && aimux_test_drops(id) == 1 }
        try require(aimux_test_active() == 0)
        try require(stream.channel.snapshot.items == 0)
    }
#if !AIMUX_STANDALONE
    @Test
#endif
    static func concurrentCloseAndAdmission() async throws {
        let (handle, id) = try newHandle()
        let request = Task { try await handle.call("slow") }
        try await eventually { aimux_test_active() == 1 }
        DispatchQueue.concurrentPerform(iterations: 100) { _ in handle.close() }
        try require(aimux_test_drops(id) == 0, "Dropped an admitted native call")
        _ = try await request.value
        try await eventually { aimux_test_drops(id) == 1 }
        do { _ = try await handle.call("echo"); throw CheckFailure(description: "Admission after close") }
        catch let error as AimuxError { try require(error.code == 304) }
        handle.close()
        try require(aimux_test_drops(id) == 1)
    }
#if !AIMUX_STANDALONE
    @Test
#endif
    static func taskCancellationAbortsCall() async throws {
        let request = Task { try await AimuxRuntime.call("wait") }
        try await eventually { aimux_test_active() == 1 }
        request.cancel()
        do { _ = try await request.value; throw CheckFailure(description: "Call ignored cancel") }
        catch is CancellationError {}
        try await eventually { aimux_test_active() == 0 }
    }
#if !AIMUX_STANDALONE
    @Test
#endif
    static func unknownErrorPreserved() async throws {
        do { _ = try await AimuxRuntime.call("error"); throw CheckFailure(description: "Missing error") }
        catch let error as AimuxError {
            try require(error.kind == "aimux" && error.code == 1 && error.retryAfterMs == 123)
            try require(error.message == "未来错误 🌍")
            let expected: JSONValue = .object([
                "name": .string("AI_FutureError"),
                "opaque": .object(["newField": .array([.bool(true), .null, .number(7)])]),
            ])
            try require(error.error == expected)
            try require(try JSONDecoder().decode(AimuxError.self, from: JSONEncoder().encode(error)) == error)
        }
        try require(aimux_test_errors() == 0 && aimux_test_strings() == 0)
    }
#if !AIMUX_STANDALONE
    @Test
#endif
    static func rejectedStream() async throws {
        let (handle, _) = try newHandle()
        defer { handle.close() }
        let emitted = aimux_test_emitted()
        do { _ = try await handle.stream("reject"); throw CheckFailure(description: "Expected startup failure") }
        catch let error as AimuxError { try require(error.error != .null) }
        try require(aimux_test_emitted() == emitted)
        try await eventually { aimux_test_active() == 0 && aimux_test_errors() == 0 }
    }
#if !AIMUX_STANDALONE
    @Test
#endif
    static func terminalFailureOnce() async throws {
        let (handle, _) = try newHandle()
        defer { handle.close() }
        let stream = try await handle.stream("terminalError")
        let iterator = stream.makeAsyncIterator()
        do { _ = try await iterator.next(); throw CheckFailure(description: "Expected terminal error") }
        catch let error as AimuxError { try require(error.message == "未来错误 🌍") }
        try require(try await iterator.next() == nil)
        try await eventually { stream.channel.snapshot.releases == 1 }
    }
#if !AIMUX_STANDALONE
    @Test
#endif
    static func callbackFailureLatch() async throws {
        let (handle, _) = try newHandle()
        defer { handle.close() }
        let stream = try await handle.stream("malformed")
        let iterator = stream.makeAsyncIterator()
        try require(try await iterator.next()?.seq == "0")
        do { _ = try await iterator.next(); throw CheckFailure(description: "Malformed callback ignored") }
        catch let error as AimuxError { try require(error.kind == "binding" && error.code == 206) }
        try require(try await iterator.next() == nil)
        try await eventually { stream.channel.snapshot.releases == 1 }
    }
#if !AIMUX_STANDALONE
    @Test
#endif
    static func missingTerminalFails() async throws {
        let (handle, _) = try newHandle()
        defer { handle.close() }
        let stream = try await handle.stream("missingEnd")
        do {
            for try await _ in stream {}
            throw CheckFailure(description: "EOF reported success")
        } catch let error as AimuxError { try require(error.code == 206) }
        try await eventually { stream.channel.snapshot.releases == 1 }
    }
#if !AIMUX_STANDALONE
    @Test
#endif
    static func oversizedFrameAndLimits() async throws {
        do { _ = try StreamLimits(maxItems: 257, maxBytes: 100, maxEventBytes: 100); throw CheckFailure(description: "Unbounded limit") }
        catch let error as AimuxError { try require(error.code == 305) }
        let (handle, _) = try newHandle()
        defer { handle.close() }
        do { _ = try await handle.stream("oversized"); throw CheckFailure(description: "Oversized frame accepted") }
        catch let error as AimuxError { try require(error.code == 305) }
        try await eventually { aimux_test_active() == 0 }
    }
#if !AIMUX_STANDALONE
    @Test
#endif
    static func secondConsumerRejected() async throws {
        let (handle, _) = try newHandle()
        defer { handle.close() }
        let stream = try await handle.stream("many", limits: smallLimits())
        let first = stream.makeAsyncIterator()
        let second = stream.makeAsyncIterator()
        do { _ = try await second.next(); throw CheckFailure(description: "Multiple readers admitted") }
        catch let error as AimuxError { try require(error.code == 303) }
        try require(try await first.next()?.seq == "0")
        first.close()
        try await eventually { stream.channel.snapshot.releases == 1 }
    }
#if !AIMUX_STANDALONE
    @Test
#endif
    static func operationPrimitives() async throws {
        let (handle, _) = try newHandle()
        defer { handle.close() }
        try await handle.push(Data())
        try await handle.push(Data([1, 2, 3]))
        try require(try await handle.nextPart(timeoutMs: 0) == .timeout)
        try require(try await handle.nextPart(timeoutMs: 1) == .ended)
        try require(try await handle.nextPart(timeoutMs: 2) == .part(.object(["text": .string("hello")])))
        do { try await handle.push(Data(count: 1_048_577)); throw CheckFailure(description: "Oversized chunk accepted") }
        catch let error as AimuxError { try require(error.code == 305) }
        try require(aimux_test_strings() == 0)
    }
#if !AIMUX_STANDALONE
    @Test
#endif
    static func cancelBlockedPushAndPoll() async throws {
        let (pushHandle, pushID) = try newHandle()
        let push = Task { try await pushHandle.push(Data(count: 8)) }
        try await eventually { aimux_test_active() == 1 }
        push.cancel()
        do { try await push.value; throw CheckFailure(description: "Push ignored cancel") }
        catch is CancellationError {}
        pushHandle.close()
        try await eventually { aimux_test_drops(pushID) == 1 }
        let (pollHandle, pollID) = try newHandle()
        let poll = Task { try await pollHandle.nextPart(timeoutMs: -1) }
        try await eventually { aimux_test_active() == 1 }
        pollHandle.close()
        do { _ = try await poll.value; throw CheckFailure(description: "Closed poll reported success") }
        catch is CancellationError {}
        try await eventually { aimux_test_drops(pollID) == 1 }
    }
#if !AIMUX_STANDALONE
    @Test
#endif
    static func wireValues() throws {
        let original: JSONValue = .object(["unknown": .null, "bool": .bool(true), "text": .string("你好 🌍")])
        try require(try JSONDecoder().decode(JSONValue.self, from: JSONEncoder().encode(original)) == original)
        let part = try JSONDecoder().decode(TextStreamPart.self, from: Data(#"{"type":"text-delta","id":"x","text":"hi"}"#.utf8))
        try require(try JSONDecoder().decode(TextStreamPart.self, from: JSONEncoder().encode(part)) == part)
        let options = GenerateTextOptions(maxOutputTokens: 12, providerOptions: ["future": ["opaque": .null]])
        try require(try JSONDecoder().decode(GenerateTextOptions.self, from: JSONEncoder().encode(options)) == options)
    }
}
#endif
