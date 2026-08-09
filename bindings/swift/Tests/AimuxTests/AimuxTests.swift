import XCTest
@testable import Aimux

final class AimuxTests: XCTestCase {

    func testModelCreation() throws {
        // Even with a fake key, the provider should construct
        // (key is validated on first API call, not construction).
        let model = try Model.openai(apiKey: "sk-test-fake-key", modelId: "gpt-4o-mini")
        XCTAssertNotNil(model)
    }

    func testAnthropicModelCreation() throws {
        let model = try Model.anthropic(
            apiKey: "sk-ant-test-fake-key",
            modelId: "claude-3-5-sonnet-20241022"
        )
        XCTAssertNotNil(model)
    }

    func testProviderModelCreation() throws {
        // Registry-backed construction: deepseek is in the provider registry
        // (key is validated on first API call, not construction).
        let model = try Model.provider(
            name: "deepseek", apiKey: "sk-test-fake-key", modelId: "deepseek-chat"
        )
        XCTAssertNotNil(model)
    }

    func testProviderModelCreationWithProviderName() throws {
        // Recommended typed spelling: ProviderName enum case (key is
        // validated on the first API call, not construction).
        let model = try Model.provider(
            name: ProviderName.groq.rawValue, apiKey: "sk-test-fake-key", modelId: "llama-3.3-70b"
        )
        XCTAssertNotNil(model)
    }

    func testInvalidPromptThrows() throws {
        let model = try Model.openai(apiKey: "sk-test-fake-key", modelId: "gpt-4o-mini")
        XCTAssertThrowsError(try model.generateText(prompt: "{invalid json}"))
    }

    func testUnknownProviderErrorValue() {
        XCTAssertThrowsError(try Model.provider(
            name: "no-such-provider", apiKey: "sk-test-fake-key", modelId: "whatever"
        )) { error in
            guard let e = error as? AimuxError else {
                return XCTFail("expected AimuxError, got \(error)")
            }
            guard case .unknownProvider = e else {
                return XCTFail("expected .unknownProvider, got \(e)")
            }
            // errorValue is the raw externally-tagged core AiMuxError JSON.
            XCTAssertTrue(e.errorValue?.contains("UnknownProvider") == true,
                          "errorValue should contain UnknownProvider, got \(e.errorValue ?? "nil")")
        }
    }

    func testStreamTextReturnsAsyncSequence() throws {
        let model = try Model.openai(apiKey: "sk-test-fake-key", modelId: "gpt-4o-mini")
        let stream = model.streamTextAsync(prompt: "\"hello\"")
        // AsyncStream is created — we don't consume it (would need network).
        XCTAssertNotNil(stream)
    }

    // MARK: - Recording ring (T10)

    /// `initRecordingRing(cap: 0)` must throw instead of silently ignoring the
    /// C ABI's -1 return (T10). Mirrors Kotlin/Java (throw) and Flutter
    /// (surface the C error). cap == 0 is rejected by the C ABI without side
    /// effects, so this does not mutate global recorder state.
    func testInitRecordingRingRejectsZeroCap() {
        XCTAssertThrowsError(try Model.initRecordingRing(cap: 0)) { error in
            guard case .invalidArgument = error as? AimuxError else {
                return XCTFail("expected .invalidArgument, got \(error)")
            }
        }
    }

    /// A positive cap constructs the ring recorder without throwing.
    func testInitRecordingRingAcceptsPositiveCap() throws {
        XCTAssertNoThrow(try Model.initRecordingRing(cap: 8))
        // Reset global recorder state so this doesn't leak into other tests.
        Model.recordingStop()
    }

    // MARK: - base_url constructors (no network: just construction)

    func testOpenAIWithBaseUrlConstructs() throws {
        // A base URL pointing at a non-listening port still constructs the
        // provider (the URL is only contacted on the first API call).
        let model = try Model.openai(
            apiKey: "test-key", modelId: "gpt-4o",
            baseUrl: "http://127.0.0.1:1"
        )
        XCTAssertNotNil(model)
    }

    func testAnthropicWithBaseUrlConstructs() throws {
        let model = try Model.anthropic(
            apiKey: "test-key", modelId: "claude-3-5-sonnet-20241022",
            baseUrl: "http://127.0.0.1:1"
        )
        XCTAssertNotNil(model)
    }

    // MARK: - E2E via mock HTTP server (full chain: Swift → FFI → reqwest → mock)

