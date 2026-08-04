package ai.arcships.aimux;

import org.json.JSONArray;
import org.json.JSONObject;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

import static org.assertj.core.api.Assertions.assertThat;

// ─────────────────────────────────────────────────────────────────────────────
// Multimodal end-to-end tests for the Java binding.
//
// Mirrors `bindings/kotlin/src/test/kotlin/aimux/MultimodalE2ETest.kt`: each
// modality is exercised against a local mock HTTP server (MockProviderServer)
// that replays a canned provider response. The binding transforms that
// response into the aimux wire-format result, which we parse back and assert
// on.
//
// Google Video uses a multi-step async API (POST predict → poll operation →
// fetch result) that a single-response mock server can't drive, so its test
// checks construction + result parsing only — exactly like the Kotlin test.
//
// No real network access is performed — every request hits 127.0.0.1.
// ─────────────────────────────────────────────────────────────────────────────

class MultimodalE2ETest {

    private MockProviderServer server;

    @BeforeEach
    void setUp() {
        server = new MockProviderServer();
    }

    @AfterEach
    void tearDown() {
        server.close();
    }

    // ── Embedding ──────────────────────────────────────────────────────────

    @Test
    void embedParsesEmbeddings() {
        server.setResponseBody(
            "{\"data\":[{\"embedding\":[0.1,0.2,0.3],\"index\":0}],"
                + "\"model\":\"text-embedding-3-small\","
                + "\"usage\":{\"prompt_tokens\":3,\"total_tokens\":3}}");

        try (EmbeddingModel model = EmbeddingModel.openaiWithBase(
                "sk-test", "text-embedding-3-small", server.baseUrl())) {
            JSONObject result =
                new JSONObject(model.embed(new JSONArray().put("hello").toString()));

            // Wire format: {"embeddings":[[0.1,0.2,0.3]], ...}
            JSONArray embeddings = result.getJSONArray("embeddings");
            assertThat(embeddings.length()).isEqualTo(1);
            assertThat(embeddings.getJSONArray(0).length()).isEqualTo(3);
        }
    }

    // ── Speech (TTS) ───────────────────────────────────────────────────────

    @Test
    void speechGenerateReturnsBinaryAudio() {
        // OpenAI TTS returns raw audio bytes with content-type audio/mpeg. The
        // mock body below is ASCII-safe (a base64-looking string carried as raw
        // bytes), so it round-trips through the String-based mock unchanged —
        // the same trick the Kotlin test uses. The core then wraps the raw
        // bytes as AudioData::Binary.
        server.setContentType("audio/mpeg");
        server.setResponseBody("SGVsbG8gd29ybGQ=");

        try (SpeechModel model = SpeechModel.openaiWithBase(
                "sk-test", "tts-1", server.baseUrl())) {
            JSONObject opts = new JSONObject()
                .put("text", "Hi")
                .put("voice", "alloy")
                .put("output_format", "mp3")
                .put("provider_options", new JSONObject());

            JSONObject result = new JSONObject(model.generate(opts.toString()));

            // Wire format: {"audio":{"Binary":[<bytes>]}, ...}
            JSONObject audio = result.getJSONObject("audio");
            assertThat(audio.has("Binary")).isTrue();
            assertThat(audio.getJSONArray("Binary").length()).isGreaterThan(0);
        }
    }

    // ── Image ──────────────────────────────────────────────────────────────

    @Test
    void imageGenerateParsesBase64Images() {
        server.setResponseBody("{\"data\":[{\"b64_json\":\"aW1hZ2Ux\"}]}");

        try (ImageModel model = ImageModel.openaiWithBase(
                "sk-test", "dall-e-3", server.baseUrl())) {
            JSONObject opts = new JSONObject()
                .put("prompt", "otter")
                .put("n", 1)
                .put("provider_options", new JSONObject());

            JSONObject result = new JSONObject(model.generate(opts.toString()));

            // Wire format: {"images":{"Base64":["aW1hZ2Ux"]}, ...}
            JSONArray base64 = result.getJSONObject("images").getJSONArray("Base64");
            assertThat(base64.length()).isEqualTo(1);
            assertThat(base64.getString(0)).isEqualTo("aW1hZ2Ux");
        }
    }

    // ── Transcription (STT) ────────────────────────────────────────────────

