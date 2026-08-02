package io.aimux;

import org.json.JSONArray;
import org.json.JSONObject;
import org.junit.jupiter.api.Test;

import java.time.Duration;
import java.util.List;
import java.util.concurrent.LinkedBlockingQueue;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicReference;
import java.util.stream.Collectors;

import static org.assertj.core.api.Assertions.assertThat;
import static org.assertj.core.api.Assertions.assertThatThrownBy;
import static org.junit.jupiter.api.Assertions.assertTimeout;

/**
 * Raw JSON layer tests (mirror of the Kotlin binding's ModelTest.kt).
 *
 * <p>Tests that call the native library point an OpenAI model at a local
 * {@link MockProviderServer} (127.0.0.1) replaying canned OpenAI wire-format
 * responses — no real network access.
 */
class ModelTest {

    // ── construction ────────────────────────────────────────────────────────

    @Test
    void openaiCreatesModelInstance() {
        // Even with a fake key, the provider should construct. A non-zero
        // handle is guaranteed by the factory (it throws IllegalArgumentException
        // if the FFI returns 0).
        try (Model model = Model.openai("sk-test-fake-key", "gpt-4o-mini")) {
            assertThat(model).isNotNull();
        }
    }

    @Test
    void anthropicCreatesModelInstance() {
        try (Model model = Model.anthropic("sk-ant-test-fake-key", "claude-3-5-sonnet-20241022")) {
            assertThat(model).isNotNull();
        }
    }

    @Test
    void deepseekCreatesModelInstance() {
        try (Model model = Model.deepseek("sk-test-fake-key", "deepseek-chat")) {
            assertThat(model).isNotNull();
        }
    }

    @Test
    void providerCreatesModelInstanceFromRegistry() {
        // Registry-backed construction (deepseek is in the provider registry);
        // the key is validated on the first API call, not construction.
        try (Model model = Model.provider("deepseek", "sk-test-fake-key", "deepseek-chat", null)) {
            assertThat(model).isNotNull();
        }
    }

    // ── generation ──────────────────────────────────────────────────────────

    @Test
    void generateTextReturnsJsonString() {
        try (MockProviderServer server = new MockProviderServer();
             Model model = Model.openaiWithBase("sk-test-fake-key", "gpt-4o-mini", server.baseUrl())) {
            server.setResponseBody(plainOpenAiResponse());

            String resultJson = model.generateText("\"Hello\"");

            JSONObject result = new JSONObject(resultJson);
            assertThat(result.getString("text"))
                .isEqualTo("Rust is a systems programming language.");
            assertThat(server.lastRequestMethod()).isEqualTo("POST");
            assertThat(server.lastRequestBody()).contains("gpt-4o-mini");
        }
    }

    @Test
    void generateTextRejectsInvalidPrompt() {
        try (Model model = Model.openai("sk-test-fake-key", "gpt-4o-mini")) {
            // Invalid prompts are rejected client-side before any network I/O
            // and surface as the C ABI error envelope ({"error":"..."}).
            String result = model.generateText("{invalid json}");
            assertThat(result).contains("\"error\"");
        }
    }

    @Test
    void closeThenCallThrows() {
        Model model = Model.openai("sk-test-fake-key", "gpt-4o-mini");
        model.close();
        // Calling close twice should not crash (idempotent).
        model.close();

        assertThatThrownBy(() -> model.generateText("\"hello\""))
            .isInstanceOf(IllegalStateException.class)
            .hasMessage("Model is closed");
    }

    // ── streaming ───────────────────────────────────────────────────────────

