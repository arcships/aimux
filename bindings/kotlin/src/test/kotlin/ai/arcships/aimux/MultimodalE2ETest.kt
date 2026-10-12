package ai.arcships.aimux

import org.assertj.core.api.Assertions.assertThat
import org.assertj.core.api.Assertions.assertThatThrownBy
import org.json.JSONArray
import org.json.JSONObject
import org.junit.jupiter.api.AfterEach
import org.junit.jupiter.api.BeforeEach
import org.junit.jupiter.api.Test

// ─────────────────────────────────────────────────────────────────────────────
// Multimodal end-to-end tests for the Kotlin/JVM binding.
//
// Mirrors `bindings/go/multimodal_withbase_test.go`: each modality is exercised
// against a local mock HTTP server (MockProviderServer, defined in
// StructuredE2ETest.kt) that replays a canned provider response. The binding
// transforms that response into the aimux wire-format result, which we parse
// back and assert on.
//
// Google Video uses a multi-step async API (POST predict → poll operation →
// fetch result) that a single-response mock server can't drive, so its test
// checks construction + result parsing only — exactly like the Go test.
//
// No real network access is performed — every request hits 127.0.0.1.
// ─────────────────────────────────────────────────────────────────────────────

class MultimodalE2ETest {

    private lateinit var server: MockProviderServer

    @BeforeEach
    fun setUp() {
        server = MockProviderServer()
    }

    @AfterEach
    fun tearDown() {
        server.stop()
    }

    // ── Embedding ──────────────────────────────────────────────────────────

    @Test
    fun `embed parses embeddings`() {
        server.responseBody =
            """{"data":[{"embedding":[0.1,0.2,0.3],"index":0}],"model":"text-embedding-3-small","usage":{"prompt_tokens":3,"total_tokens":3}}"""

        EmbeddingModel.openai("sk-test", "text-embedding-3-small", server.baseUrl).use { model ->
            val result = JSONObject(model.embed(JSONArray().put("hello").toString()))

            // Wire format: {"embeddings":[[0.1,0.2,0.3]], ...}
            val embeddings = result.getJSONArray("embeddings")
            assertThat(embeddings.length()).isEqualTo(1)
            assertThat(embeddings.getJSONArray(0).length()).isEqualTo(3)
        }
    }

    /** Closed-guard: a call after close() fails predictably, before reaching the C ABI. */
    @Test
    fun `embed after close throws IllegalStateException`() {
        val model = EmbeddingModel.openai("sk-test", "text-embedding-3-small", server.baseUrl)
        model.close()
        assertThatThrownBy { model.embed("[\"hello\"]") }
            .isInstanceOf(IllegalStateException::class.java)
            .hasMessageContaining("is closed")
    }

    // ── Speech (TTS) ───────────────────────────────────────────────────────

    @Test
    fun `speech generate returns binary audio`() {
        // OpenAI TTS returns raw audio bytes with content-type audio/mpeg. The
        // mock body below is ASCII-safe (a base64-looking string carried as raw
        // bytes), so it round-trips through the String-based mock unchanged —
        // the same trick the Go test uses (`base64Audio`). The core then wraps
        // the raw bytes as AudioData::Binary.
        server.contentType = "audio/mpeg"
        server.responseBody = "SGVsbG8gd29ybGQ="

        SpeechModel.openai("sk-test", "tts-1", server.baseUrl).use { model ->
            val opts = AimuxJson.encodeToString(
                SpeechCallOptions.serializer(),
                SpeechCallOptions(text = "Hi", voice = "alloy", outputFormat = "mp3"),
            )

            val result = AimuxJson.decodeFromString(SpeechResult.serializer(), model.generate(opts))

            // Wire format: {"audio":[<bytes>], ...} (untagged: an array is binary).
            val audio = result.audio as AudioData.Binary
            assertThat(audio.value).isNotEmpty()
        }
    }

    // ── Image ──────────────────────────────────────────────────────────────

    @Test
    fun `image generate parses base64 images`() {
        server.responseBody = """{"data":[{"b64_json":"aW1hZ2Ux"}]}"""

        ImageModel.openai("sk-test", "dall-e-3", server.baseUrl).use { model ->
            // `n` and `providerOptions` are required on the wire and always encoded.
            val opts = AimuxJson.encodeToString(
                ImageCallOptions.serializer(),
                ImageCallOptions(prompt = "otter", size = Size(1024, 1024), aspectRatio = AspectRatio(16, 9)),
            )
            assertThat(JSONObject(opts).getString("size")).isEqualTo("1024x1024")
            assertThat(JSONObject(opts).getString("aspectRatio")).isEqualTo("16:9")
            assertThat(JSONObject(opts).has("n")).isTrue()
            assertThat(JSONObject(opts).has("providerOptions")).isTrue()

            val result = AimuxJson.decodeFromString(ImageResult.serializer(), model.generate(opts))

            // Wire format: {"images":["aW1hZ2Ux"], ...} (untagged: strings are base64).
            assertThat(result.images).isEqualTo(ImageOutputs.Base64(listOf("aW1hZ2Ux")))
        }
    }

