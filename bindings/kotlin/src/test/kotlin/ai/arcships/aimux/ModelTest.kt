package ai.arcships.aimux

import org.assertj.core.api.Assertions.assertThat
import org.assertj.core.api.Assertions.assertThatThrownBy
import org.junit.jupiter.api.Test

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
    fun `generateText maps a JSON parse failure to JsonError`() {
        Model.openai("sk-test-fake-key", "gpt-4o-mini").use { model ->
            // Prompt JSON parse failures are AIMUX_E_JSON (code 4), not Other.
            // Also exercises the real free-path: the FFI-allocated message is
            // read and freed by throwFromC.
            assertThatThrownBy {
                model.generateText("{invalid json}")
            }.isInstanceOf(JsonError::class.java)
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
        }.isInstanceOf(UnknownProviderError::class.java)
            .satisfies({ ex ->
                assertThat((ex as AimuxException).errorValue).contains("UnknownProvider")
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
}