    @Test
    void streamTextInvokesOnDone() {
        try (MockProviderServer server = new MockProviderServer();
             Model model = Model.openaiWithBase("sk-test-fake-key", "gpt-4o-mini", server.baseUrl())) {
            server.setContentType("text/event-stream");
            server.setResponseBody(openAiSseResponse());

            LinkedBlockingQueue<String> parts = new LinkedBlockingQueue<>();
            AtomicBoolean done = new AtomicBoolean(false);
            AtomicReference<String> error = new AtomicReference<>();

            // Run the blocking FFI stream call on a separate daemon thread so
            // the main thread can observe callbacks as they arrive (same
            // pattern as the Kotlin E2E test).
            Thread streamThread = new Thread(() ->
                model.streamText("\"Hello\"", null, parts::add, () -> done.set(true), error::set));
            streamThread.setDaemon(true);
            streamThread.start();

            assertTimeout(Duration.ofSeconds(30), () -> {
                long deadline = System.currentTimeMillis() + 30_000;
                while (!done.get() && error.get() == null && System.currentTimeMillis() < deadline) {
                    Thread.sleep(50);
                }
            });
            streamThread.join(5_000);

            assertThat(error.get()).as("stream error").isNull();
            assertThat(done.get()).isTrue();
            assertThat(parts).isNotEmpty();
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            throw new RuntimeException(e);
        }
    }

    @Test
    void streamTextStreamIteratesParts() {
        try (MockProviderServer server = new MockProviderServer();
             Model model = Model.openaiWithBase("sk-test-fake-key", "gpt-4o-mini", server.baseUrl())) {
            server.setContentType("text/event-stream");
            server.setResponseBody(openAiSseResponse());

            List<String> parts = assertTimeout(Duration.ofSeconds(30),
                () -> model.streamTextStream("\"Hello\"").collect(Collectors.toList()));

            assertThat(parts).isNotEmpty();
            // Content deltas from the SSE stream surface in the parts.
            assertThat(parts.stream().collect(Collectors.joining())).contains("Hello");
        }
    }

    @Test
    void streamTextStreamReturnsAStream() {
        // Smoke test only: verifies the Stream can be created without network.
        // Full stream consumption is covered by streamTextStreamIteratesParts
        // (mock-server driven) and TypedModelTest.streamTextStreamDecodesTypedParts.
        try (Model model = Model.openai("sk-test-fake-key", "gpt-4o-mini")) {
            assertThat(model.streamTextStream("\"hello\"")).isNotNull();
        }
    }

    // ── canned OpenAI responses ────────────────────────────────────────────

    /** Plain OpenAI chat-completions response (no tool calls). */
    private static String plainOpenAiResponse() {
        return new JSONObject()
            .put("id", "chatcmpl-test")
            .put("model", "gpt-4o")
            .put("choices", new JSONArray().put(
                new JSONObject()
                    .put("message", new JSONObject()
                        .put("role", "assistant")
                        .put("content", "Rust is a systems programming language."))
                    .put("finish_reason", "stop")))
            .put("usage", new JSONObject()
                .put("prompt_tokens", 10)
                .put("completion_tokens", 8)
                .put("total_tokens", 18))
            .toString();
    }

    /** OpenAI SSE stream: two content deltas + finish + [DONE]. */
    private static String openAiSseResponse() {
        StringBuilder sb = new StringBuilder();
        sb.append("data: ").append(new JSONObject()
            .put("id", "1")
            .put("model", "gpt-4o")
            .put("choices", new JSONArray().put(
                new JSONObject().put("delta", new JSONObject()
                    .put("role", "assistant")
                    .put("content", "Hello")))))
            .append("\n\n");
        sb.append("data: ").append(new JSONObject()
            .put("id", "1")
            .put("model", "gpt-4o")
            .put("choices", new JSONArray().put(
                new JSONObject().put("delta", new JSONObject()
                    .put("content", " world")))))
            .append("\n\n");
        sb.append("data: ").append(new JSONObject()
            .put("id", "1")
            .put("model", "gpt-4o")
            .put("choices", new JSONArray().put(
                new JSONObject().put("delta", new JSONObject())
                    .put("finish_reason", "stop")))
            .put("usage", new JSONObject()
                .put("prompt_tokens", 5)
                .put("completion_tokens", 2)
                .put("total_tokens", 7)))
            .append("\n\n");
        sb.append("data: [DONE]\n\n");
        return sb.toString();
    }
}
