/**
 * aimux — typed multimodal data structures mirroring the aimux-core wire format.
 *
 * Same shapes as the ts-rs generated `.ts` types in `bindings/node/src/types/` and
 * the Go structs in `bindings/go/multimodal_types.go`. Field names use camelCase
 * in Kotlin and map to the wire format's snake_case via [kotlinx.serialization.SerialName].
 *
 * These types are intentionally lenient on decode (unknown keys ignored, every
 * field has a default) so future engine additions don't break existing clients.
 * The serialization config lives in [AimuxJson]. Decode a JSON string returned
 * by a [Multimodal][aimux] model with, for example:
 *
 * ```kotlin
 * val result: EmbeddingResult = AimuxJson.decodeFromString(EmbeddingResult.serializer(), jsonStr)
 * ```
 *
 * The `Base64`/`Binary`/`Url` unions ([AudioData], [ImageOutputs], [VideoData])
 * are serde-style externally-tagged enums on the wire
 * (`{"Base64": ...}` / `{"Binary": ...}` / `{"Url": ...}`), so each has a custom
 * serializer (see [StreamPartSerializer] in `Types.kt` for the same pattern).
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
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonDecoder
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonEncoder
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

// ─────────────────────────────────────────────────────────────────────────────
// Shared types.
// ─────────────────────────────────────────────────────────────────────────────

/** A provider warning (arbitrary JSON). Mirrors Go's `Warning = json.RawMessage`. */
typealias Warning = JsonElement

// ─────────────────────────────────────────────────────────────────────────────
// Embedding
// ─────────────────────────────────────────────────────────────────────────────

/** Token usage for an embedding call (input tokens only). */
@Serializable
data class EmbeddingUsage(
    val tokens: Long? = null,
)

/** Provider response metadata for embeddings. */
@Serializable
data class EmbeddingResponse(
    val headers: Map<String, String>? = null,
    val body: JsonElement? = null,
)

/** Result of an embedding call. */
@Serializable
data class EmbeddingResult(
    val embeddings: List<List<Float>> = emptyList(),
    val usage: EmbeddingUsage? = null,
    @SerialName("provider_metadata") val providerMetadata: JsonElement? = null,
    val response: EmbeddingResponse? = null,
    val warnings: List<Warning> = emptyList(),
)

/** Options for an embedding call. */
@Serializable
data class EmbeddingCallOptions(
    val values: List<String> = emptyList(),
    @SerialName("max_retries") val maxRetries: Long? = null,
    val timeout: TimeoutConfiguration? = null,
    @SerialName("provider_options") val providerOptions: JsonElement? = null,
    val headers: Map<String, String>? = null,
)

// ─────────────────────────────────────────────────────────────────────────────
// Speech (TTS)
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Generated audio: a base64 string or raw binary bytes.
 *
 * Wire format (serde externally-tagged): `{"Base64": "..."}` | `{"Binary": [n,...]}`.
 */
@Serializable(with = AudioDataSerializer::class)
sealed interface AudioData {
    /** Base64-encoded audio. */
    data class Base64(val value: String) : AudioData

    /** Raw binary audio bytes (each element is a 0–255 byte value). */
    data class Binary(val value: List<Int>) : AudioData
}

/** Request metadata for speech generation. */
@Serializable
data class SpeechRequest(
    val body: JsonElement? = null,
)

/** Provider response metadata for speech. */
@Serializable
data class SpeechResponse(
    val timestamp: String? = null,
    @SerialName("model_id") val modelId: String? = null,
    val headers: Map<String, String>? = null,
    val body: JsonElement? = null,
)

/** Result of a speech generation call. */
@Serializable
data class SpeechResult(
    val audio: AudioData = AudioData.Base64(""),
    val warnings: List<Warning> = emptyList(),
    val request: SpeechRequest? = null,
    val response: SpeechResponse = SpeechResponse(),
    @SerialName("provider_metadata") val providerMetadata: JsonElement? = null,
)

