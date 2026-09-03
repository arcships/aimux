/**
 * aimux — Unified LLM service layer for Kotlin/JVM (Rust core, 172+ providers).
 *
 * Uses JNA to call the aimux-ffi C ABI. This is the C ABI path (§3.2).
 * The native library (libaimux_ffi.so / .dylib / .dll) must be on the
 * library path or bundled in the JAR's native/ directory.
 *
 * Errors: every fallible C call returns an `aimux_error_t *` ([Pointer]?):
 * null = success, result written to the trailing out-parameter
 * ([LongByReference] for handles, [PointerByReference] for JSON strings);
 * non-null = failure. Its unified code identifies [AimuxException] (1..14),
 * [RecordingException] (100..105), or a C ABI failure (200..206). The last
 * range maps to `IllegalStateException("aimux ffi: …")`. A decoder releases
 * the returned pointer with `aimux_error_free`.
 * No JSON error envelope on the primary path.
 */

package ai.arcships.aimux

import com.sun.jna.Callback
import com.sun.jna.Library
import com.sun.jna.Native
import com.sun.jna.Pointer
import com.sun.jna.ptr.IntByReference
import com.sun.jna.ptr.LongByReference
import com.sun.jna.ptr.PointerByReference
import java.io.Closeable
import java.util.concurrent.locks.ReentrantReadWriteLock

// ─────────────────────────────────────────────────────────────────────────────
// JNA interface — direct mapping to the C ABI (aimux_error_t * return,
// uint64_t *out_handle / char **out_json out-params).
// ─────────────────────────────────────────────────────────────────────────────

internal interface AimuxFFI : Library {
    // apiKey nullable only so tests can exercise the NULL-argument C ABI failure; Model.openai never passes null.
    fun aimux_openai_new(apiKey: String?, modelId: String, outHandle: LongByReference): Pointer?
    fun aimux_anthropic_new(apiKey: String, modelId: String, outHandle: LongByReference): Pointer?
    fun aimux_openai_new_with_base(apiKey: String, modelId: String, baseUrl: String, outHandle: LongByReference): Pointer?
    fun aimux_anthropic_new_with_base(apiKey: String, modelId: String, baseUrl: String, outHandle: LongByReference): Pointer?
    fun aimux_cohere_new(apiKey: String, modelId: String, outHandle: LongByReference): Pointer?
    fun aimux_cohere_new_with_base(apiKey: String, modelId: String, baseUrl: String, outHandle: LongByReference): Pointer?
    fun aimux_mistral_new(apiKey: String, modelId: String, outHandle: LongByReference): Pointer?
    fun aimux_mistral_new_with_base(apiKey: String, modelId: String, baseUrl: String, outHandle: LongByReference): Pointer?
    fun aimux_xai_new(apiKey: String, modelId: String, outHandle: LongByReference): Pointer?
    fun aimux_xai_new_with_base(apiKey: String, modelId: String, baseUrl: String, outHandle: LongByReference): Pointer?
    fun aimux_bedrock_new(
        accessKeyId: String, secretAccessKey: String, region: String, modelId: String, outHandle: LongByReference
    ): Pointer?
    fun aimux_bedrock_new_with_base(
        accessKeyId: String, secretAccessKey: String, region: String, modelId: String, baseUrl: String, outHandle: LongByReference
    ): Pointer?
    fun aimux_vertex_new(
        accessToken: String, project: String, location: String, modelId: String, outHandle: LongByReference
    ): Pointer?
    fun aimux_vertex_new_with_base(
        accessToken: String, project: String, location: String, modelId: String, baseUrl: String, outHandle: LongByReference
    ): Pointer?
    fun aimux_anthropic_aws_new(apiKey: String, region: String, modelId: String, outHandle: LongByReference): Pointer?
    fun aimux_anthropic_aws_new_with_base(
        apiKey: String, region: String, modelId: String, baseUrl: String, outHandle: LongByReference
    ): Pointer?
    fun aimux_azure_new(
        apiKey: String, resourceName: String, deployment: String, apiVersion: String?, outHandle: LongByReference
    ): Pointer?
    fun aimux_azure_new_with_base(
        apiKey: String, baseUrl: String, deployment: String, apiVersion: String?, outHandle: LongByReference
    ): Pointer?

