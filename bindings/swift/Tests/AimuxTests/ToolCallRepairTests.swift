import Foundation
import XCTest
@testable import Aimux

final class ToolCallRepairTests: XCTestCase {
    func testSharedFixtureCases() throws {
        let url = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
            .deletingLastPathComponent().deletingLastPathComponent()
            .deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("contract-tests/fixtures/tool-call-repair.json")
        let document = try JSONSerialization.jsonObject(with: Data(contentsOf: url)) as! [String: Any]
        let cases = document["cases"] as! [[String: Any]]
        XCTAssertFalse(cases.isEmpty)

        for testCase in cases {
            let name = testCase["name"] as! String
            let input = testCase["input"] as! [String: Any]
            let options = input["opts"] is NSNull ? nil : json(input["opts"]!)
            let run: () throws -> String
            switch testCase["function"] as? String {
            case "tool_call_repair_context":
                run = { try toolCallRepairContext(toolCall: json(input["tool_call"]!),
                    prompt: json(input["prompt"]!), options: options) }
            case "apply_tool_call_repair":
                run = { try applyToolCallRepair(toolCall: json(input["tool_call"]!),
                    options: options, reply: json(input["reply"]!)) }
            case "apply_tool_call_repair_to_result":
                run = { try applyToolCallRepairToResult(result: json(input["result"]!),
                    options: options, toolCallId: input["tool_call_id"] as! String,
                    reply: json(input["reply"]!)) }
            default:
                XCTFail("unknown fixture function in \(name)")
                continue
            }
            if testCase["expected_error"] != nil {
                XCTAssertThrowsError(try run(), name) {
                    guard case .invalidArgument = $0 as? AimuxError else {
                        return XCTFail("\(name): expected invalidArgument, got \($0)")
                    }
                }
            } else {
                // `XCTAssertEqual` rethrows, so the throwing call cannot sit in its
                // argument. `.fragmentsAllowed`: the no-tools context case answers
                // with the bare JSON literal `null`.
                let actual = try JSONSerialization.jsonObject(with: Data(run().utf8), options: [.fragmentsAllowed])
                XCTAssertEqual(actual as! NSObject, testCase["expected"] as! NSObject, name)
            }
        }
    }

