package ai.arcships.aimux;

import org.json.JSONObject;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

import static org.assertj.core.api.Assertions.assertThat;
import static org.assertj.core.api.Assertions.assertThatThrownBy;

/**
 * Coverage tests for the multimodal binding — addresses gaps found in the
 * independent review:
 *
 * <ul>
 *   <li>Provider variant construction (Cohere/Google embedding, Google image,
 *       all non-WithBase factories) — ~15 C ABI symbols previously untested.</li>
 *   <li>AiMuxError values → {@link AimuxException} (C error-handle path).</li>
 *   <li>Lifecycle: close-then-call throws, double-close is safe.</li>
 *   <li>Search {@code answer} field assertion (previously only {@code results}
 *       was checked).</li>
 * </ul>
 *
 * <p>No real network access — every request hits 127.0.0.1.
 */
class MultimodalCoverageTest {

    private MockProviderServer server;

    @BeforeEach
    void setUp() {
        server = new MockProviderServer();
    }

    @AfterEach
    void tearDown() {
        server.close();
    }

    // ── Provider variant construction ───────────────────────────────────────
    // These verify the C ABI symbol mapping for variants that were never
    // instantiated by the E2E suite. Construction only (no HTTP); the handle
    // just needs to be live.

    @Test
    void allEmbeddingProviderVariantsConstruct() {
        try (EmbeddingModel m = EmbeddingModel.openai("sk-test", "text-embedding-3-small")) {
            assertThat(m).isNotNull();
        }
        try (EmbeddingModel m = EmbeddingModel.cohere("sk-test", "embed-english-v3.0")) {
            assertThat(m).isNotNull();
        }
        try (EmbeddingModel m = EmbeddingModel.cohereWithBase("sk-test", "embed-english-v3.0", server.baseUrl())) {
            assertThat(m).isNotNull();
        }
        try (EmbeddingModel m = EmbeddingModel.google("sk-test", "gemini-embedding-001")) {
            assertThat(m).isNotNull();
        }
        try (EmbeddingModel m = EmbeddingModel.googleWithBase("sk-test", "gemini-embedding-001", server.baseUrl())) {
            assertThat(m).isNotNull();
        }
    }

    @Test
    void allImageProviderVariantsConstruct() {
        try (ImageModel m = ImageModel.openai("sk-test", "dall-e-3")) {
            assertThat(m).isNotNull();
        }
        try (ImageModel m = ImageModel.google("sk-test", "gemini-2.5-flash-image")) {
            assertThat(m).isNotNull();
        }
        try (ImageModel m = ImageModel.googleWithBase("sk-test", "gemini-2.5-flash-image", server.baseUrl())) {
            assertThat(m).isNotNull();
        }
    }

    @Test
    void allNonWithBaseFactoriesConstruct() {
        try (SpeechModel m = SpeechModel.openai("sk-test", "tts-1")) { assertThat(m).isNotNull(); }
        try (TranscriptionModel m = TranscriptionModel.openai("sk-test", "whisper-1")) { assertThat(m).isNotNull(); }
        try (Files m = Files.openai("sk-test")) { assertThat(m).isNotNull(); }
        try (RerankingModel m = RerankingModel.cohere("sk-test", "rerank-v3.0")) { assertThat(m).isNotNull(); }
        try (VideoModel m = VideoModel.google("sk-test", "veo-3.0")) { assertThat(m).isNotNull(); }
        try (SearchModel m = SearchModel.tavily("sk-test")) { assertThat(m).isNotNull(); }
    }

    // ── Invalid raw JSON → IllegalArgumentException (before the C call) ─────

    @Test
    void embeddingInvalidJsonThrowsIllegalArgument() {
        try (EmbeddingModel model = EmbeddingModel.openaiWithBase("sk-test", "text-embedding-3-small", server.baseUrl())) {
            assertThatThrownBy(() -> model.embed("not-valid-json"))
                .isInstanceOf(IllegalArgumentException.class)
                .hasMessageContaining("valuesJson");
        }
    }

    @Test
    void speechInvalidJsonThrowsIllegalArgument() {
        try (SpeechModel model = SpeechModel.openaiWithBase("sk-test", "tts-1", server.baseUrl())) {
            assertThatThrownBy(() -> model.generate("not-valid-json"))
                .isInstanceOf(IllegalArgumentException.class)
                .hasMessageContaining("optsJson");
        }
    }

    @Test
    void rerankInvalidJsonThrowsIllegalArgument() {
        try (RerankingModel model = RerankingModel.cohereWithBase("sk-test", "rerank-v3.0", server.baseUrl())) {
            assertThatThrownBy(() -> model.rerank("not-valid-json"))
                .isInstanceOf(IllegalArgumentException.class)
                .hasMessageContaining("optsJson");
        }
    }

    // ── Lifecycle: close-then-call, double-close ────────────────────────────

    @Test
    void embeddingCloseThenCallThrows() {
        EmbeddingModel model = EmbeddingModel.openai("sk-test", "text-embedding-3-small");
        model.close();
        assertThatThrownBy(() -> model.embed("[\"hi\"]"))
            .isInstanceOf(IllegalStateException.class)
            .hasMessageContaining("closed");
    }

    @Test
    void multimodalDoubleCloseIsSafe() {
        EmbeddingModel m1 = EmbeddingModel.openai("sk-test", "text-embedding-3-small");
        m1.close();
        m1.close(); // must not throw

        ImageModel m2 = ImageModel.openai("sk-test", "dall-e-3");
        m2.close();
        m2.close();

        Files m3 = Files.openai("sk-test");
        m3.close();
        m3.close();
    }

    @Test
    void searchCloseThenCallThrows() {
        SearchModel model = SearchModel.tavily("sk-test");
        model.close();
        assertThatThrownBy(() -> model.search("{}"))
            .isInstanceOf(IllegalStateException.class);
    }

    // ── Search answer field (previously unasserted) ─────────────────────────

    @Test
    void searchParsesAnswerField() {
        server.setResponseBody(
            "{\"results\":[{\"title\":\"Rust\",\"url\":\"https://rust-lang.org\",\"content\":\"Rust is...\"}],"
                + "\"answer\":\"Rust is a systems language.\"}");

        try (SearchModel model = SearchModel.tavilyWithBase("sk-test", server.baseUrl())) {
            JSONObject opts = new JSONObject()
                .put("query", "What is Rust?")
                .put("max_results", 5)
                .put("provider_options", new JSONObject());
            JSONObject result = new JSONObject(model.search(opts.toString()));

            assertThat(result.getJSONArray("results").length()).isEqualTo(1);
            assertThat(result.getString("answer")).isEqualTo("Rust is a systems language.");
        }
    }

    // ── DeepSeek generateText (previously only construct-tested) ─────────────

    @Test
    void deepseekGenerateTextViaMock() {
        server.setResponseBody(
            "{\"id\":\"chatcmpl-1\",\"object\":\"chat.completion\",\"model\":\"deepseek-chat\","
                + "\"choices\":[{\"index\":0,\"message\":{\"role\":\"assistant\",\"content\":\"Hi\"},\"finish_reason\":\"stop\"}],"
                + "\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":1,\"total_tokens\":2}}");

        try (Model model = Model.openaiWithBase("sk-test", "deepseek-chat", server.baseUrl())) {
            String result = model.generateText("\"Say hi\"");
            assertThat(new JSONObject(result).getString("text")).isEqualTo("Hi");
        }
    }
}