    // ── Registry provider (RFC-0017 phase 4) ───────────────────────────────
    fun aimux_provider_new(name: String, apiKey: String?, modelId: String, configJson: String?, outHandle: LongByReference): Pointer?
    fun aimux_provider_from_env(name: String, modelId: String, outHandle: LongByReference): Pointer?

    // ── Provider handles (RFC-0027) ─────────────────────────────────────────
    fun aimux_provider_handle_new(name: String, apiKey: String?, configJson: String?, outHandle: LongByReference): Pointer?
    fun aimux_provider_list_models(handle: Long, outModelsJson: PointerByReference): Pointer?
    fun aimux_provider_model(handle: Long, modelId: String, outHandle: LongByReference): Pointer?
    fun aimux_get_model_specs(sourceUrl: String?, outSpecsJson: PointerByReference): Pointer?

    fun aimux_generate_text(handle: Long, promptJson: String, optsJson: String?, outJson: PointerByReference): Pointer?
    fun aimux_generate_object(handle: Long, promptJson: String, optsJson: String?, outJson: PointerByReference): Pointer?
    fun aimux_consume_stream_text(handle: Long, promptJson: String, optsJson: String?, outJson: PointerByReference): Pointer?
    // NULL after on_done = clean end; non-NULL = failure (no on_done).
    fun aimux_stream_text(
        handle: Long,
        promptJson: String,
        optsJson: String?,
        onPart: Callback?,
        onDone: Callback?,
        streamCtx: Pointer?,
    ): Pointer?

    // ── OpenAI-compatible output (RFC-0026) ─────────────────────────────────
    fun aimux_generate_text_as_openai(handle: Long, promptJson: String, optsJson: String?, outJson: PointerByReference): Pointer?
    fun aimux_stream_text_as_openai(
        handle: Long,
        promptJson: String,
        optsJson: String?,
        onPart: Callback?,
        onDone: Callback?,
        streamCtx: Pointer?,
    ): Pointer?

    fun aimux_drop_handle(handle: Long)
    fun aimux_free_string(ptr: Pointer?)

    // ── Returned error (aimux-error.h): aimux_error_t *. Release exactly once
    // with aimux_error_free (NULL-safe).
    // Errors are not handles: never pass one to aimux_drop_handle.
    fun aimux_error_free(err: Pointer?)
    // ── Unified code and getters (const aimux_error_t *; NULL-safe). Strings are
    // owned: aimux_free_string. Payload getters return NULL / -1 under any other code.
    fun aimux_error_code(error: Pointer?): Int
    fun aimux_error_message(error: Pointer?): Pointer?
    fun aimux_error_retryable(error: Pointer?): Int
    fun aimux_error_status(error: Pointer?): Int
    fun aimux_error_retry_ms(error: Pointer?): Long
    fun aimux_error_provider_code(error: Pointer?): Pointer?
    fun aimux_error_provider_message(error: Pointer?): Pointer?
    fun aimux_error_response_body(error: Pointer?): Pointer?
    fun aimux_error_url(error: Pointer?): Pointer?
    fun aimux_error_request_body_values(error: Pointer?): Pointer?
    fun aimux_error_response_headers(error: Pointer?): Pointer?
    fun aimux_error_provider_data(error: Pointer?): Pointer?
    // AIMUX_E_RETRY payload: reason wire name, attempt count, and each attempt
    // as a NEW OWNED error the caller frees with aimux_error_free.
    fun aimux_error_retry_reason(error: Pointer?): Pointer?
    fun aimux_error_retry_count(error: Pointer?): Int
    fun aimux_error_retry_error_at(error: Pointer?, index: Int): Pointer?
    fun aimux_error_model_id(error: Pointer?): Pointer?
    fun aimux_error_model_type(error: Pointer?): Pointer?
    fun aimux_error_provider_id(error: Pointer?): Pointer?

