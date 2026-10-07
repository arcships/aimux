/**
 * aimux — typed data classes mirroring the ts-rs output in `bindings/node/src/types`
 * (the generated TypeScript types).
 *
 * The wire JSON is the AI SDK's: camelCase field names, unions tagged with a
 * `type` key (a few use `role`, `name` or `sourceType`), optional fields
 * absent rather than null. Kotlin property names equal the wire names, so no
 * [kotlinx.serialization.SerialName] is needed on fields. The raw JSON boundary
 * is handled by [TypedModel] — callers of this layer never parse JSON by hand.
 *
 * Decode is lenient (unknown keys ignored, every result field has a default)
 * so engine additions do not break existing clients. The serialization config
 * lives in [AimuxJson].
 */

package ai.arcships.aimux

import kotlinx.serialization.ExperimentalSerializationApi
import kotlinx.serialization.KSerializer
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.SerializationException
import kotlinx.serialization.Transient
import kotlinx.serialization.descriptors.SerialDescriptor
import kotlinx.serialization.descriptors.buildClassSerialDescriptor
import kotlinx.serialization.encoding.Decoder
import kotlinx.serialization.encoding.Encoder
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonClassDiscriminator
import kotlinx.serialization.json.JsonDecoder
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonEncoder
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonPrimitive

// ─────────────────────────────────────────────────────────────────────────────
// Shared Json instance.
//
//  - ignoreUnknownKeys  : tolerate forward-compatible fields from the engine.
//  - explicitNulls=false: omit null fields when encoding (optional fields are
//                         absent on the wire, never null).
//  - encodeDefaults=false: do not encode default values (keeps payloads small).
//
// Unions tagged with `type` use kotlinx's default sealed-class polymorphism
// (`classDiscriminator` stays "type").
// ─────────────────────────────────────────────────────────────────────────────

val AimuxJson: Json = Json {
    ignoreUnknownKeys = true
    explicitNulls = false
    encodeDefaults = false
}

/** Provider-specific options / metadata: `{ providerName: { key: value } }`. */
typealias ProviderOptions = Map<String, Map<String, JsonElement>>

/** Provider-specific metadata on a result (same shape as [ProviderOptions]). */
typealias ProviderMetadata = Map<String, Map<String, JsonElement>>

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

@Serializable
data class InputTokenUsage(
    val total: Long? = null,
    val noCache: Long? = null,
    val cacheRead: Long? = null,
    val cacheWrite: Long? = null,
)

@Serializable
data class OutputTokenUsage(
    val total: Long? = null,
    val text: Long? = null,
    val reasoning: Long? = null,
)

/** Token usage statistics. Mirrors `Usage.ts`. */
@Serializable
data class Usage(
    val inputTokens: InputTokenUsage = InputTokenUsage(),
    val outputTokens: OutputTokenUsage = OutputTokenUsage(),
    val raw: JsonObject? = null,
) {
    companion object {
        /** Convenience for tests/inspection. */
        fun of(input: Long?, output: Long?): Usage =
            Usage(inputTokens = InputTokenUsage(input), outputTokens = OutputTokenUsage(output))
    }
}

/** Unified finish reason. Mirrors `FinishReason.ts`: `raw` is absent when the provider sent none. */
@Serializable
data class FinishReason(
    val unified: FinishReasonUnified = FinishReasonUnified.OTHER,
    val raw: String? = null,
)

/** Mirrors `ResponseMetadata.ts`. */
@Serializable
data class ResponseMetadata(
    val id: String? = null,
    val timestamp: String? = null,
    val modelId: String? = null,
)

/** Mirrors `ResponseInfo.ts`: [ResponseMetadata] plus the HTTP headers and body. */
@Serializable
data class ResponseInfo(
    val id: String? = null,
    val timestamp: String? = null,
    val modelId: String? = null,
    val headers: Map<String, String>? = null,
    val body: JsonElement? = null,
)

@Serializable
data class RequestInfo(val body: JsonElement? = null)

