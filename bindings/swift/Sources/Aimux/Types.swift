// Types.swift — typed Codable wrapper layer over the aimux-ffi C ABI.
//
// The raw API in `Aimux.swift` exchanges JSON strings across the Swift↔C
// boundary (`generateText(prompt:options:) -> String`, `streamText` yields
// JSON-string parts). This file adds a thin, *typed* layer on top: inputs and
// outputs are `Codable` Swift types decoding and encoding exactly the JSON of
// `bindings/node/src/types/*.ts` (the ts-rs types generated from the Rust
// serde definitions, which equal the AI SDK JSON). The raw API is left
// untouched; the typed methods live in a `Model` extension and delegate to it.
//
// Wire conventions mirrored here:
//   • field names are camelCase, so structs synthesize their `Codable`
//   • optional fields are absent, never `null`
//   • unions are internally tagged: `type` (kebab-case values); `Source` uses
//     `sourceType`, `Role`-keyed messages use `role`
//   • `ToolChoice` is mixed: "auto"|"none"|"required" or {"type":"tool","toolName":…}
//   • `ModelPrompt` / `MessageContent` / `FileBytes` are untagged (string | array)

import CAimuxFFI
import Foundation

// MARK: - JSONValue (arbitrary JSON, for `input`/`output`/`raw`/`providerMetadata`/…)

/// A type-erased JSON value that round-trips through `JSONEncoder`/`JSONDecoder`.
///
/// On this toolchain `Bool` and `Double` decode are mutually exclusive (a JSON
/// bool fails `Double` decode and a JSON number fails `Bool` decode), so the
/// scalar ordering below is unambiguous.
public enum JSONValue: Codable, Equatable, Sendable {
    case null
    case bool(Bool)
    case number(Double)
    case string(String)
    case array([JSONValue])
    case object([String: JSONValue])

    public init(from decoder: Decoder) throws {
        let c = try decoder.singleValueContainer()
        if c.decodeNil() { self = .null; return }
        if let s = try? c.decode(String.self) { self = .string(s); return }
        if let a = try? c.decode([JSONValue].self) { self = .array(a); return }
        if let o = try? c.decode([String: JSONValue].self) { self = .object(o); return }
        if let b = try? c.decode(Bool.self) { self = .bool(b); return }
        if let n = try? c.decode(Double.self) { self = .number(n); return }
        throw aimuxDecodingError(c.codingPath, "unsupported JSON value")
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        switch self {
        case .null: try c.encodeNil()
        case .bool(let b): try c.encode(b)
        case .number(let n): try c.encode(n)
        case .string(let s): try c.encode(s)
        case .array(let a): try c.encode(a)
        case .object(let o): try c.encode(o)
        }
    }

    // MARK: accessors
    public subscript(key: String) -> JSONValue? {
        if case .object(let dict) = self { return dict[key] }
        return nil
    }
    public subscript(index: Int) -> JSONValue? {
        if case .array(let arr) = self, arr.indices.contains(index) { return arr[index] }
        return nil
    }
    public var stringValue: String? { if case .string(let s) = self { return s } else { return nil } }
    public var boolValue: Bool? { if case .bool(let b) = self { return b } else { return nil } }
    public var doubleValue: Double? { if case .number(let n) = self { return n } else { return nil } }
    public var intValue: Int? { if case .number(let n) = self { return Int(exactly: n) } else { return nil } }
    public var arrayValue: [JSONValue]? { if case .array(let a) = self { return a } else { return nil } }
    public var objectValue: [String: JSONValue]? { if case .object(let o) = self { return o } else { return nil } }
}

/// Provider-keyed options: `{ "<provider>": { "<option>": <json> } }`.
public typealias ProviderOptions = [String: [String: JSONValue]]
/// Provider-keyed metadata: `{ "<provider>": { "<field>": <json> } }`.
public typealias ProviderMetadata = [String: [String: JSONValue]]

// MARK: - Tagged-union plumbing

/// A `CodingKey` accepting any string, used for the `type` tag of internally
/// tagged unions (and by `Multimodal.swift`).
struct AnyCodingKey: CodingKey {
    let stringValue: String
    init?(stringValue: String) { self.stringValue = stringValue }
    var intValue: Int? { nil }
    init?(intValue: Int) { return nil }
    init(_ s: String) { self.stringValue = s }
}

/// Build a `DecodingError.dataCorrupted` for a coding path + message.
///
/// corelibs-foundation lacks the `dataCorruptedError(in: KeyedDecodingContainer)`
/// and `dataCorrupted(codingPath:debugDescription:)` helpers, so construct the
/// `Context` directly.
func aimuxDecodingError(_ codingPath: [any CodingKey], _ message: String) -> DecodingError {
    DecodingError.dataCorrupted(.init(codingPath: codingPath, debugDescription: message, underlyingError: nil))
}

extension Decoder {
    /// The tag of an internally tagged union (`type` unless told otherwise).
    func tag(_ key: String = "type") throws -> String {
        try container(keyedBy: AnyCodingKey.self).decode(String.self, forKey: AnyCodingKey(key))
    }

    func unknownTag(_ tag: String, in union: String) -> DecodingError {
        aimuxDecodingError(codingPath, "unknown \(union) tag \(tag)")
    }
}

extension Encoder {
    /// Add the tag to the keyed container a flattened payload already wrote.
    func putTag(_ tag: String, key: String = "type") throws {
        var c = container(keyedBy: AnyCodingKey.self)
        try c.encode(tag, forKey: AnyCodingKey(key))
    }
}

// MARK: - String-backed enums

/// Who sent a message. Wire: lowercase ("system"|"user"|"assistant"|"tool").
public enum Role: String, Codable {
    case system, user, assistant, tool
}

/// Unified finish reason. Wire: kebab-case.
public enum FinishReasonUnified: String, Codable {
    case stop
    case length
    case contentFilter = "content-filter"
    case toolCalls = "tool-calls"
    case error
    case other
}

/// Reasoning effort level. Wire: kebab-case.
public enum ReasoningEffort: String, Codable {
    case providerDefault = "provider-default"
    case none
    case minimal
    case low
    case medium
    case high
    case xhigh
}

// MARK: - Shared value structs

/// Why generation stopped.
public struct FinishReason: Codable, Equatable {
    public var unified: FinishReasonUnified
    /// Raw provider-specific reason (`nil` when the provider had none).
    public var raw: String?
}

public struct InputTokenUsage: Codable, Equatable {
    public var total: UInt32?
    public var noCache: UInt32?
    public var cacheRead: UInt32?
    public var cacheWrite: UInt32?
}

public struct OutputTokenUsage: Codable, Equatable {
    public var total: UInt32?
    public var text: UInt32?
    public var reasoning: UInt32?
}

/// Token usage statistics.
public struct Usage: Codable, Equatable {
    public var inputTokens: InputTokenUsage
    public var outputTokens: OutputTokenUsage
    /// Opaque, provider-specific raw usage.
    public var raw: [String: JSONValue]?
}

/// Metadata about the API response (the payload of a `response-metadata` part).
public struct ResponseMetadata: Codable, Equatable {
    public var id: String?
    public var timestamp: String?
    public var modelId: String?
}

/// Response information for telemetry and debugging.
public struct ResponseInfo: Codable, Equatable {
    public var id: String?
    public var timestamp: String?
    public var modelId: String?
    public var headers: [String: String]?
    public var body: JSONValue?
}

public struct RequestInfo: Codable, Equatable {
    public var body: JSONValue?
}

/// A provider warning. Wire: internally tagged by `type`.
public enum Warning: Codable, Equatable {
    case unsupported(feature: String, details: String?)
    case compatibility(feature: String, details: String?)
    case deprecated(setting: String, message: String)
    case other(message: String)

    private enum Field: String, CodingKey { case type, feature, details, setting, message }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: Field.self)
        switch try c.decode(String.self, forKey: .type) {
        case "unsupported": self = .unsupported(feature: try c.decode(String.self, forKey: .feature),
                                                details: try c.decodeIfPresent(String.self, forKey: .details))
        case "compatibility": self = .compatibility(feature: try c.decode(String.self, forKey: .feature),
                                                    details: try c.decodeIfPresent(String.self, forKey: .details))
        case "deprecated": self = .deprecated(setting: try c.decode(String.self, forKey: .setting),
                                              message: try c.decode(String.self, forKey: .message))
        case "other": self = .other(message: try c.decode(String.self, forKey: .message))
        case let t: throw decoder.unknownTag(t, in: "warning")
        }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: Field.self)
        switch self {
        case .unsupported(let f, let d):
            try c.encode("unsupported", forKey: .type); try c.encode(f, forKey: .feature)
            try c.encodeIfPresent(d, forKey: .details)
        case .compatibility(let f, let d):
            try c.encode("compatibility", forKey: .type); try c.encode(f, forKey: .feature)
            try c.encodeIfPresent(d, forKey: .details)
        case .deprecated(let s, let m):
            try c.encode("deprecated", forKey: .type); try c.encode(s, forKey: .setting)
            try c.encode(m, forKey: .message)
        case .other(let m):
            try c.encode("other", forKey: .type); try c.encode(m, forKey: .message)
        }
    }
}

// MARK: - Tool / ToolChoice

public struct FunctionToolInputExample: Codable, Equatable {
    public var input: [String: JSONValue]