    // ── Embedding ──────────────────────────────────────────────────────────
    fun aimux_openai_embedding_new(apiKey: String, modelId: String, outHandle: LongByReference): Pointer?
    fun aimux_openai_embedding_new_with_base(apiKey: String, modelId: String, baseUrl: String, outHandle: LongByReference): Pointer?
    fun aimux_cohere_embedding_new(apiKey: String, modelId: String, outHandle: LongByReference): Pointer?
    fun aimux_cohere_embedding_new_with_base(apiKey: String, modelId: String, baseUrl: String, outHandle: LongByReference): Pointer?
    fun aimux_google_embedding_new(apiKey: String, modelId: String, outHandle: LongByReference): Pointer?
    fun aimux_google_embedding_new_with_base(apiKey: String, modelId: String, baseUrl: String, outHandle: LongByReference): Pointer?
    fun aimux_embed(handle: Long, valuesJson: String, optsJson: String?, outJson: PointerByReference): Pointer?

    // ── Speech (TTS) ───────────────────────────────────────────────────────
    fun aimux_openai_speech_new(apiKey: String, modelId: String, outHandle: LongByReference): Pointer?
    fun aimux_openai_speech_new_with_base(apiKey: String, modelId: String, baseUrl: String, outHandle: LongByReference): Pointer?
    fun aimux_speech_generate(handle: Long, optsJson: String, outJson: PointerByReference): Pointer?

    // ── Image ──────────────────────────────────────────────────────────────
    fun aimux_openai_image_new(apiKey: String, modelId: String, outHandle: LongByReference): Pointer?
    fun aimux_openai_image_new_with_base(apiKey: String, modelId: String, baseUrl: String, outHandle: LongByReference): Pointer?
    fun aimux_google_image_new(apiKey: String, modelId: String, outHandle: LongByReference): Pointer?
    fun aimux_google_image_new_with_base(apiKey: String, modelId: String, baseUrl: String, outHandle: LongByReference): Pointer?
    fun aimux_image_generate(handle: Long, optsJson: String, outJson: PointerByReference): Pointer?

    // ── Transcription (STT) ────────────────────────────────────────────────
    fun aimux_openai_transcription_new(apiKey: String, modelId: String, outHandle: LongByReference): Pointer?
    fun aimux_openai_transcription_new_with_base(
        apiKey: String, modelId: String, baseUrl: String, outHandle: LongByReference
    ): Pointer?
    fun aimux_transcription_generate(
        handle: Long, audioBase64: String, mediaType: String, optsJson: String?, outJson: PointerByReference
    ): Pointer?

    // ── Files ──────────────────────────────────────────────────────────────
    fun aimux_openai_files_new(apiKey: String, outHandle: LongByReference): Pointer?
    fun aimux_openai_files_new_with_base(apiKey: String, baseUrl: String, outHandle: LongByReference): Pointer?
    fun aimux_file_upload(
        handle: Long, dataBase64: String, mediaType: String, optsJson: String?, outJson: PointerByReference
    ): Pointer?

    // ── Reranking ──────────────────────────────────────────────────────────
    fun aimux_cohere_reranking_new(apiKey: String, modelId: String, outHandle: LongByReference): Pointer?
    fun aimux_cohere_reranking_new_with_base(apiKey: String, modelId: String, baseUrl: String, outHandle: LongByReference): Pointer?
    fun aimux_rerank(handle: Long, optsJson: String, outJson: PointerByReference): Pointer?

    // ── Video ──────────────────────────────────────────────────────────────
    fun aimux_google_video_new(apiKey: String, modelId: String, outHandle: LongByReference): Pointer?
    fun aimux_google_video_new_with_base(apiKey: String, modelId: String, baseUrl: String, outHandle: LongByReference): Pointer?
    fun aimux_video_generate(handle: Long, optsJson: String, outJson: PointerByReference): Pointer?

    // ── Search ─────────────────────────────────────────────────────────────
    fun aimux_tavily_search_new(apiKey: String, modelId: String, outHandle: LongByReference): Pointer?
    fun aimux_tavily_search_new_with_base(apiKey: String, modelId: String, baseUrl: String, outHandle: LongByReference): Pointer?
    fun aimux_search(handle: Long, optsJson: String, outJson: PointerByReference): Pointer?

    // Logging (RFC-0014). [C ABI]: only C ABI failures.
    fun aimux_init_logging(level: String): Pointer?

