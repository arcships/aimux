// GENERATED CODE - DO NOT MODIFY BY HAND
// ignore_for_file: type=lint, unused_element

part of 'types.dart';

// **************************************************************************
// JsonSerializableGenerator
// **************************************************************************

TokenUsage _$TokenUsageFromJson(Map<String, dynamic> json) => TokenUsage(
      total: (json['total'] as num?)?.toInt(),
      noCache: (json['no_cache'] as num?)?.toInt(),
      cacheRead: (json['cache_read'] as num?)?.toInt(),
      cacheWrite: (json['cache_write'] as num?)?.toInt(),
      text: (json['text'] as num?)?.toInt(),
      reasoning: (json['reasoning'] as num?)?.toInt(),
    );

Map<String, dynamic> _$TokenUsageToJson(TokenUsage instance) =>
    <String, dynamic>{
      'total': instance.total,
      'no_cache': instance.noCache,
      'cache_read': instance.cacheRead,
      'cache_write': instance.cacheWrite,
      'text': instance.text,
      'reasoning': instance.reasoning,
    };

Usage _$UsageFromJson(Map<String, dynamic> json) => Usage(
      inputTokens:
          TokenUsage.fromJson(json['input_tokens'] as Map<String, dynamic>),
      outputTokens:
          TokenUsage.fromJson(json['output_tokens'] as Map<String, dynamic>),
      raw: json['raw'] as Map<String, dynamic>?,
    );

Map<String, dynamic> _$UsageToJson(Usage instance) => <String, dynamic>{
      'input_tokens': instance.inputTokens,
      'output_tokens': instance.outputTokens,
      'raw': instance.raw,
    };

FinishReason _$FinishReasonFromJson(Map<String, dynamic> json) => FinishReason(
      unified: json['unified'] as String,
      raw: json['raw'] as String?,
    );

Map<String, dynamic> _$FinishReasonToJson(FinishReason instance) =>
    <String, dynamic>{
      'unified': instance.unified,
      'raw': instance.raw,
    };

ToolCall _$ToolCallFromJson(Map<String, dynamic> json) => ToolCall(
      toolCallId: json['tool_call_id'] as String,
      toolName: json['tool_name'] as String,
      input: json['input'],
      providerExecuted: json['provider_executed'] as bool?,
      isDynamic: json['dynamic'] as bool?,
      thoughtSignature: json['thought_signature'] as String?,
      providerMetadata: json['provider_metadata'],
      invalid: json['invalid'] as bool?,
      error: json['error'],
    );

Map<String, dynamic> _$ToolCallToJson(ToolCall instance) => <String, dynamic>{
      'tool_call_id': instance.toolCallId,
      'tool_name': instance.toolName,
      'input': instance.input,
      'provider_executed': instance.providerExecuted,
      'dynamic': instance.isDynamic,
      'thought_signature': instance.thoughtSignature,
      if (instance.providerMetadata != null)
        'provider_metadata': instance.providerMetadata,
      'invalid': instance.invalid,
      'error': instance.error,
    };

FunctionTool _$FunctionToolFromJson(Map<String, dynamic> json) => FunctionTool(
      name: json['name'] as String,
      description: json['description'] as String?,
      inputSchema: json['input_schema'] as Map<String, dynamic>,
      strict: json['strict'] as bool?,
      providerOptions: json['provider_options'] as Map<String, dynamic>?,
      inputExamples: (json['input_examples'] as List<dynamic>?)
          ?.map((e) => e as Map<String, dynamic>)
          .toList(),
    );

Map<String, dynamic> _$FunctionToolToJson(FunctionTool instance) =>
    <String, dynamic>{
      'name': instance.name,
      'description': instance.description,
      'input_schema': instance.inputSchema,
      'strict': instance.strict,
      'provider_options': instance.providerOptions,
      'input_examples': instance.inputExamples,
    };

ResponseMetadata _$ResponseMetadataFromJson(Map<String, dynamic> json) =>
    ResponseMetadata(
      id: json['id'] as String?,
      timestamp: json['timestamp'] as String?,
      modelId: json['model_id'] as String?,
    );

