// Types.swift — typed Codable wrapper layer over the aimux-ffi C ABI.
//
// The raw API in `Aimux.swift` exchanges JSON strings across the Swift↔C
// boundary (`generateText(prompt:options:) -> String`, `streamText` yields
// JSON-string `StreamPart`s). This file adds a thin, *typed* layer on top:
// inputs and outputs are `Codable` Swift structs/enums mirroring
// `bindings/node/src/types/*.ts` (the ts-rs types generated from the Rust serde
// definitions). The raw API is left untouched; the typed methods live in a
// `Model` extension and delegate to the raw ones.
//
// Wire conventions mirrored here (all derived from the Rust serde attributes):
//   • struct fields are snake_case on the wire → Swift camelCase via CodingKeys
//   • `Tool` / `ContentPart`  → internally tagged by `type` (variant = snake_case)
//   • `ToolChoice`           → mixed: "auto"|"none"|"required" or {"type":"tool","toolName":...}
//                              (note: `toolName` is camelCase on the wire)
//   • `ResponseFormat`       → external tag with a unit variant: "Text" or {"Json":{...}}
//   • `ModelPrompt` / `MessageContent` → untagged (string | array)
//   • `GenerateContent` / `StreamPart` / `Warning` → external tag {"Variant":{...}}

import CAimuxFFI
import Foundation

// MARK: - JSONValue (arbitrary JSON, for `input`/`output`/`raw`/`providerMetadata`/…)

/// A type-erased JSON value that round-trips through `JSONEncoder`/`JSONDecoder`.
///
/// On this toolchain `Bool` and `Double` decode are mutually exclusive (a JSON
/// bool fails `Double` decode and a JSON number fails `Bool` decode), so the
/// scalar ordering below is unambiguous.
public enum JSONValue: Codable, Equatable {
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

// MARK: - AnyCodingKey (a CodingKey that can hold any string tag/field)

/// A `CodingKey` accepting any string, used to decode/encode externally- and
/// internally-tagged enums whose tag/field names are not known at compile time.
fileprivate struct AnyCodingKey: CodingKey {
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
fileprivate func aimuxDecodingError(_ codingPath: [any CodingKey], _ message: String) -> DecodingError {
    DecodingError.dataCorrupted(.init(codingPath: codingPath, debugDescription: message, underlyingError: nil))
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

    public init(unified: FinishReasonUnified, raw: String? = nil) {
        self.unified = unified
        self.raw = raw
    }
}

/// Token usage detail (cache breakdown). All fields optional on the wire.
public struct TokenUsage: Codable, Equatable {
    public var total: UInt32?
    public var noCache: UInt32?
    public var cacheRead: UInt32?
    public var cacheWrite: UInt32?
    public var text: UInt32?
    public var reasoning: UInt32?

    enum CodingKeys: String, CodingKey {
        case total
        case noCache = "no_cache"
        case cacheRead = "cache_read"
        case cacheWrite = "cache_write"
        case text
        case reasoning
    }

    public init(total: UInt32? = nil, noCache: UInt32? = nil, cacheRead: UInt32? = nil,
                cacheWrite: UInt32? = nil, text: UInt32? = nil, reasoning: UInt32? = nil) {
        self.total = total; self.noCache = noCache; self.cacheRead = cacheRead
        self.cacheWrite = cacheWrite; self.text = text; self.reasoning = reasoning
    }
}

/// Token usage statistics.
public struct Usage: Codable, Equatable {
    public var inputTokens: TokenUsage
    public var outputTokens: TokenUsage
    /// Opaque, provider-specific raw usage.
    public var raw: JSONValue?

    enum CodingKeys: String, CodingKey {
        case inputTokens = "input_tokens"
        case outputTokens = "output_tokens"
        case raw
    }

    public init(inputTokens: TokenUsage, outputTokens: TokenUsage, raw: JSONValue? = nil) {
        self.inputTokens = inputTokens
        self.outputTokens = outputTokens
        self.raw = raw
    }
}

/// Metadata about the API response.
public struct ResponseMetadata: Codable, Equatable {
    public var id: String?
    public var timestamp: String?
    public var modelId: String?

    enum CodingKeys: String, CodingKey {
        case id, timestamp
        case modelId = "model_id"
    }

    public init(id: String? = nil, timestamp: String? = nil, modelId: String? = nil) {
        self.id = id; self.timestamp = timestamp; self.modelId = modelId
    }
}

/// A provider warning. Wire: externally tagged.
public enum Warning: Codable, Equatable {
    case unsupported(feature: String, details: String?)
    case compatibility(feature: String, details: String?)
    case deprecated(setting: String, message: String)
    case other(message: String)

    private enum Tag { static let unsupported = "Unsupported"; static let compatibility = "Compatibility"
        static let deprecated = "Deprecated"; static let other = "Other" }
    private enum Field: String, CodingKey {
        case feature, details, setting, message
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: AnyCodingKey.self)
        guard let key = c.allKeys.first?.stringValue else {
            throw aimuxDecodingError(c.codingPath, "missing warning tag")
        }
        let n = try c.nestedContainer(keyedBy: Field.self, forKey: AnyCodingKey(key))
        switch key {
        case Tag.unsupported: self = .unsupported(feature: try n.decode(String.self, forKey: .feature),
                                                  details: try n.decodeIfPresent(String.self, forKey: .details))
        case Tag.compatibility: self = .compatibility(feature: try n.decode(String.self, forKey: .feature),
                                                      details: try n.decodeIfPresent(String.self, forKey: .details))
        case Tag.deprecated: self = .deprecated(setting: try n.decode(String.self, forKey: .setting),
                                                message: try n.decode(String.self, forKey: .message))
        case Tag.other: self = .other(message: try n.decode(String.self, forKey: .message))
        default: throw aimuxDecodingError(c.codingPath, "unknown warning tag \(key)")
        }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: AnyCodingKey.self)
        switch self {
        case .unsupported(let f, let d):
            var n = c.nestedContainer(keyedBy: Field.self, forKey: AnyCodingKey(Tag.unsupported))
            try n.encode(f, forKey: .feature); try n.encodeIfPresent(d, forKey: .details)
        case .compatibility(let f, let d):
            var n = c.nestedContainer(keyedBy: Field.self, forKey: AnyCodingKey(Tag.compatibility))
            try n.encode(f, forKey: .feature); try n.encodeIfPresent(d, forKey: .details)
        case .deprecated(let s, let m):
            var n = c.nestedContainer(keyedBy: Field.self, forKey: AnyCodingKey(Tag.deprecated))
            try n.encode(s, forKey: .setting); try n.encode(m, forKey: .message)
        case .other(let m):
            var n = c.nestedContainer(keyedBy: Field.self, forKey: AnyCodingKey(Tag.other))
            try n.encode(m, forKey: .message)
        }
    }
}

// MARK: - Tool / ToolChoice

/// A user-defined function tool definition.
public struct FunctionTool: Codable, Equatable {
    public var name: String
    public var description: String?
    /// JSON Schema describing the tool's parameters.
    public var inputSchema: JSONValue
    public var strict: Bool?
    public var providerOptions: JSONValue?
    public var inputExamples: [JSONValue]?

    enum CodingKeys: String, CodingKey {
        case name, description
        case inputSchema = "input_schema"
        case strict
        case providerOptions = "provider_options"
        case inputExamples = "input_examples"
    }

    public init(name: String, inputSchema: JSONValue, description: String? = nil,
                strict: Bool? = nil, providerOptions: JSONValue? = nil,
                inputExamples: [JSONValue]? = nil) {
        self.name = name; self.inputSchema = inputSchema; self.description = description
        self.strict = strict; self.providerOptions = providerOptions; self.inputExamples = inputExamples
    }
}

/// A provider-defined tool (e.g. `anthropic.web_search_20250305`).
public struct ProviderTool: Codable, Equatable {
    public var id: String
    public var name: String
    public var args: JSONValue