    // ── Recording + mock replay (RFC-0023) ──────────────────────────────────
    // [RecordingError] NULL = ok; otherwise code 100..102 (previous recorder untouched).
    fun aimux_init_recording(dir: String): Pointer?
    // [AiMuxError] cap == 0 is AIMUX_E_INVALID_ARGUMENT.
    fun aimux_init_recording_ring(cap: Long): Pointer?
    fun aimux_init_recording_ring_default()
    fun aimux_recording_stop()
    fun aimux_recording_flush()
    // [RecordingError] NULL = data on disk (also when recording was never initialised).
    fun aimux_recording_try_flush(): Pointer?
    fun aimux_mock_replay_new(recordingsJsonl: String, outHandle: LongByReference): Pointer?

    // [AiMuxError] Register external providers from JSON config (RFC-0020).
    fun aimux_register_providers(configJson: String): Pointer?

    // [AiMuxError] Set the global proxy configuration (M6, RFC-0016).
    fun aimux_init_proxy(configJson: String): Pointer?

    // [AiMuxError] Create a RouterModel (RFC-0021) over child handles (LongArray is
    // pinned to uint64_t* by JNA). configJson may be null.
    fun aimux_router_new(handles: LongArray, len: Long, configJson: String?, outHandle: LongByReference): Pointer?

    // [AiMuxError] Create a MoaModel (RFC-0022) over reference handles + one aggregator.
    // referenceHandles may be null/empty (degrades to aggregator-only).
    fun aimux_moa_new(referenceHandles: LongArray?, refLen: Long, aggregator: Long, configJson: String?, outHandle: LongByReference): Pointer?

    // Transcription streaming sessions (RFC-0028).

    fun aimux_transcription_session_new(modelHandle: Long, abortHandle: Long, optsJson: String?, outHandle: LongByReference): Pointer?

    // [AiMuxError] Blocking while the channel is full.
    fun aimux_transcription_push_audio(session: Long, data: ByteArray?, len: Long): Pointer?

    // [C ABI] Fails only for a dead handle.
    fun aimux_transcription_input_done(session: Long): Pointer?

    // [AiMuxError] NULL + *outState PART (outPart holds JSON) / ENDED / TIMEOUT (outPart NULL).
    fun aimux_transcription_next_part(session: Long, timeoutMs: Long, outPart: PointerByReference, outState: IntByReference): Pointer?

    fun aimux_transcription_session_drop(session: Long)
}

internal object FFI {
    val lib: AimuxFFI = Native.load("aimux_ffi", AimuxFFI::class.java)
}

/** `aimux_transcription_next_part_state_t` values written to `outState` (`int32_t*` in the header). */
internal const val TRANSCRIPTION_NEXT_PART_PART = 1
internal const val TRANSCRIPTION_NEXT_PART_ENDED = 2
internal const val TRANSCRIPTION_NEXT_PART_TIMEOUT = 3

// ─────────────────────────────────────────────────────────────────────────────
// Error decoding — one decoder per call site, chosen by what the call can produce.
// ─────────────────────────────────────────────────────────────────────────────

/** Copy an owned C string and free it; null stays null. */
internal fun takeString(p: Pointer?): String? = p?.let {
    try {
        it.getString(0, "UTF-8")
    } finally {
        FFI.lib.aimux_free_string(it)
    }
}

private fun prefixOf(context: String): String = if (context.isEmpty()) "" else "$context: "

/** Read the returned error's message; does not free. */
private fun ffiError(e: Pointer, prefix: String): IllegalStateException {
    val msg = takeString(FFI.lib.aimux_error_message(e))?.ifEmpty { null } ?: "C ABI failure"
    return IllegalStateException("${prefix}aimux ffi: $msg")
}

/**
 * Decode an error from a call that may return `AiMuxError`: 1..14 →
 * [AimuxException]; 200..206 → [IllegalStateException]. Frees [e].
 */
internal fun expectAimuxError(e: Pointer, context: String = ""): RuntimeException {
    val prefix = prefixOf(context)
    try {
        val code = FFI.lib.aimux_error_code(e)
        if (isFfiCode(code)) return ffiError(e, prefix)
        check(code in AIMUX_E_OTHER..AIMUX_E_ABORTED || code == AIMUX_E_RETRY) {
            "${prefix}aimux ffi: expected AiMuxError code, got $code"
        }
        return AimuxException.fromC(e, prefix)
    } finally {
        FFI.lib.aimux_error_free(e)
    }
}

/**
 * Decode an error from a recording call: 100..105 → [RecordingException];
 * 200..206 → [IllegalStateException]. Frees [e].
 */
