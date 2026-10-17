// GENERATED CODE - DO NOT MODIFY BY HAND

part of 'types.dart';

// **************************************************************************
// JsonSerializableGenerator
// **************************************************************************

ChatCompletion _$ChatCompletionFromJson(Map<String, dynamic> json) =>
    ChatCompletion(
      id: json['id'] as String,
      object: json['object'] as String,
      created: (json['created'] as num).toInt(),
      model: json['model'] as String,
      choices: (json['choices'] as List<dynamic>)
          .map((e) => ChatCompletionChoice.fromJson(e as Map<String, dynamic>))
          .toList(),
      usage:
          ChatCompletionUsage.fromJson(json['usage'] as Map<String, dynamic>),
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

ChatCompletionChoice _$ChatCompletionChoiceFromJson(
        Map<String, dynamic> json) =>
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
          ?.map(
              (e) => ChatCompletionToolCall.fromJson(e as Map<String, dynamic>))
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
      delta:
          ChatCompletionDelta.fromJson(json['delta'] as Map<String, dynamic>),
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
