/**
 * aimux — typed multimodal data structures mirroring the aimux-core wire format.
 *
 * Same shapes as the ts-rs generated `.ts` types in `bindings/node/src/types/`:
 * camelCase field names (Kotlin property names equal the wire names), unions
 * tagged on `type`, optional fields absent rather than null.
 *
 * Decode is lenient (unknown keys ignored, every result field has a default)
 * so future engine additions don't break existing clients. The serialization
 * config lives in [AimuxJson]. Decode a JSON string returned by a
 * [Multimodal][aimux] model with, for example:
 *
 * ```kotlin
 * val result: EmbeddingResult = AimuxJson.decodeFromString(EmbeddingResult.serializer(), jsonStr)
 * ```
 *
 * [AudioData] and [ImageOutputs] are untagged unions on the wire (a string is
 * base64, an array is binary), so each has a small custom serializer; the
 * `Array<number> | string` input fields reuse [FileBytes].
 */

package ai.arcships.aimux

import kotlinx.serialization.EncodeDefault
import kotlinx.serialization.ExperimentalSerializationApi
import kotlinx.serialization.KSerializer
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.SerializationException
import kotlinx.serialization.descriptors.PrimitiveKind
import kotlinx.serialization.descriptors.PrimitiveSerialDescriptor
import kotlinx.serialization.descriptors.SerialDescriptor
import kotlinx.serialization.descriptors.buildClassSerialDescriptor
import kotlinx.serialization.encoding.Decoder
import kotlinx.serialization.encoding.Encoder
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonDecoder
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonEncoder
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonPrimitive

// ─────────────────────────────────────────────────────────────────────────────
// Shared types.
// ─────────────────────────────────────────────────────────────────────────────

/** A pixel size. Mirrors `Size.ts`: serialized as the string `"WxH"` (e.g. `"1024x1024"`). */
@Serializable(with = SizeSerializer::class)
data class Size(val width: Int, val height: Int)

/** An aspect ratio. Mirrors `AspectRatio.ts`: serialized as the string `"W:H"` (e.g. `"16:9"`). */
@Serializable(with = AspectRatioSerializer::class)
data class AspectRatio(val width: Int, val height: Int)

private fun parsePair(kind: String, sep: Char, text: String): Pair<Int, Int> {
    val parts = text.split(sep)
    val w = parts.getOrNull(0)?.toIntOrNull()
    val h = parts.getOrNull(1)?.toIntOrNull()
    if (parts.size != 2 || w == null || h == null) {
        throw SerializationException("$kind must look like \"W${sep}H\", got: \"$text\"")
    }
    return w to h
}

object SizeSerializer : KSerializer<Size> {
    override val descriptor: SerialDescriptor = PrimitiveSerialDescriptor("aimux.Size", PrimitiveKind.STRING)
    override fun deserialize(decoder: Decoder): Size =
        parsePair("Size", 'x', decoder.decodeString()).let { Size(it.first, it.second) }
    override fun serialize(encoder: Encoder, value: Size) = encoder.encodeString("${value.width}x${value.height}")
}

object AspectRatioSerializer : KSerializer<AspectRatio> {
    override val descriptor: SerialDescriptor = PrimitiveSerialDescriptor("aimux.AspectRatio", PrimitiveKind.STRING)
    override fun deserialize(decoder: Decoder): AspectRatio =
        parsePair("AspectRatio", ':', decoder.decodeString()).let { AspectRatio(it.first, it.second) }
    override fun serialize(encoder: Encoder, value: AspectRatio) = encoder.encodeString("${value.width}:${value.height}")
}

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
    val providerMetadata: ProviderMetadata? = null,
    val response: EmbeddingResponse? = null,
    val warnings: List<Warning> = emptyList(),
)

/** Options for an embedding call. */
@Serializable
data class EmbeddingCallOptions(
    val values: List<String> = emptyList(),
    val maxRetries: Long? = null,
    val timeout: TimeoutConfiguration? = null,
    val providerOptions: ProviderOptions? = null,
    val headers: Map<String, String>? = null,
)

// ─────────────────────────────────────────────────────────────────────────────
// Speech (TTS)
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Generated audio: a base64 string or raw binary bytes.
 *
 * Wire format (untagged, `AudioData.ts`): `"..."` (base64) | `[n,...]` (binary).
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
    val modelId: String? = null,
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
    val providerMetadata: ProviderMetadata? = null,
)

