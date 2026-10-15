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
    static func testToolCallProviderMetadataRoundTrip() throws {
        let original = ToolCall(
            toolCallId: "call_1",
            toolName: "get_weather",
            input: jv(#"{"location":"Tokyo"}"#),
            providerMetadata: ["openai": ["itemId": .string("item_1")]]
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
        try assertNil(missing.body)
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
        try assertEqual(file.bytes(), [104, 105])

        let output = GeneratedFileOutput(file: file, providerMetadata: ["google": ["thoughtSignature": .string("sig")]])
        let wire = #"{"type":"file","file":{"base64":"aGk=","mediaType":"text/plain"},"providerMetadata":{"google":{"thoughtSignature":"sig"}}}"#
        try assertEqual(try JSONDecoder().decode(TextStreamPart.self, from: Data(wire.utf8)), .file(output))
        try assertEqual(try JSONDecoder().decode(TextContent.self, from: Data(wire.utf8)), .file(output))

        let raw = #"{"type":"file","data":{"type":"url","url":"https://example.com/a.png"},"mediaType":"image/png"}"#
        try assertEqual(try JSONDecoder().decode(RawGenerateContent.self, from: Data(raw.utf8)),
                       .file(LanguageModelFile(data: .url(url: "https://example.com/a.png"), mediaType: "image/png")))
    }
#if !AIMUX_STANDALONE
    @Test
#endif
    static func testTextStreamToolPartsAndUsageAreCamelCase() throws {
        let cases: [(String, TextStreamPart)] = [
            (#"{"type":"tool-result","toolCallId":"srv-1","toolName":"web_search","input":{"q":"rust"},"output":"ok","providerExecuted":true}"#,
             .toolResult(TypedToolResult(toolCallId: "srv-1", toolName: "web_search", input: .object(["q": .string("rust")]),
                                         output: .string("ok"), providerExecuted: true, providerMetadata: nil,
                                         dynamic: nil, preliminary: nil))),
            (#"{"type":"tool-output-denied","toolCallId":"c","toolName":"t"}"#,
             .toolOutputDenied(ToolOutputDenied(toolCallId: "c", toolName: "t", providerExecuted: nil, dynamic: nil))),
            (#"{"type":"finish","finishReason":"stop","totalUsage":{"inputTokens":3,"inputTokenDetails":{"cacheReadTokens":1},"outputTokens":2,"outputTokenDetails":{},"totalTokens":5}}"#,
             .finish(finishReason: .stop, rawFinishReason: nil,
                     totalUsage: LanguageModelUsage(inputTokens: 3, inputTokenDetails: InputTokenDetails(cacheReadTokens: 1),
                                                    outputTokens: 2, totalTokens: 5))),
        ]
        for (wire, part) in cases {
            try assertEqual(try JSONDecoder().decode(TextStreamPart.self, from: Data(wire.utf8)), part)
            let encoded = try JSONSerialization.jsonObject(with: JSONEncoder().encode(part)) as? NSDictionary
            try assertEqual(encoded, try JSONSerialization.jsonObject(with: Data(wire.utf8)) as? NSDictionary)
        }
    }
#if !AIMUX_STANDALONE
    @Test
#endif
    static func testTextStreamDeltasAreText() throws {
        let cases: [(String, TextStreamPart)] = [
            (#"{"type":"text-delta","id":"t1","text":"Hello"}"#, .textDelta(id: "t1", text: "Hello", providerMetadata: nil)),
            (#"{"type":"reasoning-delta","id":"t1","text":"Hmm"}"#, .reasoningDelta(id: "t1", text: "Hmm", providerMetadata: nil)),
            (#"{"type":"tool-input-delta","id":"t1","delta":"{"}"#, .toolInputDelta(id: "t1", delta: "{", providerMetadata: nil)),
        ]
        for (wire, part) in cases {
            try assertEqual(try JSONDecoder().decode(TextStreamPart.self, from: Data(wire.utf8)), part)
            let encoded = try JSONSerialization.jsonObject(with: JSONEncoder().encode(part)) as? NSDictionary
            try assertEqual(encoded, try JSONSerialization.jsonObject(with: Data(wire.utf8)) as? NSDictionary)
        }
        try assertThrows(try JSONDecoder().decode(TextStreamPart.self, from: Data(#"{"type":"text-delta","id":"t1","delta":"Hello"}"#.utf8)))
    }
#if !AIMUX_STANDALONE
    @Test
#endif
    static func testFilePartsAreTheAiSdkFilePart() throws {
        let cases: [(String, ContentPart)] = [
            (#"{"type":"file","data":{"type":"url","url":"https://example.com/a.pdf"},"mediaType":"application/pdf"}"#,
             .file(data: .tagged(.url(url: "https://example.com/a.pdf")), mediaType: "application/pdf")),
            (#"{"type":"file","data":{"type":"data","data":"aGk="},"mediaType":"text/plain"}"#,
             .file(data: .tagged(.data(data: .base64("aGk="))), mediaType: "text/plain")),
            (#"{"type":"file","data":"aGk=","mediaType":"text/plain"}"#,
             .file(data: .bare("aGk="), mediaType: "text/plain")),
            (#"{"type":"file","data":{"openai":"file-abc"},"mediaType":"application/pdf"}"#,
             .file(data: .reference(["openai": "file-abc"]), mediaType: "application/pdf")),
            (#"{"type":"file","data":{"type":"reference","reference":{"openai":"file-abc"}},"mediaType":"application/pdf"}"#,
             .file(data: .tagged(.reference(reference: ["openai": "file-abc"])), mediaType: "application/pdf")),
            (#"{"type":"reasoning-file","data":"aGk=","mediaType":"image/png"}"#,
             .reasoningFile(data: .bare("aGk="), mediaType: "image/png")),
        ]
        for (wire, part) in cases {
            let decoded = try JSONDecoder().decode(ContentPart.self, from: Data(wire.utf8))
            try assertEqual(decoded, part)
            try assertEqual(try JSONDecoder().decode(ContentPart.self, from: try JSONEncoder().encode(part)), part)
        }
        for removed in ["file-base64", "file-url", "file-reference"] {
            let wire = #"{"type":"\#(removed)","data":"aGk=","url":"https://example.com/a.pdf","mediaType":"application/pdf"}"#
            try assertThrows(try JSONDecoder().decode(ContentPart.self, from: Data(wire.utf8)))
        }
    }
#if !AIMUX_STANDALONE
    @Test
#endif
    static func testToolChoiceCodableWire() throws {
        let pairs: [(ToolChoice, String)] = [
            (ToolChoice.auto, "\"auto\""),
            (ToolChoice.none, "\"none\""),
            (ToolChoice.required, "\"required\""),
        ]
        for (choice, wire) in pairs {
            let data = try JSONEncoder().encode(choice)
            try assertEqual(String(data: data, encoding: .utf8), wire)
            try assertEqual(try JSONDecoder().decode(ToolChoice.self, from: data), choice)
        }

        let tool = ToolChoice.tool(toolName: "get_weather")
        let data = try JSONEncoder().encode(tool)
        let obj = parseJSON(String(data: data, encoding: .utf8) ?? "")
        try assertEqual(obj["type"] as? String, "tool")
        try assertEqual(obj["toolName"] as? String, "get_weather")
        try assertEqual(try JSONDecoder().decode(ToolChoice.self, from: data), tool)
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
          "steps": [{"callId":"c1","stepNumber":0}]
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
        try assertNil(original.output)
        try assertEqual(original.response.id, "resp_1")
        try assertEqual(original.response.modelId, "gpt-4o")
        try assertEqual(original.warnings, [.other(message: "careful")])
        try assertEqual(original.reasoning, [.reasoning(ReasoningOutput(text: "hmm"))])
        guard case .parts(let toolContent) = original.responseMessages[1].content,
              case .toolResult(_, _, let output, _) = toolContent[0] else {
            throw CheckFailure(description: "expected a tool-result part, got \(original.responseMessages[1].content)")
        }
        try assertEqual(output, .json(.object(["temp": .number(22)])))

        // Encode → decode yields an equal value.
        let reencoded = try JSONEncoder().encode(original)
        let decoded = try JSONDecoder().decode(GenerateTextResult.self, from: reencoded)
        try assertEqual(original, decoded)
    }
}
private func assertEqual<T: Equatable>(_ lhs: T, _ rhs: T) throws {
    try require(lhs == rhs, "Values differ: \(lhs) versus \(rhs)")
}
private func assertNil<T>(_ value: T?) throws { try require(value == nil) }
private func assertThrows<T>(_ body: @autoclosure () throws -> T) throws {
    do { _ = try body() } catch { return }
    throw CheckFailure(description: "Expected decoding failure")
}
private func parseJSON(_ value: String) -> [String: Any] {
    (try? JSONSerialization.jsonObject(with: Data(value.utf8)) as? [String: Any]) ?? [:]
}
private func jv(_ value: String) -> JSONValue {
    try! JSONDecoder().decode(JSONValue.self, from: Data(value.utf8))
}
