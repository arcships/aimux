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

import kotlinx.serialization.EncodeDefault
import kotlinx.serialization.ExperimentalSerializationApi
import kotlinx.serialization.KSerializer
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.SerializationException
import kotlinx.serialization.Transient
import kotlinx.serialization.descriptors.SerialDescriptor
import kotlinx.serialization.descriptors.buildClassSerialDescriptor
import kotlinx.serialization.descriptors.nullable
import kotlinx.serialization.encoding.Decoder
import kotlinx.serialization.encoding.Encoder
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonClassDiscriminator
import kotlinx.serialization.json.JsonDecoder
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonEncoder
import kotlinx.serialization.json.JsonNull
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

/**
 * (De)serializer for a `JsonElement?` that is an optional `unknown` upstream
 * (a response `body`, `inputSchemaInput`): a missing field is `null` and an
 * explicit JSON `null` is [JsonNull], as the AI SDK's JSON keeps `null` apart
 * from `undefined` (Rust `wire::present`).
 */
object JsonValuePresentSerializer : KSerializer<JsonElement?> {
    override val descriptor: SerialDescriptor = JsonElement.serializer().descriptor.nullable

    override fun deserialize(decoder: Decoder): JsonElement? {
        val json = decoder as? JsonDecoder
            ?: throw SerializationException("a JSON value can only be decoded from JSON")
        return json.decodeJsonElement()
    }

    override fun serialize(encoder: Encoder, value: JsonElement?) {
        val json = encoder as? JsonEncoder
            ?: throw SerializationException("a JSON value can only be encoded to JSON")
        json.encodeJsonElement(value ?: JsonNull)
    }
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

/** Input-token breakdown of a [LanguageModelUsage]. Mirrors `InputTokenDetails.ts`. */
@Serializable
data class InputTokenDetails(
    @SerialName("no_cache_tokens") val noCacheTokens: Long? = null,
    @SerialName("cache_read_tokens") val cacheReadTokens: Long? = null,
    @SerialName("cache_write_tokens") val cacheWriteTokens: Long? = null,
)

/** Output-token breakdown of a [LanguageModelUsage]. Mirrors `OutputTokenDetails.ts`. */
@Serializable
data class OutputTokenDetails(
    @SerialName("text_tokens") val textTokens: Long? = null,
    @SerialName("reasoning_tokens") val reasoningTokens: Long? = null,
)

/**
 * Call-level token usage, the AI SDK's `LanguageModelUsage`: flat counts,
 * unknown counts `null`. Provider results keep [Usage].
 *
 * Mirrors `LanguageModelUsage.ts`.
 */
@Serializable
data class LanguageModelUsage(
    @SerialName("input_tokens") val inputTokens: Long? = null,
    @SerialName("input_token_details") val inputTokenDetails: InputTokenDetails = InputTokenDetails(),
    @SerialName("output_tokens") val outputTokens: Long? = null,
    @SerialName("output_token_details") val outputTokenDetails: OutputTokenDetails = OutputTokenDetails(),
    @SerialName("total_tokens") val totalTokens: Long? = null,
    val raw: JsonObject? = null,
)

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
    @Serializable(with = JsonValuePresentSerializer::class)
    val body: JsonElement? = null,
)

@Serializable
data class RequestInfo(
    @Serializable(with = JsonValuePresentSerializer::class)
    val body: JsonElement? = null,
)

/**
 * A tool call requested by the model. Mirrors `ToolCall.ts`.
 *
 * `input` is the parsed argument value (usually an object). `invalid` is set
 * by Core when the call stays invalid after optional repair; `error` is the
 * `name`-tagged `AiMuxError` JSON of the lookup, parse, schema, or repair
 * failure (`{"name":"AI_NoSuchToolError",...}`).
 *
 * Its JSON carries `"type": "tool-call"`: a call outside `content` is the AI
 * SDK's tagged `TypedToolCall` (ai generate-text/tool-call.ts).
 */
@OptIn(ExperimentalSerializationApi::class)
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
) {
    /** Always `"tool-call"`. */
    @EncodeDefault
    val type: String = "tool-call"

    init {
        requireTypeTag("tool-call", type)
    }
}

/**
 * Check the `type` tag of a value that always carries one (a standalone tool
 * call or source); JSON written before the tag has none, and reads the default.
 */