    public init(id: String, name: String, args: JSONValue) {
        self.id = id; self.name = name; self.args = args
    }
}

/// A tool: a function tool or a provider tool.
///
/// Wire: internally tagged by `type`, variant names snake_case
/// (`{"type":"function", ...}`, `{"type":"provider", ...}`).
public enum Tool: Codable, Equatable {
    case function(FunctionTool)
    case provider(ProviderTool)

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: AnyCodingKey.self)
        let type = try c.decode(String.self, forKey: AnyCodingKey("type"))
        switch type {
        case "function":
            self = .function(FunctionTool(
                name: try c.decode(String.self, forKey: AnyCodingKey("name")),
                inputSchema: try c.decode(JSONValue.self, forKey: AnyCodingKey("input_schema")),
                description: try c.decodeIfPresent(String.self, forKey: AnyCodingKey("description")),
                strict: try c.decodeIfPresent(Bool.self, forKey: AnyCodingKey("strict")),
                providerOptions: try c.decodeIfPresent(JSONValue.self, forKey: AnyCodingKey("provider_options")),
                inputExamples: try c.decodeIfPresent([JSONValue].self, forKey: AnyCodingKey("input_examples"))))
        case "provider":
            self = .provider(ProviderTool(
                id: try c.decode(String.self, forKey: AnyCodingKey("id")),
                name: try c.decode(String.self, forKey: AnyCodingKey("name")),
                args: try c.decode(JSONValue.self, forKey: AnyCodingKey("args"))))
        default:
            throw aimuxDecodingError(c.codingPath, "unknown tool type \(type)")
        }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: AnyCodingKey.self)
        switch self {
        case .function(let ft):
            try c.encode("function", forKey: AnyCodingKey("type"))
            try c.encode(ft.name, forKey: AnyCodingKey("name"))
            try c.encode(ft.inputSchema, forKey: AnyCodingKey("input_schema"))
            try c.encodeIfPresent(ft.description, forKey: AnyCodingKey("description"))
            try c.encodeIfPresent(ft.strict, forKey: AnyCodingKey("strict"))
            try c.encodeIfPresent(ft.providerOptions, forKey: AnyCodingKey("provider_options"))
            try c.encodeIfPresent(ft.inputExamples, forKey: AnyCodingKey("input_examples"))
        case .provider(let pt):
            try c.encode("provider", forKey: AnyCodingKey("type"))
            try c.encode(pt.id, forKey: AnyCodingKey("id"))
            try c.encode(pt.name, forKey: AnyCodingKey("name"))
            try c.encode(pt.args, forKey: AnyCodingKey("args"))
        }
    }
}

/// How the model should choose tools.
///
/// Wire: `"auto" | "none" | "required" | {"type":"tool","toolName":"..."}`
/// (the `toolName` field is camelCase on the wire).
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
/// Wire: external tag with a unit variant — `"Text"` or `{"Json":{...}}`.
public enum ResponseFormat: Codable, Equatable {
    case text
    case json(schema: JSONValue?, name: String?, description: String?)

    private enum Field: String, CodingKey { case schema, name, description }

    public init(from decoder: Decoder) throws {
        let c = try decoder.singleValueContainer()
        if let s = try? c.decode(String.self) {
            if s == "Text" { self = .text; return }
            throw aimuxDecodingError(c.codingPath, "unknown response format \(s)")
        }
        let o = try decoder.container(keyedBy: AnyCodingKey.self)
        guard let key = o.allKeys.first?.stringValue else {
            throw aimuxDecodingError(o.codingPath, "missing response format tag")
        }
        guard key == "Json" else {
            throw aimuxDecodingError(o.codingPath, "unknown response format tag \(key)")
        }
        let n = try o.nestedContainer(keyedBy: Field.self, forKey: AnyCodingKey(key))
        self = .json(schema: try n.decodeIfPresent(JSONValue.self, forKey: .schema),
                     name: try n.decodeIfPresent(String.self, forKey: .name),
                     description: try n.decodeIfPresent(String.self, forKey: .description))
    }

    public func encode(to encoder: Encoder) throws {
        switch self {
        case .text:
            var c = encoder.singleValueContainer(); try c.encode("Text")
        case .json(let schema, let name, let description):
            var c = encoder.container(keyedBy: AnyCodingKey.self)
            var n = c.nestedContainer(keyedBy: Field.self, forKey: AnyCodingKey("Json"))
            try n.encodeIfPresent(schema, forKey: .schema)
            try n.encodeIfPresent(name, forKey: .name)
            try n.encodeIfPresent(description, forKey: .description)
        }
    }
}

// MARK: - Messages

/// A part of a multi-part message.
///
/// Wire: internally tagged by `type` (variant names snake_case):
/// `{"type":"text","text":"..."}`, `{"type":"tool_call","tool_call_id":...}`, …
public enum ContentPart: Codable, Equatable {
    case text(text: String, providerOptions: JSONValue?)
    case image(image: [UInt8], mediaType: String, providerOptions: JSONValue?)
    case file(data: [UInt8], mediaType: String, filename: String?, providerOptions: JSONValue?)
    case fileBase64(data: String, mediaType: String, filename: String?, providerOptions: JSONValue?)
    case fileUrl(url: String, mediaType: String, providerOptions: JSONValue?)
    case fileReference(mediaType: String, reference: JSONValue, filename: String?, providerOptions: JSONValue?)
    case reasoning(text: String, signature: String?, providerOptions: JSONValue?)
    case toolCall(toolCallId: String, toolName: String, input: JSONValue,
                  providerExecuted: Bool? = nil, thoughtSignature: String? = nil,
                  providerOptions: JSONValue?)
    case toolResult(toolCallId: String, result: JSONValue, toolName: String?,
                     isError: Bool?, preliminary: Bool?, dynamic: Bool?, providerOptions: JSONValue?)

