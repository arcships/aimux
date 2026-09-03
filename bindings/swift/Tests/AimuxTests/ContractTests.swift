// ContractTests.swift — validates the Swift binding against the shared
// wire-format fixtures in `contract-tests/fixtures/wire-format.json`, the same
// file the Rust, Go, Java, Kotlin, Python and Node contract tests read.
//
// Rust is the wire authority: it produces the JSON, the bindings consume it.
// So the direction that matters here is DECODING — every fixture must decode
// into the Swift type it claims to describe. Encoding is checked as a Swift
// round-trip (decode → encode → decode) rather than byte-comparing against the
// fixture, because Swift's synthesized Codable omits nil optionals while Rust
// emits explicit nulls for most fields; a byte comparison would fail on that
// difference alone and say nothing about type correctness.
//
// These are pure serialization tests — no native library is loaded.

import XCTest
@testable import Aimux

private struct Fixture: Decodable {
    let name: String
    let type: String
    let json: String
}

final class ContractTests: XCTestCase {

    // MARK: - fixture loading

    /// Repo root derived from this file's location:
    /// <repo>/bindings/swift/Tests/AimuxTests/ContractTests.swift
    private static var fixtureURL: URL {
        URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()   // AimuxTests
            .deletingLastPathComponent()   // Tests
            .deletingLastPathComponent()   // swift
            .deletingLastPathComponent()   // bindings
            .deletingLastPathComponent()   // <repo>
            .appendingPathComponent("contract-tests/fixtures/wire-format.json")
    }

    private func loadFixtures() throws -> [Fixture] {
        let url = Self.fixtureURL
        guard FileManager.default.fileExists(atPath: url.path) else {
            XCTFail("shared fixtures not found at \(url.path)")
            return []
        }
        return try JSONDecoder().decode([Fixture].self, from: Data(contentsOf: url))
    }

    /// Decode a fixture, re-encode it, and decode again — the encoded form must
    /// survive Swift's own round-trip.
    private func assertRoundTrips<T: Codable & Equatable>(
        _ type: T.Type, _ fixture: Fixture
    ) throws {
        let data = Data(fixture.json.utf8)
        let decoded: T
        do {
            decoded = try JSONDecoder().decode(T.self, from: data)
        } catch {
            XCTFail("fixture '\(fixture.name)' (\(fixture.type)) does not decode into \(T.self): \(error)\n  json: \(fixture.json)")
            return
        }
        let reencoded = try JSONEncoder().encode(decoded)
        let again = try JSONDecoder().decode(T.self, from: reencoded)
        XCTAssertEqual(decoded, again, "fixture '\(fixture.name)' does not survive a Swift encode/decode round-trip")
    }

    func testTopLevelToolCallProviderMetadataRoundTrips() throws {
        let json = #"{"tool_call_id":"call_1","tool_name":"get_weather","input":{"city":"Paris"},"provider_metadata":{"openai":{"itemId":"item_1"}}}"#
        let call = try JSONDecoder().decode(ToolCall.self, from: Data(json.utf8))
        XCTAssertEqual(call.providerMetadata?["openai"]?["itemId"]?.stringValue, "item_1")

        let encoded = try JSONEncoder().encode(call)
        let decoded = try JSONDecoder().decode(ToolCall.self, from: encoded)
        XCTAssertEqual(decoded, call)
    }

    func testProviderExecutedToolTranscriptMessageRoundTrips() throws {
        let fixtures = try loadFixtures()
        guard let fixture = fixtures.first(where: {
            $0.name == "model_message_provider_executed_tool_transcript"
        }) else {
            return XCTFail("provider-executed transcript fixture is missing")
        }
        let message = try JSONDecoder().decode(
            ModelMessage.self,
            from: Data(fixture.json.utf8)
        )
        guard case .parts(let parts) = message.content else {
            return XCTFail("expected multipart assistant message")
        }
        guard case .toolCall(_, let name, _, let providerExecuted,
                             let thoughtSignature, _) = parts[0] else {
            return XCTFail("expected tool call")
        }
        XCTAssertEqual(name, "search")
        XCTAssertEqual(providerExecuted, true)
        XCTAssertEqual(thoughtSignature, "sig_provider")
        guard case .toolResult(_, _, let resultName, let isError,
                               let preliminary, let dynamic, _) = parts[1] else {
            return XCTFail("expected tool result")
        }
        XCTAssertEqual(resultName, "search")
        XCTAssertEqual(isError, false)
        XCTAssertEqual(preliminary, true)
        XCTAssertEqual(dynamic, true)

        let reencoded = try JSONEncoder().encode(message)
        XCTAssertEqual(try JSONDecoder().decode(ModelMessage.self, from: reencoded), message)
    }