internal fun expectRecordingError(e: Pointer, context: String = ""): RuntimeException {
    val prefix = prefixOf(context)
    try {
        val code = FFI.lib.aimux_error_code(e)
        if (isFfiCode(code)) return ffiError(e, prefix)
        check(RecordingErrorCode.isCCode(code)) {
            "${prefix}aimux ffi: expected RecordingError code, got $code"
        }
        return RecordingException.fromC(e, prefix)
    } finally {
        FFI.lib.aimux_error_free(e)
    }
}

/**
 * Decode an error from a call that only exposes C ABI failures: message →
 * [IllegalStateException]. Deletes [e].
 */
internal fun expectFfiError(e: Pointer, context: String = ""): RuntimeException {
    try {
        val code = FFI.lib.aimux_error_code(e)
        check(isFfiCode(code)) {
            "${prefixOf(context)}aimux ffi: expected C ABI failure code, got $code"
        }
        return ffiError(e, prefixOf(context))
    } finally {
        FFI.lib.aimux_error_free(e)
    }
}

private fun isFfiCode(code: Int): Boolean = code in 200..206

/**
 * Reject malformed raw JSON before it crosses the C ABI, so a caller's
 * bad input is an [IllegalArgumentException] naming the parameter rather than
 * a C ABI failure. [requireJson] is for optional (`String?`) parameters —
 * null / empty means "default"; [requireJsonRequired] is for required
 * (`String`) parameters, which must not be blank.
 */
internal fun requireJson(name: String, json: String?) {
    if (json.isNullOrEmpty()) return
    requireJsonRequired(name, json)
}

/** Required-parameter variant: distinct name because `String`/`String?` share one JVM signature. */
internal fun requireJsonRequired(name: String, json: String) {
    // ponytail: full parse (the FFI parses again); swap for a syntax-only scan if large prompts show up in profiles.
    require(json.isNotBlank()) { "$name: invalid JSON: empty" }
    try {
        AimuxJson.parseToJsonElement(json)
    } catch (e: kotlinx.serialization.SerializationException) {
        throw IllegalArgumentException("$name: ${e.message}", e)
    }
}

/** Run a constructor: null error → the handle written to `out`; otherwise throw [expectAimuxError]. */
internal inline fun handleResult(context: String = "", block: (LongByReference) -> Pointer?): Long {
    val out = LongByReference(0)
    block(out)?.let { throw expectAimuxError(it, context) }
    return out.value
}