    public init(input: [String: JSONValue]) { self.input = input }
}

/// A user-defined function tool definition.
public struct FunctionTool: Codable, Equatable {
    public var name: String
    public var description: String?
    /// JSON Schema describing the tool's parameters.
    public var inputSchema: JSONValue
    public var strict: Bool?
    public var providerOptions: ProviderOptions?
    public var inputExamples: [FunctionToolInputExample]?

    public init(name: String, inputSchema: JSONValue, description: String? = nil,
                strict: Bool? = nil, providerOptions: ProviderOptions? = nil,
                inputExamples: [FunctionToolInputExample]? = nil) {
        self.name = name; self.inputSchema = inputSchema; self.description = description
        self.strict = strict; self.providerOptions = providerOptions; self.inputExamples = inputExamples
    }
}

/// A provider-defined tool (e.g. `anthropic.web_search_20250305`).
public struct ProviderTool: Codable, Equatable {
    public var id: String
    public var name: String
    public var args: [String: JSONValue]

    public init(id: String, name: String, args: [String: JSONValue]) {
        self.id = id; self.name = name; self.args = args
    }
}

/// A tool: a function tool or a provider tool.
///
/// Wire: internally tagged by `type` (`{"type":"function", …}`, `{"type":"provider", …}`).
public enum Tool: Codable, Equatable {
    case function(FunctionTool)
    case provider(ProviderTool)

    public init(from decoder: Decoder) throws {
        switch try decoder.tag() {
        case "function": self = .function(try FunctionTool(from: decoder))
        case "provider": self = .provider(try ProviderTool(from: decoder))
        case let t: throw decoder.unknownTag(t, in: "tool")
        }
    }

    public func encode(to encoder: Encoder) throws {
        switch self {
        case .function(let v): try v.encode(to: encoder); try encoder.putTag("function")
        case .provider(let v): try v.encode(to: encoder); try encoder.putTag("provider")
        }
    }
}

/// How the model should choose tools.
///
/// Wire: `"auto" | "none" | "required" | {"type":"tool","toolName":"..."}`.
public enum ToolChoice: Codable, Equatable {
    case auto
    case none
    case required
    case tool(toolName: String)

    public init(from decoder: Decoder) throws {
        let c = try decoder.singleValueContainer()
        if let s = try? c.decode(String.self) {
            switch s {
            case "auto": self = .auto; return
            case "none": self = .none; return
            case "required": self = .required; return
            default: throw aimuxDecodingError(c.codingPath, "unknown toolChoice \(s)")
            }
        }
        let o = try decoder.container(keyedBy: AnyCodingKey.self)
        let type = try o.decode(String.self, forKey: AnyCodingKey("type"))
        guard type == "tool" else {
            throw aimuxDecodingError(o.codingPath, "unknown toolChoice type \(type)")
        }
        self = .tool(toolName: try o.decode(String.self, forKey: AnyCodingKey("toolName")))
    }

    public func encode(to encoder: Encoder) throws {
        switch self {
        case .auto: var c = encoder.singleValueContainer(); try c.encode("auto")
        case .none: var c = encoder.singleValueContainer(); try c.encode("none")
        case .required: var c = encoder.singleValueContainer(); try c.encode("required")
        case .tool(let toolName):
            var c = encoder.container(keyedBy: AnyCodingKey.self)
            try c.encode("tool", forKey: AnyCodingKey("type"))
            try c.encode(toolName, forKey: AnyCodingKey("toolName"))
        }
    }
}

/// How the model should format its response.
///
/// Wire: `{"type":"text"}` or `{"type":"json","schema"?,"name"?,"description"?}`.
public enum ResponseFormat: Codable, Equatable {
    case text
    case json(schema: JSONValue? = nil, name: String? = nil, description: String? = nil)

    private enum Field: String, CodingKey { case type, schema, name, description }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: Field.self)
        switch try c.decode(String.self, forKey: .type) {
        case "text": self = .text
        case "json":
            self = .json(schema: try c.decodeIfPresent(JSONValue.self, forKey: .schema),
                         name: try c.decodeIfPresent(String.self, forKey: .name),
                         description: try c.decodeIfPresent(String.self, forKey: .description))
        case let t: throw decoder.unknownTag(t, in: "response format")
        }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: Field.self)
        switch self {
        case .text: try c.encode("text", forKey: .type)
        case .json(let schema, let name, let description):
            try c.encode("json", forKey: .type)
            try c.encodeIfPresent(schema, forKey: .schema)
            try c.encodeIfPresent(name, forKey: .name)
            try c.encodeIfPresent(description, forKey: .description)
        }
    }
}

// MARK: - File data

/// Either raw bytes or a base64-encoded string. Wire: untagged
/// (`[…]` | `"…"`).
public enum FileBytes: Codable, Equatable {
    case binary([UInt8])
    case base64(String)

    public init(from decoder: Decoder) throws {
        let c = try decoder.singleValueContainer()
        if let bytes = try? c.decode([UInt8].self) { self = .binary(bytes); return }
        if let s = try? c.decode(String.self) { self = .base64(s); return }
        throw aimuxDecodingError(c.codingPath, "expected a byte array or a base64 string")
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        switch self {
        case .binary(let bytes): try c.encode(bytes)
        case .base64(let s): try c.encode(s)
        }
    }
}

/// File data as a tagged union. Wire: `{"type":"data","data":<FileBytes>}`,
/// `{"type":"url","url":…,"originalUrl"?}`, `{"type":"reference","reference":{…}}`,
/// `{"type":"text","text":…}`.
public enum FileData: Codable, Equatable {
    case data(data: FileBytes)
    case url(url: String, originalUrl: String? = nil)
    case reference(reference: [String: String])
    case text(text: String)

    private enum Field: String, CodingKey { case type, data, url, originalUrl, reference, text }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: Field.self)
        switch try c.decode(String.self, forKey: .type) {
        case "data": self = .data(data: try c.decode(FileBytes.self, forKey: .data))
        case "url": self = .url(url: try c.decode(String.self, forKey: .url),
                                originalUrl: try c.decodeIfPresent(String.self, forKey: .originalUrl))
        case "reference": self = .reference(reference: try c.decode([String: String].self, forKey: .reference))
        case "text": self = .text(text: try c.decode(String.self, forKey: .text))
        case let t: throw decoder.unknownTag(t, in: "file data")
        }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: Field.self)
        switch self {
        case .data(let d): try c.encode("data", forKey: .type); try c.encode(d, forKey: .data)
        case .url(let u, let o):
            try c.encode("url", forKey: .type); try c.encode(u, forKey: .url)
            try c.encodeIfPresent(o, forKey: .originalUrl)
        case .reference(let r): try c.encode("reference", forKey: .type); try c.encode(r, forKey: .reference)
        case .text(let t): try c.encode("text", forKey: .type); try c.encode(t, forKey: .text)
        }
    }
}

/// Data or a URL returned for a generated file. Wire: `{"type":"data","data":…}`
/// or `{"type":"url","url":…,"originalUrl"?}`.
public enum GeneratedFileData: Codable, Equatable {
    case data(data: FileBytes)
    case url(url: String, originalUrl: String? = nil)

    private enum Field: String, CodingKey { case type, data, url, originalUrl }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: Field.self)
        switch try c.decode(String.self, forKey: .type) {
        case "data": self = .data(data: try c.decode(FileBytes.self, forKey: .data))
        case "url": self = .url(url: try c.decode(String.self, forKey: .url),
                                originalUrl: try c.decodeIfPresent(String.self, forKey: .originalUrl))
        case let t: throw decoder.unknownTag(t, in: "generated file data")
        }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: Field.self)
        switch self {
        case .data(let d): try c.encode("data", forKey: .type); try c.encode(d, forKey: .data)
        case .url(let u, let o):
            try c.encode("url", forKey: .type); try c.encode(u, forKey: .url)
            try c.encodeIfPresent(o, forKey: .originalUrl)
        }
    }
}

public struct GeneratedFile: Codable, Equatable {
    public var data: GeneratedFileData
    public var mediaType: String
    public var providerMetadata: ProviderMetadata?

    public init(data: GeneratedFileData, mediaType: String, providerMetadata: ProviderMetadata? = nil) {
        self.data = data; self.mediaType = mediaType; self.providerMetadata = providerMetadata
    }
}

/// A source / citation. Wire: `{"sourceType":"url","id","url","title"?,…}` or
/// `{"sourceType":"document","id","mediaType","title","filename"?,…}`.
public enum Source: Codable, Equatable {
    case url(id: String, url: String, title: String?, providerMetadata: ProviderMetadata?)
    case document(id: String, mediaType: String, title: String, filename: String?,
                  providerMetadata: ProviderMetadata?)

