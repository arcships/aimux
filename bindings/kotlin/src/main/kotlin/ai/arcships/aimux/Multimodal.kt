/**
 * aimux — multimodal model wrappers for Kotlin/JVM (Rust core via the C ABI).
 *
 * Each modality (embedding / speech / image / transcription / files /
 * reranking / video / search) is a [Closeable] class wrapping a native handle,
 * exactly like [Model]. The handle is acquired via a provider-specific factory
 * (companion object) and released via [close]. All cross-boundary data uses
 * JSON strings (base64 for binary), matching the C ABI wire format.
 *
 * Implements [Closeable] — you MUST call [close] (or use `use {}`) to release
 * the native handle and avoid memory leaks.
 *
 * ```kotlin
 * EmbeddingModel.openai("sk-...", "text-embedding-3-small").use { model ->
 *     val result = model.embed("[\"hello\"]")
 * }
 * ```
 *
 * Reference implementation: `bindings/go/multimodal.go`.
 */

package ai.arcships.aimux

import com.sun.jna.Pointer
import java.io.Closeable
import java.util.concurrent.atomic.AtomicLong
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonObject

// ─────────────────────────────────────────────────────────────────────────────
// Shared helpers.
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Common FFI result extraction: copy the C string out of [ptr], free it, and
 * surface an `{"error":"..."}` envelope as [AimuxException].
 *
 * @param ptr     Pointer returned by an `aimux_*` function (null = fatal FFI failure).
 * @param context Short label for the null-pointer error message (e.g. `"embed"`).
 * @return The UTF-8 result string (caller-owned; the C allocation is already freed).
 */
private fun extractFFIString(ptr: Pointer?, context: String): String {
    if (ptr == null) throw RuntimeException("$context returned null")
    return try {
        val result = ptr.getString(0, "UTF-8")
        throwIfErrorEnvelope(result)
        result
    } finally {
        FFI.lib.aimux_free_string(ptr)
    }
}

/**
 * If [result] is a `{"error":"..."}` envelope, throw [AimuxException] with the
 * message. Otherwise return. Mirrors Go's `extractError`.
 */
