package aimux

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
        // Recommended typed spelling: ProviderName.GROQ constant, env-var key.
        Model.provider(name = ProviderName.GROQ, modelId = "llama-3.3-70b").use { model ->
            assertThat(model).isNotNull
        }
    }

    @Test
    fun `generateText rejects invalid prompt`() {
        Model.openai("sk-test-fake-key", "gpt-4o-mini").use { model ->
            assertThatThrownBy {
                model.generateText("{invalid json}")
            }.isInstanceOf(Exception::class.java)
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
    fun `model is closeable`() {
        val model = Model.openai("sk-test-fake-key", "gpt-4o-mini")
        model.close()
        // Calling close twice should not crash.
        model.close()
    }
}