    private enum Field: String, CodingKey {
        case text, image, data, mediaType = "media_type", filename, url, reference
        case signature, toolCallId = "tool_call_id", toolName = "tool_name", input, result
        case providerExecuted = "provider_executed", thoughtSignature = "thought_signature"
        case isError = "is_error", preliminary, dynamic
        case providerOptions = "provider_options"
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: AnyCodingKey.self)
        let type = try c.decode(String.self, forKey: AnyCodingKey("type"))
        func po() throws -> JSONValue? { try c.decodeIfPresent(JSONValue.self, forKey: AnyCodingKey("provider_options")) }
        switch type {
        case "text":
            self = .text(text: try c.decode(String.self, forKey: AnyCodingKey("text")),
                         providerOptions: try po())
        case "image":
            self = .image(image: try c.decode([UInt8].self, forKey: AnyCodingKey("image")),
                          mediaType: try c.decode(String.self, forKey: AnyCodingKey("media_type")),
                          providerOptions: try po())
        case "file":
            self = .file(data: try c.decode([UInt8].self, forKey: AnyCodingKey("data")),
                         mediaType: try c.decode(String.self, forKey: AnyCodingKey("media_type")),
                         filename: try c.decodeIfPresent(String.self, forKey: AnyCodingKey("filename")),
                         providerOptions: try po())
        case "file_base64":
            self = .fileBase64(data: try c.decode(String.self, forKey: AnyCodingKey("data")),
                               mediaType: try c.decode(String.self, forKey: AnyCodingKey("media_type")),
                               filename: try c.decodeIfPresent(String.self, forKey: AnyCodingKey("filename")),
                               providerOptions: try po())
        case "file_url":
            self = .fileUrl(url: try c.decode(String.self, forKey: AnyCodingKey("url")),
                            mediaType: try c.decode(String.self, forKey: AnyCodingKey("media_type")),
                            providerOptions: try po())
        case "file_reference":
            self = .fileReference(mediaType: try c.decode(String.self, forKey: AnyCodingKey("media_type")),
                                  reference: try c.decode(JSONValue.self, forKey: AnyCodingKey("reference")),
                                  filename: try c.decodeIfPresent(String.self, forKey: AnyCodingKey("filename")),
                                  providerOptions: try po())
        case "reasoning":
            self = .reasoning(text: try c.decode(String.self, forKey: AnyCodingKey("text")),
                             signature: try c.decodeIfPresent(String.self, forKey: AnyCodingKey("signature")),
                             providerOptions: try po())
        case "tool_call":
            self = .toolCall(toolCallId: try c.decode(String.self, forKey: AnyCodingKey("tool_call_id")),
                             toolName: try c.decode(String.self, forKey: AnyCodingKey("tool_name")),
                             input: try c.decode(JSONValue.self, forKey: AnyCodingKey("input")),
                             providerExecuted: try c.decodeIfPresent(Bool.self, forKey: AnyCodingKey("provider_executed")),
                             thoughtSignature: try c.decodeIfPresent(String.self, forKey: AnyCodingKey("thought_signature")),
                             providerOptions: try po())
        case "tool_result":
            self = .toolResult(toolCallId: try c.decode(String.self, forKey: AnyCodingKey("tool_call_id")),
                               result: try c.decode(JSONValue.self, forKey: AnyCodingKey("result")),
                               toolName: try c.decodeIfPresent(String.self, forKey: AnyCodingKey("tool_name")),
                               isError: try c.decodeIfPresent(Bool.self, forKey: AnyCodingKey("is_error")),
                               preliminary: try c.decodeIfPresent(Bool.self, forKey: AnyCodingKey("preliminary")),
                               dynamic: try c.decodeIfPresent(Bool.self, forKey: AnyCodingKey("dynamic")),
                               providerOptions: try po())
        default:
            throw aimuxDecodingError(c.codingPath, "unknown content part type \(type)")
        }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: AnyCodingKey.self)
        switch self {
        case .text(let text, let po):
            try c.encode("text", forKey: AnyCodingKey("type"))
            try c.encode(text, forKey: AnyCodingKey("text"))
            try c.encodeIfPresent(po, forKey: AnyCodingKey("provider_options"))
        case .image(let image, let mediaType, let po):
            try c.encode("image", forKey: AnyCodingKey("type"))
            try c.encode(image, forKey: AnyCodingKey("image"))
            try c.encode(mediaType, forKey: AnyCodingKey("media_type"))
            try c.encodeIfPresent(po, forKey: AnyCodingKey("provider_options"))
        case .file(let data, let mediaType, let filename, let po):
            try c.encode("file", forKey: AnyCodingKey("type"))
            try c.encode(data, forKey: AnyCodingKey("data"))
            try c.encode(mediaType, forKey: AnyCodingKey("media_type"))
            try c.encodeIfPresent(filename, forKey: AnyCodingKey("filename"))
            try c.encodeIfPresent(po, forKey: AnyCodingKey("provider_options"))
        case .fileBase64(let data, let mediaType, let filename, let po):
            try c.encode("file_base64", forKey: AnyCodingKey("type"))
            try c.encode(data, forKey: AnyCodingKey("data"))
            try c.encode(mediaType, forKey: AnyCodingKey("media_type"))
            try c.encodeIfPresent(filename, forKey: AnyCodingKey("filename"))
            try c.encodeIfPresent(po, forKey: AnyCodingKey("provider_options"))
        case .fileUrl(let url, let mediaType, let po):
            try c.encode("file_url", forKey: AnyCodingKey("type"))
            try c.encode(url, forKey: AnyCodingKey("url"))
            try c.encode(mediaType, forKey: AnyCodingKey("media_type"))
            try c.encodeIfPresent(po, forKey: AnyCodingKey("provider_options"))
        case .fileReference(let mediaType, let reference, let filename, let po):
            try c.encode("file_reference", forKey: AnyCodingKey("type"))
            try c.encode(mediaType, forKey: AnyCodingKey("media_type"))
            try c.encode(reference, forKey: AnyCodingKey("reference"))
            try c.encodeIfPresent(filename, forKey: AnyCodingKey("filename"))
            try c.encodeIfPresent(po, forKey: AnyCodingKey("provider_options"))
        case .reasoning(let text, let signature, let po):
            try c.encode("reasoning", forKey: AnyCodingKey("type"))
            try c.encode(text, forKey: AnyCodingKey("text"))
            try c.encodeIfPresent(signature, forKey: AnyCodingKey("signature"))
            try c.encodeIfPresent(po, forKey: AnyCodingKey("provider_options"))
        case .toolCall(let toolCallId, let toolName, let input, let providerExecuted, let thoughtSignature, let po):
            try c.encode("tool_call", forKey: AnyCodingKey("type"))
            try c.encode(toolCallId, forKey: AnyCodingKey("tool_call_id"))
            try c.encode(toolName, forKey: AnyCodingKey("tool_name"))
            try c.encode(input, forKey: AnyCodingKey("input"))
            try c.encodeIfPresent(providerExecuted, forKey: AnyCodingKey("provider_executed"))
            try c.encodeIfPresent(thoughtSignature, forKey: AnyCodingKey("thought_signature"))
            try c.encodeIfPresent(po, forKey: AnyCodingKey("provider_options"))
        case .toolResult(let toolCallId, let result, let toolName, let isError, let preliminary, let dynamic, let po):
            try c.encode("tool_result", forKey: AnyCodingKey("type"))
            try c.encode(toolCallId, forKey: AnyCodingKey("tool_call_id"))
            try c.encode(result, forKey: AnyCodingKey("result"))
            try c.encodeIfPresent(toolName, forKey: AnyCodingKey("tool_name"))
            try c.encodeIfPresent(isError, forKey: AnyCodingKey("is_error"))
            try c.encodeIfPresent(preliminary, forKey: AnyCodingKey("preliminary"))
            try c.encodeIfPresent(dynamic, forKey: AnyCodingKey("dynamic"))
            try c.encodeIfPresent(po, forKey: AnyCodingKey("provider_options"))
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
    /// Arguments as a JSON value (usually an object).
    public var input: JSONValue
    public var providerExecuted: Bool?
    public var dynamic: Bool?
    /// Provider-assigned thought signature (e.g. Google Gemini
    /// `thoughtSignature`); must be echoed back verbatim on follow-up turns.
    public var thoughtSignature: String?
    /// Additional provider-specific metadata associated with this call.
    public var providerMetadata: JSONValue?
    /// Set by Core when the tool call stays invalid after optional repair.
    public var invalid: Bool?
    /// The typed lookup, parse, schema, or repair failure for an invalid call.
    public var error: JSONValue?

    enum CodingKeys: String, CodingKey {
        case toolCallId = "tool_call_id"
        case toolName = "tool_name"
        case input
        case providerExecuted = "provider_executed"
        case dynamic
        case thoughtSignature = "thought_signature"
        case providerMetadata = "provider_metadata"
        case invalid
        case error
    }

    public init(toolCallId: String, toolName: String, input: JSONValue,
                providerExecuted: Bool? = nil, dynamic: Bool? = nil,
                thoughtSignature: String? = nil, providerMetadata: JSONValue? = nil,
                invalid: Bool? = nil,
                error: JSONValue? = nil) {
        self.toolCallId = toolCallId; self.toolName = toolName; self.input = input
        self.providerExecuted = providerExecuted; self.dynamic = dynamic
        self.thoughtSignature = thoughtSignature
        self.providerMetadata = providerMetadata
        self.invalid = invalid; self.error = error
    }
}

// MARK: - File data (payload of `GenerateContent.file` / `StreamPart.file`)