/** Run a JSON-result call: null error → the caller-owned string written to `out` (freed here); otherwise throw [expectAimuxError]. */
internal inline fun stringResult(context: String = "", block: (PointerByReference) -> Pointer?): String {
    val out = PointerByReference()
    block(out)?.let { throw expectAimuxError(it, context) }
    return takeString(out.value)
        ?: throw IllegalStateException("${if (context.isEmpty()) "" else "$context: "}aimux ffi: success with NULL result")
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
        if (closed || handle == 0L) throw IllegalStateException("Model is closed")
        return handle
    }

    /** Internal handle read for composite-model factories (router/moa). */
    internal fun handle(): Long {
        lock.readLock().lock()
        try {
            return requireHandleLocked()
        } finally {
            lock.readLock().unlock()
        }
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
     * @throws AimuxException on AiMuxError (typed subclass via C AimuxError).
     * @throws IllegalArgumentException when a raw JSON argument is malformed; IllegalStateException after [close].
     */
    fun generateText(promptJson: String, optsJson: String? = null): String {
        requireJsonRequired("promptJson", promptJson)
        requireJson("optsJson", optsJson)
        lock.readLock().lock()
        try {
            val h = requireHandleLocked()
            return stringResult { out ->
                FFI.lib.aimux_generate_text(h, promptJson, optsJson, out)
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
     * [optsJson] for schema control; aimux-core applies JSON repair before
     * parsing.
     *
     * @param promptJson JSON prompt string (bare value or {"prompt": ...}).
     * @param optsJson Optional JSON-serialized GenerateTextOptions.
     * @return JSON-serialized GenerateObjectResult.
     * @throws AimuxException on AiMuxError.
     * @throws IllegalArgumentException when a raw JSON argument is malformed; IllegalStateException after [close].
     */
    fun generateObject(promptJson: String, optsJson: String? = null): String {
        requireJsonRequired("promptJson", promptJson)
        requireJson("optsJson", optsJson)
        lock.readLock().lock()
        try {
            val h = requireHandleLocked()
            return stringResult { out ->
                FFI.lib.aimux_generate_object(h, promptJson, optsJson, out)
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
     * @throws AimuxException on AiMuxError.
     * @throws IllegalArgumentException when a raw JSON argument is malformed; IllegalStateException after [close].
     */
    fun consumeStreamText(promptJson: String, optsJson: String? = null): String {
        requireJsonRequired("promptJson", promptJson)
        requireJson("optsJson", optsJson)
        lock.readLock().lock()
        try {
            val h = requireHandleLocked()
            return stringResult { out ->
                FFI.lib.aimux_consume_stream_text(h, promptJson, optsJson, out)
            }
        } finally {
            lock.readLock().unlock()
        }
    }

    /**
     * Stream text from the model.
     *
     * Blocks the calling thread until the stream completes. Recoverable
     * frame errors (a malformed SSE frame) arrive as `StreamPart::Error`
     * data parts and the stream continues; only transport/Core failures
     * throw [AimuxException] (on_done is not invoked on failure), after any
     * parts already delivered to [onPart].
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
        requireJsonRequired("promptJson", promptJson)
        requireJson("optsJson", optsJson)
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
            FFI.lib.aimux_stream_text(h, promptJson, optsJson, partCb, doneCb, null)
                ?.let { throw expectAimuxError(it, "streamText") }
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
     * @throws AimuxException on AiMuxError.
     * @throws IllegalArgumentException when a raw JSON argument is malformed; IllegalStateException after [close].
     */
    fun generateTextAsOpenAI(promptJson: String, optsJson: String? = null): String {
        requireJsonRequired("promptJson", promptJson)
        requireJson("optsJson", optsJson)
        lock.readLock().lock()
        try {
            val h = requireHandleLocked()
            return stringResult { out ->
                FFI.lib.aimux_generate_text_as_openai(h, promptJson, optsJson, out)
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
        requireJsonRequired("promptJson", promptJson)
        requireJson("optsJson", optsJson)
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
            FFI.lib.aimux_stream_text_as_openai(h, promptJson, optsJson, partCb, doneCb, null)
                ?.let { throw expectAimuxError(it, "streamTextAsOpenAI") }
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
            Model(handleResult { out -> FFI.lib.aimux_openai_new(apiKey, modelId, out) })

        /** Create an Anthropic model instance. */
        fun anthropic(apiKey: String, modelId: String): Model =
            Model(handleResult { out -> FFI.lib.aimux_anthropic_new(apiKey, modelId, out) })

        /** Create an OpenAI model instance with a custom base URL. */
        fun openai(apiKey: String, modelId: String, baseUrl: String): Model =
            Model(handleResult { out ->
                FFI.lib.aimux_openai_new_with_base(apiKey, modelId, baseUrl, out)
            })

        /** Create an Anthropic model instance with a custom base URL. */
        fun anthropic(apiKey: String, modelId: String, baseUrl: String): Model =
            Model(handleResult { out ->
                FFI.lib.aimux_anthropic_new_with_base(apiKey, modelId, baseUrl, out)
            })

        /** Create a Cohere model instance. */
        fun cohere(apiKey: String, modelId: String): Model =
            Model(handleResult { out -> FFI.lib.aimux_cohere_new(apiKey, modelId, out) })

        /** Create a Cohere model instance with a custom base URL. */
        fun cohere(apiKey: String, modelId: String, baseUrl: String): Model =
            Model(handleResult { out ->
                FFI.lib.aimux_cohere_new_with_base(apiKey, modelId, baseUrl, out)
            })

        /** Create a Mistral model instance. */
        fun mistral(apiKey: String, modelId: String): Model =
            Model(handleResult { out -> FFI.lib.aimux_mistral_new(apiKey, modelId, out) })

        /** Create a Mistral model instance with a custom base URL. */
        fun mistral(apiKey: String, modelId: String, baseUrl: String): Model =
            Model(handleResult { out ->
                FFI.lib.aimux_mistral_new_with_base(apiKey, modelId, baseUrl, out)
            })

        /** Create an xAI model instance. */
        fun xai(apiKey: String, modelId: String): Model =
            Model(handleResult { out -> FFI.lib.aimux_xai_new(apiKey, modelId, out) })

        /** Create an xAI model instance with a custom base URL. */
        fun xai(apiKey: String, modelId: String, baseUrl: String): Model =
            Model(handleResult { out ->
                FFI.lib.aimux_xai_new_with_base(apiKey, modelId, baseUrl, out)
            })

        /** Create a Bedrock model instance (AWS SigV4 credentials). */
        fun bedrock(accessKeyId: String, secretAccessKey: String, region: String, modelId: String): Model =
            Model(handleResult { out ->
                FFI.lib.aimux_bedrock_new(accessKeyId, secretAccessKey, region, modelId, out)
            })

        /** Create a Bedrock model instance with a custom base URL. */
        fun bedrock(
            accessKeyId: String, secretAccessKey: String, region: String, modelId: String, baseUrl: String
        ): Model =
            Model(handleResult { out ->
                FFI.lib.aimux_bedrock_new_with_base(accessKeyId, secretAccessKey, region, modelId, baseUrl, out)
            })

        /** Create a Vertex AI model instance (GCP bearer token). */
        fun vertex(accessToken: String, project: String, location: String, modelId: String): Model =
            Model(handleResult { out ->
                FFI.lib.aimux_vertex_new(accessToken, project, location, modelId, out)
            })

        /** Create a Vertex AI model instance with a custom base URL. */
        fun vertex(
            accessToken: String, project: String, location: String, modelId: String, baseUrl: String
        ): Model =
            Model(handleResult { out ->
                FFI.lib.aimux_vertex_new_with_base(accessToken, project, location, modelId, baseUrl, out)
            })

        /** Create an Anthropic-on-AWS model instance (API key + region). */
        fun anthropicAws(apiKey: String, region: String, modelId: String): Model =
            Model(handleResult { out ->
                FFI.lib.aimux_anthropic_aws_new(apiKey, region, modelId, out)
            })

        /** Create an Anthropic-on-AWS model instance with a custom base URL. */
        fun anthropicAws(apiKey: String, region: String, modelId: String, baseUrl: String): Model =
            Model(handleResult { out ->
                FFI.lib.aimux_anthropic_aws_new_with_base(apiKey, region, modelId, baseUrl, out)
            })

        /** Create an Azure OpenAI model instance (API key + resource name). */
        fun azure(apiKey: String, resourceName: String, deployment: String): Model =
            Model(handleResult { out ->
                FFI.lib.aimux_azure_new(apiKey, resourceName, deployment, null, out)
            })

        /** Create an Azure OpenAI model instance with an explicit api-version. */
        fun azureWithVersion(apiKey: String, resourceName: String, deployment: String, apiVersion: String): Model =
            Model(handleResult { out ->
                FFI.lib.aimux_azure_new(apiKey, resourceName, deployment, apiVersion, out)
            })

        /** Create an Azure OpenAI model instance with a custom base URL. */
        fun azureWithBase(apiKey: String, baseUrl: String, deployment: String): Model =
            Model(handleResult { out ->
                FFI.lib.aimux_azure_new_with_base(apiKey, baseUrl, deployment, null, out)
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
        fun provider(name: String, apiKey: String? = null, modelId: String, configJson: String? = null): Model {
            requireJson("configJson", configJson)
            return Model(handleResult { out ->
                FFI.lib.aimux_provider_new(name, apiKey, modelId, configJson, out)
            })
        }

        /** Create a model from the provider registry, reading the API key from the provider's env var. */
        fun providerFromEnv(name: String, modelId: String): Model =
            Model(handleResult { out ->
                FFI.lib.aimux_provider_from_env(name, modelId, out)
            })

        /**
         * Create a **provider handle** (RFC-0027) for a registry-backed provider.
         *
         * Unlike [provider] (which binds to a single modelId), this returns a
         * [ProviderHandle] that supports [ProviderHandle.listModels] and
         * [ProviderHandle.model].
         */
        fun createProvider(name: String, apiKey: String? = null, configJson: String? = null): ProviderHandle {
            requireJson("configJson", configJson)
            return ProviderHandle(handleResult { out ->
                FFI.lib.aimux_provider_handle_new(name, apiKey, configJson, out)
            })
        }

        /**
         * Create a mock replay model from recorded JSONL (RFC-0023).
         *
         * @param recordingsJsonl One `Recording` JSON per line.
         * @throws IllegalArgumentException if a non-blank line is not valid JSON.
         */
        fun mockReplay(recordingsJsonl: String): Model {
            recordingsJsonl.lineSequence().filter { it.isNotBlank() }.forEach { requireJsonRequired("recordingsJsonl", it) }
            return Model(handleResult { out ->
                FFI.lib.aimux_mock_replay_new(recordingsJsonl, out)
            })
        }

        /**
         * Create a RouterModel (RFC-0021) over the given child models. The
         * returned model routes each call to one child and falls back across
         * the rest on error (per `configJson`).
         *
         * @param models     child models (must be non-empty).
         * @param configJson optional config: `{"router": "rule"|"weighted",
         *                   "weights": [...], "fallback": "on_error"|"none",
         *                   "provider_name", "model_id"}` — all optional.
         * @throws IllegalArgumentException if `configJson` is malformed JSON.
         * @throws AimuxException on AiMuxError.
         */
        fun router(models: List<Model>, configJson: String? = null): Model {
            require(models.isNotEmpty()) { "router: models must be non-empty" }
            requireJson("configJson", configJson)
            val handles = LongArray(models.size) { models[it].handle() }
            return Model(handleResult { out ->
                FFI.lib.aimux_router_new(handles, handles.size.toLong(), configJson, out)
            })
        }

        /**
         * Create a MoaModel (RFC-0022) over reference models + one aggregator.
         * References fan out in parallel, then the aggregator synthesizes a
         * final answer.
         *
         * @param references reference models (may be empty — runs aggregator only).
         * @param aggregator the aggregator model.
         * @param configJson optional MoaConfig.
         * @throws IllegalArgumentException if `configJson` is malformed JSON.
         * @throws AimuxException on AiMuxError.
         */
        fun moa(
            references: List<Model>,
            aggregator: Model,
            configJson: String? = null,
        ): Model {
            requireJson("configJson", configJson)
            val refHandles =
                if (references.isEmpty()) null
                else LongArray(references.size) { references[it].handle() }
            val refLen = refHandles?.size?.toLong() ?: 0L
            return Model(handleResult { out ->
                FFI.lib.aimux_moa_new(refHandles, refLen, aggregator.handle(), configJson, out)
            })
        }

        /**
         * Register external OpenAI-compatible providers from a JSON config string
         * (RFC-0020).
         *
         * `configJson` is `{ "providers": [ { "name", "base_url", ... } ] }`.
         * Entries override same-named built-ins or add new ones. Like
         * `initRecording`, this mutates process-global registry state.
         *
         * @throws IllegalArgumentException if `configJson` is blank or malformed JSON.
         * @throws AimuxException on AiMuxError (bad schema / unknown protocol).
         */
        fun registerProviders(configJson: String) {
            requireJsonRequired("configJson", configJson)
            FFI.lib.aimux_register_providers(configJson)?.let { throw expectAimuxError(it, "registerProviders") }
        }

        /**
         * Set the global proxy configuration (M6, RFC-0016). Must be called
         * before the first `generateText` / `streamText` call; a no-op if the
         * shared HTTP client is already initialised.
         *
         * @param configJson ProxyConfig JSON (`http_url`, `https_url`,
         *   `all_url`, `no_proxy` — all optional).
         * @throws IllegalArgumentException if `configJson` is blank or malformed JSON.
         * @throws AimuxException on AiMuxError (wrong shape).
         */
        fun initProxy(configJson: String) {
            requireJsonRequired("configJson", configJson)
            FFI.lib.aimux_init_proxy(configJson)?.let { throw expectAimuxError(it, "initProxy") }
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
        if (closed || handle == 0L) throw IllegalStateException("ProviderHandle is closed")
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
            return stringResult { out ->
                FFI.lib.aimux_provider_list_models(h, out)
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
            val newHandle = handleResult { out ->
                FFI.lib.aimux_provider_model(h, modelId, out)
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
    stringResult { out ->
        FFI.lib.aimux_get_model_specs(sourceUrl, out)
    }