    private enum Field: String, CodingKey {
        case id, sourceType, url, title, filename, mediaType, providerMetadata
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: Field.self)
        let id = try c.decode(String.self, forKey: .id)
        let pm = try c.decodeIfPresent(ProviderMetadata.self, forKey: .providerMetadata)
        switch try c.decode(String.self, forKey: .sourceType) {
        case "url":
            self = .url(id: id, url: try c.decode(String.self, forKey: .url),
                        title: try c.decodeIfPresent(String.self, forKey: .title), providerMetadata: pm)
        case "document":
            self = .document(id: id, mediaType: try c.decode(String.self, forKey: .mediaType),
                             title: try c.decode(String.self, forKey: .title),
                             filename: try c.decodeIfPresent(String.self, forKey: .filename),
                             providerMetadata: pm)
        case let t: throw decoder.unknownTag(t, in: "source")
        }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: Field.self)
        switch self {
        case .url(let id, let url, let title, let pm):
            try c.encode("url", forKey: .sourceType); try c.encode(id, forKey: .id)
            try c.encode(url, forKey: .url); try c.encodeIfPresent(title, forKey: .title)
            try c.encodeIfPresent(pm, forKey: .providerMetadata)
        case .document(let id, let mediaType, let title, let filename, let pm):
            try c.encode("document", forKey: .sourceType); try c.encode(id, forKey: .id)
            try c.encode(mediaType, forKey: .mediaType); try c.encode(title, forKey: .title)
            try c.encodeIfPresent(filename, forKey: .filename)
            try c.encodeIfPresent(pm, forKey: .providerMetadata)
        }
    }
}

// MARK: - Messages

/// Text content part of a tool-result `content` output.
public struct TextPart: Codable, Equatable {
    public var text: String
    public var providerOptions: ProviderOptions?

    public init(text: String, providerOptions: ProviderOptions? = nil) {
        self.text = text; self.providerOptions = providerOptions
    }
}

/// File content part of a tool-result `content` output.
public struct FilePart: Codable, Equatable {
    public var data: FileData
    public var mediaType: String
    public var filename: String?
    public var providerOptions: ProviderOptions?

    public init(data: FileData, mediaType: String, filename: String? = nil,
                providerOptions: ProviderOptions? = nil) {
        self.data = data; self.mediaType = mediaType; self.filename = filename
        self.providerOptions = providerOptions
    }
}

/// One item of a `content` tool-result output. Wire: tagged by `type`.
public enum ToolResultContent: Codable, Equatable {
    case text(TextPart)
    case file(FilePart)
    case custom(providerOptions: ProviderOptions? = nil)

    private enum Field: String, CodingKey { case type, providerOptions }

    public init(from decoder: Decoder) throws {
        switch try decoder.tag() {
        case "text": self = .text(try TextPart(from: decoder))
        case "file": self = .file(try FilePart(from: decoder))
        case "custom":
            self = .custom(providerOptions: try decoder.container(keyedBy: Field.self)
                .decodeIfPresent(ProviderOptions.self, forKey: .providerOptions))
        case let t: throw decoder.unknownTag(t, in: "tool result content")
        }
    }

    public func encode(to encoder: Encoder) throws {
        switch self {
        case .text(let v): try v.encode(to: encoder); try encoder.putTag("text")
        case .file(let v): try v.encode(to: encoder); try encoder.putTag("file")
        case .custom(let po):
            var c = encoder.container(keyedBy: Field.self)
            try c.encode("custom", forKey: .type); try c.encodeIfPresent(po, forKey: .providerOptions)
        }
    }
}

/// The result of a tool call, as sent back to the model.
///
/// Wire: `{"type":"text","value"}`, `{"type":"json","value"}`,
/// `{"type":"execution-denied","reason"?}`, `{"type":"error-text","value"}`,
/// `{"type":"error-json","value"}`, `{"type":"content","value":[…]}`; all but
/// `content` may carry `providerOptions`.
public enum ToolResultOutput: Codable, Equatable {
    case text(String, providerOptions: ProviderOptions? = nil)
    case json(JSONValue, providerOptions: ProviderOptions? = nil)
    case executionDenied(reason: String? = nil, providerOptions: ProviderOptions? = nil)
    case errorText(String, providerOptions: ProviderOptions? = nil)
    case errorJson(JSONValue, providerOptions: ProviderOptions? = nil)
    case content([ToolResultContent])

    private enum Field: String, CodingKey { case type, value, reason, providerOptions }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: Field.self)
        let po = try c.decodeIfPresent(ProviderOptions.self, forKey: .providerOptions)
        switch try c.decode(String.self, forKey: .type) {
        case "text": self = .text(try c.decode(String.self, forKey: .value), providerOptions: po)
        case "json": self = .json(try c.decode(JSONValue.self, forKey: .value), providerOptions: po)
        case "execution-denied":
            self = .executionDenied(reason: try c.decodeIfPresent(String.self, forKey: .reason), providerOptions: po)
        case "error-text": self = .errorText(try c.decode(String.self, forKey: .value), providerOptions: po)
        case "error-json": self = .errorJson(try c.decode(JSONValue.self, forKey: .value), providerOptions: po)
        case "content": self = .content(try c.decode([ToolResultContent].self, forKey: .value))
        case let t: throw decoder.unknownTag(t, in: "tool result output")
        }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: Field.self)
        switch self {
        case .text(let v, let po):
            try c.encode("text", forKey: .type); try c.encode(v, forKey: .value)
            try c.encodeIfPresent(po, forKey: .providerOptions)
        case .json(let v, let po):
            try c.encode("json", forKey: .type); try c.encode(v, forKey: .value)
            try c.encodeIfPresent(po, forKey: .providerOptions)
        case .executionDenied(let r, let po):
            try c.encode("execution-denied", forKey: .type); try c.encodeIfPresent(r, forKey: .reason)
            try c.encodeIfPresent(po, forKey: .providerOptions)
        case .errorText(let v, let po):
            try c.encode("error-text", forKey: .type); try c.encode(v, forKey: .value)
            try c.encodeIfPresent(po, forKey: .providerOptions)
        case .errorJson(let v, let po):
            try c.encode("error-json", forKey: .type); try c.encode(v, forKey: .value)
            try c.encodeIfPresent(po, forKey: .providerOptions)
        case .content(let v):
            try c.encode("content", forKey: .type); try c.encode(v, forKey: .value)
        }
    }
}

/// A part of a multi-part message.
///
/// Wire: internally tagged by `type` (kebab-case):
/// `{"type":"text","text":"..."}`, `{"type":"tool-call","toolCallId":…}`,
/// `{"type":"tool-result","toolCallId","toolName","output":<ToolResultOutput>}`, …
public enum ContentPart: Codable, Equatable {
    case text(text: String, providerOptions: ProviderOptions? = nil)
    case image(image: [UInt8], mediaType: String, providerOptions: ProviderOptions? = nil)
    case file(data: [UInt8], mediaType: String, filename: String? = nil, providerOptions: ProviderOptions? = nil)
    case fileBase64(data: String, mediaType: String, filename: String? = nil, providerOptions: ProviderOptions? = nil)
    case fileUrl(url: String, mediaType: String, providerOptions: ProviderOptions? = nil)
    case fileReference(mediaType: String, reference: JSONValue, filename: String? = nil,
                       providerOptions: ProviderOptions? = nil)
    case reasoning(text: String, signature: String? = nil, providerOptions: ProviderOptions? = nil)
    case toolCall(toolCallId: String, toolName: String, input: JSONValue,
                  providerExecuted: Bool? = nil, providerOptions: ProviderOptions? = nil)
    case toolResult(toolCallId: String, toolName: String, output: ToolResultOutput,
                    providerOptions: ProviderOptions? = nil)
    case custom(kind: String, providerOptions: ProviderOptions? = nil)
    case reasoningFile(data: GeneratedFileData, mediaType: String, providerOptions: ProviderOptions? = nil)
    case toolApprovalRequest(approvalId: String, toolCallId: String, reason: String? = nil,
                             isAutomatic: Bool? = nil, signature: String? = nil,
                             inputSchemaInput: JSONValue? = nil)