/** Options for speech generation. */
@Serializable
data class SpeechCallOptions(
    val text: String = "",
    val voice: String? = null,
    @SerialName("output_format") val outputFormat: String? = null,
    val instructions: String? = null,
    val speed: Double? = null,
    val language: String? = null,
    @SerialName("max_retries") val maxRetries: Long? = null,
    val timeout: TimeoutConfiguration? = null,
    @SerialName("provider_options") val providerOptions: JsonElement? = null,
    val headers: Map<String, String>? = null,
)

// ─────────────────────────────────────────────────────────────────────────────
// Image
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Generated images: all base64 strings or all binary byte arrays.
 *
 * Wire format (serde externally-tagged):
 * `{"Base64": ["...", ...]}` | `{"Binary": [[n,...], ...]}`.
 */
@Serializable(with = ImageOutputsSerializer::class)
sealed interface ImageOutputs {
    /** Base64-encoded images. */
    data class Base64(val value: List<String>) : ImageOutputs

    /** Raw binary images (each element is a list of 0–255 byte values). */
    data class Binary(val value: List<List<Int>>) : ImageOutputs
}

/** Token usage for image generation (if reported). */
@Serializable
data class ImageUsage(
    @SerialName("input_tokens") val inputTokens: Long? = null,
    @SerialName("output_tokens") val outputTokens: Long? = null,
    @SerialName("total_tokens") val totalTokens: Long? = null,
)

/** Provider response metadata for images. */
@Serializable
data class ImageResponse(
    val timestamp: String? = null,
    @SerialName("model_id") val modelId: String? = null,
    val headers: Map<String, String>? = null,
)

/** Result of an image generation call. */
@Serializable
data class ImageResult(
    val images: ImageOutputs = ImageOutputs.Base64(emptyList()),
    val warnings: List<Warning> = emptyList(),
    @SerialName("provider_metadata") val providerMetadata: JsonElement? = null,
    val response: ImageResponse = ImageResponse(),
    val usage: ImageUsage? = null,
)

/** Options for image generation. */
@Serializable
data class ImageCallOptions(
    val prompt: String? = null,
    val n: Int? = null,
    val size: String? = null,
    @SerialName("aspect_ratio") val aspectRatio: String? = null,
    val seed: Long? = null,
    val files: List<JsonElement> = emptyList(),
    val mask: JsonElement? = null,
    @SerialName("max_retries") val maxRetries: Long? = null,
    val timeout: TimeoutConfiguration? = null,
    @SerialName("provider_options") val providerOptions: JsonElement? = null,
    val headers: Map<String, String>? = null,
)

// ─────────────────────────────────────────────────────────────────────────────
// Transcription (STT)
// ─────────────────────────────────────────────────────────────────────────────

/** A transcript segment with timing. */
@Serializable
data class TranscriptionSegment(
    // Required f64 in the core (no Option) — see TranscriptionSegment.ts — so
    // non-nullable here; the defaults follow the file's lenient-decode contract.
    // Decode-only in practice: segments arrive from the core, which always emits
    // all three fields, and nothing here sends them back.
    val text: String = "",
    @SerialName("start_second") val startSecond: Double = 0.0,
    @SerialName("end_second") val endSecond: Double = 0.0,
)

/** Request metadata for transcription. */
@Serializable
data class TranscriptionRequest(
    @SerialName("body") val body: String? = null,
)

/** Provider response metadata for transcription. */
@Serializable
data class TranscriptionResponse(
    val timestamp: String? = null,
    @SerialName("model_id") val modelId: String? = null,
    val headers: Map<String, String>? = null,
    val body: JsonElement? = null,
)

/** Result of a transcription call. */
@Serializable
data class TranscriptionResult(
    val text: String = "",
    val segments: List<TranscriptionSegment> = emptyList(),
    val language: String? = null,
    @SerialName("duration_in_seconds") val durationInSeconds: Double? = null,
    val warnings: List<Warning> = emptyList(),
    val request: TranscriptionRequest? = null,
    val response: TranscriptionResponse = TranscriptionResponse(),
    @SerialName("provider_metadata") val providerMetadata: JsonElement? = null,
)