private fun throwIfErrorEnvelope(result: String) {
    if (result.trimStart().firstOrNull() != '{') return
    val obj = runCatching { AimuxJson.parseToJsonElement(result).jsonObject }.getOrNull() ?: return
    val err = obj["error"] ?: return
    if (err is JsonPrimitive && err.isString) {
        throw AimuxException(err.content)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// EmbeddingModel
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Generates vector embeddings for text. Wraps a Rust `Arc<dyn EmbeddingModel>`.
 *
 * Implements [Closeable] — call [close] (or `use {}`) to release the handle.
 *
 * ```kotlin
 * EmbeddingModel.openai("sk-...", "text-embedding-3-small").use { model ->
 *     val result = model.embed("[\"hello\",\"world\"]")
 * }
 * ```
 */
class EmbeddingModel private constructor(handle: Long) : Closeable {
    private val handle = AtomicLong(handle)

    override fun close() {
        val h = handle.getAndSet(0L)
        if (h != 0L) {
            FFI.lib.aimux_drop_handle(h)
        }
    }

    private fun requireHandle(): Long = handle.get().also {
        check(it != 0L) { "EmbeddingModel is closed" }
    }

    protected fun finalize() {
        close()
    }

    // ── Provider constructors ──────────────────────────────────────────────

    companion object {
        /** Create an OpenAI embedding model (e.g. `text-embedding-3-small`). */
        fun openai(apiKey: String, modelId: String): EmbeddingModel {
            val h = extractHandle(
                FFI.lib.aimux_openai_embedding_new(apiKey, modelId), "Failed to create OpenAI embedding model")
            return EmbeddingModel(h)
        }

        /** Create an OpenAI embedding model with a custom base URL. */
        fun openai(apiKey: String, modelId: String, baseUrl: String): EmbeddingModel {
            val h = extractHandle(
                FFI.lib.aimux_openai_embedding_new_with_base(apiKey, modelId, baseUrl),
                "Failed to create OpenAI embedding model")
            return EmbeddingModel(h)
        }

        /** Create a Cohere embedding model (e.g. `embed-english-v3.0`). */
        fun cohere(apiKey: String, modelId: String): EmbeddingModel {
            val h = extractHandle(
                FFI.lib.aimux_cohere_embedding_new(apiKey, modelId), "Failed to create Cohere embedding model")
            return EmbeddingModel(h)
        }

        /** Create a Cohere embedding model with a custom base URL. */
        fun cohere(apiKey: String, modelId: String, baseUrl: String): EmbeddingModel {
            val h = extractHandle(
                FFI.lib.aimux_cohere_embedding_new_with_base(apiKey, modelId, baseUrl),
                "Failed to create Cohere embedding model")
            return EmbeddingModel(h)
        }

        /** Create a Google embedding model (e.g. `gemini-embedding-001`). */
        fun google(apiKey: String, modelId: String): EmbeddingModel {
            val h = extractHandle(
                FFI.lib.aimux_google_embedding_new(apiKey, modelId), "Failed to create Google embedding model")
            return EmbeddingModel(h)
        }

        /** Create a Google embedding model with a custom base URL. */
        fun google(apiKey: String, modelId: String, baseUrl: String): EmbeddingModel {
            val h = extractHandle(
                FFI.lib.aimux_google_embedding_new_with_base(apiKey, modelId, baseUrl),
                "Failed to create Google embedding model")
            return EmbeddingModel(h)
        }
    }

    // ── Embedding ──────────────────────────────────────────────────────────

    /**
     * Generate embeddings for the given text values.
     *
     * @param valuesJson JSON array of strings to embed (e.g. `["a","b"]`).
     * @param optsJson   Optional JSON-serialized `EmbeddingCallOptions`.
     * @return JSON-serialized `EmbeddingResult` (or throws on `{"error":"..."}`).
     */
    fun embed(valuesJson: String, optsJson: String? = null): String {
        val ptr = FFI.lib.aimux_embed(requireHandle(), valuesJson, optsJson)
        return extractFFIString(ptr, "embed")
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// SpeechModel (TTS)
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Converts text to speech audio. Wraps a Rust `Arc<dyn SpeechModel>`.
 *
 * Implements [Closeable] — call [close] (or `use {}`) to release the handle.
 */
class SpeechModel private constructor(handle: Long) : Closeable {
    private val handle = AtomicLong(handle)

    override fun close() {
        val h = handle.getAndSet(0L)
        if (h != 0L) {
            FFI.lib.aimux_drop_handle(h)
        }
    }

    private fun requireHandle(): Long = handle.get().also {
        check(it != 0L) { "SpeechModel is closed" }
    }

    protected fun finalize() {
        close()
    }

    companion object {
        /** Create an OpenAI speech (TTS) model. */
        fun openai(apiKey: String, modelId: String): SpeechModel {
            val h = extractHandle(
                FFI.lib.aimux_openai_speech_new(apiKey, modelId), "Failed to create OpenAI speech model")
            return SpeechModel(h)
        }

        /** Create an OpenAI speech model with a custom base URL. */
        fun openai(apiKey: String, modelId: String, baseUrl: String): SpeechModel {
            val h = extractHandle(
                FFI.lib.aimux_openai_speech_new_with_base(apiKey, modelId, baseUrl),
                "Failed to create OpenAI speech model")
            return SpeechModel(h)
        }
    }

    /**
     * Generate speech audio from the given options.
     *
     * @param optsJson JSON-serialized `SpeechCallOptions`.
     * @return JSON-serialized `SpeechResult` (or throws on `{"error":"..."}`).
     */
    fun generate(optsJson: String): String {
        val ptr = FFI.lib.aimux_speech_generate(requireHandle(), optsJson)
        return extractFFIString(ptr, "speech_generate")
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// TranscriptionModel (STT)
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Converts audio to text. Wraps a Rust `Arc<dyn TranscriptionModel>`.
 *
 * Implements [Closeable] — call [close] (or `use {}`) to release the handle.
 */
class TranscriptionModel private constructor(handle: Long) : Closeable {
    private val handle = AtomicLong(handle)

    override fun close() {
        val h = handle.getAndSet(0L)
        if (h != 0L) {
            FFI.lib.aimux_drop_handle(h)
        }
    }

    private fun requireHandle(): Long = handle.get().also {
        check(it != 0L) { "TranscriptionModel is closed" }
    }

    protected fun finalize() {
        close()
    }

    companion object {
        /** Create an OpenAI transcription (STT) model. */
        fun openai(apiKey: String, modelId: String): TranscriptionModel {
            val h = extractHandle(
                FFI.lib.aimux_openai_transcription_new(apiKey, modelId),
                "Failed to create OpenAI transcription model")
            return TranscriptionModel(h)
        }

        /** Create an OpenAI transcription model with a custom base URL. */
        fun openai(apiKey: String, modelId: String, baseUrl: String): TranscriptionModel {
            val h = extractHandle(
                FFI.lib.aimux_openai_transcription_new_with_base(apiKey, modelId, baseUrl),
                "Failed to create OpenAI transcription model")
            return TranscriptionModel(h)
        }
    }

    /**
     * Transcribe audio (base64-encoded) to text.
     *
     * @param audioBase64 Base64-encoded audio bytes.
     * @param mediaType   Media type of the audio (e.g. `audio/wav`).
     * @param optsJson    Optional JSON-serialized `TranscriptionCallOptions`.
     * @return JSON-serialized `TranscriptionResult` (or throws on `{"error":"..."}`).
     */
    fun generate(audioBase64: String, mediaType: String, optsJson: String? = null): String {
        val ptr = FFI.lib.aimux_transcription_generate(requireHandle(), audioBase64, mediaType, optsJson)
        return extractFFIString(ptr, "transcription_generate")
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// ImageModel
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Generates images from prompts. Wraps a Rust `Arc<dyn ImageModel>`.
 *
 * Implements [Closeable] — call [close] (or `use {}`) to release the handle.
 */
class ImageModel private constructor(handle: Long) : Closeable {
    private val handle = AtomicLong(handle)

    override fun close() {
        val h = handle.getAndSet(0L)
        if (h != 0L) {
            FFI.lib.aimux_drop_handle(h)
        }
    }

    private fun requireHandle(): Long = handle.get().also {
        check(it != 0L) { "ImageModel is closed" }
    }

    protected fun finalize() {
        close()
    }

    companion object {
        /** Create an OpenAI image model (e.g. `dall-e-3`). */
        fun openai(apiKey: String, modelId: String): ImageModel {
            val h = extractHandle(
                FFI.lib.aimux_openai_image_new(apiKey, modelId), "Failed to create OpenAI image model")
            return ImageModel(h)
        }

        /** Create an OpenAI image model with a custom base URL. */
        fun openai(apiKey: String, modelId: String, baseUrl: String): ImageModel {
            val h = extractHandle(
                FFI.lib.aimux_openai_image_new_with_base(apiKey, modelId, baseUrl),
                "Failed to create OpenAI image model")
            return ImageModel(h)
        }

        /** Create a Google image model (e.g. `gemini-2.5-flash-image`). */
        fun google(apiKey: String, modelId: String): ImageModel {
            val h = extractHandle(
                FFI.lib.aimux_google_image_new(apiKey, modelId), "Failed to create Google image model")
            return ImageModel(h)
        }

        /** Create a Google image model with a custom base URL. */
        fun google(apiKey: String, modelId: String, baseUrl: String): ImageModel {
            val h = extractHandle(
                FFI.lib.aimux_google_image_new_with_base(apiKey, modelId, baseUrl),
                "Failed to create Google image model")
            return ImageModel(h)
        }
    }

    /**
     * Generate images from the given options.
     *
     * @param optsJson JSON-serialized `ImageCallOptions`.
     * @return JSON-serialized `ImageResult` (or throws on `{"error":"..."}`).
     */
    fun generate(optsJson: String): String {
        val ptr = FFI.lib.aimux_image_generate(requireHandle(), optsJson)
        return extractFFIString(ptr, "image_generate")
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// VideoModel
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Generates videos from prompts. Wraps a Rust `Arc<dyn VideoModel>`.
 *
 * Implements [Closeable] — call [close] (or `use {}`) to release the handle.
 */
class VideoModel private constructor(handle: Long) : Closeable {
    private val handle = AtomicLong(handle)

    override fun close() {
        val h = handle.getAndSet(0L)
        if (h != 0L) {
            FFI.lib.aimux_drop_handle(h)
        }
    }

    private fun requireHandle(): Long = handle.get().also {
        check(it != 0L) { "VideoModel is closed" }
    }

    protected fun finalize() {
        close()
    }

    companion object {
        /** Create a Google video model (e.g. `veo-3.0`). */
        fun google(apiKey: String, modelId: String): VideoModel {
            val h = extractHandle(
                FFI.lib.aimux_google_video_new(apiKey, modelId), "Failed to create Google video model")
            return VideoModel(h)
        }

        /** Create a Google video model with a custom base URL. */
        fun google(apiKey: String, modelId: String, baseUrl: String): VideoModel {
            val h = extractHandle(
                FFI.lib.aimux_google_video_new_with_base(apiKey, modelId, baseUrl),
                "Failed to create Google video model")
            return VideoModel(h)
        }
    }

    /**
     * Generate videos from the given options.
     *
     * @param optsJson JSON-serialized `VideoCallOptions`.
     * @return JSON-serialized `VideoResult` (or throws on `{"error":"..."}`).
     */
    fun generate(optsJson: String): String {
        val ptr = FFI.lib.aimux_video_generate(requireHandle(), optsJson)
        return extractFFIString(ptr, "video_generate")
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// RerankingModel
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Reranks documents by relevance to a query. Wraps a Rust `Arc<dyn RerankingModel>`.
 *
 * Implements [Closeable] — call [close] (or `use {}`) to release the handle.
 */
class RerankingModel private constructor(handle: Long) : Closeable {
    private val handle = AtomicLong(handle)

    override fun close() {
        val h = handle.getAndSet(0L)
        if (h != 0L) {
            FFI.lib.aimux_drop_handle(h)
        }
    }

    private fun requireHandle(): Long = handle.get().also {
        check(it != 0L) { "RerankingModel is closed" }
    }

    protected fun finalize() {
        close()
    }

    companion object {
        /** Create a Cohere reranking model (e.g. `rerank-v3.0`). */
        fun cohere(apiKey: String, modelId: String): RerankingModel {
            val h = extractHandle(
                FFI.lib.aimux_cohere_reranking_new(apiKey, modelId), "Failed to create Cohere reranking model")
            return RerankingModel(h)
        }

        /** Create a Cohere reranking model with a custom base URL. */
        fun cohere(apiKey: String, modelId: String, baseUrl: String): RerankingModel {
            val h = extractHandle(
                FFI.lib.aimux_cohere_reranking_new_with_base(apiKey, modelId, baseUrl),
                "Failed to create Cohere reranking model")
            return RerankingModel(h)
        }
    }

    /**
     * Rerank documents against a query.
     *
     * @param optsJson JSON-serialized `RerankingCallOptions`.
     * @return JSON-serialized `RerankingResult` (or throws on `{"error":"..."}`).
     */
    fun rerank(optsJson: String): String {
        val ptr = FFI.lib.aimux_rerank(requireHandle(), optsJson)
        return extractFFIString(ptr, "rerank")
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// SearchModel
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Performs web search. Wraps a Rust `Arc<dyn SearchModel>`.
 *
 * Implements [Closeable] — call [close] (or `use {}`) to release the handle.
 */
class SearchModel private constructor(handle: Long) : Closeable {
    private val handle = AtomicLong(handle)

    override fun close() {
        val h = handle.getAndSet(0L)
        if (h != 0L) {
            FFI.lib.aimux_drop_handle(h)
        }
    }

    private fun requireHandle(): Long = handle.get().also {
        check(it != 0L) { "SearchModel is closed" }
    }

    protected fun finalize() {
        close()
    }

    companion object {
        /**
         * Create a Tavily search model. Tavily uses a fixed endpoint, so no
         * model ID is needed (the C ABI still takes one, so an empty string is
         * passed and ignored).
         */
        fun tavily(apiKey: String): SearchModel {
            val h = extractHandle(
                FFI.lib.aimux_tavily_search_new(apiKey, ""), "Failed to create Tavily search model")
            return SearchModel(h)
        }

        /** Create a Tavily search model with a custom base URL (e.g. for mocks). */
        fun tavily(apiKey: String, baseUrl: String): SearchModel {
            val h = extractHandle(
                FFI.lib.aimux_tavily_search_new_with_base(apiKey, "", baseUrl),
                "Failed to create Tavily search model")
            return SearchModel(h)
        }
    }

    /**
     * Perform a web search.
     *
     * @param optsJson JSON-serialized `SearchCallOptions`.
     * @return JSON-serialized `SearchResult` (or throws on `{"error":"..."}`).
     */
    fun search(optsJson: String): String {
        val ptr = FFI.lib.aimux_search(requireHandle(), optsJson)
        return extractFFIString(ptr, "search")
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Files
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Manages file uploads to providers. Wraps a Rust `Arc<dyn FilesModel>`.
 *
 * Implements [Closeable] — call [close] (or `use {}`) to release the handle.
 */
class Files private constructor(handle: Long) : Closeable {
    private val handle = AtomicLong(handle)

    override fun close() {
        val h = handle.getAndSet(0L)
        if (h != 0L) {
            FFI.lib.aimux_drop_handle(h)
        }
    }

    private fun requireHandle(): Long = handle.get().also {
        check(it != 0L) { "Files is closed" }
    }

    protected fun finalize() {
        close()
    }

    companion object {
        /** Create an OpenAI files manager. */
        fun openai(apiKey: String): Files {
            val h = extractHandle(FFI.lib.aimux_openai_files_new(apiKey), "Failed to create OpenAI files manager")
            return Files(h)
        }

        /** Create an OpenAI files manager with a custom base URL. */
        fun openai(apiKey: String, baseUrl: String): Files {
            val h = extractHandle(
                FFI.lib.aimux_openai_files_new_with_base(apiKey, baseUrl), "Failed to create OpenAI files manager")
            return Files(h)
        }
    }

    /**
     * Upload a file (base64-encoded) to the provider.
     *
     * @param dataBase64 Base64-encoded file bytes.
     * @param mediaType  Media type of the file (e.g. `application/pdf`).
     * @param optsJson   Optional JSON-serialized `UploadFileCallOptions`.
     * @return JSON-serialized `UploadFileResult` (or throws on `{"error":"..."}`).
     */
    fun uploadFile(dataBase64: String, mediaType: String, optsJson: String? = null): String {
        val ptr = FFI.lib.aimux_file_upload(requireHandle(), dataBase64, mediaType, optsJson)
        return extractFFIString(ptr, "file_upload")
    }
}