    private enum Field: String, CodingKey {
        case type, text, image, data, mediaType, filename, url, reference, signature
        case toolCallId, toolName, input, output, providerExecuted, providerOptions
        case kind, approvalId, reason, isAutomatic, inputSchemaInput
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: Field.self)
        let po = try c.decodeIfPresent(ProviderOptions.self, forKey: .providerOptions)
        switch try c.decode(String.self, forKey: .type) {
        case "text":
            self = .text(text: try c.decode(String.self, forKey: .text), providerOptions: po)
        case "image":
            self = .image(image: try c.decode([UInt8].self, forKey: .image),
                          mediaType: try c.decode(String.self, forKey: .mediaType), providerOptions: po)
        case "file":
            self = .file(data: try c.decode([UInt8].self, forKey: .data),
                         mediaType: try c.decode(String.self, forKey: .mediaType),
                         filename: try c.decodeIfPresent(String.self, forKey: .filename), providerOptions: po)
        case "file-base64":
            self = .fileBase64(data: try c.decode(String.self, forKey: .data),
                               mediaType: try c.decode(String.self, forKey: .mediaType),
                               filename: try c.decodeIfPresent(String.self, forKey: .filename), providerOptions: po)
        case "file-url":
            self = .fileUrl(url: try c.decode(String.self, forKey: .url),
                            mediaType: try c.decode(String.self, forKey: .mediaType), providerOptions: po)
        case "file-reference":
            self = .fileReference(mediaType: try c.decode(String.self, forKey: .mediaType),
                                  reference: try c.decode(JSONValue.self, forKey: .reference),
                                  filename: try c.decodeIfPresent(String.self, forKey: .filename), providerOptions: po)
        case "reasoning":
            self = .reasoning(text: try c.decode(String.self, forKey: .text),
                              signature: try c.decodeIfPresent(String.self, forKey: .signature), providerOptions: po)
        case "tool-call":
            self = .toolCall(toolCallId: try c.decode(String.self, forKey: .toolCallId),
                             toolName: try c.decode(String.self, forKey: .toolName),
                             input: try c.decode(JSONValue.self, forKey: .input),
                             providerExecuted: try c.decodeIfPresent(Bool.self, forKey: .providerExecuted),
                             providerOptions: po)
        case "tool-result":
            self = .toolResult(toolCallId: try c.decode(String.self, forKey: .toolCallId),
                               toolName: try c.decode(String.self, forKey: .toolName),
                               output: try c.decode(ToolResultOutput.self, forKey: .output), providerOptions: po)
        case "custom":
            self = .custom(kind: try c.decode(String.self, forKey: .kind), providerOptions: po)
        case "reasoning-file":
            self = .reasoningFile(data: try c.decode(GeneratedFileData.self, forKey: .data),
                                  mediaType: try c.decode(String.self, forKey: .mediaType), providerOptions: po)
        case "tool-approval-request":
            self = .toolApprovalRequest(approvalId: try c.decode(String.self, forKey: .approvalId),
                                        toolCallId: try c.decode(String.self, forKey: .toolCallId),
                                        reason: try c.decodeIfPresent(String.self, forKey: .reason),
                                        isAutomatic: try c.decodeIfPresent(Bool.self, forKey: .isAutomatic),
                                        signature: try c.decodeIfPresent(String.self, forKey: .signature),
                                        inputSchemaInput: try c.decodeIfPresent(JSONValue.self, forKey: .inputSchemaInput))
        case let t: throw decoder.unknownTag(t, in: "content part")
        }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: Field.self)
        func tag(_ t: String, _ po: ProviderOptions?) throws {
            try c.encode(t, forKey: .type); try c.encodeIfPresent(po, forKey: .providerOptions)
        }
        switch self {
        case .text(let text, let po):
            try tag("text", po); try c.encode(text, forKey: .text)
        case .image(let image, let mediaType, let po):
            try tag("image", po); try c.encode(image, forKey: .image); try c.encode(mediaType, forKey: .mediaType)
        case .file(let data, let mediaType, let filename, let po):
            try tag("file", po); try c.encode(data, forKey: .data); try c.encode(mediaType, forKey: .mediaType)
            try c.encodeIfPresent(filename, forKey: .filename)
        case .fileBase64(let data, let mediaType, let filename, let po):
            try tag("file-base64", po); try c.encode(data, forKey: .data); try c.encode(mediaType, forKey: .mediaType)
            try c.encodeIfPresent(filename, forKey: .filename)
        case .fileUrl(let url, let mediaType, let po):
            try tag("file-url", po); try c.encode(url, forKey: .url); try c.encode(mediaType, forKey: .mediaType)
        case .fileReference(let mediaType, let reference, let filename, let po):
            try tag("file-reference", po); try c.encode(mediaType, forKey: .mediaType)
            try c.encode(reference, forKey: .reference); try c.encodeIfPresent(filename, forKey: .filename)
        case .reasoning(let text, let signature, let po):
            try tag("reasoning", po); try c.encode(text, forKey: .text)
            try c.encodeIfPresent(signature, forKey: .signature)
        case .toolCall(let id, let name, let input, let pe, let po):
            try tag("tool-call", po); try c.encode(id, forKey: .toolCallId); try c.encode(name, forKey: .toolName)
            try c.encode(input, forKey: .input); try c.encodeIfPresent(pe, forKey: .providerExecuted)
        case .toolResult(let id, let name, let output, let po):
            try tag("tool-result", po); try c.encode(id, forKey: .toolCallId); try c.encode(name, forKey: .toolName)
            try c.encode(output, forKey: .output)
        case .custom(let kind, let po):
            try tag("custom", po); try c.encode(kind, forKey: .kind)
        case .reasoningFile(let data, let mediaType, let po):
            try tag("reasoning-file", po); try c.encode(data, forKey: .data); try c.encode(mediaType, forKey: .mediaType)
        case .toolApprovalRequest(let approvalId, let toolCallId, let reason, let isAutomatic, let signature, let schemaInput):
            try tag("tool-approval-request", nil); try c.encode(approvalId, forKey: .approvalId)
            try c.encode(toolCallId, forKey: .toolCallId); try c.encodeIfPresent(reason, forKey: .reason)
            try c.encodeIfPresent(isAutomatic, forKey: .isAutomatic); try c.encodeIfPresent(signature, forKey: .signature)
            try c.encodeIfPresent(schemaInput, forKey: .inputSchemaInput)
        }
    }
}

/// Message body: a simple string or multi-part content. Wire: untagged.
public enum MessageContent: Codable, Equatable {
    case text(String)
    case parts([ContentPart])

    public init(from decoder: Decoder) throws {
        let c = try decoder.singleValueContainer()
        if let s = try? c.decode(String.self) { self = .text(s); return }
        if let a = try? c.decode([ContentPart].self) { self = .parts(a); return }
        throw aimuxDecodingError(c.codingPath, "expected string or content parts")
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        switch self {
        case .text(let s): try c.encode(s)
        case .parts(let a): try c.encode(a)
        }
    }
}

/// A single user-facing chat message.
public struct ModelMessage: Codable, Equatable {
    public var role: Role
    public var content: MessageContent

    public init(role: Role, content: MessageContent) {
        self.role = role
        self.content = content
    }

    public static func system(_ text: String) -> ModelMessage { ModelMessage(role: .system, content: .text(text)) }
    public static func user(_ text: String) -> ModelMessage { ModelMessage(role: .user, content: .text(text)) }
    public static func assistant(_ text: String) -> ModelMessage { ModelMessage(role: .assistant, content: .text(text)) }
}

/// What the user passes as `prompt`: a plain string or a list of messages.
/// Wire: untagged (`"text"` or `[{...}]`).
public enum ModelPrompt: Codable, Equatable {
    case text(String)
    case messages([ModelMessage])

    public init(from decoder: Decoder) throws {
        let c = try decoder.singleValueContainer()
        if let s = try? c.decode(String.self) { self = .text(s); return }
        if let m = try? c.decode([ModelMessage].self) { self = .messages(m); return }
        throw aimuxDecodingError(c.codingPath, "expected prompt string or messages array")
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        switch self {
        case .text(let s): try c.encode(s)
        case .messages(let m): try c.encode(m)
        }
    }
}

// MARK: - Result types

/// A tool call requested by the model (user-facing).
public struct ToolCall: Codable, Equatable {
    public var toolCallId: String
    public var toolName: String
    /// Parsed arguments (usually an object); the original string when `invalid`
    /// is true and the provider input was not valid JSON.
    public var input: JSONValue
    public var providerExecuted: Bool?
    public var dynamic: Bool?
    /// Additional provider-specific metadata associated with this call.
    public var providerMetadata: ProviderMetadata?
    /// Set when the tool call stays invalid after the optional repair.
    public var invalid: Bool?
    /// The typed lookup, parse, schema, or repair failure for an invalid call:
    /// an `AiMuxError` object tagged by `name` (e.g. `"AI_InvalidToolInputError"`).
    public var error: JSONValue?

    public init(toolCallId: String, toolName: String, input: JSONValue,
                providerExecuted: Bool? = nil, dynamic: Bool? = nil,
                providerMetadata: ProviderMetadata? = nil,
                invalid: Bool? = nil, error: JSONValue? = nil) {
        self.toolCallId = toolCallId; self.toolName = toolName; self.input = input
        self.providerExecuted = providerExecuted; self.dynamic = dynamic
        self.providerMetadata = providerMetadata
        self.invalid = invalid; self.error = error
    }
}

/// A tool result produced by a provider-executed tool, next to the call it answers.
public struct ToolResult: Codable, Equatable {
    public var toolCallId: String
    public var toolName: String
    public var result: JSONValue
    public var isError: Bool?
    public var preliminary: Bool?
    public var dynamic: Bool?
    public var providerMetadata: ProviderMetadata?
}

/// A reasoning / thinking segment (provider metadata carries e.g. the
/// Anthropic signature at `anthropic.signature`).
public struct ReasoningOutput: Codable, Equatable {
    public var text: String
    public var providerMetadata: ProviderMetadata?
}

/// A file generated as part of reasoning (call-layer shape).
public struct ReasoningFileOutput: Codable, Equatable {
    public var file: GeneratedFile
    public var providerMetadata: ProviderMetadata?
}

/// Reasoning text or a generated file produced during reasoning.
public enum ReasoningPart: Codable, Equatable {
    case reasoning(ReasoningOutput)
    case reasoningFile(ReasoningFileOutput)

    public init(from decoder: Decoder) throws {
        switch try decoder.tag() {
        case "reasoning": self = .reasoning(try ReasoningOutput(from: decoder))
        case "reasoning-file": self = .reasoningFile(try ReasoningFileOutput(from: decoder))
        case let t: throw decoder.unknownTag(t, in: "reasoning part")
        }
    }

    public func encode(to encoder: Encoder) throws {
        switch self {
        case .reasoning(let v): try v.encode(to: encoder); try encoder.putTag("reasoning")
        case .reasoningFile(let v): try v.encode(to: encoder); try encoder.putTag("reasoning-file")
        }
    }
}