/** Options for transcription. */
@Serializable
data class TranscriptionCallOptions(
    val audio: JsonElement = JsonObject(emptyMap()),
    @SerialName("media_type") val mediaType: String = "",
    @SerialName("max_retries") val maxRetries: Long? = null,
    val timeout: TimeoutConfiguration? = null,
    @SerialName("provider_options") val providerOptions: JsonElement? = null,
    val headers: Map<String, String>? = null,
)

// ─────────────────────────────────────────────────────────────────────────────
// Reranking
// ─────────────────────────────────────────────────────────────────────────────

/** A single reranked entry. */
@Serializable
data class RerankingRank(
    val index: Int = 0,
    @SerialName("relevance_score") val relevanceScore: Double = 0.0,
)

/** Provider response metadata for reranking. */
@Serializable
data class RerankingResponse(
    val id: String? = null,
    val timestamp: String? = null,
    @SerialName("model_id") val modelId: String? = null,
    val headers: Map<String, String>? = null,
    val body: JsonElement? = null,
)

/** Result of a reranking call. */
@Serializable
data class RerankingResult(
    val ranking: List<RerankingRank> = emptyList(),
    @SerialName("provider_metadata") val providerMetadata: JsonElement? = null,
    val warnings: List<Warning> = emptyList(),
    val response: RerankingResponse? = null,
)

/** Options for reranking. */
@Serializable
data class RerankingCallOptions(
    val documents: JsonElement = JsonArray(emptyList()),
    val query: String = "",
    @SerialName("top_n") val topN: Int? = null,
    @SerialName("max_retries") val maxRetries: Long? = null,
    val timeout: TimeoutConfiguration? = null,
    @SerialName("provider_options") val providerOptions: JsonElement? = null,
    val headers: Map<String, String>? = null,
)

// ─────────────────────────────────────────────────────────────────────────────
// Video
// ─────────────────────────────────────────────────────────────────────────────

/** The URL variant of [VideoData]. */
@Serializable
data class VideoUrlData(
    val url: String = "",
    @SerialName("media_type") val mediaType: String = "",
)

/** The base64 variant of [VideoData]. */
@Serializable
data class VideoBase64Data(
    val data: String = "",
    @SerialName("media_type") val mediaType: String = "",
)

/** The binary variant of [VideoData]. */
@Serializable
data class VideoBinaryData(
    val data: List<Int> = emptyList(),
    @SerialName("media_type") val mediaType: String = "",
)

/**
 * Generated video: a URL, base64 string, or raw binary bytes.
 *
 * Wire format (serde externally-tagged):
 * `{"Url": {...}}` | `{"Base64": {...}}` | `{"Binary": {...}}`.
 */
@Serializable(with = VideoDataSerializer::class)
sealed interface VideoData {
    /** A URL pointing at the generated video. */
    data class Url(val value: VideoUrlData) : VideoData

    /** Base64-encoded video. */
    data class Base64(val value: VideoBase64Data) : VideoData

    /** Raw binary video bytes (each element is a 0–255 byte value). */
    data class Binary(val value: VideoBinaryData) : VideoData
}

/** Provider response metadata for video. */
@Serializable
data class VideoResponse(
    val timestamp: String? = null,
    @SerialName("model_id") val modelId: String? = null,
    val headers: Map<String, String>? = null,
)

/** Result of a video generation call. */
@Serializable
data class VideoResult(
    val videos: List<VideoData> = emptyList(),
    val warnings: List<Warning> = emptyList(),
    @SerialName("provider_metadata") val providerMetadata: JsonElement? = null,
    val response: VideoResponse = VideoResponse(),
)

/** Per-call pacing overrides for the Core-owned video status poll loop. */
@Serializable
data class VideoPollOptions(
    @SerialName("interval_ms") val intervalMs: Long? = null,
    @SerialName("timeout_ms") val timeoutMs: Long? = null,
)

