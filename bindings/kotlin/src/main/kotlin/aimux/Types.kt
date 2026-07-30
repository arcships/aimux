/**
 * aimux — typed data classes mirroring the ts-rs output in `aimux-core/bindings`
 * (the generated TypeScript types).
 *
 * Field names use camelCase in Kotlin and are mapped to the wire format's
 * snake_case via [kotlinx.serialization.SerialName]. The raw JSON boundary is
 * handled by [TypedModel] — callers of this layer never parse JSON by hand.
 *
 * These types intentionally lenient on decode (unknown keys ignored, every
 * field has a default) so that future provider/engine additions do not break
 * existing clients. The serialization config lives in [AimuxJson].
 */

package aimux

import kotlinx.serialization.KSerializer
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.SerializationException
import kotlinx.serialization.descriptors.SerialDescriptor
import kotlinx.serialization.descriptors.buildClassSerialDescriptor
import kotlinx.serialization.encoding.Decoder
import kotlinx.serialization.encoding.Encoder
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonDecoder
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonEncoder
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

// ─────────────────────────────────────────────────────────────────────────────
// Shared Json instance.
//
//  - ignoreUnknownKeys  : tolerate forward-compatible fields from the engine.
//  - explicitNulls=false: omit null fields when encoding (so GenerateTextOptions
//                         only carries fields the caller actually set, matching
//                         the engine's optional-everything schema).
//  - encodeDefaults=false: do not encode default values (keeps payloads small).
// ─────────────────────────────────────────────────────────────────────────────

val AimuxJson: Json = Json {
    ignoreUnknownKeys = true
    explicitNulls = false
    encodeDefaults = false
}

// ─────────────────────────────────────────────────────────────────────────────
// Enums (simple string enums on the wire).
// ─────────────────────────────────────────────────────────────────────────────

@Serializable
enum class Role {
    @SerialName("system") SYSTEM,
    @SerialName("user") USER,
    @SerialName("assistant") ASSISTANT,
    @SerialName("tool") TOOL,
}

@Serializable
enum class FinishReasonUnified {
    @SerialName("stop") STOP,
    @SerialName("length") LENGTH,
    @SerialName("content-filter") CONTENT_FILTER,
    @SerialName("tool-calls") TOOL_CALLS,
    @SerialName("error") ERROR,
    @SerialName("other") OTHER,
}

@Serializable
enum class ReasoningEffort {
    @SerialName("provider-default") PROVIDER_DEFAULT,
    @SerialName("none") NONE,
    @SerialName("minimal") MINIMAL,
    @SerialName("low") LOW,
    @SerialName("medium") MEDIUM,
    @SerialName("high") HIGH,
    @SerialName("xhigh") XHIGH,
}

// ─────────────────────────────────────────────────────────────────────────────
// Core nested types.
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Token usage detail (with cache breakdown). Every field is nullable because
 * providers do not always populate all breakdowns.
 */
@Serializable
data class TokenUsage(
    val total: Long? = null,
    @SerialName("no_cache") val noCache: Long? = null,
    @SerialName("cache_read") val cacheRead: Long? = null,
    @SerialName("cache_write") val cacheWrite: Long? = null,
    val text: Long? = null,
    val reasoning: Long? = null,
) {
    companion object {
        fun of(total: Long?): TokenUsage = TokenUsage(total = total)
    }
}

/**
 * Token usage statistics.
 *
 * Mirrors `Usage.ts`: `{ input_tokens: TokenUsage, output_tokens: TokenUsage,
 * raw?: JsonValue | null }`.
 */
@Serializable
data class Usage(
    @SerialName("input_tokens") val inputTokens: TokenUsage = TokenUsage(),
    @SerialName("output_tokens") val outputTokens: TokenUsage = TokenUsage(),
    val raw: JsonElement? = null,
) {
    companion object {
        /** Convenience for tests/inspection. */
        fun of(input: Long?, output: Long?): Usage =
            Usage(inputTokens = TokenUsage.of(input), outputTokens = TokenUsage.of(output))
    }
}