Map<String, dynamic> _$ResponseMetadataToJson(ResponseMetadata instance) =>
    <String, dynamic>{
      'id': instance.id,
      'timestamp': instance.timestamp,
      'model_id': instance.modelId,
    };

GenerateTextResult _$GenerateTextResultFromJson(Map<String, dynamic> json) =>
    GenerateTextResult(
      text: json['text'] as String,
      toolCalls: (json['tool_calls'] as List<dynamic>)
          .map((e) => ToolCall.fromJson(e as Map<String, dynamic>))
          .toList(),
      finishReason:
          FinishReason.fromJson(json['finish_reason'] as Map<String, dynamic>),
      usage: Usage.fromJson(json['usage'] as Map<String, dynamic>),
      warnings: (json['warnings'] as List<dynamic>?)
              ?.map((e) => e as Map<String, dynamic>)
              .toList() ??
          const [],
      raw: GenerateResult.fromJson(json['raw'] as Map<String, dynamic>),
      reasoning: (json['reasoning'] as List<dynamic>?)
              ?.map((e) => e as Map<String, dynamic>)
              .toList() ??
          const [],
      reasoningText: json['reasoning_text'] as String? ?? '',
      sources: (json['sources'] as List<dynamic>?)
              ?.map((e) => e as Map<String, dynamic>)
              .toList() ??
          const [],
      files: (json['files'] as List<dynamic>?)
              ?.map((e) => e as Map<String, dynamic>)
              .toList() ??
          const [],
      responseMessages: (json['response_messages'] as List<dynamic>?)
              ?.map((e) => ModelMessage.fromJson(e as Map<String, dynamic>))
              .toList() ??
          const [],
      rawFinishReason: json['raw_finish_reason'] as String?,
      providerMetadata: json['provider_metadata'] as Map<String, dynamic>?,
      response: json['response'] != null
          ? ResponseMetadata.fromJson(json['response'] as Map<String, dynamic>)
          : ResponseMetadata(),
      totalUsage: json['total_usage'] != null
          ? Usage.fromJson(json['total_usage'] as Map<String, dynamic>)
          : Usage(inputTokens: TokenUsage(), outputTokens: TokenUsage()),
    );

Map<String, dynamic> _$GenerateTextResultToJson(GenerateTextResult instance) =>
    <String, dynamic>{
      'text': instance.text,
      'tool_calls': instance.toolCalls,
      'finish_reason': instance.finishReason,
      'usage': instance.usage,
      'warnings': instance.warnings,
      'raw': instance.raw,
      'reasoning': instance.reasoning,
      'reasoning_text': instance.reasoningText,
      'sources': instance.sources,
      'files': instance.files,
      'response_messages': instance.responseMessages,
      'raw_finish_reason': instance.rawFinishReason,
      'provider_metadata': instance.providerMetadata,
      'response': instance.response,
      'total_usage': instance.totalUsage,
    };

GenerateObjectResult _$GenerateObjectResultFromJson(
        Map<String, dynamic> json) =>
    GenerateObjectResult(
      object: json['object'],
      finishReason:
          FinishReason.fromJson(json['finish_reason'] as Map<String, dynamic>),
      rawFinishReason: json['raw_finish_reason'] as String?,
      usage: json['usage'] != null
          ? Usage.fromJson(json['usage'] as Map<String, dynamic>)
          : Usage(inputTokens: TokenUsage(), outputTokens: TokenUsage()),
      warnings: (json['warnings'] as List<dynamic>?)
              ?.map((e) => e as Map<String, dynamic>)
              .toList() ??
          const [],
      reasoning: json['reasoning'] as String?,
      providerMetadata: json['provider_metadata'] as Map<String, dynamic>?,
      response: json['response'] != null
          ? ResponseMetadata.fromJson(json['response'] as Map<String, dynamic>)
          : ResponseMetadata(),
      raw: GenerateTextResult.fromJson(json['raw'] as Map<String, dynamic>),
    );