/** Options for video generation. */
@Serializable
data class VideoCallOptions(
    val prompt: String? = null,
    val n: Int? = null,
    @SerialName("aspect_ratio") val aspectRatio: String? = null,
    val resolution: String? = null,
    val seed: Long? = null,
    @SerialName("max_retries") val maxRetries: Long? = null,
    val poll: VideoPollOptions? = null,
    val timeout: TimeoutConfiguration? = null,
    @SerialName("provider_options") val providerOptions: JsonElement? = null,
    val headers: Map<String, String>? = null,
)

// ─────────────────────────────────────────────────────────────────────────────
// Search
// ─────────────────────────────────────────────────────────────────────────────

/** A single search result. */
@Serializable
data class SearchResultItem(
    val title: String? = null,
    val url: String? = null,
    val content: String? = null,
    @SerialName("raw_content") val rawContent: String? = null,
    val score: Double? = null,
)

/** Provider response metadata for search. */
@Serializable
data class SearchResponse(
    val headers: Map<String, String>? = null,
    val body: JsonElement? = null,
)

/** Result of a search call. */
@Serializable
data class SearchResult(
    val results: List<SearchResultItem> = emptyList(),
    val answer: String? = null,
    @SerialName("provider_metadata") val providerMetadata: JsonElement? = null,
    val warnings: List<Warning> = emptyList(),
    val response: SearchResponse? = null,
)

/** Options for a search call. */
@Serializable
data class SearchCallOptions(
    val query: String = "",
    @SerialName("max_results") val maxResults: Int? = null,
    @SerialName("include_raw_content") val includeRawContent: Boolean? = null,
    @SerialName("time_range") val timeRange: String? = null,
    @SerialName("include_domains") val includeDomains: List<String> = emptyList(),
    @SerialName("exclude_domains") val excludeDomains: List<String> = emptyList(),
    @SerialName("max_retries") val maxRetries: Long? = null,
    val timeout: TimeoutConfiguration? = null,
    @SerialName("provider_options") val providerOptions: JsonElement? = null,
    val headers: Map<String, String>? = null,
)

// ─────────────────────────────────────────────────────────────────────────────
// Files
// ─────────────────────────────────────────────────────────────────────────────

/** Result of a file upload. */
@Serializable
data class UploadFileResult(
    @SerialName("provider_reference") val providerReference: Map<String, String> = emptyMap(),
    @SerialName("media_type") val mediaType: String? = null,
    val filename: String? = null,
    @SerialName("provider_metadata") val providerMetadata: JsonElement? = null,
    val warnings: List<Warning> = emptyList(),
)

/** Options for a file upload. */
@Serializable
data class UploadFileCallOptions(
    val data: JsonElement = JsonObject(emptyMap()),
    @SerialName("media_type") val mediaType: String = "",
    val filename: String? = null,
    @SerialName("provider_options") val providerOptions: JsonElement? = null,
)

// ─────────────────────────────────────────────────────────────────────────────
// Custom serializers for the serde-style externally-tagged unions.
// (Same approach as [StreamPartSerializer] in `Types.kt`.)
// ─────────────────────────────────────────────────────────────────────────────

/** (De)serializer for [AudioData]: `{"Base64": "..."}` | `{"Binary": [n,...]}`. */
object AudioDataSerializer : KSerializer<AudioData> {
    override val descriptor: SerialDescriptor = buildClassSerialDescriptor("aimux.AudioData")

    override fun deserialize(decoder: Decoder): AudioData {
        val json = decoder as? JsonDecoder
            ?: throw SerializationException("AudioData can only be decoded from JSON")
        val obj = json.decodeJsonElement().jsonObject
        require(obj.size == 1) { "AudioData must be a single-key object, got: $obj" }
        val (tag, inner) = obj.entries.single()
        return when (tag) {
            "Base64" -> AudioData.Base64(inner.jsonPrimitive.content)
            "Binary" -> AudioData.Binary(inner.jsonArray.map { it.jsonPrimitive.content.toInt() })
            else -> throw SerializationException("Unknown AudioData variant: '$tag'")
        }
    }

