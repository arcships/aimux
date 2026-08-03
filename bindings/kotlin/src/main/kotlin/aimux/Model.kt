/**
 * aimux — Unified LLM service layer for Kotlin/JVM (Rust core, 172+ providers).
 *
 * Uses JNA to call the aimux-ffi C ABI. This is the C ABI path (§3.2).
 * The native library (libaimux_ffi.so / .dylib / .dll) must be on the
 * library path or bundled in the JAR's native/ directory.
 */

package aimux

import com.sun.jna.Library
import com.sun.jna.Native
import com.sun.jna.Pointer
import java.io.Closeable
import java.util.concurrent.atomic.AtomicLong

// ─────────────────────────────────────────────────────────────────────────────
// JNA interface — direct mapping to the C ABI.
// ─────────────────────────────────────────────────────────────────────────────

internal interface AimuxFFI : Library {
    fun aimux_openai_new(apiKey: String, modelId: String): Long
    fun aimux_anthropic_new(apiKey: String, modelId: String): Long
    fun aimux_openai_new_with_base(apiKey: String, modelId: String, baseUrl: String): Long
    fun aimux_anthropic_new_with_base(apiKey: String, modelId: String, baseUrl: String): Long
    fun aimux_cohere_new(apiKey: String, modelId: String): Long
    fun aimux_cohere_new_with_base(apiKey: String, modelId: String, baseUrl: String): Long
    fun aimux_mistral_new(apiKey: String, modelId: String): Long
    fun aimux_mistral_new_with_base(apiKey: String, modelId: String, baseUrl: String): Long
    fun aimux_xai_new(apiKey: String, modelId: String): Long
    fun aimux_xai_new_with_base(apiKey: String, modelId: String, baseUrl: String): Long
    fun aimux_bedrock_new(accessKeyId: String, secretAccessKey: String, region: String, modelId: String): Long
    fun aimux_bedrock_new_with_base(
        accessKeyId: String, secretAccessKey: String, region: String, modelId: String, baseUrl: String
    ): Long
    fun aimux_vertex_new(accessToken: String, project: String, location: String, modelId: String): Long
    fun aimux_vertex_new_with_base(
        accessToken: String, project: String, location: String, modelId: String, baseUrl: String
    ): Long
    fun aimux_anthropic_aws_new(apiKey: String, region: String, modelId: String): Long
    fun aimux_anthropic_aws_new_with_base(apiKey: String, region: String, modelId: String, baseUrl: String): Long
    fun aimux_azure_new(apiKey: String, resourceName: String, deployment: String, apiVersion: String?): Long
    fun aimux_azure_new_with_base(apiKey: String, baseUrl: String, deployment: String, apiVersion: String?): Long

    // ── Registry provider (RFC-0017 phase 4) ───────────────────────────────
    // apiKey may be null (read the provider's env var from the registry entry);
    // configJson may be null (defaults) or a JSON object of ProviderOptions.
    fun aimux_provider_new(name: String, apiKey: String?, modelId: String, configJson: String?): Long
    fun aimux_provider_from_env(name: String, modelId: String): Long

    fun aimux_generate_text(handle: Long, promptJson: String, optsJson: String?): Pointer?
    fun aimux_stream_text(
        handle: Long,
        promptJson: String,
        optsJson: String?,
        onPart: com.sun.jna.Callback?,
        onDone: com.sun.jna.Callback?,
        onError: com.sun.jna.Callback?,
    )

    fun aimux_drop_handle(handle: Long)
    fun aimux_free_string(ptr: Pointer?)

    // ── Embedding ──────────────────────────────────────────────────────────
    fun aimux_openai_embedding_new(apiKey: String, modelId: String): Long
    fun aimux_openai_embedding_new_with_base(apiKey: String, modelId: String, baseUrl: String): Long
    fun aimux_cohere_embedding_new(apiKey: String, modelId: String): Long
    fun aimux_cohere_embedding_new_with_base(apiKey: String, modelId: String, baseUrl: String): Long
    fun aimux_google_embedding_new(apiKey: String, modelId: String): Long
    fun aimux_google_embedding_new_with_base(apiKey: String, modelId: String, baseUrl: String): Long
    fun aimux_embed(handle: Long, valuesJson: String, optsJson: String?): Pointer?

    // ── Speech (TTS) ───────────────────────────────────────────────────────
    fun aimux_openai_speech_new(apiKey: String, modelId: String): Long
    fun aimux_openai_speech_new_with_base(apiKey: String, modelId: String, baseUrl: String): Long
    fun aimux_speech_generate(handle: Long, optsJson: String): Pointer?

