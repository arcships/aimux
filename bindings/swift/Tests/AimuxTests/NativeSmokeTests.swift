#if !canImport(CAimuxTestABI)
import Foundation
@testable import Aimux
#if !AIMUX_STANDALONE
import Testing
#endif

struct NativeSmokeTests {
#if !AIMUX_STANDALONE
    @Test
#endif
    static func manifest() async throws {
        let manifest = try await AimuxRuntime.manifest()
        try require(manifest["ops"]?.arrayValue?.contains { $0["name"] == .string(AimuxOps.OP_DECISION_DECIDE) } == true)
    }
#if !AIMUX_STANDALONE
    @Test
#endif
    static func unknownOpEnvelope() async throws {
        do {
            _ = try await AimuxRuntime.call("swift.unknown.op")
            throw CheckFailure(description: "Unknown op accepted")
        } catch let error as AimuxError {
            try require(error.kind == "protocol" && error.code == 300)
            try require(error.error["name"] == .string("InvalidOp"))
        }
    }

    private static func provider(_ package: String) async throws -> AimuxHandle {
        try await AimuxRuntime.open(AimuxOps.OP_PROVIDER_CREATE, params: .object([
            "package": .string(package), "settings": .object(["apiKey": .string("test-key")]),
        ]))
    }
#if !AIMUX_STANDALONE
    @Test
#endif
    static func providerAndModel() async throws {
        let provider = try await provider("openai")
        defer { provider.close() }
        let model = try await provider.open(AimuxOps.OP_PROVIDER_MODEL, params: .object([
            "method": .string("languageModel"), "modelId": .string("test-model"),
        ]))
        defer { model.close() }
        // Models retain native ownership after their provider facade closes.
        provider.close()
        let info = try await model.call(AimuxOps.OP_MODEL_INFO)
        try require(info["modelId"] == .string("test-model"))
        model.close()
        model.close()
        do { _ = try await model.call(AimuxOps.OP_MODEL_INFO); throw CheckFailure(description: "Closed handle admitted") }
        catch let error as AimuxError { try require(error.code == 304) }
    }
#if !AIMUX_STANDALONE
    @Test
#endif
    static func secondaryHandleReference() async throws {
        let provider = try await provider("openai")
        defer { provider.close() }
        guard let id = provider.id else { throw CheckFailure(description: "Missing provider ID") }
        let registry = try await AimuxRuntime.open(AimuxOps.OP_REGISTRY_CREATE, params: .object([
            "namespace": .string("swift"),
            "entries": .array([.object(["key": .string("test"), "provider": .string(String(id))])]),
        ]))
        defer { registry.close() }
        provider.close()
        try require(provider.id == nil)
        let retained = try await registry.open(AimuxOps.OP_REGISTRY_PROVIDER, params: .object(["key": .string("test")]))
        defer { retained.close() }
        let model = try await retained.open(AimuxOps.OP_PROVIDER_MODEL, params: .object([
            "method": .string("languageModel"), "modelId": .string("retained-model"),
        ]))
        defer { model.close() }
        try require(try await model.call(AimuxOps.OP_MODEL_INFO)["modelId"] == .string("retained-model"))
    }
#if !AIMUX_STANDALONE
    @Test
#endif
    static func decision() async throws {
        let provider = try await provider("jev")
        defer { provider.close() }
        let model = try await provider.open(AimuxOps.OP_PROVIDER_MODEL, params: .object([
            "method": .string("decision"), "modelId": .string("test-decision"),
        ]))
        defer { model.close() }
        let capabilities = try await model.call(AimuxOps.OP_DECISION_CAPABILITIES)
        try require(capabilities.objectValue != nil)
    }
#if !AIMUX_STANDALONE
    @Test
#endif
    static func terminalErrorAndRejectedStream() async throws {
        let provider = try await provider("openai")
        defer { provider.close() }
        let model = try await provider.open(AimuxOps.OP_PROVIDER_MODEL, params: .object([
            "method": .string("languageModel"), "modelId": .string("test-model"),
        ]))
        defer { model.close() }
        let stream = try await model.stream(AimuxOps.OP_TEXT_STREAM, params: .object(["prompt": .array([])]))
        let iterator = stream.makeAsyncIterator()
        do { _ = try await iterator.next(); throw CheckFailure(description: "Empty prompt succeeded") }
        catch let error as AimuxError { try require(error.kind == "aimux" && error.code == 6) }
        try require(try await iterator.next() == nil)
        do { _ = try await provider.stream(AimuxOps.OP_TEXT_STREAM); throw CheckFailure(description: "Wrong handle kind accepted") }
        catch let error as AimuxError { try require(error.code == 203) }
    }
#if !AIMUX_STANDALONE
    @Test
#endif
    static func primitiveValidation() async throws {
        let abort = try await AimuxRuntime.open(AimuxOps.OP_ABORT_CREATE)
        defer { abort.close() }
        _ = try await abort.call(AimuxOps.OP_ABORT_SIGNAL)
        do { try await abort.push(Data()); throw CheckFailure(description: "Push accepted an Abort") }
        catch let error as AimuxError { try require(error.code == 203) }
        do { _ = try await abort.nextPart(timeoutMs: 0); throw CheckFailure(description: "Poll accepted an Abort") }
        catch let error as AimuxError { try require(error.code == 203) }
        do { _ = try await abort.nextPart(timeoutMs: -1); throw CheckFailure(description: "Negative timeout accepted") }
        catch let error as AimuxError { try require(error.code == 301) }
    }
}
#endif
