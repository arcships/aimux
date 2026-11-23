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
        guard case .sourceError(let source)? = error.cause else {
            throw CheckFailure(description: "transport cause is not a SourceError: \(String(describing: error.cause))")
        }
        try require(source.message == "tcp connect error")
        try require(source.cause?.message == "Connection refused")
        try require(error.error["cause"]?["message"] == .string("tcp connect error"))
        try require(AimuxError.binding("x").cause == nil)
        let nested = try JSONDecoder().decode(AimuxError.self, from: Data(#"""
            {"kind":"aimux","code":11,"message":"m","error":{"name":"AI_APICallError","url":"https://example.test/v1",
             "requestBodyValues":{},"statusCode":200,"message":"m","isRetryable":false,
             "cause":{"name":"AI_JSONParseError","message":"Unexpected end of JSON input"}}}
            """#.utf8))
        guard case .aiMuxError(.aijsonParseError(_, let message))? = nested.cause else {
            throw CheckFailure(description: "nested cause is not an AiMuxError: \(String(describing: nested.cause))")
        }
        try require(message == "Unexpected end of JSON input")
        let retry = try JSONDecoder().decode(AimuxError.self, from: Data(#"""
            {"kind":"aimux","code":10,"message":"r","error":{"name":"AI_RetryError","message":"r","reason":"maxRetriesExceeded",
             "errors":[{"name":"AI_JSONParseError","message":"first"},{"name":"AI_JSONParseError","message":"last"}]}}
            """#.utf8))
        guard case .aiMuxError(.aijsonParseError(_, let last))? = retry.cause else {
            throw CheckFailure(description: "retry cause is not its last attempt: \(String(describing: retry.cause))")
        }
        try require(last == "last")
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
          "totalUsage": {"inputTokens":3,"inputTokenDetails":{},"outputTokens":2,"outputTokenDetails":{},"totalTokens":5},
          "steps": [{
            "callId":"c1", "stepNumber":0, "content":[
              {"type":"text","text":"hello"},
              {"type":"reasoning","text":"hmm"},
              {"type":"source","sourceType":"url","id":"s1","url":"https://example.com"},
              {"type":"tool-call","toolCallId":"call_1","toolName":"get_weather","input":{"location":"Tokyo"}}
            ], "finishReason":"tool-calls", "rawFinishReason":"tool_calls",
            "model":{"modelId":"gpt-4o","provider":"openai"},
            "performance":{"effectiveOutputTokensPerSecond":0,"effectiveTotalTokensPerSecond":0,
                           "responseTimeMs":0,"stepTimeMs":0,"toolExecutionMs":{}},
            "request":{},
            "response":{"id":"resp_1","timestamp":"2026-01-01T00:00:00.000Z","modelId":"gpt-4o","messages":[
              {"role":"assistant","content":"hello"},
              {"role":"tool","content":[
                {"type":"tool-result","toolCallId":"call_1","toolName":"get_weather",
                 "output":{"type":"json","value":{"temp":22}}}
              ]}
            ]},
            "usage":{"inputTokenDetails":{},"outputTokenDetails":{}},
            "warnings":[{"type":"other","message":"careful"}]
          }],
          "outputParsed": false
        }
        """
        let original = try JSONDecoder().decode(GenerateTextResult.self,
                                                from: json.data(using: .utf8)!)
        try assertEqual(original.text, "hello")
        try assertEqual(original.staticToolCalls[0].toolName, "get_weather")
        try assertEqual(original.finishReason, .toolCalls)
        try assertEqual(original.rawFinishReason, "tool_calls")
        try assertEqual(original.usage.totalTokens, 5)
        try assertEqual(original.reasoningText, "hmm")
        try assertEqual(original.output, .missing)
        try assertEqual(original.outputValue, .missing)
        try assertEqual(original.outputParsed, false)
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
        let encoded = try JSONDecoder().decode([String: JSONValue].self, from: reencoded)
        try assertEqual(Set(encoded.keys), ["steps", "totalUsage", "outputParsed"])

        var plainText = encoded
        plainText.removeValue(forKey: "outputParsed")
        let plain = try JSONDecoder().decode(GenerateTextResult.self, from: try JSONEncoder().encode(plainText))
        try assertEqual(plain.output, .value(.string("hello")))
        try assertEqual(plain.outputValue, .missing)

        var parsedNull = plainText
        parsedNull["output"] = .null
        let withNull = try JSONDecoder().decode(GenerateTextResult.self, from: try JSONEncoder().encode(parsedNull))
        try assertEqual(withNull.output, .null)
        try assertEqual(withNull.outputValue, .null)

        var contradictory = encoded
        contradictory["output"] = .null
        try assertThrows(JSONDecoder().decode(GenerateTextResult.self, from: try JSONEncoder().encode(contradictory)))

        var empty = encoded
        empty["steps"] = .array([])
        let malformed = try JSONEncoder().encode(empty)
        try assertThrows(JSONDecoder().decode(GenerateTextResult.self, from: malformed))
        let decoded = try JSONDecoder().decode(GenerateTextResult.self, from: reencoded)
        try assertEqual(original, decoded)
    }
#if !AIMUX_STANDALONE
    @Test
#endif
    static func testClosedTaggedVariantsRejectUnknownFields() throws {
        _ = try JSONDecoder().decode(BackendConfig.self, from: Data(#"{"type":"ring","capacity":1}"#.utf8))
        try assertThrows(try JSONDecoder().decode(BackendConfig.self, from: Data(#"{"type":"ring","capacity":1,"extra":true}"#.utf8)))
        try assertThrows(try JSONDecoder().decode(StreamFrame.self, from: Data(#"{"type":"end","seq":"0","outcome":"success","extra":true}"#.utf8)))
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
