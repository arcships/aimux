/**
 * aimux — multimodal model wrappers for Kotlin/JVM (Rust core via the C ABI).
 *
 * Each modality (embedding / speech / image / transcription / files /
 * reranking / video / search) is a [Closeable] class wrapping a native handle,
 * exactly like [Model]. The handle is acquired via a provider-specific factory
 * (companion object) and released via [close]. All C ABI data uses
 * JSON strings (base64 for binary), matching the C ABI wire format.
 *
 * Every fallible C call returns an `aimux_error_t *` (null = success) that
 * [expectAimuxError] decodes codes 1..13 as [AimuxException] and 200..206 as
 * [IllegalStateException] `"aimux ffi: …"` (malformed raw JSON is
 * caught before the C call by [requireJson] as [IllegalArgumentException]).
 * No JSON envelope on the primary path.
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

import com.sun.jna.ptr.IntByReference
import com.sun.jna.ptr.PointerByReference
import java.io.Closeable
import java.util.concurrent.atomic.AtomicLong

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
        if (it == 0L) throw IllegalStateException("EmbeddingModel is closed")
    }

    protected fun finalize() {
        close()
    }

    companion object {
        /** Create an OpenAI embedding model (e.g. `text-embedding-3-small`). */
        fun openai(apiKey: String, modelId: String): EmbeddingModel =
            EmbeddingModel(handleResult { out ->
                FFI.lib.aimux_openai_embedding_new(apiKey, modelId, out)
            })

        /** Create an OpenAI embedding model with a custom base URL. */
        fun openai(apiKey: String, modelId: String, baseUrl: String): EmbeddingModel =
            EmbeddingModel(handleResult { out ->
                FFI.lib.aimux_openai_embedding_new_with_base(apiKey, modelId, baseUrl, out)
            })

        /** Create a Cohere embedding model (e.g. `embed-english-v3.0`). */
        fun cohere(apiKey: String, modelId: String): EmbeddingModel =
            EmbeddingModel(handleResult { out ->
                FFI.lib.aimux_cohere_embedding_new(apiKey, modelId, out)
            })

        /** Create a Cohere embedding model with a custom base URL. */
        fun cohere(apiKey: String, modelId: String, baseUrl: String): EmbeddingModel =
            EmbeddingModel(handleResult { out ->
                FFI.lib.aimux_cohere_embedding_new_with_base(apiKey, modelId, baseUrl, out)
            })

        /** Create a Google embedding model (e.g. `gemini-embedding-001`). */
        fun google(apiKey: String, modelId: String): EmbeddingModel =
            EmbeddingModel(handleResult { out ->
                FFI.lib.aimux_google_embedding_new(apiKey, modelId, out)
            })

        /** Create a Google embedding model with a custom base URL. */
        fun google(apiKey: String, modelId: String, baseUrl: String): EmbeddingModel =
            EmbeddingModel(handleResult { out ->
                FFI.lib.aimux_google_embedding_new_with_base(apiKey, modelId, baseUrl, out)
            })
    }

    /**
     * Generate embeddings for the given text values.
     *
     * @param valuesJson JSON array of strings to embed (e.g. `["a","b"]`).
     * @param optsJson   Optional JSON-serialized `EmbeddingCallOptions`.
     * @return JSON-serialized `EmbeddingResult`.
     * @throws AimuxException on AiMuxError failure.
     * @throws IllegalArgumentException when a raw JSON argument is malformed; IllegalStateException after [close].
     */
    fun embed(valuesJson: String, optsJson: String? = null): String {
        requireJsonRequired("valuesJson", valuesJson)
        requireJson("optsJson", optsJson)
        return stringResult { out -> FFI.lib.aimux_embed(requireHandle(), valuesJson, optsJson, out) }
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
        if (it == 0L) throw IllegalStateException("SpeechModel is closed")
    }

    protected fun finalize() {
        close()
    }

    companion object {
        /** Create an OpenAI speech (TTS) model. */
        fun openai(apiKey: String, modelId: String): SpeechModel =
            SpeechModel(handleResult { out ->
                FFI.lib.aimux_openai_speech_new(apiKey, modelId, out)
            })

        /** Create an OpenAI speech model with a custom base URL. */
        fun openai(apiKey: String, modelId: String, baseUrl: String): SpeechModel =
            SpeechModel(handleResult { out ->
                FFI.lib.aimux_openai_speech_new_with_base(apiKey, modelId, baseUrl, out)
            })
    }

    /**
     * Generate speech audio from the given options.
     *
     * @param optsJson JSON-serialized `SpeechCallOptions`.
     * @return JSON-serialized `SpeechResult`.
     * @throws AimuxException on AiMuxError failure.
     * @throws IllegalArgumentException when a raw JSON argument is malformed; IllegalStateException after [close].
     */
    fun generate(optsJson: String): String {
        requireJsonRequired("optsJson", optsJson)
        return stringResult { out -> FFI.lib.aimux_speech_generate(requireHandle(), optsJson, out) }
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
        if (it == 0L) throw IllegalStateException("TranscriptionModel is closed")
    }

    protected fun finalize() {
        close()
    }

    companion object {
        /** Create an OpenAI transcription (STT) model. */
        fun openai(apiKey: String, modelId: String): TranscriptionModel =
            TranscriptionModel(handleResult { out ->
                FFI.lib.aimux_openai_transcription_new(apiKey, modelId, out)
            })

        /** Create an OpenAI transcription model with a custom base URL. */
        fun openai(apiKey: String, modelId: String, baseUrl: String): TranscriptionModel =
            TranscriptionModel(handleResult { out ->
                FFI.lib.aimux_openai_transcription_new_with_base(apiKey, modelId, baseUrl, out)
            })
    }

    /**
     * Transcribe audio (base64-encoded) to text.
     *
     * @param audioBase64 Base64-encoded audio bytes.
     * @param mediaType   Media type of the audio (e.g. `audio/wav`).
     * @param optsJson    Optional JSON-serialized `TranscriptionCallOptions`.
     * @return JSON-serialized `TranscriptionResult`.
     * @throws AimuxException on AiMuxError failure.
     * @throws IllegalArgumentException when a raw JSON argument is malformed; IllegalStateException after [close].
     */
    fun generate(audioBase64: String, mediaType: String, optsJson: String? = null): String {
        requireJson("optsJson", optsJson)
        return stringResult { out -> FFI.lib.aimux_transcription_generate(requireHandle(), audioBase64, mediaType, optsJson, out) }
    }

    /**
     * Start a streaming transcription session (RFC-0028) on this model.
     * Requires a model that supports streaming (realtime models).
     *
     * @param optsJson    optional session options JSON (`input_audio_format` /
     *                    `provider_options` / `headers` / `include_raw_chunks`).
     * @param abortHandle abort handle (from `Model.abortSignalNew()`), or 0.
     * @return a new live session.
     */
    fun startStream(optsJson: String? = null, abortHandle: Long = 0L): TranscriptionSession {
        requireJson("optsJson", optsJson)
        val h = requireHandle()
        return TranscriptionSession(
            handleResult { out ->
                FFI.lib.aimux_transcription_session_new(h, abortHandle, optsJson, out)
            }
        )
    }
}

