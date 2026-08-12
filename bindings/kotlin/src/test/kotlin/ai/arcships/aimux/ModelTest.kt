package ai.arcships.aimux

import org.assertj.core.api.Assertions.assertThat
import org.assertj.core.api.Assertions.assertThatCode
import org.assertj.core.api.Assertions.assertThatThrownBy
import org.json.JSONArray
import org.json.JSONObject
import org.junit.jupiter.api.Test
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference

class ModelTest {

    @Test
    fun `openai creates model instance`() {
        // Even with a fake key, the provider should construct.
        Model.openai("sk-test-fake-key", "gpt-4o-mini").use { model ->
            assertThat(model).isNotNull
        }
    }

    @Test
    fun `anthropic creates model instance`() {
        Model.anthropic("sk-ant-test-fake-key", "claude-3-5-sonnet-20241022").use { model ->
            assertThat(model).isNotNull
        }
    }

    @Test
    fun `provider creates model instance from registry`() {
        // Registry-backed construction (deepseek is in the provider registry);
        // the key is validated on the first API call, not construction.
        Model.provider("deepseek", apiKey = "sk-test-fake-key", modelId = "deepseek-chat").use { model ->
            assertThat(model).isNotNull
        }
    }

    @Test
    fun `provider creates model with ProviderName constant (recommended)`() {
        // Recommended typed spelling: ProviderName.GROQ constant (key is
        // validated on the first API call, not construction).
        Model.provider(name = ProviderName.GROQ, apiKey = "sk-test-fake-key", modelId = "llama-3.3-70b").use { model ->
            assertThat(model).isNotNull
        }
    }

    @Test
    fun `generateText maps a JSON parse failure to JSONParseError`() {
        Model.openai("sk-test-fake-key", "gpt-4o-mini").use { model ->
            // Prompt JSON parse failures are AIMUX_E_JSON_PARSE (code 2), not Other.
            // Also exercises the real free-path: the FFI-allocated message is
            // read and freed by throwFromC.
            assertThatThrownBy {
                model.generateText("{invalid json}")
            }.isInstanceOf(JSONParseError::class.java)
        }
    }

    @Test
    fun `streamTextSequence surfaces a typed AimuxException on failure`() {
        // Unroutable base URL: the 7-arg stream signature is only checked by
        // JNA at call time, so actually consume the sequence and assert the
        // typed error surfaces.
        Model.openai("sk-test-fake-key", "gpt-4o-mini", "http://127.0.0.1:9").use { model ->
            assertThatThrownBy {
                model.streamTextSequence("\"hello\"").toList()
            }.isInstanceOf(AimuxException::class.java)
        }
    }

    @Test
    fun `streamTextSequence returns a sequence`() {
        Model.openai("sk-test-fake-key", "gpt-4o-mini").use { model ->
            // We don't consume the sequence (would need network),
            // but verify it can be created.
            val seq = model.streamTextSequence("\"hello\"")
            assertThat(seq).isNotNull
        }
    }

    @Test
    fun `unknown provider failure carries lossless errorValue JSON`() {
        // Core-originated failure: error_value is the externally-tagged
        // AiMuxError JSON. throwFromC frees both C strings after mapping.
        assertThatThrownBy {
            Model.provider("definitely-not-a-provider", apiKey = "k", modelId = "m")
        }.isInstanceOf(NoSuchProviderError::class.java)
            .satisfies({ ex ->
                assertThat((ex as AimuxException).errorValue).contains("NoSuchProvider")
            })
    }

    @Test
    fun `FFI-synthesized failure has null errorValue`() {
        // Invalid handle: the failure is synthesized at the FFI boundary,
        // so C error_value is NULL and errorValue maps to Kotlin null.
        val err = AimuxCError()
        val res = FFI.lib.aimux_generate_text(0x7FFF_FFFFL, "\"hi\"", null, err)
        assertThat(res).isNull()
        val ex = AimuxException.fromC(err)
        FFI.lib.aimux_free_string(err.message)
        FFI.lib.aimux_free_string(err.error_value)
        assertThat(ex).isInstanceOf(InvalidArgumentError::class.java)
        assertThat(ex.errorValue).isNull()
    }

