// MultimodalTests.swift — E2E tests for the multimodal bindings.
//
// Mirrors bindings/go/multimodal_withbase_test.go (the gold reference). Each
// test stands up a MockHTTPServer with a canned *provider* JSON response,
// constructs a model via the provider factory with `baseUrl` pointing at the
// mock, calls the method, and parses the returned JSON-string result to assert
// fields.
//
// Important: the methods return the aimux *canonical* wire shape produced by
// the FFI, NOT the raw provider response. The FFI converts provider shapes,
// e.g. OpenAI `data[].b64_json` → `images.Base64[]`, an OpenAI file object →
// `provider_reference.openai`, raw audio bytes → `audio.Binary[]`. The canned
// responses below are the provider shapes (what the mock returns); the
// assertions check the converted aimux shapes (what the FFI returns).
//
// Call-options JSON always includes `"provider_options":{}` because the Rust
// SharedProviderOptions (HashMap) requires a map, not null — matching the Go
// bindings' non-omitempty `jsonObj` (see bindings/go/multimodal_types.go).

import XCTest
@testable import Aimux

final class MultimodalTests: XCTestCase {

    // MARK: - Embedding

    /// OpenAI embedding: mock returns `data[].embedding`; the FFI exposes it as
    /// `embeddings` (an array of float arrays). The first embedding has 3 dims.
    func testE2E_EmbeddingViaMock() throws {
        let server = MockHTTPServer(response: jsonResponse(
            #"{"data":[{"embedding":[0.1,0.2,0.3],"index":0}],"model":"text-embedding-3-small","usage":{"prompt_tokens":3,"total_tokens":3}}"#
        ))
        try server.start()
        defer { server.stop() }

        let model = try EmbeddingModel.openai(
            apiKey: "test", modelId: "text-embedding-3-small", baseUrl: server.baseURL
        )
        let r = parseJSON(try model.embed(values: jsonEncode(["hello"])))

        // `embeddings` is [[float]]; verify one embedding vector of dimension 3.
        let embeddings = (r["embeddings"] as? [Any]) ?? []
        XCTAssertEqual(embeddings.count, 1)
        let firstDims = (embeddings.first as? [Any]) ?? []
        XCTAssertEqual(firstDims.count, 3, "first embedding should have 3 dimensions")
    }

    // MARK: - Image

    /// OpenAI image: mock returns `data[].b64_json`; the FFI exposes it as
    /// `images.Base64[]`.
    func testE2E_ImageViaMock() throws {
        let server = MockHTTPServer(response: jsonResponse(
            #"{"data":[{"b64_json":"aW1hZ2Ux"}]}"#
        ))
        try server.start()
        defer { server.stop() }

        let model = try ImageModel.openai(
            apiKey: "test", modelId: "dall-e-3", baseUrl: server.baseURL
        )
        let opts = jsonEncode([
            "prompt": "otter",
            "n": 1,
            "provider_options": [String: Any](),
        ])
        let r = parseJSON(try model.generate(options: opts))

        let images = (r["images"] as? [String: Any]) ?? [:]
        let base64 = (images["Base64"] as? [String]) ?? []
        XCTAssertEqual(base64.count, 1)
        XCTAssertEqual(base64[0], "aW1hZ2Ux")
    }

    // MARK: - Transcription

    /// OpenAI transcription: mock returns `{"text":"..."}`; the FFI passes the
    /// text through as `text`.
    func testE2E_TranscriptionViaMock() throws {
        let server = MockHTTPServer(response: jsonResponse(
            #"{"text":"Hello world"}"#
        ))
        try server.start()
        defer { server.stop() }

        let model = try TranscriptionModel.openai(
            apiKey: "test", modelId: "whisper-1", baseUrl: server.baseURL
        )
        let r = parseJSON(try model.generate(audioBase64: "dGVzdA==", mediaType: "audio/mp3"))

        XCTAssertEqual(r["text"] as? String, "Hello world")
    }

    // MARK: - Reranking