/**
 * A tool call requested by the model. Mirrors `ToolCall.ts`.
 *
 * `input` is the parsed argument value (usually an object). `invalid` is set
 * by Core when the call stays invalid after optional repair; `error` is the
 * `name`-tagged `AiMuxError` JSON of the lookup, parse, schema, or repair
 * failure (`{"name":"AI_NoSuchToolError",...}`).
 */
@Serializable
data class ToolCall(
    val toolCallId: String,
    val toolName: String,
    val input: JsonElement = JsonObject(emptyMap()),
    val providerExecuted: Boolean? = null,
    val dynamic: Boolean? = null,
    val providerMetadata: ProviderMetadata? = null,
    val invalid: Boolean? = null,
    val error: JsonElement? = null,
)

// ─────────────────────────────────────────────────────────────────────────────
// Tools (input side of GenerateTextOptions).
//
// `Tool` is an internally-tagged union on `type` (`"function" | "provider"`).
// ─────────────────────────────────────────────────────────────────────────────

@Serializable
data class FunctionToolInputExample(val input: JsonObject)

/** A user-defined function tool. Mirrors `FunctionTool.ts`; `inputSchema` is a JSON Schema. */
@Serializable
data class FunctionTool(
    val name: String,
    val description: String? = null,
    val inputSchema: JsonElement,
    val strict: Boolean? = null,
    val providerOptions: ProviderOptions? = null,
    val inputExamples: List<FunctionToolInputExample>? = null,
)

/** A provider-defined tool (e.g. `anthropic.web_search_20250305`). Mirrors `ProviderTool.ts`. */
@Serializable
data class ProviderTool(
    val id: String,
    val name: String,
    val args: JsonObject = JsonObject(emptyMap()),
)

/**
 * A function tool or a provider tool. Mirrors `Tool.ts`; serialized as
 * `{"type":"function", ...}` / `{"type":"provider", ...}`.
 */
