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

package ai.arcships.aimux

import kotlinx.serialization.encodeToString
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
        // Engine failures throw AimuxException.fromC in the raw layer.
        // Decode failures are local InvalidArgumentError.
        return try {
            AimuxJson.decodeFromString(GenerateTextResult.serializer(), resultJson)
        } catch (e: Exception) {
            throw InvalidArgumentError(
                "failed to decode GenerateTextResult: ${e.message ?: e::class.simpleName}",
                cause = e,
            )
        }
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
                try {
                    onPart(AimuxJson.decodeFromString(StreamPartSerializer, partJson))
                } catch (error: Exception) {
                    onError(
                        "failed to decode StreamPart: ${error.message ?: error::class.simpleName}",
                    )
                }
            },
            onDone = onDone,
        )
        // raw.streamText throws AimuxException on native failure; onError only
        // reports local decode failures.
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
        // LinkedBlockingQueue rejects null, so end-of-stream is a sentinel object.
        val eos = Any()
        val parts = java.util.concurrent.LinkedBlockingQueue<Any>()
        var streamError: AimuxException? = null

        try {
            streamTextParts(
                promptJson = promptJson,
                options = options,
                onPart = { parts.put(it) },
                onDone = { parts.put(eos) },
                onError = { msg ->
                    streamError = OtherError(msg)
                    parts.put(eos)
                },
            )
        } catch (e: AimuxException) {
            streamError = e
            parts.put(eos)
        }

        while (true) {
            val part = parts.take()
            if (part === eos) break
            yield(part as StreamPart)
        }
        streamError?.let { throw it }
    }

    // ── OpenAI-compatible output (RFC-0026) ─────────────────────────────────

    /**
     * Generate text (non-streaming) with OpenAI Chat Completions output.
     *
     * @param prompt  Plain text prompt (encoded as a JSON string on the wire).
     * @param options Optional typed [GenerateTextOptions].
     * @return Decoded [ChatCompletion].
     */
    fun generateTextAsOpenAI(prompt: String, options: GenerateTextOptions? = null): ChatCompletion {
        val promptJson = AimuxJson.encodeToString(prompt)
        val optsJson = options?.let { AimuxJson.encodeToString(GenerateTextOptions.serializer(), it) }
        val resultJson = raw.generateTextAsOpenAI(promptJson, optsJson)
        return decodeChatCompletion(resultJson)
    }

    /**
     * Generate text from a multi-role message list with OpenAI Chat Completions
     * output.
     *
     * @param messages Conversation as a list of [ModelMessage]s.
     * @param options  Optional typed [GenerateTextOptions].
     * @return Decoded [ChatCompletion].
     */
    fun generateTextAsOpenAI(
        messages: List<ModelMessage>, options: GenerateTextOptions? = null
    ): ChatCompletion {
        val promptJson = AimuxJson.encodeToString(
            kotlinx.serialization.builtins.ListSerializer(ModelMessage.serializer()),
            messages,
        )
        val optsJson = options?.let { AimuxJson.encodeToString(GenerateTextOptions.serializer(), it) }
        val resultJson = raw.generateTextAsOpenAI(promptJson, optsJson)
        return decodeChatCompletion(resultJson)
    }

    private fun decodeChatCompletion(resultJson: String): ChatCompletion {
        return try {
            AimuxJson.decodeFromString(ChatCompletion.serializer(), resultJson)
        } catch (e: Exception) {
            throw InvalidArgumentError(
                "failed to decode ChatCompletion: ${e.message ?: e::class.simpleName}",
                cause = e,
            )
        }
    }

    /**
     * Stream text with OpenAI Chat Completions output, yielding typed
     * [ChatCompletionChunk]s. Blocks the calling thread until the stream
     * completes.
     *
     * @param prompt   Plain text prompt.
     * @param options  Optional typed [GenerateTextOptions].
     * @param onPart   Called for each decoded [ChatCompletionChunk].
     * @param onDone   Called when the stream completes normally.
     * @param onError  Called (with a message) on a stream error.
     */
    fun streamTextAsOpenAI(
        prompt: String,
        options: GenerateTextOptions? = null,
        onPart: (ChatCompletionChunk) -> Unit,
        onDone: () -> Unit,
        onError: (String) -> Unit,
    ) {
        streamTextAsOpenAIChunks(
            promptJson = AimuxJson.encodeToString(prompt),
            options = options,
            onPart = onPart,
            onDone = onDone,
            onError = onError,
        )
    }

    /**
     * Stream text from a multi-role message list with OpenAI Chat Completions
     * output, yielding typed [ChatCompletionChunk]s.
     */
    fun streamTextAsOpenAI(
        messages: List<ModelMessage>,
        options: GenerateTextOptions? = null,
        onPart: (ChatCompletionChunk) -> Unit,
        onDone: () -> Unit,
        onError: (String) -> Unit,
    ) {
        streamTextAsOpenAIChunks(
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

    private fun streamTextAsOpenAIChunks(
        promptJson: String,
        options: GenerateTextOptions?,
        onPart: (ChatCompletionChunk) -> Unit,
        onDone: () -> Unit,
        onError: (String) -> Unit,
    ) {
        val optsJson = options?.let { AimuxJson.encodeToString(GenerateTextOptions.serializer(), it) }
        raw.streamTextAsOpenAI(
            promptJson = promptJson,
            optsJson = optsJson,
            onPart = { chunkJson ->
                try {
                    onPart(AimuxJson.decodeFromString(ChatCompletionChunk.serializer(), chunkJson))
                } catch (error: Exception) {
                    onError("failed to decode ChatCompletionChunk: ${error.message ?: error::class.simpleName}")
                }
            },
            onDone = onDone,
        )
    }

    /**
     * Stream text with OpenAI Chat Completions output as a [Sequence] of typed
     * [ChatCompletionChunk]s (RFC-0026).
     */
    fun streamTextAsOpenAISequence(
        prompt: String,
        options: GenerateTextOptions? = null,
    ): Sequence<ChatCompletionChunk> = streamTextAsOpenAISequenceParts(
        AimuxJson.encodeToString(prompt), options,
    )

    /**
     * Stream text from a multi-role message list with OpenAI Chat Completions
     * output as a [Sequence] of typed [ChatCompletionChunk]s.
     */
    fun streamTextAsOpenAISequence(
        messages: List<ModelMessage>,
        options: GenerateTextOptions? = null,
    ): Sequence<ChatCompletionChunk> = streamTextAsOpenAISequenceParts(
        AimuxJson.encodeToString(
            kotlinx.serialization.builtins.ListSerializer(ModelMessage.serializer()),
            messages,
        ),
        options,
    )

    private fun streamTextAsOpenAISequenceParts(
        promptJson: String,
        options: GenerateTextOptions?,
    ): Sequence<ChatCompletionChunk> = sequence {
        // LinkedBlockingQueue rejects null, so end-of-stream is a sentinel object.
        val eos = Any()
        val parts = java.util.concurrent.LinkedBlockingQueue<Any>()
        var streamError: AimuxException? = null

        try {
            streamTextAsOpenAIChunks(
                promptJson = promptJson,
                options = options,
                onPart = { parts.put(it) },
                onDone = { parts.put(eos) },
                onError = { msg ->
                    streamError = OtherError(msg)
                    parts.put(eos)
                },
            )
        } catch (e: AimuxException) {
            streamError = e
            parts.put(eos)
        }

        while (true) {
            val part = parts.take()
            if (part === eos) break
            yield(part as ChatCompletionChunk)
        }
        streamError?.let { throw it }
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

        /** Create a registry-backed [TypedModel] (RFC-0017 phase 4) that owns its [Model]. */
        fun provider(name: String, apiKey: String? = null, modelId: String, configJson: String? = null): TypedModel =
            TypedModel(Model.provider(name, apiKey, modelId, configJson), ownsModel = true)
    }
}