/// Provider approval request for a provider-executed call.
public struct RawToolApprovalRequest: Codable, Equatable {
    public var approvalId: String
    public var toolCallId: String
    public var providerMetadata: ProviderMetadata?
}

/// Approval request exposed by text generation.
public struct ToolApprovalRequestOutput: Codable, Equatable {
    public var approvalId: String
    public var toolCall: ToolCall
    public var reason: String?
    public var isAutomatic: Bool?
    public var signature: String?
}

/// A content item in a generation result. Wire: internally tagged by `type`;
/// the payload types of `toolCall`, `toolApprovalRequest` and
/// `reasoningFile` differ between the provider-facing and the call-layer
/// shape, hence the generics (see ``RawGenerateContent`` / ``TextContent``).
public enum GenerateContent<
    C: Codable & Equatable, A: Codable & Equatable, R: Codable & Equatable
>: Codable, Equatable {
    case text(text: String, providerMetadata: ProviderMetadata?)
    case toolCall(C)
    case source(Source)
    case reasoning(ReasoningOutput)
    case file(GeneratedFile)
    case reasoningFile(R)
    case custom(kind: String, providerMetadata: ProviderMetadata?)
    case toolApprovalRequest(A)
    case toolResult(ToolResult)

    private enum Field: String, CodingKey { case type, text, kind, providerMetadata }

    public init(from decoder: Decoder) throws {
        let t = try decoder.tag()
        switch t {
        case "text", "custom":
            let c = try decoder.container(keyedBy: Field.self)
            let pm = try c.decodeIfPresent(ProviderMetadata.self, forKey: .providerMetadata)
            self = t == "text"
                ? .text(text: try c.decode(String.self, forKey: .text), providerMetadata: pm)
                : .custom(kind: try c.decode(String.self, forKey: .kind), providerMetadata: pm)
        case "tool-call": self = .toolCall(try C(from: decoder))
        case "source": self = .source(try Source(from: decoder))
        case "reasoning": self = .reasoning(try ReasoningOutput(from: decoder))
        case "file": self = .file(try GeneratedFile(from: decoder))
        case "reasoning-file": self = .reasoningFile(try R(from: decoder))
        case "tool-approval-request": self = .toolApprovalRequest(try A(from: decoder))
        case "tool-result": self = .toolResult(try ToolResult(from: decoder))
        default: throw decoder.unknownTag(t, in: "content")
        }
    }

    public func encode(to encoder: Encoder) throws {
        switch self {
        case .text(let text, let pm):
            var c = encoder.container(keyedBy: Field.self)
            try c.encode("text", forKey: .type); try c.encode(text, forKey: .text)
            try c.encodeIfPresent(pm, forKey: .providerMetadata)
        case .custom(let kind, let pm):
            var c = encoder.container(keyedBy: Field.self)
            try c.encode("custom", forKey: .type); try c.encode(kind, forKey: .kind)
            try c.encodeIfPresent(pm, forKey: .providerMetadata)
        case .toolCall(let v): try v.encode(to: encoder); try encoder.putTag("tool-call")
        case .source(let v): try v.encode(to: encoder); try encoder.putTag("source")
        case .reasoning(let v): try v.encode(to: encoder); try encoder.putTag("reasoning")
        case .file(let v): try v.encode(to: encoder); try encoder.putTag("file")
        case .reasoningFile(let v): try v.encode(to: encoder); try encoder.putTag("reasoning-file")
        case .toolApprovalRequest(let v): try v.encode(to: encoder); try encoder.putTag("tool-approval-request")
        case .toolResult(let v): try v.encode(to: encoder); try encoder.putTag("tool-result")
        }
    }
}

/// Provider-facing content (`GenerateResult.content`): raw tool calls.
public typealias RawGenerateContent = GenerateContent<RawToolCall, RawToolApprovalRequest, GeneratedFile>
/// Call-layer content (`GenerateTextResult.content`): parsed tool calls.
public typealias TextContent = GenerateContent<ToolCall, ToolApprovalRequestOutput, ReasoningFileOutput>

/// Raw provider result (the `raw` field of `GenerateTextResult`).
public struct GenerateResult: Codable, Equatable {
    public var content: [RawGenerateContent]
    public var finishReason: FinishReason
    public var usage: Usage
    public var warnings: [Warning]
    public var providerMetadata: ProviderMetadata?
    public var request: RequestInfo?
    public var response: ResponseInfo?
}

/// Result of `generateText` (user-facing).
public struct GenerateTextResult: Codable, Equatable {
    /// Ordered generated content with parsed tool calls.
    public var content: [TextContent]?
    /// The generated text (concatenated from all text content parts).
    public var text: String
    /// Tool calls requested by the model.
    public var toolCalls: [ToolCall]
    public var finishReason: FinishReason
    public var usage: Usage
    public var warnings: [Warning]
    /// Raw provider result (for advanced use).
    public var raw: GenerateResult
    public var reasoning: [ReasoningPart]
    /// Concatenated reasoning text.
    public var reasoningText: String
    public var sources: [Source]
    public var files: [GeneratedFile]
    /// Assistant messages ready to append for the next turn.
    public var responseMessages: [ModelMessage]
    /// Raw provider-specific finish reason string (e.g. "stop", "end_turn").
    public var rawFinishReason: String?
    public var providerMetadata: ProviderMetadata?
    public var request: RequestInfo
    public var response: ResponseInfo
    /// Total token usage across all steps (single step: equals `usage`).
    public var totalUsage: Usage
}

/// Result of `generateObject`: the parsed JSON object plus convenience fields
/// from the underlying `generateText` call.
public struct GenerateObjectResult: Codable, Equatable {
    /// The parsed JSON object returned by the model.
    public var object: JSONValue
    public var finishReason: FinishReason
    public var rawFinishReason: String?
    public var usage: Usage
    public var warnings: [Warning]
    /// Concatenated reasoning text (if the model produced reasoning/thinking).
    public var reasoning: String?
    public var providerMetadata: ProviderMetadata?
    public var response: ResponseMetadata
    /// The full `generateText` result (for advanced use).
    public var raw: GenerateTextResult
}

/// Aggregated result of `consumeStreamText`; `GenerateTextResult`'s fields
/// without `raw`.
public struct StreamTextResultAggregated: Codable, Equatable {
    public var content: [TextContent]?
    public var text: String
    public var reasoning: [ReasoningPart]
    public var reasoningText: String
    public var toolCalls: [ToolCall]
    public var sources: [Source]
    public var files: [GeneratedFile]
    public var finishReason: FinishReason
    public var rawFinishReason: String?
    public var usage: Usage
    public var totalUsage: Usage
    public var warnings: [Warning]
    public var providerMetadata: ProviderMetadata?
    public var request: RequestInfo
    public var response: ResponseInfo
    public var responseMessages: [ModelMessage]
}

// MARK: - TimeoutConfiguration

/// Per-call timeout configuration. All values are milliseconds; `nil`
/// disables the corresponding limit. A `total` timeout also covers retry
/// backoff and the whole streamed response.
public struct TimeoutConfiguration: Codable, Equatable {
    public var totalMs: UInt64?
    public var stepMs: UInt64?
    public var firstChunkMs: UInt64?
    public var chunkMs: UInt64?

    public init(totalMs: UInt64? = nil, stepMs: UInt64? = nil,
                firstChunkMs: UInt64? = nil, chunkMs: UInt64? = nil) {
        self.totalMs = totalMs; self.stepMs = stepMs
        self.firstChunkMs = firstChunkMs; self.chunkMs = chunkMs
    }
}

// MARK: - GenerateTextOptions

/// User-facing options for `generateText` / `streamText`.
///
/// All fields are optional. Encoding omits `nil` fields, matching the
/// partial-options usage of the raw API.
public struct GenerateTextOptions: Codable, Equatable {
    public var maxOutputTokens: UInt32?
    public var temperature: Double?
    public var stopSequences: [String]?
    public var topP: Double?
    public var topK: Double?
    public var presencePenalty: Double?
    public var frequencyPenalty: Double?
    public var responseFormat: ResponseFormat?
    public var seed: UInt64?
    public var tools: [Tool]?
    public var toolChoice: ToolChoice?
    public var headers: [String: String]?
    public var providerOptions: ProviderOptions?
    public var reasoning: ReasoningEffort?
    public var instructions: String?
    public var maxRetries: UInt32?
    public var timeout: TimeoutConfiguration?
    public var includeRawChunks: Bool?
    public var sessionId: String?

    /// One-shot host-side repair for tool calls the model got wrong
    /// (RFC-0035), mirroring the AI SDK's `repairToolCall`.
    ///
    /// Runs after generation, on every tool call the engine marked invalid, in
    /// document order and at most once per call. Return a corrected
    /// ``RawToolCall`` (it is re-validated: still invalid means the call keeps
    /// an `invalid` flag carrying a `ToolCallRepair` error), `nil` to leave the
    /// call untouched with its original error, or throw to fail the repair
    /// with the thrown error's message as cause. A call made without a tool set
    /// is never repaired and the function is not invoked for it.
    ///
    /// The function runs outside any native call, so it may itself call back
    /// into aimux — asking a model to fix the arguments is the usual reason to
    /// set it. It is host-side only and never appears in the options JSON.
    ///
    /// Applies to `generateText`, `generateObject`, `consumeStreamText`,
    /// `streamText` and `generateTextAsOpenAI` (which repairs the native result
    /// before converting it). `streamTextAsOpenAI` does not reflect repair: its
    /// argument deltas are the provider's text, as in the AI SDK.
    ///
    /// While streaming, the stream waits for the function, and an abort does
    /// not take effect until it returns. If a repair fails at the boundary
    /// (not in the function — a throw is a `ToolCallRepair` error on the
    /// call), `onError` is told and the stream continues without that part.
    public var repairToolCall: RepairToolCall? {
        get { repairToolCallBox?.run }
        set { repairToolCallBox = newValue.map(RepairToolCallBox.init(run:)) }
    }

