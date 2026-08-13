/**
 * aimux — Unified LLM service layer for Kotlin/JVM (Rust core, 172+ providers).
 *
 * Uses JNA to call the aimux-ffi C ABI. This is the C ABI path (§3.2).
 * The native library (libaimux_ffi.so / .dylib / .dll) must be on the
 * library path or bundled in the JAR's native/ directory.
 *
 * Errors: fallible calls take trailing [AimuxCError]; failure returns 0 / NULL.
 * Map with [AimuxException.fromC]. No JSON error envelope on the primary path.
 */

package ai.arcships.aimux

import com.sun.jna.Callback
import com.sun.jna.Library
import com.sun.jna.Native
import com.sun.jna.Pointer
import java.io.Closeable
import java.util.concurrent.locks.ReentrantReadWriteLock

// ─────────────────────────────────────────────────────────────────────────────
// JNA interface — direct mapping to the C ABI (uint64_t handles + AimuxError *).
// ─────────────────────────────────────────────────────────────────────────────

internal interface AimuxFFI : Library {
    fun aimux_openai_new(apiKey: String, modelId: String, err: AimuxCError?): Long
    fun aimux_anthropic_new(apiKey: String, modelId: String, err: AimuxCError?): Long
    fun aimux_openai_new_with_base(apiKey: String, modelId: String, baseUrl: String, err: AimuxCError?): Long
    fun aimux_anthropic_new_with_base(apiKey: String, modelId: String, baseUrl: String, err: AimuxCError?): Long
    fun aimux_cohere_new(apiKey: String, modelId: String, err: AimuxCError?): Long
    fun aimux_cohere_new_with_base(apiKey: String, modelId: String, baseUrl: String, err: AimuxCError?): Long
    fun aimux_mistral_new(apiKey: String, modelId: String, err: AimuxCError?): Long
    fun aimux_mistral_new_with_base(apiKey: String, modelId: String, baseUrl: String, err: AimuxCError?): Long
    fun aimux_xai_new(apiKey: String, modelId: String, err: AimuxCError?): Long
    fun aimux_xai_new_with_base(apiKey: String, modelId: String, baseUrl: String, err: AimuxCError?): Long
    fun aimux_bedrock_new(
        accessKeyId: String, secretAccessKey: String, region: String, modelId: String, err: AimuxCError?
    ): Long
    fun aimux_bedrock_new_with_base(
        accessKeyId: String, secretAccessKey: String, region: String, modelId: String, baseUrl: String, err: AimuxCError?
    ): Long
    fun aimux_vertex_new(
        accessToken: String, project: String, location: String, modelId: String, err: AimuxCError?
    ): Long
    fun aimux_vertex_new_with_base(
        accessToken: String, project: String, location: String, modelId: String, baseUrl: String, err: AimuxCError?
    ): Long
    fun aimux_anthropic_aws_new(apiKey: String, region: String, modelId: String, err: AimuxCError?): Long
    fun aimux_anthropic_aws_new_with_base(
        apiKey: String, region: String, modelId: String, baseUrl: String, err: AimuxCError?
    ): Long
    fun aimux_azure_new(
        apiKey: String, resourceName: String, deployment: String, apiVersion: String?, err: AimuxCError?
    ): Long
    fun aimux_azure_new_with_base(
        apiKey: String, baseUrl: String, deployment: String, apiVersion: String?, err: AimuxCError?
    ): Long

    // ── Registry provider (RFC-0017 phase 4) ───────────────────────────────
    fun aimux_provider_new(name: String, apiKey: String?, modelId: String, configJson: String?, err: AimuxCError?): Long
    fun aimux_provider_from_env(name: String, modelId: String, err: AimuxCError?): Long

    // ── Provider handles (RFC-0027) ─────────────────────────────────────────
    fun aimux_provider_handle_new(name: String, apiKey: String?, configJson: String?, err: AimuxCError?): Long
    fun aimux_provider_list_models(handle: Long, err: AimuxCError?): Pointer?
    fun aimux_provider_model(handle: Long, modelId: String, err: AimuxCError?): Long
    fun aimux_get_model_specs(sourceUrl: String?, err: AimuxCError?): Pointer?