    /// Cohere reranking: mock returns `results[]`; the FFI exposes it as
    /// `ranking[]` with `index` and `relevance_score`.
    func testE2E_RerankingViaMock() throws {
        let server = MockHTTPServer(response: jsonResponse(
            #"{"results":[{"index":1,"relevance_score":0.95},{"index":0,"relevance_score":0.3}]}"#
        ))
        try server.start()
        defer { server.stop() }

        let model = try RerankingModel.cohere(
            apiKey: "test", modelId: "rerank-v3.0", baseUrl: server.baseURL
        )
        let opts = jsonEncode([
            "query": "which?",
            "documents": ["Text": ["values": ["doc1", "doc2"]]],
            "top_n": 2,
            "provider_options": [String: Any](),
        ])
        let r = parseJSON(try model.rerank(options: opts))

        let ranking = (r["ranking"] as? [[String: Any]]) ?? []
        XCTAssertEqual(ranking.count, 2)
        XCTAssertEqual(ranking[0]["index"] as? Int, 1)
        XCTAssertEqual(ranking[0]["relevance_score"] as? Double, 0.95)
    }

    // MARK: - Search

    /// Tavily search: mock returns `results[]` + `answer`; the FFI exposes them
    /// as `results[]` (title/url/content) and `answer`.
    func testE2E_SearchViaMock() throws {
        let server = MockHTTPServer(response: jsonResponse(
            #"{"results":[{"title":"Rust","url":"https://rust-lang.org","content":"Rust is..."}],"answer":"Rust is a systems language."}"#
        ))
        try server.start()
        defer { server.stop() }

        let model = try SearchModel.tavily(apiKey: "test", baseUrl: server.baseURL)
        let opts = jsonEncode([
            "query": "What is Rust?",
            "provider_options": [String: Any](),
        ])
        let r = parseJSON(try model.search(options: opts))

        let results = (r["results"] as? [[String: Any]]) ?? []
        XCTAssertEqual(results.count, 1)
        XCTAssertEqual(results[0]["title"] as? String, "Rust")
        XCTAssertEqual(r["answer"] as? String, "Rust is a systems language.")
    }

    // MARK: - Files

    /// OpenAI files: mock returns an OpenAI file object; the FFI exposes the
    /// file id as `provider_reference.openai`.
    func testE2E_FilesViaMock() throws {
        let server = MockHTTPServer(response: jsonResponse(
            #"{"id":"file-abc","object":"file","bytes":1024,"created_at":1234,"filename":"test.pdf","purpose":"assistants"}"#
        ))
        try server.start()
        defer { server.stop() }

        let files = try Files.openai(apiKey: "test", baseUrl: server.baseURL)
        let r = parseJSON(try files.upload(dataBase64: "dGVzdA==", mediaType: "application/pdf"))

        let providerRef = (r["provider_reference"] as? [String: Any]) ?? [:]
        XCTAssertEqual(providerRef["openai"] as? String, "file-abc")
    }

    // MARK: - Speech (TTS)

    /// OpenAI speech (TTS): the mock returns a raw binary body with
    /// `Content-Type: audio/mpeg`. Unlike a URLProtocol mock, this POSIX-socket
    /// MockHTTPServer supports arbitrary content-types via direct `MockResponse`
    /// construction, so we exercise the full round trip (Swift → FFI → reqwest →
    /// mock). The FFI returns the bytes as `audio.Binary`.
    func testE2E_SpeechViaMock() throws {
        // A non-empty binary payload (the literal bytes of a base64-looking
        // string, matching bindings/go/multimodal_withbase_test.go).
        let audioBody = Data("SGVsbG8gd29ybGQ=".utf8)
        let server = MockHTTPServer(response: MockResponse(
            status: 200, contentType: "audio/mpeg", body: audioBody
        ))
        try server.start()
        defer { server.stop() }

        let model = try SpeechModel.openai(
            apiKey: "test", modelId: "tts-1", baseUrl: server.baseURL
        )
        let opts = jsonEncode([
            "text": "Hi",
            "voice": "alloy",
            "output_format": "mp3",
            "provider_options": [String: Any](),
        ])
        let r = parseJSON(try model.generate(options: opts))

        let audio = (r["audio"] as? [String: Any]) ?? [:]
        let binary = (audio["Binary"] as? [Any]) ?? []
        XCTAssertFalse(binary.isEmpty, "speech audio should contain binary bytes")
    }

    // MARK: - Video (construction + result parsing only)