/// Either raw bytes or a base64-encoded string (mirrors `aimux_core::FileBytes`).
///
/// Wire: externally tagged newtype — `{"Binary":[…]}`, `{"Base64":"…"}`.
public enum FileBytes: Codable, Equatable {
    case binary([UInt8])
    case base64(String)

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: AnyCodingKey.self)
        guard let key = c.allKeys.first?.stringValue else {
            throw aimuxDecodingError(c.codingPath, "missing file-bytes tag")
        }
        switch key {
        case "Binary":
            self = .binary(try c.decode([UInt8].self, forKey: AnyCodingKey(key)))
        case "Base64":
            self = .base64(try c.decode(String.self, forKey: AnyCodingKey(key)))
        default:
            throw aimuxDecodingError(c.codingPath, "unknown file-bytes tag \(key)")
        }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: AnyCodingKey.self)
        switch self {
        case .binary(let bytes):
            try c.encode(bytes, forKey: AnyCodingKey("Binary"))
        case .base64(let s):
            try c.encode(s, forKey: AnyCodingKey("Base64"))
        }
    }
}

/// File data as a tagged discriminated union (mirrors `aimux_core::FileData`).
///
/// Wire: externally tagged — `{"Data":{"data":<FileBytes>}}`,
/// `{"Url":{"url":"…"}}`, `{"Reference":{"reference":{…}}}`,
/// `{"Text":{"text":"…"}}`.
public enum FileData: Codable, Equatable {
    case data(data: FileBytes)
    case url(url: String)
    case reference(reference: [String: String])
    case text(text: String)

    private enum Field: String, CodingKey {
        case data, url, reference, text
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: AnyCodingKey.self)
        guard let key = c.allKeys.first?.stringValue else {
            throw aimuxDecodingError(c.codingPath, "missing file-data tag")
        }
        let n = try c.nestedContainer(keyedBy: Field.self, forKey: AnyCodingKey(key))
        switch key {
        case "Data":
            self = .data(data: try n.decode(FileBytes.self, forKey: .data))
        case "Url":
            self = .url(url: try n.decode(String.self, forKey: .url))
        case "Reference":
            self = .reference(reference: try n.decode([String: String].self, forKey: .reference))
        case "Text":
            self = .text(text: try n.decode(String.self, forKey: .text))
        default:
            throw aimuxDecodingError(c.codingPath, "unknown file-data tag \(key)")
        }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: AnyCodingKey.self)
        switch self {
        case .data(let data):
            var n = c.nestedContainer(keyedBy: Field.self, forKey: AnyCodingKey("Data"))
            try n.encode(data, forKey: .data)
        case .url(let url):
            var n = c.nestedContainer(keyedBy: Field.self, forKey: AnyCodingKey("Url"))
            try n.encode(url, forKey: .url)
        case .reference(let reference):
            var n = c.nestedContainer(keyedBy: Field.self, forKey: AnyCodingKey("Reference"))
            try n.encode(reference, forKey: .reference)
        case .text(let text):
            var n = c.nestedContainer(keyedBy: Field.self, forKey: AnyCodingKey("Text"))
            try n.encode(text, forKey: .text)
        }
    }
}

/// A content item in the generation result.
///
/// Wire: externally tagged — `{"Text":{...}}`, `{"ToolCall":{...}}`, …
public enum GenerateContent: Codable, Equatable {
    case text(text: String, providerMetadata: JSONValue?)
    case toolCall(toolCallId: String, toolName: String, input: JSONValue,
                  providerExecuted: Bool?, dynamic: Bool?, thoughtSignature: String? = nil,
                  providerMetadata: JSONValue?)
    case source(id: String, sourceType: String, url: String?, title: String?,
                providerMetadata: JSONValue?)
    case reasoning(text: String, providerMetadata: JSONValue?)
    case file(data: FileData, mediaType: String, providerMetadata: JSONValue?)
    case toolResult(toolCallId: String, toolName: String, result: JSONValue,
                    isError: Bool?, preliminary: Bool?, dynamic: Bool?, providerMetadata: JSONValue?)

    private enum Field: String, CodingKey {
        case text
        case toolCallId = "tool_call_id", toolName = "tool_name", input, result
        case providerExecuted = "provider_executed", dynamic
        case thoughtSignature = "thought_signature", providerMetadata = "provider_metadata"
        case id, sourceType = "source_type", url, title
        case isError = "is_error", preliminary
        case data, mediaType = "media_type"
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: AnyCodingKey.self)
        guard let key = c.allKeys.first?.stringValue else {
            throw aimuxDecodingError(c.codingPath, "missing generate-content tag")
        }
        let n = try c.nestedContainer(keyedBy: Field.self, forKey: AnyCodingKey(key))
        switch key {
        case "Text":
            self = .text(text: try n.decode(String.self, forKey: .text),
                         providerMetadata: try n.decodeIfPresent(JSONValue.self, forKey: .providerMetadata))
        case "ToolCall":
            self = .toolCall(toolCallId: try n.decode(String.self, forKey: .toolCallId),
                             toolName: try n.decode(String.self, forKey: .toolName),
                             input: try n.decode(JSONValue.self, forKey: .input),
                             providerExecuted: try n.decodeIfPresent(Bool.self, forKey: .providerExecuted),
                             dynamic: try n.decodeIfPresent(Bool.self, forKey: .dynamic),
                             thoughtSignature: try n.decodeIfPresent(String.self, forKey: .thoughtSignature),
                             providerMetadata: try n.decodeIfPresent(JSONValue.self, forKey: .providerMetadata))
        case "Source":
            self = .source(id: try n.decode(String.self, forKey: .id),
                           sourceType: try n.decode(String.self, forKey: .sourceType),
                           url: try n.decodeIfPresent(String.self, forKey: .url),
                           title: try n.decodeIfPresent(String.self, forKey: .title),
                           providerMetadata: try n.decodeIfPresent(JSONValue.self, forKey: .providerMetadata))
        case "Reasoning":
            self = .reasoning(text: try n.decode(String.self, forKey: .text),
                             providerMetadata: try n.decodeIfPresent(JSONValue.self, forKey: .providerMetadata))
        case "File":
            self = .file(data: try n.decode(FileData.self, forKey: .data),
                         mediaType: try n.decode(String.self, forKey: .mediaType),
                         providerMetadata: try n.decodeIfPresent(JSONValue.self, forKey: .providerMetadata))
        case "ToolResult":
            self = .toolResult(toolCallId: try n.decode(String.self, forKey: .toolCallId),
                               toolName: try n.decode(String.self, forKey: .toolName),
                               result: try n.decode(JSONValue.self, forKey: .result),
                               isError: try n.decodeIfPresent(Bool.self, forKey: .isError),
                               preliminary: try n.decodeIfPresent(Bool.self, forKey: .preliminary),
                               dynamic: try n.decodeIfPresent(Bool.self, forKey: .dynamic),
                               providerMetadata: try n.decodeIfPresent(JSONValue.self, forKey: .providerMetadata))
        default:
            throw aimuxDecodingError(c.codingPath, "unknown generate-content tag \(key)")
        }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: AnyCodingKey.self)
        switch self {
        case .text(let text, let pm):
            var n = c.nestedContainer(keyedBy: Field.self, forKey: AnyCodingKey("Text"))
            try n.encode(text, forKey: .text)
            try n.encodeIfPresent(pm, forKey: .providerMetadata)
        case .toolCall(let toolCallId, let toolName, let input, let pe, let dyn, let signature, let pm):
            var n = c.nestedContainer(keyedBy: Field.self, forKey: AnyCodingKey("ToolCall"))
            try n.encode(toolCallId, forKey: .toolCallId)
            try n.encode(toolName, forKey: .toolName)
            try n.encode(input, forKey: .input)
            try n.encodeIfPresent(pe, forKey: .providerExecuted)
            try n.encodeIfPresent(dyn, forKey: .dynamic)
            try n.encodeIfPresent(signature, forKey: .thoughtSignature)
            try n.encodeIfPresent(pm, forKey: .providerMetadata)
        case .source(let id, let sourceType, let url, let title, let pm):
            var n = c.nestedContainer(keyedBy: Field.self, forKey: AnyCodingKey("Source"))
            try n.encode(id, forKey: .id)
            try n.encode(sourceType, forKey: .sourceType)
            try n.encodeIfPresent(url, forKey: .url)
            try n.encodeIfPresent(title, forKey: .title)
            try n.encodeIfPresent(pm, forKey: .providerMetadata)
        case .reasoning(let text, let pm):
            var n = c.nestedContainer(keyedBy: Field.self, forKey: AnyCodingKey("Reasoning"))
            try n.encode(text, forKey: .text)
            try n.encodeIfPresent(pm, forKey: .providerMetadata)
        case .file(let data, let mediaType, let pm):
            var n = c.nestedContainer(keyedBy: Field.self, forKey: AnyCodingKey("File"))
            try n.encode(data, forKey: .data)
            try n.encode(mediaType, forKey: .mediaType)
            try n.encodeIfPresent(pm, forKey: .providerMetadata)
        case .toolResult(let toolCallId, let toolName, let result, let ie, let prel, let dyn, let pm):
            var n = c.nestedContainer(keyedBy: Field.self, forKey: AnyCodingKey("ToolResult"))
            try n.encode(toolCallId, forKey: .toolCallId)
            try n.encode(toolName, forKey: .toolName)
            try n.encode(result, forKey: .result)
            try n.encodeIfPresent(ie, forKey: .isError)
            try n.encodeIfPresent(prel, forKey: .preliminary)
            try n.encodeIfPresent(dyn, forKey: .dynamic)
            try n.encodeIfPresent(pm, forKey: .providerMetadata)
        }
    }
}