    func testGenerateRepairsRawInputAndTranscript() throws {
        let server = MockHTTPServer(response: .json(invalidResponse))
        try server.start(); defer { server.stop() }
        var rawInput: String?
        let result = try Model.openai(apiKey: "test", modelId: "gpt-4o", baseUrl: server.baseURL)
            .generateText(prompt: .text("weather?"), options: options { context in
                rawInput = context.toolCall.input
                return repaired(context)
            })
        XCTAssertEqual(rawInput, #"{"town":"Singapore"}"#)
        XCTAssertEqual(result.toolCalls[0].input["city"]?.stringValue, "Singapore")
        let transcript = result.responseMessages.compactMap { message -> JSONValue? in
            guard case .parts(let parts) = message.content else { return nil }
            return parts.compactMap { if case .toolCall(_, _, let input, _, _, _) = $0 { input } else { nil } }.first
        }.first
        XCTAssertEqual(transcript?["city"]?.stringValue, "Singapore")
    }

    func testGenerateAsOpenAICarriesRepairedArguments() throws {
        // A ChatCompletion has no invalid marker: the native result is
        // repaired, then converted.
        let server = MockHTTPServer(response: .json(invalidResponse))
        try server.start(); defer { server.stop() }
        let completion = try Model.openai(apiKey: "test", modelId: "gpt-4o", baseUrl: server.baseURL)
            .generateTextAsOpenAI(prompt: .text("weather?"), options: options(repaired))
        let call = completion.choices[0].message.toolCalls?.first
        XCTAssertEqual(call?.id, "call-1")
        XCTAssertEqual(call?.function.arguments, #"{"city":"Singapore"}"#)
    }

    func testStreamRepairsToolCallAndPreservesDeltas() throws {
        let parts = try streamParts(responses: [.sse(streamBody)]) { _ in repaired }
        XCTAssertEqual(parts.compactMap { if case .toolInputDelta(_, let value, _) = $0 { value } else { nil } }.joined(),
                       #"{"town":"Singapore"}"#)
        let call = parts.compactMap { part -> JSONValue? in
            if case .toolCall(_, _, let input, _, _, _, _, let invalid, _) = part, invalid != true { input } else { nil }
        }.first
        XCTAssertEqual(call?["city"]?.stringValue, "Singapore")
    }

    func testStreamRepairBridgesNestedAimuxGenerationOffRawThread() throws {
        var nestedError: Error?
        let parts = try streamParts(responses: [.sse(streamBody), .json(repairResponse)]) { server in
            let model = try! Model.openai(apiKey: "test", modelId: "gpt-4o", baseUrl: server.baseURL)
            return { context in
                do {
                    let fixed = try model.generateText(prompt: .text("fix"))
                    return RawToolCall(toolCallId: context.toolCall.toolCallId,
                                       toolName: context.toolCall.toolName, input: fixed.text)
                } catch { nestedError = error; throw error }
            }
        }
        XCTAssertNil(nestedError)
        XCTAssertTrue(parts.contains { if case .toolCall(_, _, let input, _, _, _, _, let invalid, _) = $0 {
            return invalid != true && input["city"]?.stringValue == "Singapore"
        } else { return false } })
    }

    /// The four host-loop branches around the repair closure, which the shared
    /// fixture (pure native functions only) never reaches.
    func testHostLoopBranchesAroundRepairClosure() throws {
        struct RepairUnavailable: LocalizedError {
            let errorDescription: String? = "repair model unavailable"
        }
        var calls = 0
        /// One `generateText` against a fresh server; the counter is re-armed
        /// per sub-case, as is the mock (it has no setter for its response).
        func run(_ response: [String: Any], tools: Bool = true,
                 _ repair: @escaping RepairToolCall) throws -> ToolCall {
            calls = 0
            let hooked: RepairToolCall = { calls += 1; return try repair($0) }
            let server = MockHTTPServer(response: .json(response))
            try server.start(); defer { server.stop() }
            return try Model.openai(apiKey: "test", modelId: "gpt-4o", baseUrl: server.baseURL)
                .generateText(prompt: .text("weather?"),
                              options: tools ? options(hooked)
                                             : GenerateTextOptions(repairToolCall: hooked))
                .toolCalls[0]
        }

        let thrown = try run(invalidResponse) { _ in throw RepairUnavailable() }
        XCTAssertEqual(calls, 1)
        XCTAssertEqual(thrown.invalid, true)
        XCTAssertEqual(thrown.error?["ToolCallRepair"]?["cause"]?["Other"]?.stringValue,
                       "repair model unavailable")
        XCTAssertNotNil(thrown.error?["ToolCallRepair"]?["original_error"]?["InvalidToolInput"])

        let unchanged = try run(invalidResponse) { _ in nil }
        XCTAssertEqual(calls, 1)
        XCTAssertEqual(unchanged.invalid, true)
        XCTAssertNil(unchanged.error?["ToolCallRepair"])
        XCTAssertNotNil(unchanged.error?["InvalidToolInput"])

        let valid = try run(validResponse) { _ in nil }
        XCTAssertEqual(calls, 0)
        XCTAssertNotEqual(valid.invalid, true)
        XCTAssertEqual(valid.input["city"]?.stringValue, "Singapore")

        let untooled = try run(invalidResponse, tools: false) { _ in nil }
        XCTAssertEqual(calls, 0)
        XCTAssertEqual(untooled.invalid, true)
    }
}

private func streamParts(responses: [MockResponse],
                         repair: (MockHTTPServer) -> RepairToolCall) throws -> [StreamPart] {
    let server = MockHTTPServer(responses: responses)
    try server.start(); defer { server.stop() }
    var parts: [StreamPart] = []; var failure: Error?
    try Model.openai(apiKey: "test", modelId: "gpt-4o", baseUrl: server.baseURL)
        .streamText(prompt: .text("weather?"), options: options(repair(server)),
                    onPart: { parts.append($0) }, onDone: {}, onError: { failure = $0 })
    if let failure { throw failure }
    return parts
}

private func options(_ repair: @escaping RepairToolCall) -> GenerateTextOptions {
    let schema = try! JSONDecoder().decode(JSONValue.self, from: Data(
        #"{"type":"object","properties":{"city":{"type":"string"}},"required":["city"],"additionalProperties":false}"#.utf8))
    return GenerateTextOptions(tools: [.function(FunctionTool(name: "weather", inputSchema: schema))],
                               repairToolCall: repair)
}

private func repaired(_ context: ToolCallRepairContext) -> RawToolCall? {
    RawToolCall(toolCallId: context.toolCall.toolCallId, toolName: context.toolCall.toolName,
                input: #"{"city":"Singapore"}"#)
}

private func json(_ value: Any) -> String {
    String(data: try! JSONSerialization.data(withJSONObject: value, options: [.fragmentsAllowed]), encoding: .utf8)!
}

private let invalidResponse: [String: Any] = ["id": "1", "model": "gpt-4o", "choices": [[
    "message": ["role": "assistant", "content": NSNull(), "tool_calls": [["id": "call-1", "type": "function",
        "function": ["name": "weather", "arguments": #"{"town":"Singapore"}"#]]]], "finish_reason": "tool_calls"]],
    "usage": ["prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2]]

private let validResponse: [String: Any] = ["id": "1", "model": "gpt-4o", "choices": [[
    "message": ["role": "assistant", "content": NSNull(), "tool_calls": [["id": "call-1", "type": "function",
        "function": ["name": "weather", "arguments": #"{"city":"Singapore"}"#]]]], "finish_reason": "tool_calls"]],
    "usage": ["prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2]]

private let repairResponse: [String: Any] = ["id": "2", "model": "gpt-4o", "choices": [[
    "message": ["role": "assistant", "content": #"{"city":"Singapore"}"#], "finish_reason": "stop"]],
    "usage": ["prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2]]

private let streamBody = [
    ["id": "1", "model": "gpt-4o", "choices": [["delta": ["role": "assistant", "tool_calls": [["index": 0,
        "id": "call-1", "type": "function", "function": ["name": "weather", "arguments": ""]]]]]]],
    ["id": "1", "model": "gpt-4o", "choices": [["delta": ["tool_calls": [["index": 0,
        "function": ["arguments": "{\"town\":"]]]]]]],
    ["id": "1", "model": "gpt-4o", "choices": [["delta": ["tool_calls": [["index": 0,
        "function": ["arguments": "\"Singapore\"}"]]]]]]],
    ["id": "1", "model": "gpt-4o", "choices": [["delta": [String: Any](), "finish_reason": "tool_calls"]],
        "usage": ["prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2]],
].map { "data: " + json($0) }.joined(separator: "\n\n") + "\n\ndata: [DONE]\n\n"