/** Options for speech generation. */
@Serializable
data class SpeechCallOptions(
    val text: String = "",
    val voice: String? = null,
    val outputFormat: String? = null,
    val instructions: String? = null,
    val speed: Double? = null,
    val language: String? = null,
    val maxRetries: Long? = null,
    val timeout: TimeoutConfiguration? = null,
    val providerOptions: ProviderOptions? = null,
    val headers: Map<String, String>? = null,
)

// ─────────────────────────────────────────────────────────────────────────────
// Image
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Generated images: all base64 strings or all binary byte arrays.
 *
 * Wire format (untagged, `ImageOutputs.ts`): `["...", ...]` | `[[n,...], ...]`.
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
    val inputTokens: Long? = null,
    val outputTokens: Long? = null,
    val totalTokens: Long? = null,
)

/** Provider response metadata for images. */
@Serializable
data class ImageResponse(
    val timestamp: String? = null,
    val modelId: String? = null,
    val headers: Map<String, String>? = null,
)

/** Result of an image generation call. */
@Serializable
data class ImageResult(
    val images: ImageOutputs = ImageOutputs.Base64(emptyList()),
    val warnings: List<Warning> = emptyList(),
    val providerMetadata: ProviderMetadata? = null,
    val response: ImageResponse = ImageResponse(),
    val usage: ImageUsage? = null,
)

/** An input image (edit source or mask), tagged on `type`. Mirrors `ImageFile.ts`. */
@Serializable
sealed interface ImageFile {
    @Serializable
    @SerialName("file")
    data class File(val mediaType: String, val data: FileBytes) : ImageFile

    @Serializable
    @SerialName("url")
    data class Url(val url: String) : ImageFile
}

/**
 * Options for image generation. `n` and `providerOptions` are required on the
 * wire, so they are always encoded (defaults: 1 image, no options).
 */
@OptIn(ExperimentalSerializationApi::class)
@Serializable
data class ImageCallOptions(
    val prompt: String? = null,
    @EncodeDefault val n: Int = 1,
    val size: Size? = null,
    val aspectRatio: AspectRatio? = null,
    val seed: Long? = null,
    val files: List<ImageFile>? = null,
    val mask: ImageFile? = null,
    @EncodeDefault val providerOptions: ProviderOptions = emptyMap(),
    val maxRetries: Long? = null,
    val timeout: TimeoutConfiguration? = null,
    val headers: Map<String, String>? = null,
)

// ─────────────────────────────────────────────────────────────────────────────
// Transcription (STT)
// ─────────────────────────────────────────────────────────────────────────────

/** A transcript segment with timing. */
@Serializable
data class TranscriptionSegment(
    val text: String = "",
    val startSecond: Double = 0.0,
    val endSecond: Double = 0.0,
)

/** Request metadata for transcription. */
@Serializable
data class TranscriptionRequest(
    val body: String? = null,
)

/** Provider response metadata for transcription. */
@Serializable
data class TranscriptionResponse(
    val timestamp: String? = null,
    val modelId: String? = null,
    val headers: Map<String, String>? = null,
    val body: JsonElement? = null,
)

/** Result of a transcription call. */
@Serializable
data class TranscriptionResult(
    val text: String = "",
    val segments: List<TranscriptionSegment> = emptyList(),
    val language: String? = null,
    val durationInSeconds: Double? = null,
    val warnings: List<Warning> = emptyList(),
    val request: TranscriptionRequest? = null,
    val response: TranscriptionResponse = TranscriptionResponse(),
    val providerMetadata: ProviderMetadata? = null,
)

/** Options for transcription. `audio` is raw bytes or a base64 string. */
@Serializable
data class TranscriptionCallOptions(
    val audio: FileBytes = FileBytes.Base64(""),
    val mediaType: String = "",
    val maxRetries: Long? = null,
    val timeout: TimeoutConfiguration? = null,
    val providerOptions: ProviderOptions? = null,
    val headers: Map<String, String>? = null,
)

// ─────────────────────────────────────────────────────────────────────────────
// Reranking
// ─────────────────────────────────────────────────────────────────────────────

/** A single reranked entry. */
@Serializable
data class RerankingRank(
    val index: Int = 0,
    val relevanceScore: Double = 0.0,
)