    /// Not in `CodingKeys`, so it needs its own default for the synthesized
    /// `init(from:)` — and stays out of the options JSON in both directions.
    var repairToolCallBox: RepairToolCallBox? = nil

    enum CodingKeys: String, CodingKey {
        case maxOutputTokens, temperature, stopSequences, topP, topK, presencePenalty
        case frequencyPenalty, responseFormat, seed, tools, toolChoice, headers
        case providerOptions, reasoning, instructions, maxRetries, timeout
        case includeRawChunks, sessionId
    }

    public init(maxOutputTokens: UInt32? = nil, temperature: Double? = nil,
                stopSequences: [String]? = nil, topP: Double? = nil, topK: Double? = nil,
                presencePenalty: Double? = nil, frequencyPenalty: Double? = nil,
                responseFormat: ResponseFormat? = nil, seed: UInt64? = nil,
                tools: [Tool]? = nil, toolChoice: ToolChoice? = nil,
                headers: [String: String]? = nil, providerOptions: ProviderOptions? = nil,
                reasoning: ReasoningEffort? = nil, instructions: String? = nil,
                maxRetries: UInt32? = nil,
                timeout: TimeoutConfiguration? = nil,
                includeRawChunks: Bool? = nil,
                sessionId: String? = nil,
                repairToolCall: RepairToolCall? = nil) {
        self.maxOutputTokens = maxOutputTokens; self.temperature = temperature
        self.stopSequences = stopSequences; self.topP = topP; self.topK = topK
        self.presencePenalty = presencePenalty; self.frequencyPenalty = frequencyPenalty
        self.responseFormat = responseFormat; self.seed = seed; self.tools = tools
        self.toolChoice = toolChoice; self.headers = headers; self.providerOptions = providerOptions
        self.reasoning = reasoning; self.instructions = instructions
        self.maxRetries = maxRetries; self.timeout = timeout
        self.includeRawChunks = includeRawChunks
        self.sessionId = sessionId
        self.repairToolCallBox = repairToolCall.map(RepairToolCallBox.init(run:))
    }
}

// MARK: - TextStreamPart

/// A single chunk in the stream returned by `streamText` (the call-layer
/// `TextStreamPart`: parsed tool calls).
///
/// Wire: internally tagged by `type` — `{"type":"text-delta","id":"…","delta":"…"}`, …
public enum TextStreamPart: Codable, Equatable {
    case textStart(id: String, providerMetadata: ProviderMetadata?)
    case textDelta(id: String, delta: String, providerMetadata: ProviderMetadata?)
    case textEnd(id: String, providerMetadata: ProviderMetadata?)
    case streamStart(warnings: [Warning])
    case finish(finishReason: FinishReason, usage: Usage, providerMetadata: ProviderMetadata?)
    case finishStep(finishReason: FinishReason, usage: Usage, providerMetadata: ProviderMetadata?,
                    response: ResponseInfo)
    /// `error` is an `AiMuxError` object tagged by `name`.
    case error(error: JSONValue)
    case toolInputStart(id: String, toolName: String, providerExecuted: Bool?, dynamic: Bool?,
                        title: String?, providerMetadata: ProviderMetadata?)
    case toolInputDelta(id: String, delta: String, providerMetadata: ProviderMetadata?)
    case toolInputEnd(id: String, providerMetadata: ProviderMetadata?)
    case toolCall(ToolCall)
    case toolResult(ToolResult)
    case file(GeneratedFile)
    case reasoningFile(ReasoningFileOutput)
    case custom(kind: String, providerMetadata: ProviderMetadata?)
    case toolApprovalRequest(ToolApprovalRequestOutput)
    case reasoningStart(id: String, providerMetadata: ProviderMetadata?)
    case reasoningDelta(id: String, delta: String, providerMetadata: ProviderMetadata?)
    case reasoningEnd(id: String, providerMetadata: ProviderMetadata?)
    case source(Source)
    case raw(rawValue: JSONValue)

    private enum Field: String, CodingKey {
        case type, id, delta, warnings, finishReason, usage, providerMetadata, response, error
        case toolName, providerExecuted, dynamic, title, kind, rawValue
    }

    public init(from decoder: Decoder) throws {
        let t = try decoder.tag()
        let c = try decoder.container(keyedBy: Field.self)
        func pm() throws -> ProviderMetadata? { try c.decodeIfPresent(ProviderMetadata.self, forKey: .providerMetadata) }
        func id() throws -> String { try c.decode(String.self, forKey: .id) }
        func delta() throws -> String { try c.decode(String.self, forKey: .delta) }
        switch t {
        case "text-start": self = .textStart(id: try id(), providerMetadata: try pm())
        case "text-delta": self = .textDelta(id: try id(), delta: try delta(), providerMetadata: try pm())
        case "text-end": self = .textEnd(id: try id(), providerMetadata: try pm())
        case "stream-start": self = .streamStart(warnings: try c.decode([Warning].self, forKey: .warnings))
        case "finish":
            self = .finish(finishReason: try c.decode(FinishReason.self, forKey: .finishReason),
                           usage: try c.decode(Usage.self, forKey: .usage), providerMetadata: try pm())
        case "finish-step":
            self = .finishStep(finishReason: try c.decode(FinishReason.self, forKey: .finishReason),
                               usage: try c.decode(Usage.self, forKey: .usage), providerMetadata: try pm(),
                               response: try c.decode(ResponseInfo.self, forKey: .response))
        case "error": self = .error(error: try c.decode(JSONValue.self, forKey: .error))
        case "tool-input-start":
            self = .toolInputStart(id: try id(), toolName: try c.decode(String.self, forKey: .toolName),
                                   providerExecuted: try c.decodeIfPresent(Bool.self, forKey: .providerExecuted),
                                   dynamic: try c.decodeIfPresent(Bool.self, forKey: .dynamic),
                                   title: try c.decodeIfPresent(String.self, forKey: .title), providerMetadata: try pm())
        case "tool-input-delta": self = .toolInputDelta(id: try id(), delta: try delta(), providerMetadata: try pm())
        case "tool-input-end": self = .toolInputEnd(id: try id(), providerMetadata: try pm())
        case "tool-call": self = .toolCall(try ToolCall(from: decoder))
        case "tool-result": self = .toolResult(try ToolResult(from: decoder))
        case "file": self = .file(try GeneratedFile(from: decoder))
        case "reasoning-file": self = .reasoningFile(try ReasoningFileOutput(from: decoder))
        case "custom": self = .custom(kind: try c.decode(String.self, forKey: .kind), providerMetadata: try pm())
        case "tool-approval-request": self = .toolApprovalRequest(try ToolApprovalRequestOutput(from: decoder))
        case "reasoning-start": self = .reasoningStart(id: try id(), providerMetadata: try pm())
        case "reasoning-delta": self = .reasoningDelta(id: try id(), delta: try delta(), providerMetadata: try pm())
        case "reasoning-end": self = .reasoningEnd(id: try id(), providerMetadata: try pm())
        case "source": self = .source(try Source(from: decoder))
        case "raw": self = .raw(rawValue: try c.decode(JSONValue.self, forKey: .rawValue))
        default: throw decoder.unknownTag(t, in: "stream part")
        }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: Field.self)
        func head(_ t: String, id: String? = nil, delta: String? = nil, _ pm: ProviderMetadata? = nil) throws {
            try c.encode(t, forKey: .type)
            try c.encodeIfPresent(id, forKey: .id); try c.encodeIfPresent(delta, forKey: .delta)
            try c.encodeIfPresent(pm, forKey: .providerMetadata)
        }
        switch self {
        case .textStart(let i, let pm): try head("text-start", id: i, pm)
        case .textDelta(let i, let d, let pm): try head("text-delta", id: i, delta: d, pm)
        case .textEnd(let i, let pm): try head("text-end", id: i, pm)
        case .streamStart(let w): try head("stream-start"); try c.encode(w, forKey: .warnings)
        case .finish(let fr, let u, let pm):
            try head("finish", pm); try c.encode(fr, forKey: .finishReason); try c.encode(u, forKey: .usage)
        case .finishStep(let fr, let u, let pm, let r):
            try head("finish-step", pm); try c.encode(fr, forKey: .finishReason); try c.encode(u, forKey: .usage)
            try c.encode(r, forKey: .response)
        case .error(let e): try head("error"); try c.encode(e, forKey: .error)
        case .toolInputStart(let i, let n, let pe, let dyn, let title, let pm):
            try head("tool-input-start", id: i, pm); try c.encode(n, forKey: .toolName)
            try c.encodeIfPresent(pe, forKey: .providerExecuted); try c.encodeIfPresent(dyn, forKey: .dynamic)
            try c.encodeIfPresent(title, forKey: .title)
        case .toolInputDelta(let i, let d, let pm): try head("tool-input-delta", id: i, delta: d, pm)
        case .toolInputEnd(let i, let pm): try head("tool-input-end", id: i, pm)
        case .toolCall(let v): try v.encode(to: encoder); try encoder.putTag("tool-call")
        case .toolResult(let v): try v.encode(to: encoder); try encoder.putTag("tool-result")
        case .file(let v): try v.encode(to: encoder); try encoder.putTag("file")
        case .reasoningFile(let v): try v.encode(to: encoder); try encoder.putTag("reasoning-file")
        case .custom(let k, let pm): try head("custom", pm); try c.encode(k, forKey: .kind)
        case .toolApprovalRequest(let v): try v.encode(to: encoder); try encoder.putTag("tool-approval-request")
        case .reasoningStart(let i, let pm): try head("reasoning-start", id: i, pm)
        case .reasoningDelta(let i, let d, let pm): try head("reasoning-delta", id: i, delta: d, pm)
        case .reasoningEnd(let i, let pm): try head("reasoning-end", id: i, pm)
        case .source(let v): try v.encode(to: encoder); try encoder.putTag("source")
        case .raw(let v): try head("raw"); try c.encode(v, forKey: .rawValue)
        }
    }
}

