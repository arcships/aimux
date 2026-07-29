/**
 * aimux — typed wrapper over the raw JSON-string [Model] API.
 *
 * [TypedModel] eliminates the JSON string boundary: inputs and outputs are
 * Kotlin data classes (see [Types.kt]). The raw [Model] (JNA → C ABI) is left
 * untouched and remains available for callers that need the untyped escape
 * hatch.
 *
 * ```kotlin
 * TypedModel.openai("sk-...", "gpt-4o", baseUrl).use { model ->
 *     val result = model.generateText("What is Rust?")
 *     println(result.text)
 * }
 * ```
 */

package aimux

import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import java.io.Closeable

/**
 * A typed view over a [Model]. Delegates every call to [raw] and (de)serializes
 * the JSON boundary with [AimuxJson].
 *
 * The wrapped [Model] is NOT closed by this wrapper — the caller owns the
 * underlying handle. If you created the [Model] solely for this wrapper, close
 * the [TypedModel] (which closes the underlying [Model]); otherwise prefer
 * constructing via the companion factories (`openai`/`anthropic`) which own the
 * handle.
 */
class TypedModel(private val raw: Model, private val ownsModel: Boolean = false) : Closeable {

    override fun close() {
        if (ownsModel) raw.close()
    }

    // ── generateText ──────────────────────────────────────────────────────

    /**
     * Generate text from a simple string prompt.
     *
     * @param prompt  Plain text prompt (encoded as a JSON string on the wire).
     * @param options Optional typed [GenerateTextOptions].
     * @return Decoded [GenerateTextResult].
     */
    fun generateText(prompt: String, options: GenerateTextOptions? = null): GenerateTextResult {
        val promptJson = AimuxJson.encodeToString(prompt)
        val optsJson = options?.let { AimuxJson.encodeToString(GenerateTextOptions.serializer(), it) }
        val resultJson = raw.generateText(promptJson, optsJson)
        return decodeResult(resultJson)
    }

    /**
     * Generate text from a multi-role message list.
     *
     * @param messages Conversation as a list of [ModelMessage]s.
     * @param options  Optional typed [GenerateTextOptions].
     * @return Decoded [GenerateTextResult].
     */
    fun generateText(messages: List<ModelMessage>, options: GenerateTextOptions? = null): GenerateTextResult {
        val promptJson = AimuxJson.encodeToString(
            kotlinx.serialization.builtins.ListSerializer(ModelMessage.serializer()),
            messages,
        )
        val optsJson = options?.let { AimuxJson.encodeToString(GenerateTextOptions.serializer(), it) }
        val resultJson = raw.generateText(promptJson, optsJson)
        return decodeResult(resultJson)
    }

    private fun decodeResult(resultJson: String): GenerateTextResult {
        // The raw layer returns {"error": "..."} on failure rather than throwing.
        // Surface the engine's failure as an exception while keeping the typed
        // happy path clean.
        val firstChar = resultJson.trimStart().firstOrNull()
        if (firstChar == '{') {
            val obj = AimuxJson.parseToJsonElement(resultJson).jsonObject
            obj["error"]?.jsonPrimitive?.contentOrNull?.let { msg ->
                throw AimuxException(msg)
            }
        }
        return AimuxJson.decodeFromString(GenerateTextResult.serializer(), resultJson)
    }

    // ── streamText ───────────────────────────────────────────────────────

    /**
     * Stream text, delivering each chunk as a typed [StreamPart].
     *
     * Blocks the calling thread until the stream completes.
     *
     * @param prompt   Plain text prompt.
     * @param options  Optional typed [GenerateTextOptions].
     * @param onPart   Called for each decoded [StreamPart].
     * @param onDone   Called when the stream completes normally.
     * @param onError  Called (with a message) on a stream error.
     */
    fun streamText(
        prompt: String,
        options: GenerateTextOptions? = null,
        onPart: (StreamPart) -> Unit,
        onDone: () -> Unit,
        onError: (String) -> Unit,
    ) {
        streamTextParts(
            promptJson = AimuxJson.encodeToString(prompt),
            options = options,
            onPart = onPart,
            onDone = onDone,
            onError = onError,
        )
    }

