// Computed result properties are excluded from Codable.
extension GenerateTextResult {
    public var finalStep: StepResult { steps[steps.count - 1] }
    public var text: String {
        finalStep.content.compactMap { part in
            if case let .text(_, text) = part { return text }
            return nil
        }.joined()
    }
    public var content: [ContentPart] { steps.flatMap(\.content) }
    public var reasoning: [ReasoningPart] {
        finalStep.content.compactMap {
            switch $0 {
            case let .reasoning(metadata, text): .reasoning(providerMetadata: metadata, text: text)
            case let .reasoningFile(file, metadata): .reasoningFile(file: file, providerMetadata: metadata)
            default: nil
            }
        }
    }
    public var reasoningText: String? {
        let text = reasoning.compactMap { part in
            if case let .reasoning(_, text) = part { return text }
            return nil
        }.joined()
        return text.isEmpty ? nil : text
    }
    public var files: [GeneratedFile] {
        content.compactMap {
            if case let .file(file, _) = $0 { return file }
            return nil
        }
    }
    public var sources: [TypedSource] {
        content.compactMap {
            switch $0 {
            case let .sourceUrl(id, metadata, _, title, url):
                .url(id: id, providerMetadata: metadata, title: title, type: "source", url: url)
            case let .sourceDocument(filename, id, mediaType, metadata, _, title):
                .document(filename: filename, id: id, mediaType: mediaType,
                          providerMetadata: metadata, title: title, type: "source")
            default: nil
            }
        }
    }
    public var toolCalls: [TypedToolCall] {
        content.compactMap {
            guard case let .toolCall(dynamic, error, input, invalid, executed, metadata, id, name) = $0 else {
                return nil
            }
            return TypedToolCall(dynamic: dynamic, error: error, input: input, invalid: invalid,
                                 providerExecuted: executed, providerMetadata: metadata,
                                 toolCallId: id, toolName: name, type: "tool-call")
        }
    }
    public var staticToolCalls: [TypedToolCall] { toolCalls.filter { $0.dynamic != true } }
    public var dynamicToolCalls: [TypedToolCall] { toolCalls.filter { $0.dynamic == true } }
    public var toolResults: [TaggedToolResult] {
        content.compactMap {
            guard case let .toolResult(dynamic, input, output, preliminary, executed, metadata, id, name) = $0 else {
                return nil
            }
            return TaggedToolResult(dynamic: dynamic, input: input, output: output, preliminary: preliminary,
                                    providerExecuted: executed, providerMetadata: metadata,
                                    toolCallId: id, toolName: name, type: "tool-result")
        }
    }
    public var staticToolResults: [TaggedToolResult] { toolResults.filter { $0.dynamic != true } }
    public var dynamicToolResults: [TaggedToolResult] { toolResults.filter { $0.dynamic == true } }
    public var finishReason: FinishReasonUnified { finalStep.finishReason }
    public var rawFinishReason: String? { finalStep.rawFinishReason }
    public var usage: LanguageModelUsage { totalUsage }
    public var warnings: [Warning] { steps.flatMap(\.warnings) }
    public var request: RequestInfo { finalStep.request }
    public var response: StepResponse { finalStep.response }
    public var providerMetadata: [String: [String: JSONValue]]? { finalStep.providerMetadata }
    public var responseMessages: [ModelMessage] { steps.flatMap { $0.response.messages } }
    public var output: FieldPresence<JSONValue> {
        if case .missing = outputValue {
            return outputParsed == false ? .missing : .value(.string(text))
        }
        return outputValue
    }
}