/**
 * Unified finish reason.
 *
 * Mirrors `FinishReason.ts`: `{ unified: FinishReasonUnified, raw: string | null }`.
 */
@Serializable
data class FinishReason(
    val unified: FinishReasonUnified = FinishReasonUnified.OTHER,
    val raw: String? = null,
)

/**
 * Metadata about the API response.
 *
 * Mirrors `ResponseMetadata.ts`: `{ id: string | null, timestamp: string | null,
 * model_id: string | null }`.
 */
@Serializable
data class ResponseMetadata(
    val id: String? = null,
    val timestamp: String? = null,
    @SerialName("model_id") val modelId: String? = null,
)

/**
 * A tool call requested by the model.
 *
 * Mirrors `ToolCall.ts`: `{ tool_call_id, tool_name, input: JsonValue,
 * provider_executed?: bool | null, dynamic?: bool | null }`.
 *
 * `input` is a [JsonElement] because it is usually an arbitrary JSON object
 * (the tool arguments) whose shape is tool-specific.
 */
@Serializable
data class ToolCall(
    @SerialName("tool_call_id") val toolCallId: String,
    @SerialName("tool_name") val toolName: String,
    val input: JsonElement = JsonObject(emptyMap()),
    @SerialName("provider_executed") val providerExecuted: Boolean? = null,
    @SerialName("dynamic") val dynamic: Boolean? = null,
)

// ─────────────────────────────────────────────────────────────────────────────
// Tools (input side of GenerateTextOptions).
//
// `Tool` is an internally-tagged union on `type` (`"function" | "provider"`),
// which matches kotlinx.serialization's default sealed-class polymorphism with
// the `"type"` discriminator — no custom serializer needed.
// ─────────────────────────────────────────────────────────────────────────────

/**
 * A user-defined function tool.
 *
 * Mirrors `FunctionTool.ts`. `input_schema` is a [JsonElement] (a JSON Schema).
 */
@Serializable
data class FunctionTool(
    val name: String,
    val description: String? = null,
    @SerialName("input_schema") val inputSchema: JsonElement,
    val strict: Boolean? = null,
    @SerialName("provider_options") val providerOptions: Map<String, JsonElement>? = null,
    @SerialName("input_examples") val inputExamples: List<JsonElement>? = null,
)

/**
 * A provider-defined tool (e.g. `anthropic.web_search_20250305`).
 *
 * Mirrors `ProviderTool.ts`.
 */
@Serializable
data class ProviderTool(
    val id: String,
    val name: String,
    val args: JsonElement = JsonObject(emptyMap()),
)

/**
 * A tool that can be either a function tool or a provider tool.
 *
 * Mirrors `Tool.ts`. Serialized as `{"type":"function", ...}` /
 * `{"type":"provider", ...}` (internal `"type"` discriminator).
 */
@Serializable
sealed interface Tool {
    @Serializable
    @SerialName("function")
    data class Function(
        val name: String,
        val description: String? = null,
        @SerialName("input_schema") val inputSchema: JsonElement,
        val strict: Boolean? = null,
        @SerialName("provider_options") val providerOptions: Map<String, JsonElement>? = null,
        @SerialName("input_examples") val inputExamples: List<JsonElement>? = null,
    ) : Tool {
        companion object {
            /** Convenience constructor from a [FunctionTool]. */
            fun from(tool: FunctionTool): Function = Function(
                name = tool.name,
                description = tool.description,
                inputSchema = tool.inputSchema,
                strict = tool.strict,
                providerOptions = tool.providerOptions,
                inputExamples = tool.inputExamples,
            )
        }
    }

    @Serializable
    @SerialName("provider")
    data class Provider(
        val id: String,
        val name: String,
        val args: JsonElement = JsonObject(emptyMap()),
    ) : Tool {
        companion object {
            fun from(tool: ProviderTool): Provider = Provider(tool.id, tool.name, tool.args)
        }
    }
}

