// WrapperTests.swift — tests for the typed Codable wrapper layer (Types.swift).
//
// Exercises the `Model` extension methods that take `ModelPrompt` /
// `GenerateTextOptions` and return `GenerateTextResult` / `TextStreamPart`,
// verifying the JSON↔Codable boundary is handled correctly end-to-end
// (Swift typed API → FFI → reqwest → MockHTTPServer). The raw string-based
// API is left untouched; these tests use only the typed layer.

import XCTest
@testable import Aimux

final class WrapperTests: XCTestCase {

    // MARK: - generateText: plain text result

    /// A plain string prompt returns a `GenerateTextResult` with `.text`,
    /// `.content` (a `Text` content item), a unified `.stop` finish reason,
    /// and token usage decoded from the mock.
    func testTypedGenerateTextPlainText() throws {
        let server = MockHTTPServer(response: .json(openaiPlainResponse))
        try server.start()
        defer { server.stop() }

        let model = try chatModel(server.baseURL)
        let result = try model.generateText(prompt: .text("What is Rust?"))

        XCTAssertEqual(result.text, "Rust is a systems programming language.")
        XCTAssertEqual(result.finishReason, .stop)
        XCTAssertEqual(result.usage.inputTokens, 10)
        XCTAssertEqual(result.usage.outputTokens, 8)
        XCTAssertEqual(result.steps.count, 1)

        // content carries a `Text` content item.
        let textContent = result.content.compactMap { part -> String? in
            part["Text"]?["text"]?.stringValue
        }
        XCTAssertEqual(textContent, ["Rust is a systems programming language."])

        XCTAssertEqual(server.lastRequestPath, "/chat/completions")
    }

    // MARK: - generateText: tool calls