    fun aimux_generate_text(handle: Long, promptJson: String, optsJson: String?, err: AimuxCError?): Pointer?
    fun aimux_generate_object(handle: Long, promptJson: String, optsJson: String?, err: AimuxCError?): Pointer?
    fun aimux_consume_stream_text(handle: Long, promptJson: String, optsJson: String?, err: AimuxCError?): Pointer?
    fun aimux_stream_text(
        handle: Long,
        promptJson: String,
        optsJson: String?,
        onPart: Callback?,
        onDone: Callback?,
        streamCtx: Pointer?,
        err: AimuxCError?,
    ): Int

    // ── OpenAI-compatible output (RFC-0026) ─────────────────────────────────
    fun aimux_generate_text_as_openai(handle: Long, promptJson: String, optsJson: String?, err: AimuxCError?): Pointer?
    fun aimux_stream_text_as_openai(
        handle: Long,
        promptJson: String,
        optsJson: String?,
        onPart: Callback?,
        onDone: Callback?,
        streamCtx: Pointer?,
        err: AimuxCError?,
    ): Int

    fun aimux_drop_handle(handle: Long)
    fun aimux_free_string(ptr: Pointer?)

    // ── Embedding ──────────────────────────────────────────────────────────
    fun aimux_openai_embedding_new(apiKey: String, modelId: String, err: AimuxCError?): Long
    fun aimux_openai_embedding_new_with_base(apiKey: String, modelId: String, baseUrl: String, err: AimuxCError?): Long
    fun aimux_cohere_embedding_new(apiKey: String, modelId: String, err: AimuxCError?): Long
    fun aimux_cohere_embedding_new_with_base(apiKey: String, modelId: String, baseUrl: String, err: AimuxCError?): Long
    fun aimux_google_embedding_new(apiKey: String, modelId: String, err: AimuxCError?): Long
    fun aimux_google_embedding_new_with_base(apiKey: String, modelId: String, baseUrl: String, err: AimuxCError?): Long
    fun aimux_embed(handle: Long, valuesJson: String, optsJson: String?, err: AimuxCError?): Pointer?

    // ── Speech (TTS) ───────────────────────────────────────────────────────
    fun aimux_openai_speech_new(apiKey: String, modelId: String, err: AimuxCError?): Long
    fun aimux_openai_speech_new_with_base(apiKey: String, modelId: String, baseUrl: String, err: AimuxCError?): Long
    fun aimux_speech_generate(handle: Long, optsJson: String, err: AimuxCError?): Pointer?

    // ── Image ──────────────────────────────────────────────────────────────
    fun aimux_openai_image_new(apiKey: String, modelId: String, err: AimuxCError?): Long
    fun aimux_openai_image_new_with_base(apiKey: String, modelId: String, baseUrl: String, err: AimuxCError?): Long
    fun aimux_google_image_new(apiKey: String, modelId: String, err: AimuxCError?): Long
    fun aimux_google_image_new_with_base(apiKey: String, modelId: String, baseUrl: String, err: AimuxCError?): Long
    fun aimux_image_generate(handle: Long, optsJson: String, err: AimuxCError?): Pointer?

    // ── Transcription (STT) ────────────────────────────────────────────────
    fun aimux_openai_transcription_new(apiKey: String, modelId: String, err: AimuxCError?): Long
    fun aimux_openai_transcription_new_with_base(
        apiKey: String, modelId: String, baseUrl: String, err: AimuxCError?
    ): Long
    fun aimux_transcription_generate(
        handle: Long, audioBase64: String, mediaType: String, optsJson: String?, err: AimuxCError?
    ): Pointer?

    // ── Files ──────────────────────────────────────────────────────────────
    fun aimux_openai_files_new(apiKey: String, err: AimuxCError?): Long
    fun aimux_openai_files_new_with_base(apiKey: String, baseUrl: String, err: AimuxCError?): Long
    fun aimux_file_upload(
        handle: Long, dataBase64: String, mediaType: String, optsJson: String?, err: AimuxCError?
    ): Pointer?

    // ── Reranking ──────────────────────────────────────────────────────────
    fun aimux_cohere_reranking_new(apiKey: String, modelId: String, err: AimuxCError?): Long
    fun aimux_cohere_reranking_new_with_base(apiKey: String, modelId: String, baseUrl: String, err: AimuxCError?): Long
    fun aimux_rerank(handle: Long, optsJson: String, err: AimuxCError?): Pointer?