    /**
     * Stream text from a multi-role message list, delivering typed [StreamPart]s.
     */
    fun streamText(
        messages: List<ModelMessage>,
        options: GenerateTextOptions? = null,
        onPart: (StreamPart) -> Unit,
        onDone: () -> Unit,
        onError: (String) -> Unit,
    ) {
        streamTextParts(
            promptJson = AimuxJson.encodeToString(
                kotlinx.serialization.builtins.ListSerializer(ModelMessage.serializer()),
                messages,
            ),
            options = options,
            onPart = onPart,
            onDone = onDone,
            onError = onError,
        )
    }

    private fun streamTextParts(
        promptJson: String,
        options: GenerateTextOptions?,
        onPart: (StreamPart) -> Unit,
        onDone: () -> Unit,
        onError: (String) -> Unit,
    ) {
        val optsJson = options?.let { AimuxJson.encodeToString(GenerateTextOptions.serializer(), it) }
        raw.streamText(
            promptJson = promptJson,
            optsJson = optsJson,
            onPart = { partJson ->
                val part = runCatching {
                    AimuxJson.decodeFromString(StreamPartSerializer, partJson)
                }.getOrElse { StreamPart.Unknown("<parse-error>", JsonPrimitive(partJson)) }
                onPart(part)
            },
            onDone = onDone,
            onError = onError,
        )
    }

    /**
     * Stream text as a [Sequence] of typed [StreamPart]s.
     */
    fun streamTextSequence(
        prompt: String,
        options: GenerateTextOptions? = null,
    ): Sequence<StreamPart> = streamTextSequenceParts(
        AimuxJson.encodeToString(prompt), options,
    )

    /**
     * Stream text from a multi-role message list as a [Sequence] of typed [StreamPart]s.
     */
    fun streamTextSequence(
        messages: List<ModelMessage>,
        options: GenerateTextOptions? = null,
    ): Sequence<StreamPart> = streamTextSequenceParts(
        AimuxJson.encodeToString(
            kotlinx.serialization.builtins.ListSerializer(ModelMessage.serializer()),
            messages,
        ),
        options,
    )

    private fun streamTextSequenceParts(
        promptJson: String,
        options: GenerateTextOptions?,
    ): Sequence<StreamPart> = sequence {
        val parts = java.util.concurrent.LinkedBlockingQueue<StreamPart?>()
        var error: String? = null

        streamTextParts(
            promptJson = promptJson,
            options = options,
            onPart = { parts.put(it) },
            onDone = { parts.put(null) },
            onError = { error = it; parts.put(null) },
        )

        while (true) {
            val part = parts.take() ?: break
            yield(part)
        }
        error?.let { throw AimuxException(it) }
    }

    // ── Companion: provider constructors that own the handle ─────────────

    companion object {
        /** Wrap an existing [Model]; the caller retains ownership of the handle. */
        fun of(model: Model): TypedModel = TypedModel(model, ownsModel = false)

        /** Create an OpenAI-backed [TypedModel] that owns its [Model]. */
        fun openai(apiKey: String, modelId: String): TypedModel =
            TypedModel(Model.openai(apiKey, modelId), ownsModel = true)

        /** Create an Anthropic-backed [TypedModel] that owns its [Model]. */
        fun anthropic(apiKey: String, modelId: String): TypedModel =
            TypedModel(Model.anthropic(apiKey, modelId), ownsModel = true)

        /** Create an OpenAI-backed [TypedModel] with a custom base URL; owns its [Model]. */
        fun openai(apiKey: String, modelId: String, baseUrl: String): TypedModel =
            TypedModel(Model.openai(apiKey, modelId, baseUrl), ownsModel = true)

        /** Create an Anthropic-backed [TypedModel] with a custom base URL; owns its [Model]. */
        fun anthropic(apiKey: String, modelId: String, baseUrl: String): TypedModel =
            TypedModel(Model.anthropic(apiKey, modelId, baseUrl), ownsModel = true)
    }
}

/** Raised when the engine returns an `{"error": "..."}` result or stream error. */
class AimuxException(message: String) : RuntimeException(message)

/** Returns the string content of a [JsonPrimitive], or null if not a string primitive. */
private val JsonPrimitive.contentOrNull: String?
    get() = if (isString) content else null