/**
 * How the model should choose tools.
 *
 * Mirrors `ToolChoice.ts`: `"auto" | "none" | "required" | { type: "tool",
 * toolName: "..." }`. This is a mixed tagged/untagged shape (bare strings plus a
 * tagged object), so a custom serializer handles the two forms.
 */
@Serializable(with = ToolChoiceSerializer::class)
sealed interface ToolChoice {
    data object Auto : ToolChoice
    data object None : ToolChoice
    data object Required : ToolChoice

    data class Tool(val toolName: String) : ToolChoice

    companion object {
        val AUTO: ToolChoice = Auto
        val NONE: ToolChoice = None
        val REQUIRED: ToolChoice = Required
    }
}

object ToolChoiceSerializer : KSerializer<ToolChoice> {
    override val descriptor: SerialDescriptor = buildClassSerialDescriptor("aimux.ToolChoice")

    override fun deserialize(decoder: Decoder): ToolChoice {
        val json = decoder as? JsonDecoder
            ?: throw SerializationException("ToolChoice can only be decoded from JSON")
        return when (val el = json.decodeJsonElement()) {
            is JsonPrimitive -> when (el.content) {
                "auto" -> ToolChoice.Auto
                "none" -> ToolChoice.None
                "required" -> ToolChoice.Required
                else -> throw SerializationException("Unknown ToolChoice string: '${el.content}'")
            }
            is JsonObject -> {
                val type = el["type"]?.jsonPrimitive?.content
                when (type) {
                    "tool" -> ToolChoice.Tool(el["toolName"]?.jsonPrimitive?.content ?: "")
                    else -> throw SerializationException("Unknown ToolChoice object: $el")
                }
            }
            else -> throw SerializationException("Unexpected ToolChoice element: $el")
        }
    }