    @Test
    fun `model is closeable`() {
        val model = Model.openai("sk-test-fake-key", "gpt-4o-mini")
        model.close()
        // Calling close twice should not crash.
        model.close()
    }

    // ── concurrency (T6: close vs. in-flight FFI call) ─────────────────────

    /**
     * Smoke test for the read/write-lock close-race fix (T6): one thread loops
     * `generateText` (each call holds the read lock for the whole FFI call)
     * while another calls `close()` (write lock). The write lock must wait for
     * the in-flight read to finish, so `close()` never drops a handle out from
     * under an active call — i.e. no use-after-free / native crash. After
     * `close()` completes, further calls must throw a predictable
     * [IllegalStateException]. A use-after-free would crash the JVM (SIGSEGV)
     * and fail the whole test process, not just this method.
     */
    @Test
    fun `concurrent close and generate does not crash`() {
        // Reuses the internal MockProviderServer (same test module/package).
        val server = MockProviderServer()
        server.responseBody = plainOpenAiResponse()
        try {
            val model = Model.openai("sk-test-fake-key", "gpt-4o-mini", server.baseUrl)
            val iterations = 300
            val unexpected = AtomicReference<Throwable?>(null)
            val sawClosed = AtomicBoolean(false)

            // Worker: loop generateText. Pre-close calls succeed against the
            // mock; post-close calls throw IllegalStateException (closed). Any
            // other throwable (incl. a native crash, which surfaces as an Error)
            // is recorded and fails the test.
            val worker = Thread {
                for (i in 0 until iterations) {
                    try {
                        model.generateText("\"Hello\"")
                    } catch (e: IllegalStateException) {
                        sawClosed.set(true)
                    } catch (e: AimuxException) {
                        // A transient transport hiccup during teardown is
                        // acceptable; keep looping.
                    } catch (t: Throwable) {
                        unexpected.set(t)
                        break
                    }
                }
            }.apply { isDaemon = true }

            // Closer: close after a brief delay so it races with in-flight calls.
            val closer = Thread {
                try {
                    Thread.sleep(15)
                } catch (e: InterruptedException) {
                    Thread.currentThread().interrupt()
                }
                model.close()
            }.apply { isDaemon = true }

            worker.start()
            closer.start()
            worker.join(60_000)
            closer.join(5_000)

            assertThat(unexpected.get()).`as`("unexpected exception in worker").isNull()
            // After close, calls must report closed predictably (close-then-use
            // ordering assertion).
            assertThatThrownBy { model.generateText("\"Hello\"") }
                .isInstanceOf(IllegalStateException::class.java)
                .hasMessage("Model is closed")
        } finally {
            server.stop()
        }
    }

    // ── recording (RFC-0023) ────────────────────────────────────────────────

    @Test
    fun `initRecordingRing with null cap uses library default`() {
        // Omitting cap uses the library default capacity (FFI
        // aimux_init_recording_ring_default) and must not throw. This reaches
        // the FFI, so it requires the native library on java.library.path /
        // LD_LIBRARY_PATH (same as the other Model tests in this class).
        assertThatCode { initRecordingRing() }.doesNotThrowAnyException()
        recordingStop()
    }

    // ── canned OpenAI responses ────────────────────────────────────────────

    /** Plain OpenAI chat-completions response (no tool calls). */
    private fun plainOpenAiResponse(): String =
        JSONObject().apply {
            put("id", "chatcmpl-test")
            put("model", "gpt-4o")
            put(
                "choices",
                JSONArray().put(
                    JSONObject()
                        .put(
                            "message",
                            JSONObject()
                                .put("role", "assistant")
                                .put("content", "Rust is a systems programming language."),
                        ).put("finish_reason", "stop"),
                ),
            )
            put(
                "usage",
                JSONObject()
                    .put("prompt_tokens", 10)
                    .put("completion_tokens", 8)
                    .put("total_tokens", 18),
            )
        }.toString()
}