/// Raw provider result (the `raw` field of `GenerateTextResult`).
public struct GenerateResult: Codable, Equatable {
    public var content: [GenerateContent]
    public var finishReason: FinishReason?
    public var usage: Usage?
    public var warnings: [Warning]?
    public var providerMetadata: JSONValue?
    public var response: ResponseMetadata?
    public var requestBody: JSONValue?
    public var responseHeaders: [String: String]?

    enum CodingKeys: String, CodingKey {
        case content
        case finishReason = "finish_reason"
        case usage, warnings
        case providerMetadata = "provider_metadata"
        case response
        case requestBody = "request_body"
        case responseHeaders = "response_headers"
    }

    public init(content: [GenerateContent], finishReason: FinishReason? = nil, usage: Usage? = nil,
                warnings: [Warning]? = nil, providerMetadata: JSONValue? = nil,
                response: ResponseMetadata? = nil, requestBody: JSONValue? = nil,
                responseHeaders: [String: String]? = nil) {
        self.content = content; self.finishReason = finishReason; self.usage = usage
        self.warnings = warnings; self.providerMetadata = providerMetadata
        self.response = response; self.requestBody = requestBody; self.responseHeaders = responseHeaders
    }
}

/// Result of `generate_text` (user-facing).
public struct GenerateTextResult: Codable, Equatable {
    /// The generated text (concatenated from all text content parts).
    public var text: String
    /// Tool calls requested by the model.
    public var toolCalls: [ToolCall]
    /// Why generation stopped.
    public var finishReason: FinishReason
    /// Token usage.
    public var usage: Usage
    /// Warnings produced while generating the response.
    public var warnings: [Warning]
    /// Raw provider result (for advanced use).
    public var raw: GenerateResult
    /// Reasoning / thinking segments (M7). Weak type — use `raw.content` for full typing.
    public var reasoning: [JSONValue]
    /// Concatenated reasoning text (M7).
    public var reasoningText: String
    /// Sources / citations (M7). Weak type.
    public var sources: [JSONValue]
    /// Files generated by the model (M7). Weak type.
    public var files: [JSONValue]
    /// Assistant messages ready to append for the next turn (M7).
    public var responseMessages: [ModelMessage]
    /// Raw provider-specific finish reason string (M12, e.g. "stop", "end_turn").
    public var rawFinishReason: String?
    /// Provider-specific metadata (e.g. Anthropic cache info). Mirrored from
    /// `raw.provider_metadata` for top-level convenience. Weak type.
    public var providerMetadata: JSONValue?
    /// Response metadata (id, timestamp, model_id). Mirrored from `raw.response`
    /// for top-level convenience.
    public var response: ResponseMetadata
    /// Total token usage across all steps. In single-step mode (aimux's
    /// default), equals `usage`. Provided for AI SDK parity.
    public var totalUsage: Usage

    enum CodingKeys: String, CodingKey {
        case text
        case toolCalls = "tool_calls"
        case finishReason = "finish_reason"
        case usage, warnings, raw
        case reasoning
        case reasoningText = "reasoning_text"
        case sources, files
        case responseMessages = "response_messages"
        case rawFinishReason = "raw_finish_reason"
        case providerMetadata = "provider_metadata"
        case response
        case totalUsage = "total_usage"
    }

    public init(text: String, toolCalls: [ToolCall], finishReason: FinishReason,
                usage: Usage, warnings: [Warning] = [], raw: GenerateResult,
                reasoning: [JSONValue] = [], reasoningText: String = "",
                sources: [JSONValue] = [], files: [JSONValue] = [],
                responseMessages: [ModelMessage] = [],
                rawFinishReason: String? = nil,
                providerMetadata: JSONValue? = nil,
                response: ResponseMetadata = ResponseMetadata(),
                totalUsage: Usage = Usage(inputTokens: TokenUsage(), outputTokens: TokenUsage())) {
        self.text = text; self.toolCalls = toolCalls; self.finishReason = finishReason
        self.usage = usage; self.warnings = warnings; self.raw = raw
        self.reasoning = reasoning; self.reasoningText = reasoningText
        self.sources = sources; self.files = files
        self.responseMessages = responseMessages
        self.rawFinishReason = rawFinishReason
        self.providerMetadata = providerMetadata
        self.response = response
        self.totalUsage = totalUsage
    }
}

/// Result of `generate_object` (user-facing, M12). The parsed JSON object plus
/// convenience fields from the underlying `generate_text` call.
public struct GenerateObjectResult: Codable, Equatable {
    /// The parsed JSON object returned by the model (arbitrary JSON, weak type).
    public var object: JSONValue
    /// Why generation stopped.
    public var finishReason: FinishReason
    /// Raw provider-specific finish reason string.
    public var rawFinishReason: String?
    /// Token usage.
    public var usage: Usage
    /// Warnings from the provider.
    public var warnings: [Warning]
    /// Concatenated reasoning text (if the model produced reasoning/thinking).
    public var reasoning: String?
    /// Provider-specific metadata (e.g. Anthropic cache info). Weak type.
    public var providerMetadata: JSONValue?
    /// Response metadata (id, timestamp, model_id).
    public var response: ResponseMetadata
    /// The full `generate_text` result (for advanced use).
    public var raw: GenerateTextResult

    enum CodingKeys: String, CodingKey {
        case object
        case finishReason = "finish_reason"
        case rawFinishReason = "raw_finish_reason"
        case usage, warnings
        case reasoning
        case providerMetadata = "provider_metadata"
        case response, raw
    }

    public init(object: JSONValue, finishReason: FinishReason,
                rawFinishReason: String? = nil,
                usage: Usage = Usage(inputTokens: TokenUsage(), outputTokens: TokenUsage()),
                warnings: [Warning] = [],
                reasoning: String? = nil,
                providerMetadata: JSONValue? = nil,
                response: ResponseMetadata = ResponseMetadata(),
                raw: GenerateTextResult) {
        self.object = object; self.finishReason = finishReason
        self.rawFinishReason = rawFinishReason; self.usage = usage
        self.warnings = warnings; self.reasoning = reasoning
        self.providerMetadata = providerMetadata; self.response = response
        self.raw = raw
    }
}