    func testToolCallProviderMetadataRoundTrip() throws {
        let original = ToolCall(
            toolCallId: "call_1",
            toolName: "get_weather",
            input: jv(#"{"location":"Tokyo"}"#),
            providerMetadata: ["openai": ["itemId": .string("item_1")]]
        )

        let encoded = try JSONEncoder().encode(original)
        let wire = try JSONSerialization.jsonObject(with: encoded) as? [String: Any]
        let metadata = wire?["providerMetadata"] as? [String: Any]
        XCTAssertNotNil(metadata?["openai"])

        let decoded = try JSONDecoder().decode(ToolCall.self, from: encoded)
        XCTAssertEqual(decoded.providerMetadata, original.providerMetadata)
    }

    /// Passing a typed `tools` option yields a typed `ToolCall` in the result:
    /// `.toolCalls[0].toolName`, `.toolCallId`, and the `ToolCall` variant
    /// of `.content`.
    func testTypedGenerateTextToolCall() throws {
        let server = MockHTTPServer(response: .json(openaiToolCallResponse))
        try server.start()
        defer { server.stop() }

        let model = try chatModel(server.baseURL)
        let options = GenerateTextOptions(tools: [
            .function(FunctionTool(
                name: "get_weather",
                inputSchema: jv(#"{"type":"object","properties":{"location":{"type":"string"}},"required":["location"]}"#),
                description: "Get weather for a location"
            ))
        ])
        let result = try model.generateText(
            prompt: .text("What's the weather in Tokyo?"), options: options
        )

        // Convenience tool_calls array (decoded from the top-level field).
        XCTAssertEqual(result.toolCalls.count, 1)
        XCTAssertEqual(result.toolCalls[0].toolName, "get_weather")
        XCTAssertEqual(result.toolCalls[0].toolCallId, "call_abc")
        XCTAssertEqual(result.toolCalls[0].input["location"]?.stringValue, "Tokyo")

        // content contains a tool-call item with the parsed input.
        let toolContents = result.content.filter { $0["type"]?.stringValue == "tool-call" }
        XCTAssertEqual(toolContents.count, 1)
        XCTAssertEqual(toolContents[0]["toolName"]?.stringValue, "get_weather")
        XCTAssertEqual(toolContents[0]["toolCallId"]?.stringValue, "call_abc")
        XCTAssertEqual(toolContents[0]["input"]?["location"]?.stringValue, "Tokyo")
        XCTAssertEqual(result.finishReason, .toolCalls)
    }

    // MARK: - generateText: multi-role messages

    /// A `.messages` prompt with system + user roles reaches the provider
    /// request body verbatim.
    func testTypedGenerateTextMultiRoleMessages() throws {
        let server = MockHTTPServer(response: .json(openaiPlainResponse))
        try server.start()
        defer { server.stop() }

        let model = try chatModel(server.baseURL)
        let result = try model.generateText(prompt: .messages([
            .system("You are a helpful assistant."),
            .user("What is Rust?"),
        ]))
        XCTAssertEqual(result.text, "Rust is a systems programming language.")

        // The full multi-role message sequence reaches the provider request.
        let reqBody = parseJSON(server.lastRequestBody)
        let msgs = (reqBody["messages"] as? [[String: Any]]) ?? []
        XCTAssertEqual(msgs.count, 2)
        XCTAssertEqual(msgs[0]["role"] as? String, "system")
        XCTAssertEqual(msgs[0]["content"] as? String, "You are a helpful assistant.")
        XCTAssertEqual(msgs[1]["role"] as? String, "user")
        XCTAssertEqual(msgs[1]["content"] as? String, "What is Rust?")
    }

    // MARK: - generateText: tool_choice

    /// A typed `toolChoice: .required` reaches the provider request body as
    /// the bare string `"required"`.
    func testTypedGenerateTextToolChoiceRequired() throws {
        let server = MockHTTPServer(response: .json(openaiToolCallResponse))
        try server.start()
        defer { server.stop() }

        let model = try chatModel(server.baseURL)
        let options = GenerateTextOptions(
            tools: [.function(FunctionTool(
                name: "get_weather",
                inputSchema: jv(#"{"type":"object","properties":{"location":{"type":"string"}}}"#)
            ))],
            toolChoice: .required
        )
        // Must not throw.
        _ = try model.generateText(prompt: .text("Hello"), options: options)

        let reqBody = parseJSON(server.lastRequestBody)
        XCTAssertEqual(reqBody["tool_choice"] as? String, "required")
    }

    /// `ToolChoice` encodes to the aimux wire shape and round-trips through
    /// `JSONEncoder`/`JSONDecoder`: bare strings for the unit variants, and
    /// `{"type":"tool","toolName":...}` (camelCase `toolName`) for the named
    /// tool. (The provider later remaps the named form to its own wire shape,
    /// so we validate the wrapper's encoding directly here.)
    /// `body?: unknown` (provider language-model-v4-generate-result.ts:41, :56)
    /// and `inputSchemaInput?: unknown` (provider-utils
    /// types/tool-approval-request.ts:41): an explicit null is kept apart from
    /// a missing value.
    func testUnknownFieldsKeepNullApartFromMissing() throws {
        let missing = try JSONDecoder().decode(ResponseInfo.self, from: Data("{}".utf8))
        XCTAssertNil(missing.body)
        let null = try JSONDecoder().decode(ResponseInfo.self, from: Data(#"{"body":null}"#.utf8))
        XCTAssertEqual(null.body, .null)
        XCTAssertEqual(String(decoding: try JSONEncoder().encode(null), as: UTF8.self), #"{"body":null}"#)
        let request = try JSONDecoder().decode(RequestInfo.self, from: Data(#"{"body":null}"#.utf8))
        XCTAssertEqual(request.body, .null)
    }

    /// GeneratedFile is {base64, mediaType, providerMetadata?} (ai
    /// generate-text/generated-file.ts:10-32); a text file part is
    /// {type, file, providerMetadata?} (convert-language-model-content.ts:55-70);
    /// the provider's LanguageModelV4File keeps {type, data, mediaType}.
    func testGeneratedFilesAreBase64AndMediaType() throws {
        let file = GeneratedFile(base64: "aGk=", mediaType: "text/plain")
        let decoded = try JSONDecoder().decode(GeneratedFile.self, from: Data(#"{"base64":"aGk=","mediaType":"text/plain"}"#.utf8))
        XCTAssertEqual(decoded, file)
        XCTAssertEqual(file.bytes(), [104, 105])

        let output = GeneratedFileOutput(file: file, providerMetadata: ["google": ["thoughtSignature": .string("sig")]])
        let wire = #"{"type":"file","file":{"base64":"aGk=","mediaType":"text/plain"},"providerMetadata":{"google":{"thoughtSignature":"sig"}}}"#
        XCTAssertEqual(try JSONDecoder().decode(TextStreamPart.self, from: Data(wire.utf8)), .file(output))
        XCTAssertEqual(try JSONDecoder().decode(TextContent.self, from: Data(wire.utf8)), .file(output))

        let raw = #"{"type":"file","data":{"type":"url","url":"https://example.com/a.png"},"mediaType":"image/png"}"#
        XCTAssertEqual(try JSONDecoder().decode(RawGenerateContent.self, from: Data(raw.utf8)),
                       .file(LanguageModelFile(data: .url(url: "https://example.com/a.png"), mediaType: "image/png")))
    }

    /// The text stream's tool parts are the AI SDK's TypedToolResult and
    /// TypedToolOutputDenied, camelCase (ai generate-text/tool-result.ts:10-42,
    /// tool-output-denied.ts:7-15), and a finish part's totalUsage is its
    /// LanguageModelUsage (types/usage.ts:10-67): camelCase counts, an unknown
    /// count absent.
    func testTextStreamToolPartsAndUsageAreCamelCase() throws {
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
            XCTAssertEqual(try JSONDecoder().decode(TextStreamPart.self, from: Data(wire.utf8)), part)
            let encoded = try JSONSerialization.jsonObject(with: JSONEncoder().encode(part)) as? NSDictionary
            XCTAssertEqual(encoded, try JSONSerialization.jsonObject(with: Data(wire.utf8)) as? NSDictionary)
        }
    }

    /// The text stream's text-delta and reasoning-delta carry `text` (ai
    /// generate-text/stream-text-result.ts:435-440, :466-471); a
    /// tool-input-delta keeps `delta` (:496-501).
    func testTextStreamDeltasAreText() throws {
        let cases: [(String, TextStreamPart)] = [
            (#"{"type":"text-delta","id":"t1","text":"Hello"}"#, .textDelta(id: "t1", text: "Hello", providerMetadata: nil)),
            (#"{"type":"reasoning-delta","id":"t1","text":"Hmm"}"#, .reasoningDelta(id: "t1", text: "Hmm", providerMetadata: nil)),
            (#"{"type":"tool-input-delta","id":"t1","delta":"{"}"#, .toolInputDelta(id: "t1", delta: "{", providerMetadata: nil)),
        ]
        for (wire, part) in cases {
            XCTAssertEqual(try JSONDecoder().decode(TextStreamPart.self, from: Data(wire.utf8)), part)
            let encoded = try JSONSerialization.jsonObject(with: JSONEncoder().encode(part)) as? NSDictionary
            XCTAssertEqual(encoded, try JSONSerialization.jsonObject(with: Data(wire.utf8)) as? NSDictionary)
        }
        XCTAssertThrowsError(try JSONDecoder().decode(TextStreamPart.self, from: Data(#"{"type":"text-delta","id":"t1","delta":"Hello"}"#.utf8)))
    }

    /// A result's toolCalls and sources are the AI SDK's TypedToolCall and
    /// Source, tagged {type: 'tool-call'} (ai generate-text/tool-call.ts:8-14)
    /// and {type: 'source', sourceType} (provider language-model-v4-source.ts:8,
    /// :36); generate-text-result.ts:73, :78. A repair function's call is a
    /// tagged LanguageModelV4ToolCall (language-model-v4-tool-call.ts:7).
    func testStandaloneToolCallsAndSourcesAreTagged() throws {
        func object(_ data: Data) throws -> NSDictionary? {
            try JSONSerialization.jsonObject(with: data) as? NSDictionary
        }
        let call = #"{"type":"tool-call","toolCallId":"call-1","toolName":"weather","input":{"city":"SG"}}"#
        let decoded = try JSONDecoder().decode(ToolCall.self, from: Data(call.utf8))
        XCTAssertEqual(try object(JSONEncoder().encode(decoded)), try object(Data(call.utf8)))
        let raw = RawToolCall(toolCallId: "c", toolName: "t", input: "{}")
        XCTAssertEqual(try object(JSONEncoder().encode(raw)),
                       try object(Data(#"{"type":"tool-call","toolCallId":"c","toolName":"t","input":"{}"}"#.utf8)))
        let url = #"{"type":"source","sourceType":"url","id":"s1","url":"https://example.com"}"#
        let source = try JSONDecoder().decode(Source.self, from: Data(url.utf8))
        XCTAssertEqual(try object(JSONEncoder().encode(source)), try object(Data(url.utf8)))
        // In a stream part the union's tag and the source's agree.
        let part = try JSONDecoder().decode(TextStreamPart.self, from: Data(url.utf8))
        XCTAssertEqual(try object(JSONEncoder().encode(part)), try object(Data(url.utf8)))

        // JSON written before the tag still reads; another tag is rejected.
        XCTAssertNoThrow(try JSONDecoder().decode(ToolCall.self, from: Data(#"{"toolCallId":"c","toolName":"t","input":{}}"#.utf8)))
        XCTAssertThrowsError(try JSONDecoder().decode(
            ToolCall.self, from: Data(#"{"type":"source","toolCallId":"c","toolName":"t","input":{}}"#.utf8)))

        // The native repair function writes the standalone call tagged.
        let invalid = #"{"toolCallId":"call-1","toolName":"weather","input":{"town":"SG"},"dynamic":true,"invalid":true,"error":{"name":"AI_InvalidToolInputError","toolName":"weather","toolInput":"{\"town\":\"SG\"}","cause":"missing city"}}"#
        let options = #"{"tools":[{"type":"function","name":"weather","inputSchema":{"type":"object"}}]}"#
        let repaired = try applyToolCallRepair(toolCall: invalid, options: options, reply: #"{"type":"unchanged"}"#)
        XCTAssertTrue(repaired.hasPrefix(#"{"type":"tool-call","toolCallId":"call-1""#), repaired)
    }

    /// FilePart.data is FileData | DataContent | URL | ProviderReference
    /// (provider-utils types/content-part.ts:75, file-data.ts:44-48);
    /// ReasoningFilePart.data is tagged data or a URL, or a string (:164).
    /// There is no file-base64 / file-url / file-reference part.
    func testFilePartsAreTheAiSdkFilePart() throws {
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
            XCTAssertEqual(decoded, part)
            XCTAssertEqual(try JSONDecoder().decode(ContentPart.self, from: try JSONEncoder().encode(part)), part)
        }
        for removed in ["file-base64", "file-url", "file-reference"] {
            let wire = #"{"type":"\#(removed)","data":"aGk=","url":"https://example.com/a.pdf","mediaType":"application/pdf"}"#
            XCTAssertThrowsError(try JSONDecoder().decode(ContentPart.self, from: Data(wire.utf8)))
        }
    }

    func testToolChoiceCodableWire() throws {
        let pairs: [(ToolChoice, String)] = [
            (ToolChoice.auto, "\"auto\""),
            (ToolChoice.none, "\"none\""),
            (ToolChoice.required, "\"required\""),
        ]
        for (choice, wire) in pairs {
            let data = try JSONEncoder().encode(choice)
            XCTAssertEqual(String(data: data, encoding: .utf8), wire)
            XCTAssertEqual(try JSONDecoder().decode(ToolChoice.self, from: data), choice)
        }

        let tool = ToolChoice.tool(toolName: "get_weather")
        let data = try JSONEncoder().encode(tool)
        let obj = parseJSON(String(data: data, encoding: .utf8) ?? "")
        XCTAssertEqual(obj["type"] as? String, "tool")
        XCTAssertEqual(obj["toolName"] as? String, "get_weather")
        XCTAssertEqual(try JSONDecoder().decode(ToolChoice.self, from: data), tool)
    }

    // MARK: - streamText: typed TextStreamPart

    /// Streaming text deltas are delivered as typed `TextStreamPart.textDelta`
    /// values, reassembling into the full text.
    func testTypedStreamTextYieldsStreamParts() throws {
        let server = MockHTTPServer(response: .sse(sse(openaiStreamTextEvents)))
        try server.start()
        defer { server.stop() }

        let model = try chatModel(server.baseURL)
        var parts: [TextStreamPart] = []
        var streamErr: (any Error)?
        model.streamText(
            prompt: .text("Say hello"),
            onPart: { parts.append($0) },
            onDone: {},
            onError: { streamErr = $0 }
        )
        XCTAssertNil(streamErr)
        XCTAssertFalse(parts.isEmpty)

        // The text deltas reassemble to "Hello world".
        let text = parts.reduce(into: "") { acc, part in
            if case .textDelta(_, let text, _) = part { acc += text }
        }
        XCTAssertEqual(text, "Hello world")

        // Every emitted part decoded into a concrete TextStreamPart variant;
        // the stream opens with start and start-step and closes with
        // finish-step and finish.
        XCTAssertTrue(parts.contains { if case .textDelta = $0 { true } else { false } })
        XCTAssertEqual(parts.first, .start)
        if case .startStep = parts[1] {} else { XCTFail("expected startStep, got \(parts[1])") }
        if case .finishStep = parts[parts.count - 2] {} else { XCTFail("expected finishStep") }
        if case .finish(let reason, _, _) = parts[parts.count - 1] {
            XCTAssertEqual(reason, .stop)
        } else {
            XCTFail("expected finish")
        }
    }

    /// Streaming tool-call fragments are delivered as typed tool-related
    /// `TextStreamPart`s, including a final `.toolCall` carrying the tool name.
    func testTypedStreamTextToolCall() throws {
        let server = MockHTTPServer(response: .sse(sse(openaiStreamToolEvents)))
        try server.start()
        defer { server.stop() }

        let model = try chatModel(server.baseURL)
        let options = GenerateTextOptions(tools: [
            .function(FunctionTool(
                name: "get_weather",
                inputSchema: jv(#"{"type":"object","properties":{"location":{"type":"string"}}}"#)
            ))
        ])
        var parts: [TextStreamPart] = []
        var streamErr: (any Error)?
        model.streamText(
            prompt: .text("What's the weather?"),
            options: options,
            onPart: { parts.append($0) },
            onDone: {},
            onError: { streamErr = $0 }
        )
        XCTAssertNil(streamErr)
        XCTAssertFalse(parts.isEmpty)

        // At least one tool-related part was emitted.
        let hasToolPart = parts.contains { part in
            switch part {
            case .toolInputStart, .toolInputDelta, .toolInputEnd, .toolCall, .toolResult:
                return true
            default: return false
            }
        }
        XCTAssertTrue(hasToolPart, "stream should contain a tool-related TextStreamPart")

        // The complete ToolCall part carries the tool name and structured input.
        let toolCall = parts.compactMap { part -> (String, JSONValue)? in
            if case .toolCall(let call) = part { return (call.toolName, call.input) }
            return nil
        }.first
        XCTAssertEqual(toolCall?.0, "get_weather")
    }

    /// The async-sequence typed wrapper yields the same typed parts as the
    /// callback form.
    func testTypedStreamTextAsyncYieldsStreamParts() async throws {
        let server = MockHTTPServer(response: .sse(sse(openaiStreamTextEvents)))
        try server.start()
        defer { server.stop() }

        let model = try chatModel(server.baseURL)
        let parts = try await collect(model.streamTextAsync(prompt: .text("Say hello")))
        XCTAssertFalse(parts.isEmpty)
        let text = parts.reduce(into: "") { acc, part in
            if case .textDelta(_, let text, _) = part { acc += text }
        }
        XCTAssertEqual(text, "Hello world")
    }

    // MARK: - Codable round-trip

    /// A `GenerateTextResult` decoded from a representative JSON blob
    /// round-trips through encode → decode losslessly.
    func testGenerateTextResultRoundTrip() throws {
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
        XCTAssertEqual(original.text, "hello")
        XCTAssertEqual(original.toolCalls[0].toolName, "get_weather")
        XCTAssertEqual(original.finishReason, .toolCalls)
        XCTAssertEqual(original.rawFinishReason, "tool_calls")
        XCTAssertEqual(original.usage.totalTokens, 5)
        XCTAssertEqual(original.reasoningText, "hmm")
        XCTAssertNil(original.output)
        XCTAssertEqual(original.response.id, "resp_1")
        XCTAssertEqual(original.response.modelId, "gpt-4o")
        XCTAssertEqual(original.warnings, [.other(message: "careful")])
        XCTAssertEqual(original.reasoning, [.reasoning(ReasoningOutput(text: "hmm"))])
        guard case .parts(let toolContent) = original.responseMessages[1].content,
              case .toolResult(_, _, let output, _) = toolContent[0] else {
            return XCTFail("expected a tool-result part, got \(original.responseMessages[1].content)")
        }
        XCTAssertEqual(output, .json(.object(["temp": .number(22)])))

        // Encode → decode yields an equal value.
        let reencoded = try JSONEncoder().encode(original)
        let decoded = try JSONDecoder().decode(GenerateTextResult.self, from: reencoded)
        XCTAssertEqual(original, decoded)
    }
}

// MARK: - Helpers

/// Drain an `AsyncThrowingStream<TextStreamPart, Error>` into an array (the mock stream completes
/// synchronously inside `streamText`, so this returns promptly).
private func collect(_ stream: AsyncThrowingStream<TextStreamPart, Error>) async throws -> [TextStreamPart] {
    var parts: [TextStreamPart] = []
    for try await part in stream { parts.append(part) }
    return parts
}

/// Parse a JSON object string into `[String: Any]` (empty on failure).
private func parseJSON(_ s: String) -> [String: Any] {
    guard let data = s.data(using: .utf8),
          let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
        return [:]
    }
    return obj
}

/// Build a `JSONValue` from a JSON string (round-trips through the type's
/// own Codable, so it exercises the same decode path as the wrapper).
private func jv(_ json: String) -> JSONValue {
    try! JSONDecoder().decode(JSONValue.self, from: json.data(using: .utf8)!)
}

/// Build an SSE body from a list of `data:` JSON events, terminated by [DONE].
private func sse(_ events: [[String: Any]]) -> String {
    let body = events.map { ev -> String in
        let data = try! JSONSerialization.data(withJSONObject: ev)
        return "data: " + (String(data: data, encoding: .utf8) ?? "")
    }.joined(separator: "\n\n")
    return body + "\n\n" + "data: [DONE]\n\n"
}

// MARK: - Mock response fixtures (same shape as AimuxTests / Node / Rust e2e)

private let openaiPlainResponse: [String: Any] = [
    "id": "chatcmpl-test",
    "model": "gpt-4o",
    "choices": [[
        "message": ["role": "assistant", "content": "Rust is a systems programming language."],
        "finish_reason": "stop",
    ]],
    "usage": ["prompt_tokens": 10, "completion_tokens": 8, "total_tokens": 18],
]

private let openaiToolCallResponse: [String: Any] = [
    "id": "chatcmpl-tc",
    "model": "gpt-4o",
    "choices": [[
        "message": [
            "role": "assistant",
            "content": NSNull(),
            "tool_calls": [[
                "id": "call_abc",
                "type": "function",
                "function": ["name": "get_weather", "arguments": "{\"location\":\"Tokyo\"}"],
            ]],
        ],
        "finish_reason": "tool_calls",
    ]],
    "usage": ["prompt_tokens": 20, "completion_tokens": 10, "total_tokens": 30],
]

/// OpenAI SSE text deltas → "Hello world".
private let openaiStreamTextEvents: [[String: Any]] = [
    ["id": "1", "model": "gpt-4o", "choices": [["delta": ["content": "Hello"]]]],
    ["id": "1", "model": "gpt-4o", "choices": [["delta": ["content": " world"]]]],
    [
        "id": "1", "model": "gpt-4o",
        "choices": [["delta": [String: Any](), "finish_reason": "stop"]],
        "usage": ["prompt_tokens": 5, "completion_tokens": 2, "total_tokens": 7],
    ],
]

/// OpenAI SSE tool-call deltas (incremental argument JSON).
private let openaiStreamToolEvents: [[String: Any]] = [
    [
        "id": "1", "model": "gpt-4o",
        "choices": [["delta": [
            "role": "assistant",
            "tool_calls": [[
                "index": 0, "id": "call_xyz", "type": "function",
                "function": ["name": "get_weather", "arguments": ""],
            ]],
        ]]],
    ],
    [
        "id": "1", "model": "gpt-4o",
        "choices": [["delta": [
            "tool_calls": [[
                "index": 0,
                "function": ["arguments": "{\"location\":\"Tokyo\"}"],
            ]],
        ]]],
    ],
    [
        "id": "1", "model": "gpt-4o",
        "choices": [["delta": [String: Any](), "finish_reason": "tool_calls"]],
        "usage": ["prompt_tokens": 5, "completion_tokens": 2, "total_tokens": 7],
    ],
]