Map<String, dynamic> _$GenerateObjectResultToJson(
        GenerateObjectResult instance) =>
    <String, dynamic>{
      'object': instance.object,
      'finish_reason': instance.finishReason,
      'raw_finish_reason': instance.rawFinishReason,
      'usage': instance.usage,
      'warnings': instance.warnings,
      'reasoning': instance.reasoning,
      'provider_metadata': instance.providerMetadata,
      'response': instance.response,
      'raw': instance.raw,
    };

StreamTextResultAggregated _$StreamTextResultAggregatedFromJson(
        Map<String, dynamic> json) =>
    StreamTextResultAggregated(
      text: json['text'] as String? ?? '',
      reasoning: (json['reasoning'] as List<dynamic>?)
              ?.map((e) => e as Map<String, dynamic>)
              .toList() ??
          const [],
      reasoningText: json['reasoning_text'] as String? ?? '',
      toolCalls: (json['tool_calls'] as List<dynamic>?)
              ?.map((e) => ToolCall.fromJson(e as Map<String, dynamic>))
              .toList() ??
          const [],
      sources: (json['sources'] as List<dynamic>?)
              ?.map((e) => e as Map<String, dynamic>)
              .toList() ??
          const [],
      files: (json['files'] as List<dynamic>?)
              ?.map((e) => e as Map<String, dynamic>)
              .toList() ??
          const [],
      finishReason:
          FinishReason.fromJson(json['finish_reason'] as Map<String, dynamic>),
      rawFinishReason: json['raw_finish_reason'] as String?,
      usage: json['usage'] != null
          ? Usage.fromJson(json['usage'] as Map<String, dynamic>)
          : Usage(inputTokens: TokenUsage(), outputTokens: TokenUsage()),
      totalUsage: json['total_usage'] != null
          ? Usage.fromJson(json['total_usage'] as Map<String, dynamic>)
          : Usage(inputTokens: TokenUsage(), outputTokens: TokenUsage()),
      warnings: (json['warnings'] as List<dynamic>?)
              ?.map((e) => e as Map<String, dynamic>)
              .toList() ??
          const [],
      providerMetadata: json['provider_metadata'] as Map<String, dynamic>?,
      response: json['response'] == null
          ? null
          : ResponseMetadata.fromJson(json['response'] as Map<String, dynamic>),
      responseMessages: (json['response_messages'] as List<dynamic>?)
              ?.map((e) => ModelMessage.fromJson(e as Map<String, dynamic>))
              .toList() ??
          const [],
    );

Map<String, dynamic> _$StreamTextResultAggregatedToJson(
        StreamTextResultAggregated instance) =>
    <String, dynamic>{
      'text': instance.text,
      'reasoning': instance.reasoning,
      'reasoning_text': instance.reasoningText,
      'tool_calls': instance.toolCalls,
      'sources': instance.sources,
      'files': instance.files,
      'finish_reason': instance.finishReason,
      'raw_finish_reason': instance.rawFinishReason,
      'usage': instance.usage,
      'total_usage': instance.totalUsage,
      'warnings': instance.warnings,
      'provider_metadata': instance.providerMetadata,
      'response': instance.response,
      'response_messages': instance.responseMessages,
    };

TimeoutConfiguration _$TimeoutConfigurationFromJson(
        Map<String, dynamic> json) =>
    TimeoutConfiguration(
      totalMs: (json['total_ms'] as num?)?.toInt(),
      firstChunkMs: (json['first_chunk_ms'] as num?)?.toInt(),
      chunkMs: (json['chunk_ms'] as num?)?.toInt(),
    );

Map<String, dynamic> _$TimeoutConfigurationToJson(
        TimeoutConfiguration instance) =>
    <String, dynamic>{
      'total_ms': instance.totalMs,
      'first_chunk_ms': instance.firstChunkMs,
      'chunk_ms': instance.chunkMs,
    };