    // ── Video ──────────────────────────────────────────────────────────────
    fun aimux_google_video_new(apiKey: String, modelId: String, err: AimuxCError?): Long
    fun aimux_google_video_new_with_base(apiKey: String, modelId: String, baseUrl: String, err: AimuxCError?): Long
    fun aimux_video_generate(handle: Long, optsJson: String, err: AimuxCError?): Pointer?

    // ── Search ─────────────────────────────────────────────────────────────
    fun aimux_tavily_search_new(apiKey: String, modelId: String, err: AimuxCError?): Long
    fun aimux_tavily_search_new_with_base(apiKey: String, modelId: String, baseUrl: String, err: AimuxCError?): Long
    fun aimux_search(handle: Long, optsJson: String, err: AimuxCError?): Pointer?

    // Logging (RFC-0014).
    fun aimux_init_logging(level: String): Int

    // ── Recording + mock replay (RFC-0023) ──────────────────────────────────
    fun aimux_init_recording(dir: String): Int
    fun aimux_init_recording_ring(cap: Long): Int
    fun aimux_recording_stop(): Int
    fun aimux_recording_flush(): Int
    fun aimux_mock_replay_new(recordingsJsonl: String, err: AimuxCError?): Long

    // Register external providers from JSON config (RFC-0020). Returns 1 on success, 0 on failure.
    fun aimux_register_providers(configJson: String, err: AimuxCError?): Int

    // Set the global proxy configuration (M6, RFC-0016). Returns 1 on success, 0 on failure.
    fun aimux_init_proxy(configJson: String, err: AimuxCError?): Int
}

internal object FFI {
    val lib: AimuxFFI = Native.load("aimux_ffi", AimuxFFI::class.java)
}

/**
 * Map a failed FFI call's [err] to a typed [AimuxException] and throw it.
 * Frees the FFI-allocated [AimuxCError.message] and [AimuxCError.error_value]
 * exactly once each (fromC itself is a pure mapping and never frees).
 */
internal fun throwFromC(err: AimuxCError): Nothing {
    val ex = AimuxException.fromC(err)
    FFI.lib.aimux_free_string(err.message)
    FFI.lib.aimux_free_string(err.error_value)
    throw ex
}

/**
 * Turn a constructor `uint64_t` handle + filled [AimuxCError] into a non-zero handle.
 * Throws [AimuxException] (typed subclass) when [handle] is 0.
 */
internal fun extractHandle(handle: Long, err: AimuxCError): Long {
    if (handle == 0L) throwFromC(err)
    return handle
}

/** Allocate, run [block], map 0 → typed [AimuxException]. */
internal inline fun withCErrorHandle(block: (AimuxCError) -> Long): Long {
    val err = AimuxCError()
    return extractHandle(block(err), err)
}