    func testResultToolCallThoughtSignaturesRoundTrip() throws {
        let fixtures = try loadFixtures()
        guard let fixture = fixtures.first(where: { $0.name == "generate_content_tool_call" }) else {
            return XCTFail("generate-content tool-call fixture is missing")
        }
        let content = try JSONDecoder().decode(
            GenerateContent.self,
            from: Data(fixture.json.utf8)
        )
        guard case .toolCall(_, _, _, _, _, let generateSignature, _) = content else {
            return XCTFail("expected generate-content tool call")
        }
        XCTAssertEqual(generateSignature, "sig_abc")
        let contentData = try JSONEncoder().encode(content)
        XCTAssertEqual(try JSONDecoder().decode(GenerateContent.self, from: contentData), content)

        let streamJSON = #"{"ToolCall":{"tool_call_id":"stream_1","tool_name":"search","input":{"query":"Rust"},"thought_signature":"sig_stream"}}"#
        let part = try JSONDecoder().decode(StreamPart.self, from: Data(streamJSON.utf8))
        guard case .toolCall(_, _, _, _, _, let streamSignature, _, _, _) = part else {
            return XCTFail("expected stream tool call")
        }
        XCTAssertEqual(streamSignature, "sig_stream")
        let partData = try JSONEncoder().encode(part)
        XCTAssertEqual(try JSONDecoder().decode(StreamPart.self, from: partData), part)
    }

    // MARK: - every fixture decodes

    /// A fixture type with no case here fails rather than being skipped:
    /// silent skipping is how a net grows holes.
    func testEveryFixtureDecodesIntoItsSwiftType() throws {
        let fixtures = try loadFixtures()
        XCTAssertFalse(fixtures.isEmpty, "no fixtures loaded")

        for fixture in fixtures {
            switch fixture.type {
            case "ToolChoice":           try assertRoundTrips(ToolChoice.self, fixture)
            case "StreamPart":           try assertRoundTrips(StreamPart.self, fixture)
            case "GenerateContent":      try assertRoundTrips(GenerateContent.self, fixture)
            case "GenerateTextOptions":  try assertRoundTrips(GenerateTextOptions.self, fixture)
            case "TimeoutConfiguration": try assertRoundTrips(TimeoutConfiguration.self, fixture)
            case "Role":                 try assertRoundTrips(Role.self, fixture)
            case "ModelMessage":         try assertRoundTrips(ModelMessage.self, fixture)
            case "FinishReasonUnified":  try assertRoundTrips(FinishReasonUnified.self, fixture)
            case "ReasoningEffort":      try assertRoundTrips(ReasoningEffort.self, fixture)
            default:
                XCTFail("fixture '\(fixture.name)' declares type '\(fixture.type)', which has no case in ContractTests — wire it up so the fixture is actually checked against Swift")
            }
        }
    }

    // MARK: - the numeric type probe

    /// `top_k` is fractional in the fixture on purpose: Rust is `Option<f64>`
    /// (matching the upstream AI SDK's `topK?: number`), so a binding that
    /// declares it as an integer cannot hold this value. Kotlin and Java both
    /// declared it `Long` until this fixture was added.
    func testNumericTypesSurviveWithFullPrecision() throws {
        let fixtures = try loadFixtures()
        guard let fixture = fixtures.first(where: { $0.name == "generate_text_options_numeric_types" }) else {
            XCTFail("fixture 'generate_text_options_numeric_types' is missing")
            return
        }
        let opts = try JSONDecoder().decode(GenerateTextOptions.self, from: Data(fixture.json.utf8))
        XCTAssertEqual(opts.topK, 40.5, "top_k must keep its fraction — an integer type would truncate it to 40")
        XCTAssertEqual(opts.frequencyPenalty, -0.5, "penalties must stay signed")
        XCTAssertEqual(opts.temperature, 0.7)
        XCTAssertEqual(opts.topP, 0.95)
        XCTAssertEqual(opts.maxOutputTokens, 256)
        XCTAssertEqual(opts.seed, 42)
        XCTAssertEqual(opts.maxRetries, 3)
    }
}
