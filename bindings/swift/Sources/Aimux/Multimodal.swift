import Foundation

// MARK: - Embedding

/// Token usage for an embedding call.
public struct EmbeddingUsage: Codable, Equatable {
    public var tokens: UInt32
}

/// Provider response metadata for embeddings.
public struct EmbeddingResponse: Codable, Equatable {
    public var headers: [String: String]?
    public var body: JSONValue?
}

/// The result of an embedding call.
public struct EmbeddingResult: Codable, Equatable {
    public var embeddings: [[Float]]
    public var usage: EmbeddingUsage?
    public var providerMetadata: ProviderMetadata?
    public var response: EmbeddingResponse?
    public var warnings: [Warning]
}

// MARK: - Speech (TTS)

/// Generated audio: a base64-encoded string or raw bytes. Wire: a base64
/// string (`speech-model-v4-result.ts:16`), the same shape as ``FileBytes``.
public typealias AudioData = FileBytes

public struct SpeechRequest: Codable, Equatable {
    public var body: JSONValue?
}

/// Response information for a speech call.
public struct SpeechResponse: Codable, Equatable {
    public var timestamp: String?
    public var modelId: String?
    public var headers: [String: String]?
    public var body: JSONValue?
}

/// The result of a speech generation call.
public struct SpeechResult: Codable, Equatable {
    public var audio: AudioData
    public var warnings: [Warning]
    public var request: SpeechRequest?
    public var response: SpeechResponse
    public var providerMetadata: ProviderMetadata?
}

// MARK: - Image

/// Generated images: all base64 strings or all binary byte arrays. Wire:
/// base64 strings `["…"]` (`image-model-v4-result.ts:23`); `.binary` is
/// written as base64 and parsing yields `.base64`.
public enum ImageOutputs: Codable, Equatable {
    case base64([String])
    case binary([[UInt8]])

    public init(from decoder: Decoder) throws {
        let c = try decoder.singleValueContainer()
        guard let strings = try? c.decode([String].self) else {
            throw aimuxDecodingError(c.codingPath, "expected base64 strings")
        }
        self = .base64(strings)
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        switch self {
        case .base64(let strings): try c.encode(strings)
        case .binary(let batches): try c.encode(batches.map(Base64Bytes.encode))
        }
    }
}

/// Response information for an image generation call.
public struct ImageResponse: Codable, Equatable {
    public var timestamp: String?
    public var modelId: String?
    public var headers: [String: String]?
}

public struct ImageUsage: Codable, Equatable {
    public var inputTokens: UInt32?
    public var outputTokens: UInt32?
    public var totalTokens: UInt32?
}

/// The result of an image generation call.
public struct ImageResult: Codable, Equatable {
    public var images: ImageOutputs
    public var warnings: [Warning]
    public var providerMetadata: ProviderMetadata?
    public var response: ImageResponse
    public var usage: ImageUsage?
}

// MARK: - Transcription (STT)

/// A transcript segment with timing.
public struct TranscriptionSegment: Codable, Equatable {
    public var text: String
    public var startSecond: Double
    public var endSecond: Double
}

public struct TranscriptionRequest: Codable, Equatable {
    public var body: String?
}

/// Response information for a transcription call.
public struct TranscriptionResponse: Codable, Equatable {
    public var timestamp: String?
    public var modelId: String?
    public var headers: [String: String]?
    public var body: JSONValue?
}

/// The result of a transcription call.
public struct TranscriptionResult: Codable, Equatable {
    public var text: String
    public var segments: [TranscriptionSegment]
    public var language: String?
    public var durationInSeconds: Double?
    public var warnings: [Warning]
    public var request: TranscriptionRequest?
    public var response: TranscriptionResponse
    public var providerMetadata: ProviderMetadata?
}

// MARK: - Video

/// Generated video. Wire: `{"type":"url","url","mediaType"}`,
/// `{"type":"base64","data":"…","mediaType"}` or
/// `{"type":"binary","data":"<base64>","mediaType"}` (`video-model-v4-result.ts:28-29`).
public enum VideoData: Codable, Equatable {
    case url(url: String, mediaType: String)
    case base64(data: String, mediaType: String)
    case binary(data: [UInt8], mediaType: String)

    private enum Field: String, CodingKey { case type, url, data, mediaType }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: Field.self)
        let mediaType = try c.decode(String.self, forKey: .mediaType)
        switch try c.decode(String.self, forKey: .type) {
        case "url": self = .url(url: try c.decode(String.self, forKey: .url), mediaType: mediaType)
        case "base64": self = .base64(data: try c.decode(String.self, forKey: .data), mediaType: mediaType)
        case "binary": self = .binary(data: try Base64Bytes.decode(c, forKey: .data), mediaType: mediaType)
        case let t: throw decoder.unknownTag(t, in: "video data")
        }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: Field.self)
        switch self {
        case .url(let url, let mediaType):
            try c.encode("url", forKey: .type); try c.encode(url, forKey: .url)
            try c.encode(mediaType, forKey: .mediaType)
        case .base64(let data, let mediaType):
            try c.encode("base64", forKey: .type); try c.encode(data, forKey: .data)
            try c.encode(mediaType, forKey: .mediaType)
        case .binary(let data, let mediaType):
            try c.encode("binary", forKey: .type); try c.encode(Base64Bytes.encode(data), forKey: .data)
            try c.encode(mediaType, forKey: .mediaType)
        }
    }
}

/// Response information for a video generation call.
public struct VideoResponse: Codable, Equatable {
    public var timestamp: String?
    public var modelId: String?
    public var headers: [String: String]?
}

/// The result of a video generation call.
public struct VideoResult: Codable, Equatable {
    public var videos: [VideoData]
    public var warnings: [Warning]
    public var providerMetadata: ProviderMetadata?
    public var response: VideoResponse
}

// MARK: - Reranking

/// A single reranked entry.
public struct RerankingRank: Codable, Equatable {
    public var index: Int
    public var relevanceScore: Double
}

/// Optional response information for a reranking call.
public struct RerankingResponse: Codable, Equatable {
    public var id: String?
    public var timestamp: String?
    public var modelId: String?
    public var headers: [String: String]?
    public var body: JSONValue?
}

/// The result of a reranking call.
public struct RerankingResult: Codable, Equatable {
    public var ranking: [RerankingRank]
    public var providerMetadata: ProviderMetadata?
    public var warnings: [Warning]?
    public var response: RerankingResponse?
}

// MARK: - Search

/// A single search result.
public struct SearchResultItem: Codable, Equatable {
    public var title: String?
    public var url: String?
    public var content: String?
    public var rawContent: String?
    public var score: Double?
    public var providerMetadata: ProviderMetadata?
}

/// Optional response information for a search call.
public struct SearchResponse: Codable, Equatable {
    public var headers: [String: String]?
    public var body: JSONValue?
}

/// The result of a search call.
public struct SearchResult: Codable, Equatable {
    public var results: [SearchResultItem]
    public var answer: String?
    public var providerMetadata: ProviderMetadata?
    public var warnings: [Warning]
    public var response: SearchResponse?
}

// MARK: - Files

/// The result of a file upload.
public struct UploadFileResult: Codable, Equatable {
    public var providerReference: [String: String]
    public var mediaType: String?
    public var filename: String?
    public var providerMetadata: ProviderMetadata?
    public var warnings: [Warning]
}