/**
 * A live streaming-transcription session (RFC-0028): push audio chunks with
 * [pushAudio], mark end-of-audio with [inputDone], then pull transcription
 * parts (JSON `TranscriptionStreamPart`) with [nextPart]. [close] releases
 * the session (aborts the driver; idempotent).
 */
// Internal constructor: sessions are created via TranscriptionModel.startStream.
class TranscriptionSession internal constructor(handle: Long) : Closeable {
    private val handle = AtomicLong(handle)

    override fun close() {
        val h = handle.getAndSet(0L)
        if (h != 0L) {
            FFI.lib.aimux_transcription_session_drop(h)
        }
    }

    private fun requireHandle(): Long = handle.get().also {
        if (it == 0L) throw IllegalStateException("TranscriptionSession is closed")
    }

    protected fun finalize() {
        close()
    }

    /**
     * Push one binary audio chunk. **Blocks** while the internal channel is
     * full (backpressure propagation).
     */
    fun pushAudio(audio: ByteArray) {
        val h = requireHandle()
        FFI.lib.aimux_transcription_push_audio(h, audio, audio.size.toLong())
            ?.let { throw expectAimuxError(it, "pushAudio") }
    }

    /** Signal end-of-audio (idempotent). */
    fun inputDone() {
        val h = requireHandle()
        FFI.lib.aimux_transcription_input_done(h)?.let { throw expectFfiError(it, "inputDone") }
    }

