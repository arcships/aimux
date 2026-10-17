import Foundation

/// A tool call as the provider produced it (`aimux_core::RawToolCall`).
///
/// Field-for-field the same as ``ToolCall``, except `input` is the model's raw
/// argument *text* rather than a parsed value — that text is exactly what a
/// repair function needs to see, and what it returns a corrected version of.
public struct RawToolCall: Codable, Equatable {
    public var toolCallId: String
    public var toolName: String
    /// The model's raw argument text, e.g. `#"{"city":"Singapore"}"#`.
    public var input: String
    public var providerExecuted: Bool?
    public var dynamic: Bool?
    public var providerMetadata: ProviderMetadata?

    public init(toolCallId: String, toolName: String, input: String,
                providerExecuted: Bool? = nil, dynamic: Bool? = nil,
                providerMetadata: ProviderMetadata? = nil) {
        self.toolCallId = toolCallId; self.toolName = toolName; self.input = input
        self.providerExecuted = providerExecuted; self.dynamic = dynamic
        self.providerMetadata = providerMetadata
    }

    private enum CodingKeys: String, CodingKey {
        case type, toolCallId, toolName, input, providerExecuted, dynamic, providerMetadata
    }

    /// Reads a call; a `type` field, when present, must be `"tool-call"`.
    public init(from decoder: Decoder) throws {
        try decoder.checkTypeTag("tool-call")
        let c = try decoder.container(keyedBy: CodingKeys.self)
        toolCallId = try c.decode(String.self, forKey: .toolCallId)
        toolName = try c.decode(String.self, forKey: .toolName)
        input = try c.decode(String.self, forKey: .input)
        providerExecuted = try c.decodeIfPresent(Bool.self, forKey: .providerExecuted)
        dynamic = try c.decodeIfPresent(Bool.self, forKey: .dynamic)
        providerMetadata = try c.decodeIfPresent(ProviderMetadata.self, forKey: .providerMetadata)
    }

    /// Writes the call with its `"type": "tool-call"` tag, as the AI SDK's
    /// `LanguageModelV4ToolCall`.
    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode("tool-call", forKey: .type)
        try c.encode(toolCallId, forKey: .toolCallId)
        try c.encode(toolName, forKey: .toolName)
        try c.encode(input, forKey: .input)
        try c.encodeIfPresent(providerExecuted, forKey: .providerExecuted)
        try c.encodeIfPresent(dynamic, forKey: .dynamic)
        try c.encodeIfPresent(providerMetadata, forKey: .providerMetadata)
    }
}

/// Data consumed by the pure tool repair operations.
public struct ToolCallRepairContext: Codable, Equatable {
    /// The failing call, with the provider's raw argument text.
    public var toolCall: RawToolCall
    /// The failure the call already carries: an `AiMuxError` object tagged by
    /// `name` (the same encoding as ``ToolCall/error``), e.g.
    /// `{"name":"AI_InvalidToolInputError","toolName":…,"toolInput":…,"cause":…}`.
    public var error: JSONValue
    /// JSON Schema of the named tool. A name that does not resolve to a
    /// function tool yields the AI SDK's default empty-object schema.
    public var inputSchema: JSONValue
    /// The tool set the call was generated with.
    public var tools: [Tool]
    /// The messages the model saw, derived from the same prompt.
    public var messages: [ModelMessage]
    /// The instructions the call was generated with, if any.
    public var instructions: String?

    public init(toolCall: RawToolCall, error: JSONValue, inputSchema: JSONValue,
                tools: [Tool], messages: [ModelMessage], instructions: String? = nil) {
        self.toolCall = toolCall; self.error = error; self.inputSchema = inputSchema
        self.tools = tools; self.messages = messages; self.instructions = instructions
    }
}
