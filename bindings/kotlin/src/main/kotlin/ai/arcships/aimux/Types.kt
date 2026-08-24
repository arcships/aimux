/**
 * aimux — typed data classes mirroring the ts-rs output in `bindings/node/src/types`
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

package ai.arcships.aimux

import kotlinx.serialization.KSerializer
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.SerializationException
import kotlinx.serialization.descriptors.SerialDescriptor
import kotlinx.serialization.descriptors.buildClassSerialDescriptor
import kotlinx.serialization.encoding.Decoder
import kotlinx.serialization.encoding.Encoder
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
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
 * provider_executed?: bool | null, dynamic?: bool | null,
 * provider_metadata?: JsonValue | null }`.
 *
 * `input` is a [JsonElement] because it is usually an arbitrary JSON object
 * (the tool arguments) whose shape is tool-specific.
 *
 * `invalid` is set by Core when the tool call stays invalid after optional
 * repair; `error` is the typed lookup, parse, schema, or repair failure.
 */
@Serializable
data class ToolCall(
    @SerialName("tool_call_id") val toolCallId: String,
    @SerialName("tool_name") val toolName: String,
    val input: JsonElement = JsonObject(emptyMap()),
    @SerialName("provider_executed") val providerExecuted: Boolean? = null,
    @SerialName("dynamic") val dynamic: Boolean? = null,
    @SerialName("thought_signature") val thoughtSignature: String? = null,
    @SerialName("provider_metadata") val providerMetadata: JsonElement? = null,
    val invalid: Boolean? = null,
    val error: JsonElement? = null,
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
// ContentPart (multi-part message content).
//
// `ContentPart` is an internally-tagged enum (`{"type": "text", "text": ...}`,
// `{"type": "image", "image": [...], ...}`). A custom serializer dispatches on
// the `"type"` key and re-injects it on encode.
// ─────────────────────────────────────────────────────────────────────────────

/**
 * A part of a multi-part message.
 *
 * Mirrors `ContentPart.ts` (internally tagged on `type`). Shared between
 * [ModelMessage] (user-facing) and the provider-facing prompt. Every field has
 * a default and unknown keys are ignored on decode (see [AimuxJson]), so future
 * part additions do not break existing clients. `provider_options` is a
 * [JsonElement] because it is an opaque `Record<string, JSONObject>`.
 */
@Serializable(with = ContentPartSerializer::class)
sealed interface ContentPart {

    @Serializable
    data class Text(
        val text: String = "",
        @SerialName("provider_options") val providerOptions: JsonElement? = null,
    ) : ContentPart

    @Serializable
    data class Image(
        val image: List<Int> = emptyList(),
        @SerialName("media_type") val mediaType: String = "",
        @SerialName("provider_options") val providerOptions: JsonElement? = null,
    ) : ContentPart

    @Serializable
    data class File(
        val data: List<Int> = emptyList(),
        @SerialName("media_type") val mediaType: String = "",
        val filename: String? = null,
        @SerialName("provider_options") val providerOptions: JsonElement? = null,
    ) : ContentPart

    @Serializable
    data class FileBase64(
        val data: String = "",
        @SerialName("media_type") val mediaType: String = "",
        val filename: String? = null,
        @SerialName("provider_options") val providerOptions: JsonElement? = null,
    ) : ContentPart

    @Serializable
    data class FileUrl(
        val url: String = "",
        @SerialName("media_type") val mediaType: String = "",
        @SerialName("provider_options") val providerOptions: JsonElement? = null,
    ) : ContentPart

    @Serializable
    data class FileReference(
        @SerialName("media_type") val mediaType: String = "",
        val reference: JsonElement = JsonObject(emptyMap()),
        val filename: String? = null,
        @SerialName("provider_options") val providerOptions: JsonElement? = null,
    ) : ContentPart

    @Serializable
    data class Reasoning(
        val text: String = "",
        val signature: String? = null,
        @SerialName("provider_options") val providerOptions: JsonElement? = null,
    ) : ContentPart

    @Serializable
    data class ToolCall(
        @SerialName("tool_call_id") val toolCallId: String = "",
        @SerialName("tool_name") val toolName: String = "",
        val input: JsonElement = JsonObject(emptyMap()),
        @SerialName("thought_signature") val thoughtSignature: String? = null,
        @SerialName("provider_options") val providerOptions: JsonElement? = null,
    ) : ContentPart

    @Serializable
    data class ToolResult(
        @SerialName("tool_call_id") val toolCallId: String = "",
        val result: JsonElement = JsonObject(emptyMap()),
        @SerialName("tool_name") val toolName: String? = null,
        @SerialName("is_error") val isError: Boolean? = null,
        @SerialName("preliminary") val preliminary: Boolean? = null,
        @SerialName("dynamic") val dynamic: Boolean? = null,
        @SerialName("provider_options") val providerOptions: JsonElement? = null,
    ) : ContentPart
}

/**
 * Custom (de)serializer for [ContentPart].
 *
 * The wire format is internally tagged: each part is a JSON object whose
 * `"type"` key selects the variant. On decode the `"type"` key is read and the
 * matching variant's generated serializer decodes the object (the `"type"` key
 * is ignored thanks to `ignoreUnknownKeys`). On encode the variant is encoded
 * and the `"type"` key is re-injected.
 */
object ContentPartSerializer : KSerializer<ContentPart> {
    override val descriptor: SerialDescriptor =
        buildClassSerialDescriptor("aimux.ContentPart")

    override fun deserialize(decoder: Decoder): ContentPart {
        val json = decoder as? JsonDecoder
            ?: throw SerializationException("ContentPart can only be decoded from JSON")
        val element = json.decodeJsonElement()
        val obj = element.jsonObject
        val type = obj["type"]?.jsonPrimitive?.content
            ?: throw SerializationException("ContentPart is missing the 'type' discriminator: $element")
        val ctx = json.json
        return when (type) {
            "text" -> ctx.decodeFromJsonElement(ContentPart.Text.serializer(), obj)
            "image" -> ctx.decodeFromJsonElement(ContentPart.Image.serializer(), obj)
            "file" -> ctx.decodeFromJsonElement(ContentPart.File.serializer(), obj)
            "file_base64" -> ctx.decodeFromJsonElement(ContentPart.FileBase64.serializer(), obj)
            "file_url" -> ctx.decodeFromJsonElement(ContentPart.FileUrl.serializer(), obj)
            "file_reference" -> ctx.decodeFromJsonElement(ContentPart.FileReference.serializer(), obj)
            "reasoning" -> ctx.decodeFromJsonElement(ContentPart.Reasoning.serializer(), obj)
            "tool_call" -> ctx.decodeFromJsonElement(ContentPart.ToolCall.serializer(), obj)
            "tool_result" -> ctx.decodeFromJsonElement(ContentPart.ToolResult.serializer(), obj)
            else -> throw SerializationException("Unknown ContentPart type: '$type'")
        }
    }

    override fun serialize(encoder: Encoder, value: ContentPart) {
        val json = encoder as? JsonEncoder
            ?: throw SerializationException("ContentPart can only be encoded to JSON")
        val ctx = json.json
        val (typeTag, inner) = when (value) {
            is ContentPart.Text -> "text" to ctx.encodeToJsonElement(ContentPart.Text.serializer(), value)
            is ContentPart.Image -> "image" to ctx.encodeToJsonElement(ContentPart.Image.serializer(), value)
            is ContentPart.File -> "file" to ctx.encodeToJsonElement(ContentPart.File.serializer(), value)
            is ContentPart.FileBase64 -> "file_base64" to ctx.encodeToJsonElement(ContentPart.FileBase64.serializer(), value)
            is ContentPart.FileUrl -> "file_url" to ctx.encodeToJsonElement(ContentPart.FileUrl.serializer(), value)
            is ContentPart.FileReference -> "file_reference" to ctx.encodeToJsonElement(ContentPart.FileReference.serializer(), value)
            is ContentPart.Reasoning -> "reasoning" to ctx.encodeToJsonElement(ContentPart.Reasoning.serializer(), value)
            is ContentPart.ToolCall -> "tool_call" to ctx.encodeToJsonElement(ContentPart.ToolCall.serializer(), value)
            is ContentPart.ToolResult -> "tool_result" to ctx.encodeToJsonElement(ContentPart.ToolResult.serializer(), value)
        }
        val merged = JsonObject(buildMap {
            put("type", JsonPrimitive(typeTag))
            putAll(inner as? JsonObject ?: JsonObject(emptyMap()))
        })
        json.encodeJsonElement(merged)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// ModelMessage (prompt side).
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Message body: either a simple string or multi-part content.
 *
 * Mirrors `MessageContent.ts`: `string | Array<ContentPart>`. Modeled as a
 * sealed union with a custom (untagged) serializer: a JSON string decodes to
 * [MessageContent.Text], a JSON array decodes to [MessageContent.Parts].
 */
@Serializable(with = MessageContentSerializer::class)
sealed interface MessageContent {

    @Serializable
    data class Text(val text: String = "") : MessageContent

    @Serializable
    data class Parts(val parts: List<ContentPart> = emptyList()) : MessageContent
}

/**
 * Custom (de)serializer for the untagged [MessageContent] union.
 */
object MessageContentSerializer : KSerializer<MessageContent> {
    override val descriptor: SerialDescriptor =
        buildClassSerialDescriptor("aimux.MessageContent")

    override fun deserialize(decoder: Decoder): MessageContent {
        val json = decoder as? JsonDecoder
            ?: throw SerializationException("MessageContent can only be decoded from JSON")
        val element = json.decodeJsonElement()
        val ctx = json.json
        return when (element) {
            is JsonPrimitive -> MessageContent.Text(element.content)
            is JsonArray -> MessageContent.Parts(
                element.map { ctx.decodeFromJsonElement(ContentPart.serializer(), it) }
            )
            else -> throw SerializationException(
                "MessageContent must be a string or array of ContentPart, got: $element"
            )
        }
    }

    override fun serialize(encoder: Encoder, value: MessageContent) {
        val json = encoder as? JsonEncoder
            ?: throw SerializationException("MessageContent can only be encoded to JSON")
        val ctx = json.json
        val element: JsonElement = when (value) {
            is MessageContent.Text -> JsonPrimitive(value.text)
            is MessageContent.Parts -> JsonArray(
                value.parts.map { ctx.encodeToJsonElement(ContentPart.serializer(), it) }
            )
        }
        json.encodeJsonElement(element)
    }
}

/**
 * A single user-facing chat message.
 *
 * Mirrors `ModelMessage.ts`: `{ role: Role, content: MessageContent }` where
 * [MessageContent] is a string-or-parts union. Use [contentString] /
 * [contentParts] for ergonomic access, or the companion factories to build one.
 */
@Serializable
data class ModelMessage(
    val role: Role,
    val content: MessageContent = MessageContent.Text(""),
) {
    /** The content as a plain string, if the message was sent with string content. */
    val contentString: String?
        get() = (content as? MessageContent.Text)?.text

    /** The content as a list of parts, if the message was sent with multi-part content. */
    val contentParts: List<ContentPart>?
        get() = (content as? MessageContent.Parts)?.parts

    companion object {
        /** Build a message with plain string content (the common case). */
        fun text(role: Role, text: String): ModelMessage =
            ModelMessage(role, MessageContent.Text(text))

        /** Build a message from a list of [ContentPart]s. */
        fun parts(role: Role, parts: List<ContentPart>): ModelMessage =
            ModelMessage(role, MessageContent.Parts(parts))

        /** Build a message from a pre-built [MessageContent]. */
        fun of(role: Role, content: MessageContent): ModelMessage = ModelMessage(role, content)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// GenerateTextOptions (input).
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Per-call timeout configuration.
 *
 * Mirrors `TimeoutConfiguration.ts`. All values are milliseconds; `null`
 * disables the corresponding limit. A `total` timeout also covers retry
 * backoff and the whole streamed response.
 */
@Serializable
data class TimeoutConfiguration(
    /** Overall timeout for the entire call (including retries and, for streaming, the whole stream), in milliseconds. */
    @SerialName("total_ms") val totalMs: Long? = null,
    /** Timeout waiting for the first stream chunk (streaming only). */
    @SerialName("first_chunk_ms") val firstChunkMs: Long? = null,
    /** Maximum idle time between consecutive stream chunks (streaming only). */
    @SerialName("chunk_ms") val chunkMs: Long? = null,
)

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
    @SerialName("top_k") val topK: Double? = null,
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
    /** Per-call JSON deep-merge overrides for the request body. Untyped ([JsonElement]). */
    @SerialName("body_overrides") val bodyOverrides: JsonElement? = null,
    /** Per-call retry count (0 = disable retries). */
    @SerialName("max_retries") val maxRetries: Long? = null,
    /** Emit raw provider stream chunks as `StreamPart.Raw` (debugging; OpenAI-compatible family only). */
    @SerialName("include_raw_chunks") val includeRawChunks: Boolean? = null,
    /** Per-call timeout configuration (overall / first chunk / inter-chunk idle, in ms). */
    @SerialName("timeout") val timeout: TimeoutConfiguration? = null,
    /** Session identifier (RFC-0024): groups consecutive calls into a session. */
    @SerialName("session_id") val sessionId: String? = null,
)

// ─────────────────────────────────────────────────────────────────────────────
// File bytes / file data (shared V4 file types).
//
// `FileBytes` and `FileData` are externally-tagged enums
// (`{"Binary": [...]}`, `{"Data": {"data": ...}}`, ...). Each is modeled with a
// custom serializer that dispatches on the single tag key.
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Either raw bytes or a base64-encoded string.
 *
 * Mirrors `FileBytes.ts`: `{"Binary": Array<number>} | {"Base64": string}`.
 * The binary payload is a list of byte ints (serde's default `Vec<u8>` encoding).
 */
@Serializable(with = FileBytesSerializer::class)
sealed interface FileBytes {

    /** Raw binary bytes (a JSON array of 0–255 ints on the wire). */
    data class Binary(val data: List<Int> = emptyList()) : FileBytes

    /** A base64-encoded string. */
    data class Base64(val data: String = "") : FileBytes
}

/**
 * Custom (de)serializer for [FileBytes].
 *
 * Externally tagged: `{"Binary": [..]}` / `{"Base64": "..."}`. The inner value
 * is the raw array/string (not an object), so it is read/written directly.
 */
object FileBytesSerializer : KSerializer<FileBytes> {
    override val descriptor: SerialDescriptor =
        buildClassSerialDescriptor("aimux.FileBytes")

    override fun deserialize(decoder: Decoder): FileBytes {
        val json = decoder as? JsonDecoder
            ?: throw SerializationException("FileBytes can only be decoded from JSON")
        val element = json.decodeJsonElement()
        val obj = element.jsonObject
        require(obj.size == 1) {
            "FileBytes must be a single-key externally-tagged object, got: $element"
        }
        val (tag, inner) = obj.entries.single()
        return when (tag) {
            "Binary" -> {
                val arr = (inner as? JsonArray) ?: JsonArray(emptyList())
                FileBytes.Binary(arr.map { it.jsonPrimitive.content.toInt() })
            }
            "Base64" -> FileBytes.Base64(inner.jsonPrimitive.content)
            else -> throw SerializationException("Unknown FileBytes tag: '$tag'")
        }
    }

    override fun serialize(encoder: Encoder, value: FileBytes) {
        val json = encoder as? JsonEncoder
            ?: throw SerializationException("FileBytes can only be encoded to JSON")
        val (tag, inner) = when (value) {
            is FileBytes.Binary -> "Binary" to JsonArray(value.data.map { JsonPrimitive(it) })
            is FileBytes.Base64 -> "Base64" to JsonPrimitive(value.data)
        }
        json.encodeJsonElement(JsonObject(mapOf(tag to inner)))
    }
}

/**
 * File data as a tagged discriminated union.
 *
 * Mirrors `FileData.ts`: `{"Data": {"data": FileBytes}} | {"Url": {"url": ...}}
 * | {"Reference": {"reference": {...}}} | {"Text": {"text": ...}}`.
 */
@Serializable(with = FileDataSerializer::class)
sealed interface FileData {

    @Serializable
    data class Data(val data: FileBytes = FileBytes.Base64("")) : FileData

    @Serializable
    data class Url(val url: String = "") : FileData

    @Serializable
    data class Reference(val reference: JsonElement = JsonObject(emptyMap())) : FileData

    @Serializable
    data class Text(val text: String = "") : FileData
}

/**
 * Custom (de)serializer for [FileData].
 *
 * Externally tagged: each variant is a single-key object whose value is the
 * variant's inner object; delegation goes to the matching variant's generated
 * serializer.
 */
object FileDataSerializer : KSerializer<FileData> {
    override val descriptor: SerialDescriptor =
        buildClassSerialDescriptor("aimux.FileData")

    override fun deserialize(decoder: Decoder): FileData {
        val json = decoder as? JsonDecoder
            ?: throw SerializationException("FileData can only be decoded from JSON")
        val element = json.decodeJsonElement()
        val obj = element.jsonObject
        require(obj.size == 1) {
            "FileData must be a single-key externally-tagged object, got: $element"
        }
        val (tag, inner) = obj.entries.single()
        val innerObj = inner as? JsonObject ?: JsonObject(emptyMap())
        val ctx = json.json
        return when (tag) {
            "Data" -> ctx.decodeFromJsonElement(FileData.Data.serializer(), innerObj)
            "Url" -> ctx.decodeFromJsonElement(FileData.Url.serializer(), innerObj)
            "Reference" -> ctx.decodeFromJsonElement(FileData.Reference.serializer(), innerObj)
            "Text" -> ctx.decodeFromJsonElement(FileData.Text.serializer(), innerObj)
            else -> throw SerializationException("Unknown FileData tag: '$tag'")
        }
    }

    override fun serialize(encoder: Encoder, value: FileData) {
        val json = encoder as? JsonEncoder
            ?: throw SerializationException("FileData can only be encoded to JSON")
        val ctx = json.json
        val (tag, inner) = when (value) {
            is FileData.Data -> "Data" to ctx.encodeToJsonElement(FileData.Data.serializer(), value)
            is FileData.Url -> "Url" to ctx.encodeToJsonElement(FileData.Url.serializer(), value)
            is FileData.Reference -> "Reference" to ctx.encodeToJsonElement(FileData.Reference.serializer(), value)
            is FileData.Text -> "Text" to ctx.encodeToJsonElement(FileData.Text.serializer(), value)
        }
        json.encodeJsonElement(JsonObject(mapOf(tag to inner)))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// GenerateContent / GenerateResult (the `raw` field of GenerateTextResult).
//
// `GenerateContent` is an externally-tagged enum
// (`{"Text": {...}}`, `{"ToolCall": {...}}`, ...). It is modeled as a sealed
// interface with a custom serializer; unrecognized variants fall back to
// [GenerateContent.Unknown] for forward compatibility (mirroring [StreamPart]).
// ─────────────────────────────────────────────────────────────────────────────

/**
 * A content item in the generation result.
 *
 * Mirrors `GenerateContent.ts` (externally tagged). `provider_metadata` is a
 * [JsonElement] (`ProviderMetadata = serde_json::Value`). Every field has a
 * default; the `File` variant has no `filename` (matching Rust).
 */
@Serializable(with = GenerateContentSerializer::class)
sealed interface GenerateContent {

    @Serializable
    data class Text(
        val text: String = "",
        @SerialName("provider_metadata") val providerMetadata: JsonElement? = null,
    ) : GenerateContent

    @Serializable
    data class ToolCall(
        @SerialName("tool_call_id") val toolCallId: String = "",
        @SerialName("tool_name") val toolName: String = "",
        val input: JsonElement = JsonObject(emptyMap()),
        @SerialName("provider_executed") val providerExecuted: Boolean? = null,
        @SerialName("dynamic") val dynamic: Boolean? = null,
        @SerialName("thought_signature") val thoughtSignature: String? = null,
        @SerialName("provider_metadata") val providerMetadata: JsonElement? = null,
    ) : GenerateContent

    @Serializable
    data class Source(
        val id: String = "",
        @SerialName("source_type") val sourceType: String = "",
        val url: String? = null,
        val title: String? = null,
        @SerialName("provider_metadata") val providerMetadata: JsonElement? = null,
    ) : GenerateContent

    @Serializable
    data class Reasoning(
        val text: String = "",
        @SerialName("provider_metadata") val providerMetadata: JsonElement? = null,
    ) : GenerateContent

    @Serializable
    data class File(
        val data: FileData = FileData.Text(""),
        @SerialName("media_type") val mediaType: String = "",
        @SerialName("provider_metadata") val providerMetadata: JsonElement? = null,
    ) : GenerateContent

    @Serializable
    data class ToolResult(
        @SerialName("tool_call_id") val toolCallId: String = "",
        @SerialName("tool_name") val toolName: String = "",
        val result: JsonElement = JsonObject(emptyMap()),
        @SerialName("is_error") val isError: Boolean? = null,
        @SerialName("preliminary") val preliminary: Boolean? = null,
        @SerialName("dynamic") val dynamic: Boolean? = null,
        @SerialName("provider_metadata") val providerMetadata: JsonElement? = null,
    ) : GenerateContent

    /** Fallback for variants introduced after this wrapper was written. */
    data class Unknown(val tag: String, val data: JsonElement) : GenerateContent
}

/** The externally-tagged variant name for this [GenerateContent] (e.g. "Text", "ToolCall"). */
val GenerateContent.variantTag: String
    get() = when (this) {
        is GenerateContent.Text -> "Text"
        is GenerateContent.ToolCall -> "ToolCall"
        is GenerateContent.Source -> "Source"
        is GenerateContent.Reasoning -> "Reasoning"
        is GenerateContent.File -> "File"
        is GenerateContent.ToolResult -> "ToolResult"
        is GenerateContent.Unknown -> tag
    }

/**
 * Custom (de)serializer for [GenerateContent].
 *
 * The wire format is externally tagged: each item is a single-key JSON object
 * `{"<VariantName>": { ...inner... }}`. This serializer reads the tag, then
 * delegates to the matching variant's generated serializer. Unknown tags become
 * [GenerateContent.Unknown].
 */
object GenerateContentSerializer : KSerializer<GenerateContent> {
    override val descriptor: SerialDescriptor =
        buildClassSerialDescriptor("aimux.GenerateContent")

    override fun deserialize(decoder: Decoder): GenerateContent {
        val json = decoder as? JsonDecoder
            ?: throw SerializationException("GenerateContent can only be decoded from JSON")
        val element = json.decodeJsonElement()
        val obj = element.jsonObject
        require(obj.size == 1) {
            "GenerateContent must be a single-key externally-tagged object, got: $element"
        }
        val (tag, inner) = obj.entries.single()
        val innerObj = inner as? JsonObject ?: JsonObject(emptyMap())
        val ctx = json.json
        return when (tag) {
            "Text" -> ctx.decodeFromJsonElement(GenerateContent.Text.serializer(), innerObj)
            "ToolCall" -> ctx.decodeFromJsonElement(GenerateContent.ToolCall.serializer(), innerObj)
            "Source" -> ctx.decodeFromJsonElement(GenerateContent.Source.serializer(), innerObj)
            "Reasoning" -> ctx.decodeFromJsonElement(GenerateContent.Reasoning.serializer(), innerObj)
            "File" -> ctx.decodeFromJsonElement(GenerateContent.File.serializer(), innerObj)
            "ToolResult" -> ctx.decodeFromJsonElement(GenerateContent.ToolResult.serializer(), innerObj)
            else -> GenerateContent.Unknown(tag, inner)
        }
    }

    override fun serialize(encoder: Encoder, value: GenerateContent) {
        val json = encoder as? JsonEncoder
            ?: throw SerializationException("GenerateContent can only be encoded to JSON")
        val ctx = json.json
        val (tag, inner) = when (value) {
            is GenerateContent.Text -> "Text" to ctx.encodeToJsonElement(GenerateContent.Text.serializer(), value)
            is GenerateContent.ToolCall -> "ToolCall" to ctx.encodeToJsonElement(GenerateContent.ToolCall.serializer(), value)
            is GenerateContent.Source -> "Source" to ctx.encodeToJsonElement(GenerateContent.Source.serializer(), value)
            is GenerateContent.Reasoning -> "Reasoning" to ctx.encodeToJsonElement(GenerateContent.Reasoning.serializer(), value)
            is GenerateContent.File -> "File" to ctx.encodeToJsonElement(GenerateContent.File.serializer(), value)
            is GenerateContent.ToolResult -> "ToolResult" to ctx.encodeToJsonElement(GenerateContent.ToolResult.serializer(), value)
            is GenerateContent.Unknown -> value.tag to value.data
        }
        json.encodeJsonElement(JsonObject(mapOf(tag to inner)))
    }
}

/**
 * Result of `LanguageModel::do_generate` (non-streaming) — the raw provider
 * result surfaced via `GenerateTextResult.raw`.
 *
 * Mirrors `GenerateResult.ts`. [content] holds typed [GenerateContent] items.
 */
@Serializable
data class GenerateResult(
    val content: List<GenerateContent> = emptyList(),
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
        get() = content.map { it.variantTag }

    /** `true` if any content item carries the given externally-tagged variant. */
    fun hasContentVariant(tag: String): Boolean =
        content.any { it.variantTag == tag }
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
    // M7: top-level aggregation fields
    val reasoning: List<JsonElement> = emptyList(),
    @SerialName("reasoning_text") val reasoningText: String = "",
    val sources: List<JsonElement> = emptyList(),
    val files: List<JsonElement> = emptyList(),
    @SerialName("response_messages") val responseMessages: List<ModelMessage> = emptyList(),
    // M12: raw provider-specific finish reason string.
    @SerialName("raw_finish_reason") val rawFinishReason: String? = null,
    // Provider-specific metadata (e.g. Anthropic cache info). Mirrored from
    // raw.provider_metadata for top-level convenience. Weak type (JsonElement?).
    @SerialName("provider_metadata") val providerMetadata: JsonElement? = null,
    // Response metadata (id, timestamp, model_id). Mirrored from raw.response.
    val response: ResponseMetadata = ResponseMetadata(),
    // Total token usage across all steps. In single-step mode (aimux's
    // default), equals usage. Provided for AI SDK parity.
    @SerialName("total_usage") val totalUsage: Usage = Usage(),
)

/**
 * Result of `generate_object` (user-facing, M12). The parsed JSON object plus
 * convenience fields from the underlying `generate_text` call.
 *
 * Mirrors `GenerateObjectResult.ts`. `object` is a [JsonElement] (arbitrary
 * JSON value, weak type).
 */
@Serializable
data class GenerateObjectResult(
    // `object` is an arbitrary JSON value — weak type (JsonElement).
    val `object`: JsonElement,
    @SerialName("finish_reason") val finishReason: FinishReason = FinishReason(),
    @SerialName("raw_finish_reason") val rawFinishReason: String? = null,
    val usage: Usage = Usage(),
    val warnings: List<JsonElement> = emptyList(),
    // Concatenated reasoning text (if the model produced reasoning/thinking).
    val reasoning: String? = null,
    // Provider-specific metadata (e.g. Anthropic cache info). Weak type.
    @SerialName("provider_metadata") val providerMetadata: JsonElement? = null,
    // Response metadata (id, timestamp, model_id).
    val response: ResponseMetadata = ResponseMetadata(),
    val raw: GenerateTextResult = GenerateTextResult(),
)

/**
 * Aggregated result of `stream_text().consume()` (M11). Mirrors
 * `GenerateTextResult`'s user-facing fields (without `raw`, since streaming
 * has no `GenerateResult` equivalent).
 *
 * Mirrors `StreamTextResultAggregated.ts`. reasoning/sources/files use weak
 * types (JsonElement) — same strategy as [GenerateTextResult].
 */
@Serializable
data class StreamTextResultAggregated(
    val text: String = "",
    // reasoning/sources/files use weak types (JsonElement).
    val reasoning: List<JsonElement> = emptyList(),
    @SerialName("reasoning_text") val reasoningText: String = "",
    @SerialName("tool_calls") val toolCalls: List<ToolCall> = emptyList(),
    val sources: List<JsonElement> = emptyList(),
    val files: List<JsonElement> = emptyList(),
    @SerialName("finish_reason") val finishReason: FinishReason = FinishReason(),
    @SerialName("raw_finish_reason") val rawFinishReason: String? = null,
    val usage: Usage = Usage(),
    // Total token usage across all steps. In single-step mode (aimux's
    // default), equals usage. Provided for AI SDK parity.
    @SerialName("total_usage") val totalUsage: Usage = Usage(),
    val warnings: List<JsonElement> = emptyList(),
    // Provider-specific metadata from the Finish chunk. Weak type.
    @SerialName("provider_metadata") val providerMetadata: JsonElement? = null,
    // Response metadata (id, timestamp, model_id) if emitted by the stream.
    val response: ResponseMetadata? = null,
    @SerialName("response_messages") val responseMessages: List<ModelMessage> = emptyList(),
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

    /**
     * `invalid` is set by Core when the tool call stays invalid after optional
     * repair; `error` is the typed lookup, parse, schema, or repair failure.
     */
    @Serializable
    data class ToolCall(
        @SerialName("tool_call_id") val toolCallId: String = "",
        @SerialName("tool_name") val toolName: String = "",
        val input: JsonElement = JsonObject(emptyMap()),
        @SerialName("provider_executed") val providerExecuted: Boolean? = null,
        @SerialName("dynamic") val dynamic: Boolean? = null,
        @SerialName("thought_signature") val thoughtSignature: String? = null,
        @SerialName("provider_metadata") val providerMetadata: JsonElement? = null,
        val invalid: Boolean? = null,
        val error: JsonElement? = null,
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

// ─────────────────────────────────────────────────────────────────────────────
// OpenAI Chat Completions output (RFC-0026).
//
// Mirrors `aimux-core::openai_output`. Field names are camelCase in Kotlin and
// mapped to the wire's snake_case via [SerialName]. The `type` field is JSON
// `"type"` (Rust `#[serde(rename = "type")]`) → `toolType`. Arbitrary-JSON
// fields (`logprobs`, `annotations`) are [JsonElement].
// ─────────────────────────────────────────────────────────────────────────────

/** A complete Chat Completion response (non-streaming). Mirrors OpenAI `chat.completion`. */
@Serializable
data class ChatCompletion(
    val id: String = "",
    val `object`: String = "chat.completion",
    val created: Long = 0,
    val model: String = "",
    val choices: List<ChatCompletionChoice> = emptyList(),
    val usage: ChatCompletionUsage = ChatCompletionUsage(),
    @SerialName("system_fingerprint") val systemFingerprint: String? = null,
)

@Serializable
data class ChatCompletionChoice(
    val index: Int = 0,
    val message: ChatCompletionMessage = ChatCompletionMessage(),
    @SerialName("finish_reason") val finishReason: String? = null,
    val logprobs: JsonElement? = null,
)

@Serializable
data class ChatCompletionMessage(
    val role: String = "assistant",
    val content: String? = null,
    @SerialName("reasoning_content") val reasoningContent: String? = null,
    @SerialName("tool_calls") val toolCalls: List<ChatCompletionToolCall>? = null,
    val annotations: List<JsonElement>? = null,
)

/**
 * A tool call in a [ChatCompletionMessage].
 *
 * Wire: `{"id","type":"function","function":{"name","arguments"}}`. The `type`
 * field is JSON `"type"` (Rust `#[serde(rename = "type")]`).
 */
@Serializable
data class ChatCompletionToolCall(
    val id: String = "",
    @SerialName("type") val toolType: String = "function",
    val function: ChatCompletionFunction = ChatCompletionFunction(),
)

@Serializable
data class ChatCompletionFunction(
    val name: String = "",
    val arguments: String = "",
)

/** A single Chat Completion chunk (streaming). Mirrors OpenAI `chat.completion.chunk`. */
@Serializable
data class ChatCompletionChunk(
    val id: String = "",
    val `object`: String = "chat.completion.chunk",
    val created: Long = 0,
    val model: String = "",
    val choices: List<ChatCompletionChunkChoice> = emptyList(),
    val usage: ChatCompletionUsage? = null,
)

@Serializable
data class ChatCompletionChunkChoice(
    val index: Int = 0,
    val delta: ChatCompletionDelta = ChatCompletionDelta(),
    @SerialName("finish_reason") val finishReason: String? = null,
    val logprobs: JsonElement? = null,
)

@Serializable
data class ChatCompletionDelta(
    val role: String? = null,
    val content: String? = null,
    @SerialName("reasoning_content") val reasoningContent: String? = null,
    @SerialName("tool_calls") val toolCalls: List<ChatCompletionChunkToolCall>? = null,
)

/**
 * A tool call delta in a [ChatCompletionChunk].
 *
 * Wire: `{"index","id"?,"type":"function"?,"function":{"name"?,"arguments"?}}`.
 * The `type` field is JSON `"type"` (Rust `#[serde(rename = "type")]`).
 */
@Serializable
data class ChatCompletionChunkToolCall(
    val index: Int = 0,
    val id: String? = null,
    @SerialName("type") val toolType: String? = null,
    val function: ChatCompletionChunkFunction = ChatCompletionChunkFunction(),
)

@Serializable
data class ChatCompletionChunkFunction(
    val name: String? = null,
    val arguments: String? = null,
)

/** Token usage statistics (shared by streaming and non-streaming). */
@Serializable
data class ChatCompletionUsage(
    @SerialName("prompt_tokens") val promptTokens: Int = 0,
    @SerialName("completion_tokens") val completionTokens: Int = 0,
    @SerialName("total_tokens") val totalTokens: Int = 0,
    @SerialName("prompt_tokens_details") val promptTokensDetails: PromptTokensDetails? = null,
    @SerialName("completion_tokens_details") val completionTokensDetails: CompletionTokensDetails? = null,
)

@Serializable
data class PromptTokensDetails(
    @SerialName("cached_tokens") val cachedTokens: Int = 0,
    @SerialName("cache_write_tokens") val cacheWriteTokens: Int? = null,
)

@Serializable
data class CompletionTokensDetails(
    @SerialName("reasoning_tokens") val reasoningTokens: Int? = null,
)