GenerateTextOptions _$GenerateTextOptionsFromJson(Map<String, dynamic> json) =>
    GenerateTextOptions(
      maxOutputTokens: (json['max_output_tokens'] as num?)?.toInt(),
      temperature: (json['temperature'] as num?)?.toDouble(),
      stopSequences: (json['stop_sequences'] as List<dynamic>?)
          ?.map((e) => e as String)
          .toList(),
      topP: (json['top_p'] as num?)?.toDouble(),
      topK: (json['top_k'] as num?)?.toDouble(),
      presencePenalty: (json['presence_penalty'] as num?)?.toDouble(),
      frequencyPenalty: (json['frequency_penalty'] as num?)?.toDouble(),
      responseFormat: json['response_format'] as Map<String, dynamic>?,
      seed: (json['seed'] as num?)?.toInt(),
      tools: (json['tools'] as List<dynamic>?)
          ?.map((e) => Tool.fromJson(e as Map<String, dynamic>))
          .toList(),
      toolChoice: json['tool_choice'] == null
          ? null
          : ToolChoice.fromJson(json['tool_choice']),
      headers: (json['headers'] as Map<String, dynamic>?)?.map(
        (k, e) => MapEntry(k, e as String),
      ),
      providerOptions: json['provider_options'] as Map<String, dynamic>?,
      reasoning:
          $enumDecodeNullable(_$ReasoningEffortEnumMap, json['reasoning']),
      instructions: json['instructions'] as String?,
      bodyOverrides: json['body_overrides'] as Map<String, dynamic>?,
      maxRetries: (json['max_retries'] as num?)?.toInt(),
      timeout: json['timeout'] == null
          ? null
          : TimeoutConfiguration.fromJson(
              json['timeout'] as Map<String, dynamic>),
    );

Map<String, dynamic> _$GenerateTextOptionsToJson(
        GenerateTextOptions instance) =>
    <String, dynamic>{
      'max_output_tokens': instance.maxOutputTokens,
      'temperature': instance.temperature,
      'stop_sequences': instance.stopSequences,
      'top_p': instance.topP,
      'top_k': instance.topK,
      'presence_penalty': instance.presencePenalty,
      'frequency_penalty': instance.frequencyPenalty,
      'response_format': instance.responseFormat,
      'seed': instance.seed,
      'tools': instance.tools,
      'tool_choice': instance.toolChoice,
      'headers': instance.headers,
      'provider_options': instance.providerOptions,
      'reasoning': instance.reasoning,
      'instructions': instance.instructions,
      'body_overrides': instance.bodyOverrides,
      'max_retries': instance.maxRetries,
      'timeout': instance.timeout,
    };

const _$ReasoningEffortEnumMap = {
  ReasoningEffort.providerDefault: 'providerDefault',
  ReasoningEffort.none: 'none',
  ReasoningEffort.minimal: 'minimal',
  ReasoningEffort.low: 'low',
  ReasoningEffort.medium: 'medium',
  ReasoningEffort.high: 'high',
  ReasoningEffort.xhigh: 'xhigh',
};

ModelMessage _$ModelMessageFromJson(Map<String, dynamic> json) => ModelMessage(
      role: json['role'] as String,
      content: json['content'] as Object,
    );

Map<String, dynamic> _$ModelMessageToJson(ModelMessage instance) =>
    <String, dynamic>{
      'role': instance.role,
      'content': instance.content,
    };

// ─────────────────────────────────────────────────────────────────────────────
// OpenAI Chat Completions output (RFC-0026).
// ─────────────────────────────────────────────────────────────────────────────

ChatCompletion _$ChatCompletionFromJson(Map<String, dynamic> json) =>
    ChatCompletion(
      id: json['id'] as String,
      object: json['object'] as String,
      created: (json['created'] as num).toInt(),
      model: json['model'] as String,
      choices: (json['choices'] as List<dynamic>)
          .map((e) => ChatCompletionChoice.fromJson(e as Map<String, dynamic>))
          .toList(),
      usage: ChatCompletionUsage.fromJson(json['usage'] as Map<String, dynamic>),
      systemFingerprint: json['system_fingerprint'] as String?,
    );

Map<String, dynamic> _$ChatCompletionToJson(ChatCompletion instance) =>
    <String, dynamic>{
      'id': instance.id,
      'object': instance.object,
      'created': instance.created,
      'model': instance.model,
      'choices': instance.choices,
      'usage': instance.usage,
      'system_fingerprint': instance.systemFingerprint,
    };