@Serializable
sealed interface Tool {
    @Serializable
    @SerialName("function")
    data class Function(
        val name: String,
        val description: String? = null,
        val inputSchema: JsonElement,
        val strict: Boolean? = null,
        val providerOptions: ProviderOptions? = null,
        val inputExamples: List<FunctionToolInputExample>? = null,
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
        val args: JsonObject = JsonObject(emptyMap()),
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
 * toolName: "..." }`. This is a mixed untagged/tagged shape (bare strings plus
 * a tagged object), so a custom serializer handles the two forms.
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
// File bytes / file data.
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Raw bytes or a base64 string. Mirrors `FileBytes.ts`: `Array<number> | string`
 * (untagged — a JSON array is binary, a JSON string is base64). Also the shape
 * of every `Array<number> | string` field (audio input, image / video file data).
 */
@Serializable(with = FileBytesSerializer::class)
sealed interface FileBytes {

    /** Raw binary bytes (a JSON array of 0–255 ints on the wire). */
    data class Binary(val data: List<Int> = emptyList()) : FileBytes

    /** A base64-encoded string. */
    data class Base64(val data: String = "") : FileBytes
}

object FileBytesSerializer : KSerializer<FileBytes> {
    override val descriptor: SerialDescriptor = buildClassSerialDescriptor("aimux.FileBytes")

    override fun deserialize(decoder: Decoder): FileBytes {
        val json = decoder as? JsonDecoder
            ?: throw SerializationException("FileBytes can only be decoded from JSON")
        return when (val el = json.decodeJsonElement()) {
            is JsonArray -> FileBytes.Binary(el.map { it.jsonPrimitive.content.toInt() })
            is JsonPrimitive -> FileBytes.Base64(el.content)
            else -> throw SerializationException("FileBytes must be a byte array or a base64 string, got: $el")
        }
    }

    override fun serialize(encoder: Encoder, value: FileBytes) {
        val json = encoder as? JsonEncoder
            ?: throw SerializationException("FileBytes can only be encoded to JSON")
        json.encodeJsonElement(
            when (value) {
                is FileBytes.Binary -> JsonArray(value.data.map { JsonPrimitive(it) })
                is FileBytes.Base64 -> JsonPrimitive(value.data)
            }
        )
    }
}

/** File data, tagged on `type`. Mirrors `FileData.ts`. */
@Serializable
sealed interface FileData {

    @Serializable
    @SerialName("data")
    data class Data(val data: FileBytes = FileBytes.Base64("")) : FileData

    @Serializable
    @SerialName("url")
    data class Url(val url: String = "", val originalUrl: String? = null) : FileData

    @Serializable
    @SerialName("reference")
    data class Reference(val reference: Map<String, String> = emptyMap()) : FileData

    @Serializable
    @SerialName("text")
    data class Text(val text: String = "") : FileData
}

/** Generated file data, tagged on `type`. Mirrors `GeneratedFileData.ts`. */
@Serializable
sealed interface GeneratedFileData {

    @Serializable
    @SerialName("data")
    data class Data(val data: FileBytes = FileBytes.Base64("")) : GeneratedFileData

    @Serializable
    @SerialName("url")
    data class Url(val url: String = "", val originalUrl: String? = null) : GeneratedFileData
}

/** A file produced by the model. Mirrors `GeneratedFile.ts`. */
@Serializable
data class GeneratedFile(
    val data: GeneratedFileData = GeneratedFileData.Data(),
    val mediaType: String = "",
    val providerMetadata: ProviderMetadata? = null,
)

// ─────────────────────────────────────────────────────────────────────────────
// Tool results inside a prompt (ContentPart `tool-result`).
// ─────────────────────────────────────────────────────────────────────────────

/** One element of a `content` tool output. Mirrors `ToolResultContent.ts`. */
@Serializable
sealed interface ToolResultContent {
    @Serializable
    @SerialName("text")
    data class Text(val text: String = "", val providerOptions: ProviderOptions? = null) : ToolResultContent

    @Serializable
    @SerialName("file")
    data class File(
        val data: FileData = FileData.Data(),
        val mediaType: String = "",
        val filename: String? = null,
        val providerOptions: ProviderOptions? = null,
    ) : ToolResultContent

    @Serializable
    @SerialName("custom")
    data class Custom(val providerOptions: ProviderOptions? = null) : ToolResultContent
}

/** The output of a tool call, tagged on `type`. Mirrors `ToolResultOutput.ts`. */
@Serializable
sealed interface ToolResultOutput {
    @Serializable
    @SerialName("text")
    data class Text(val value: String, val providerOptions: ProviderOptions? = null) : ToolResultOutput

    @Serializable
    @SerialName("json")
    data class JsonValue(val value: JsonElement, val providerOptions: ProviderOptions? = null) : ToolResultOutput

    @Serializable
    @SerialName("execution-denied")
    data class ExecutionDenied(val reason: String? = null, val providerOptions: ProviderOptions? = null) : ToolResultOutput

    @Serializable
    @SerialName("error-text")
    data class ErrorText(val value: String, val providerOptions: ProviderOptions? = null) : ToolResultOutput

    @Serializable
    @SerialName("error-json")
    data class ErrorJson(val value: JsonElement, val providerOptions: ProviderOptions? = null) : ToolResultOutput

    @Serializable
    @SerialName("content")
    data class Content(val value: List<ToolResultContent>) : ToolResultOutput
}

// ─────────────────────────────────────────────────────────────────────────────
// ContentPart (multi-part message content), tagged on `type`.
// ─────────────────────────────────────────────────────────────────────────────

/**
 * A part of a multi-part message. Mirrors `ContentPart.ts`.
 *
 * Shared by [ModelMessage] (user-facing) and the provider-facing prompt. A
 * tool result carries a [ToolResultOutput] (`output`), not a bare value.
 */
@Serializable
sealed interface ContentPart {
    @Serializable
    @SerialName("text")
    data class Text(val text: String = "", val providerOptions: ProviderOptions? = null) : ContentPart

    @Serializable
    @SerialName("custom")
    data class Custom(val kind: String, val providerOptions: ProviderOptions? = null) : ContentPart

    @Serializable
    @SerialName("reasoning-file")
    data class ReasoningFile(
        val data: GeneratedFileData,
        val mediaType: String,
        val providerOptions: ProviderOptions? = null,
    ) : ContentPart

    @Serializable
    @SerialName("tool-approval-request")
    data class ToolApprovalRequest(
        val approvalId: String,
        val toolCallId: String,
        val reason: String? = null,
        val isAutomatic: Boolean? = null,
        val signature: String? = null,
        val inputSchemaInput: JsonElement? = null,
    ) : ContentPart

    @Serializable
    @SerialName("image")
    data class Image(
        val image: List<Int> = emptyList(),
        val mediaType: String = "",
        val providerOptions: ProviderOptions? = null,
    ) : ContentPart

    @Serializable
    @SerialName("file")
    data class File(
        val data: List<Int> = emptyList(),
        val mediaType: String = "",
        val filename: String? = null,
        val providerOptions: ProviderOptions? = null,
    ) : ContentPart

    @Serializable
    @SerialName("file-base64")
    data class FileBase64(
        val data: String = "",
        val mediaType: String = "",
        val filename: String? = null,
        val providerOptions: ProviderOptions? = null,
    ) : ContentPart

    @Serializable
    @SerialName("file-url")
    data class FileUrl(
        val url: String = "",
        val mediaType: String = "",
        val providerOptions: ProviderOptions? = null,
    ) : ContentPart

    @Serializable
    @SerialName("file-reference")
    data class FileReference(
        val mediaType: String = "",
        val reference: JsonElement = JsonObject(emptyMap()),
        val filename: String? = null,
        val providerOptions: ProviderOptions? = null,
    ) : ContentPart

    @Serializable
    @SerialName("reasoning")
    data class Reasoning(
        val text: String = "",
        val signature: String? = null,
        val providerOptions: ProviderOptions? = null,
    ) : ContentPart

    @Serializable
    @SerialName("tool-call")
    data class ToolCall(
        val toolCallId: String = "",
        val toolName: String = "",
        val input: JsonElement = JsonObject(emptyMap()),
        val providerExecuted: Boolean? = null,
        val providerOptions: ProviderOptions? = null,
    ) : ContentPart

    @Serializable
    @SerialName("tool-result")
    data class ToolResult(
        val toolCallId: String,
        val toolName: String,
        val output: ToolResultOutput,
        val providerOptions: ProviderOptions? = null,
    ) : ContentPart
}

// ─────────────────────────────────────────────────────────────────────────────
// ModelMessage (prompt side).
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Message body: either a simple string or multi-part content.
 *
 * Mirrors `MessageContent.ts`: `string | Array<ContentPart>` (untagged): a JSON
 * string decodes to [MessageContent.Text], a JSON array to [MessageContent.Parts].
 */
@Serializable(with = MessageContentSerializer::class)
sealed interface MessageContent {

    @Serializable
    data class Text(val text: String = "") : MessageContent

    @Serializable
    data class Parts(val parts: List<ContentPart> = emptyList()) : MessageContent
}

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
 * A single user-facing chat message. Mirrors `ModelMessage.ts`:
 * `{ role: Role, content: MessageContent }`. Use [contentString] /
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
 * Per-call timeout configuration. Mirrors `TimeoutConfiguration.ts`. All values
 * are milliseconds; `null` disables the corresponding limit. A `total` timeout
 * also covers retry backoff and the whole streamed response.
 */
@Serializable
data class TimeoutConfiguration(
    /** Overall timeout for the entire call (including retries and, for streaming, the whole stream), in milliseconds. */
    val totalMs: Long? = null,
    /** Timeout for one model operation attempt, in milliseconds. */
    val stepMs: Long? = null,
    /** Timeout waiting for the first stream chunk (streaming only). */
    val firstChunkMs: Long? = null,
    /** Maximum idle time between consecutive stream chunks (streaming only). */
    val chunkMs: Long? = null,
)

/** Response format, tagged on `type`. Mirrors `ResponseFormat.ts`. */
@Serializable
sealed interface ResponseFormat {
    @Serializable
    @SerialName("text")
    data object Text : ResponseFormat

    /** JSON output; `schema` is a JSON Schema for structured output. */
    @Serializable
    @SerialName("json")
    data class Json(
        val schema: JsonElement? = null,
        val name: String? = null,
        val description: String? = null,
    ) : ResponseFormat
}

/**
 * User-facing options for `generate_text` / `stream_text`.
 *
 * Mirrors `GenerateTextOptions.ts`. Every field is nullable with a `null`
 * default; combined with `explicitNulls=false`, only the fields the caller sets
 * are serialized onto the wire.
 */
@Serializable
data class GenerateTextOptions(
    val maxOutputTokens: Long? = null,
    val temperature: Double? = null,
    val stopSequences: List<String>? = null,
    val topP: Double? = null,
    val topK: Double? = null,
    val presencePenalty: Double? = null,
    val frequencyPenalty: Double? = null,
    val responseFormat: ResponseFormat? = null,
    val seed: Long? = null,
    val tools: List<Tool>? = null,
    val toolChoice: ToolChoice? = null,
    val headers: Map<String, String>? = null,
    val providerOptions: ProviderOptions? = null,
    val reasoning: ReasoningEffort? = null,
    val instructions: String? = null,
    /** Per-call retry count (0 = disable retries). */
    val maxRetries: Long? = null,
    /** Emit raw provider stream chunks as `raw` stream parts (debugging; OpenAI-compatible family only). */
    val includeRawChunks: Boolean? = null,
    /** Per-call timeout configuration (overall / first chunk / inter-chunk idle, in ms). */
    val timeout: TimeoutConfiguration? = null,
    /** Session identifier (RFC-0024): groups consecutive calls into a session. */
    val sessionId: String? = null,
    /**
     * One repair attempt per invalid tool call (RFC-0035), run on the JVM after
     * the call returns. Mirrors AI SDK `repairToolCall`.
     *
     * [Transient]: a function cannot be serialized, and core never sees it —
     * [TypedModel] runs it host-side. Honoured by `generateText`,
     * `generateObject`, `consumeStreamText`, `streamText` and
     * `generateTextAsOpenAI` (which repairs the native result before
     * converting it); `streamTextAsOpenAI` ignores it, because the OpenAI
     * stream forwards provider argument deltas verbatim. The raw JSON-string
     * [Model] ignores it too — it never sees typed options.
     *
     * If a repair fails at the boundary while streaming (not in the hook — a
     * throwing hook is a `ToolCallRepair` error on the call), `onError` is told
     * and the unrepaired part is still delivered. The stream waits for the
     * hook, without a timeout.
     *
     * While streaming, the hook runs on a worker thread that is lent the
     * stream's read hold on this model, so it may call this same model even
     * against a concurrent `close()`. The lend is valid only while the stream
     * thread blocks holding that read lock, and only on the thread the hook is
     * invoked on: extra threads the hook starts itself take the lock normally
     * and would deadlock against a concurrent `close()`. Call the model from
     * the thread the hook is invoked on.
     */
    @Transient val repairToolCall: RepairToolCall? = null,
)

// ─────────────────────────────────────────────────────────────────────────────
// Results.
// ─────────────────────────────────────────────────────────────────────────────

/** A URL or document used as a source for the response. Mirrors `Source.ts` (tagged on `sourceType`). */
@OptIn(ExperimentalSerializationApi::class)
@Serializable
@JsonClassDiscriminator("sourceType")
sealed interface Source {
    val id: String
    val providerMetadata: ProviderMetadata?

    @Serializable
    @SerialName("url")
    data class Url(
        override val id: String,
        val url: String,
        val title: String? = null,
        override val providerMetadata: ProviderMetadata? = null,
    ) : Source

    @Serializable
    @SerialName("document")
    data class Document(
        override val id: String,
        val mediaType: String,
        val title: String,
        val filename: String? = null,
        override val providerMetadata: ProviderMetadata? = null,
    ) : Source
}

/** A provider warning, tagged on `type`. Mirrors `Warning.ts`. */
@Serializable
sealed interface Warning {
    @Serializable
    @SerialName("unsupported")
    data class Unsupported(val feature: String, val details: String? = null) : Warning

    @Serializable
    @SerialName("compatibility")
    data class Compatibility(val feature: String, val details: String? = null) : Warning

    @Serializable
    @SerialName("deprecated")
    data class Deprecated(val setting: String, val message: String) : Warning

    @Serializable
    @SerialName("other")
    data class Other(val message: String) : Warning
}

/**
 * A content item of the provider-level result (`GenerateResult.content`),
 * tagged on `type`. Mirrors `GenerateContent<RawToolCall, RawToolApprovalRequest,
 * GeneratedFile>`: a tool call's `input` is the raw argument text, and an
 * approval request carries just the ids.
 */
@Serializable
sealed interface GenerateContent {
    @Serializable
    @SerialName("text")
    data class Text(val text: String = "", val providerMetadata: ProviderMetadata? = null) : GenerateContent

    @Serializable
    @SerialName("tool-call")
    data class ToolCall(
        val toolCallId: String = "",
        val toolName: String = "",
        val input: String = "",
        val providerExecuted: Boolean? = null,
        val dynamic: Boolean? = null,
        val providerMetadata: ProviderMetadata? = null,
    ) : GenerateContent

    @Serializable
    @SerialName("source")
    data class Source(
        val sourceType: String,
        val id: String,
        val url: String? = null,
        val mediaType: String? = null,
        val title: String? = null,
        val filename: String? = null,
        val providerMetadata: ProviderMetadata? = null,
    ) : GenerateContent

    @Serializable
    @SerialName("reasoning")
    data class Reasoning(val text: String = "", val providerMetadata: ProviderMetadata? = null) : GenerateContent

    @Serializable
    @SerialName("file")
    data class File(
        val data: GeneratedFileData = GeneratedFileData.Data(),
        val mediaType: String = "",
        val providerMetadata: ProviderMetadata? = null,
    ) : GenerateContent

    @Serializable
    @SerialName("reasoning-file")
    data class ReasoningFile(
        val data: GeneratedFileData = GeneratedFileData.Data(),
        val mediaType: String = "",
        val providerMetadata: ProviderMetadata? = null,
    ) : GenerateContent

    @Serializable
    @SerialName("custom")
    data class Custom(val kind: String, val providerMetadata: ProviderMetadata? = null) : GenerateContent

    @Serializable
    @SerialName("tool-approval-request")
    data class ToolApprovalRequest(
        val approvalId: String,
        val toolCallId: String,
        val providerMetadata: ProviderMetadata? = null,
    ) : GenerateContent

    @Serializable
    @SerialName("tool-result")
    data class ToolResult(
        val toolCallId: String = "",
        val toolName: String = "",
        val result: JsonElement = JsonObject(emptyMap()),
        val isError: Boolean? = null,
        val preliminary: Boolean? = null,
        val dynamic: Boolean? = null,
        val providerMetadata: ProviderMetadata? = null,
    ) : GenerateContent
}

/** Result of `LanguageModel::do_generate`, surfaced as `GenerateTextResult.raw`. Mirrors `GenerateResult.ts`. */
@Serializable
data class GenerateResult(
    val content: List<GenerateContent> = emptyList(),
    val finishReason: FinishReason = FinishReason(),
    val usage: Usage = Usage(),
    val warnings: List<Warning> = emptyList(),
    val providerMetadata: ProviderMetadata? = null,
    val request: RequestInfo? = null,
    val response: ResponseInfo? = null,
)

/**
 * Result of `generate_text` (user-facing). Mirrors `GenerateTextResult.ts`.
 *
 * `content`, `reasoning` and the tool-call items are kept as raw [JsonElement]s
 * (their element types are generic over the call layer).
 */
@Serializable
data class GenerateTextResult(
    val content: List<JsonElement>? = null,
    val text: String = "",
    val toolCalls: List<ToolCall> = emptyList(),
    val finishReason: FinishReason = FinishReason(),
    val usage: Usage = Usage(),
    val warnings: List<Warning> = emptyList(),
    val raw: GenerateResult = GenerateResult(),
    val reasoning: List<JsonElement> = emptyList(),
    val reasoningText: String = "",
    val sources: List<Source> = emptyList(),
    val files: List<GeneratedFile> = emptyList(),
    val responseMessages: List<ModelMessage> = emptyList(),
    val rawFinishReason: String? = null,
    val providerMetadata: ProviderMetadata? = null,
    val request: RequestInfo = RequestInfo(),
    val response: ResponseInfo = ResponseInfo(),
    /** Total token usage across all steps (equals [usage] in single-step mode). */
    val totalUsage: Usage = Usage(),
)

/**
 * Result of `generate_object` (user-facing). The parsed JSON object plus
 * convenience fields from the underlying `generate_text` call. Mirrors
 * `GenerateObjectResult.ts`; `object` is an arbitrary JSON value.
 */
@Serializable
data class GenerateObjectResult(
    val `object`: JsonElement,
    val finishReason: FinishReason = FinishReason(),
    val rawFinishReason: String? = null,
    val usage: Usage = Usage(),
    val warnings: List<Warning> = emptyList(),
    /** Concatenated reasoning text, if the model produced reasoning. */
    val reasoning: String? = null,
    val providerMetadata: ProviderMetadata? = null,
    val response: ResponseMetadata = ResponseMetadata(),
    val raw: GenerateTextResult = GenerateTextResult(),
)

/**
 * Aggregated result of `stream_text().consume()`. Mirrors
 * `StreamTextResultAggregated.ts` (`GenerateTextResult` without `raw`).
 */
@Serializable
data class StreamTextResultAggregated(
    val content: List<JsonElement>? = null,
    val text: String = "",
    val reasoning: List<JsonElement> = emptyList(),
    val reasoningText: String = "",
    val toolCalls: List<ToolCall> = emptyList(),
    val sources: List<Source> = emptyList(),
    val files: List<GeneratedFile> = emptyList(),
    val finishReason: FinishReason = FinishReason(),
    val rawFinishReason: String? = null,
    val usage: Usage = Usage(),
    val totalUsage: Usage = Usage(),
    val warnings: List<Warning> = emptyList(),
    val providerMetadata: ProviderMetadata? = null,
    val request: RequestInfo = RequestInfo(),
    val response: ResponseInfo = ResponseInfo(),
    val responseMessages: List<ModelMessage> = emptyList(),
)

// ─────────────────────────────────────────────────────────────────────────────
// StreamPart (the streaming chunk type), tagged on `type`.
//
// Mirrors `TextStreamPart.ts`, the part type `aimux_stream_text` emits. An
// unknown `type` fails decoding ([TypedModel] reports it through `onError`).
// ─────────────────────────────────────────────────────────────────────────────

@Serializable
sealed interface StreamPart {
    @Serializable
    @SerialName("text-start")
    data class TextStart(val id: String = "", val providerMetadata: ProviderMetadata? = null) : StreamPart

    @Serializable
    @SerialName("text-delta")
    data class TextDelta(
        val id: String = "",
        val delta: String = "",
        val providerMetadata: ProviderMetadata? = null,
    ) : StreamPart

    @Serializable
    @SerialName("text-end")
    data class TextEnd(val id: String = "", val providerMetadata: ProviderMetadata? = null) : StreamPart

    @Serializable
    @SerialName("stream-start")
    data class StreamStart(val warnings: List<Warning> = emptyList()) : StreamPart

    @Serializable
    @SerialName("finish")
    data class Finish(
        val finishReason: FinishReason = FinishReason(),
        val usage: Usage = Usage(),
        val providerMetadata: ProviderMetadata? = null,
    ) : StreamPart

    @Serializable
    @SerialName("finish-step")
    data class FinishStep(
        val finishReason: FinishReason = FinishReason(),
        val usage: Usage = Usage(),
        val providerMetadata: ProviderMetadata? = null,
        val response: ResponseInfo = ResponseInfo(),
    ) : StreamPart

    /** `error` is the `name`-tagged `AiMuxError` JSON. */
    @Serializable
    @SerialName("error")
    data class Error(val error: JsonElement = JsonObject(emptyMap())) : StreamPart

    @Serializable
    @SerialName("tool-input-start")
    data class ToolInputStart(
        val id: String = "",
        val toolName: String = "",
        val providerExecuted: Boolean? = null,
        val dynamic: Boolean? = null,
        val title: String? = null,
        val providerMetadata: ProviderMetadata? = null,
    ) : StreamPart

    @Serializable
    @SerialName("tool-input-delta")
    data class ToolInputDelta(
        val id: String = "",
        val delta: String = "",
        val providerMetadata: ProviderMetadata? = null,
    ) : StreamPart

    @Serializable
    @SerialName("tool-input-end")
    data class ToolInputEnd(val id: String = "", val providerMetadata: ProviderMetadata? = null) : StreamPart

    /**
     * `invalid` is set by Core when the tool call stays invalid after optional
     * repair; `error` is the typed lookup, parse, schema, or repair failure.
     */
    @Serializable
    @SerialName("tool-call")
    data class ToolCall(
        val toolCallId: String = "",
        val toolName: String = "",
        val input: JsonElement = JsonObject(emptyMap()),
        val providerExecuted: Boolean? = null,
        val dynamic: Boolean? = null,
        val providerMetadata: ProviderMetadata? = null,
        val invalid: Boolean? = null,
        val error: JsonElement? = null,
    ) : StreamPart

    @Serializable
    @SerialName("tool-result")
    data class ToolResult(
        val toolCallId: String = "",
        val toolName: String = "",
        val result: JsonElement = JsonObject(emptyMap()),
        val isError: Boolean? = null,
        val preliminary: Boolean? = null,
        val dynamic: Boolean? = null,
        val providerMetadata: ProviderMetadata? = null,
    ) : StreamPart

    /** A file generated by the model (e.g. an image or document). */
    @Serializable
    @SerialName("file")
    data class File(
        val data: GeneratedFileData = GeneratedFileData.Data(),
        val mediaType: String = "",
        val providerMetadata: ProviderMetadata? = null,
    ) : StreamPart

    @Serializable
    @SerialName("reasoning-file")
    data class ReasoningFile(
        val file: GeneratedFile = GeneratedFile(),
        val providerMetadata: ProviderMetadata? = null,
    ) : StreamPart

    @Serializable
    @SerialName("custom")
    data class Custom(val kind: String, val providerMetadata: ProviderMetadata? = null) : StreamPart

    @Serializable
    @SerialName("tool-approval-request")
    data class ToolApprovalRequest(
        val approvalId: String,
        val toolCall: ai.arcships.aimux.ToolCall,
        val reason: String? = null,
        val isAutomatic: Boolean? = null,
        val signature: String? = null,
    ) : StreamPart

    @Serializable
    @SerialName("reasoning-start")
    data class ReasoningStart(val id: String = "", val providerMetadata: ProviderMetadata? = null) : StreamPart

    @Serializable
    @SerialName("reasoning-delta")
    data class ReasoningDelta(
        val id: String = "",
        val delta: String = "",
        val providerMetadata: ProviderMetadata? = null,
    ) : StreamPart

    @Serializable
    @SerialName("reasoning-end")
    data class ReasoningEnd(val id: String = "", val providerMetadata: ProviderMetadata? = null) : StreamPart

    /** A source cited by the response (flat: `type: "source"` plus the [Source] fields). */
    @Serializable
    @SerialName("source")
    data class Source(
        val sourceType: String,
        val id: String,
        val url: String? = null,
        val mediaType: String? = null,
        val title: String? = null,
        val filename: String? = null,
        val providerMetadata: ProviderMetadata? = null,
    ) : StreamPart

    @Serializable
    @SerialName("raw")
    data class Raw(val rawValue: JsonElement = JsonObject(emptyMap())) : StreamPart
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