    // ── Image ──────────────────────────────────────────────────────────────
    fun aimux_openai_image_new(apiKey: String, modelId: String): Long
    fun aimux_openai_image_new_with_base(apiKey: String, modelId: String, baseUrl: String): Long
    fun aimux_google_image_new(apiKey: String, modelId: String): Long
    fun aimux_google_image_new_with_base(apiKey: String, modelId: String, baseUrl: String): Long
    fun aimux_image_generate(handle: Long, optsJson: String): Pointer?

    // ── Transcription (STT) ────────────────────────────────────────────────
    fun aimux_openai_transcription_new(apiKey: String, modelId: String): Long
    fun aimux_openai_transcription_new_with_base(apiKey: String, modelId: String, baseUrl: String): Long
    fun aimux_transcription_generate(handle: Long, audioBase64: String, mediaType: String, optsJson: String?): Pointer?

    // ── Files ──────────────────────────────────────────────────────────────
    fun aimux_openai_files_new(apiKey: String): Long
    fun aimux_openai_files_new_with_base(apiKey: String, baseUrl: String): Long
    fun aimux_file_upload(handle: Long, dataBase64: String, mediaType: String, optsJson: String?): Pointer?

    // ── Reranking ──────────────────────────────────────────────────────────
    fun aimux_cohere_reranking_new(apiKey: String, modelId: String): Long
    fun aimux_cohere_reranking_new_with_base(apiKey: String, modelId: String, baseUrl: String): Long
    fun aimux_rerank(handle: Long, optsJson: String): Pointer?

    // ── Video ──────────────────────────────────────────────────────────────
    fun aimux_google_video_new(apiKey: String, modelId: String): Long
    fun aimux_google_video_new_with_base(apiKey: String, modelId: String, baseUrl: String): Long
    fun aimux_video_generate(handle: Long, optsJson: String): Pointer?

    // ── Search ─────────────────────────────────────────────────────────────
    fun aimux_tavily_search_new(apiKey: String, modelId: String): Long
    fun aimux_tavily_search_new_with_base(apiKey: String, modelId: String, baseUrl: String): Long
    fun aimux_search(handle: Long, optsJson: String): Pointer?

    // Logging (RFC-0014).

    fun aimux_init_logging(level: String): Int
}

internal object FFI {
    val lib: AimuxFFI = Native.load("aimux_ffi", AimuxFFI::class.java)
}

// ─────────────────────────────────────────────────────────────────────────────
// Model — Closeable wrapper around a C ABI handle.
// ─────────────────────────────────────────────────────────────────────────────

/**
 * A model instance backed by a Rust `Arc<dyn LanguageModel>`.
 *
 * Implements [Closeable] — you MUST call [close] (or use `use {}`) to release
 * the native handle and avoid memory leaks.
 *
 * ```kotlin
 * Model.openai("sk-...", "gpt-4o-mini").use { model ->
 *     val result = model.generateText("\"Hello!\"")
 * }
 * ```
 */
class Model private constructor(handle: Long) : Closeable {
    private val handle = AtomicLong(handle)

    override fun close() {
        val h = handle.getAndSet(0L)
        if (h != 0L) {
            FFI.lib.aimux_drop_handle(h)
        }
    }

    private fun requireHandle(): Long = handle.get().also {
        check(it != 0L) { "Model is closed" }
    }

    protected fun finalize() {
        close()
    }

    // ── Provider constructors ──────────────────────────────────────────────