/** Provider response metadata for reranking. */
@Serializable
data class RerankingResponse(
    val id: String? = null,
    val timestamp: String? = null,
    val modelId: String? = null,
    val headers: Map<String, String>? = null,
    val body: JsonElement? = null,
)

/** Result of a reranking call. */
@Serializable
data class RerankingResult(
    val ranking: List<RerankingRank> = emptyList(),
    val providerMetadata: ProviderMetadata? = null,
    val warnings: List<Warning>? = null,
    val response: RerankingResponse? = null,
)

/** The documents to rerank, tagged on `type`. Mirrors `RerankingDocuments.ts`. */
@Serializable
sealed interface RerankingDocuments {
    @Serializable
    @SerialName("text")
    data class Text(val values: List<String>) : RerankingDocuments

    @Serializable
    @SerialName("object")
    data class Objects(val values: List<JsonElement>) : RerankingDocuments
}

/** Options for reranking. */
@Serializable
data class RerankingCallOptions(
    val documents: RerankingDocuments,
    val query: String = "",
    val topN: Int? = null,
    val maxRetries: Long? = null,
    val timeout: TimeoutConfiguration? = null,
    val providerOptions: ProviderOptions? = null,
    val headers: Map<String, String>? = null,
)

// ─────────────────────────────────────────────────────────────────────────────
// Video
// ─────────────────────────────────────────────────────────────────────────────

/** Generated video, tagged on `type`. Mirrors `VideoData.ts`. */
@Serializable
sealed interface VideoData {
    val mediaType: String

    /** A URL pointing at the generated video. */
    @Serializable
    @SerialName("url")
    data class Url(val url: String, override val mediaType: String) : VideoData

    /** Base64-encoded video. */
    @Serializable
    @SerialName("base64")
    data class Base64(val data: String, override val mediaType: String) : VideoData

    /** Raw binary video bytes (each element is a 0–255 byte value). */
    @Serializable
    @SerialName("binary")
    data class Binary(val data: List<Int>, override val mediaType: String) : VideoData
}

/** An input video/image, tagged on `type`. Mirrors `VideoFile.ts`. */
@Serializable
sealed interface VideoFile {
    @Serializable
    @SerialName("file")
    data class File(val mediaType: String, val data: FileBytes) : VideoFile

    @Serializable
    @SerialName("url")
    data class Url(val url: String, val mediaType: String? = null) : VideoFile
}

@Serializable
enum class VideoFrameType {
    @SerialName("first_frame") FIRST_FRAME,
    @SerialName("last_frame") LAST_FRAME,
}

/** An image pinned to a frame of the video. Mirrors `VideoFrameImage.ts`. */
@Serializable
data class VideoFrameImage(val image: VideoFile, val frameType: VideoFrameType)

/** Provider response metadata for video. */
@Serializable
data class VideoResponse(
    val timestamp: String? = null,
    val modelId: String? = null,
    val headers: Map<String, String>? = null,
)

/** Result of a video generation call. */
@Serializable
data class VideoResult(
    val videos: List<VideoData> = emptyList(),
    val warnings: List<Warning> = emptyList(),
    val providerMetadata: ProviderMetadata? = null,
    val response: VideoResponse = VideoResponse(),
)

/** Per-call pacing overrides for the Core-owned video status poll loop. */
@Serializable
data class VideoPollOptions(
    val intervalMs: Long? = null,
    val timeoutMs: Long? = null,
)

/** Options for video generation. `n` and `providerOptions` are always encoded. */
@OptIn(ExperimentalSerializationApi::class)
@Serializable
data class VideoCallOptions(
    val prompt: String? = null,
    @EncodeDefault val n: Int = 1,
    val aspectRatio: AspectRatio? = null,
    val resolution: Size? = null,
    val duration: Double? = null,
    val fps: Double? = null,
    val seed: Long? = null,
    val image: VideoFile? = null,
    val frameImages: List<VideoFrameImage>? = null,
    val inputReferences: List<VideoFile>? = null,
    val generateAudio: Boolean? = null,
    @EncodeDefault val providerOptions: ProviderOptions = emptyMap(),
    val maxRetries: Long? = null,
    val poll: VideoPollOptions? = null,
    val timeout: TimeoutConfiguration? = null,
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
    val rawContent: String? = null,
    val score: Double? = null,
    val providerMetadata: ProviderMetadata? = null,
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
    val providerMetadata: ProviderMetadata? = null,
    val warnings: List<Warning> = emptyList(),
    val response: SearchResponse? = null,
)

