// GENERATED CODE - DO NOT MODIFY BY HAND

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
      input: json['input'] as Map<String, dynamic>?,
      providerExecuted: json['provider_executed'] as bool?,
      isDynamic: json['dynamic'] as bool?,
    );

Map<String, dynamic> _$ToolCallToJson(ToolCall instance) => <String, dynamic>{
      'tool_call_id': instance.toolCallId,
      'tool_name': instance.toolName,
      'input': instance.input,
      'provider_executed': instance.providerExecuted,
      'dynamic': instance.isDynamic,
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
      raw: GenerateResult.fromJson(json['raw'] as Map<String, dynamic>),
    );

Map<String, dynamic> _$GenerateTextResultToJson(GenerateTextResult instance) =>
    <String, dynamic>{
      'text': instance.text,
      'tool_calls': instance.toolCalls,
      'finish_reason': instance.finishReason,
      'usage': instance.usage,
      'raw': instance.raw,
    };

GenerateTextOptions _$GenerateTextOptionsFromJson(Map<String, dynamic> json) =>
    GenerateTextOptions(
      maxOutputTokens: (json['max_output_tokens'] as num?)?.toInt(),
      temperature: (json['temperature'] as num?)?.toDouble(),
      tools: (json['tools'] as List<dynamic>?)
          ?.map((e) => Tool.fromJson(e as Map<String, dynamic>))
          .toList(),
      toolChoice: json['tool_choice'] == null
          ? null
          : ToolChoice.fromJson(json['tool_choice']),
    );

Map<String, dynamic> _$GenerateTextOptionsToJson(
        GenerateTextOptions instance) =>
    <String, dynamic>{
      'max_output_tokens': instance.maxOutputTokens,
      'temperature': instance.temperature,
      'tools': instance.tools,
      'tool_choice': instance.toolChoice,
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
