package ai.arcships.aimux;

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
        // handle is guaranteed by the factory (it throws AimuxException
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

    @Test
    void providerCreatesModelWithProviderNameConstant() {
        // Recommended typed spelling: ProviderName.GROQ constant (key is
        // validated on the first API call, not construction).
        try (Model model = Model.provider(ProviderName.GROQ, "sk-test-fake-key", "llama-3.3-70b", null)) {
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
            // Invalid prompts fail via C AimuxError → AimuxException (no JSON envelope).
            assertThatThrownBy(() -> model.generateText("{invalid json}"))
                .isInstanceOf(AimuxException.class);
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
            Thread streamThread = new Thread(() -> {
                try {
                    model.streamText("\"Hello\"", null, parts::add, () -> done.set(true));
                } catch (RuntimeException e) {
                    error.set(e.getMessage());
                }
            });
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

    // ── concurrency (T6: close vs. in-flight FFI call) ─────────────────────

    /**
     * Smoke test for the read/write-lock close-race fix (T6): one thread loops
     * {@code generateText} (each call holds the read lock for the whole FFI
     * call) while another calls {@code close()} (write lock). The write lock
     * must wait for the in-flight read to finish, so {@code close()} never drops
     * a handle out from under an active call — i.e. no use-after-free / native
     * crash. After {@code close()} completes, further calls must throw a
     * predictable {@link IllegalStateException}. A use-after-free would crash
     * the JVM (SIGSEGV) and fail the whole test process, not just this method.
     */
    @Test
    void concurrentCloseAndGenerateDoesNotCrash() throws InterruptedException {
        try (MockProviderServer server = new MockProviderServer();
             Model model = Model.openaiWithBase("sk-test-fake-key", "gpt-4o-mini", server.baseUrl())) {
            server.setResponseBody(plainOpenAiResponse());

            final int iterations = 300;
            final AtomicReference<Throwable> unexpected = new AtomicReference<>(null);
            final AtomicBoolean sawClosed = new AtomicBoolean(false);

            // Worker: loop generateText. Pre-close calls succeed against the
            // mock; post-close calls throw IllegalStateException (closed). Any
            // other throwable (incl. a native crash, which surfaces as an Error)
            // is recorded and fails the test.
            Thread worker = new Thread(() -> {
                for (int i = 0; i < iterations; i++) {
                    try {
                        model.generateText("\"Hello\"");
                    } catch (IllegalStateException e) {
                        sawClosed.set(true);
                    } catch (AimuxException e) {
                        // A transient transport hiccup during teardown is
                        // acceptable; keep looping.
                    } catch (Throwable t) {
                        unexpected.set(t);
                        break;
                    }
                }
            });
            worker.setDaemon(true);
            worker.start();

            // Closer: close after a brief delay so it races with in-flight calls.
            Thread closer = new Thread(() -> {
                try {
                    Thread.sleep(15);
                } catch (InterruptedException e) {
                    Thread.currentThread().interrupt();
                }
                model.close();
            });
            closer.setDaemon(true);
            closer.start();

            worker.join(60_000);
            closer.join(5_000);

            assertThat(unexpected.get()).as("unexpected exception in worker").isNull();
            // After close, calls must report closed predictably (close-then-use
            // ordering assertion).
            assertThatThrownBy(() -> model.generateText("\"Hello\""))
                .isInstanceOf(IllegalStateException.class)
                .hasMessage("Model is closed");
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