    companion object {
        /** Create an OpenAI model instance. */
        fun openai(apiKey: String, modelId: String): Model {
            val h = FFI.lib.aimux_openai_new(apiKey, modelId)
            require(h != 0L) { "Failed to create OpenAI model" }
            return Model(h)
        }

        /** Create an Anthropic model instance. */
        fun anthropic(apiKey: String, modelId: String): Model {
            val h = FFI.lib.aimux_anthropic_new(apiKey, modelId)
            require(h != 0L) { "Failed to create Anthropic model" }
            return Model(h)
        }

        /** Create an OpenAI model instance with a custom base URL. */
        fun openai(apiKey: String, modelId: String, baseUrl: String): Model {
            val h = FFI.lib.aimux_openai_new_with_base(apiKey, modelId, baseUrl)
            require(h != 0L) { "Failed to create OpenAI model" }
            return Model(h)
        }

        /** Create an Anthropic model instance with a custom base URL. */
        fun anthropic(apiKey: String, modelId: String, baseUrl: String): Model {
            val h = FFI.lib.aimux_anthropic_new_with_base(apiKey, modelId, baseUrl)
            require(h != 0L) { "Failed to create Anthropic model" }
            return Model(h)
        }

        /** Create a Cohere model instance. */
        fun cohere(apiKey: String, modelId: String): Model {
            val h = FFI.lib.aimux_cohere_new(apiKey, modelId)
            require(h != 0L) { "Failed to create Cohere model" }
            return Model(h)
        }

        /** Create a Cohere model instance with a custom base URL. */
        fun cohere(apiKey: String, modelId: String, baseUrl: String): Model {
            val h = FFI.lib.aimux_cohere_new_with_base(apiKey, modelId, baseUrl)
            require(h != 0L) { "Failed to create Cohere model" }
            return Model(h)
        }

        /** Create a Mistral model instance. */
        fun mistral(apiKey: String, modelId: String): Model {
            val h = FFI.lib.aimux_mistral_new(apiKey, modelId)
            require(h != 0L) { "Failed to create Mistral model" }
            return Model(h)
        }

        /** Create a Mistral model instance with a custom base URL. */
        fun mistral(apiKey: String, modelId: String, baseUrl: String): Model {
            val h = FFI.lib.aimux_mistral_new_with_base(apiKey, modelId, baseUrl)
            require(h != 0L) { "Failed to create Mistral model" }
            return Model(h)
        }

        /** Create an xAI model instance. */
        fun xai(apiKey: String, modelId: String): Model {
            val h = FFI.lib.aimux_xai_new(apiKey, modelId)
            require(h != 0L) { "Failed to create xAI model" }
            return Model(h)
        }

        /** Create an xAI model instance with a custom base URL. */
        fun xai(apiKey: String, modelId: String, baseUrl: String): Model {
            val h = FFI.lib.aimux_xai_new_with_base(apiKey, modelId, baseUrl)
            require(h != 0L) { "Failed to create xAI model" }
            return Model(h)
        }

        /** Create a Bedrock model instance (AWS SigV4 credentials). */
        fun bedrock(accessKeyId: String, secretAccessKey: String, region: String, modelId: String): Model {
            val h = FFI.lib.aimux_bedrock_new(accessKeyId, secretAccessKey, region, modelId)
            require(h != 0L) { "Failed to create Bedrock model" }
            return Model(h)
        }

        /** Create a Bedrock model instance with a custom base URL. */
        fun bedrock(
            accessKeyId: String, secretAccessKey: String, region: String, modelId: String, baseUrl: String
        ): Model {
            val h = FFI.lib.aimux_bedrock_new_with_base(accessKeyId, secretAccessKey, region, modelId, baseUrl)
            require(h != 0L) { "Failed to create Bedrock model" }
            return Model(h)
        }

        /** Create a Vertex AI model instance (GCP bearer token). */
        fun vertex(accessToken: String, project: String, location: String, modelId: String): Model {
            val h = FFI.lib.aimux_vertex_new(accessToken, project, location, modelId)
            require(h != 0L) { "Failed to create Vertex model" }
            return Model(h)
        }

        /** Create a Vertex AI model instance with a custom base URL. */
        fun vertex(
            accessToken: String, project: String, location: String, modelId: String, baseUrl: String
        ): Model {
            val h = FFI.lib.aimux_vertex_new_with_base(accessToken, project, location, modelId, baseUrl)
            require(h != 0L) { "Failed to create Vertex model" }
            return Model(h)
        }

        /** Create an Anthropic-on-AWS model instance (API key + region). */
        fun anthropicAws(apiKey: String, region: String, modelId: String): Model {
            val h = FFI.lib.aimux_anthropic_aws_new(apiKey, region, modelId)
            require(h != 0L) { "Failed to create Anthropic AWS model" }
            return Model(h)
        }

        /** Create an Anthropic-on-AWS model instance with a custom base URL. */
        fun anthropicAws(apiKey: String, region: String, modelId: String, baseUrl: String): Model {
            val h = FFI.lib.aimux_anthropic_aws_new_with_base(apiKey, region, modelId, baseUrl)
            require(h != 0L) { "Failed to create Anthropic AWS model" }
            return Model(h)
        }

        /** Create an Azure OpenAI model instance (API key + resource name). */
        fun azure(apiKey: String, resourceName: String, deployment: String): Model {
            val h = FFI.lib.aimux_azure_new(apiKey, resourceName, deployment, null)
            require(h != 0L) { "Failed to create Azure model" }
            return Model(h)
        }

        /** Create an Azure OpenAI model instance with an explicit api-version. */
        fun azureWithVersion(apiKey: String, resourceName: String, deployment: String, apiVersion: String): Model {
            val h = FFI.lib.aimux_azure_new(apiKey, resourceName, deployment, apiVersion)
            require(h != 0L) { "Failed to create Azure model" }
            return Model(h)
        }

        /** Create an Azure OpenAI model instance with a custom base URL. */
        fun azureWithBase(apiKey: String, baseUrl: String, deployment: String): Model {
            val h = FFI.lib.aimux_azure_new_with_base(apiKey, baseUrl, deployment, null)
            require(h != 0L) { "Failed to create Azure model" }
            return Model(h)
        }

        /**
         * Create a model from the provider registry by name (RFC-0017 phase 4).
         *
         * @param name       Registry provider name (e.g. `"deepseek"`, `"groq"`).
         * @param apiKey     API key, or null to read the provider's env var from
         *                   the registry entry.
         * @param modelId    Model id.
         * @param configJson Optional JSON object of ProviderOptions
         *                   (`{"base_url": "...", "headers": {...}, "max_retries": 0,
         *                   "body_overrides": {...}}`); null for defaults.
         */
        fun provider(name: String, apiKey: String? = null, modelId: String, configJson: String? = null): Model {
            val h = FFI.lib.aimux_provider_new(name, apiKey, modelId, configJson)
            require(h != 0L) { "Failed to create provider model '$name'" }
            return Model(h)
        }

        /** Create a model from the provider registry, reading the API key from the provider's env var. */
        fun providerFromEnv(name: String, modelId: String): Model {
            val h = FFI.lib.aimux_provider_from_env(name, modelId)
            require(h != 0L) { "Failed to create provider model '$name' from env" }
            return Model(h)
        }
    }