/** Options for a search call. */
@Serializable
data class SearchCallOptions(
    val query: String = "",
    val maxResults: Int? = null,
    val includeRawContent: Boolean? = null,
    val timeRange: String? = null,
    val includeDomains: List<String>? = null,
    val excludeDomains: List<String>? = null,
    val maxRetries: Long? = null,
    val timeout: TimeoutConfiguration? = null,
    val providerOptions: ProviderOptions? = null,
    val headers: Map<String, String>? = null,
)

// ─────────────────────────────────────────────────────────────────────────────
// Files
// ─────────────────────────────────────────────────────────────────────────────

/** Result of a file upload. */
@Serializable
data class UploadFileResult(
    val providerReference: Map<String, String> = emptyMap(),
    val mediaType: String? = null,
    val filename: String? = null,
    val providerMetadata: ProviderMetadata? = null,
    val warnings: List<Warning> = emptyList(),
)

/** Upload payload, tagged on `type`. Mirrors `UploadFileData.ts`. */
@Serializable
sealed interface UploadFileData {
    @Serializable
    @SerialName("data")
    data class Data(val data: FileBytes) : UploadFileData

    @Serializable
    @SerialName("text")
    data class Text(val text: String) : UploadFileData
}

/** Options for a file upload. */
@Serializable
data class UploadFileCallOptions(
    val data: UploadFileData = UploadFileData.Data(FileBytes.Base64("")),
    val mediaType: String = "",
    val filename: String? = null,
    val providerOptions: ProviderOptions? = null,
)

// ─────────────────────────────────────────────────────────────────────────────
// Custom serializers for the untagged unions.
// ─────────────────────────────────────────────────────────────────────────────

/** (De)serializer for [AudioData]: `"..."` | `[n,...]`. */
object AudioDataSerializer : KSerializer<AudioData> {
    override val descriptor: SerialDescriptor = buildClassSerialDescriptor("aimux.AudioData")

    override fun deserialize(decoder: Decoder): AudioData {
        val json = decoder as? JsonDecoder
            ?: throw SerializationException("AudioData can only be decoded from JSON")
        return when (val el = json.decodeJsonElement()) {
            is JsonArray -> AudioData.Binary(el.map { it.jsonPrimitive.content.toInt() })
            is JsonPrimitive -> AudioData.Base64(el.content)
            else -> throw SerializationException("AudioData must be a base64 string or a byte array, got: $el")
        }
    }

    override fun serialize(encoder: Encoder, value: AudioData) {
        val json = encoder as? JsonEncoder
            ?: throw SerializationException("AudioData can only be encoded to JSON")
        json.encodeJsonElement(
            when (value) {
                is AudioData.Base64 -> JsonPrimitive(value.value)
                is AudioData.Binary -> JsonArray(value.value.map { JsonPrimitive(it) })
            }
        )
    }
}

/** (De)serializer for [ImageOutputs]: `["...",...]` | `[[n,...],...]`. */
object ImageOutputsSerializer : KSerializer<ImageOutputs> {
    override val descriptor: SerialDescriptor = buildClassSerialDescriptor("aimux.ImageOutputs")

    override fun deserialize(decoder: Decoder): ImageOutputs {
        val json = decoder as? JsonDecoder
            ?: throw SerializationException("ImageOutputs can only be decoded from JSON")
        val items = json.decodeJsonElement().jsonArray
        // An empty list carries no evidence either way; treat it as base64.
        return if (items.firstOrNull() is JsonArray) {
            ImageOutputs.Binary(items.map { row -> row.jsonArray.map { it.jsonPrimitive.content.toInt() } })
        } else {
            ImageOutputs.Base64(items.map { it.jsonPrimitive.content })
        }
    }

    override fun serialize(encoder: Encoder, value: ImageOutputs) {
        val json = encoder as? JsonEncoder
            ?: throw SerializationException("ImageOutputs can only be encoded to JSON")
        json.encodeJsonElement(
            when (value) {
                is ImageOutputs.Base64 -> JsonArray(value.value.map { JsonPrimitive(it) })
                is ImageOutputs.Binary -> JsonArray(value.value.map { row -> JsonArray(row.map { JsonPrimitive(it) }) })
            }
        )
    }
}
