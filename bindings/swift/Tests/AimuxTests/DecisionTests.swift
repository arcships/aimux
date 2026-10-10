import XCTest
import Foundation
@testable import Aimux

final class DecisionTests: XCTestCase {
    func testProviderDecisionModelOwnsItsHandle() throws {
        var provider: ProviderHandle? = try Model.createProvider(name: "openai", apiKey: "test-key")
        let model = try provider!.decisionModel("gpt-6-luna")
        defer { model.close() }
        provider = nil // ProviderHandle uses ARC; the decision model owns its handle.
        let caps = try JSONSerialization.jsonObject(with: Data(model.capabilities().utf8)) as! [String: Any]
        XCTAssertEqual(caps["min_choices"] as? Int, 2)
        XCTAssertNil(provider)
    }

    func testOfficialStructuredContractAndCapabilities() throws {
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
        let caps = try decode(model.capabilities())
        XCTAssertEqual(caps["max_choices"] as? Int, 255)
        XCTAssertEqual((caps["rounding"] as? [String: Any])?["score_decimals"] as? Int, 2)
        XCTAssertEqual(server.lastRequestBody, "")
        let request = fixture["request"] as! [String: Any]
        let result = try decode(model.decide(options: encode(request)))
        let answers = result["answers"] as! [String: Any]
        XCTAssertEqual((answers["urgent"] as? [String: Any])?["probability_true"] as? Double, 0.9)
        let questions = request["questions"] as! [[String: Any]]
        let actual = try decode(server.lastRequestBody)["questions"] as! [String: Any]
        XCTAssertEqual(server.lastRequestPath, "/v1/systemone")
        XCTAssertEqual((actual["urgent"] as! [String: Any])["criteria"] as? NSDictionary,
            questions[0]["criteria"] as? NSDictionary)
        XCTAssertEqual((answers["severity"] as! [String: Any])["levels"] as? NSArray,
            questions[2]["levels"] as? NSArray)
        XCTAssertThrowsError(try model.decide(options: "{"))
        model.close()
        XCTAssertThrowsError(try model.capabilities())
        XCTAssertThrowsError(try DecisionModel.jev(apiKey: "test", modelId: "jev-latest", probabilitySource: "unknown"))
    }

    func testOpenAIMediaAndTypedValues() throws {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        let fixture = try JSONSerialization.jsonObject(with: Data(contentsOf:
            root.appendingPathComponent("contract-tests/fixtures/decision-openai-full.json"))) as! [String: Any]
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
        let result = try JSONSerialization.jsonObject(with: Data(model.decide(options: encode(fixture["options"]!)).utf8)) as! [String: Any]
        let actual = try JSONSerialization.jsonObject(with: Data(server.lastRequestBody.utf8)) as! NSDictionary
        XCTAssertEqual(actual, fixture["request"] as! NSDictionary)
        XCTAssertEqual(server.lastRequestPath, "/v1/decisions")
        let answers = result["answers"] as! [String: Any]
        XCTAssertEqual((answers["choice"] as! [String: Any])["value"] as? Bool, true)
        let options = fixture["options"] as! [String: Any]
        let questions = options["questions"] as! [[String: Any]]
        XCTAssertEqual((answers["score"] as! [String: Any])["levels"] as? NSArray, questions[1]["levels"] as? NSArray)
    }
}
