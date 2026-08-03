// Multimodal.swift — Swift wrapper around the aimux-ffi multimodal C ABI.
//
// Mirrors the Go binding (bindings/go/multimodal.go): eight modality model
// types (Embedding, Speech, Transcription, Image, Video, Reranking, Search,
// Files), each wrapping an opaque C handle. Handles are ARC-managed and
// released via `aimux_drop_handle` on deallocation, just like `Model` in
// Aimux.swift. Cross-boundary data uses JSON strings (base64 for binary),
// matching the C ABI wire format.
//
// The instance methods form the *raw* layer (JSON-string in, JSON-string out,
// consistent with `Model.generateText`). The Codable types below let callers
// decode those JSON strings into typed Swift values.

import CAimuxFFI
import Foundation

// ─────────────────────────────────────────────────────────────────────────────
// FFI helper
// ─────────────────────────────────────────────────────────────────────────────

/// Call a C ABI function that returns a `char*` JSON result, copy it into a
/// Swift `String`, free the C allocation, and surface a provider error if the
/// result is an `{"error":"..."}` envelope.
///
/// `body` performs the actual C call (capturing the handle and arguments) and
/// returns the owned `char*` (or `nil` on failure). The handle is checked here
/// so every call site shares the same invalid-handle guard.
private func ffiStringCall(
    handle: UInt64,
    body: () -> UnsafeMutablePointer<CChar>?
) throws -> String {
    guard handle != 0 else {
        throw AimuxError.invalidHandle
    }
    guard let ptr = body() else {
        throw AimuxError.serializationError("FFI call returned null")
    }

    let result = String(cString: ptr)
    aimux_free_string(ptr)

    // Check for an error envelope in the result JSON.
    if let data = result.data(using: .utf8),
       let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
       let error = json["error"] as? String {
        throw AimuxError.providerError(error)
    }

    return result
}

// ─────────────────────────────────────────────────────────────────────────────
// EmbeddingModel
// ─────────────────────────────────────────────────────────────────────────────

/// A model instance backed by a Rust `Arc<dyn EmbeddingModel>`.
///
/// The C handle is automatically released when this object is deallocated.
public final class EmbeddingModel: @unchecked Sendable {

    // The opaque handle from aimux-ffi. 0 means invalid/freed.
    private var handle: UInt64

    private init(handle: UInt64) {
        self.handle = handle
    }

    deinit {
        if handle != 0 {
            aimux_drop_handle(handle)
        }
    }

    // ── Provider constructors ──────────────────────────────────────────────

    /// Create an OpenAI embedding model instance (e.g. text-embedding-3-small).
    public static func openai(apiKey: String, modelId: String) throws -> EmbeddingModel {
        let handle = try Model.extractHandle(aimux_openai_embedding_new(apiKey, modelId))
        return EmbeddingModel(handle: handle)
    }

    /// Create an OpenAI embedding model instance with a custom base URL.
    public static func openai(apiKey: String, modelId: String, baseUrl: String) throws -> EmbeddingModel {
        let handle = try Model.extractHandle(aimux_openai_embedding_new_with_base(apiKey, modelId, baseUrl))
        return EmbeddingModel(handle: handle)
    }

    /// Create a Cohere embedding model instance (e.g. embed-english-v3.0).
    public static func cohere(apiKey: String, modelId: String) throws -> EmbeddingModel {
        let handle = try Model.extractHandle(aimux_cohere_embedding_new(apiKey, modelId))
        return EmbeddingModel(handle: handle)
    }

    /// Create a Cohere embedding model instance with a custom base URL.
    public static func cohere(apiKey: String, modelId: String, baseUrl: String) throws -> EmbeddingModel {
        let handle = try Model.extractHandle(aimux_cohere_embedding_new_with_base(apiKey, modelId, baseUrl))
        return EmbeddingModel(handle: handle)
    }

    /// Create a Google embedding model instance (e.g. gemini-embedding-001).
    public static func google(apiKey: String, modelId: String) throws -> EmbeddingModel {
        let handle = try Model.extractHandle(aimux_google_embedding_new(apiKey, modelId))
        return EmbeddingModel(handle: handle)
    }