    // ── Transcription (STT) ────────────────────────────────────────────────

    @Test
    fun `transcription generate parses text`() {
        server.responseBody = """{"text":"Hello world"}"""

        TranscriptionModel.openai("sk-test", "whisper-1", server.baseUrl).use { model ->
            val result = JSONObject(model.generate("dGVzdA==", "audio/mp3"))

            // Wire format: {"text":"Hello world", ...}
            assertThat(result.getString("text")).isEqualTo("Hello world")
        }
    }

    // ── Reranking ──────────────────────────────────────────────────────────

    @Test
    fun `rerank parses ranking`() {
        server.responseBody =
            """{"results":[{"index":1,"relevance_score":0.95},{"index":0,"relevance_score":0.3}]}"""

        RerankingModel.cohere("sk-test", "rerank-v3.0", server.baseUrl).use { model ->
            val opts = AimuxJson.encodeToString(
                RerankingCallOptions.serializer(),
                RerankingCallOptions(
                    documents = RerankingDocuments.Text(listOf("doc1", "doc2")),
                    query = "which?",
                    topN = 2,
                ),
            )
            assertThat(JSONObject(opts).getJSONObject("documents").getString("type")).isEqualTo("text")

            val result = AimuxJson.decodeFromString(RerankingResult.serializer(), model.rerank(opts))

            // Wire format: {"ranking":[{"index":1,"relevanceScore":0.95}, ...], ...}
            assertThat(result.ranking).hasSize(2)
            assertThat(result.ranking[0]).isEqualTo(RerankingRank(index = 1, relevanceScore = 0.95))
        }
    }

    // ── Search ─────────────────────────────────────────────────────────────

    @Test
    fun `search parses results`() {
        server.responseBody =
            """{"results":[{"title":"Rust","url":"https://rust-lang.org","content":"Rust is..."}],"answer":"Rust is a systems language."}"""

        SearchModel.tavily("sk-test", server.baseUrl).use { model ->
            val opts = AimuxJson.encodeToString(
                SearchCallOptions.serializer(),
                SearchCallOptions(query = "What is Rust?", maxResults = 5),
            )

            val result = JSONObject(model.search(opts))

            // Wire format: {"results":[{"title":"Rust", ...}], "answer":"...", ...}
            val results = result.getJSONArray("results")
            assertThat(results.length()).isEqualTo(1)
            assertThat(results.getJSONObject(0).getString("title")).isEqualTo("Rust")
        }
    }

    // ── Files ──────────────────────────────────────────────────────────────

    @Test
    fun `uploadFile parses provider reference`() {
        server.responseBody =
            """{"id":"file-abc","object":"file","bytes":1024,"created_at":1234,"filename":"test.pdf","purpose":"assistants"}"""

        Files.openai("sk-test", server.baseUrl).use { model ->
            val result = JSONObject(model.uploadFile("dGVzdA==", "application/pdf"))

            // Wire format: {"providerReference":{"openai":"file-abc"}, ...}
            assertThat(result.getJSONObject("providerReference").getString("openai"))
                .isEqualTo("file-abc")
        }
    }

    // ── Video (construction + result parsing only) ─────────────────────────

    @Test
    fun `video construction and result parsing`() {
        // Google Video uses a multi-step async API (POST predict → poll
        // operation → fetch result). A single-response mock server can't drive
        // the full flow, so — like the Go test — we only verify construction
        // (via the WithBase factory) and parsing of a canned VideoResult.
        VideoModel.google("sk-test", "veo-3.0", "http://localhost:9999").use { model ->
            // Construction succeeded; the native handle is live and will be
            // released by `use` on exit.
            assertThat(model).isNotNull()
        }

        // Wire format: {"videos":[{"type":"url","url":"...","mediaType":"..."}], ...}
        val parsed = AimuxJson.decodeFromString(
            VideoResult.serializer(),
            """{"videos":[{"type":"url","url":"https://example.com/v.mp4","mediaType":"video/mp4"}]}""",
        )
        assertThat(parsed.videos).containsExactly(VideoData.Url("https://example.com/v.mp4", "video/mp4"))
    }
}
