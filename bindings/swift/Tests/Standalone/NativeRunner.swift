import Foundation

@main struct NativeRunner {
    static func main() async throws {
        alarm(60)
        let checks: [(String, @Sendable () async throws -> Void)] = [
            ("manifest", { try await NativeSmokeTests.manifest() }),
            ("unknownOpEnvelope", { try await NativeSmokeTests.unknownOpEnvelope() }),
            ("providerAndModel", { try await NativeSmokeTests.providerAndModel() }),
            ("secondaryHandleReference", { try await NativeSmokeTests.secondaryHandleReference() }),
            ("decision", { try await NativeSmokeTests.decision() }),
            ("terminalErrorAndRejectedStream", { try await NativeSmokeTests.terminalErrorAndRejectedStream() }),
            ("primitiveValidation", { try await NativeSmokeTests.primitiveValidation() }),
            ("handleResults", { try WireTests.handleResults() }),
        ]
        for (name, check) in checks {
            try await check()
            print("PASS \(name)")
        }
        alarm(0)
        print("Passed \(checks.count) native Swift checks")
    }
}