    /// Create a Google embedding model instance with a custom base URL.
    public static func google(apiKey: String, modelId: String, baseUrl: String) throws -> EmbeddingModel {
        let handle = try Model.extractHandle(aimux_google_embedding_new_with_base(apiKey, modelId, baseUrl))
        return EmbeddingModel(handle: handle)
    }

    // ── Generation ─────────────────────────────────────────────────────────

    /// Generate embeddings for the given text values.
    ///
    /// - Parameters:
    ///   - values: A JSON array of strings to embed (e.g. `["hello","world"]`).
    ///   - options: Optional EmbeddingCallOptions serialized as JSON.
    /// - Returns: The JSON-serialized EmbeddingResult.
    public func embed(values: String, options: String? = nil) throws -> String {
        let h = handle
        return try ffiStringCall(handle: h) {
            aimux_embed(h, values, options)
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// SpeechModel (TTS)
// ─────────────────────────────────────────────────────────────────────────────

/// A model instance backed by a Rust `Arc<dyn SpeechModel>`.
///
/// The C handle is automatically released when this object is deallocated.
public final class SpeechModel: @unchecked Sendable {

    // The opaque handle from aimux-ffi. 0 means invalid/freed.
    private var handle: UInt64

    private init(handle: UInt64) {
        self.handle = handle
    }

    deinit {
        if handle != 0 {
            aimux_drop_handle(handle)
        }
    }

    // ── Provider constructors ──────────────────────────────────────────────

    /// Create an OpenAI speech (TTS) model instance.
    public static func openai(apiKey: String, modelId: String) throws -> SpeechModel {
        let handle = try Model.extractHandle(aimux_openai_speech_new(apiKey, modelId))
        return SpeechModel(handle: handle)
    }

    /// Create an OpenAI speech model instance with a custom base URL.
    public static func openai(apiKey: String, modelId: String, baseUrl: String) throws -> SpeechModel {
        let handle = try Model.extractHandle(aimux_openai_speech_new_with_base(apiKey, modelId, baseUrl))
        return SpeechModel(handle: handle)
    }

    // ── Generation ─────────────────────────────────────────────────────────

    /// Generate speech audio from the given options.
    ///
    /// - Parameter options: SpeechCallOptions serialized as JSON.
    /// - Returns: The JSON-serialized SpeechResult.
    public func generate(options: String? = nil) throws -> String {
        let h = handle
        return try ffiStringCall(handle: h) {
            aimux_speech_generate(h, options)
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// TranscriptionModel (STT)
// ─────────────────────────────────────────────────────────────────────────────

/// A model instance backed by a Rust `Arc<dyn TranscriptionModel>`.
///
/// The C handle is automatically released when this object is deallocated.
public final class TranscriptionModel: @unchecked Sendable {

    // The opaque handle from aimux-ffi. 0 means invalid/freed.
    private var handle: UInt64

    private init(handle: UInt64) {
        self.handle = handle
    }

    deinit {
        if handle != 0 {
            aimux_drop_handle(handle)
        }
    }

    // ── Provider constructors ──────────────────────────────────────────────

    /// Create an OpenAI transcription (STT) model instance.
    public static func openai(apiKey: String, modelId: String) throws -> TranscriptionModel {
        let handle = try Model.extractHandle(aimux_openai_transcription_new(apiKey, modelId))
        return TranscriptionModel(handle: handle)
    }

    /// Create an OpenAI transcription model instance with a custom base URL.
    public static func openai(apiKey: String, modelId: String, baseUrl: String) throws -> TranscriptionModel {
        let handle = try Model.extractHandle(aimux_openai_transcription_new_with_base(apiKey, modelId, baseUrl))
        return TranscriptionModel(handle: handle)
    }

    // ── Generation ─────────────────────────────────────────────────────────

    /// Transcribe audio (base64-encoded) to text.
    ///
    /// - Parameters:
    ///   - audioBase64: Base64-encoded audio bytes.
    ///   - mediaType: Media type of the audio (e.g. `audio/wav`).
    ///   - options: Optional TranscriptionCallOptions serialized as JSON.
    /// - Returns: The JSON-serialized TranscriptionResult.
    public func generate(audioBase64: String, mediaType: String, options: String? = nil) throws -> String {
        let h = handle
        return try ffiStringCall(handle: h) {
            aimux_transcription_generate(h, audioBase64, mediaType, options)
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// ImageModel
// ─────────────────────────────────────────────────────────────────────────────

/// A model instance backed by a Rust `Arc<dyn ImageModel>`.
///
/// The C handle is automatically released when this object is deallocated.
public final class ImageModel: @unchecked Sendable {

    // The opaque handle from aimux-ffi. 0 means invalid/freed.
    private var handle: UInt64

    private init(handle: UInt64) {
        self.handle = handle
    }

    deinit {
        if handle != 0 {
            aimux_drop_handle(handle)
        }
    }

    // ── Provider constructors ──────────────────────────────────────────────

    /// Create an OpenAI image model instance (e.g. dall-e-3).
    public static func openai(apiKey: String, modelId: String) throws -> ImageModel {
        let handle = try Model.extractHandle(aimux_openai_image_new(apiKey, modelId))
        return ImageModel(handle: handle)
    }

    /// Create an OpenAI image model instance with a custom base URL.
    public static func openai(apiKey: String, modelId: String, baseUrl: String) throws -> ImageModel {
        let handle = try Model.extractHandle(aimux_openai_image_new_with_base(apiKey, modelId, baseUrl))
        return ImageModel(handle: handle)
    }

    /// Create a Google image model instance (e.g. gemini-2.5-flash-image).
    public static func google(apiKey: String, modelId: String) throws -> ImageModel {
        let handle = try Model.extractHandle(aimux_google_image_new(apiKey, modelId))
        return ImageModel(handle: handle)
    }

    /// Create a Google image model instance with a custom base URL.
    public static func google(apiKey: String, modelId: String, baseUrl: String) throws -> ImageModel {
        let handle = try Model.extractHandle(aimux_google_image_new_with_base(apiKey, modelId, baseUrl))
        return ImageModel(handle: handle)
    }

    // ── Generation ─────────────────────────────────────────────────────────

    /// Generate images from the given options.
    ///
    /// - Parameter options: ImageCallOptions serialized as JSON.
    /// - Returns: The JSON-serialized ImageResult.
    public func generate(options: String? = nil) throws -> String {
        let h = handle
        return try ffiStringCall(handle: h) {
            aimux_image_generate(h, options)
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// VideoModel
// ─────────────────────────────────────────────────────────────────────────────

/// A model instance backed by a Rust `Arc<dyn VideoModel>`.
///
/// The C handle is automatically released when this object is deallocated.
public final class VideoModel: @unchecked Sendable {

    // The opaque handle from aimux-ffi. 0 means invalid/freed.
    private var handle: UInt64

    private init(handle: UInt64) {
        self.handle = handle
    }

    deinit {
        if handle != 0 {
            aimux_drop_handle(handle)
        }
    }

    // ── Provider constructors ──────────────────────────────────────────────

    /// Create a Google video model instance (e.g. veo-3.0).
    public static func google(apiKey: String, modelId: String) throws -> VideoModel {
        let handle = try Model.extractHandle(aimux_google_video_new(apiKey, modelId))
        return VideoModel(handle: handle)
    }

    /// Create a Google video model instance with a custom base URL.
    public static func google(apiKey: String, modelId: String, baseUrl: String) throws -> VideoModel {
        let handle = try Model.extractHandle(aimux_google_video_new_with_base(apiKey, modelId, baseUrl))
        return VideoModel(handle: handle)
    }

    // ── Generation ─────────────────────────────────────────────────────────

    /// Generate videos from the given options.
    ///
    /// - Parameter options: VideoCallOptions serialized as JSON.
    /// - Returns: The JSON-serialized VideoResult.
    public func generate(options: String? = nil) throws -> String {
        let h = handle
        return try ffiStringCall(handle: h) {
            aimux_video_generate(h, options)
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// RerankingModel
// ─────────────────────────────────────────────────────────────────────────────

/// A model instance backed by a Rust `Arc<dyn RerankingModel>`.
///
/// The C handle is automatically released when this object is deallocated.
public final class RerankingModel: @unchecked Sendable {

    // The opaque handle from aimux-ffi. 0 means invalid/freed.
    private var handle: UInt64

    private init(handle: UInt64) {
        self.handle = handle
    }

    deinit {
        if handle != 0 {
            aimux_drop_handle(handle)
        }
    }

    // ── Provider constructors ──────────────────────────────────────────────

    /// Create a Cohere reranking model instance (e.g. rerank-v3.0).
    public static func cohere(apiKey: String, modelId: String) throws -> RerankingModel {
        let handle = try Model.extractHandle(aimux_cohere_reranking_new(apiKey, modelId))
        return RerankingModel(handle: handle)
    }

    /// Create a Cohere reranking model instance with a custom base URL.
    public static func cohere(apiKey: String, modelId: String, baseUrl: String) throws -> RerankingModel {
        let handle = try Model.extractHandle(aimux_cohere_reranking_new_with_base(apiKey, modelId, baseUrl))
        return RerankingModel(handle: handle)
    }

    // ── Generation ─────────────────────────────────────────────────────────

    /// Rerank documents by relevance to a query.
    ///
    /// - Parameter options: RerankingCallOptions serialized as JSON.
    /// - Returns: The JSON-serialized RerankingResult.
    public func rerank(options: String? = nil) throws -> String {
        let h = handle
        return try ffiStringCall(handle: h) {
            aimux_rerank(h, options)
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// SearchModel
// ─────────────────────────────────────────────────────────────────────────────

/// A model instance backed by a Rust `Arc<dyn SearchModel>`.
///
/// The C handle is automatically released when this object is deallocated.
public final class SearchModel: @unchecked Sendable {

    // The opaque handle from aimux-ffi. 0 means invalid/freed.
    private var handle: UInt64

    private init(handle: UInt64) {
        self.handle = handle
    }

    deinit {
        if handle != 0 {
            aimux_drop_handle(handle)
        }
    }

    // ── Provider constructors ──────────────────────────────────────────────

    /// Create a Tavily search model instance. Tavily uses a fixed endpoint, so
    /// no model ID is needed (the C ABI's `model_id` argument is ignored).
    public static func tavily(apiKey: String) throws -> SearchModel {
        let handle = try Model.extractHandle(aimux_tavily_search_new(apiKey, ""))
        return SearchModel(handle: handle)
    }

    /// Create a Tavily search model instance with a custom base URL (useful for
    /// testing against a mock server).
    public static func tavily(apiKey: String, baseUrl: String) throws -> SearchModel {
        let handle = try Model.extractHandle(aimux_tavily_search_new_with_base(apiKey, "", baseUrl))
        return SearchModel(handle: handle)
    }

    // ── Generation ─────────────────────────────────────────────────────────

    /// Perform a web search.
    ///
    /// - Parameter options: SearchCallOptions serialized as JSON.
    /// - Returns: The JSON-serialized SearchResult.
    public func search(options: String? = nil) throws -> String {
        let h = handle
        return try ffiStringCall(handle: h) {
            aimux_search(h, options)
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Files
// ─────────────────────────────────────────────────────────────────────────────

/// A file-upload manager backed by a Rust `Arc<dyn Files>`.
///
/// The C handle is automatically released when this object is deallocated.
public final class Files: @unchecked Sendable {

    // The opaque handle from aimux-ffi. 0 means invalid/freed.
    private var handle: UInt64

    private init(handle: UInt64) {
        self.handle = handle
    }

    deinit {
        if handle != 0 {
            aimux_drop_handle(handle)
        }
    }

    // ── Provider constructors ──────────────────────────────────────────────

    /// Create an OpenAI files manager instance. No model ID is required.
    public static func openai(apiKey: String) throws -> Files {
        let handle = try Model.extractHandle(aimux_openai_files_new(apiKey))
        return Files(handle: handle)
    }

    /// Create an OpenAI files manager instance with a custom base URL.
    public static func openai(apiKey: String, baseUrl: String) throws -> Files {
        let handle = try Model.extractHandle(aimux_openai_files_new_with_base(apiKey, baseUrl))
        return Files(handle: handle)
    }

    // ── Upload ─────────────────────────────────────────────────────────────

    /// Upload a file (base64-encoded) to the provider.
    ///
    /// - Parameters:
    ///   - dataBase64: Base64-encoded file bytes.
    ///   - mediaType: Media type of the file (e.g. `application/pdf`).
    ///   - options: Optional UploadFileCallOptions serialized as JSON.
    /// - Returns: The JSON-serialized UploadFileResult.
    public func upload(dataBase64: String, mediaType: String, options: String? = nil) throws -> String {
        let h = handle
        return try ffiStringCall(handle: h) {
            aimux_file_upload(h, dataBase64, mediaType, options)
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Multimodal result types (Codable)
//
// These mirror the Go structs in bindings/go/multimodal_types.go. Field names
// are snake_case on the wire; Swift camelCase is restored via CodingKeys (the
// same approach used in Types.swift). `provider_metadata` uses the existing
// `JSONValue` type; `warnings` is decoded leniently as `[JSONValue]` (matching
// Go's `json.RawMessage`), so unknown warning shapes don't break decoding.
// ─────────────────────────────────────────────────────────────────────────────

/// A `CodingKey` accepting any string, used to decode/encode the externally
/// tagged multimodal enums (`AudioData`, `ImageOutputs`, `VideoData`). Mirrors
/// `AnyCodingKey` in Types.swift, which is file-private and unavailable here.
fileprivate struct MMCodingKey: CodingKey {
    let stringValue: String
    init?(stringValue: String) { self.stringValue = stringValue }
    var intValue: Int? { nil }
    init?(intValue: Int) { return nil }
    init(_ s: String) { self.stringValue = s }
}

/// Build a `DecodingError.dataCorrupted` for a coding path + message.
fileprivate func mmDecodingError(_ codingPath: [any CodingKey], _ message: String) -> DecodingError {
    DecodingError.dataCorrupted(.init(codingPath: codingPath, debugDescription: message, underlyingError: nil))
}

// MARK: - Embedding

/// Token usage for an embedding call (input tokens only).
public struct EmbeddingUsage: Codable, Equatable {
    public var tokens: UInt32?

    public init(tokens: UInt32? = nil) {
        self.tokens = tokens
    }
}

/// Provider response metadata for embeddings.
public struct EmbeddingResponse: Codable, Equatable {
    public var id: String?
    public var modelId: String?
    public var timestamp: String?

    enum CodingKeys: String, CodingKey {
        case id
        case modelId = "model_id"
        case timestamp
    }

    public init(id: String? = nil, modelId: String? = nil, timestamp: String? = nil) {
        self.id = id; self.modelId = modelId; self.timestamp = timestamp
    }
}

/// The result of an embedding call.
public struct EmbeddingResult: Codable, Equatable {
    public var embeddings: [[Float]]
    public var usage: EmbeddingUsage?
    public var response: EmbeddingResponse?
    public var providerMetadata: JSONValue?
    public var warnings: [JSONValue]?

    enum CodingKeys: String, CodingKey {
        case embeddings, usage, response
        case providerMetadata = "provider_metadata"
        case warnings
    }

    public init(embeddings: [[Float]], usage: EmbeddingUsage? = nil, response: EmbeddingResponse? = nil,
                providerMetadata: JSONValue? = nil, warnings: [JSONValue]? = nil) {
        self.embeddings = embeddings; self.usage = usage; self.response = response
        self.providerMetadata = providerMetadata; self.warnings = warnings
    }
}

// MARK: - Speech (TTS)

/// Generated audio: a base64-encoded string or raw binary bytes.
/// Wire: `{"Base64":"…"}` | `{"Binary":[…]}`.
public enum AudioData: Codable, Equatable {
    case base64(String)
    case binary([UInt8])

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: MMCodingKey.self)
        guard let key = c.allKeys.first?.stringValue else {
            throw mmDecodingError(c.codingPath, "missing audio-data tag")
        }
        switch key {
        case "Base64":
            self = .base64(try c.decode(String.self, forKey: MMCodingKey(key)))
        case "Binary":
            self = .binary(try c.decode([UInt8].self, forKey: MMCodingKey(key)))
        default:
            throw mmDecodingError(c.codingPath, "unknown audio-data tag \(key)")
        }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: MMCodingKey.self)
        switch self {
        case .base64(let s):
            try c.encode(s, forKey: MMCodingKey("Base64"))
        case .binary(let bytes):
            try c.encode(bytes, forKey: MMCodingKey("Binary"))
        }
    }
}

/// The result of a speech generation call.
public struct SpeechResult: Codable, Equatable {
    public var audio: AudioData
    public var providerMetadata: JSONValue?
    public var warnings: [JSONValue]?

    enum CodingKeys: String, CodingKey {
        case audio
        case providerMetadata = "provider_metadata"
        case warnings
    }

    public init(audio: AudioData, providerMetadata: JSONValue? = nil, warnings: [JSONValue]? = nil) {
        self.audio = audio; self.providerMetadata = providerMetadata; self.warnings = warnings
    }
}

// MARK: - Image

/// Generated images: all base64 strings or all binary byte arrays.
/// Wire: `{"Base64":["…"]}` | `{"Binary":[[…]]}`.
public enum ImageOutputs: Codable, Equatable {
    case base64([String])
    case binary([[UInt8]])

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: MMCodingKey.self)
        guard let key = c.allKeys.first?.stringValue else {
            throw mmDecodingError(c.codingPath, "missing image-outputs tag")
        }
        switch key {
        case "Base64":
            self = .base64(try c.decode([String].self, forKey: MMCodingKey(key)))
        case "Binary":
            self = .binary(try c.decode([[UInt8]].self, forKey: MMCodingKey(key)))
        default:
            throw mmDecodingError(c.codingPath, "unknown image-outputs tag \(key)")
        }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: MMCodingKey.self)
        switch self {
        case .base64(let strings):
            try c.encode(strings, forKey: MMCodingKey("Base64"))
        case .binary(let batches):
            try c.encode(batches, forKey: MMCodingKey("Binary"))
        }
    }
}

/// The result of an image generation call.
public struct ImageResult: Codable, Equatable {
    public var images: ImageOutputs
    public var providerMetadata: JSONValue?
    public var warnings: [JSONValue]?

    enum CodingKeys: String, CodingKey {
        case images
        case providerMetadata = "provider_metadata"
        case warnings
    }

    public init(images: ImageOutputs, providerMetadata: JSONValue? = nil, warnings: [JSONValue]? = nil) {
        self.images = images; self.providerMetadata = providerMetadata; self.warnings = warnings
    }
}

// MARK: - Transcription (STT)

/// A transcript segment with timing.
public struct TranscriptionSegment: Codable, Equatable {
    public var text: String
    public var start: Double?
    public var end: Double?

    public init(text: String, start: Double? = nil, end: Double? = nil) {
        self.text = text; self.start = start; self.end = end
    }
}

/// The result of a transcription call.
public struct TranscriptionResult: Codable, Equatable {
    public var text: String
    public var segments: [TranscriptionSegment]?
    public var language: String?
    public var providerMetadata: JSONValue?
    public var warnings: [JSONValue]?

    enum CodingKeys: String, CodingKey {
        case text, segments, language
        case providerMetadata = "provider_metadata"
        case warnings
    }

    public init(text: String, segments: [TranscriptionSegment]? = nil, language: String? = nil,
                providerMetadata: JSONValue? = nil, warnings: [JSONValue]? = nil) {
        self.text = text; self.segments = segments; self.language = language
        self.providerMetadata = providerMetadata; self.warnings = warnings
    }
}

// MARK: - Video

/// The URL variant of `VideoData`.
public struct VideoUrlData: Codable, Equatable {
    public var url: String
    public var mediaType: String?

    enum CodingKeys: String, CodingKey {
        case url
        case mediaType = "media_type"
    }

    public init(url: String, mediaType: String? = nil) {
        self.url = url; self.mediaType = mediaType
    }
}

/// The base64 variant of `VideoData`.
public struct VideoBase64Data: Codable, Equatable {
    public var data: String
    public var mediaType: String?

    enum CodingKeys: String, CodingKey {
        case data
        case mediaType = "media_type"
    }

    public init(data: String, mediaType: String? = nil) {
        self.data = data; self.mediaType = mediaType
    }
}

/// The binary variant of `VideoData`.
public struct VideoBinaryData: Codable, Equatable {
    public var data: [UInt8]
    public var mediaType: String?

    enum CodingKeys: String, CodingKey {
        case data
        case mediaType = "media_type"
    }

    public init(data: [UInt8], mediaType: String? = nil) {
        self.data = data; self.mediaType = mediaType
    }
}

/// Generated video: a download URL, a base64 string, or raw binary bytes.
/// Wire: `{"Url":{…}}` | `{"Base64":{…}}` | `{"Binary":{…}}`.
public enum VideoData: Codable, Equatable {
    case url(VideoUrlData)
    case base64(VideoBase64Data)
    case binary(VideoBinaryData)

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: MMCodingKey.self)
        guard let key = c.allKeys.first?.stringValue else {
            throw mmDecodingError(c.codingPath, "missing video-data tag")
        }
        switch key {
        case "Url":
            self = .url(try c.decode(VideoUrlData.self, forKey: MMCodingKey(key)))
        case "Base64":
            self = .base64(try c.decode(VideoBase64Data.self, forKey: MMCodingKey(key)))
        case "Binary":
            self = .binary(try c.decode(VideoBinaryData.self, forKey: MMCodingKey(key)))
        default:
            throw mmDecodingError(c.codingPath, "unknown video-data tag \(key)")
        }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: MMCodingKey.self)
        switch self {
        case .url(let d):
            try c.encode(d, forKey: MMCodingKey("Url"))
        case .base64(let d):
            try c.encode(d, forKey: MMCodingKey("Base64"))
        case .binary(let d):
            try c.encode(d, forKey: MMCodingKey("Binary"))
        }
    }
}

/// The result of a video generation call.
public struct VideoResult: Codable, Equatable {
    public var videos: [VideoData]
    public var providerMetadata: JSONValue?
    public var warnings: [JSONValue]?

    enum CodingKeys: String, CodingKey {
        case videos
        case providerMetadata = "provider_metadata"
        case warnings
    }

    public init(videos: [VideoData], providerMetadata: JSONValue? = nil, warnings: [JSONValue]? = nil) {
        self.videos = videos; self.providerMetadata = providerMetadata; self.warnings = warnings
    }
}

// MARK: - Reranking

/// A single reranked entry.
public struct RerankingRank: Codable, Equatable {
    public var index: Int
    public var relevanceScore: Double

    enum CodingKeys: String, CodingKey {
        case index
        case relevanceScore = "relevance_score"
    }

    public init(index: Int, relevanceScore: Double) {
        self.index = index; self.relevanceScore = relevanceScore
    }
}

/// The result of a reranking call.
public struct RerankingResult: Codable, Equatable {
    public var ranking: [RerankingRank]
    public var providerMetadata: JSONValue?
    public var warnings: [JSONValue]?

    enum CodingKeys: String, CodingKey {
        case ranking
        case providerMetadata = "provider_metadata"
        case warnings
    }

    public init(ranking: [RerankingRank], providerMetadata: JSONValue? = nil, warnings: [JSONValue]? = nil) {
        self.ranking = ranking; self.providerMetadata = providerMetadata; self.warnings = warnings
    }
}

// MARK: - Search

/// A single search result.
public struct SearchResultItem: Codable, Equatable {
    public var title: String?
    public var url: String?
    public var content: String?

    public init(title: String? = nil, url: String? = nil, content: String? = nil) {
        self.title = title; self.url = url; self.content = content
    }
}

/// The result of a search call.
public struct SearchResult: Codable, Equatable {
    public var results: [SearchResultItem]
    public var answer: String?
    public var providerMetadata: JSONValue?
    public var warnings: [JSONValue]?

    enum CodingKeys: String, CodingKey {
        case results, answer
        case providerMetadata = "provider_metadata"
        case warnings
    }

    public init(results: [SearchResultItem], answer: String? = nil,
                providerMetadata: JSONValue? = nil, warnings: [JSONValue]? = nil) {
        self.results = results; self.answer = answer
        self.providerMetadata = providerMetadata; self.warnings = warnings
    }
}

// MARK: - Files

/// The result of a file upload.
public struct UploadFileResult: Codable, Equatable {
    public var providerReference: [String: String]
    public var providerMetadata: JSONValue?
    public var warnings: [JSONValue]?

    enum CodingKeys: String, CodingKey {
        case providerReference = "provider_reference"
        case providerMetadata = "provider_metadata"
        case warnings
    }

    public init(providerReference: [String: String], providerMetadata: JSONValue? = nil, warnings: [JSONValue]? = nil) {
        self.providerReference = providerReference
        self.providerMetadata = providerMetadata; self.warnings = warnings
    }
}
