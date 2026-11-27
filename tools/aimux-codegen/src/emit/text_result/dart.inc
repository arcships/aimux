
  StepResult get finalStep => steps.last;
  String get text =>
      finalStep.content.whereType<ContentPartText>().map((p) => p.text).join();
  List<ContentPart> get content => steps.expand((step) => step.content).toList();
  List<ContentPart> get reasoning => finalStep.content
      .where((p) => p is ContentPartReasoning || p is ContentPartReasoningFile)
      .toList();
  String? get reasoningText {
    final text = reasoning.whereType<ContentPartReasoning>().map((p) => p.text).join();
    return text.isEmpty ? null : text;
  }

  List<GeneratedFile> get files =>
      content.whereType<ContentPartFile>().map((p) => p.file).toList();
  List<ContentPart> get sources => content
      .where((p) => p is ContentPartSourceUrl || p is ContentPartSourceDocument)
      .toList();
  List<ContentPartToolCall> get toolCalls =>
      content.whereType<ContentPartToolCall>().toList();
  List<ContentPartToolCall> get staticToolCalls =>
      toolCalls.where((p) => p.dynamic_ != true).toList();
  List<ContentPartToolCall> get dynamicToolCalls =>
      toolCalls.where((p) => p.dynamic_ == true).toList();
  List<ContentPartToolResult> get toolResults =>
      content.whereType<ContentPartToolResult>().toList();
  List<ContentPartToolResult> get staticToolResults =>
      toolResults.where((p) => p.dynamic_ != true).toList();
  List<ContentPartToolResult> get dynamicToolResults =>
      toolResults.where((p) => p.dynamic_ == true).toList();
  FinishReasonUnified get finishReason => finalStep.finishReason;
  String? get rawFinishReason => finalStep.rawFinishReason;
  LanguageModelUsage get usage => totalUsage;
  List<Warning> get warnings => steps.expand((step) => step.warnings).toList();
  RequestInfo get request => finalStep.request;
  StepResponse get response => finalStep.response;
  Map<String, Map<String, Object?>>? get providerMetadata =>
      finalStep.providerMetadata;
  List<ModelMessage> get responseMessages =>
      steps.expand((step) => step.response.messages).toList();
  Field<Object?> get output => outputValue.isPresent
      ? outputValue
      : (outputParsed == false ? const Field.absent() : Field.of(text));