    /// Google video uses a multi-step async API (POST predict → poll operation →
    /// fetch result) that a single-response mock server can't cover. We verify
    /// construction with a base URL (pointing at a non-listening port, never
    /// contacted) and parse a canned VideoResult JSON, checking the `Url`
    /// variant's `url`/`media_type` — mirroring the Go test.
    func testE2E_VideoConstructionAndParse() throws {
        let model = try VideoModel.google(
            apiKey: "test", modelId: "veo-3.0", baseUrl: "http://localhost:9999"
        )
        XCTAssertNotNil(model)

        let r = parseJSON(
            #"{"videos":[{"Url":{"url":"https://example.com/v.mp4","media_type":"video/mp4"}}]}"#
        )
        let videos = (r["videos"] as? [[String: Any]]) ?? []
        XCTAssertEqual(videos.count, 1)
        let urlData = (videos[0]["Url"] as? [String: Any]) ?? [:]
        XCTAssertEqual(urlData["url"] as? String, "https://example.com/v.mp4")
        XCTAssertEqual(urlData["media_type"] as? String, "video/mp4")
    }

    // MARK: - TranscriptionSession use-after-close

    // A live session needs a realtime websocket provider, which no mock here
    // can stand up. `close()` is defined as `handle = 0`, so a session built
    // on handle 0 *is* a closed session — and the drop is skipped, so nothing
    // is freed twice. These tests pin the contract that matters: after close,
    // every method throws and none of them traps (a trap is SIGTRAP, which
    // XCTest cannot catch — if this regresses, the test run dies here).

    /// All three methods on a closed session throw a C ABI failure —
    /// catchable (Go `ErrClosed` / Dart `StateError` / Kotlin
    /// `IllegalStateException` are the peers) and deliberately NOT
    /// `AimuxTranscriptionEndedError`: a pump loop that breaks on "ended"
    /// must not read a transcript truncated by its own `close()` as complete.
    func testClosedSessionMethodsThrowFfiErrorNotEnded() {
        let session = TranscriptionSession(handle: 0)

        for (name, call) in [
            ("pushAudio", { try session.pushAudio([0x00, 0x01]) }),
            ("inputDone", { try session.inputDone() }),
            ("nextPart", { _ = try session.nextPart(timeoutMs: 0) }),
        ] as [(String, () throws -> Void)] {
            XCTAssertThrowsError(try call(), name) { error in
                XCTAssertFalse(
                    error is AimuxTranscriptionEndedError,
                    "\(name): a closed session must not look like a clean end of stream"
                )
                XCTAssertTrue(
                    error is DecodingError,
                    "\(name): expected the Swift boundary projection, got \(error)"
                )
            }
        }
    }

    /// The `defer { session.close() }` shape: close, then one more call. This
    /// used to trap the host process; it must be an ordinary catch.
    func testCloseThenCallIsCatchable() {
        let session = TranscriptionSession(handle: 0)
        session.close()
        session.close()  // idempotent

        var caught = false
        do {
            try session.inputDone()
        } catch is AimuxTranscriptionEndedError {
            XCTFail("a closed session must not be reported as a clean end of stream")
        } catch is DecodingError {
            caught = true
        } catch {
            XCTFail("expected the Swift boundary projection, got \(error)")
        }
        XCTAssertTrue(caught, "use-after-close must be catchable, not a trap")
    }

    // MARK: - router([])

    /// An empty child list throws C's zero-children failure instead of
    /// trapping — the array may well come from a `.filter`, and the function
    /// already has a throwing channel.
    func testRouterWithNoChildrenThrows() {
        XCTAssertThrowsError(try Model.router([]))
    }
}

// MARK: - Helpers

/// Build a JSON `MockResponse` from a raw JSON string. This lets the canned
/// responses mirror the Go test fixtures verbatim (no double-serialization).
private func jsonResponse(_ raw: String) -> MockResponse {
    MockResponse(status: 200, contentType: "application/json", body: Data(raw.utf8))
}

/// JSON-encode an array or dictionary object.
private func jsonEncode(_ object: Any) -> String {
    let data = try! JSONSerialization.data(withJSONObject: object)
    return String(data: data, encoding: .utf8)!
}

/// Parse a JSON object string into `[String: Any]` (empty on failure).
private func parseJSON(_ s: String) -> [String: Any] {
    guard let data = s.data(using: .utf8),
          let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
        return [:]
    }
    return obj
}