ChatCompletionChoice _$ChatCompletionChoiceFromJson(Map<String, dynamic> json) =>
    ChatCompletionChoice(
      index: (json['index'] as num).toInt(),
      message: ChatCompletionMessage.fromJson(
          json['message'] as Map<String, dynamic>),
      finishReason: json['finish_reason'] as String?,
      logprobs: json['logprobs'],
    );

Map<String, dynamic> _$ChatCompletionChoiceToJson(
        ChatCompletionChoice instance) =>
    <String, dynamic>{
      'index': instance.index,
      'message': instance.message,
      'finish_reason': instance.finishReason,
      'logprobs': instance.logprobs,
    };

ChatCompletionMessage _$ChatCompletionMessageFromJson(
        Map<String, dynamic> json) =>
    ChatCompletionMessage(
      role: json['role'] as String,
      content: json['content'] as String?,
      reasoningContent: json['reasoning_content'] as String?,
      toolCalls: (json['tool_calls'] as List<dynamic>?)
          ?.map((e) => ChatCompletionToolCall.fromJson(e as Map<String, dynamic>))
          .toList(),
      annotations: json['annotations'] as List<dynamic>?,
    );

Map<String, dynamic> _$ChatCompletionMessageToJson(
        ChatCompletionMessage instance) =>
    <String, dynamic>{
      'role': instance.role,
      'content': instance.content,
      'reasoning_content': instance.reasoningContent,
      'tool_calls': instance.toolCalls,
      'annotations': instance.annotations,
    };

ChatCompletionToolCall _$ChatCompletionToolCallFromJson(
        Map<String, dynamic> json) =>
    ChatCompletionToolCall(
      id: json['id'] as String,
      toolType: json['type'] as String,
      function: ChatCompletionFunction.fromJson(
          json['function'] as Map<String, dynamic>),
    );

Map<String, dynamic> _$ChatCompletionToolCallToJson(
        ChatCompletionToolCall instance) =>
    <String, dynamic>{
      'id': instance.id,
      'type': instance.toolType,
      'function': instance.function,
    };

ChatCompletionFunction _$ChatCompletionFunctionFromJson(
        Map<String, dynamic> json) =>
    ChatCompletionFunction(
      name: json['name'] as String,
      arguments: json['arguments'] as String,
    );

Map<String, dynamic> _$ChatCompletionFunctionToJson(
        ChatCompletionFunction instance) =>
    <String, dynamic>{
      'name': instance.name,
      'arguments': instance.arguments,
    };

ChatCompletionUsage _$ChatCompletionUsageFromJson(Map<String, dynamic> json) =>
    ChatCompletionUsage(
      promptTokens: (json['prompt_tokens'] as num).toInt(),
      completionTokens: (json['completion_tokens'] as num).toInt(),
      totalTokens: (json['total_tokens'] as num).toInt(),
      promptTokensDetails: json['prompt_tokens_details'] == null
          ? null
          : PromptTokensDetails.fromJson(
              json['prompt_tokens_details'] as Map<String, dynamic>),
      completionTokensDetails: json['completion_tokens_details'] == null
          ? null
          : CompletionTokensDetails.fromJson(
              json['completion_tokens_details'] as Map<String, dynamic>),
    );

Map<String, dynamic> _$ChatCompletionUsageToJson(
        ChatCompletionUsage instance) =>
    <String, dynamic>{
      'prompt_tokens': instance.promptTokens,
      'completion_tokens': instance.completionTokens,
      'total_tokens': instance.totalTokens,
      'prompt_tokens_details': instance.promptTokensDetails,
      'completion_tokens_details': instance.completionTokensDetails,
    };

PromptTokensDetails _$PromptTokensDetailsFromJson(Map<String, dynamic> json) =>
    PromptTokensDetails(
      cachedTokens: (json['cached_tokens'] as num).toInt(),
      cacheWriteTokens: (json['cache_write_tokens'] as num?)?.toInt(),
    );

Map<String, dynamic> _$PromptTokensDetailsToJson(
        PromptTokensDetails instance) =>
    <String, dynamic>{
      'cached_tokens': instance.cachedTokens,
      'cache_write_tokens': instance.cacheWriteTokens,
    };