    /// Plain OpenAI generate_text: base_url routes reqwest to the mock server.
    func testE2EOpenAIGenerateTextViaBaseUrl() throws {
        let server = MockHTTPServer(response: .json(openaiPlainResponse))
        try server.start()
        defer { server.stop() }

        let model = try Model.openai(
            apiKey: "test-key", modelId: "gpt-4o", baseUrl: server.baseURL
        )
        let resultJson = try model.generateText(prompt: jsonEncodeString("What is Rust?"))
        let r = parseJSON(resultJson)
        XCTAssertEqual(r["text"] as? String, "Rust is a systems programming language.")
        XCTAssertNotNil(r["usage"])
        XCTAssertEqual(server.lastRequestPath, "/chat/completions")
    }

    /// Tool-call parsing: mock returns tool_calls; verify convenience field
    /// `tool_calls` and the structured `raw.content` ToolCall variant.
    func testE2EOpenAIToolCallParsing() throws {
        let server = MockHTTPServer(response: .json(openaiToolCallResponse))
        try server.start()
        defer { server.stop() }

        let model = try Model.openai(
            apiKey: "test-key", modelId: "gpt-4o", baseUrl: server.baseURL
        )
        let opts = jsonEncode([
            "tools": [[
                "type": "function",
                "name": "get_weather",
                "description": "Get weather for a location",
                "input_schema": [
                    "type": "object",
                    "properties": ["location": ["type": "string"]],
                    "required": ["location"],
                ],
            ]],
        ])
        let r = parseJSON(try model.generateText(
            prompt: jsonEncodeString("What's the weather in Tokyo?"), options: opts
        ))

        // Convenience field: tool_calls extracted
        let toolCalls = (r["tool_calls"] as? [[String: Any]]) ?? []
        XCTAssertEqual(toolCalls.count, 1)
        XCTAssertEqual(toolCalls[0]["tool_name"] as? String, "get_weather")
        XCTAssertEqual(toolCalls[0]["tool_call_id"] as? String, "call_abc")
        let input = toolCalls[0]["input"] as? [String: Any]
        XCTAssertEqual(input?["location"] as? String, "Tokyo")

        // Structured content: raw.content contains a ToolCall variant
        let raw = r["raw"] as? [String: Any]
        let content = (raw?["content"] as? [[String: Any]]) ?? []
        let tcVariant = content.first(where: { $0["ToolCall"] != nil })
        XCTAssertNotNil(tcVariant, "raw.content should contain a ToolCall variant")
        let tc = tcVariant?["ToolCall"] as? [String: Any]
        XCTAssertEqual(tc?["tool_name"] as? String, "get_weather")
        XCTAssertEqual(tc?["tool_call_id"] as? String, "call_abc")
    }

    /// Multi-role messages: a system+user array reaches the provider verbatim.
    func testE2EOpenAIMultiRoleMessages() throws {
        let server = MockHTTPServer(response: .json(openaiPlainResponse))
        try server.start()
        defer { server.stop() }

        let model = try Model.openai(
            apiKey: "test-key", modelId: "gpt-4o", baseUrl: server.baseURL
        )
        let prompt = jsonEncode([
            ["role": "system", "content": "You are a helpful assistant."],
            ["role": "user", "content": "What is Rust?"],
        ])
        let r = parseJSON(try model.generateText(prompt: prompt))
        XCTAssertEqual(r["text"] as? String, "Rust is a systems programming language.")

        // The full multi-role message sequence reaches the provider request body.
        let reqBody = parseJSON(server.lastRequestBody)
        let msgs = (reqBody["messages"] as? [[String: Any]]) ?? []
        XCTAssertEqual(msgs.count, 2)
        XCTAssertEqual(msgs[0]["role"] as? String, "system")
        XCTAssertEqual(msgs[0]["content"] as? String, "You are a helpful assistant.")
        XCTAssertEqual(msgs[1]["role"] as? String, "user")
        XCTAssertEqual(msgs[1]["content"] as? String, "What is Rust?")
    }