    override fun serialize(encoder: Encoder, value: ToolChoice) {
        val json = encoder as? JsonEncoder
            ?: throw SerializationException("ToolChoice can only be encoded to JSON")
        val el: JsonElement = when (value) {
            ToolChoice.Auto -> JsonPrimitive("auto")
            ToolChoice.None -> JsonPrimitive("none")
            ToolChoice.Required -> JsonPrimitive("required")
            is ToolChoice.Tool -> JsonObject(
                mapOf(
                    "type" to JsonPrimitive("tool"),
                    "toolName" to JsonPrimitive(value.toolName),
                )
            )
        }
        json.encodeJsonElement(el)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// ModelMessage (prompt side).
// ─────────────────────────────────────────────────────────────────────────────

/**
 * A single user-facing chat message.
 *
 * Mirrors `ModelMessage.ts`: `{ role: Role, content: MessageContent }` where
 * `MessageContent = string | Array<ContentPart>`. Because content is a
 * heterogeneous union (string or array of parts), it is modeled as a
 * [JsonElement]. Use [contentString] / [contentParts] for ergonomic access, or
 * the companion factories to build one.
 */
@Serializable
data class ModelMessage(
    val role: Role,
    val content: JsonElement,
) {
    /** The content as a plain string, if the message was sent with string content. */
    val contentString: String?
        get() = (content as? JsonPrimitive)?.let { if (it.isString) it.content else null }

    companion object {
        /** Build a message with plain string content (the common case). */
        fun text(role: Role, text: String): ModelMessage =
            ModelMessage(role, JsonPrimitive(text))

        /** Build a message from a pre-built content [JsonElement] (e.g. a part array). */
        fun of(role: Role, content: JsonElement): ModelMessage = ModelMessage(role, content)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// GenerateTextOptions (input).
// ─────────────────────────────────────────────────────────────────────────────

/**
 * User-facing options for `generate_text` / `stream_text`.
 *
 * Mirrors `GenerateTextOptions.ts`. Every field is nullable with a `null`
 * default; combined with `explicitNulls=false`, only the fields the caller sets
 * are serialized onto the wire.
 */
@Serializable
data class GenerateTextOptions(
    @SerialName("max_output_tokens") val maxOutputTokens: Long? = null,
    val temperature: Double? = null,
    @SerialName("stop_sequences") val stopSequences: List<String>? = null,
    @SerialName("top_p") val topP: Double? = null,
    @SerialName("top_k") val topK: Long? = null,
    @SerialName("presence_penalty") val presencePenalty: Double? = null,
    @SerialName("frequency_penalty") val frequencyPenalty: Double? = null,
    /** Response format. Untyped ([JsonElement]) — use a `ResponseFormat` JSON object if needed. */
    @SerialName("response_format") val responseFormat: JsonElement? = null,
    val seed: Long? = null,
    val tools: List<Tool>? = null,
    @SerialName("tool_choice") val toolChoice: ToolChoice? = null,
    val headers: Map<String, String>? = null,
    @SerialName("provider_options") val providerOptions: Map<String, JsonElement>? = null,
    val reasoning: ReasoningEffort? = null,
    val instructions: String? = null,
)

// ─────────────────────────────────────────────────────────────────────────────
// GenerateResult / GenerateContent (the `raw` field of GenerateTextResult).
//
// `GenerateContent` is an externally-tagged enum
// (`{"Text": {...}}`, `{"ToolCall": {...}}`, ...). To keep decode robust and
// forward-compatible, `content` is exposed as a list of [JsonElement]s; each
// element is the raw tagged object so callers can inspect the variant tag.
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Result of `LanguageModel::do_generate` (non-streaming) — the raw provider
 * result surfaced via `GenerateTextResult.raw`.
 *
 * Mirrors `GenerateResult.ts`. `content` holds the raw externally-tagged
 * content items (e.g. `{"Text": {"text": "..."}}`, `{"ToolCall": {...}}`).
 */
@Serializable
data class GenerateResult(
    val content: List<JsonElement> = emptyList(),
    @SerialName("finish_reason") val finishReason: FinishReason = FinishReason(),
    val usage: Usage = Usage(),
    val warnings: List<JsonElement> = emptyList(),
    @SerialName("provider_metadata") val providerMetadata: JsonElement? = null,
    val response: ResponseMetadata = ResponseMetadata(),
    @SerialName("request_body") val requestBody: JsonElement? = null,
    @SerialName("response_headers") val responseHeaders: Map<String, String>? = null,
) {
    /** Names of the variant tags present in [content] (e.g. "Text", "ToolCall"). */
    val contentVariantTags: List<String>
        get() = content.mapNotNull { (it as? JsonObject)?.keys?.firstOrNull() }

    /** `true` if any content item carries the given externally-tagged variant. */
    fun hasContentVariant(tag: String): Boolean =
        content.any { (it as? JsonObject)?.containsKey(tag) == true }
}

/**
 * Result of `generate_text` (user-facing).
 *
 * Mirrors `GenerateTextResult.ts`.
 */
@Serializable
data class GenerateTextResult(
    val text: String = "",
    @SerialName("tool_calls") val toolCalls: List<ToolCall> = emptyList(),
    @SerialName("finish_reason") val finishReason: FinishReason = FinishReason(),
    val usage: Usage = Usage(),
    val warnings: List<JsonElement> = emptyList(),
    val raw: GenerateResult = GenerateResult(),
)

// ─────────────────────────────────────────────────────────────────────────────
// StreamPart (the streaming chunk type).
//
// `StreamPart` is an externally-tagged enum
// (`{"TextDelta": {...}}`, `{"ToolCall": {...}}`, ...). It is modeled as a
// sealed class with a custom deserializer that dispatches on the single tag
// key; unrecognized variants fall back to [StreamPart.Unknown] for forward
// compatibility.
// ─────────────────────────────────────────────────────────────────────────────

@Serializable(with = StreamPartSerializer::class)
sealed interface StreamPart {

    @Serializable
    data class TextStart(
        val id: String = "",
        @SerialName("provider_metadata") val providerMetadata: JsonElement? = null,
    ) : StreamPart

    @Serializable
    data class TextDelta(
        val id: String = "",
        val delta: String = "",
        @SerialName("provider_metadata") val providerMetadata: JsonElement? = null,
    ) : StreamPart

    @Serializable
    data class TextEnd(
        val id: String = "",
        @SerialName("provider_metadata") val providerMetadata: JsonElement? = null,
    ) : StreamPart

    @Serializable
    data class StreamStart(val warnings: List<JsonElement> = emptyList()) : StreamPart

    @Serializable
    data class Finish(
        @SerialName("finish_reason") val finishReason: FinishReason = FinishReason(),
        val usage: Usage = Usage(),
        @SerialName("provider_metadata") val providerMetadata: JsonElement? = null,
    ) : StreamPart

    @Serializable
    data class ToolInputStart(
        val id: String = "",
        @SerialName("tool_name") val toolName: String = "",
        @SerialName("provider_executed") val providerExecuted: Boolean? = null,
        @SerialName("dynamic") val dynamic: Boolean? = null,
        val title: String? = null,
        @SerialName("provider_metadata") val providerMetadata: JsonElement? = null,
    ) : StreamPart

    @Serializable
    data class ToolInputDelta(
        val id: String = "",
        val delta: String = "",
        @SerialName("provider_metadata") val providerMetadata: JsonElement? = null,
    ) : StreamPart

    @Serializable
    data class ToolInputEnd(
        val id: String = "",
        @SerialName("provider_metadata") val providerMetadata: JsonElement? = null,
    ) : StreamPart

    @Serializable
    data class ToolCall(
        @SerialName("tool_call_id") val toolCallId: String = "",
        @SerialName("tool_name") val toolName: String = "",
        val input: JsonElement = JsonObject(emptyMap()),
        @SerialName("provider_executed") val providerExecuted: Boolean? = null,
        @SerialName("dynamic") val dynamic: Boolean? = null,
        @SerialName("provider_metadata") val providerMetadata: JsonElement? = null,
    ) : StreamPart

    @Serializable
    data class ToolResult(
        @SerialName("tool_call_id") val toolCallId: String = "",
        @SerialName("tool_name") val toolName: String = "",
        val result: JsonElement = JsonObject(emptyMap()),
        @SerialName("is_error") val isError: Boolean? = null,
        @SerialName("preliminary") val preliminary: Boolean? = null,
        @SerialName("dynamic") val dynamic: Boolean? = null,
        @SerialName("provider_metadata") val providerMetadata: JsonElement? = null,
    ) : StreamPart

    // ── P2: file ──
    /** A file generated by the model (e.g. an image or document). */
    @Serializable
    data class File(
        val data: JsonElement = JsonObject(emptyMap()),
        @SerialName("media_type") val mediaType: String = "",
        @SerialName("provider_metadata") val providerMetadata: JsonElement? = null,
    ) : StreamPart

    @Serializable
    data class ReasoningStart(
        val id: String = "",
        @SerialName("provider_metadata") val providerMetadata: JsonElement? = null,
    ) : StreamPart

    @Serializable
    data class ReasoningDelta(
        val id: String = "",
        val delta: String = "",
        @SerialName("provider_metadata") val providerMetadata: JsonElement? = null,
    ) : StreamPart

    @Serializable
    data class ReasoningEnd(
        val id: String = "",
        @SerialName("provider_metadata") val providerMetadata: JsonElement? = null,
    ) : StreamPart

    @Serializable
    data class ResponseMetadata(
        val id: String? = null,
        val timestamp: String? = null,
        @SerialName("model_id") val modelId: String? = null,
    ) : StreamPart

    @Serializable
    data class Source(
        val id: String = "",
        @SerialName("source_type") val sourceType: String = "",
        val url: String? = null,
        val title: String? = null,
        @SerialName("provider_metadata") val providerMetadata: JsonElement? = null,
    ) : StreamPart

    @Serializable
    data class Raw(
        @SerialName("raw_value") val rawValue: JsonElement = JsonObject(emptyMap()),
    ) : StreamPart

    @Serializable
    data class Error(val error: JsonElement = JsonObject(emptyMap())) : StreamPart

    /** Fallback for variants introduced after this wrapper was written. */
    data class Unknown(val tag: String, val data: JsonElement) : StreamPart
}

/** The externally-tagged variant name for this [StreamPart] (e.g. "TextDelta", "ToolCall"). */
val StreamPart.variantTag: String
    get() = when (this) {
        is StreamPart.TextStart -> "TextStart"
        is StreamPart.TextDelta -> "TextDelta"
        is StreamPart.TextEnd -> "TextEnd"
        is StreamPart.StreamStart -> "StreamStart"
        is StreamPart.Finish -> "Finish"
        is StreamPart.ToolInputStart -> "ToolInputStart"
        is StreamPart.ToolInputDelta -> "ToolInputDelta"
        is StreamPart.ToolInputEnd -> "ToolInputEnd"
        is StreamPart.ToolCall -> "ToolCall"
        is StreamPart.ToolResult -> "ToolResult"
        is StreamPart.File -> "File"
        is StreamPart.ReasoningStart -> "ReasoningStart"
        is StreamPart.ReasoningDelta -> "ReasoningDelta"
        is StreamPart.ReasoningEnd -> "ReasoningEnd"
        is StreamPart.ResponseMetadata -> "ResponseMetadata"
        is StreamPart.Source -> "Source"
        is StreamPart.Raw -> "Raw"
        is StreamPart.Error -> "Error"
        is StreamPart.Unknown -> tag
    }

/**
 * Custom (de)serializer for [StreamPart].
 *
 * The wire format is externally tagged: each part is a single-key JSON object
 * `{"<VariantName>": { ...inner... }}`. This serializer reads the tag, then
 * delegates to the matching variant's generated serializer. Unknown tags become
 * [StreamPart.Unknown].
 */
object StreamPartSerializer : KSerializer<StreamPart> {
    override val descriptor: SerialDescriptor =
        buildClassSerialDescriptor("aimux.StreamPart")

    override fun deserialize(decoder: Decoder): StreamPart {
        val json = decoder as? JsonDecoder
            ?: throw SerializationException("StreamPart can only be decoded from JSON")
        val element = json.decodeJsonElement()
        val obj = element.jsonObject
        require(obj.size == 1) {
            "StreamPart must be a single-key externally-tagged object, got: $element"
        }
        val (tag, inner) = obj.entries.single()
        val innerObj = inner as? JsonObject ?: JsonObject(emptyMap())
        val ctx = json.json
        return when (tag) {
            "TextStart" -> ctx.decodeFromJsonElement(StreamPart.TextStart.serializer(), innerObj)
            "TextDelta" -> ctx.decodeFromJsonElement(StreamPart.TextDelta.serializer(), innerObj)
            "TextEnd" -> ctx.decodeFromJsonElement(StreamPart.TextEnd.serializer(), innerObj)
            "StreamStart" -> ctx.decodeFromJsonElement(StreamPart.StreamStart.serializer(), innerObj)
            "Finish" -> ctx.decodeFromJsonElement(StreamPart.Finish.serializer(), innerObj)
            "ToolInputStart" -> ctx.decodeFromJsonElement(StreamPart.ToolInputStart.serializer(), innerObj)
            "ToolInputDelta" -> ctx.decodeFromJsonElement(StreamPart.ToolInputDelta.serializer(), innerObj)
            "ToolInputEnd" -> ctx.decodeFromJsonElement(StreamPart.ToolInputEnd.serializer(), innerObj)
            "ToolCall" -> ctx.decodeFromJsonElement(StreamPart.ToolCall.serializer(), innerObj)
            "ToolResult" -> ctx.decodeFromJsonElement(StreamPart.ToolResult.serializer(), innerObj)
            "File" -> ctx.decodeFromJsonElement(StreamPart.File.serializer(), innerObj)
            "ReasoningStart" -> ctx.decodeFromJsonElement(StreamPart.ReasoningStart.serializer(), innerObj)
            "ReasoningDelta" -> ctx.decodeFromJsonElement(StreamPart.ReasoningDelta.serializer(), innerObj)
            "ReasoningEnd" -> ctx.decodeFromJsonElement(StreamPart.ReasoningEnd.serializer(), innerObj)
            "ResponseMetadata" -> ctx.decodeFromJsonElement(StreamPart.ResponseMetadata.serializer(), innerObj)
            "Source" -> ctx.decodeFromJsonElement(StreamPart.Source.serializer(), innerObj)
            "Raw" -> ctx.decodeFromJsonElement(StreamPart.Raw.serializer(), innerObj)
            "Error" -> ctx.decodeFromJsonElement(StreamPart.Error.serializer(), innerObj)
            else -> StreamPart.Unknown(tag, inner)
        }
    }

    override fun serialize(encoder: Encoder, value: StreamPart) {
        val json = encoder as? JsonEncoder
            ?: throw SerializationException("StreamPart can only be encoded to JSON")
        val ctx = json.json
        val (tag, inner) = when (value) {
            is StreamPart.TextStart -> "TextStart" to ctx.encodeToJsonElement(StreamPart.TextStart.serializer(), value)
            is StreamPart.TextDelta -> "TextDelta" to ctx.encodeToJsonElement(StreamPart.TextDelta.serializer(), value)
            is StreamPart.TextEnd -> "TextEnd" to ctx.encodeToJsonElement(StreamPart.TextEnd.serializer(), value)
            is StreamPart.StreamStart -> "StreamStart" to ctx.encodeToJsonElement(StreamPart.StreamStart.serializer(), value)
            is StreamPart.Finish -> "Finish" to ctx.encodeToJsonElement(StreamPart.Finish.serializer(), value)
            is StreamPart.ToolInputStart -> "ToolInputStart" to ctx.encodeToJsonElement(StreamPart.ToolInputStart.serializer(), value)
            is StreamPart.ToolInputDelta -> "ToolInputDelta" to ctx.encodeToJsonElement(StreamPart.ToolInputDelta.serializer(), value)
            is StreamPart.ToolInputEnd -> "ToolInputEnd" to ctx.encodeToJsonElement(StreamPart.ToolInputEnd.serializer(), value)
            is StreamPart.ToolCall -> "ToolCall" to ctx.encodeToJsonElement(StreamPart.ToolCall.serializer(), value)
            is StreamPart.ToolResult -> "ToolResult" to ctx.encodeToJsonElement(StreamPart.ToolResult.serializer(), value)
            is StreamPart.File -> "File" to ctx.encodeToJsonElement(StreamPart.File.serializer(), value)
            is StreamPart.ReasoningStart -> "ReasoningStart" to ctx.encodeToJsonElement(StreamPart.ReasoningStart.serializer(), value)
            is StreamPart.ReasoningDelta -> "ReasoningDelta" to ctx.encodeToJsonElement(StreamPart.ReasoningDelta.serializer(), value)
            is StreamPart.ReasoningEnd -> "ReasoningEnd" to ctx.encodeToJsonElement(StreamPart.ReasoningEnd.serializer(), value)
            is StreamPart.ResponseMetadata -> "ResponseMetadata" to ctx.encodeToJsonElement(StreamPart.ResponseMetadata.serializer(), value)
            is StreamPart.Source -> "Source" to ctx.encodeToJsonElement(StreamPart.Source.serializer(), value)
            is StreamPart.Raw -> "Raw" to ctx.encodeToJsonElement(StreamPart.Raw.serializer(), value)
            is StreamPart.Error -> "Error" to ctx.encodeToJsonElement(StreamPart.Error.serializer(), value)
            is StreamPart.Unknown -> value.tag to value.data
        }
        json.encodeJsonElement(JsonObject(mapOf(tag to inner)))
    }
}