    override fun serialize(encoder: Encoder, value: AudioData) {
        val json = encoder as? JsonEncoder
            ?: throw SerializationException("AudioData can only be encoded to JSON")
        val (tag, inner) = when (value) {
            is AudioData.Base64 -> "Base64" to JsonPrimitive(value.value)
            is AudioData.Binary -> "Binary" to JsonArray(value.value.map { JsonPrimitive(it) })
        }
        json.encodeJsonElement(JsonObject(mapOf(tag to inner)))
    }
}

/** (De)serializer for [ImageOutputs]: `{"Base64": [...]}` | `{"Binary": [[...],...]}`. */
object ImageOutputsSerializer : KSerializer<ImageOutputs> {
    override val descriptor: SerialDescriptor = buildClassSerialDescriptor("aimux.ImageOutputs")

    override fun deserialize(decoder: Decoder): ImageOutputs {
        val json = decoder as? JsonDecoder
            ?: throw SerializationException("ImageOutputs can only be decoded from JSON")
        val obj = json.decodeJsonElement().jsonObject
        require(obj.size == 1) { "ImageOutputs must be a single-key object, got: $obj" }
        val (tag, inner) = obj.entries.single()
        return when (tag) {
            "Base64" -> ImageOutputs.Base64(inner.jsonArray.map { it.jsonPrimitive.content })
            "Binary" -> ImageOutputs.Binary(
                inner.jsonArray.map { row -> row.jsonArray.map { it.jsonPrimitive.content.toInt() } },
            )
            else -> throw SerializationException("Unknown ImageOutputs variant: '$tag'")
        }
    }

    override fun serialize(encoder: Encoder, value: ImageOutputs) {
        val json = encoder as? JsonEncoder
            ?: throw SerializationException("ImageOutputs can only be encoded to JSON")
        val (tag, inner) = when (value) {
            is ImageOutputs.Base64 -> "Base64" to JsonArray(value.value.map { JsonPrimitive(it) })
            is ImageOutputs.Binary -> "Binary" to JsonArray(value.value.map { row -> JsonArray(row.map { JsonPrimitive(it) }) })
        }
        json.encodeJsonElement(JsonObject(mapOf(tag to inner)))
    }
}

/** (De)serializer for [VideoData]: `{"Url": {...}}` | `{"Base64": {...}}` | `{"Binary": {...}}`. */
object VideoDataSerializer : KSerializer<VideoData> {
    override val descriptor: SerialDescriptor = buildClassSerialDescriptor("aimux.VideoData")

    override fun deserialize(decoder: Decoder): VideoData {
        val json = decoder as? JsonDecoder
            ?: throw SerializationException("VideoData can only be decoded from JSON")
        val obj = json.decodeJsonElement().jsonObject
        require(obj.size == 1) { "VideoData must be a single-key object, got: $obj" }
        val (tag, inner) = obj.entries.single()
        val innerObj = inner as? JsonObject ?: JsonObject(emptyMap())
        val ctx = json.json
        return when (tag) {
            "Url" -> VideoData.Url(ctx.decodeFromJsonElement(VideoUrlData.serializer(), innerObj))
            "Base64" -> VideoData.Base64(ctx.decodeFromJsonElement(VideoBase64Data.serializer(), innerObj))
            "Binary" -> VideoData.Binary(ctx.decodeFromJsonElement(VideoBinaryData.serializer(), innerObj))
            else -> throw SerializationException("Unknown VideoData variant: '$tag'")
        }
    }

    override fun serialize(encoder: Encoder, value: VideoData) {
        val json = encoder as? JsonEncoder
            ?: throw SerializationException("VideoData can only be encoded to JSON")
        val ctx = json.json
        val (tag, inner) = when (value) {
            is VideoData.Url -> "Url" to ctx.encodeToJsonElement(VideoUrlData.serializer(), value.value)
            is VideoData.Base64 -> "Base64" to ctx.encodeToJsonElement(VideoBase64Data.serializer(), value.value)
            is VideoData.Binary -> "Binary" to ctx.encodeToJsonElement(VideoBinaryData.serializer(), value.value)
        }
        json.encodeJsonElement(JsonObject(mapOf(tag to inner)))
    }
}