    /// ToolChoice: passing `tool_choice: "required"` reaches the provider body.
    func testE2EOpenAIToolChoiceRequired() throws {
        let server = MockHTTPServer(response: .json(openaiToolCallResponse))
        try server.start()
        defer { server.stop() }

        let model = try Model.openai(
            apiKey: "test-key", modelId: "gpt-4o", baseUrl: server.baseURL
        )
        let opts = jsonEncode([
            "tools": [[
                "type": "function",
                "name": "get_weather",
                "input_schema": [
                    "type": "object",
                    "properties": ["location": ["type": "string"]],
                ],
            ]],
            "tool_choice": "required",
        ])
        // Must not throw.
        _ = try model.generateText(prompt: jsonEncodeString("Hello"), options: opts)

        // tool_choice reaches the provider request body as "required".
        let reqBody = parseJSON(server.lastRequestBody)
        XCTAssertEqual(reqBody["tool_choice"] as? String, "required")
    }

    /// Streaming text: SSE deltas reassemble into the full text.
    func testE2EOpenAIStreamText() throws {
        let server = MockHTTPServer(response: .sse(sse(openaiStreamTextEvents)))
        try server.start()
        defer { server.stop() }

        let model = try Model.openai(
            apiKey: "test-key", modelId: "gpt-4o", baseUrl: server.baseURL
        )
        var parts: [String] = []
        var streamErr: AimuxError?
        model.streamText(
            prompt: jsonEncodeString("Say hello"),
            onPart: { parts.append($0) },
            onDone: {},
            onError: { streamErr = $0 }
        )
        XCTAssertNil(streamErr)
        XCTAssertFalse(parts.isEmpty)

        let parsed = parts.map { parseJSON($0) }
        let text = parsed
            .compactMap { ($0["TextDelta"] as? [String: Any])?["delta"] as? String }
            .joined()
        XCTAssertEqual(text, "Hello world")
    }

    /// Streaming tool call: the stream emits a tool-related StreamPart
    /// (ToolCall / ToolInputStart / ToolInputDelta).
    func testE2EOpenAIStreamToolCall() throws {
        let server = MockHTTPServer(response: .sse(sse(openaiStreamToolEvents)))
        try server.start()
        defer { server.stop() }

        let model = try Model.openai(
            apiKey: "test-key", modelId: "gpt-4o", baseUrl: server.baseURL
        )
        let opts = jsonEncode([
            "tools": [[
                "type": "function",
                "name": "get_weather",
                "input_schema": [
                    "type": "object",
                    "properties": ["location": ["type": "string"]],
                ],
            ]],
        ])
        var parts: [String] = []
        var streamErr: AimuxError?
        model.streamText(
            prompt: jsonEncodeString("What's the weather?"),
            options: opts,
            onPart: { parts.append($0) },
            onDone: {},
            onError: { streamErr = $0 }
        )
        XCTAssertNil(streamErr)
        XCTAssertFalse(parts.isEmpty)

        let parsed = parts.map { parseJSON($0) }
        let hasToolPart = parsed.contains {
            $0["ToolCall"] != nil
                || $0["ToolInputDelta"] != nil
                || $0["ToolInputStart"] != nil
        }
        XCTAssertTrue(hasToolPart, "stream should contain a tool-related StreamPart")

        if let toolCall = parsed.first(where: { $0["ToolCall"] != nil })?["ToolCall"] as? [String: Any] {
            XCTAssertEqual(toolCall["tool_name"] as? String, "get_weather")
        }
    }

    /// Anthropic generate_text via base_url (provider interchangeability).
    func testE2EAnthropicGenerateTextViaBaseUrl() throws {
        let server = MockHTTPServer(response: .json([
            "id": "msg_test",
            "type": "message",
            "role": "assistant",
            "model": "claude-3-5-sonnet-20241022",
            "content": [["type": "text", "text": "Hello from Claude!"]],
            "stop_reason": "end_turn",
            "usage": ["input_tokens": 10, "output_tokens": 5],
        ]))
        try server.start()
        defer { server.stop() }

        let model = try Model.anthropic(
            apiKey: "test-key", modelId: "claude-3-5-sonnet-20241022",
            baseUrl: server.baseURL
        )
        let r = parseJSON(try model.generateText(prompt: jsonEncodeString("Hello")))
        XCTAssertEqual(r["text"] as? String, "Hello from Claude!")
        XCTAssertNotNil(r["usage"])
        XCTAssertEqual(server.lastRequestPath, "/v1/messages")
    }