/** Allocate, run [block]; NULL → throw typed [AimuxException]; free the result on success. */
internal inline fun withCErrorString(block: (AimuxCError) -> Pointer?): String {
    val err = AimuxCError()
    val ptr = block(err) ?: throwFromC(err)
    return try {
        ptr.getString(0, "UTF-8")
    } finally {
        FFI.lib.aimux_free_string(ptr)
    }
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
 * **Thread-safety / concurrency.** [Model] is safe for concurrent use. It
 * guards the native handle with a Go-style [ReentrantReadWriteLock] (fair,
 * FIFO): every FFI call ([generateText], [streamText], streaming variants)
 * holds the _read_ lock for its entire duration, and [close] takes the _write_
 * lock. As a result [close] blocks until all in-flight calls finish before
 * dropping the native handle — this closes the use-after-free race where a
 * caller could observe a non-zero handle and then race with [close]'s drop.
 * Because a streaming call holds the read lock until the stream completes,
 * [close] will not interrupt or drop a handle out from under an active stream.
 * Do not call [close] from within a stream callback (would self-deadlock).
 *
 * ```kotlin
 * Model.openai("sk-...", "gpt-4o-mini").use { model ->
 *     val result = model.generateText("\"Hello!\"")
 * }
 * ```
 */
class Model internal constructor(handle: Long) : Closeable {
    // Go-style read/write lock: every FFI call holds the read lock for its
    // entire duration; close() takes the write lock and thus blocks until all
    // in-flight calls finish before dropping the native handle. This closes the
    // read-then-drop use-after-free race the AtomicLong version had. Fair
    // (FIFO) so a pending close is not starved by barging readers — matches
    // Go's sync.RWMutex writer-priority semantics.
    private val lock = ReentrantReadWriteLock(true)
    private var handle: Long = handle
    private var closed: Boolean = false

    /**
     * Release the native handle. Idempotent and thread-safe: subsequent calls
     * are no-ops, and every other method throws [IllegalStateException].
     *
     * Acquires the write lock and therefore _blocks until all in-flight FFI
     * calls_ (which hold the read lock) finish before dropping the native
     * handle — prevents a use-after-free race between a concurrent caller and
     * [close]. A streaming call holds the read lock for the entire stream, so
     * [close] blocks until the stream completes. Do not call [close] from
     * within a stream callback (would self-deadlock).
     */
    override fun close() {
        lock.writeLock().lock()
        try {
            if (closed || handle == 0L) {
                return
            }
            val h = handle
            handle = 0L
            closed = true
            FFI.lib.aimux_drop_handle(h)
        } finally {
            lock.writeLock().unlock()
        }
    }

    // Caller MUST already hold the read lock (each public FFI method acquires
    // it and releases it in a finally after the FFI call returns). Holding the
    // read lock across the FFI call is what lets close()'s write lock wait for
    // the call to finish, closing the use-after-free race.
    private fun requireHandleLocked(): Long {
        check(!closed && handle != 0L) { "Model is closed" }
        return handle
    }

    protected fun finalize() {
        close()
    }

    // ── Generation ─────────────────────────────────────────────────────────

    /**
     * Generate text (non-streaming).
     *
     * @param promptJson JSON prompt string (bare value or {"prompt": ...}).
     * @param optsJson Optional JSON-serialized GenerateTextOptions.
     * @return JSON-serialized GenerateTextResult.
     * @throws AimuxException on engine / binding failure (typed subclass via C AimuxError).
     */
    fun generateText(promptJson: String, optsJson: String? = null): String {
        lock.readLock().lock()
        try {
            val h = requireHandleLocked()
            return withCErrorString { err ->
                FFI.lib.aimux_generate_text(h, promptJson, optsJson, err)
            }
        } finally {
            lock.readLock().unlock()
        }
    }

    /**
     * Generate a structured JSON object (M12, RFC-0016).
     *
     * Same signature as [generateText]; returns a JSON-serialized
     * `GenerateObjectResult`. Pass `response_format: { "Json": { ... } }` via
     * [optsJson] for schema control; the engine applies JSON repair before
     * parsing.
     *
     * @param promptJson JSON prompt string (bare value or {"prompt": ...}).
     * @param optsJson Optional JSON-serialized GenerateTextOptions.
     * @return JSON-serialized GenerateObjectResult.
     * @throws AimuxException on engine / binding failure.
     */
    fun generateObject(promptJson: String, optsJson: String? = null): String {
        lock.readLock().lock()
        try {
            val h = requireHandleLocked()
            return withCErrorString { err ->
                FFI.lib.aimux_generate_object(h, promptJson, optsJson, err)
            }
        } finally {
            lock.readLock().unlock()
        }
    }

    /**
     * Consume a stream to completion and return the aggregated result
     * (M11, RFC-0016). Synchronous (blocks until the stream finishes).
     *
     * Same signature as [generateText]; returns a JSON-serialized
     * `StreamTextResultAggregated`.
     *
     * @param promptJson JSON prompt string (bare value or {"prompt": ...}).
     * @param optsJson Optional JSON-serialized GenerateTextOptions.
     * @return JSON-serialized StreamTextResultAggregated.
     * @throws AimuxException on engine / binding failure.
     */
    fun consumeStreamText(promptJson: String, optsJson: String? = null): String {
        lock.readLock().lock()
        try {
            val h = requireHandleLocked()
            return withCErrorString { err ->
                FFI.lib.aimux_consume_stream_text(h, promptJson, optsJson, err)
            }
        } finally {
            lock.readLock().unlock()
        }
    }

    /**
     * Stream text from the model.
     *
     * Blocks the calling thread until the stream completes. Stream failures
     * throw [AimuxException] (on_done is not invoked on failure). Mid-stream
     * errors surface the same way after any parts already delivered to [onPart].
     *
     * @param promptJson JSON prompt string.
     * @param optsJson Optional JSON-serialized GenerateTextOptions.
     * @param onPart Called for each StreamPart (JSON string).
     * @param onDone Called when the stream completes normally.
     */
    fun streamText(
        promptJson: String,
        optsJson: String? = null,
        onPart: (String) -> Unit,
        onDone: () -> Unit,
    ) {
        // JNA callbacks — must be held in variables to prevent GC.
        // C ABI: on_part(const char* json, void* stream_ctx), on_done(void* stream_ctx).
        val partCb = object : Callback {
            @Suppress("unused")
            fun callback(jsonPtr: Pointer?, @Suppress("UNUSED_PARAMETER") streamCtx: Pointer?) {
                if (jsonPtr != null) {
                    onPart(jsonPtr.getString(0, "UTF-8"))
                }
            }
        }
        val doneCb = object : Callback {
            @Suppress("unused")
            fun callback(@Suppress("UNUSED_PARAMETER") streamCtx: Pointer?) {
                onDone()
            }
        }

        // Hold the read lock for the whole blocking stream so close() cannot
        // drop the handle mid-stream (it blocks on the write lock until the
        // stream completes).
        lock.readLock().lock()
        try {
            val h = requireHandleLocked()
            val err = AimuxCError()
            val rc = FFI.lib.aimux_stream_text(
                h, promptJson, optsJson, partCb, doneCb, null, err,
            )
            if (rc == 0) throwFromC(err)
        } finally {
            lock.readLock().unlock()
        }
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
        // LinkedBlockingQueue rejects null, so end-of-stream is a sentinel object.
        val eos = Any()
        val parts = java.util.concurrent.LinkedBlockingQueue<Any>()
        var streamError: AimuxException? = null

        try {
            streamText(
                promptJson = promptJson,
                optsJson = optsJson,
                onPart = { parts.put(it) },
                onDone = { parts.put(eos) },
            )
        } catch (e: AimuxException) {
            streamError = e
            parts.put(eos)
        }

        while (true) {
            val part = parts.take()
            if (part === eos) break
            yield(part as String)
        }

        streamError?.let { throw it }
    }

    // ── OpenAI-compatible output (RFC-0026) ─────────────────────────────────

    /**
     * Generate text (non-streaming) with OpenAI Chat Completions output.
     *
     * @return JSON-serialized ChatCompletion.
     * @throws AimuxException on engine / binding failure.
     */
    fun generateTextAsOpenAI(promptJson: String, optsJson: String? = null): String {
        lock.readLock().lock()
        try {
            val h = requireHandleLocked()
            return withCErrorString { err ->
                FFI.lib.aimux_generate_text_as_openai(h, promptJson, optsJson, err)
            }
        } finally {
            lock.readLock().unlock()
        }
    }

    /**
     * Stream text with OpenAI Chat Completions output.
     * Each [onPart] receives a serialized ChatCompletionChunk JSON string.
     *
     * @throws AimuxException on stream failure (on_done not invoked).
     */
    fun streamTextAsOpenAI(
        promptJson: String,
        optsJson: String? = null,
        onPart: (String) -> Unit,
        onDone: () -> Unit,
    ) {
        val partCb = object : Callback {
            @Suppress("unused")
            fun callback(jsonPtr: Pointer?, @Suppress("UNUSED_PARAMETER") streamCtx: Pointer?) {
                if (jsonPtr != null) {
                    onPart(jsonPtr.getString(0, "UTF-8"))
                }
            }
        }
        val doneCb = object : Callback {
            @Suppress("unused")
            fun callback(@Suppress("UNUSED_PARAMETER") streamCtx: Pointer?) {
                onDone()
            }
        }

        // Hold the read lock for the whole blocking stream so close() cannot
        // drop the handle mid-stream.
        lock.readLock().lock()
        try {
            val h = requireHandleLocked()
            val err = AimuxCError()
            val rc = FFI.lib.aimux_stream_text_as_openai(
                h, promptJson, optsJson, partCb, doneCb, null, err,
            )
            if (rc == 0) throwFromC(err)
        } finally {
            lock.readLock().unlock()
        }
    }

    /**
     * Stream text with OpenAI Chat Completions output as a [Sequence] of
     * ChatCompletionChunk JSON strings (RFC-0026).
     */
    fun streamTextAsOpenAISequence(
        promptJson: String,
        optsJson: String? = null,
    ): Sequence<String> = sequence {
        // LinkedBlockingQueue rejects null, so end-of-stream is a sentinel object.
        val eos = Any()
        val parts = java.util.concurrent.LinkedBlockingQueue<Any>()
        var streamError: AimuxException? = null

        try {
            streamTextAsOpenAI(
                promptJson = promptJson,
                optsJson = optsJson,
                onPart = { parts.put(it) },
                onDone = { parts.put(eos) },
            )
        } catch (e: AimuxException) {
            streamError = e
            parts.put(eos)
        }

        while (true) {
            val part = parts.take()
            if (part === eos) break
            yield(part as String)
        }

        streamError?.let { throw it }
    }

    // ── Provider constructors ──────────────────────────────────────────────

    companion object {
        /** Create an OpenAI model instance. */
        fun openai(apiKey: String, modelId: String): Model =
            Model(withCErrorHandle { err -> FFI.lib.aimux_openai_new(apiKey, modelId, err) })

        /** Create an Anthropic model instance. */
        fun anthropic(apiKey: String, modelId: String): Model =
            Model(withCErrorHandle { err -> FFI.lib.aimux_anthropic_new(apiKey, modelId, err) })

        /** Create an OpenAI model instance with a custom base URL. */
        fun openai(apiKey: String, modelId: String, baseUrl: String): Model =
            Model(withCErrorHandle { err ->
                FFI.lib.aimux_openai_new_with_base(apiKey, modelId, baseUrl, err)
            })

        /** Create an Anthropic model instance with a custom base URL. */
        fun anthropic(apiKey: String, modelId: String, baseUrl: String): Model =
            Model(withCErrorHandle { err ->
                FFI.lib.aimux_anthropic_new_with_base(apiKey, modelId, baseUrl, err)
            })

        /** Create a Cohere model instance. */
        fun cohere(apiKey: String, modelId: String): Model =
            Model(withCErrorHandle { err -> FFI.lib.aimux_cohere_new(apiKey, modelId, err) })

        /** Create a Cohere model instance with a custom base URL. */
        fun cohere(apiKey: String, modelId: String, baseUrl: String): Model =
            Model(withCErrorHandle { err ->
                FFI.lib.aimux_cohere_new_with_base(apiKey, modelId, baseUrl, err)
            })

        /** Create a Mistral model instance. */
        fun mistral(apiKey: String, modelId: String): Model =
            Model(withCErrorHandle { err -> FFI.lib.aimux_mistral_new(apiKey, modelId, err) })

        /** Create a Mistral model instance with a custom base URL. */
        fun mistral(apiKey: String, modelId: String, baseUrl: String): Model =
            Model(withCErrorHandle { err ->
                FFI.lib.aimux_mistral_new_with_base(apiKey, modelId, baseUrl, err)
            })

        /** Create an xAI model instance. */
        fun xai(apiKey: String, modelId: String): Model =
            Model(withCErrorHandle { err -> FFI.lib.aimux_xai_new(apiKey, modelId, err) })

        /** Create an xAI model instance with a custom base URL. */
        fun xai(apiKey: String, modelId: String, baseUrl: String): Model =
            Model(withCErrorHandle { err ->
                FFI.lib.aimux_xai_new_with_base(apiKey, modelId, baseUrl, err)
            })

        /** Create a Bedrock model instance (AWS SigV4 credentials). */
        fun bedrock(accessKeyId: String, secretAccessKey: String, region: String, modelId: String): Model =
            Model(withCErrorHandle { err ->
                FFI.lib.aimux_bedrock_new(accessKeyId, secretAccessKey, region, modelId, err)
            })

        /** Create a Bedrock model instance with a custom base URL. */
        fun bedrock(
            accessKeyId: String, secretAccessKey: String, region: String, modelId: String, baseUrl: String
        ): Model =
            Model(withCErrorHandle { err ->
                FFI.lib.aimux_bedrock_new_with_base(accessKeyId, secretAccessKey, region, modelId, baseUrl, err)
            })

        /** Create a Vertex AI model instance (GCP bearer token). */
        fun vertex(accessToken: String, project: String, location: String, modelId: String): Model =
            Model(withCErrorHandle { err ->
                FFI.lib.aimux_vertex_new(accessToken, project, location, modelId, err)
            })

        /** Create a Vertex AI model instance with a custom base URL. */
        fun vertex(
            accessToken: String, project: String, location: String, modelId: String, baseUrl: String
        ): Model =
            Model(withCErrorHandle { err ->
                FFI.lib.aimux_vertex_new_with_base(accessToken, project, location, modelId, baseUrl, err)
            })

        /** Create an Anthropic-on-AWS model instance (API key + region). */
        fun anthropicAws(apiKey: String, region: String, modelId: String): Model =
            Model(withCErrorHandle { err ->
                FFI.lib.aimux_anthropic_aws_new(apiKey, region, modelId, err)
            })

        /** Create an Anthropic-on-AWS model instance with a custom base URL. */
        fun anthropicAws(apiKey: String, region: String, modelId: String, baseUrl: String): Model =
            Model(withCErrorHandle { err ->
                FFI.lib.aimux_anthropic_aws_new_with_base(apiKey, region, modelId, baseUrl, err)
            })

        /** Create an Azure OpenAI model instance (API key + resource name). */
        fun azure(apiKey: String, resourceName: String, deployment: String): Model =
            Model(withCErrorHandle { err ->
                FFI.lib.aimux_azure_new(apiKey, resourceName, deployment, null, err)
            })

        /** Create an Azure OpenAI model instance with an explicit api-version. */
        fun azureWithVersion(apiKey: String, resourceName: String, deployment: String, apiVersion: String): Model =
            Model(withCErrorHandle { err ->
                FFI.lib.aimux_azure_new(apiKey, resourceName, deployment, apiVersion, err)
            })

        /** Create an Azure OpenAI model instance with a custom base URL. */
        fun azureWithBase(apiKey: String, baseUrl: String, deployment: String): Model =
            Model(withCErrorHandle { err ->
                FFI.lib.aimux_azure_new_with_base(apiKey, baseUrl, deployment, null, err)
            })

        /**
         * Create a model from the provider registry by name (RFC-0017 phase 4).
         *
         * @param name       Registry provider name (e.g. `"deepseek"`, `"groq"`).
         * @param apiKey     API key, or null to read the provider's env var from
         *                   the registry entry.
         * @param modelId    Model id.
         * @param configJson Optional JSON object of ProviderOptions; null for defaults.
         */
        fun provider(name: String, apiKey: String? = null, modelId: String, configJson: String? = null): Model =
            Model(withCErrorHandle { err ->
                FFI.lib.aimux_provider_new(name, apiKey, modelId, configJson, err)
            })

        /** Create a model from the provider registry, reading the API key from the provider's env var. */
        fun providerFromEnv(name: String, modelId: String): Model =
            Model(withCErrorHandle { err ->
                FFI.lib.aimux_provider_from_env(name, modelId, err)
            })

        /**
         * Create a **provider handle** (RFC-0027) for a registry-backed provider.
         *
         * Unlike [provider] (which binds to a single modelId), this returns a
         * [ProviderHandle] that supports [ProviderHandle.listModels] and
         * [ProviderHandle.model].
         */
        fun createProvider(name: String, apiKey: String? = null, configJson: String? = null): ProviderHandle =
            ProviderHandle(withCErrorHandle { err ->
                FFI.lib.aimux_provider_handle_new(name, apiKey, configJson, err)
            })

        /**
         * Create a mock replay model from recorded JSONL (RFC-0023).
         *
         * @param recordingsJsonl One `Recording` JSON per line.
         */
        fun mockReplay(recordingsJsonl: String): Model =
            Model(withCErrorHandle { err ->
                FFI.lib.aimux_mock_replay_new(recordingsJsonl, err)
            })

        /**
         * Register external OpenAI-compatible providers from a JSON config string
         * (RFC-0020).
         *
         * `configJson` is `{ "providers": [ { "name", "base_url", ... } ] }`.
         * Entries override same-named built-ins or add new ones. Like
         * `initRecording`, this mutates process-global registry state.
         *
         * The C entry point returns an `int` (1 = success, 0 = failure) rather
         * than a handle, so the rc is checked inline and `throwFromC` is called
         * directly (no `extractHandle`).
         *
         * @throws AimuxException if the C call fails (rc == 0).
         */
        fun registerProviders(configJson: String) {
            val err = AimuxCError()
            val rc = FFI.lib.aimux_register_providers(configJson, err)
            if (rc == 0) throwFromC(err)
        }

        /**
         * Set the global proxy configuration (M6, RFC-0016). Must be called
         * before the first `generateText` / `streamText` call; a no-op if the
         * shared HTTP client is already initialised.
         *
         * The C entry point returns an `int` (1 = success, 0 = failure).
         *
         * @param configJson ProxyConfig JSON (`http_url`, `https_url`,
         *   `all_url`, `no_proxy` — all optional).
         * @throws AimuxException if the C call fails (rc == 0).
         */
        fun initProxy(configJson: String) {
            val err = AimuxCError()
            val rc = FFI.lib.aimux_init_proxy(configJson, err)
            if (rc == 0) throwFromC(err)
        }
    }
}

/**
 * A provider handle — created by `Model.createProvider`, supports `listModels()`
 * (runtime discovery) and `model()` (build a model from a discovered id).
 *
 * **Thread-safety / concurrency.** Safe for concurrent use. The native handle
 * is guarded by a Go-style [ReentrantReadWriteLock] (fair, FIFO):
 * [listModels] and [model] hold the _read_ lock for the entire FFI call, and
 * [close] takes the _write_ lock. [close] therefore blocks until in-flight
 * calls finish before dropping the native handle — closes the check-then-use
 * use-after-free race where a caller could pass the closed check and then race
 * with [close]'s drop.
 */
class ProviderHandle internal constructor(handle: Long) : AutoCloseable {

    // Go-style read/write lock — see Model for the rationale. Each FFI call
    // (listModels/model) holds the read lock for its whole duration; close()
    // takes the write lock and waits for in-flight calls before dropping the
    // handle, preventing a use-after-free.
    private val lock = ReentrantReadWriteLock(true)
    private var handle: Long = handle
    private var closed: Boolean = false

    /**
     * Release the native handle. Idempotent and thread-safe: subsequent calls
     * are no-ops. Acquires the write lock and blocks until in-flight
     * [listModels] / [model] calls finish before dropping the native handle
     * (prevents use-after-free).
     */
    override fun close() {
        lock.writeLock().lock()
        try {
            if (closed || handle == 0L) {
                return
            }
            val h = handle
            handle = 0L
            closed = true
            FFI.lib.aimux_drop_handle(h)
        } finally {
            lock.writeLock().unlock()
        }
    }

    protected fun finalize() = close()

    // Caller MUST already hold the read lock; held across the FFI call so
    // close() cannot drop the handle mid-call.
    private fun requireHandleLocked(): Long {
        check(!closed && handle != 0L) { "ProviderHandle is closed" }
        return handle
    }

    /**
     * List models available on this provider (runtime discovery + anya2a spec).
     * Returns a JSON array of ResolvedModel.
     */
    fun listModels(): String {
        lock.readLock().lock()
        try {
            val h = requireHandleLocked()
            return withCErrorString { err ->
                FFI.lib.aimux_provider_list_models(h, err)
            }
        } finally {
            lock.readLock().unlock()
        }
    }

    /** Build a language model from a discovered model id. */
    fun model(modelId: String): Model {
        lock.readLock().lock()
        try {
            val h = requireHandleLocked()
            val newHandle = withCErrorHandle { err ->
                FFI.lib.aimux_provider_model(h, modelId, err)
            }
            return Model(newHandle)
        } finally {
            lock.readLock().unlock()
        }
    }
}

/**
 * Fetch the community model catalogue (anya2a). Returns a JSON-serialized
 * Catalogue string. Thin fetch — no caching.
 *
 * @param sourceUrl Optional URL override (null = default endpoint).
 */
fun getModelSpecs(sourceUrl: String? = null): String =
    withCErrorString { err ->
        FFI.lib.aimux_get_model_specs(sourceUrl, err)
    }
