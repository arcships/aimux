
/** A text result returned by the typed client. Derived properties are non-enumerable. */
export interface GenerateTextResult extends GenerateTextResultWire {
  readonly output?: unknown
  readonly finalStep: StepResult
  readonly content: ContentPart[]
  readonly text: string
  readonly reasoning: ReasoningPart[]
  readonly reasoningText?: string
  readonly files: GeneratedFile[]
  readonly sources: TypedSource[]
  readonly toolCalls: TypedToolCall[]
  readonly staticToolCalls: GenerateTextResult['toolCalls']
  readonly dynamicToolCalls: GenerateTextResult['toolCalls']
  readonly toolResults: TaggedToolResult[]
  readonly staticToolResults: GenerateTextResult['toolResults']
  readonly dynamicToolResults: GenerateTextResult['toolResults']
  readonly finishReason: FinishReasonUnified
  readonly rawFinishReason?: string | null
  readonly usage: LanguageModelUsage
  readonly warnings: Warning[]
  readonly request: RequestInfo
  readonly response: StepResponse
  readonly providerMetadata?: StepResult['providerMetadata']
  readonly responseMessages: ModelMessage[]
}