// MARK: - OpenAI Chat Completions output (RFC-0026)

/// A complete Chat Completion response (non-streaming).
///
/// Mirrors the OpenAI `chat.completion` object (`aimux-core`:
/// `ChatCompletion`). Produced by `generateTextAsOpenAI`.
public struct ChatCompletion: Codable, Equatable {
    public var id: String
    public var object: String
    public var created: UInt64
    public var model: String
    public var choices: [ChatCompletionChoice]
    public var usage: ChatCompletionUsage
    public var systemFingerprint: String?

    enum CodingKeys: String, CodingKey {
        case id, object, created, model, choices, usage
        case systemFingerprint = "system_fingerprint"
    }

    public init(id: String, object: String, created: UInt64, model: String,
                choices: [ChatCompletionChoice], usage: ChatCompletionUsage,
                systemFingerprint: String? = nil) {
        self.id = id; self.object = object; self.created = created
        self.model = model; self.choices = choices; self.usage = usage
        self.systemFingerprint = systemFingerprint
    }
}

public struct ChatCompletionChoice: Codable, Equatable {
    public var index: UInt32
    public var message: ChatCompletionMessage
    public var finishReason: String?
    /// Raw `logprobs` payload (arbitrary JSON).
    public var logprobs: JSONValue?

    enum CodingKeys: String, CodingKey {
        case index, message
        case finishReason = "finish_reason"
        case logprobs
    }

    public init(index: UInt32, message: ChatCompletionMessage,
                finishReason: String? = nil, logprobs: JSONValue? = nil) {
        self.index = index; self.message = message
        self.finishReason = finishReason; self.logprobs = logprobs
    }
}

public struct ChatCompletionMessage: Codable, Equatable {
    public var role: String
    public var content: String?
    public var reasoningContent: String?
    public var toolCalls: [ChatCompletionToolCall]?
    /// Raw `annotations` payload (array of arbitrary JSON).
    public var annotations: [JSONValue]?

    enum CodingKeys: String, CodingKey {
        case role, content
        case reasoningContent = "reasoning_content"
        case toolCalls = "tool_calls"
        case annotations
    }

    public init(role: String, content: String? = nil, reasoningContent: String? = nil,
                toolCalls: [ChatCompletionToolCall]? = nil, annotations: [JSONValue]? = nil) {
        self.role = role; self.content = content; self.reasoningContent = reasoningContent
        self.toolCalls = toolCalls; self.annotations = annotations
    }
}

/// A tool call in a `ChatCompletionMessage`.
///
/// Wire: `{"id","type":"function","function":{"name","arguments"}}`.
/// The `type` field is JSON `"type"` (Rust `#[serde(rename = "type")]`).
public struct ChatCompletionToolCall: Codable, Equatable {
    public var id: String
    /// Wire key `"type"`.
    public var toolType: String
    public var function: ChatCompletionFunction

    enum CodingKeys: String, CodingKey {
        case id
        case toolType = "type"
        case function
    }

    public init(id: String, toolType: String, function: ChatCompletionFunction) {
        self.id = id; self.toolType = toolType; self.function = function
    }
}

public struct ChatCompletionFunction: Codable, Equatable {
    public var name: String
    public var arguments: String

    public init(name: String, arguments: String) {
        self.name = name; self.arguments = arguments
    }
}

/// A single Chat Completion chunk (streaming).
///
/// Mirrors the OpenAI `chat.completion.chunk` object (`aimux-core`:
/// `ChatCompletionChunk`). Emitted by `streamTextAsOpenAI`.
public struct ChatCompletionChunk: Codable, Equatable {
    public var id: String
    public var object: String
    public var created: UInt64
    public var model: String
    public var choices: [ChatCompletionChunkChoice]
    public var usage: ChatCompletionUsage?

    public init(id: String, object: String, created: UInt64, model: String,
                choices: [ChatCompletionChunkChoice], usage: ChatCompletionUsage? = nil) {
        self.id = id; self.object = object; self.created = created
        self.model = model; self.choices = choices; self.usage = usage
    }
}

public struct ChatCompletionChunkChoice: Codable, Equatable {
    public var index: UInt32
    public var delta: ChatCompletionDelta
    public var finishReason: String?
    public var logprobs: JSONValue?

    enum CodingKeys: String, CodingKey {
        case index, delta
        case finishReason = "finish_reason"
        case logprobs
    }

    public init(index: UInt32, delta: ChatCompletionDelta,
                finishReason: String? = nil, logprobs: JSONValue? = nil) {
        self.index = index; self.delta = delta
        self.finishReason = finishReason; self.logprobs = logprobs
    }
}

public struct ChatCompletionDelta: Codable, Equatable {
    public var role: String?
    public var content: String?
    public var reasoningContent: String?
    public var toolCalls: [ChatCompletionChunkToolCall]?

    enum CodingKeys: String, CodingKey {
        case role, content
        case reasoningContent = "reasoning_content"
        case toolCalls = "tool_calls"
    }

    public init(role: String? = nil, content: String? = nil,
                reasoningContent: String? = nil, toolCalls: [ChatCompletionChunkToolCall]? = nil) {
        self.role = role; self.content = content; self.reasoningContent = reasoningContent
        self.toolCalls = toolCalls
    }
}

/// A tool call delta in a `ChatCompletionChunk`.
///
/// Wire: `{"index","id"?,"type":"function"?,"function":{"name"?,"arguments"?}}`.
/// The `type` field is JSON `"type"` (Rust `#[serde(rename = "type")]`).
public struct ChatCompletionChunkToolCall: Codable, Equatable {
    public var index: UInt32
    public var id: String?
    /// Wire key `"type"`.
    public var toolType: String?
    public var function: ChatCompletionChunkFunction

    enum CodingKeys: String, CodingKey {
        case index, id
        case toolType = "type"
        case function
    }

    public init(index: UInt32, id: String? = nil, toolType: String? = nil,
                function: ChatCompletionChunkFunction) {
        self.index = index; self.id = id; self.toolType = toolType; self.function = function
    }
}

public struct ChatCompletionChunkFunction: Codable, Equatable {
    public var name: String?
    public var arguments: String?

    public init(name: String? = nil, arguments: String? = nil) {
        self.name = name; self.arguments = arguments
    }
}

public struct ChatCompletionUsage: Codable, Equatable {
    public var promptTokens: UInt32
    public var completionTokens: UInt32
    public var totalTokens: UInt32
    public var promptTokensDetails: PromptTokensDetails?
    public var completionTokensDetails: CompletionTokensDetails?

    enum CodingKeys: String, CodingKey {
        case promptTokens = "prompt_tokens"
        case completionTokens = "completion_tokens"
        case totalTokens = "total_tokens"
        case promptTokensDetails = "prompt_tokens_details"
        case completionTokensDetails = "completion_tokens_details"
    }

    public init(promptTokens: UInt32, completionTokens: UInt32, totalTokens: UInt32,
                promptTokensDetails: PromptTokensDetails? = nil,
                completionTokensDetails: CompletionTokensDetails? = nil) {
        self.promptTokens = promptTokens; self.completionTokens = completionTokens
        self.totalTokens = totalTokens
        self.promptTokensDetails = promptTokensDetails
        self.completionTokensDetails = completionTokensDetails
    }
}

public struct PromptTokensDetails: Codable, Equatable {
    public var cachedTokens: UInt32
    public var cacheWriteTokens: UInt32?

    enum CodingKeys: String, CodingKey {
        case cachedTokens = "cached_tokens"
        case cacheWriteTokens = "cache_write_tokens"
    }

    public init(cachedTokens: UInt32, cacheWriteTokens: UInt32? = nil) {
        self.cachedTokens = cachedTokens; self.cacheWriteTokens = cacheWriteTokens
    }
}

public struct CompletionTokensDetails: Codable, Equatable {
    public var reasoningTokens: UInt32?

    enum CodingKeys: String, CodingKey {
        case reasoningTokens = "reasoning_tokens"
    }