    /**
     * Pull the next transcription part (JSON string).
     *
     * @param timeoutMs wait bound: >0 wait at most; 0 immediate poll; <0 wait
     *                  indefinitely.
     * @return the part JSON.
     * @throws AimuxTranscriptionEndedException   the stream finished normally.
     * @throws AimuxTranscriptionTimeoutException no part arrived in time —
     *                  **retryable**: the session stays live, call again
     *                  (RFC-0028: state TIMEOUT, not an error). This is NOT an
     *                  [AimuxException] / [TimeoutError]; catch the sentinel
     *                  explicitly.
     * @throws AimuxException                     the stream failed.
     * @throws IllegalStateException              the session is closed.
     */
    fun nextPart(timeoutMs: Long): String {
        val h = requireHandle()
        val outPart = PointerByReference()
        val outState = IntByReference()
        FFI.lib.aimux_transcription_next_part(h, timeoutMs, outPart, outState)
            ?.let { throw expectAimuxError(it, "nextPart") }
        return when (val state = outState.value) {
            TRANSCRIPTION_NEXT_PART_PART -> takeString(outPart.value)
                ?: throw IllegalStateException("nextPart: aimux ffi: PART state with NULL part")
            TRANSCRIPTION_NEXT_PART_ENDED -> throw AimuxTranscriptionEndedException()
            // Not an error: nothing to decode, the session stays live.
            TRANSCRIPTION_NEXT_PART_TIMEOUT -> throw AimuxTranscriptionTimeoutException()
            else -> throw IllegalStateException("nextPart: aimux ffi: unknown next_part state $state")
        }
    }

    /** The transcription stream ended normally. */
    class AimuxTranscriptionEndedException :
        RuntimeException("transcription stream ended")