    // ── Generation ─────────────────────────────────────────────────────────

    /**
     * Generate text (non-streaming).
     *
     * @param promptJson JSON prompt string (bare value or {"prompt": ...}).
     * @param optsJson Optional JSON-serialized GenerateTextOptions.
     * @return JSON-serialized GenerateTextResult (or {"error":"..."} on failure).
     */
    fun generateText(promptJson: String, optsJson: String? = null): String {
        val ptr = FFI.lib.aimux_generate_text(requireHandle(), promptJson, optsJson)
            ?: throw RuntimeException("generate_text returned null")

        try {
            return ptr.getString(0, "UTF-8")
        } finally {
            FFI.lib.aimux_free_string(ptr)
        }
    }

    /**
     * Stream text from the model.
     *
     * Blocks the calling thread until the stream completes.
     *
     * @param promptJson JSON prompt string.
     * @param optsJson Optional JSON-serialized GenerateTextOptions.
     * @param onPart Called for each StreamPart (JSON string).
     * @param onDone Called when the stream completes normally.
     * @param onError Called on a stream error (JSON error string).
     */
    fun streamText(
        promptJson: String,
        optsJson: String? = null,
        onPart: (String) -> Unit,
        onDone: () -> Unit,
        onError: (String) -> Unit,
    ) {
        // JNA callbacks — must be held in variables to prevent GC.
        val partCb = object : com.sun.jna.Callback {
            @Suppress("unused")
            fun callback(jsonPtr: Pointer?) {
                if (jsonPtr != null) {
                    onPart(jsonPtr.getString(0, "UTF-8"))
                }
            }
        }
        val doneCb = object : com.sun.jna.Callback {
            @Suppress("unused")
            fun callback() {
                onDone()
            }
        }
        val errCb = object : com.sun.jna.Callback {
            @Suppress("unused")
            fun callback(errPtr: Pointer?) {
                if (errPtr != null) {
                    onError(errPtr.getString(0, "UTF-8"))
                }
            }
        }

        FFI.lib.aimux_stream_text(requireHandle(), promptJson, optsJson, partCb, doneCb, errCb)
    }

    /**
     * Stream text as a Sequence of StreamPart JSON strings.
     *
     * Usage:
     * ```kotlin
     * model.streamTextSequence("\"Write a haiku\"").forEach { part ->
     *     println(part)
     * }
     * ```
     */
    fun streamTextSequence(
        promptJson: String,
        optsJson: String? = null,
    ): Sequence<String> = sequence {
        val parts = java.util.concurrent.LinkedBlockingQueue<String?>()
        var error: String? = null

        streamText(
            promptJson = promptJson,
            optsJson = optsJson,
            onPart = { parts.put(it) },
            onDone = { parts.put(null) }, // sentinel = end
            onError = { error = it; parts.put(null) },
        )

        while (true) {
            val part = parts.take() ?: break
            yield(part)
        }

        error?.let { throw RuntimeException(it) }
    }
}