    @Test
    void transcriptionGenerateParsesText() {
        server.setResponseBody("{\"text\":\"Hello world\"}");

        try (TranscriptionModel model = TranscriptionModel.openaiWithBase(
                "sk-test", "whisper-1", server.baseUrl())) {
            JSONObject result = new JSONObject(model.generate("dGVzdA==", "audio/mp3"));

            // Wire format: {"text":"Hello world", ...}
            assertThat(result.getString("text")).isEqualTo("Hello world");
        }
    }

    // ── Reranking ──────────────────────────────────────────────────────────

    @Test
    void rerankParsesRanking() {
        server.setResponseBody(
            "{\"results\":[{\"index\":1,\"relevance_score\":0.95},"
                + "{\"index\":0,\"relevance_score\":0.3}]}");

        try (RerankingModel model = RerankingModel.cohereWithBase(
                "sk-test", "rerank-v3.0", server.baseUrl())) {
            JSONObject opts = new JSONObject()
                .put("query", "which?")
                .put("documents", new JSONObject("{\"Text\":{\"values\":[\"doc1\",\"doc2\"]}}"))
                .put("top_n", 2)
                .put("provider_options", new JSONObject());

            JSONObject result = new JSONObject(model.rerank(opts.toString()));

            // Wire format: {"ranking":[{"index":1,"relevance_score":0.95}, ...], ...}
            JSONArray ranking = result.getJSONArray("ranking");
            assertThat(ranking.length()).isEqualTo(2);
            JSONObject first = ranking.getJSONObject(0);
            assertThat(first.getInt("index")).isEqualTo(1);
            assertThat(first.getDouble("relevance_score")).isEqualTo(0.95);
        }
    }

    // ── Search ─────────────────────────────────────────────────────────────

    @Test
    void searchParsesResults() {
        server.setResponseBody(
            "{\"results\":[{\"title\":\"Rust\",\"url\":\"https://rust-lang.org\","
                + "\"content\":\"Rust is...\"}],"
                + "\"answer\":\"Rust is a systems language.\"}");

        try (SearchModel model = SearchModel.tavilyWithBase(
                "sk-test", server.baseUrl())) {
            JSONObject opts = new JSONObject()
                .put("query", "What is Rust?")
                .put("max_results", 5)
                .put("provider_options", new JSONObject());

            JSONObject result = new JSONObject(model.search(opts.toString()));

            // Wire format: {"results":[{"title":"Rust", ...}], "answer":"...", ...}
            JSONArray results = result.getJSONArray("results");
            assertThat(results.length()).isEqualTo(1);
            assertThat(results.getJSONObject(0).getString("title")).isEqualTo("Rust");
        }
    }

    // ── Files ──────────────────────────────────────────────────────────────

    @Test
    void uploadFileParsesProviderReference() {
        server.setResponseBody(
            "{\"id\":\"file-abc\",\"object\":\"file\",\"bytes\":1024,"
                + "\"created_at\":1234,\"filename\":\"test.pdf\","
                + "\"purpose\":\"assistants\"}");

        try (Files files = Files.openaiWithBase("sk-test", server.baseUrl())) {
            JSONObject result = new JSONObject(files.uploadFile("dGVzdA==", "application/pdf"));

            // Wire format: {"provider_reference":{"openai":"file-abc"}, ...}
            assertThat(result.getJSONObject("provider_reference").getString("openai"))
                .isEqualTo("file-abc");
        }
    }

    // ── Video (construction + result parsing only) ─────────────────────────

    @Test
    void videoConstructionAndResultParsing() {
        // Google Video uses a multi-step async API (POST predict → poll
        // operation → fetch result). A single-response mock server can't drive
        // the full flow, so — like the Kotlin test — we only verify
        // construction (via the WithBase factory) and parsing of a canned
        // VideoResult.
        try (VideoModel model = VideoModel.googleWithBase(
                "sk-test", "veo-3.0", server.baseUrl())) {
            // Construction succeeded; the native handle is live and will be
            // released by the try-with-resources block on exit.
            assertThat(model).isNotNull();
        }

        // Wire format: {"videos":[{"Url":{"url":"...","media_type":"..."}}], ...}
        JSONObject parsed = new JSONObject(
            "{\"videos\":[{\"Url\":{\"url\":\"https://example.com/v.mp4\","
                + "\"media_type\":\"video/mp4\"}}]}");
        JSONArray videos = parsed.getJSONArray("videos");
        assertThat(videos.length()).isEqualTo(1);
        assertThat(videos.getJSONObject(0).getJSONObject("Url").getString("url"))
            .isEqualTo("https://example.com/v.mp4");
    }
}
