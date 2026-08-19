import XCTest
@testable import Aimux
import CAimuxFFI

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
            name: ProviderName.Groq.rawValue, apiKey: "sk-test-fake-key", modelId: "llama-3.3-70b"
        )
        XCTAssertNotNil(model)
    }

    func testInvalidPromptThrows() throws {
        let model = try Model.openai(apiKey: "sk-test-fake-key", modelId: "gpt-4o-mini")
        XCTAssertThrowsError(try model.generateText(prompt: "{invalid json}"))
    }

    func testNoSuchProviderCarriesProviderId() {
        XCTAssertThrowsError(try Model.provider(
            name: "no-such-provider", apiKey: "sk-test-fake-key", modelId: "whatever"
        )) { error in
            guard let e = error as? AimuxError else {
                return XCTFail("expected AimuxError, got \(error)")
            }
            guard case .noSuchProvider = e else {
                return XCTFail("expected .noSuchProvider, got \(e)")
            }
            XCTAssertEqual(e.providerId, "no-such-provider")
        }
    }

    /// `expectAimuxError` reads code 10, copies the per-code payload into the
    /// matching case, leaves unrelated payload accessors `nil`, and frees the
    /// returned error.
    /// Driven through the real FFI: an unknown provider fails offline.
    func testExpectAimuxReadsReturnedError() throws {
        var h: UInt64 = 0
        guard let e = aimux_provider_new("no-such-provider", "k", "m", nil, &h) else {
            return XCTFail("expected a returned error")
        }
        XCTAssertEqual(h, 0)
        XCTAssertEqual(aimux_error_code(e), Int32(AIMUX_E_NO_SUCH_PROVIDER.rawValue))
        guard let err = expectAimuxError(e, context: "test") as? AimuxError else { // frees `e`
            return XCTFail("expected AimuxError")
        }
        guard case .noSuchProvider = err else {
            return XCTFail("expected .noSuchProvider, got \(err)")
        }
        XCTAssertFalse(err.message.isEmpty)
        XCTAssertEqual(err.providerId, "no-such-provider")
        XCTAssertNil(err.status)
        XCTAssertNil(err.retryMs)
        XCTAssertFalse(err.retryable)
        XCTAssertNil(err.providerCode)
        XCTAssertNil(err.modelId)
    }

    /// NULL owners answer nothing (NULL-safe getters, aimux-error.h).
    func testNullOwnerGettersAreSafe() {
        XCTAssertEqual(aimux_error_code(nil), 0)
        XCTAssertNil(aimux_error_message(nil))
        XCTAssertNil(aimux_error_message(nil))
        aimux_error_free(nil)
    }

    // MARK: C ABI failure codes

    /// A NULL required string argument is an invariant a correct binding never
    /// triggers: code 200 has a non-empty message, and the
    /// decoder throws `DecodingError` (never an `AimuxError`).
    func testNullPointerArgIsDecodingError() throws {
        var h: UInt64 = 0
        guard let e = aimux_openai_new(nil, "gpt-4o", &h) else {
            return XCTFail("expected a returned error")
        }
        XCTAssertEqual(h, 0)
        XCTAssertEqual(aimux_error_code(e), 200)
        let raw = aimux_error_message(e)
        XCTAssertNotNil(raw)
        XCTAssertFalse(String(cString: raw!).isEmpty)
        aimux_free_string(raw)
        let error = expectAimuxError(e, context: "test") // frees `e`
        XCTAssertFalse(error is AimuxError)
        guard case DecodingError.dataCorrupted(let ctx) = error else {
            return XCTFail("expected DecodingError.dataCorrupted, got \(error)")
        }
        XCTAssertTrue(ctx.debugDescription.hasPrefix("aimux ffi: "), ctx.debugDescription)
        XCTAssertTrue(ctx.debugDescription.contains("api_key"), ctx.debugDescription)
    }

    /// A dead / never-issued handle reaching the FFI (the wrappers guard this
    /// locally with `precondition`) is likewise the invariant `DecodingError`,
    /// whichever decoder the call site uses.
    func testInvalidHandleIsDecodingError() {
        var out: UnsafeMutablePointer<CChar>? = nil
        guard let e = aimux_generate_text(0x7FFF_FFFF_FFFF_FFFF, "\"hi\"", nil, &out) else {
            return XCTFail("expected a returned error")
        }
        XCTAssertNil(out)
        XCTAssertEqual(aimux_error_code(e), 203)
        let error = expectAimuxError(e, context: "test")
        XCTAssertFalse(error is AimuxError)
        XCTAssertTrue(error is DecodingError, "expected DecodingError, got \(error)")

        guard let e2 = aimux_transcription_input_done(0x7FFF_FFFF_FFFF_FFFF) else {
            return XCTFail("expected a returned error")
        }
        XCTAssertEqual(aimux_error_code(e2), 203)
        XCTAssertTrue(expectRecordingError(e2, context: "test") is DecodingError)
    }

    /// Malformed raw JSON is rejected by the binding before the C call:
    /// `DecodingError.dataCorrupted` naming the parameter, not an `AimuxError`.
    func testMalformedJsonIsRejectedBeforeFfi() throws {
        let model = try Model.openai(apiKey: "sk-test-fake-key", modelId: "gpt-4o-mini")
        XCTAssertThrowsError(try model.generateText(prompt: "{not json")) { error in
            XCTAssertFalse(error is AimuxError)
            guard case DecodingError.dataCorrupted(let ctx) = error else {
                return XCTFail("expected DecodingError.dataCorrupted, got \(error)")
            }
            XCTAssertTrue(ctx.debugDescription.contains("prompt"), ctx.debugDescription)
        }
        XCTAssertThrowsError(try model.generateText(prompt: "\"hi\"", options: "{not json")) { error in
            guard case DecodingError.dataCorrupted(let ctx) = error else {
                return XCTFail("expected DecodingError.dataCorrupted, got \(error)")
            }
            XCTAssertTrue(ctx.debugDescription.contains("options"), ctx.debugDescription)
        }
        // Valid JSON fragments pass the pre-check (they reach the C ABI).
        XCTAssertNoThrow(try validateJson("\"hi\"", parameter: "prompt"))
        XCTAssertNoThrow(try validateJson(nil, parameter: "options"))
        // Optional params: blank means default (FFI rule); required: rejected.
        XCTAssertNoThrow(try validateJson(Optional(" "), parameter: "options"))
        XCTAssertThrowsError(try validateJson("", parameter: "prompt"))
    }

    /// Every raw-JSON entry pre-validates: multimodal `embed(values:)`,
    /// composite `router(configJson:)` and `mockReplay` reject malformed JSON
    /// with `DecodingError` before any C call (constructors are infallible
    /// offline, so this runs without a network).
    func testRawJsonEntriesRejectMalformedJsonBeforeFfi() throws {
        let embedder = try EmbeddingModel.openai(apiKey: "sk-test-fake-key", modelId: "text-embedding-3-small")
        XCTAssertThrowsError(try embedder.embed(values: "{not json")) { error in
            guard case DecodingError.dataCorrupted(let ctx) = error else {
                return XCTFail("expected DecodingError.dataCorrupted, got \(error)")
            }
            XCTAssertTrue(ctx.debugDescription.contains("values"), ctx.debugDescription)
        }
        let child = try Model.openai(apiKey: "sk-test-fake-key", modelId: "gpt-4o-mini")
        XCTAssertThrowsError(try Model.router([child], configJson: "{not json")) { error in
            guard case DecodingError.dataCorrupted(let ctx) = error else {
                return XCTFail("expected DecodingError.dataCorrupted, got \(error)")
            }
            XCTAssertTrue(ctx.debugDescription.contains("configJson"), ctx.debugDescription)
        }
        XCTAssertThrowsError(try Model.mockReplay(recordingsJsonl: "{}\n{not json\n")) { error in
            guard case DecodingError.dataCorrupted(let ctx) = error else {
                return XCTFail("expected DecodingError.dataCorrupted, got \(error)")
            }
            XCTAssertTrue(ctx.debugDescription.contains("recordingsJsonl"), ctx.debugDescription)
        }
    }

    /// `code` returns the mapped C constant; there is no binding-local case
    /// without one.
    func testCodeAccessor() {
        XCTAssertEqual(AimuxError.timeout(message: "t", status: -1, retryMs: -1, retryable: true).code,
                       Int32(AIMUX_E_TIMEOUT.rawValue))
    }

    /// HTTP-shaped failures all arrive as `.apiCall`; the classification is the
    /// `status` field (there are no per-status cases any more).
    func testApiCallClassifiesByStatus() {
        let rateLimited = AimuxError.apiCall(
            message: "API call error: HTTP 429: slow down",
            status: 429, retryMs: 1500, retryable: true
        )
        XCTAssertEqual(rateLimited.status, 429)
        XCTAssertEqual(rateLimited.retryMs, 1500)
        XCTAssertTrue(rateLimited.retryable)

        let auth = AimuxError.apiCall(
            message: "API call error: HTTP 401: invalid api key",
            status: 401, retryMs: -1, retryable: false
        )
        XCTAssertEqual(auth.status, 401)
        XCTAssertNil(auth.retryMs)
        XCTAssertFalse(auth.retryable)

        // Transport failure: no response, so no status.
        let transport = AimuxError.apiCall(
            message: "API call error: connection reset",
            status: -1, retryMs: -1, retryable: true
        )
        XCTAssertNil(transport.status)
    }

    /// `retryable` is not derivable from `status`: both of these report no
    /// status and disagree about whether a retry is worth attempting.
    func testRetryableIsNotDerivableFromStatus() {
        let transport = AimuxError.apiCall(
            message: "API call error: connection reset",
            status: -1, retryMs: -1, retryable: true
        )
        let missingKey = AimuxError.apiCall(
            message: "API call error: missing api key",
            status: -1, retryMs: -1, retryable: false
        )
        XCTAssertNil(transport.status)
        XCTAssertNil(missingKey.status)
        XCTAssertTrue(transport.retryable)
        XCTAssertFalse(missingKey.retryable)
    }

    /// End to end: a 401 from the provider surfaces as `.apiCall` with the
    /// observed status, not a dedicated auth case.
    func testE2EAuthFailureIsApiCall() throws {
        let server = MockHTTPServer(response: .json(
            ["error": ["message": "invalid api key", "type": "invalid_request_error"]],
            status: 401
        ))
        try server.start()
        defer { server.stop() }

        let model = try Model.openai(
            apiKey: "bad-key", modelId: "gpt-4o", baseUrl: server.baseURL
        )
        XCTAssertThrowsError(try model.generateText(prompt: jsonEncodeString("hi"))) { error in
            guard let e = error as? AimuxError else {
                return XCTFail("expected AimuxError, got \(error)")
            }
            guard case .apiCall = e else {
                return XCTFail("expected .apiCall, got \(e)")
            }
            XCTAssertEqual(e.status, 401)
            XCTAssertTrue(e.message.contains("401"), "got \(e.message)")
            XCTAssertFalse(e.retryable)
            XCTAssertEqual(e.responseBody?.contains("invalid api key"), true, "got \(String(describing: e.responseBody))")
        }
    }

    func testStreamTextReturnsAsyncSequence() throws {
        let model = try Model.openai(apiKey: "sk-test-fake-key", modelId: "gpt-4o-mini")
        let stream = model.streamTextAsync(prompt: "\"hello\"")
        // AsyncStream is created — we don't consume it (would need network).
        XCTAssertNotNil(stream)
    }

    // MARK: - Recording ring (T10)

    /// `cap: 0` is decided by aimux-core, not by a local trap: the C layer
    /// classifies it as `AIMUX_E_INVALID_ARGUMENT`, so Swift throws
    /// `AimuxError.invalidArgument` — catchable, identical to what a C caller
    /// sees.
    func testInitRecordingRingZeroCapThrowsInvalidArgument() {
        XCTAssertThrowsError(try Model.initRecordingRing(cap: 0)) { error in
            guard let e = error as? AimuxError else {
                return XCTFail("expected AimuxError, got \(error)")
            }
            guard case .invalidArgument = e else {
                return XCTFail("expected .invalidArgument, got \(e)")
            }
        }
        // A rejected cap must not have replaced the global recorder.
        Model.recordingStop()
    }

    /// A positive cap constructs the ring recorder.
    func testInitRecordingRingAcceptsPositiveCap() throws {
        try Model.initRecordingRing(cap: 8)
        // Reset global recorder state so this doesn't leak into other tests.
        Model.recordingStop()
    }

    /// Omitting cap uses the library default capacity (FFI
    /// aimux_init_recording_ring_default) and must not throw.
    func testInitRecordingRingDefaultNoArg() throws {
        try Model.initRecordingRing()
        // Reset global recorder state so this doesn't leak into other tests.
        Model.recordingStop()
    }

    // MARK: - recordingTryFlush

    /// Nothing recording: the checked flush is a no-op that must not throw.
    func testRecordingTryFlushNoRecorder() {
        Model.recordingStop()
        XCTAssertNoThrow(try Model.recordingTryFlush())
    }

    func testRecordingCodeOutsideRustEnumIsRejected() {
        XCTAssertNil(RecordingError.code(fromC: 999))
    }

    /// Unwritable dir (parent path is a regular file): init fails with
    /// `RecordingError.initFailed` (not an `AimuxError`), nothing is
    /// installed, and the checked flush afterwards succeeds.
    func testRecordingInitFailedUnwritableDir() throws {
        let blocker = FileManager.default.temporaryDirectory
            .appendingPathComponent("aimux-swift-blocker-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: blocker, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: blocker) }
        let occupied = blocker.appendingPathComponent("occupied")
        try Data("x".utf8).write(to: occupied)

        defer { Model.recordingStop() }
        XCTAssertThrowsError(
            try Model.initRecording(dir: occupied.appendingPathComponent("sub").path)
        ) { error in
            XCTAssertNil(error as? AimuxError, "recording errors are not AimuxError")
            guard let e = error as? RecordingError else {
                return XCTFail("expected RecordingError, got \(error)")
            }
            XCTAssertEqual(e.code, .initFailed)
            XCTAssertFalse(e.message.isEmpty)
        }
        XCTAssertNoThrow(try Model.recordingTryFlush())
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
        var streamErr: (any Error)?
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
        var streamErr: (any Error)?
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
