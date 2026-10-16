import Foundation

@main struct StandaloneRunner {
    static func main() async throws {
        alarm(60)
        // The same checks used by swift test, runnable when SwiftPM cannot load.
        let checks: [(String, @Sendable () async throws -> Void)] = [
            ("manifestRequiresExactVersionAndDigest", { try ContractTests.manifestRequiresExactVersionAndDigest() }),
            ("manifestAndHandles", { try await ContractTests.manifestAndHandles() }),
            ("versionMismatch", { try await ContractTests.versionMismatch() }),
            ("mainActorHeartbeat", { try await ContractTests.mainActorHeartbeat() }),
            ("boundedLosslessStream", { try await ContractTests.boundedLosslessStream() }),
            ("byteBudgetAndTerminalFullQueue", { try await ContractTests.byteBudgetAndTerminalFullQueue() }),
            ("cancelBeforeAcceptanceAndWhileWaiting", { try await ContractTests.cancelBeforeAcceptanceAndWhileWaiting() }),
            ("earlyLoopExit", { try await ContractTests.earlyLoopExit() }),
            ("cancelFullQueue", { try await ContractTests.cancelFullQueue() }),
            ("closeFullQueue", { try await ContractTests.closeFullQueue() }),
            ("concurrentCloseAndAdmission", { try await ContractTests.concurrentCloseAndAdmission() }),
            ("taskCancellationAbortsCall", { try await ContractTests.taskCancellationAbortsCall() }),
            ("unknownErrorPreserved", { try await ContractTests.unknownErrorPreserved() }),
            ("rejectedStream", { try await ContractTests.rejectedStream() }),
            ("terminalFailureOnce", { try await ContractTests.terminalFailureOnce() }),
            ("callbackFailureLatch", { try await ContractTests.callbackFailureLatch() }),
            ("missingTerminalFails", { try await ContractTests.missingTerminalFails() }),
            ("oversizedFrameAndLimits", { try await ContractTests.oversizedFrameAndLimits() }),
            ("secondConsumerRejected", { try await ContractTests.secondConsumerRejected() }),
            ("operationPrimitives", { try await ContractTests.operationPrimitives() }),
            ("cancelBlockedPushAndPoll", { try await ContractTests.cancelBlockedPushAndPoll() }),
            ("handleResults", { try WireTests.handleResults() }),
            ("wireValues", { try ContractTests.wireValues() }),
            ("testToolCallProviderMetadataRoundTrip", { try WireTests.testToolCallProviderMetadataRoundTrip() }),
            ("testUnknownFieldsKeepNullApartFromMissing", { try WireTests.testUnknownFieldsKeepNullApartFromMissing() }),
            ("testGeneratedFilesAreBase64AndMediaType", { try WireTests.testGeneratedFilesAreBase64AndMediaType() }),
            ("testTextStreamToolPartsAndUsageAreCamelCase", { try WireTests.testTextStreamToolPartsAndUsageAreCamelCase() }),
            ("testTextStreamDeltasAreText", { try WireTests.testTextStreamDeltasAreText() }),
            ("testFilePartsAreTheAiSdkFilePart", { try WireTests.testFilePartsAreTheAiSdkFilePart() }),
            ("testToolChoiceCodableWire", { try WireTests.testToolChoiceCodableWire() }),
            ("testGenerateTextResultRoundTrip", { try WireTests.testGenerateTextResultRoundTrip() }),
        ]
        for (name, check) in checks {
            try await check()
            print("PASS \(name)")
        }
        alarm(0)
        print("Passed \(checks.count) Swift ABI contract checks")
    }
}
