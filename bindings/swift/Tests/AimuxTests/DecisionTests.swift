import XCTest
import Foundation
@testable import Aimux

final class DecisionTests: XCTestCase {
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
}