    public init(reasoningTokens: UInt32? = nil) {
        self.reasoningTokens = reasoningTokens
    }
}

// MARK: - Typed wrapper methods (extra layer over the raw C-ABI API)

public extension Model {

    /// Generate text (non-streaming) with typed inputs/outputs.
    ///
    /// - Parameters:
    ///   - prompt: A `ModelPrompt` — a plain string (`.text`) or a message list
    ///     (`.messages`), serialized to the JSON shape the FFI expects.
    ///   - options: Optional `GenerateTextOptions`.
    /// - Returns: A decoded `GenerateTextResult`.
    func generateText(
        prompt: ModelPrompt,
        options: GenerateTextOptions? = nil
    ) throws -> GenerateTextResult {
        let promptJson = try AimuxCodable.jsonString(for: prompt)
        let optsJson = try options.map { try AimuxCodable.jsonString(for: $0) }
        let resultJson = try repairedResultJson(
            generateText(prompt: promptJson, options: optsJson),
            promptJson: promptJson, optsJson: optsJson, options: options
        )
        return try JSONDecoder().decode(GenerateTextResult.self, from: Data(resultJson.utf8))
    }

    /// Generate a structured JSON object with typed inputs/outputs (M12, RFC-0016).
    ///
    /// Same signature as the typed `generateText`; returns a decoded
    /// `GenerateObjectResult`. Pass `responseFormat: .json(schema: …)`
    /// via `options` for schema control; the engine applies JSON repair
    /// before parsing.
    ///
    /// - Parameters:
    ///   - prompt: A `ModelPrompt` — a plain string (`.text`) or a message list
    ///     (`.messages`), serialized to the JSON shape the FFI expects.
    ///   - options: Optional `GenerateTextOptions`.
    /// - Returns: A decoded `GenerateObjectResult`.
    func generateObject(
        prompt: ModelPrompt,
        options: GenerateTextOptions? = nil
    ) throws -> GenerateObjectResult {
        let promptJson = try AimuxCodable.jsonString(for: prompt)
        let optsJson = try options.map { try AimuxCodable.jsonString(for: $0) }
        let resultJson = try repairedResultJson(
            generateObject(prompt: promptJson, options: optsJson),
            promptJson: promptJson, optsJson: optsJson, options: options
        )
        return try JSONDecoder().decode(GenerateObjectResult.self, from: Data(resultJson.utf8))
    }

    /// Consume a stream to completion and return the aggregated typed result
    /// (M11, RFC-0016). Synchronous (blocks until the stream finishes).
    ///
    /// - Parameters:
    ///   - prompt: A `ModelPrompt` — a plain string (`.text`) or a message list
    ///     (`.messages`), serialized to the JSON shape the FFI expects.
    ///   - options: Optional `GenerateTextOptions`.
    /// - Returns: A decoded `StreamTextResultAggregated`.
    func consumeStreamText(
        prompt: ModelPrompt,
        options: GenerateTextOptions? = nil
    ) throws -> StreamTextResultAggregated {
        let promptJson = try AimuxCodable.jsonString(for: prompt)
        let optsJson = try options.map { try AimuxCodable.jsonString(for: $0) }
        let resultJson = try repairedResultJson(
            consumeStreamText(prompt: promptJson, options: optsJson),
            promptJson: promptJson, optsJson: optsJson, options: options
        )
        return try JSONDecoder().decode(StreamTextResultAggregated.self, from: Data(resultJson.utf8))
    }

    /// Stream text with typed `TextStreamPart`s.
    ///
    /// Each raw JSON-string part is decoded into a `TextStreamPart` before being
    /// passed to `onPart`. A part that fails to decode is reported via
    /// `onError` (the stream otherwise continues until done/error).
    func streamText(
        prompt: ModelPrompt,
        options: GenerateTextOptions? = nil,
        onPart: @escaping (TextStreamPart) -> Void,
        onDone: @escaping () -> Void,
        onError: @escaping (any Error) -> Void
    ) {
        let promptJson: String
        let optsJson: String?
        do {
            promptJson = try AimuxCodable.jsonString(for: prompt)
            optsJson = try options.map { try AimuxCodable.jsonString(for: $0) }
        } catch {
            onError(error) // EncodingError from JSONEncoder
            return
        }
        streamText(prompt: promptJson, options: optsJson,
                   onPart: { json in
                       do {
                           // An invalid tool-call part is replaced by its
                           // repaired form; every other part — argument deltas
                           // above all — passes through untouched.
                           let partJson = try repairedStreamPartJson(
                               json, promptJson: promptJson, optsJson: optsJson, options: options
                           )
                           try onPart(JSONDecoder().decode(TextStreamPart.self, from: Data(partJson.utf8)))
                       } catch {
                           onError(error) // DecodingError from JSONDecoder, AimuxError from repair
                       }
                   },
                   onDone: onDone,
                   onError: onError)
    }

    /// Stream text as an `AsyncSequence` of typed `TextStreamPart`s.
    ///
    /// The stream finishes on normal completion and throws `AimuxError`
    /// (AiMuxError failure, C codes preserved via `fromC`) or the native
    /// `EncodingError` / `DecodingError` when typed (de)serialization fails.
    func streamTextAsync(
        prompt: ModelPrompt,
        options: GenerateTextOptions? = nil
    ) -> AsyncThrowingStream<TextStreamPart, Error> {
        AsyncThrowingStream { continuation in
            self.streamText(
                prompt: prompt, options: options,
                onPart: { continuation.yield($0) },
                onDone: { continuation.finish() },
                onError: { continuation.finish(throwing: $0) }
            )
        }
    }

    // MARK: OpenAI-compatible output (RFC-0026)

    /// Generate text (non-streaming) with OpenAI Chat Completions output.
    ///
    /// - Parameters:
    ///   - prompt: A `ModelPrompt` — a plain string (`.text`) or a message list
    ///     (`.messages`), serialized to the JSON shape the FFI expects.
    ///   - options: Optional `GenerateTextOptions`.
    /// - Returns: A decoded `ChatCompletion`.
    func generateTextAsOpenAI(
        prompt: ModelPrompt,
        options: GenerateTextOptions? = nil
    ) throws -> ChatCompletion {
        let promptJson = try AimuxCodable.jsonString(for: prompt)
        let optsJson = try options.map { try AimuxCodable.jsonString(for: $0) }
        let completionJson: String
        if options?.repairToolCall == nil {
            completionJson = try generateTextAsOpenAI(prompt: promptJson, options: optsJson)
        } else {
            // A ChatCompletion has no invalid marker: repair the native result,
            // then convert it.
            let resultJson = try repairedResultJson(
                generateText(prompt: promptJson, options: optsJson),
                promptJson: promptJson, optsJson: optsJson, options: options
            )
            completionJson = try generateTextResultAsOpenAI(resultJson)
        }
        return try JSONDecoder().decode(ChatCompletion.self, from: Data(completionJson.utf8))
    }

    /// Stream text with OpenAI Chat Completions output, yielding typed
    /// `ChatCompletionChunk`s.
    ///
    /// Each raw JSON-string chunk is decoded into a `ChatCompletionChunk`
    /// before being passed to `onPart`. A chunk that fails to decode is
    /// reported via `onError` (the stream otherwise continues until
    /// done/error). Stream options (`include_usage`, `include_reasoning`) are
    /// passed via `options.providerOptions.openai.stream_options`.
    func streamTextAsOpenAI(
        prompt: ModelPrompt,
        options: GenerateTextOptions? = nil,
        onPart: @escaping (ChatCompletionChunk) -> Void,
        onDone: @escaping () -> Void,
        onError: @escaping (any Error) -> Void
    ) {
        let promptJson: String
        let optsJson: String?
        do {
            promptJson = try AimuxCodable.jsonString(for: prompt)
            optsJson = try options.map { try AimuxCodable.jsonString(for: $0) }
        } catch {
            onError(error) // EncodingError from JSONEncoder
            return
        }
        streamTextAsOpenAI(prompt: promptJson, options: optsJson,
                           onPart: { json in
                               do {
                                   try onPart(JSONDecoder().decode(ChatCompletionChunk.self, from: Data(json.utf8)))
                               } catch {
                                   onError(error) // DecodingError from JSONDecoder
                               }
                           },
                           onDone: onDone,
                           onError: onError)
    }

    /// Stream text with OpenAI Chat Completions output as an `AsyncSequence`
    /// of typed `ChatCompletionChunk`s (RFC-0026).
    func streamTextAsOpenAIAsync(
        prompt: ModelPrompt,
        options: GenerateTextOptions? = nil
    ) -> AsyncThrowingStream<ChatCompletionChunk, Error> {
        AsyncThrowingStream { continuation in
            self.streamTextAsOpenAI(
                prompt: prompt, options: options,
                onPart: { continuation.yield($0) },
                onDone: { continuation.finish() },
                onError: { continuation.finish(throwing: $0) }
            )
        }
    }
}

/// JSON (de)serialization helpers shared by the typed wrapper.
fileprivate enum AimuxCodable {
    /// Encode an `Encodable` value to a JSON string (the wire is camelCase, so
    /// the default key strategy applies).
    static func jsonString<T: Encodable>(for value: T) throws -> String {
        let data = try JSONEncoder().encode(value)
        return String(data: data, encoding: .utf8) ?? ""
    }
}