/// Aggregated result of `stream_text().consume()` (M11). Mirrors
/// `GenerateTextResult`'s user-facing fields (without `raw`, since streaming
/// has no `GenerateResult` equivalent).
public struct StreamTextResultAggregated: Codable, Equatable {
    /// The generated text (concatenated `TextDelta`).
    public var text: String
    /// Reasoning / thinking segments. Weak type.
    public var reasoning: [JSONValue]
    /// Concatenated reasoning text.
    public var reasoningText: String
    /// Tool calls requested by the model.
    public var toolCalls: [ToolCall]
    /// Sources / citations. Weak type.
    public var sources: [JSONValue]
    /// Files generated by the model. Weak type.
    public var files: [JSONValue]
    /// Why generation stopped.
    public var finishReason: FinishReason
    /// Raw provider-specific finish reason string.
    public var rawFinishReason: String?
    /// Token usage.
    public var usage: Usage
    /// Total token usage across all steps. In single-step mode (aimux's
    /// default), equals `usage`. Provided for AI SDK parity.
    public var totalUsage: Usage
    /// Warnings from the provider.
    public var warnings: [Warning]
    /// Provider-specific metadata from the Finish chunk. Weak type.
    public var providerMetadata: JSONValue?
    /// Response metadata (id, timestamp, model_id) if emitted by the stream.
    public var response: ResponseMetadata?
    /// Assistant messages ready to append for the next turn.
    public var responseMessages: [ModelMessage]

    enum CodingKeys: String, CodingKey {
        case text
        case reasoning
        case reasoningText = "reasoning_text"
        case toolCalls = "tool_calls"
        case sources, files
        case finishReason = "finish_reason"
        case rawFinishReason = "raw_finish_reason"
        case usage
        case totalUsage = "total_usage"
        case warnings
        case providerMetadata = "provider_metadata"
        case response
        case responseMessages = "response_messages"
    }

    public init(text: String = "", reasoning: [JSONValue] = [],
                reasoningText: String = "", toolCalls: [ToolCall] = [],
                sources: [JSONValue] = [], files: [JSONValue] = [],
                finishReason: FinishReason, rawFinishReason: String? = nil,
                usage: Usage = Usage(inputTokens: TokenUsage(), outputTokens: TokenUsage()),
                totalUsage: Usage = Usage(inputTokens: TokenUsage(), outputTokens: TokenUsage()),
                warnings: [Warning] = [],
                providerMetadata: JSONValue? = nil,
                response: ResponseMetadata? = nil,
                responseMessages: [ModelMessage] = []) {
        self.text = text; self.reasoning = reasoning; self.reasoningText = reasoningText
        self.toolCalls = toolCalls; self.sources = sources; self.files = files
        self.finishReason = finishReason; self.rawFinishReason = rawFinishReason
        self.usage = usage; self.totalUsage = totalUsage; self.warnings = warnings
        self.providerMetadata = providerMetadata; self.response = response
        self.responseMessages = responseMessages
    }
}

// MARK: - TimeoutConfiguration

/// Per-call timeout configuration.
///
/// Mirrors `TimeoutConfiguration.ts`. All values are milliseconds; `nil`
/// disables the corresponding limit. A `total` timeout also covers retry
/// backoff and the whole streamed response.
public struct TimeoutConfiguration: Codable, Equatable {
    public var totalMs: UInt64?
    public var firstChunkMs: UInt64?
    public var chunkMs: UInt64?

    enum CodingKeys: String, CodingKey {
        case totalMs = "total_ms"
        case firstChunkMs = "first_chunk_ms"
        case chunkMs = "chunk_ms"
    }

    public init(totalMs: UInt64? = nil, firstChunkMs: UInt64? = nil, chunkMs: UInt64? = nil) {
        self.totalMs = totalMs; self.firstChunkMs = firstChunkMs; self.chunkMs = chunkMs
    }
}

// MARK: - GenerateTextOptions

/// User-facing options for `generate_text` / `stream_text`.
///
/// All fields are optional. Encoding omits `nil` fields (the Rust side decodes
/// missing `Option` fields as `None`), matching the partial-options usage of
/// the raw API.
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
    public var providerOptions: JSONValue?
    public var reasoning: ReasoningEffort?
    public var instructions: String?
    public var bodyOverrides: JSONValue?
    public var maxRetries: UInt32?
    public var timeout: TimeoutConfiguration?
    public var includeRawChunks: Bool?
    public var sessionId: String?

    enum CodingKeys: String, CodingKey {
        case maxOutputTokens = "max_output_tokens"
        case temperature
        case stopSequences = "stop_sequences"
        case topP = "top_p"
        case topK = "top_k"
        case presencePenalty = "presence_penalty"
        case frequencyPenalty = "frequency_penalty"
        case responseFormat = "response_format"
        case seed, tools
        case toolChoice = "tool_choice"
        case headers
        case providerOptions = "provider_options"
        case reasoning, instructions
        case bodyOverrides = "body_overrides"
        case maxRetries = "max_retries"
        case timeout
        case includeRawChunks = "include_raw_chunks"
        case sessionId = "session_id"
    }

    public init(maxOutputTokens: UInt32? = nil, temperature: Double? = nil,
                stopSequences: [String]? = nil, topP: Double? = nil, topK: Double? = nil,
                presencePenalty: Double? = nil, frequencyPenalty: Double? = nil,
                responseFormat: ResponseFormat? = nil, seed: UInt64? = nil,
                tools: [Tool]? = nil, toolChoice: ToolChoice? = nil,
                headers: [String: String]? = nil, providerOptions: JSONValue? = nil,
                reasoning: ReasoningEffort? = nil, instructions: String? = nil,
                bodyOverrides: JSONValue? = nil, maxRetries: UInt32? = nil,
                timeout: TimeoutConfiguration? = nil,
                includeRawChunks: Bool? = nil,
                sessionId: String? = nil) {
        self.maxOutputTokens = maxOutputTokens; self.temperature = temperature
        self.stopSequences = stopSequences; self.topP = topP; self.topK = topK
        self.presencePenalty = presencePenalty; self.frequencyPenalty = frequencyPenalty
        self.responseFormat = responseFormat; self.seed = seed; self.tools = tools
        self.toolChoice = toolChoice; self.headers = headers; self.providerOptions = providerOptions
        self.reasoning = reasoning; self.instructions = instructions
        self.bodyOverrides = bodyOverrides; self.maxRetries = maxRetries; self.timeout = timeout
        self.includeRawChunks = includeRawChunks
        self.sessionId = sessionId
    }
}

// MARK: - StreamPart

/// A single chunk in the stream returned by `stream_text`.
///
/// Wire: externally tagged — `{"TextDelta":{"id":"…","delta":"…"}}`, …
public enum StreamPart: Codable, Equatable {
    // P0: text
    case textStart(id: String, providerMetadata: JSONValue?)
    case textDelta(id: String, delta: String, providerMetadata: JSONValue?)
    case textEnd(id: String, providerMetadata: JSONValue?)
    // P0: stream lifecycle
    case streamStart(warnings: [Warning])
    case finish(finishReason: FinishReason, usage: Usage, providerMetadata: JSONValue?)
    case error(error: JSONValue)
    // P1: tool calls
    case toolInputStart(id: String, toolName: String, providerExecuted: Bool?, dynamic: Bool?,
                        title: String?, providerMetadata: JSONValue?)
    case toolInputDelta(id: String, delta: String, providerMetadata: JSONValue?)
    case toolInputEnd(id: String, providerMetadata: JSONValue?)
    case toolCall(toolCallId: String, toolName: String, input: JSONValue,
                  providerExecuted: Bool?, dynamic: Bool?, thoughtSignature: String? = nil,
                  providerMetadata: JSONValue?,
                  invalid: Bool?, error: JSONValue?)
    case toolResult(toolCallId: String, toolName: String, result: JSONValue,
                    isError: Bool?, preliminary: Bool?, dynamic: Bool?, providerMetadata: JSONValue?)
    // P2: file
    case file(data: FileData, mediaType: String, providerMetadata: JSONValue?)
    // P2: reasoning
    case reasoningStart(id: String, providerMetadata: JSONValue?)
    case reasoningDelta(id: String, delta: String, providerMetadata: JSONValue?)
    case reasoningEnd(id: String, providerMetadata: JSONValue?)
    // P2: metadata
    case responseMetadata(id: String?, timestamp: String?, modelId: String?)
    case source(id: String, sourceType: String, url: String?, title: String?,
                providerMetadata: JSONValue?)
    case raw(rawValue: JSONValue)