    /**
     * No transcription part arrived within the timeout — **retryable**: the
     * session stays live, call [nextPart] again (RFC-0028). Deliberately not
     * an [AimuxException]: a timeout is not a stream failure, so generic
     * AimuxError handling must not swallow it — same shape as the Go / Java
     * / Swift / Flutter sentinels.
     */
    class AimuxTranscriptionTimeoutException :
        RuntimeException("transcription part timeout")
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
        if (it == 0L) throw IllegalStateException("ImageModel is closed")
    }

    protected fun finalize() {
        close()
    }

    companion object {
        /** Create an OpenAI image model (e.g. `dall-e-3`). */
        fun openai(apiKey: String, modelId: String): ImageModel =
            ImageModel(handleResult { out ->
                FFI.lib.aimux_openai_image_new(apiKey, modelId, out)
            })

        /** Create an OpenAI image model with a custom base URL. */
        fun openai(apiKey: String, modelId: String, baseUrl: String): ImageModel =
            ImageModel(handleResult { out ->
                FFI.lib.aimux_openai_image_new_with_base(apiKey, modelId, baseUrl, out)
            })

        /** Create a Google image model (e.g. `gemini-2.5-flash-image`). */
        fun google(apiKey: String, modelId: String): ImageModel =
            ImageModel(handleResult { out ->
                FFI.lib.aimux_google_image_new(apiKey, modelId, out)
            })

        /** Create a Google image model with a custom base URL. */
        fun google(apiKey: String, modelId: String, baseUrl: String): ImageModel =
            ImageModel(handleResult { out ->
                FFI.lib.aimux_google_image_new_with_base(apiKey, modelId, baseUrl, out)
            })
    }

    /**
     * Generate images from the given options.
     *
     * @param optsJson JSON-serialized `ImageCallOptions`.
     * @return JSON-serialized `ImageResult`.
     * @throws AimuxException on AiMuxError failure.
     * @throws IllegalArgumentException when a raw JSON argument is malformed; IllegalStateException after [close].
     */
    fun generate(optsJson: String): String {
        requireJsonRequired("optsJson", optsJson)
        return stringResult { out -> FFI.lib.aimux_image_generate(requireHandle(), optsJson, out) }
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
        if (it == 0L) throw IllegalStateException("VideoModel is closed")
    }

    protected fun finalize() {
        close()
    }

    companion object {
        /** Create a Google video model (e.g. `veo-3.0`). */
        fun google(apiKey: String, modelId: String): VideoModel =
            VideoModel(handleResult { out ->
                FFI.lib.aimux_google_video_new(apiKey, modelId, out)
            })

        /** Create a Google video model with a custom base URL. */
        fun google(apiKey: String, modelId: String, baseUrl: String): VideoModel =
            VideoModel(handleResult { out ->
                FFI.lib.aimux_google_video_new_with_base(apiKey, modelId, baseUrl, out)
            })
    }

    /**
     * Generate videos from the given options.
     *
     * @param optsJson JSON-serialized `VideoCallOptions`.
     * @return JSON-serialized `VideoResult`.
     * @throws AimuxException on AiMuxError failure.
     * @throws IllegalArgumentException when a raw JSON argument is malformed; IllegalStateException after [close].
     */
    fun generate(optsJson: String): String {
        requireJsonRequired("optsJson", optsJson)
        return stringResult { out -> FFI.lib.aimux_video_generate(requireHandle(), optsJson, out) }
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
        if (it == 0L) throw IllegalStateException("RerankingModel is closed")
    }

    protected fun finalize() {
        close()
    }

    companion object {
        /** Create a Cohere reranking model (e.g. `rerank-v3.0`). */
        fun cohere(apiKey: String, modelId: String): RerankingModel =
            RerankingModel(handleResult { out ->
                FFI.lib.aimux_cohere_reranking_new(apiKey, modelId, out)
            })

        /** Create a Cohere reranking model with a custom base URL. */
        fun cohere(apiKey: String, modelId: String, baseUrl: String): RerankingModel =
            RerankingModel(handleResult { out ->
                FFI.lib.aimux_cohere_reranking_new_with_base(apiKey, modelId, baseUrl, out)
            })
    }

    /**
     * Rerank documents against a query.
     *
     * @param optsJson JSON-serialized `RerankingCallOptions`.
     * @return JSON-serialized `RerankingResult`.
     * @throws AimuxException on AiMuxError failure.
     * @throws IllegalArgumentException when a raw JSON argument is malformed; IllegalStateException after [close].
     */
    fun rerank(optsJson: String): String {
        requireJsonRequired("optsJson", optsJson)
        return stringResult { out -> FFI.lib.aimux_rerank(requireHandle(), optsJson, out) }
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
        if (it == 0L) throw IllegalStateException("SearchModel is closed")
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
        fun tavily(apiKey: String): SearchModel =
            SearchModel(handleResult { out ->
                FFI.lib.aimux_tavily_search_new(apiKey, "", out)
            })

        /** Create a Tavily search model with a custom base URL (e.g. for mocks). */
        fun tavily(apiKey: String, baseUrl: String): SearchModel =
            SearchModel(handleResult { out ->
                FFI.lib.aimux_tavily_search_new_with_base(apiKey, "", baseUrl, out)
            })
    }

    /**
     * Perform a web search.
     *
     * @param optsJson JSON-serialized `SearchCallOptions`.
     * @return JSON-serialized `SearchResult`.
     * @throws AimuxException on AiMuxError failure.
     * @throws IllegalArgumentException when a raw JSON argument is malformed; IllegalStateException after [close].
     */
    fun search(optsJson: String): String {
        requireJsonRequired("optsJson", optsJson)
        return stringResult { out -> FFI.lib.aimux_search(requireHandle(), optsJson, out) }
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
        if (it == 0L) throw IllegalStateException("Files is closed")
    }

    protected fun finalize() {
        close()
    }

    companion object {
        /** Create an OpenAI files manager. */
        fun openai(apiKey: String): Files =
            Files(handleResult { out ->
                FFI.lib.aimux_openai_files_new(apiKey, out)
            })

        /** Create an OpenAI files manager with a custom base URL. */
        fun openai(apiKey: String, baseUrl: String): Files =
            Files(handleResult { out ->
                FFI.lib.aimux_openai_files_new_with_base(apiKey, baseUrl, out)
            })
    }

    /**
     * Upload a file (base64-encoded) to the provider.
     *
     * @param dataBase64 Base64-encoded file bytes.
     * @param mediaType  Media type of the file (e.g. `application/pdf`).
     * @param optsJson   Optional JSON-serialized `UploadFileCallOptions`.
     * @return JSON-serialized `UploadFileResult`.
     * @throws AimuxException on AiMuxError failure.
     * @throws IllegalArgumentException when a raw JSON argument is malformed; IllegalStateException after [close].
     */
    fun uploadFile(dataBase64: String, mediaType: String, optsJson: String? = null): String {
        requireJson("optsJson", optsJson)
        return stringResult { out -> FFI.lib.aimux_file_upload(requireHandle(), dataBase64, mediaType, optsJson, out) }
    }
}
