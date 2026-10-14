import XCTest
import Foundation
@testable import Aimux

final class DecisionTests: XCTestCase {
    func testProviderDecisionModelOwnsItsHandle() throws {
        var provider: ProviderHandle? = try Model.createProvider(name: "openai", apiKey: "test-key")
        let model = try provider!.decisionModel("gpt-6-luna")
        defer { model.close() }
        provider = nil // ProviderHandle uses ARC; the decision model owns its handle.
        let description = try JSONSerialization.jsonObject(with: Data(model.describe().utf8)) as! [String: Any]
        XCTAssertEqual(description["specificationVersion"] as? String, "v4")
        XCTAssertEqual(description["provider"] as? String, "openai")
        XCTAssertEqual(description["modelId"] as? String, "gpt-6-luna")
        XCTAssertNil(provider)
    }

    func testOfficialStructuredContractAndDescription() throws {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        let fixture = try JSONSerialization.jsonObject(with: Data(contentsOf:
            root.appendingPathComponent("contract-tests/fixtures/decision-native.json"))) as! [String: Any]
        func encode(_ value: Any) throws -> String {
            String(data: try JSONSerialization.data(withJSONObject: value), encoding: .utf8)!
        }
        func decode(_ value: String) throws -> [String: Any] {
            try JSONSerialization.jsonObject(with: Data(value.utf8)) as! [String: Any]
        }
        let response = MockResponse.json(fixture["response"]!)
        let server = MockHTTPServer(response: response)
        try server.start()
        defer { server.stop() }
        let model = try DecisionModel.jev(apiKey: "test-key", modelId: "jev-latest",
            endpoint: server.baseURL + "/v1/systemone")
        defer { model.close() }
        let description = try decode(model.describe())
        XCTAssertEqual(description["provider"] as? String, "jev")
        XCTAssertEqual((description["supportedQuestionTypes"] as? [String])?.count, 3)
        XCTAssertEqual(server.lastRequestBody, "")
        let request = fixture["request"] as! [String: Any]
        let result = try decode(model.decide(options: encode(request)))
        let answers = result["answers"] as! [String: Any]
        XCTAssertEqual((answers["urgent"] as? [String: Any])?["probability"] as? Double, 0.9)
        XCTAssertEqual((result["rounding"] as? [String: Any])?["scoreDecimals"] as? Int, 2)
        XCTAssertEqual(result["usage"] as? NSDictionary,
            ["inputTokens": 50, "outputTokens": 10, "totalTokens": 60] as NSDictionary)
        let questions = request["questions"] as! [String: Any]
        let actual = try decode(server.lastRequestBody)["questions"] as! [String: Any]
        XCTAssertEqual(server.lastRequestPath, "/v1/systemone")
        XCTAssertEqual((actual["urgent"] as! [String: Any])["criteria"] as? NSDictionary,
            (questions["urgent"] as! [String: Any])["criteria"] as? NSDictionary)
        XCTAssertEqual((actual["severity"] as! [String: Any])["criteria"] as? NSArray,
            (questions["severity"] as! [String: Any])["criteria"] as? NSArray)
        XCTAssertThrowsError(try model.decide(options: "{"))
        model.close()
        XCTAssertThrowsError(try model.describe())
        XCTAssertThrowsError(try DecisionModel.jev(apiKey: "test", modelId: "jev-latest", probabilitySource: "unknown"))
    }

    func testOpenAIMediaAndTypedValues() throws {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        let fixtureData = try Data(contentsOf:
            root.appendingPathComponent("contract-tests/fixtures/decision-openai-full.json"))
        let fixture = try JSONSerialization.jsonObject(with: fixtureData) as! [String: Any]
        func encode(_ value: Any) throws -> String {
            String(data: try JSONSerialization.data(withJSONObject: value), encoding: .utf8)!
        }
        let server = MockHTTPServer(response: MockResponse.json(fixture["response"]!))
        try server.start()
        defer { server.stop() }
        let provider = try Model.createProvider(name: "openai", apiKey: "test-key",
            configJson: encode(["base_url": server.baseURL + "/v1"]))
        let model = try provider.decisionModel("gpt-6-luna")
        defer { model.close() }
        // The questions and criteria are maps whose order is the request's
        // order, and JSONSerialization keeps no key order: send the fixture's
        // own text for the options.
        let options = rawObject(String(decoding: fixtureData, as: UTF8.self), key: "options")
        let result = try JSONSerialization.jsonObject(with: Data(model.decide(options: options).utf8)) as! [String: Any]
        let actual = try JSONSerialization.jsonObject(with: Data(server.lastRequestBody.utf8)) as! NSDictionary
        XCTAssertEqual(actual, fixture["request"] as! NSDictionary)
        XCTAssertEqual(server.lastRequestPath, "/v1/decisions")
        let answers = result["answers"] as! [String: Any]
        XCTAssertEqual((answers["choice"] as! [String: Any])["choice"] as? String, "boolean_true")
        let metadata = (result["providerMetadata"] as! [String: Any])["openai"] as! [String: Any]
        let choice = (metadata["answers"] as! [String: Any])["choice"] as! [String: Any]
        XCTAssertEqual(choice["value"] as? Bool, true)
        XCTAssertEqual((answers["score"] as! [String: Any])["score"] as? Double, 0.75)
    }
}

/// The raw text of the top-level `key` object in the JSON `text`.
private func rawObject(_ text: String, key: String) -> String {
    let marker = text.range(of: "\"\(key)\":")!
    let start = text[marker.upperBound...].firstIndex(of: "{")!
    var depth = 0
    var inString = false
    var escaped = false
    var index = start
    while index < text.endIndex {
        let c = text[index]
        if inString {
            if escaped {
                escaped = false
            } else if c == "\\" {
                escaped = true
            } else if c == "\"" {
                inString = false
            }
        } else if c == "\"" {
            inString = true
        } else if c == "{" {
            depth += 1
        } else if c == "}" {
            depth -= 1
            if depth == 0 { return String(text[start...index]) }
        }
        index = text.index(after: index)
    }
    fatalError("no balanced object for \(key)")
}
