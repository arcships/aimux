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
        val resultJson = repaired(raw.generateText(promptJson, optsJson), promptJson, optsJson, options)
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
        val resultJson = repaired(raw.generateText(promptJson, optsJson), promptJson, optsJson, options)
        return decodeResult(resultJson)
    }

    /**
     * Give every invalid tool call in a result one repair attempt (RFC-0035),
     * before the JSON is decoded — the patched document is what the caller
     * sees, transcript included. A no-op unless [GenerateTextOptions.repairToolCall]
     * is set.
     */
    private fun repaired(
        resultJson: String,
        promptJson: String,
        optsJson: String?,
        options: GenerateTextOptions?,
    ): String {
        val repair = options?.repairToolCall ?: return resultJson
        return repairToolCallsInResult(resultJson, promptJson, optsJson, repair)
    }

    private fun decodeResult(resultJson: String): GenerateTextResult {
        // AiMuxError values throw AimuxException.fromC in the raw layer.
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

    // ── generateObject (M12, RFC-0016) ───────────────────────────────────

    /**
     * Generate a structured JSON object from a simple string prompt (M12).
     *
     * @param prompt  Plain text prompt.
     * @param options Optional typed [GenerateTextOptions].
     * @return Decoded [GenerateObjectResult].
     */
    fun generateObject(prompt: String, options: GenerateTextOptions? = null): GenerateObjectResult {
        val promptJson = AimuxJson.encodeToString(prompt)
        val optsJson = options?.let { AimuxJson.encodeToString(GenerateTextOptions.serializer(), it) }
        val resultJson = repaired(raw.generateObject(promptJson, optsJson), promptJson, optsJson, options)
        return decodeObjectResult(resultJson)
    }

    /**
     * Generate a structured JSON object from a multi-role message list (M12).
     *
     * @param messages Conversation as a list of [ModelMessage]s.
     * @param options  Optional typed [GenerateTextOptions].
     * @return Decoded [GenerateObjectResult].
     */
    fun generateObject(messages: List<ModelMessage>, options: GenerateTextOptions? = null): GenerateObjectResult {
        val promptJson = AimuxJson.encodeToString(
            kotlinx.serialization.builtins.ListSerializer(ModelMessage.serializer()),
            messages,
        )
        val optsJson = options?.let { AimuxJson.encodeToString(GenerateTextOptions.serializer(), it) }
        val resultJson = repaired(raw.generateObject(promptJson, optsJson), promptJson, optsJson, options)
        return decodeObjectResult(resultJson)
    }

    private fun decodeObjectResult(resultJson: String): GenerateObjectResult {
        return try {
            AimuxJson.decodeFromString(GenerateObjectResult.serializer(), resultJson)
        } catch (e: Exception) {
            throw InvalidArgumentError(
                "failed to decode GenerateObjectResult: ${e.message ?: e::class.simpleName}",
                cause = e,
            )
        }
    }

    // ── consumeStreamText (M11, RFC-0016) ────────────────────────────────

    /**
     * Consume a stream to completion from a simple string prompt and return
     * the aggregated result (M11). Synchronous (blocks until finished).
     *
     * @param prompt  Plain text prompt.
     * @param options Optional typed [GenerateTextOptions].
     * @return Decoded [StreamTextResultAggregated].
     */
    fun consumeStreamText(prompt: String, options: GenerateTextOptions? = null): StreamTextResultAggregated {
        val promptJson = AimuxJson.encodeToString(prompt)
        val optsJson = options?.let { AimuxJson.encodeToString(GenerateTextOptions.serializer(), it) }
        val resultJson = repaired(raw.consumeStreamText(promptJson, optsJson), promptJson, optsJson, options)
        return decodeAggregated(resultJson)
    }

    /**
     * Consume a stream to completion from a multi-role message list and
     * return the aggregated result (M11).
     *
     * @param messages Conversation as a list of [ModelMessage]s.
     * @param options  Optional typed [GenerateTextOptions].
     * @return Decoded [StreamTextResultAggregated].
     */
    fun consumeStreamText(messages: List<ModelMessage>, options: GenerateTextOptions? = null): StreamTextResultAggregated {
        val promptJson = AimuxJson.encodeToString(
            kotlinx.serialization.builtins.ListSerializer(ModelMessage.serializer()),
            messages,
        )
        val optsJson = options?.let { AimuxJson.encodeToString(GenerateTextOptions.serializer(), it) }
        val resultJson = repaired(raw.consumeStreamText(promptJson, optsJson), promptJson, optsJson, options)
        return decodeAggregated(resultJson)
    }

    private fun decodeAggregated(resultJson: String): StreamTextResultAggregated {
        return try {
            AimuxJson.decodeFromString(StreamTextResultAggregated.serializer(), resultJson)
        } catch (e: Exception) {
            throw InvalidArgumentError(
                "failed to decode StreamTextResultAggregated: ${e.message ?: e::class.simpleName}",
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
        // Off the callback thread: the native re-entrancy guard is thread-local
        // (see offCallbackThread), so a hook calling aimux would fail with 204.
        val repair = options?.repairToolCall?.let { offCallbackThread(raw, it) }
        raw.streamText(
            promptJson = promptJson,
            optsJson = optsJson,
            onPart = { rawPartJson ->
                // Only an invalid ToolCall part is rewritten; tool-input deltas
                // pass through untouched and immediately. A repair that fails
                // at the boundary (rather than in the user's function, which
                // core turns into a ToolCallRepair error) must not kill the
                // stream: report it and deliver the unrepaired part.
                val partJson = if (repair == null) rawPartJson else try {
                    repairToolCallStreamPart(rawPartJson, promptJson, optsJson, repair)
                } catch (error: Exception) {
                    onError("failed to repair tool call: ${error.message ?: error::class.simpleName}")
                    rawPartJson
                }
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
        return generateTextAsOpenAIParts(promptJson, optsJson, options)
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
        return generateTextAsOpenAIParts(promptJson, optsJson, options)
    }

    /**
     * A `ChatCompletion` has no `invalid` marker to repair from, so with a
     * [GenerateTextOptions.repairToolCall] the native result is generated,
     * repaired, then converted — its tool calls carry the repaired arguments.
     */
    private fun generateTextAsOpenAIParts(
        promptJson: String,
        optsJson: String?,
        options: GenerateTextOptions?,
    ): ChatCompletion {
        if (options?.repairToolCall == null) {
            return decodeChatCompletion(raw.generateTextAsOpenAI(promptJson, optsJson))
        }
        val resultJson = repaired(raw.generateText(promptJson, optsJson), promptJson, optsJson, options)
        return decodeChatCompletion(raw.generateTextResultAsOpenAI(resultJson))
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