internal fun requireTypeTag(expected: String, actual: String) {
    if (actual != expected) {
        throw SerializationException("expected type \"$expected\", got \"$actual\"")
    }
}

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
 * Bytes in the AI SDK's JSON: a `Uint8Array` is a base64 string there
 * (`@ai-sdk/provider-utils` `uint8-utils.ts:12-23`), never an array of numbers.
 * Byte values (0–255) are encoded and decoded at the JSON boundary.
 */
object Base64Bytes {
    /** Standard, padded base64, as `btoa`. */
    fun encode(bytes: List<Int>): String =
        java.util.Base64.getEncoder().encodeToString(ByteArray(bytes.size) { bytes[it].toByte() })

    /**
     * Decodes as `convertBase64ToUint8Array` (`uint8-utils.ts:6-10`): `-` and `_`
     * read as `+` and `/`, padding optional, ASCII whitespace ignored.
     */
    fun decode(text: String): List<Int> {
        val normalized = buildString {
            for (c in text) {
                when (c) {
                    ' ', '\t', '\n', '\u000C', '\r', '=' -> Unit
                    '-' -> append('+')
                    '_' -> append('/')
                    else -> append(c)
                }
            }
        }
        val raw = try {
            java.util.Base64.getDecoder().decode(normalized)
        } catch (e: IllegalArgumentException) {
            throw SerializationException("invalid base64 data: ${e.message}")
        }
        return raw.map { it.toInt() and 0xff }
    }

    internal fun decode(el: JsonElement, what: String): List<Int> {
        val text = (el as? JsonPrimitive)?.takeIf { it.isString }?.content
            ?: throw SerializationException("$what must be a base64 string, got: $el")
        return decode(text)
    }
}

/** (De)serializer for a byte list field: a base64 string on the wire. */
object Base64ByteListSerializer : KSerializer<List<Int>> {
    override val descriptor: SerialDescriptor = buildClassSerialDescriptor("aimux.Base64Bytes")

    override fun deserialize(decoder: Decoder): List<Int> {
        val json = decoder as? JsonDecoder
            ?: throw SerializationException("bytes can only be decoded from JSON")
        return Base64Bytes.decode(json.decodeJsonElement(), "bytes")
    }

    override fun serialize(encoder: Encoder, value: List<Int>) {
        val json = encoder as? JsonEncoder
            ?: throw SerializationException("bytes can only be encoded to JSON")
        json.encodeJsonElement(JsonPrimitive(Base64Bytes.encode(value)))
    }
}

/**
 * Raw bytes or a base64 string. Mirrors `FileBytes.ts`: `string`, the AI SDK's
 * `Uint8Array | string` in JSON (`shared-v4-file-data.ts:7-10`). [Binary] is
 * written as base64; parsing yields [Base64]. Also the shape of every
 * `Uint8Array | string` field (audio input, image / video file data).
 */
@Serializable(with = FileBytesSerializer::class)
sealed interface FileBytes {

    /** Raw binary bytes (0–255 ints), written as a base64 string. */
    data class Binary(val data: List<Int> = emptyList()) : FileBytes

    /** A base64-encoded string. */
    data class Base64(val data: String = "") : FileBytes
}

object FileBytesSerializer : KSerializer<FileBytes> {
    override val descriptor: SerialDescriptor = buildClassSerialDescriptor("aimux.FileBytes")

    override fun deserialize(decoder: Decoder): FileBytes {
        val json = decoder as? JsonDecoder
            ?: throw SerializationException("FileBytes can only be decoded from JSON")
        val el = json.decodeJsonElement()
        val text = (el as? JsonPrimitive)?.takeIf { it.isString }?.content
            ?: throw SerializationException("FileBytes must be a base64 string, got: $el")
        return FileBytes.Base64(text)
    }

