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

// ─────────────────────────────────────────────────────────────────────────────
// JNA interface — direct mapping to the C ABI.
// ─────────────────────────────────────────────────────────────────────────────

internal interface AimuxFFI : Library {
    fun aimux_openai_new(apiKey: String, modelId: String): Long
    fun aimux_anthropic_new(apiKey: String, modelId: String): Long
    fun aimux_openai_new_with_base(apiKey: String, modelId: String, baseUrl: String): Long
    fun aimux_anthropic_new_with_base(apiKey: String, modelId: String, baseUrl: String): Long

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
class Model private constructor(private val handle: Long) : Closeable {

    override fun close() {
        if (handle != 0L) {
            FFI.lib.aimux_drop_handle(handle)
        }
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
        val ptr = FFI.lib.aimux_generate_text(handle, promptJson, optsJson)
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

        FFI.lib.aimux_stream_text(handle, promptJson, optsJson, partCb, doneCb, errCb)
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
