// WrapperTests.swift — tests for the typed Codable wrapper layer (Types.swift).
//
// Exercises the `Model` extension methods that take `ModelPrompt` /
// `GenerateTextOptions` and return `GenerateTextResult` / `StreamPart`,
// verifying the JSON↔Codable boundary is handled correctly end-to-end
// (Swift typed API → FFI → reqwest → MockHTTPServer). The raw string-based
// API is left untouched; these tests use only the typed layer.

import XCTest
@testable import Aimux

final class WrapperTests: XCTestCase {

    // MARK: - generateText: plain text result

    /// A plain string prompt returns a `GenerateTextResult` with `.text`,
    /// `.raw.content` (a `.text` content item), a unified `.stop` finish
    /// reason, and token usage decoded from the mock.
    func testTypedGenerateTextPlainText() throws {
        let server = MockHTTPServer(response: .json(openaiPlainResponse))
        try server.start()
        defer { server.stop() }

        let model = try Model.openai(
            apiKey: "test-key", modelId: "gpt-4o", baseUrl: server.baseURL
        )
        let result = try model.generateText(prompt: .text("What is Rust?"))

        XCTAssertEqual(result.text, "Rust is a systems programming language.")
        XCTAssertEqual(result.finishReason.unified, .stop)
        XCTAssertEqual(result.usage.inputTokens.total, 10)
        XCTAssertEqual(result.usage.outputTokens.total, 8)

        // raw.content carries a typed .text GenerateContent item.
        let textContent = result.raw.content.compactMap { part -> String? in
            if case .text(let t, _) = part { return t } else { return nil }
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
            providerMetadata: jv(#"{"openai":{"item_id":"item_1"}}"#)
        )

        let encoded = try JSONEncoder().encode(original)
        let wire = try JSONSerialization.jsonObject(with: encoded) as? [String: Any]
        let metadata = wire?["provider_metadata"] as? [String: Any]
        XCTAssertNotNil(metadata?["openai"])

        let decoded = try JSONDecoder().decode(ToolCall.self, from: encoded)
        XCTAssertEqual(decoded.providerMetadata, original.providerMetadata)
    }

    /// Passing a typed `tools` option yields a typed `ToolCall` in the result:
    /// `.toolCalls[0].toolName`, `.toolCallId`, and the structured
    /// `.raw.content` `.toolCall` variant.
    func testTypedGenerateTextToolCall() throws {
        let server = MockHTTPServer(response: .json(openaiToolCallResponse))
        try server.start()
        defer { server.stop() }

        let model = try Model.openai(
            apiKey: "test-key", modelId: "gpt-4o", baseUrl: server.baseURL
        )
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

        // Structured raw.content contains a ToolCall variant mirroring the call.
        let toolContents = result.raw.content.compactMap { part -> ToolCall? in
            if case .toolCall(let id, let name, let input, _, _, _) = part {
                return ToolCall(toolCallId: id, toolName: name, input: input)
            }
            return nil
        }
        XCTAssertEqual(toolContents.count, 1)
        XCTAssertEqual(toolContents[0].toolName, "get_weather")
        XCTAssertEqual(toolContents[0].toolCallId, "call_abc")
        // Raw content keeps the provider's argument text; the parsed object
        // lives on the top-level toolCalls (asserted above).
        XCTAssertEqual(toolContents[0].input.stringValue, "{\"location\":\"Tokyo\"}")
    }

    // MARK: - generateText: multi-role messages

    /// A `.messages` prompt with system + user roles reaches the provider
    /// request body verbatim.
    func testTypedGenerateTextMultiRoleMessages() throws {
        let server = MockHTTPServer(response: .json(openaiPlainResponse))
        try server.start()
        defer { server.stop() }

        let model = try Model.openai(
            apiKey: "test-key", modelId: "gpt-4o", baseUrl: server.baseURL
        )
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

        let model = try Model.openai(
            apiKey: "test-key", modelId: "gpt-4o", baseUrl: server.baseURL
        )
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

    // MARK: - streamText: typed StreamPart

    /// Streaming text deltas are delivered as typed `StreamPart.textDelta`
    /// values, reassembling into the full text.
    func testTypedStreamTextYieldsStreamParts() throws {
        let server = MockHTTPServer(response: .sse(sse(openaiStreamTextEvents)))
        try server.start()
        defer { server.stop() }

        let model = try Model.openai(
            apiKey: "test-key", modelId: "gpt-4o", baseUrl: server.baseURL
        )
        var parts: [StreamPart] = []
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
            if case .textDelta(_, let delta, _) = part { acc += delta }
        }
        XCTAssertEqual(text, "Hello world")

        // Every emitted part decoded into a concrete StreamPart variant.
        XCTAssertTrue(parts.contains { if case .textDelta = $0 { true } else { false } })
    }

    /// Streaming tool-call fragments are delivered as typed tool-related
    /// `StreamPart`s, including a final `.toolCall` carrying the tool name.
    func testTypedStreamTextToolCall() throws {
        let server = MockHTTPServer(response: .sse(sse(openaiStreamToolEvents)))
        try server.start()
        defer { server.stop() }

        let model = try Model.openai(
            apiKey: "test-key", modelId: "gpt-4o", baseUrl: server.baseURL
        )
        let options = GenerateTextOptions(tools: [
            .function(FunctionTool(
                name: "get_weather",
                inputSchema: jv(#"{"type":"object","properties":{"location":{"type":"string"}}}"#)
            ))
        ])
        var parts: [StreamPart] = []
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
        XCTAssertTrue(hasToolPart, "stream should contain a tool-related StreamPart")

        // The complete ToolCall part carries the tool name and structured input.
        let toolCall = parts.compactMap { part -> (String, JSONValue)? in
            if case .toolCall(_, let name, let input, _, _, _, _, _) = part { return (name, input) }
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

        let model = try Model.openai(
            apiKey: "test-key", modelId: "gpt-4o", baseUrl: server.baseURL
        )
        let parts = try await collect(model.streamTextAsync(prompt: .text("Say hello")))
        XCTAssertFalse(parts.isEmpty)
        let text = parts.reduce(into: "") { acc, part in
            if case .textDelta(_, let delta, _) = part { acc += delta }
        }
        XCTAssertEqual(text, "Hello world")
    }

    // MARK: - Codable round-trip

    /// A `GenerateTextResult` decoded from a representative JSON blob
    /// round-trips through encode → decode losslessly.
    func testGenerateTextResultRoundTrip() throws {
        let json = """
        {
          "text": "hello",
          "tool_calls": [
            {"tool_call_id":"call_1","tool_name":"get_weather","input":{"location":"Tokyo"}}
          ],
          "finish_reason": {"unified":"stop","raw":"stop"},
          "usage": {"input_tokens":{"total":3},"output_tokens":{"total":2}},
          "warnings": [],
          "reasoning": [],
          "reasoning_text": "",
          "sources": [],
          "files": [],
          "response_messages": [],
          "raw_finish_reason": "stop",
          "provider_metadata": null,
          "response": {"id":"resp_1","timestamp":null,"model_id":"gpt-4o"},
          "total_usage": {"input_tokens":{"total":3},"output_tokens":{"total":2}},
          "raw": {
            "content": [
              {"Text":{"text":"hello"}},
              {"ToolCall":{"tool_call_id":"call_1","tool_name":"get_weather","input":{"location":"Tokyo"}}}
            ],
            "finish_reason": {"unified":"tool-calls","raw":"tool_calls"},
            "usage": {"input_tokens":{"total":3},"output_tokens":{"total":2}},
            "warnings": [],
            "provider_metadata": null,
            "response": {"id":"resp_1","timestamp":null,"model_id":"gpt-4o"},
            "request_body": null,
            "response_headers": null
          }
        }
        """
        let original = try JSONDecoder().decode(GenerateTextResult.self,
                                                from: json.data(using: .utf8)!)
        XCTAssertEqual(original.text, "hello")
        XCTAssertEqual(original.toolCalls[0].toolName, "get_weather")
        XCTAssertEqual(original.finishReason.unified, .stop)
        XCTAssertEqual(original.raw.finishReason?.unified, .toolCalls)
        XCTAssertEqual(original.raw.response?.id, "resp_1")
        XCTAssertEqual(original.raw.response?.modelId, "gpt-4o")

        // Encode → decode yields an equal value.
        let reencoded = try JSONEncoder().encode(original)
        let decoded = try JSONDecoder().decode(GenerateTextResult.self, from: reencoded)
        XCTAssertEqual(original, decoded)
    }
}

// MARK: - Helpers

/// Drain an `AsyncThrowingStream<StreamPart, Error>` into an array (the mock stream completes
/// synchronously inside `streamText`, so this returns promptly).
private func collect(_ stream: AsyncThrowingStream<StreamPart, Error>) async throws -> [StreamPart] {
    var parts: [StreamPart] = []
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
