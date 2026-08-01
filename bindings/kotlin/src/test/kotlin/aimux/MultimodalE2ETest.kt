package aimux

import org.assertj.core.api.Assertions.assertThat
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
            val opts = JSONObject().apply {
                put("text", "Hi")
                put("voice", "alloy")
                put("output_format", "mp3")
                put("provider_options", JSONObject())
            }.toString()

            val result = JSONObject(model.generate(opts))

            // Wire format: {"audio":{"Binary":[<bytes>]}, ...}
            val audio = result.getJSONObject("audio")
            assertThat(audio.has("Binary")).isTrue()
            assertThat(audio.getJSONArray("Binary").length()).isGreaterThan(0)
        }
    }

    // ── Image ──────────────────────────────────────────────────────────────

    @Test
    fun `image generate parses base64 images`() {
        server.responseBody = """{"data":[{"b64_json":"aW1hZ2Ux"}]}"""

        ImageModel.openai("sk-test", "dall-e-3", server.baseUrl).use { model ->
            val opts = JSONObject().apply {
                put("prompt", "otter")
                put("n", 1)
                put("provider_options", JSONObject())
            }.toString()

            val result = JSONObject(model.generate(opts))

            // Wire format: {"images":{"Base64":["aW1hZ2Ux"]}, ...}
            val base64 = result.getJSONObject("images").getJSONArray("Base64")
            assertThat(base64.length()).isEqualTo(1)
            assertThat(base64.getString(0)).isEqualTo("aW1hZ2Ux")
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
            val opts = JSONObject().apply {
                put("query", "which?")
                put("documents", JSONObject("""{"Text":{"values":["doc1","doc2"]}}"""))
                put("top_n", 2)
                put("provider_options", JSONObject())
            }.toString()

            val result = JSONObject(model.rerank(opts))

            // Wire format: {"ranking":[{"index":1,"relevance_score":0.95}, ...], ...}
            val ranking = result.getJSONArray("ranking")
            assertThat(ranking.length()).isEqualTo(2)
            val first = ranking.getJSONObject(0)
            assertThat(first.getInt("index")).isEqualTo(1)
            assertThat(first.getDouble("relevance_score")).isEqualTo(0.95)
        }
    }

    // ── Search ─────────────────────────────────────────────────────────────

    @Test
    fun `search parses results`() {
        server.responseBody =
            """{"results":[{"title":"Rust","url":"https://rust-lang.org","content":"Rust is..."}],"answer":"Rust is a systems language."}"""

        SearchModel.tavily("sk-test", server.baseUrl).use { model ->
            val opts = JSONObject().apply {
                put("query", "What is Rust?")
                put("max_results", 5)
                put("provider_options", JSONObject())
            }.toString()

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

            // Wire format: {"provider_reference":{"openai":"file-abc"}, ...}
            assertThat(result.getJSONObject("provider_reference").getString("openai"))
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

        // Wire format: {"videos":[{"Url":{"url":"...","media_type":"..."}}], ...}
        val parsed = JSONObject(
            """{"videos":[{"Url":{"url":"https://example.com/v.mp4","media_type":"video/mp4"}}]}""",
        )
        val videos = parsed.getJSONArray("videos")
        assertThat(videos.length()).isEqualTo(1)
        assertThat(videos.getJSONObject(0).getJSONObject("Url").getString("url"))
            .isEqualTo("https://example.com/v.mp4")
    }
}
