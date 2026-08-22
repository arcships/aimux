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
        let handle = try Model.wrapHandle(expecting: expectFfiError) { aimux_openai_embedding_new(apiKey, modelId, $0) }
        return EmbeddingModel(handle: handle)
    }

    /// Create an OpenAI embedding model instance with a custom base URL.
    public static func openai(apiKey: String, modelId: String, baseUrl: String) throws -> EmbeddingModel {
        let handle = try Model.wrapHandle(expecting: expectFfiError) { aimux_openai_embedding_new_with_base(apiKey, modelId, baseUrl, $0) }
        return EmbeddingModel(handle: handle)
    }

    /// Create a Cohere embedding model instance (e.g. embed-english-v3.0).
    public static func cohere(apiKey: String, modelId: String) throws -> EmbeddingModel {
        let handle = try Model.wrapHandle(expecting: expectFfiError) { aimux_cohere_embedding_new(apiKey, modelId, $0) }
        return EmbeddingModel(handle: handle)
    }

    /// Create a Cohere embedding model instance with a custom base URL.
    public static func cohere(apiKey: String, modelId: String, baseUrl: String) throws -> EmbeddingModel {
        let handle = try Model.wrapHandle(expecting: expectFfiError) { aimux_cohere_embedding_new_with_base(apiKey, modelId, baseUrl, $0) }
        return EmbeddingModel(handle: handle)
    }

    /// Create a Google embedding model instance (e.g. gemini-embedding-001).
    public static func google(apiKey: String, modelId: String) throws -> EmbeddingModel {
        let handle = try Model.wrapHandle(expecting: expectFfiError) { aimux_google_embedding_new(apiKey, modelId, $0) }
        return EmbeddingModel(handle: handle)
    }

    /// Create a Google embedding model instance with a custom base URL.
    public static func google(apiKey: String, modelId: String, baseUrl: String) throws -> EmbeddingModel {
        let handle = try Model.wrapHandle(expecting: expectFfiError) { aimux_google_embedding_new_with_base(apiKey, modelId, baseUrl, $0) }
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
        try validateJson(values, parameter: "values")
        try validateJson(options, parameter: "options")
        let h = handle
        return try ffiStringCall {
            aimux_embed(h, values, options, $0)
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
        let handle = try Model.wrapHandle(expecting: expectFfiError) { aimux_openai_speech_new(apiKey, modelId, $0) }
        return SpeechModel(handle: handle)
    }

    /// Create an OpenAI speech model instance with a custom base URL.
    public static func openai(apiKey: String, modelId: String, baseUrl: String) throws -> SpeechModel {
        let handle = try Model.wrapHandle(expecting: expectFfiError) { aimux_openai_speech_new_with_base(apiKey, modelId, baseUrl, $0) }
        return SpeechModel(handle: handle)
    }

    // ── Generation ─────────────────────────────────────────────────────────

    /// Generate speech audio from the given options.
    ///
    /// - Parameter options: SpeechCallOptions serialized as JSON (required — carries the input).
    /// - Returns: The JSON-serialized SpeechResult.
    public func generate(options: String) throws -> String {
        try validateJson(options, parameter: "options")
        let h = handle
        return try ffiStringCall {
            aimux_speech_generate(h, options, $0)
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
        let handle = try Model.wrapHandle(expecting: expectFfiError) { aimux_openai_transcription_new(apiKey, modelId, $0) }
        return TranscriptionModel(handle: handle)
    }

    /// Create an OpenAI transcription model instance with a custom base URL.
    public static func openai(apiKey: String, modelId: String, baseUrl: String) throws -> TranscriptionModel {
        let handle = try Model.wrapHandle(expecting: expectFfiError) { aimux_openai_transcription_new_with_base(apiKey, modelId, baseUrl, $0) }
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
        try validateJson(options, parameter: "options")
        let h = handle
        return try ffiStringCall {
            aimux_transcription_generate(h, audioBase64, mediaType, options, $0)
        }
    }

    /// Start a streaming transcription session (RFC-0028) on this model.
    /// Requires a model that supports `do_stream` (realtime models).
    ///
    /// - Parameters:
    ///   - options: Optional session options JSON
    ///     (`{"input_audio_format": {...}, "provider_options", "headers",
    ///     "include_raw_chunks"}`).
    ///   - abortHandle: Optional abort handle (`aimux_abort_signal_new`);
    ///     firing it aborts the session.
    public func startStream(options: String? = nil, abortHandle: UInt64 = 0) throws -> TranscriptionSession {
        try validateJson(options, parameter: "options")
        let h = handle
        let session = try Model.wrapHandle { out in
            aimux_transcription_session_new(h, abortHandle, options, out)
        }
        return TranscriptionSession(handle: session)
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
        let handle = try Model.wrapHandle(expecting: expectFfiError) { aimux_openai_image_new(apiKey, modelId, $0) }
        return ImageModel(handle: handle)
    }

    /// Create an OpenAI image model instance with a custom base URL.
    public static func openai(apiKey: String, modelId: String, baseUrl: String) throws -> ImageModel {
        let handle = try Model.wrapHandle(expecting: expectFfiError) { aimux_openai_image_new_with_base(apiKey, modelId, baseUrl, $0) }
        return ImageModel(handle: handle)
    }

    /// Create a Google image model instance (e.g. gemini-2.5-flash-image).
    public static func google(apiKey: String, modelId: String) throws -> ImageModel {
        let handle = try Model.wrapHandle(expecting: expectFfiError) { aimux_google_image_new(apiKey, modelId, $0) }
        return ImageModel(handle: handle)
    }

    /// Create a Google image model instance with a custom base URL.
    public static func google(apiKey: String, modelId: String, baseUrl: String) throws -> ImageModel {
        let handle = try Model.wrapHandle(expecting: expectFfiError) { aimux_google_image_new_with_base(apiKey, modelId, baseUrl, $0) }
        return ImageModel(handle: handle)
    }

    // ── Generation ─────────────────────────────────────────────────────────

    /// Generate images from the given options.
    ///
    /// - Parameter options: ImageCallOptions serialized as JSON (required — carries the input).
    /// - Returns: The JSON-serialized ImageResult.
    public func generate(options: String) throws -> String {
        try validateJson(options, parameter: "options")
        let h = handle
        return try ffiStringCall {
            aimux_image_generate(h, options, $0)
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
        let handle = try Model.wrapHandle(expecting: expectFfiError) { aimux_google_video_new(apiKey, modelId, $0) }
        return VideoModel(handle: handle)
    }

    /// Create a Google video model instance with a custom base URL.
    public static func google(apiKey: String, modelId: String, baseUrl: String) throws -> VideoModel {
        let handle = try Model.wrapHandle(expecting: expectFfiError) { aimux_google_video_new_with_base(apiKey, modelId, baseUrl, $0) }
        return VideoModel(handle: handle)
    }

    // ── Generation ─────────────────────────────────────────────────────────

    /// Generate videos from the given options.
    ///
    /// - Parameter options: VideoCallOptions serialized as JSON (required — carries the input).
    /// - Returns: The JSON-serialized VideoResult.
    public func generate(options: String) throws -> String {
        try validateJson(options, parameter: "options")
        let h = handle
        return try ffiStringCall {
            aimux_video_generate(h, options, $0)
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
        let handle = try Model.wrapHandle(expecting: expectFfiError) { aimux_cohere_reranking_new(apiKey, modelId, $0) }
        return RerankingModel(handle: handle)
    }

    /// Create a Cohere reranking model instance with a custom base URL.
    public static func cohere(apiKey: String, modelId: String, baseUrl: String) throws -> RerankingModel {
        let handle = try Model.wrapHandle(expecting: expectFfiError) { aimux_cohere_reranking_new_with_base(apiKey, modelId, baseUrl, $0) }
        return RerankingModel(handle: handle)
    }

    // ── Generation ─────────────────────────────────────────────────────────

    /// Rerank documents by relevance to a query.
    ///
    /// - Parameter options: RerankingCallOptions serialized as JSON (required — carries the input).
    /// - Returns: The JSON-serialized RerankingResult.
    public func rerank(options: String) throws -> String {
        try validateJson(options, parameter: "options")
        let h = handle
        return try ffiStringCall {
            aimux_rerank(h, options, $0)
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
        let handle = try Model.wrapHandle(expecting: expectFfiError) { aimux_tavily_search_new(apiKey, "", $0) }
        return SearchModel(handle: handle)
    }

    /// Create a Tavily search model instance with a custom base URL (useful for
    /// testing against a mock server).
    public static func tavily(apiKey: String, baseUrl: String) throws -> SearchModel {
        let handle = try Model.wrapHandle(expecting: expectFfiError) { aimux_tavily_search_new_with_base(apiKey, "", baseUrl, $0) }
        return SearchModel(handle: handle)
    }

    // ── Generation ─────────────────────────────────────────────────────────

    /// Perform a web search.
    ///
    /// - Parameter options: SearchCallOptions serialized as JSON (required — carries the input).
    /// - Returns: The JSON-serialized SearchResult.
    public func search(options: String) throws -> String {
        try validateJson(options, parameter: "options")
        let h = handle
        return try ffiStringCall {
            aimux_search(h, options, $0)
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
        let handle = try Model.wrapHandle(expecting: expectFfiError) { aimux_openai_files_new(apiKey, $0) }
        return Files(handle: handle)
    }

    /// Create an OpenAI files manager instance with a custom base URL.
    public static func openai(apiKey: String, baseUrl: String) throws -> Files {
        let handle = try Model.wrapHandle(expecting: expectFfiError) { aimux_openai_files_new_with_base(apiKey, baseUrl, $0) }
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
        try validateJson(options, parameter: "options")
        let h = handle
        return try ffiStringCall {
            aimux_file_upload(h, dataBase64, mediaType, options, $0)
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
    public var headers: [String: String]?
    public var body: JSONValue?

    enum CodingKeys: String, CodingKey {
        case headers
        case body
    }

    public init(headers: [String: String]? = nil, body: JSONValue? = nil) {
        self.headers = headers; self.body = body
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

/// Response information for a speech call.
public struct SpeechResponse: Codable, Equatable {
    public var timestamp: String?
    public var modelId: String?
    public var headers: [String: String]?
    public var body: JSONValue?

    enum CodingKeys: String, CodingKey {
        case timestamp
        case modelId = "model_id"
        case headers
        case body
    }

    public init(timestamp: String? = nil, modelId: String? = nil,
                headers: [String: String]? = nil, body: JSONValue? = nil) {
        self.timestamp = timestamp; self.modelId = modelId
        self.headers = headers; self.body = body
    }
}

/// The result of a speech generation call.
public struct SpeechResult: Codable, Equatable {
    public var audio: AudioData
    public var response: SpeechResponse
    public var providerMetadata: JSONValue?
    public var warnings: [JSONValue]?

    enum CodingKeys: String, CodingKey {
        case audio, response
        case providerMetadata = "provider_metadata"
        case warnings
    }

    public init(audio: AudioData, response: SpeechResponse,
                providerMetadata: JSONValue? = nil, warnings: [JSONValue]? = nil) {
        self.audio = audio; self.response = response
        self.providerMetadata = providerMetadata; self.warnings = warnings
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

/// Response information for an image generation call.
public struct ImageResponse: Codable, Equatable {
    public var timestamp: String?
    public var modelId: String?
    public var headers: [String: String]?

    enum CodingKeys: String, CodingKey {
        case timestamp
        case modelId = "model_id"
        case headers
    }

    public init(timestamp: String? = nil, modelId: String? = nil, headers: [String: String]? = nil) {
        self.timestamp = timestamp; self.modelId = modelId; self.headers = headers
    }
}

/// The result of an image generation call.
public struct ImageResult: Codable, Equatable {
    public var images: ImageOutputs
    public var response: ImageResponse
    public var providerMetadata: JSONValue?
    public var warnings: [JSONValue]?

    enum CodingKeys: String, CodingKey {
        case images, response
        case providerMetadata = "provider_metadata"
        case warnings
    }

    public init(images: ImageOutputs, response: ImageResponse,
                providerMetadata: JSONValue? = nil, warnings: [JSONValue]? = nil) {
        self.images = images; self.response = response
        self.providerMetadata = providerMetadata; self.warnings = warnings
    }
}

// MARK: - Transcription (STT)

/// A transcript segment with timing.
public struct TranscriptionSegment: Codable, Equatable {
    public var text: String
    /// Required `f64` in the core, not an `Option` — see `TranscriptionSegment.ts` —
    /// so these stay non-optional and are always encoded; a provider that omits
    /// them decodes to `0` rather than failing.
    public var startSecond: Double
    public var endSecond: Double

    enum CodingKeys: String, CodingKey {
        case text
        case startSecond = "start_second"
        case endSecond = "end_second"
    }

    public init(text: String, startSecond: Double, endSecond: Double) {
        self.text = text; self.startSecond = startSecond; self.endSecond = endSecond
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        self.text = try c.decodeIfPresent(String.self, forKey: .text) ?? ""
        self.startSecond = try c.decodeIfPresent(Double.self, forKey: .startSecond) ?? 0
        self.endSecond = try c.decodeIfPresent(Double.self, forKey: .endSecond) ?? 0
    }
}

/// Response information for a transcription call.
public struct TranscriptionResponse: Codable, Equatable {
    public var timestamp: String?
    public var modelId: String?
    public var headers: [String: String]?
    public var body: JSONValue?

    enum CodingKeys: String, CodingKey {
        case timestamp
        case modelId = "model_id"
        case headers
        case body
    }

    public init(timestamp: String? = nil, modelId: String? = nil,
                headers: [String: String]? = nil, body: JSONValue? = nil) {
        self.timestamp = timestamp; self.modelId = modelId
        self.headers = headers; self.body = body
    }
}

/// The result of a transcription call.
public struct TranscriptionResult: Codable, Equatable {
    public var text: String
    public var segments: [TranscriptionSegment]?
    public var language: String?
    public var response: TranscriptionResponse
    public var providerMetadata: JSONValue?
    public var warnings: [JSONValue]?

    enum CodingKeys: String, CodingKey {
        case text, segments, language, response
        case providerMetadata = "provider_metadata"
        case warnings
    }

    public init(text: String, response: TranscriptionResponse,
                segments: [TranscriptionSegment]? = nil, language: String? = nil,
                providerMetadata: JSONValue? = nil, warnings: [JSONValue]? = nil) {
        self.text = text; self.response = response
        self.segments = segments; self.language = language
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

/// Response information for a video generation call.
public struct VideoResponse: Codable, Equatable {
    public var timestamp: String?
    public var modelId: String?
    public var headers: [String: String]?

    enum CodingKeys: String, CodingKey {
        case timestamp
        case modelId = "model_id"
        case headers
    }

    public init(timestamp: String? = nil, modelId: String? = nil, headers: [String: String]? = nil) {
        self.timestamp = timestamp; self.modelId = modelId; self.headers = headers
    }
}

/// The result of a video generation call.
public struct VideoResult: Codable, Equatable {
    public var videos: [VideoData]
    public var response: VideoResponse
    public var providerMetadata: JSONValue?
    public var warnings: [JSONValue]?

    enum CodingKeys: String, CodingKey {
        case videos, response
        case providerMetadata = "provider_metadata"
        case warnings
    }

    public init(videos: [VideoData], response: VideoResponse,
                providerMetadata: JSONValue? = nil, warnings: [JSONValue]? = nil) {
        self.videos = videos; self.response = response
        self.providerMetadata = providerMetadata; self.warnings = warnings
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

/// Optional response information for a reranking call.
public struct RerankingResponse: Codable, Equatable {
    public var id: String?
    public var timestamp: String?
    public var modelId: String?
    public var headers: [String: String]?
    public var body: JSONValue?

    enum CodingKeys: String, CodingKey {
        case id
        case timestamp
        case modelId = "model_id"
        case headers
        case body
    }

    public init(id: String? = nil, timestamp: String? = nil, modelId: String? = nil,
                headers: [String: String]? = nil, body: JSONValue? = nil) {
        self.id = id; self.timestamp = timestamp; self.modelId = modelId
        self.headers = headers; self.body = body
    }
}

/// The result of a reranking call.
public struct RerankingResult: Codable, Equatable {
    public var ranking: [RerankingRank]
    public var response: RerankingResponse?
    public var providerMetadata: JSONValue?
    public var warnings: [JSONValue]?

    enum CodingKeys: String, CodingKey {
        case ranking, response
        case providerMetadata = "provider_metadata"
        case warnings
    }

    public init(ranking: [RerankingRank], response: RerankingResponse? = nil,
                providerMetadata: JSONValue? = nil, warnings: [JSONValue]? = nil) {
        self.ranking = ranking; self.response = response
        self.providerMetadata = providerMetadata; self.warnings = warnings
    }
}

// MARK: - Search

/// A single search result.
public struct SearchResultItem: Codable, Equatable {
    public var title: String?
    public var url: String?
    public var content: String?
    public var rawContent: String?
    public var score: Double?
    public var providerMetadata: [String: JSONValue]?

    enum CodingKeys: String, CodingKey {
        case title, url, content
        case rawContent = "raw_content"
        case score
        case providerMetadata = "provider_metadata"
    }

    public init(title: String? = nil, url: String? = nil, content: String? = nil,
                rawContent: String? = nil, score: Double? = nil, providerMetadata: [String: JSONValue]? = nil) {
        self.title = title; self.url = url; self.content = content
        self.rawContent = rawContent; self.score = score; self.providerMetadata = providerMetadata
    }
}

/// Optional response information for a search call.
public struct SearchResponse: Codable, Equatable {
    public var headers: [String: String]?
    public var body: JSONValue?

    enum CodingKeys: String, CodingKey {
        case headers
        case body
    }

    public init(headers: [String: String]? = nil, body: JSONValue? = nil) {
        self.headers = headers; self.body = body
    }
}

/// The result of a search call.
public struct SearchResult: Codable, Equatable {
    public var results: [SearchResultItem]
    public var answer: String?
    public var response: SearchResponse?
    public var providerMetadata: JSONValue?
    public var warnings: [JSONValue]?

    enum CodingKeys: String, CodingKey {
        case results, answer, response
        case providerMetadata = "provider_metadata"
        case warnings
    }

    public init(results: [SearchResultItem], answer: String? = nil, response: SearchResponse? = nil,
                providerMetadata: JSONValue? = nil, warnings: [JSONValue]? = nil) {
        self.results = results; self.answer = answer; self.response = response
        self.providerMetadata = providerMetadata; self.warnings = warnings
    }
}

// MARK: - Files

/// The result of a file upload.
public struct UploadFileResult: Codable, Equatable {
    public var providerReference: [String: String]
    public var mediaType: String?
    public var filename: String?
    public var providerMetadata: JSONValue?
    public var warnings: [JSONValue]?

    enum CodingKeys: String, CodingKey {
        case providerReference = "provider_reference"
        case mediaType = "media_type"
        case filename
        case providerMetadata = "provider_metadata"
        case warnings
    }

    public init(providerReference: [String: String], mediaType: String? = nil, filename: String? = nil,
                providerMetadata: JSONValue? = nil, warnings: [JSONValue]? = nil) {
        self.providerReference = providerReference
        self.mediaType = mediaType; self.filename = filename
        self.providerMetadata = providerMetadata; self.warnings = warnings
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// TranscriptionSession (RFC-0028 streaming)
// ─────────────────────────────────────────────────────────────────────────────

/// Thrown by `TranscriptionSession.nextPart` when the stream ended normally
/// (a `Finish` part was delivered earlier).
///
/// A session the caller `close()`d does NOT throw this — it throws the Swift
/// boundary projection (`DecodingError.dataCorrupted`, "transcription session
/// is closed"). Keeping the two apart matters: a pump loop that `break`s on
/// this error must not report a transcript its own `defer { close() }`
/// truncated as a complete one.
public struct AimuxTranscriptionEndedError: Error {}

/// Error for `TranscriptionSession.nextPart`: no part arrived within the
/// timeout. The session stays live — call again.
public struct AimuxTranscriptionTimeoutError: Error {}

/// A live streaming-transcription session (RFC-0028): push audio chunks,
/// mark end-of-audio, then pull transcription parts (JSON
/// `TranscriptionStreamPart`s).
public final class TranscriptionSession: @unchecked Sendable {

    // The opaque session handle from aimux-ffi. 0 means invalid/freed.
    private var handle: UInt64

    // Internal: sessions are created via `TranscriptionModel.startStream`.
    init(handle: UInt64) {
        self.handle = handle
    }

    deinit {
        if handle != 0 {
            aimux_transcription_session_drop(handle)
        }
    }

    // The handle read + nil-check must stay on one thread with the call;
    // sessions are single-threaded (like Model). Using a closed session is a
    // C ABI failure, thrown (never trapped — a trap here is uncatchable
    // SIGTRAP in a host app) and deliberately NOT the same type as a clean
    // end of stream: a pump loop that breaks on AimuxTranscriptionEndedError
    // must not read "transcript complete" from a transcript its own
    // `defer { close() }` truncated.
    private func withHandle<T>(_ body: (UInt64) throws -> T) throws -> T {
        let h = handle
        guard h != 0 else {
            throw invariant("aimux: transcription session is closed")
        }
        return try body(h)
    }

    /// Push one binary audio chunk. Blocks while the internal channel is full
    /// (backpressure propagation).
    ///
    /// Throws the Swift boundary projection (`DecodingError.dataCorrupted`,
    /// "transcription session is closed") if the session was closed — never
    /// `AimuxTranscriptionEndedError`, which means the stream ended on its own.
    public func pushAudio(_ audio: [UInt8]) throws {
        try withHandle { h in
            let e = audio.withUnsafeBufferPointer { buf -> OpaquePointer? in
                aimux_transcription_push_audio(h, buf.baseAddress, audio.count)
            }
            if let e { throw expectAimuxError(e, context: "pushAudio") }
        }
    }

    /// Signal end-of-audio (idempotent).
    ///
    /// Throws the Swift boundary projection (`DecodingError.dataCorrupted`,
    /// "transcription session is closed") if the session was closed — the same
    /// signal as the other two methods, matching Go's `ErrClosed`, which its
    /// `InputDone` also returns rather than swallowing.
    public func inputDone() throws {
        try withHandle { h in
            if let e = aimux_transcription_input_done(h) { throw expectFfiError(e, context: "inputDone") }
        }
    }

    /// Pull the next transcription part (JSON `TranscriptionStreamPart`).
    ///
    /// Throws `AimuxTranscriptionEndedError` when the stream finished
    /// normally, the Swift boundary projection (`DecodingError.dataCorrupted`)
    /// when the session was closed — the two are deliberately distinct, so a
    /// pump loop that breaks on "ended" cannot read a transcript its own
    /// `close()` truncated as complete — and `AimuxTranscriptionTimeoutError`
    /// when no part arrived in time (retryable, not an error at the C ABI —
    /// nothing is decoded).
    /// `timeoutMs`: >0 wait at most; 0 immediate poll; negative = wait
    /// indefinitely.
    public func nextPart(timeoutMs: Int64) throws -> String {
        try withHandle { h in
            var part: UnsafeMutablePointer<CChar>? = nil
            var state: Int32 = 0
            if let e = aimux_transcription_next_part(h, timeoutMs, &part, &state) {
                throw expectAimuxError(e, context: "nextPart")
            }
            switch state {
            case Int32(AIMUX_TRANSCRIPTION_NEXT_PART_PART.rawValue):
                guard let part else { throw invariant("aimux ffi: nextPart: PART state but no part written") }
                defer { aimux_free_string(part) }
                return String(cString: part)
            case Int32(AIMUX_TRANSCRIPTION_NEXT_PART_ENDED.rawValue):
                throw AimuxTranscriptionEndedError()
            case Int32(AIMUX_TRANSCRIPTION_NEXT_PART_TIMEOUT.rawValue):
                throw AimuxTranscriptionTimeoutError()
            default:
                throw invariant("aimux ffi: nextPart: unknown aimux_transcription_next_part_state_t \(state)")
            }
        }
    }

    /// Terminate and release the session (aborts the driver; idempotent).
    public func close() {
        if handle != 0 {
            aimux_transcription_session_drop(handle)
            handle = 0
        }
    }
}