    /// Full tool-call round trip: the first `generateText` returns a tool call;
    /// the caller back-fills a `ToolResult` and calls `generateText` again,
    /// which returns the final text. Verifies the two-request exchange
    /// end-to-end (Swift → FFI → reqwest → mock) and that the second request
    /// body carries the tool-result message.
    ///
    /// The replayed messages use the aimux content-part shape (type-tagged
    /// `ToolCall` / `ToolResult`), which is what `generate_text` deserializes
    /// and the OpenAI converter maps to the wire `tool_calls` / `tool` format.
    func testE2EToolCallRoundTrip() throws {
        let server = MockHTTPServer(responses: [
            .json(openaiToolCallResponse),
            .json(roundTripFinalResponse),
        ])
        try server.start()
        defer { server.stop() }

        let model = try Model.openai(
            apiKey: "test-key", modelId: "gpt-4o", baseUrl: server.baseURL
        )
        let opts = jsonEncode([
            "tools": [[
                "type": "function",
                "name": "get_weather",
                "description": "Get weather for a location",
                "input_schema": [
                    "type": "object",
                    "properties": ["location": ["type": "string"]],
                    "required": ["location"],
                ],
            ]],
        ])

        // 1) First call: the model asks to call get_weather.
        let r1 = parseJSON(try model.generateText(
            prompt: jsonEncodeString("What's the weather in Tokyo?"), options: opts
        ))
        let toolCalls = (r1["tool_calls"] as? [[String: Any]]) ?? []
        XCTAssertEqual(toolCalls.count, 1)
        XCTAssertEqual(toolCalls[0]["tool_name"] as? String, "get_weather")
        XCTAssertEqual(toolCalls[0]["tool_call_id"] as? String, "call_abc")

        // 2) Second call: replay the conversation with the ToolResult filled in.
        let secondPrompt = jsonEncode([
            ["role": "user", "content": "What's the weather in Tokyo?"],
            ["role": "assistant", "content": [[
                "type": "tool_call",
                "tool_call_id": "call_abc",
                "tool_name": "get_weather",
                "input": ["location": "Tokyo"],
            ]]],
            ["role": "tool", "content": [[
                "type": "tool_result",
                "tool_call_id": "call_abc",
                "output": ["temperature": 22, "condition": "sunny"],
            ]]],
        ])
        let r2 = parseJSON(try model.generateText(prompt: secondPrompt, options: opts))
        XCTAssertEqual(r2["text"] as? String, "The weather in Tokyo is sunny.")

        // 3) The second request body carries all three messages; the last is
        //    the tool result.
        let reqBody = parseJSON(server.lastRequestBody)
        let msgs = (reqBody["messages"] as? [[String: Any]]) ?? []
        XCTAssertEqual(msgs.count, 3)
        XCTAssertEqual(msgs.last?["role"] as? String, "tool")
    }
}

// MARK: - Mock response fixtures (same shape as Node e2e.test.ts / Rust e2e_test.rs)

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

/// Final text response for the tool-call round trip (second request).
private let roundTripFinalResponse: [String: Any] = [
    "id": "chatcmpl-test",
    "model": "gpt-4o",
    "choices": [[
        "message": ["role": "assistant", "content": "The weather in Tokyo is sunny."],
        "finish_reason": "stop",
    ]],
    "usage": ["prompt_tokens": 30, "completion_tokens": 8, "total_tokens": 38],
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

// MARK: - JSON helpers

/// JSON-encode a bare string prompt (`"text"`) as the FFI expects.
private func jsonEncodeString(_ s: String) -> String {
    let data = try! JSONEncoder().encode(s)
    return String(data: data, encoding: .utf8)!
}

/// JSON-encode an array or dictionary object.
private func jsonEncode(_ object: Any) -> String {
    let data = try! JSONSerialization.data(withJSONObject: object)
    return String(data: data, encoding: .utf8)!
}

/// Build an SSE body from a list of `data:` JSON events, terminated by [DONE].
private func sse(_ events: [[String: Any]]) -> String {
    let body = events.map { ev -> String in
        let data = try! JSONSerialization.data(withJSONObject: ev)
        return "data: " + (String(data: data, encoding: .utf8) ?? "")
    }.joined(separator: "\n\n")
    return body + "\n\n" + "data: [DONE]\n\n"
}

/// Parse a JSON object string into `[String: Any]` (empty on failure).
private func parseJSON(_ s: String) -> [String: Any] {
    guard let data = s.data(using: .utf8),
          let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
        return [:]
    }
    return obj
}
