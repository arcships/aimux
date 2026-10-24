import Foundation
@testable import Aimux
#if !AIMUX_STANDALONE
import Testing
#endif

// Existing wire checks retained through the ABI switch.
struct WireTests {
#if !AIMUX_STANDALONE
    @Test
#endif
    static func handleResults() throws {
        try require(try handleID(#"{"handle":"18446744073709551615","kind":"Provider"}"#) == UInt64.max)
        try require(try handleID(#""9007199254740993""#) == 9_007_199_254_740_993)
        for json in [#"{"handle":1}"#, #""0""#, #""01""#, #""18446744073709551616""#] {
            do { _ = try handleID(json); throw CheckFailure(description: "Invalid handle accepted") }
            catch is DecodingError {}
        }
    }

#if !AIMUX_STANDALONE
    @Test
#endif
    static func testErrorCauseIsThePayloadCause() throws {
        let error = try JSONDecoder().decode(AimuxError.self, from: Data(#"""
            {"kind":"aimux","code":11,"message":"API call error: Cannot connect","error":{"name":"AI_APICallError",
             "message":"Cannot connect","cause":{"message":"tcp connect error","cause":{"message":"Connection refused"}}}}
            """#.utf8))
        try require(error.cause?["message"] == .string("tcp connect error"))
        try require(error.cause?["cause"]?["message"] == .string("Connection refused"))
        try require(AimuxError.binding("x").cause == nil)
        let retry = try JSONDecoder().decode(AimuxError.self, from: Data(#"""
            {"kind":"aimux","code":10,"message":"r","error":{"name":"AI_RetryError","message":"r","reason":"maxRetriesExceeded",
             "errors":[{"name":"AI_APICallError","message":"first"},{"name":"AI_APICallError","message":"last"}]}}
            """#.utf8))
        try require(retry.cause?["message"] == .string("last"))
    }

#if !AIMUX_STANDALONE
    @Test
#endif
    static func testToolCallProviderMetadataRoundTrip() throws {
        let original = ToolCall(
            input: jv(#"{"location":"Tokyo"}"#),
            providerMetadata: ["openai": ["itemId": .string("item_1")]],
            toolCallId: "call_1", toolName: "get_weather"
        )

        let encoded = try JSONEncoder().encode(original)
        let wire = try JSONSerialization.jsonObject(with: encoded) as? [String: Any]
        let metadata = wire?["providerMetadata"] as? [String: Any]
        try require(metadata?["openai"] != nil)

        let decoded = try JSONDecoder().decode(ToolCall.self, from: encoded)
        try assertEqual(decoded.providerMetadata, original.providerMetadata)
    }
#if !AIMUX_STANDALONE
    @Test
#endif
    static func testUnknownFieldsKeepNullApartFromMissing() throws {
        let missing = try JSONDecoder().decode(ResponseInfo.self, from: Data("{}".utf8))
        try assertEqual(missing.body, .missing)
        let null = try JSONDecoder().decode(ResponseInfo.self, from: Data(#"{"body":null}"#.utf8))
        try assertEqual(null.body, .null)
        try assertEqual(String(decoding: try JSONEncoder().encode(null), as: UTF8.self), #"{"body":null}"#)
        let request = try JSONDecoder().decode(RequestInfo.self, from: Data(#"{"body":null}"#.utf8))
        try assertEqual(request.body, .null)
    }
#if !AIMUX_STANDALONE
    @Test
#endif
    static func testGeneratedFilesAreBase64AndMediaType() throws {
        let file = GeneratedFile(base64: "aGk=", mediaType: "text/plain")
        let decoded = try JSONDecoder().decode(GeneratedFile.self, from: Data(#"{"base64":"aGk=","mediaType":"text/plain"}"#.utf8))
        try assertEqual(decoded, file)
        try assertEqual(Data(base64Encoded: file.base64), Data([104, 105]))
        let wire = #"{"type":"file","file":{"base64":"aGk=","mediaType":"text/plain"},"providerMetadata":{"google":{"thoughtSignature":"sig"}}}"#
        try assertRoundTrip(TextStreamPart.self, wire)
        try assertRoundTrip(ContentPart.self, wire)
        let raw = #"{"type":"file","data":{"type":"url","url":"https://example.com/a.png"},"mediaType":"image/png"}"#
        try assertRoundTrip(GenerateContent.self, raw)
    }
#if !AIMUX_STANDALONE
    @Test
#endif
    static func testTextStreamToolPartsAndUsageAreCamelCase() throws {
        let cases = [
            #"{"type":"tool-result","toolCallId":"srv-1","toolName":"web_search","input":{"q":"rust"},"output":"ok","providerExecuted":true}"#,
            #"{"type":"tool-output-denied","toolCallId":"c","toolName":"t"}"#,
            #"{"type":"finish","finishReason":"stop","totalUsage":{"inputTokens":3,"inputTokenDetails":{"cacheReadTokens":1},"outputTokens":2,"outputTokenDetails":{},"totalTokens":5}}"#,
        ]
        for wire in cases { try assertRoundTrip(TextStreamPart.self, wire) }
    }
#if !AIMUX_STANDALONE
    @Test
#endif
    static func testTextStreamDeltasAreText() throws {
        let cases = [
            #"{"type":"text-delta","id":"t1","text":"Hello"}"#,
            #"{"type":"reasoning-delta","id":"t1","text":"Hmm"}"#,
            #"{"type":"tool-input-delta","id":"t1","delta":"{"}"#,
        ]
        for wire in cases { try assertRoundTrip(TextStreamPart.self, wire) }
        try assertThrows(try JSONDecoder().decode(TextStreamPart.self, from: Data(#"{"type":"text-delta","id":"t1","delta":"Hello"}"#.utf8)))
    }
#if !AIMUX_STANDALONE
    @Test
#endif
    static func testFilePartsAreTheAiSdkFilePart() throws {
        let cases = [
            #"{"type":"file","data":{"type":"url","url":"https://example.com/a.pdf"},"mediaType":"application/pdf"}"#,
            #"{"type":"file","data":{"type":"data","data":"aGk="},"mediaType":"text/plain"}"#,
            #"{"type":"file","data":"aGk=","mediaType":"text/plain"}"#,
            #"{"type":"file","data":{"openai":"file-abc"},"mediaType":"application/pdf"}"#,
            #"{"type":"file","data":{"type":"reference","reference":{"openai":"file-abc"}},"mediaType":"application/pdf"}"#,
        ]
        for wire in cases { try assertRoundTrip(UserContentPart.self, wire) }
        try assertRoundTrip(AssistantContentPart.self, #"{"type":"reasoning-file","data":"aGk=","mediaType":"image/png"}"#)
        for removed in ["file-base64", "file-url", "file-reference"] {
            let wire = #"{"type":"\#(removed)","data":"aGk=","url":"https://example.com/a.pdf","mediaType":"application/pdf"}"#
            try assertThrows(try JSONDecoder().decode(UserContentPart.self, from: Data(wire.utf8)))
        }
    }
#if !AIMUX_STANDALONE
    @Test
#endif
    static func testToolChoiceCodableWire() throws {
        for wire in [#""auto""#, #""none""#, #""required""#, #"{"type":"tool","toolName":"get_weather"}"#] {
            try assertRoundTrip(ToolChoice.self, wire)
        }
    }
#if !AIMUX_STANDALONE
    @Test
#endif
    static func testGenerateTextResultRoundTrip() throws {
        let json = """
        {
          "content": [
            {"type":"text","text":"hello"},
            {"type":"tool-call","toolCallId":"call_1","toolName":"get_weather","input":{"location":"Tokyo"}}
          ],
          "text": "hello",
          "reasoning": [{"type":"reasoning","text":"hmm"}],
          "reasoningText": "hmm",
          "files": [],
          "sources": [{"type":"source","sourceType":"url","id":"s1","url":"https://example.com"}],
          "toolCalls": [
            {"type":"tool-call","toolCallId":"call_1","toolName":"get_weather","input":{"location":"Tokyo"}}
          ],
          "toolResults": [],
          "finishReason": "tool-calls",
          "rawFinishReason": "tool_calls",
          "usage": {"inputTokens":3,"inputTokenDetails":{},"outputTokens":2,"outputTokenDetails":{},"totalTokens":5},
          "totalUsage": {"inputTokens":3,"inputTokenDetails":{},"outputTokens":2,"outputTokenDetails":{},"totalTokens":5},
          "warnings": [{"type":"other","message":"careful"}],
          "request": {},
          "response": {"id":"resp_1","timestamp":"2026-01-01T00:00:00.000Z","modelId":"gpt-4o","messages":[]},
          "responseMessages": [
            {"role":"assistant","content":[
              {"type":"tool-call","toolCallId":"call_1","toolName":"get_weather","input":{"location":"Tokyo"}}
            ]},
            {"role":"tool","content":[
              {"type":"tool-result","toolCallId":"call_1","toolName":"get_weather",
               "output":{"type":"json","value":{"temp":22}}}
            ]}
          ],
          "steps": [{
            "callId":"c1", "stepNumber":0, "content":[], "finishReason":"stop",
            "model":{"modelId":"gpt-4o","provider":"openai"},
            "performance":{"effectiveOutputTokensPerSecond":0,"effectiveTotalTokensPerSecond":0,
                           "responseTimeMs":0,"stepTimeMs":0,"toolExecutionMs":{}},
            "request":{},
            "response":{"id":"resp_1","timestamp":"2026-01-01T00:00:00.000Z","modelId":"gpt-4o","messages":[]},
            "usage":{"inputTokenDetails":{},"outputTokenDetails":{}}, "warnings":[]
          }]
        }
        """
        let original = try JSONDecoder().decode(GenerateTextResult.self,
                                                from: json.data(using: .utf8)!)
        try assertEqual(original.text, "hello")
        try assertEqual(original.toolCalls[0].toolName, "get_weather")
        try assertEqual(original.finishReason, .toolCalls)
        try assertEqual(original.rawFinishReason, "tool_calls")
        try assertEqual(original.usage.totalTokens, 5)
        try assertEqual(original.reasoningText, "hmm")
        try assertEqual(original.output, .missing)
        try assertEqual(original.response.id, "resp_1")
        try assertEqual(original.response.modelId, "gpt-4o")
        try assertEqual(original.warnings, [.other(message: "careful")])
        try assertEqual(original.reasoning, [.reasoning(providerMetadata: nil, text: "hmm")])
        guard case .tool(let toolContent, _) = original.responseMessages[1],
              case .toolResult(let output, _, _, _) = toolContent[0] else {
            throw CheckFailure(description: "Expected a tool-result part")
        }
        try assertEqual(output, .json(providerOptions: nil, value: .object(["temp": .integer(22)])))
        try assertEqual(original.steps[0].callId, "c1")

        // Encode → decode yields an equal value.
        let reencoded = try JSONEncoder().encode(original)
        let decoded = try JSONDecoder().decode(GenerateTextResult.self, from: reencoded)
        try assertEqual(original, decoded)
    }
}
private func assertEqual<T: Equatable>(_ lhs: T, _ rhs: T) throws {
    try require(lhs == rhs, "Values differ: \(lhs) versus \(rhs)")
}
private func assertThrows<T>(_ body: @autoclosure () throws -> T) throws {
    do { _ = try body() } catch { return }
    throw CheckFailure(description: "Expected decoding failure")
}
private func jv(_ value: String) -> JSONValue {
    try! JSONDecoder().decode(JSONValue.self, from: Data(value.utf8))
}

private func assertRoundTrip<T: Codable & Equatable>(_ type: T.Type, _ wire: String) throws {
    let decoded = try JSONDecoder().decode(type, from: Data(wire.utf8))
    let encoded = try JSONEncoder().encode(decoded)
    try assertEqual(try JSONDecoder().decode(JSONValue.self, from: encoded), jv(wire))
    try assertEqual(try JSONDecoder().decode(type, from: encoded), decoded)
}