CompletionTokensDetails _$CompletionTokensDetailsFromJson(
        Map<String, dynamic> json) =>
    CompletionTokensDetails(
      reasoningTokens: (json['reasoning_tokens'] as num?)?.toInt(),
    );

Map<String, dynamic> _$CompletionTokensDetailsToJson(
        CompletionTokensDetails instance) =>
    <String, dynamic>{
      'reasoning_tokens': instance.reasoningTokens,
    };

ChatCompletionChunk _$ChatCompletionChunkFromJson(Map<String, dynamic> json) =>
    ChatCompletionChunk(
      id: json['id'] as String,
      object: json['object'] as String,
      created: (json['created'] as num).toInt(),
      model: json['model'] as String,
      choices: (json['choices'] as List<dynamic>)
          .map((e) =>
              ChatCompletionChunkChoice.fromJson(e as Map<String, dynamic>))
          .toList(),
      usage: json['usage'] == null
          ? null
          : ChatCompletionUsage.fromJson(json['usage'] as Map<String, dynamic>),
    );

Map<String, dynamic> _$ChatCompletionChunkToJson(
        ChatCompletionChunk instance) =>
    <String, dynamic>{
      'id': instance.id,
      'object': instance.object,
      'created': instance.created,
      'model': instance.model,
      'choices': instance.choices,
      'usage': instance.usage,
    };

ChatCompletionChunkChoice _$ChatCompletionChunkChoiceFromJson(
        Map<String, dynamic> json) =>
    ChatCompletionChunkChoice(
      index: (json['index'] as num).toInt(),
      delta: ChatCompletionDelta.fromJson(
          json['delta'] as Map<String, dynamic>),
      finishReason: json['finish_reason'] as String?,
      logprobs: json['logprobs'],
    );

Map<String, dynamic> _$ChatCompletionChunkChoiceToJson(
        ChatCompletionChunkChoice instance) =>
    <String, dynamic>{
      'index': instance.index,
      'delta': instance.delta,
      'finish_reason': instance.finishReason,
      'logprobs': instance.logprobs,
    };

ChatCompletionDelta _$ChatCompletionDeltaFromJson(Map<String, dynamic> json) =>
    ChatCompletionDelta(
      role: json['role'] as String?,
      content: json['content'] as String?,
      reasoningContent: json['reasoning_content'] as String?,
      toolCalls: (json['tool_calls'] as List<dynamic>?)
          ?.map((e) =>
              ChatCompletionChunkToolCall.fromJson(e as Map<String, dynamic>))
          .toList(),
    );

Map<String, dynamic> _$ChatCompletionDeltaToJson(
        ChatCompletionDelta instance) =>
    <String, dynamic>{
      'role': instance.role,
      'content': instance.content,
      'reasoning_content': instance.reasoningContent,
      'tool_calls': instance.toolCalls,
    };

ChatCompletionChunkToolCall _$ChatCompletionChunkToolCallFromJson(
        Map<String, dynamic> json) =>
    ChatCompletionChunkToolCall(
      index: (json['index'] as num).toInt(),
      id: json['id'] as String?,
      toolType: json['type'] as String?,
      function: ChatCompletionChunkFunction.fromJson(
          json['function'] as Map<String, dynamic>),
    );

Map<String, dynamic> _$ChatCompletionChunkToolCallToJson(
        ChatCompletionChunkToolCall instance) =>
    <String, dynamic>{
      'index': instance.index,
      'id': instance.id,
      'type': instance.toolType,
      'function': instance.function,
    };

ChatCompletionChunkFunction _$ChatCompletionChunkFunctionFromJson(
        Map<String, dynamic> json) =>
    ChatCompletionChunkFunction(
      name: json['name'] as String?,
      arguments: json['arguments'] as String?,
    );

Map<String, dynamic> _$ChatCompletionChunkFunctionToJson(
        ChatCompletionChunkFunction instance) =>
    <String, dynamic>{
      'name': instance.name,
      'arguments': instance.arguments,
    };