    private enum Field: String, CodingKey {
        case id, delta, warnings, usage, text
        case finishReason = "finish_reason", providerMetadata = "provider_metadata"
        case error
        case toolName = "tool_name", toolCallId = "tool_call_id", input, result
        case providerExecuted = "provider_executed", dynamic, thoughtSignature = "thought_signature", invalid
        case isError = "is_error", preliminary
        case timestamp, modelId = "model_id"
        case sourceType = "source_type", url, title
        case data, mediaType = "media_type"
        case rawValue = "raw_value"
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: AnyCodingKey.self)
        guard let key = c.allKeys.first?.stringValue else {
            throw aimuxDecodingError(c.codingPath, "missing stream-part tag")
        }
        // Unit-ish variants with no nested payload (none in StreamPart) would be
        // bare strings; every variant here carries a payload object.
        let n = try c.nestedContainer(keyedBy: Field.self, forKey: AnyCodingKey(key))
        switch key {
        case "TextStart":
            self = .textStart(id: try n.decode(String.self, forKey: .id),
                              providerMetadata: try n.decodeIfPresent(JSONValue.self, forKey: .providerMetadata))
        case "TextDelta":
            self = .textDelta(id: try n.decode(String.self, forKey: .id),
                              delta: try n.decode(String.self, forKey: .delta),
                              providerMetadata: try n.decodeIfPresent(JSONValue.self, forKey: .providerMetadata))
        case "TextEnd":
            self = .textEnd(id: try n.decode(String.self, forKey: .id),
                            providerMetadata: try n.decodeIfPresent(JSONValue.self, forKey: .providerMetadata))
        case "StreamStart":
            self = .streamStart(warnings: try n.decode([Warning].self, forKey: .warnings))
        case "Finish":
            self = .finish(finishReason: try n.decode(FinishReason.self, forKey: .finishReason),
                           usage: try n.decode(Usage.self, forKey: .usage),
                           providerMetadata: try n.decodeIfPresent(JSONValue.self, forKey: .providerMetadata))
        case "Error":
            self = .error(error: try n.decode(JSONValue.self, forKey: .error))
        case "ToolInputStart":
            self = .toolInputStart(id: try n.decode(String.self, forKey: .id),
                                   toolName: try n.decode(String.self, forKey: .toolName),
                                   providerExecuted: try n.decodeIfPresent(Bool.self, forKey: .providerExecuted),
                                   dynamic: try n.decodeIfPresent(Bool.self, forKey: .dynamic),
                                   title: try n.decodeIfPresent(String.self, forKey: .title),
                                   providerMetadata: try n.decodeIfPresent(JSONValue.self, forKey: .providerMetadata))
        case "ToolInputDelta":
            self = .toolInputDelta(id: try n.decode(String.self, forKey: .id),
                                   delta: try n.decode(String.self, forKey: .delta),
                                   providerMetadata: try n.decodeIfPresent(JSONValue.self, forKey: .providerMetadata))
        case "ToolInputEnd":
            self = .toolInputEnd(id: try n.decode(String.self, forKey: .id),
                                 providerMetadata: try n.decodeIfPresent(JSONValue.self, forKey: .providerMetadata))
        case "ToolCall":
            self = .toolCall(toolCallId: try n.decode(String.self, forKey: .toolCallId),
                             toolName: try n.decode(String.self, forKey: .toolName),
                             input: try n.decode(JSONValue.self, forKey: .input),
                             providerExecuted: try n.decodeIfPresent(Bool.self, forKey: .providerExecuted),
                             dynamic: try n.decodeIfPresent(Bool.self, forKey: .dynamic),
                             thoughtSignature: try n.decodeIfPresent(String.self, forKey: .thoughtSignature),
                             providerMetadata: try n.decodeIfPresent(JSONValue.self, forKey: .providerMetadata),
                             invalid: try n.decodeIfPresent(Bool.self, forKey: .invalid),
                             error: try n.decodeIfPresent(JSONValue.self, forKey: .error))
        case "ToolResult":
            self = .toolResult(toolCallId: try n.decode(String.self, forKey: .toolCallId),
                               toolName: try n.decode(String.self, forKey: .toolName),
                               result: try n.decode(JSONValue.self, forKey: .result),
                               isError: try n.decodeIfPresent(Bool.self, forKey: .isError),
                               preliminary: try n.decodeIfPresent(Bool.self, forKey: .preliminary),
                               dynamic: try n.decodeIfPresent(Bool.self, forKey: .dynamic),
                               providerMetadata: try n.decodeIfPresent(JSONValue.self, forKey: .providerMetadata))
        case "File":
            self = .file(data: try n.decode(FileData.self, forKey: .data),
                         mediaType: try n.decode(String.self, forKey: .mediaType),
                         providerMetadata: try n.decodeIfPresent(JSONValue.self, forKey: .providerMetadata))
        case "ReasoningStart":
            self = .reasoningStart(id: try n.decode(String.self, forKey: .id),
                                   providerMetadata: try n.decodeIfPresent(JSONValue.self, forKey: .providerMetadata))
        case "ReasoningDelta":
            self = .reasoningDelta(id: try n.decode(String.self, forKey: .id),
                                   delta: try n.decode(String.self, forKey: .delta),
                                   providerMetadata: try n.decodeIfPresent(JSONValue.self, forKey: .providerMetadata))
        case "ReasoningEnd":
            self = .reasoningEnd(id: try n.decode(String.self, forKey: .id),
                                 providerMetadata: try n.decodeIfPresent(JSONValue.self, forKey: .providerMetadata))
        case "ResponseMetadata":
            self = .responseMetadata(id: try n.decodeIfPresent(String.self, forKey: .id),
                                     timestamp: try n.decodeIfPresent(String.self, forKey: .timestamp),
                                     modelId: try n.decodeIfPresent(String.self, forKey: .modelId))
        case "Source":
            self = .source(id: try n.decode(String.self, forKey: .id),
                           sourceType: try n.decode(String.self, forKey: .sourceType),
                           url: try n.decodeIfPresent(String.self, forKey: .url),
                           title: try n.decodeIfPresent(String.self, forKey: .title),
                           providerMetadata: try n.decodeIfPresent(JSONValue.self, forKey: .providerMetadata))
        case "Raw":
            self = .raw(rawValue: try n.decode(JSONValue.self, forKey: .rawValue))
        default:
            throw aimuxDecodingError(c.codingPath, "unknown stream-part tag \(key)")
        }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: AnyCodingKey.self)
        switch self {
        case .textStart(let id, let pm):
            var n = c.nestedContainer(keyedBy: Field.self, forKey: AnyCodingKey("TextStart"))
            try n.encode(id, forKey: .id); try n.encodeIfPresent(pm, forKey: .providerMetadata)
        case .textDelta(let id, let delta, let pm):
            var n = c.nestedContainer(keyedBy: Field.self, forKey: AnyCodingKey("TextDelta"))
            try n.encode(id, forKey: .id); try n.encode(delta, forKey: .delta)
            try n.encodeIfPresent(pm, forKey: .providerMetadata)
        case .textEnd(let id, let pm):
            var n = c.nestedContainer(keyedBy: Field.self, forKey: AnyCodingKey("TextEnd"))
            try n.encode(id, forKey: .id); try n.encodeIfPresent(pm, forKey: .providerMetadata)
        case .streamStart(let warnings):
            var n = c.nestedContainer(keyedBy: Field.self, forKey: AnyCodingKey("StreamStart"))
            try n.encode(warnings, forKey: .warnings)
        case .finish(let fr, let usage, let pm):
            var n = c.nestedContainer(keyedBy: Field.self, forKey: AnyCodingKey("Finish"))
            try n.encode(fr, forKey: .finishReason); try n.encode(usage, forKey: .usage)
            try n.encodeIfPresent(pm, forKey: .providerMetadata)
        case .error(let error):
            var n = c.nestedContainer(keyedBy: Field.self, forKey: AnyCodingKey("Error"))
            try n.encode(error, forKey: .error)
        case .toolInputStart(let id, let toolName, let pe, let dyn, let title, let pm):
            var n = c.nestedContainer(keyedBy: Field.self, forKey: AnyCodingKey("ToolInputStart"))
            try n.encode(id, forKey: .id); try n.encode(toolName, forKey: .toolName)
            try n.encodeIfPresent(pe, forKey: .providerExecuted); try n.encodeIfPresent(dyn, forKey: .dynamic)
            try n.encodeIfPresent(title, forKey: .title); try n.encodeIfPresent(pm, forKey: .providerMetadata)
        case .toolInputDelta(let id, let delta, let pm):
            var n = c.nestedContainer(keyedBy: Field.self, forKey: AnyCodingKey("ToolInputDelta"))
            try n.encode(id, forKey: .id); try n.encode(delta, forKey: .delta)
            try n.encodeIfPresent(pm, forKey: .providerMetadata)
        case .toolInputEnd(let id, let pm):
            var n = c.nestedContainer(keyedBy: Field.self, forKey: AnyCodingKey("ToolInputEnd"))
            try n.encode(id, forKey: .id); try n.encodeIfPresent(pm, forKey: .providerMetadata)
        case .toolCall(let toolCallId, let toolName, let input, let pe, let dyn, let signature, let pm, let inv, let err):
            var n = c.nestedContainer(keyedBy: Field.self, forKey: AnyCodingKey("ToolCall"))
            try n.encode(toolCallId, forKey: .toolCallId); try n.encode(toolName, forKey: .toolName)
            try n.encode(input, forKey: .input)
            try n.encodeIfPresent(pe, forKey: .providerExecuted); try n.encodeIfPresent(dyn, forKey: .dynamic)
            try n.encodeIfPresent(signature, forKey: .thoughtSignature)
            try n.encodeIfPresent(pm, forKey: .providerMetadata)
            try n.encodeIfPresent(inv, forKey: .invalid); try n.encodeIfPresent(err, forKey: .error)
        case .toolResult(let toolCallId, let toolName, let result, let ie, let prel, let dyn, let pm):
            var n = c.nestedContainer(keyedBy: Field.self, forKey: AnyCodingKey("ToolResult"))
            try n.encode(toolCallId, forKey: .toolCallId); try n.encode(toolName, forKey: .toolName)
            try n.encode(result, forKey: .result)
            try n.encodeIfPresent(ie, forKey: .isError); try n.encodeIfPresent(prel, forKey: .preliminary)
            try n.encodeIfPresent(dyn, forKey: .dynamic)
            try n.encodeIfPresent(pm, forKey: .providerMetadata)
        case .file(let data, let mediaType, let pm):
            var n = c.nestedContainer(keyedBy: Field.self, forKey: AnyCodingKey("File"))
            try n.encode(data, forKey: .data); try n.encode(mediaType, forKey: .mediaType)
            try n.encodeIfPresent(pm, forKey: .providerMetadata)
        case .reasoningStart(let id, let pm):
            var n = c.nestedContainer(keyedBy: Field.self, forKey: AnyCodingKey("ReasoningStart"))
            try n.encode(id, forKey: .id); try n.encodeIfPresent(pm, forKey: .providerMetadata)
        case .reasoningDelta(let id, let delta, let pm):
            var n = c.nestedContainer(keyedBy: Field.self, forKey: AnyCodingKey("ReasoningDelta"))
            try n.encode(id, forKey: .id); try n.encode(delta, forKey: .delta)
            try n.encodeIfPresent(pm, forKey: .providerMetadata)
        case .reasoningEnd(let id, let pm):
            var n = c.nestedContainer(keyedBy: Field.self, forKey: AnyCodingKey("ReasoningEnd"))
            try n.encode(id, forKey: .id); try n.encodeIfPresent(pm, forKey: .providerMetadata)
        case .responseMetadata(let id, let timestamp, let modelId):
            var n = c.nestedContainer(keyedBy: Field.self, forKey: AnyCodingKey("ResponseMetadata"))
            try n.encodeIfPresent(id, forKey: .id)
            try n.encodeIfPresent(timestamp, forKey: .timestamp)
            try n.encodeIfPresent(modelId, forKey: .modelId)
        case .source(let id, let sourceType, let url, let title, let pm):
            var n = c.nestedContainer(keyedBy: Field.self, forKey: AnyCodingKey("Source"))
            try n.encode(id, forKey: .id); try n.encode(sourceType, forKey: .sourceType)
            try n.encodeIfPresent(url, forKey: .url); try n.encodeIfPresent(title, forKey: .title)
            try n.encodeIfPresent(pm, forKey: .providerMetadata)
        case .raw(let rawValue):
            var n = c.nestedContainer(keyedBy: Field.self, forKey: AnyCodingKey("Raw"))
            try n.encode(rawValue, forKey: .rawValue)
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
        let resultJson = try generateText(prompt: promptJson, options: optsJson)
        return try JSONDecoder().decode(GenerateTextResult.self, from: Data(resultJson.utf8))
    }

    /// Generate a structured JSON object with typed inputs/outputs (M12, RFC-0016).
    ///
    /// Same signature as the typed `generateText`; returns a decoded
    /// `GenerateObjectResult`. Pass `response_format: { "Json": { ... } }`
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
        let resultJson = try generateObject(prompt: promptJson, options: optsJson)
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
        let resultJson = try consumeStreamText(prompt: promptJson, options: optsJson)
        return try JSONDecoder().decode(StreamTextResultAggregated.self, from: Data(resultJson.utf8))
    }

    /// Stream text with typed `StreamPart`s.
    ///
    /// Each raw JSON-string part is decoded into a `StreamPart` before being
    /// passed to `onPart`. A part that fails to decode is reported via
    /// `onError` (the stream otherwise continues until done/error).
    func streamText(
        prompt: ModelPrompt,
        options: GenerateTextOptions? = nil,
        onPart: @escaping (StreamPart) -> Void,
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
                           try onPart(JSONDecoder().decode(StreamPart.self, from: Data(json.utf8)))
                       } catch {
                           onError(error) // DecodingError from JSONDecoder
                       }
                   },
                   onDone: onDone,
                   onError: onError)
    }

    /// Stream text as an `AsyncSequence` of typed `StreamPart`s.
    ///
    /// The stream finishes on normal completion and throws `AimuxError`
    /// (AiMuxError failure, C codes preserved via `fromC`) or the native
    /// `EncodingError` / `DecodingError` when typed (de)serialization fails.
    func streamTextAsync(
        prompt: ModelPrompt,
        options: GenerateTextOptions? = nil
    ) -> AsyncThrowingStream<StreamPart, Error> {
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
        let resultJson = try generateTextAsOpenAI(prompt: promptJson, options: optsJson)
        return try JSONDecoder().decode(ChatCompletion.self, from: Data(resultJson.utf8))
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
    /// Encode an `Encodable` value to a JSON string using the default key
    /// strategy (snake_case mapping is handled per-type via `CodingKeys`).
    static func jsonString<T: Encodable>(for value: T) throws -> String {
        let data = try JSONEncoder().encode(value)
        return String(data: data, encoding: .utf8) ?? ""
    }
}