    override fun serialize(encoder: Encoder, value: FileBytes) {
        val json = encoder as? JsonEncoder
            ?: throw SerializationException("FileBytes can only be encoded to JSON")
        json.encodeJsonElement(
            when (value) {
                is FileBytes.Binary -> JsonPrimitive(Base64Bytes.encode(value.data))
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

/**
 * `FilePart.data` (Rust `FilePartData`, provider-utils
 * `types/content-part.ts:75`): `FileData | DataContent | URL |
 * ProviderReference`, in JSON a tagged [FileData], a string (base64 or a URL)
 * or a bare `{provider: id}` reference.
 */
@Serializable(with = FilePartDataSerializer::class)
sealed interface FilePartData {
    /** A tagged [FileData]. */
    data class Tagged(val data: FileData) : FilePartData

    /** A bare string: base64 data or a URL. */
    data class Bare(val value: String) : FilePartData

    /** A bare provider reference, `{provider: id}`. */
    data class Reference(val reference: Map<String, String>) : FilePartData
}

private val FILE_DATA_TAGS = setOf("data", "url", "reference", "text")
private val GENERATED_FILE_DATA_TAGS = setOf("data", "url")

private fun fileDataTag(element: JsonElement): String? =
    ((element as? JsonObject)?.get("type") as? JsonPrimitive)?.takeIf { it.isString }?.content

object FilePartDataSerializer : KSerializer<FilePartData> {
    override val descriptor: SerialDescriptor = buildClassSerialDescriptor("aimux.FilePartData")

    override fun deserialize(decoder: Decoder): FilePartData {
        val json = decoder as? JsonDecoder
            ?: throw SerializationException("FilePartData can only be decoded from JSON")
        val el = json.decodeJsonElement()
        if (el is JsonPrimitive && el.isString) return FilePartData.Bare(el.content)
        if (fileDataTag(el) in FILE_DATA_TAGS) {
            return FilePartData.Tagged(json.json.decodeFromJsonElement(FileData.serializer(), el))
        }
        if (el is JsonObject) {
            return FilePartData.Reference(el.mapValues { (_, id) ->
                (id as? JsonPrimitive)?.takeIf { it.isString }?.content
                    ?: throw SerializationException("a provider reference maps providers to string ids, got: $el")
            })
        }
        throw SerializationException("file data must be a tagged object, a string or a provider reference, got: $el")
    }

    override fun serialize(encoder: Encoder, value: FilePartData) {
        val json = encoder as? JsonEncoder
            ?: throw SerializationException("FilePartData can only be encoded to JSON")
        json.encodeJsonElement(
            when (value) {
                is FilePartData.Tagged -> json.json.encodeToJsonElement(FileData.serializer(), value.data)
                is FilePartData.Bare -> JsonPrimitive(value.value)
                is FilePartData.Reference -> JsonObject(value.reference.mapValues { JsonPrimitive(it.value) })
            }
        )
    }
}

/**
 * `ReasoningFilePart.data` (Rust `ReasoningFilePartData`,
 * `types/content-part.ts:164`): tagged data or a URL ([GeneratedFileData]), or
 * a bare string (base64 or a URL).
 */
@Serializable(with = ReasoningFilePartDataSerializer::class)
sealed interface ReasoningFilePartData {
    /** Tagged data or a URL. */
    data class Tagged(val data: GeneratedFileData) : ReasoningFilePartData

    /** A bare string: base64 data or a URL. */
    data class Bare(val value: String) : ReasoningFilePartData
}

object ReasoningFilePartDataSerializer : KSerializer<ReasoningFilePartData> {
    override val descriptor: SerialDescriptor = buildClassSerialDescriptor("aimux.ReasoningFilePartData")

    override fun deserialize(decoder: Decoder): ReasoningFilePartData {
        val json = decoder as? JsonDecoder
            ?: throw SerializationException("ReasoningFilePartData can only be decoded from JSON")
        val el = json.decodeJsonElement()
        if (el is JsonPrimitive && el.isString) return ReasoningFilePartData.Bare(el.content)
        if (fileDataTag(el) in GENERATED_FILE_DATA_TAGS) {
            return ReasoningFilePartData.Tagged(json.json.decodeFromJsonElement(GeneratedFileData.serializer(), el))
        }
        throw SerializationException("reasoning file data must be tagged data or a URL, or a string, got: $el")
    }

    override fun serialize(encoder: Encoder, value: ReasoningFilePartData) {
        val json = encoder as? JsonEncoder
            ?: throw SerializationException("ReasoningFilePartData can only be encoded to JSON")
        json.encodeJsonElement(
            when (value) {
                is ReasoningFilePartData.Tagged ->
                    json.json.encodeToJsonElement(GeneratedFileData.serializer(), value.data)
                is ReasoningFilePartData.Bare -> JsonPrimitive(value.value)
            }
        )
    }
}

/**
 * A generated file (Rust `GeneratedFile`, the AI SDK's `GeneratedFile`,
 * `generate-text/generated-file.ts:10-32`): `{"base64", "mediaType",
 * "providerMetadata"?}` on the wire. Mirrors `GeneratedFile.ts`.
 */
@Serializable
data class GeneratedFile(
    /** The file as a base64 string. */
    val base64: String,
    val mediaType: String,
    val providerMetadata: ProviderMetadata? = null,
) {
    /** The file's bytes (0–255). */
    fun bytes(): List<Int> = Base64Bytes.decode(base64)
}

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

    /** The AI SDK `ReasoningFilePart` (`types/content-part.ts:146-180`). */
    @Serializable
    @SerialName("reasoning-file")
    data class ReasoningFile(
        val data: ReasoningFilePartData,
        val mediaType: String,
        val providerOptions: ProviderOptions? = null,
    ) : ContentPart {
        constructor(data: GeneratedFileData, mediaType: String, providerOptions: ProviderOptions? = null) :
            this(ReasoningFilePartData.Tagged(data), mediaType, providerOptions)
    }

    @Serializable
    @SerialName("tool-approval-request")
    data class ToolApprovalRequest(
        val approvalId: String,
        val toolCallId: String,
        val reason: String? = null,
        val isAutomatic: Boolean? = null,
        val signature: String? = null,
        @Serializable(with = JsonValuePresentSerializer::class)
        val inputSchemaInput: JsonElement? = null,
    ) : ContentPart

    @Serializable
    @SerialName("image")
    data class Image(
        /** Image bytes (0–255), a base64 string on the wire. */
        @Serializable(with = Base64ByteListSerializer::class)
        val image: List<Int> = emptyList(),
        val mediaType: String = "",
        val providerOptions: ProviderOptions? = null,
    ) : ContentPart

    /**
     * The AI SDK `FilePart` (provider-utils `types/content-part.ts:62-102`):
     * `data` is a [FilePartData]: tagged [FileData], a bare string (base64 or a
     * URL) or a bare provider reference.
     */
    @Serializable
    @SerialName("file")
    data class File(
        val data: FilePartData,
        val mediaType: String = "",
        val filename: String? = null,
        val providerOptions: ProviderOptions? = null,
    ) : ContentPart {
        /** Tagged data, e.g. [FileData.Url]. */
        constructor(data: FileData, mediaType: String, filename: String? = null, providerOptions: ProviderOptions? = null) :
            this(FilePartData.Tagged(data), mediaType, filename, providerOptions)

        /** Inline bytes (0–255), written as `{"type":"data","data":"<base64>"}`. */
        constructor(data: List<Int>, mediaType: String, filename: String? = null, providerOptions: ProviderOptions? = null) :
            this(FileData.Data(FileBytes.Binary(data)), mediaType, filename, providerOptions)
    }

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
    /**
     * Structured output, the AI SDK's `output`: for example
     * `{"Object": {"schema": {...}}}`, `{"Array": {"element": {...}}}`,
     * `{"Choice": {"options": [...]}}` or `{"Json": {}}`. Untyped
     * ([JsonElement]). The parsed value is [GenerateTextResult.output].
     */
    val output: JsonElement? = null,
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

/**
 * A URL or document used as a source for the response. Mirrors `Source.ts`
 * (tagged on `sourceType`). Its JSON also carries `"type": "source"`, as the
 * AI SDK's `Source` (provider language-model-v4-source.ts) does in a result's
 * `sources`.
 */
@OptIn(ExperimentalSerializationApi::class)
@Serializable
@JsonClassDiscriminator("sourceType")
sealed interface Source {
    val id: String
    val providerMetadata: ProviderMetadata?

    /** Always `"source"`. */
    val type: String

    @Serializable
    @SerialName("url")
    data class Url(
        override val id: String,
        val url: String,
        val title: String? = null,
        override val providerMetadata: ProviderMetadata? = null,
    ) : Source {
        @EncodeDefault
        override val type: String = "source"

        init {
            requireTypeTag("source", type)
        }
    }

    @Serializable
    @SerialName("document")
    data class Document(
        override val id: String,
        val mediaType: String,
        val title: String,
        val filename: String? = null,
        override val providerMetadata: ProviderMetadata? = null,
    ) : Source {
        @EncodeDefault
        override val type: String = "source"

        init {
            requireTypeTag("source", type)
        }
    }
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

/** Result of `LanguageModel::do_generate`, the provider-level result of one model call. Mirrors `GenerateResult.ts`. */
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
 * Result of `generate_text` and of a consumed stream (user-facing), the AI
 * SDK's `GenerateTextResult`. [steps] holds every step (weak type); the other
 * fields are derived from them: the last step's text, finish reason,
 * reasoning, request, response and provider metadata, every step's content,
 * files, sources, tool calls, tool results and warnings, and the usage summed
 * over the steps. [reasoning], [reasoningText] (`null` when there is none),
 * [totalUsage] (equal to [usage]), [request], [response] and
 * [providerMetadata] are deprecated upstream. [output] is the value parsed by
 * the `output` option, `null` when it was not parsed.
 *
 * Mirrors `GenerateTextResult.ts`; [response] reads the id, timestamp and
 * model id of the step response. `content`, `reasoning`, the tool results and
 * the steps are kept as raw [JsonElement]s.
 */
@Serializable
data class GenerateTextResult(
    val text: String = "",
    val content: List<JsonElement> = emptyList(),
    val reasoning: List<JsonElement> = emptyList(),
    val reasoningText: String? = null,
    val files: List<GeneratedFile> = emptyList(),
    val sources: List<Source> = emptyList(),
    val toolCalls: List<ToolCall> = emptyList(),
    val toolResults: List<JsonElement> = emptyList(),
    val finishReason: FinishReasonUnified = FinishReasonUnified.OTHER,
    val rawFinishReason: String? = null,
    val usage: LanguageModelUsage = LanguageModelUsage(),
    val totalUsage: LanguageModelUsage = LanguageModelUsage(),
    val warnings: List<Warning> = emptyList(),
    val request: RequestInfo = RequestInfo(),
    val response: ResponseMetadata = ResponseMetadata(),
    val responseMessages: List<ModelMessage> = emptyList(),
    val providerMetadata: ProviderMetadata? = null,
    val steps: List<JsonElement> = emptyList(),
    val output: JsonElement? = null,
)

/**
 * Result of `generate_object` (user-facing, M12), the AI SDK's
 * `GenerateObjectResult`: the parsed JSON object and the call's reasoning,
 * finish reason, usage, warnings, request, response and provider metadata.
 * The AI SDK deprecates `generateObject` for `generateText` with an `output`.
 *
 * Mirrors `GenerateObjectResult.ts`. `object` is an arbitrary JSON value;
 * [response] reads the id, timestamp and model id.
 */
@Serializable
data class GenerateObjectResult(
    val `object`: JsonElement,
    /** The reasoning text, `null` when there is none. */
    val reasoning: String? = null,
    val finishReason: FinishReasonUnified = FinishReasonUnified.OTHER,
    val usage: LanguageModelUsage = LanguageModelUsage(),
    val warnings: List<Warning> = emptyList(),
    val request: RequestInfo = RequestInfo(),
    val response: ResponseMetadata = ResponseMetadata(),
    val providerMetadata: ProviderMetadata? = null,
)

// ─────────────────────────────────────────────────────────────────────────────
// StreamPart (the streaming chunk type), tagged on `type`.
//
// Mirrors `TextStreamPart.ts`, the AI SDK's full stream that
// `aimux_stream_text` emits: `start`, then per step `start-step` …
// `finish-step`, then `finish` (or `abort`). An unknown `type` fails decoding
// ([TypedModel] reports it through `onError`).
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
        /** The delta, under `text` as in the AI SDK's TextStreamPart. */
        val text: String = "",
        val providerMetadata: ProviderMetadata? = null,
    ) : StreamPart

    @Serializable
    @SerialName("text-end")
    data class TextEnd(val id: String = "", val providerMetadata: ProviderMetadata? = null) : StreamPart

    /** Opens the stream (the AI SDK's `start` part). */
    @Serializable
    @SerialName("start")
    class Start : StreamPart {
        override fun equals(other: Any?): Boolean = other is Start
        override fun hashCode(): Int = 0
        override fun toString(): String = "Start"
    }

    /** Opens a step: the step's request and the provider's call warnings. */
    @Serializable
    @SerialName("start-step")
    data class StartStep(
        val request: RequestInfo = RequestInfo(),
        val warnings: List<Warning> = emptyList(),
    ) : StreamPart

    /** Closes a step: its response metadata, usage, performance and finish reason. */
    @Serializable
    @SerialName("finish-step")
    data class FinishStep(
        val response: ResponseMetadata = ResponseMetadata(),
        val usage: LanguageModelUsage = LanguageModelUsage(),
        /** Timing of the step (Rust `StepResultPerformance`). Weak type. */
        val performance: JsonElement? = null,
        val finishReason: FinishReasonUnified = FinishReasonUnified.OTHER,
        val rawFinishReason: String? = null,
        val providerMetadata: ProviderMetadata? = null,
    ) : StreamPart

    /** Closes the stream: the last step's finish reason and the usage summed over the steps. */
    @Serializable
    @SerialName("finish")
    data class Finish(
        val finishReason: FinishReasonUnified = FinishReasonUnified.OTHER,
        val rawFinishReason: String? = null,
        val totalUsage: LanguageModelUsage = LanguageModelUsage(),
    ) : StreamPart

    /** Ends a stream whose call was aborted. */
    @Serializable
    @SerialName("abort")
    data class Abort(val reason: String? = null) : StreamPart

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

    /**
     * A provider-executed tool's result (Rust `TypedToolResult`), with the
     * input of the call it answers (`null` for a deferred result).
     */
    @Serializable
    @SerialName("tool-result")
    data class ToolResult(
        @SerialName("tool_call_id") val toolCallId: String = "",
        @SerialName("tool_name") val toolName: String = "",
        val input: JsonElement? = null,
        val output: JsonElement = JsonNull,
        @SerialName("provider_executed") val providerExecuted: Boolean? = null,
        @SerialName("provider_metadata") val providerMetadata: ProviderMetadata? = null,
        val dynamic: Boolean? = null,
        val preliminary: Boolean? = null,
    ) : StreamPart

    /**
     * A provider-executed tool's error result, or the error that follows an
     * invalid tool call (Rust `TypedToolError`).
     */
    @Serializable
    @SerialName("tool-error")
    data class ToolError(
        @SerialName("tool_call_id") val toolCallId: String = "",
        @SerialName("tool_name") val toolName: String = "",
        val input: JsonElement? = null,
        val error: JsonElement = JsonNull,
        @SerialName("provider_executed") val providerExecuted: Boolean? = null,
        @SerialName("provider_metadata") val providerMetadata: ProviderMetadata? = null,
        val dynamic: Boolean? = null,
    ) : StreamPart

    /** A tool whose execution was denied (Rust `ToolOutputDenied`). */
    @Serializable
    @SerialName("tool-output-denied")
    data class ToolOutputDenied(
        @SerialName("tool_call_id") val toolCallId: String = "",
        @SerialName("tool_name") val toolName: String = "",
        @SerialName("provider_executed") val providerExecuted: Boolean? = null,
        val dynamic: Boolean? = null,
    ) : StreamPart

    /**
     * A file generated by the model (e.g. an image or document):
     * `{"type": "file", "file": GeneratedFile, "providerMetadata"?}`
     * (`stream-language-model-call.ts:569-590`).
     */
    @Serializable
    @SerialName("file")
    data class File(
        val file: GeneratedFile,
        val providerMetadata: ProviderMetadata? = null,
    ) : StreamPart

    /** A file generated as part of reasoning, with the same shape as [File]. */
    @Serializable
    @SerialName("reasoning-file")
    data class ReasoningFile(
        val file: GeneratedFile,
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

    /** The answer to a tool approval request (Rust `ToolApprovalResponseOutput`). */
    @Serializable
    @SerialName("tool-approval-response")
    data class ToolApprovalResponse(
        @SerialName("approval_id") val approvalId: String,
        @SerialName("tool_call") val toolCall: ai.arcships.aimux.ToolCall,
        val approved: Boolean,
        val reason: String? = null,
        @SerialName("provider_executed") val providerExecuted: Boolean? = null,
    ) : StreamPart

    @Serializable
    @SerialName("reasoning-start")
    data class ReasoningStart(val id: String = "", val providerMetadata: ProviderMetadata? = null) : StreamPart

    @Serializable
    @SerialName("reasoning-delta")
    data class ReasoningDelta(
        val id: String = "",
        /** The delta, under `text` as in the AI SDK's TextStreamPart. */
        val text: String = "",
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
