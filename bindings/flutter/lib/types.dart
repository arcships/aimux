// types.dart — typed models mirroring aimux-core/bindings/*.ts (ts-rs).
//
// Hand-written Dart classes with json_serializable. Field names are
// camelCase; `@JsonKey(name: ...)` maps to the snake_case wire format
// produced by the Rust core (serde). These wrap the raw
// `Map<String, dynamic>` boundary exposed by the dart:ffi binding in
// `aimux.dart`, so callers get compile-time types instead of dynamic maps.
//
// Wire shapes are derived from the Rust structs in aimux-core
// (`types.rs`, `tool.rs`, `generate.rs`, `result.rs`, `stream_part.rs`,
// `shared.rs`):
//   - GenerateTextResult  (generate.rs:90)
//   - GenerateContent      (result.rs)        — sealed, externally-tagged
//   - StreamPart           (stream_part.rs)   — sealed, externally-tagged
//   - ContentPart          (content.rs)       — sealed, internally-tagged (type)
//   - FileData/FileBytes   (shared.rs)        — sealed, externally-tagged
//   - ToolCall             (tool.rs:102)
//   - Usage / TokenUsage   (types.rs:33, types.rs:44)
//   - FinishReason         (types.rs:10)

import 'package:json_annotation/json_annotation.dart';

part 'types.g.dart';

// ─────────────────────────────────────────────────────────────────────────────
// Shared enums
// ─────────────────────────────────────────────────────────────────────────────

enum Role {
  system('system'),
  user('user'),
  assistant('assistant'),
  tool('tool');

  const Role(this.wireValue);
  final String wireValue;
  static Role fromJson(String value) =>
      Role.values.firstWhere((item) => item.wireValue == value);
  String toJson() => wireValue;
}

enum FinishReasonUnified {
  stop('stop'),
  length('length'),
  contentFilter('content-filter'),
  toolCalls('tool-calls'),
  error('error'),
  other('other');

  const FinishReasonUnified(this.wireValue);
  final String wireValue;
  static FinishReasonUnified fromJson(String value) =>
      FinishReasonUnified.values.firstWhere((item) => item.wireValue == value);
  String toJson() => wireValue;
}

enum ReasoningEffort {
  providerDefault('provider-default'),
  none('none'),
  minimal('minimal'),
  low('low'),
  medium('medium'),
  high('high'),
  xhigh('xhigh');

  const ReasoningEffort(this.wireValue);
  final String wireValue;
  static ReasoningEffort fromJson(String value) =>
      ReasoningEffort.values.firstWhere((item) => item.wireValue == value);
  String toJson() => wireValue;
}

// ─────────────────────────────────────────────────────────────────────────────
// Token usage
// ─────────────────────────────────────────────────────────────────────────────

/// Token usage detail (with cache breakdown). Mirrors `TokenUsage.ts`.
///
/// All fields are nullable: `total` is always serialized by the Rust core
/// (null when unknown), the rest are omitted when `None`.
@JsonSerializable()
class TokenUsage {
  final int? total;
  @JsonKey(name: 'no_cache')
  final int? noCache;
  @JsonKey(name: 'cache_read')
  final int? cacheRead;
  @JsonKey(name: 'cache_write')
  final int? cacheWrite;
  final int? text;
  final int? reasoning;

  TokenUsage({
    this.total,
    this.noCache,
    this.cacheRead,
    this.cacheWrite,
    this.text,
    this.reasoning,
  });

  factory TokenUsage.fromJson(Map<String, dynamic> json) =>
      _$TokenUsageFromJson(json);
  Map<String, dynamic> toJson() => _$TokenUsageToJson(this);
}

/// Token usage statistics. Mirrors `Usage.ts`.
@JsonSerializable()
class Usage {
  @JsonKey(name: 'input_tokens')
  final TokenUsage inputTokens;
  @JsonKey(name: 'output_tokens')
  final TokenUsage outputTokens;
  /// Raw usage information from the provider (opaque JSON).
  final Map<String, dynamic>? raw;

  Usage({required this.inputTokens, required this.outputTokens, this.raw});

  factory Usage.fromJson(Map<String, dynamic> json) => _$UsageFromJson(json);
  Map<String, dynamic> toJson() => _$UsageToJson(this);
}

// ─────────────────────────────────────────────────────────────────────────────
// Finish reason
// ─────────────────────────────────────────────────────────────────────────────

/// Why generation stopped. Mirrors `FinishReason.ts`.
///
/// `unified` is the kebab-case unified reason (`"stop"`, `"length"`,
/// `"content-filter"`, `"tool-calls"`, `"error"`, `"other"`); `raw` is the
/// provider-specific reason string (nullable).
@JsonSerializable()
class FinishReason {
  final String unified;
  final String? raw;

  FinishReason({required this.unified, this.raw});

  factory FinishReason.fromJson(Map<String, dynamic> json) =>
      _$FinishReasonFromJson(json);
  Map<String, dynamic> toJson() => _$FinishReasonToJson(this);
}

// ─────────────────────────────────────────────────────────────────────────────
// Tool calls
// ─────────────────────────────────────────────────────────────────────────────

/// A tool call requested by the model. Mirrors `ToolCall.ts`.
@JsonSerializable()
class ToolCall {
  @JsonKey(name: 'tool_call_id')
  final String toolCallId;
  @JsonKey(name: 'tool_name')
  final String toolName;
  final Map<String, dynamic>? input;
  @JsonKey(name: 'provider_executed')
  final bool? providerExecuted;
  // `dynamic` is a Dart built-in identifier — field is named `isDynamic`
  // and mapped to the `dynamic` JSON key.
  @JsonKey(name: 'dynamic')
  final bool? isDynamic;
  @JsonKey(name: 'thought_signature')
  final String? thoughtSignature;

  ToolCall({
    required this.toolCallId,
    required this.toolName,
    this.input,
    this.providerExecuted,
    this.isDynamic,
    this.thoughtSignature,
  });

  factory ToolCall.fromJson(Map<String, dynamic> json) =>
      _$ToolCallFromJson(json);
  Map<String, dynamic> toJson() => _$ToolCallToJson(this);
}

// ─────────────────────────────────────────────────────────────────────────────
// Tools
// ─────────────────────────────────────────────────────────────────────────────

/// A function tool definition. Mirrors `FunctionTool.ts`.
@JsonSerializable()
class FunctionTool {
  final String name;
  final String? description;
  @JsonKey(name: 'input_schema')
  final Map<String, dynamic> inputSchema;
  final bool? strict;
  @JsonKey(name: 'provider_options')
  final Map<String, dynamic>? providerOptions;
  @JsonKey(name: 'input_examples')
  final List<Map<String, dynamic>>? inputExamples;

  FunctionTool({
    required this.name,
    this.description,
    required this.inputSchema,
    this.strict,
    this.providerOptions,
    this.inputExamples,
  });

  factory FunctionTool.fromJson(Map<String, dynamic> json) =>
      _$FunctionToolFromJson(json);
  Map<String, dynamic> toJson() => _$FunctionToolToJson(this);
}

/// A tool definition: function or provider tool. Mirrors `Tool.ts`.
class Tool {
  final String type;
  final FunctionTool? function;
  final String? id;
  final String? name;
  final Map<String, dynamic>? args;

  Tool._({required this.type, this.function, this.id, this.name, this.args});

  factory Tool.function(FunctionTool fn) =>
      Tool._(type: 'function', function: fn);
  factory Tool.provider({required String id, required String name, required Map<String, dynamic> args}) =>
      Tool._(type: 'provider', id: id, name: name, args: args);

  Map<String, dynamic> toJson() {
    if (function != null) return {'type': 'function', ...?function!.toJson()};
    return {'type': 'provider', 'id': id!, 'name': name!, 'args': args ?? {}};
  }
  factory Tool.fromJson(Map<String, dynamic> json) {
    if (json['type'] == 'function') {
      return Tool._(type: 'function', function: FunctionTool.fromJson(json));
    }
    return Tool._(type: 'provider', id: json['id'] as String, name: json['name'] as String, args: json['args'] as Map<String, dynamic>?);
  }
}

/// How the model should choose tools. Mirrors `ToolChoice.ts`.
class ToolChoice {
  final String _kind;
  final String? toolName;
  const ToolChoice._(this._kind, this.toolName);
  static const auto = ToolChoice._('auto', null);
  static const none = ToolChoice._('none', null);
  static const required = ToolChoice._('required', null);
  factory ToolChoice.tool(String toolName) => ToolChoice._('tool', toolName);

  dynamic toJson() => _kind == 'tool' ? {'type': 'tool', 'toolName': toolName} : _kind;
  factory ToolChoice.fromJson(dynamic json) =>
    json is String ? ToolChoice._(json, null) : ToolChoice._('tool', (json as Map<String, dynamic>)['toolName'] as String);
}

// ─────────────────────────────────────────────────────────────────────────────
// GenerateResult (raw provider result)
// ─────────────────────────────────────────────────────────────────────────────

@JsonSerializable()
class ResponseMetadata {
  final String? id;
  final String? timestamp;
  @JsonKey(name: 'model_id') final String? modelId;
  ResponseMetadata({this.id, this.timestamp, this.modelId});
  factory ResponseMetadata.fromJson(Map<String, dynamic> json) => _$ResponseMetadataFromJson(json);
  Map<String, dynamic> toJson() => _$ResponseMetadataToJson(this);
}

/// A content item in a `GenerateResult`. Mirrors `GenerateContent.ts`
/// (`aimux-core/src/result.rs`).
///
/// Externally-tagged union — each item is a single-key map whose key is the
/// variant tag and whose value is the variant payload. Modeled as a sealed
/// class hierarchy so callers get compile-time types: every known variant is a
/// typed subclass with named fields, and [GenerateContentUnknown] is the
/// fallback for tags added by newer core versions so they pass through verbatim
/// instead of crashing.
///
/// Known variants: [GenerateContentText], [GenerateContentToolCall],
/// [GenerateContentSource], [GenerateContentReasoning], [GenerateContentFile],
/// [GenerateContentToolResult]. The `File` variant carries model-generated
/// files (e.g. images or documents) as a `FileData` tagged union under `data`,
/// alongside a `media_type` (there is **no** `filename` field). Narrow via
/// `switch`/`is`/`whereType` to read variant-specific fields.
sealed class GenerateContent {
  const GenerateContent();

  /// The variant tag — the single top-level key on the wire, e.g. `'Text'`,
  /// `'ToolCall'`. Useful for logging / generic dispatch.
  String get tag;

  /// Re-encode to the externally-tagged wire shape (`{tag: payload}`).
  Map<String, dynamic> toJson();

  /// Decode an externally-tagged content map. Unknown tags fall back to
  /// [GenerateContentUnknown] instead of throwing.
  factory GenerateContent.fromJson(Map<String, dynamic> json) {
    final e = json.entries.first;
    final payload = e.value as Map<String, dynamic>;
    return switch (e.key) {
      'Text' => GenerateContentText.fromJson(payload),
      'ToolCall' => GenerateContentToolCall.fromJson(payload),
      'Source' => GenerateContentSource.fromJson(payload),
      'Reasoning' => GenerateContentReasoning.fromJson(payload),
      'File' => GenerateContentFile.fromJson(payload),
      'ToolResult' => GenerateContentToolResult.fromJson(payload),
      _ => GenerateContentUnknown(tag: e.key, data: payload),
    };
  }
}

/// Generated text.
final class GenerateContentText extends GenerateContent {
  final String text;
  final Map<String, dynamic>? providerMetadata;

  GenerateContentText({required this.text, this.providerMetadata});

  @override
  String get tag => 'Text';

  factory GenerateContentText.fromJson(Map<String, dynamic> json) =>
      GenerateContentText(
        text: json['text'] as String,
        providerMetadata:
            json['provider_metadata'] as Map<String, dynamic>?,
      );

  @override
  Map<String, dynamic> toJson() => {
        'Text': {
          'text': text,
          if (providerMetadata != null) 'provider_metadata': providerMetadata,
        },
      };
}

/// A tool call requested by the model.
final class GenerateContentToolCall extends GenerateContent {
  final String toolCallId;
  final String toolName;
  final dynamic input;
  final bool? providerExecuted;
  final bool? isDynamic;
  final String? thoughtSignature;
  final Map<String, dynamic>? providerMetadata;

  GenerateContentToolCall({
    required this.toolCallId,
    required this.toolName,
    required this.input,
    this.providerExecuted,
    this.isDynamic,
    this.thoughtSignature,
    this.providerMetadata,
  });

  @override
  String get tag => 'ToolCall';

  factory GenerateContentToolCall.fromJson(Map<String, dynamic> json) =>
      GenerateContentToolCall(
        toolCallId: json['tool_call_id'] as String,
        toolName: json['tool_name'] as String,
        input: json['input'],
        providerExecuted: json['provider_executed'] as bool?,
        isDynamic: json['dynamic'] as bool?,
        thoughtSignature: json['thought_signature'] as String?,
        providerMetadata:
            json['provider_metadata'] as Map<String, dynamic>?,
      );

  @override
  Map<String, dynamic> toJson() => {
        'ToolCall': {
          'tool_call_id': toolCallId,
          'tool_name': toolName,
          'input': input,
          if (providerExecuted != null) 'provider_executed': providerExecuted,
          if (isDynamic != null) 'dynamic': isDynamic,
          if (thoughtSignature != null) 'thought_signature': thoughtSignature,
          if (providerMetadata != null) 'provider_metadata': providerMetadata,
        },
      };
}

/// A source / citation (e.g. URL citation from search-preview models).
final class GenerateContentSource extends GenerateContent {
  final String id;
  final String sourceType;
  final String? url;
  final String? title;
  final Map<String, dynamic>? providerMetadata;

  GenerateContentSource({
    required this.id,
    required this.sourceType,
    this.url,
    this.title,
    this.providerMetadata,
  });

  @override
  String get tag => 'Source';

  factory GenerateContentSource.fromJson(Map<String, dynamic> json) =>
      GenerateContentSource(
        id: json['id'] as String,
        sourceType: json['source_type'] as String,
        url: json['url'] as String?,
        title: json['title'] as String?,
        providerMetadata:
            json['provider_metadata'] as Map<String, dynamic>?,
      );

  @override
  Map<String, dynamic> toJson() => {
        'Source': {
          'id': id,
          'source_type': sourceType,
          if (url != null) 'url': url,
          if (title != null) 'title': title,
          if (providerMetadata != null) 'provider_metadata': providerMetadata,
        },
      };
}

/// A reasoning / thinking segment produced by the model.
final class GenerateContentReasoning extends GenerateContent {
  final String text;
  final Map<String, dynamic>? providerMetadata;

  GenerateContentReasoning({required this.text, this.providerMetadata});

  @override
  String get tag => 'Reasoning';

  factory GenerateContentReasoning.fromJson(Map<String, dynamic> json) =>
      GenerateContentReasoning(
        text: json['text'] as String,
        providerMetadata:
            json['provider_metadata'] as Map<String, dynamic>?,
      );

  @override
  Map<String, dynamic> toJson() => {
        'Reasoning': {
          'text': text,
          if (providerMetadata != null) 'provider_metadata': providerMetadata,
        },
      };
}

/// A file generated by the model (e.g. an image or document). The `data`
/// field is a `FileData` tagged union; there is **no** `filename` field.
final class GenerateContentFile extends GenerateContent {
  final FileData data;
  final String mediaType;
  final Map<String, dynamic>? providerMetadata;

  GenerateContentFile({
    required this.data,
    required this.mediaType,
    this.providerMetadata,
  });

  @override
  String get tag => 'File';

  factory GenerateContentFile.fromJson(Map<String, dynamic> json) =>
      GenerateContentFile(
        data: FileData.fromJson(json['data'] as Map<String, dynamic>),
        mediaType: json['media_type'] as String,
        providerMetadata:
            json['provider_metadata'] as Map<String, dynamic>?,
      );

  @override
  Map<String, dynamic> toJson() => {
        'File': {
          'data': data.toJson(),
          'media_type': mediaType,
          if (providerMetadata != null) 'provider_metadata': providerMetadata,
        },
      };
}

/// A tool result from a provider-executed tool (e.g. xAI file_search,
/// web_search). Emitted alongside the preceding `ToolCall`.
final class GenerateContentToolResult extends GenerateContent {
  final String toolCallId;
  final String toolName;
  final dynamic result;
  final bool? isError;
  final bool? preliminary;
  final bool? isDynamic;
  final Map<String, dynamic>? providerMetadata;

  GenerateContentToolResult({
    required this.toolCallId,
    required this.toolName,
    required this.result,
    this.isError,
    this.preliminary,
    this.isDynamic,
    this.providerMetadata,
  });

  @override
  String get tag => 'ToolResult';

  factory GenerateContentToolResult.fromJson(Map<String, dynamic> json) =>
      GenerateContentToolResult(
        toolCallId: json['tool_call_id'] as String,
        toolName: json['tool_name'] as String,
        result: json['result'],
        isError: json['is_error'] as bool?,
        preliminary: json['preliminary'] as bool?,
        isDynamic: json['dynamic'] as bool?,
        providerMetadata:
            json['provider_metadata'] as Map<String, dynamic>?,
      );

  @override
  Map<String, dynamic> toJson() => {
        'ToolResult': {
          'tool_call_id': toolCallId,
          'tool_name': toolName,
          'result': result,
          if (isError != null) 'is_error': isError,
          if (preliminary != null) 'preliminary': preliminary,
          if (isDynamic != null) 'dynamic': isDynamic,
          if (providerMetadata != null) 'provider_metadata': providerMetadata,
        },
      };
}

/// Fallback for unknown/forward-compatible `GenerateContent` variants.
///
/// Newer core versions may emit tags this binding does not model yet; this
/// class passes them through verbatim (`{tag: data}`) instead of discarding
/// or crashing.
final class GenerateContentUnknown extends GenerateContent {
  @override
  final String tag;
  final Map<String, dynamic> data;

  GenerateContentUnknown({required this.tag, required this.data});

  @override
  Map<String, dynamic> toJson() => {tag: data};
}

class GenerateResult {
  final List<GenerateContent> content;
  final FinishReason finishReason;
  final Usage usage;
  final List<Map<String, dynamic>> warnings;
  final Map<String, dynamic>? providerMetadata;
  final ResponseMetadata response;
  final Map<String, dynamic>? requestBody;
  final Map<String, dynamic>? responseHeaders;
  GenerateResult({required this.content, required this.finishReason, required this.usage, this.warnings = const [], this.providerMetadata, required this.response, this.requestBody, this.responseHeaders});
  factory GenerateResult.fromJson(Map<String, dynamic> json) => GenerateResult(
    content: (json['content'] as List<dynamic>? ?? []).map((e) => GenerateContent.fromJson(e as Map<String, dynamic>)).toList(),
    finishReason: FinishReason.fromJson(json['finish_reason'] as Map<String, dynamic>),
    usage: Usage.fromJson(json['usage'] as Map<String, dynamic>),
    warnings: (json['warnings'] as List<dynamic>? ?? []).map((e) => e as Map<String, dynamic>).toList(),
    providerMetadata: json['provider_metadata'] as Map<String, dynamic>?,
    response: ResponseMetadata.fromJson(json['response'] as Map<String, dynamic>),
    requestBody: json['request_body'] as Map<String, dynamic>?,
    responseHeaders: json['response_headers'] as Map<String, dynamic>?,
  );
  Map<String, dynamic> toJson() => {'content': content.map((c) => c.toJson()).toList(), 'finish_reason': finishReason.toJson(), 'usage': usage.toJson(), 'warnings': warnings, 'response': response.toJson(), if (providerMetadata != null) 'provider_metadata': providerMetadata, if (requestBody != null) 'request_body': requestBody, if (responseHeaders != null) 'response_headers': responseHeaders};
}

// ─────────────────────────────────────────────────────────────────────────────
// Result / options
// ─────────────────────────────────────────────────────────────────────────────

/// Result of `generate_text` (user-facing). Mirrors `GenerateTextResult.ts`.
@JsonSerializable()
class GenerateTextResult {
  final String text;
  @JsonKey(name: 'tool_calls')
  final List<ToolCall> toolCalls;
  @JsonKey(name: 'finish_reason')
  final FinishReason finishReason;
  final Usage usage;
  final List<Map<String, dynamic>> warnings;
  final GenerateResult raw;

  GenerateTextResult({
    required this.text,
    required this.toolCalls,
    required this.finishReason,
    required this.usage,
    this.warnings = const [],
    required this.raw,
  });

  factory GenerateTextResult.fromJson(Map<String, dynamic> json) =>
      _$GenerateTextResultFromJson(json);
  Map<String, dynamic> toJson() => _$GenerateTextResultToJson(this);
}

/// User-facing options for `generate_text` / `stream_text`. Mirrors
/// `GenerateTextOptions.ts`.
@JsonSerializable()
class GenerateTextOptions {
  @JsonKey(name: 'max_output_tokens')
  final int? maxOutputTokens;
  final double? temperature;
  @JsonKey(name: 'stop_sequences')
  final List<String>? stopSequences;
  @JsonKey(name: 'top_p')
  final double? topP;
  @JsonKey(name: 'top_k')
  final double? topK;
  @JsonKey(name: 'presence_penalty')
  final double? presencePenalty;
  @JsonKey(name: 'frequency_penalty')
  final double? frequencyPenalty;
  @JsonKey(name: 'response_format')
  final Map<String, dynamic>? responseFormat;
  final int? seed;
  final List<Tool>? tools;
  @JsonKey(name: 'tool_choice')
  final ToolChoice? toolChoice;
  final Map<String, String>? headers;
  @JsonKey(name: 'provider_options')
  final Map<String, dynamic>? providerOptions;
  final ReasoningEffort? reasoning;
  final String? instructions;
  @JsonKey(name: 'body_overrides')
  final Map<String, dynamic>? bodyOverrides;
  @JsonKey(name: 'max_retries')
  final int? maxRetries;

  GenerateTextOptions({
    this.maxOutputTokens,
    this.temperature,
    this.stopSequences,
    this.topP,
    this.topK,
    this.presencePenalty,
    this.frequencyPenalty,
    this.responseFormat,
    this.seed,
    this.tools,
    this.toolChoice,
    this.headers,
    this.providerOptions,
    this.reasoning,
    this.instructions,
    this.bodyOverrides,
    this.maxRetries,
  });

  factory GenerateTextOptions.fromJson(Map<String, dynamic> json) {
    return GenerateTextOptions(
      maxOutputTokens: json['max_output_tokens'] as int?,
      temperature: (json['temperature'] as num?)?.toDouble(),
      stopSequences: (json['stop_sequences'] as List<dynamic>?)?.cast<String>(),
      topP: (json['top_p'] as num?)?.toDouble(),
      topK: (json['top_k'] as num?)?.toDouble(),
      presencePenalty: (json['presence_penalty'] as num?)?.toDouble(),
      frequencyPenalty: (json['frequency_penalty'] as num?)?.toDouble(),
      responseFormat: json['response_format'] as Map<String, dynamic>?,
      seed: json['seed'] as int?,
      tools: (json['tools'] as List<dynamic>?)
          ?.map((e) => Tool.fromJson(e as Map<String, dynamic>))
          .toList(),
      toolChoice: json['tool_choice'] != null
          ? ToolChoice.fromJson(json['tool_choice'])
          : null,
      headers: (json['headers'] as Map<String, dynamic>?)?.cast<String, String>(),
      providerOptions: json['provider_options'] as Map<String, dynamic>?,
      reasoning: json['reasoning'] != null
          ? ReasoningEffort.fromJson(json['reasoning'] as String)
          : null,
      instructions: json['instructions'] as String?,
      bodyOverrides: json['body_overrides'] as Map<String, dynamic>?,
      maxRetries: json['max_retries'] as int?,
    );
  }
  Map<String, dynamic> toJson() => {
        if (maxOutputTokens != null) 'max_output_tokens': maxOutputTokens,
        if (temperature != null) 'temperature': temperature,
        if (stopSequences != null) 'stop_sequences': stopSequences,
        if (topP != null) 'top_p': topP,
        if (topK != null) 'top_k': topK,
        if (presencePenalty != null) 'presence_penalty': presencePenalty,
        if (frequencyPenalty != null) 'frequency_penalty': frequencyPenalty,
        if (responseFormat != null) 'response_format': responseFormat,
        if (seed != null) 'seed': seed,
        if (tools != null) 'tools': tools!.map((t) => t.toJson()).toList(),
        if (toolChoice != null) 'tool_choice': toolChoice!.toJson(),
        if (headers != null) 'headers': headers,
        if (providerOptions != null) 'provider_options': providerOptions,
        if (reasoning != null) 'reasoning': reasoning!.toJson(),
        if (instructions != null) 'instructions': instructions,
        if (bodyOverrides != null) 'body_overrides': bodyOverrides,
        if (maxRetries != null) 'max_retries': maxRetries,
      };
}

// ─────────────────────────────────────────────────────────────────────────────
// Messages
// ─────────────────────────────────────────────────────────────────────────────

/// A single user-facing chat message. Mirrors `ModelMessage.ts`.
///
/// `content` is either a plain `String` or a `List` of content-part maps
/// (`[{"type":"text","text":"..."}, ...]`); it is kept as `Object` so both
/// shapes pass through verbatim.
@JsonSerializable()
class ModelMessage {
  final String role;
  final Object content;

  ModelMessage({required this.role, required this.content});

  factory ModelMessage.fromJson(Map<String, dynamic> json) =>
      _$ModelMessageFromJson(json);
  Map<String, dynamic> toJson() => _$ModelMessageToJson(this);

  /// Convenience view of [content] as a list of content-part maps.
  ///
  /// If [content] is a `String`, returns `[{'type': 'text', 'text': content}]`;
  /// if it is a `List`, returns it cast to `List<Map<String, dynamic>>`;
  /// otherwise an empty list. `content` itself stays `Object` so both the
  /// string and the part-list wire shapes pass through verbatim.
  List<Map<String, dynamic>> get contentParts {
    final c = content;
    if (c is String) {
      return <Map<String, dynamic>>[
        {'type': 'text', 'text': c},
      ];
    }
    if (c is List) {
      return c.whereType<Map<String, dynamic>>().toList();
    }
    return const <Map<String, dynamic>>[];
  }
}

// ─────────────────────────────────────────────────────────────────────────────
// Streaming
// ─────────────────────────────────────────────────────────────────────────────

/// A single chunk in the stream returned by `stream_text`. Mirrors
/// `StreamPart.ts` (`aimux-core/src/stream_part.rs`).
///
/// `StreamPart` is an externally-tagged union: each part is a single-key map
/// like `{"TextDelta": {"id": "...", "delta": "..."}}`. Modeled as a sealed
/// class hierarchy so callers get compile-time types: every known variant is a
/// typed subclass with named fields, and [StreamPartUnknown] is the fallback
/// for tags added by newer core versions.
///
/// Known variants: [StreamPartTextStart]/[StreamPartTextDelta]/[StreamPartTextEnd],
/// [StreamPartStreamStart], [StreamPartFinish], [StreamPartError],
/// [StreamPartToolInputStart]/[StreamPartToolInputDelta]/[StreamPartToolInputEnd],
/// [StreamPartToolCall], [StreamPartToolResult], [StreamPartFile],
/// [StreamPartReasoningStart]/[StreamPartReasoningDelta]/[StreamPartReasoningEnd],
/// [StreamPartResponseMetadata], [StreamPartSource], [StreamPartRaw]. Narrow
/// via `switch`/`is`/`whereType` to read variant-specific fields.
sealed class StreamPart {
  const StreamPart();

  /// The variant tag — the single top-level key on the wire (e.g.
  /// `'TextDelta'`, `'ToolCall'`, `'ToolResult'`, `'File'`, `'Finish'`,
  /// `'Error'`). Empty only for a malformed part.
  String get type => switch (this) {
        StreamPartTextStart() => 'TextStart',
        StreamPartTextDelta() => 'TextDelta',
        StreamPartTextEnd() => 'TextEnd',
        StreamPartStreamStart() => 'StreamStart',
        StreamPartFinish() => 'Finish',
        StreamPartError() => 'Error',
        StreamPartToolInputStart() => 'ToolInputStart',
        StreamPartToolInputDelta() => 'ToolInputDelta',
        StreamPartToolInputEnd() => 'ToolInputEnd',
        StreamPartToolCall() => 'ToolCall',
        StreamPartToolResult() => 'ToolResult',
        StreamPartFile() => 'File',
        StreamPartReasoningStart() => 'ReasoningStart',
        StreamPartReasoningDelta() => 'ReasoningDelta',
        StreamPartReasoningEnd() => 'ReasoningEnd',
        StreamPartResponseMetadata() => 'ResponseMetadata',
        StreamPartSource() => 'Source',
        StreamPartRaw() => 'Raw',
        StreamPartUnknown(:final tag) => tag,
      };

  /// Re-encode to the externally-tagged wire shape (`{tag: payload}`).
  Map<String, dynamic> toJson();

  /// Decode an externally-tagged stream-part map. Unknown tags fall back to
  /// [StreamPartUnknown] instead of throwing.
  factory StreamPart.fromJson(Map<String, dynamic> json) {
    final e = json.entries.first;
    final payload = e.value as Map<String, dynamic>;
    return switch (e.key) {
      'TextStart' => StreamPartTextStart.fromJson(payload),
      'TextDelta' => StreamPartTextDelta.fromJson(payload),
      'TextEnd' => StreamPartTextEnd.fromJson(payload),
      'StreamStart' => StreamPartStreamStart.fromJson(payload),
      'Finish' => StreamPartFinish.fromJson(payload),
      'Error' => StreamPartError.fromJson(payload),
      'ToolInputStart' => StreamPartToolInputStart.fromJson(payload),
      'ToolInputDelta' => StreamPartToolInputDelta.fromJson(payload),
      'ToolInputEnd' => StreamPartToolInputEnd.fromJson(payload),
      'ToolCall' => StreamPartToolCall.fromJson(payload),
      'ToolResult' => StreamPartToolResult.fromJson(payload),
      'File' => StreamPartFile.fromJson(payload),
      'ReasoningStart' => StreamPartReasoningStart.fromJson(payload),
      'ReasoningDelta' => StreamPartReasoningDelta.fromJson(payload),
      'ReasoningEnd' => StreamPartReasoningEnd.fromJson(payload),
      'ResponseMetadata' => StreamPartResponseMetadata.fromJson(payload),
      'Source' => StreamPartSource.fromJson(payload),
      'Raw' => StreamPartRaw.fromJson(payload),
      _ => StreamPartUnknown(tag: e.key, data: payload),
    };
  }

  @override
  String toString() => 'StreamPart($type)';
}

// ── Text variants ────────────────────────────────────────────────────────────

/// Start of a text segment.
final class StreamPartTextStart extends StreamPart {
  final String id;
  final Map<String, dynamic>? providerMetadata;

  StreamPartTextStart({required this.id, this.providerMetadata});

  factory StreamPartTextStart.fromJson(Map<String, dynamic> json) =>
      StreamPartTextStart(
        id: json['id'] as String,
        providerMetadata:
            json['provider_metadata'] as Map<String, dynamic>?,
      );

  @override
  Map<String, dynamic> toJson() => {
        'TextStart': {
          'id': id,
          if (providerMetadata != null) 'provider_metadata': providerMetadata,
        },
      };
}

/// A delta of generated text.
final class StreamPartTextDelta extends StreamPart {
  final String id;
  final String delta;
  final Map<String, dynamic>? providerMetadata;

  StreamPartTextDelta({
    required this.id,
    required this.delta,
    this.providerMetadata,
  });

  factory StreamPartTextDelta.fromJson(Map<String, dynamic> json) =>
      StreamPartTextDelta(
        id: json['id'] as String,
        delta: json['delta'] as String,
        providerMetadata:
            json['provider_metadata'] as Map<String, dynamic>?,
      );

  @override
  Map<String, dynamic> toJson() => {
        'TextDelta': {
          'id': id,
          'delta': delta,
          if (providerMetadata != null) 'provider_metadata': providerMetadata,
        },
      };
}

/// End of a text segment.
final class StreamPartTextEnd extends StreamPart {
  final String id;
  final Map<String, dynamic>? providerMetadata;

  StreamPartTextEnd({required this.id, this.providerMetadata});

  factory StreamPartTextEnd.fromJson(Map<String, dynamic> json) =>
      StreamPartTextEnd(
        id: json['id'] as String,
        providerMetadata:
            json['provider_metadata'] as Map<String, dynamic>?,
      );

  @override
  Map<String, dynamic> toJson() => {
        'TextEnd': {
          'id': id,
          if (providerMetadata != null) 'provider_metadata': providerMetadata,
        },
      };
}

// ── Stream lifecycle ──────────────────────────────────────────────────────────

/// First chunk — carries warnings from the provider.
final class StreamPartStreamStart extends StreamPart {
  final List<Map<String, dynamic>> warnings;

  StreamPartStreamStart({this.warnings = const <Map<String, dynamic>>[]});

  factory StreamPartStreamStart.fromJson(Map<String, dynamic> json) =>
      StreamPartStreamStart(
        warnings: ((json['warnings'] as List<dynamic>?) ?? const <dynamic>[])
            .map((e) => e as Map<String, dynamic>)
            .toList(),
      );

  @override
  Map<String, dynamic> toJson() => {
        'StreamStart': {'warnings': warnings},
      };
}

/// Final chunk — carries usage, finish reason, and metadata.
final class StreamPartFinish extends StreamPart {
  final FinishReason finishReason;
  final Usage usage;
  final Map<String, dynamic>? providerMetadata;

  StreamPartFinish({
    required this.finishReason,
    required this.usage,
    this.providerMetadata,
  });

  factory StreamPartFinish.fromJson(Map<String, dynamic> json) =>
      StreamPartFinish(
        finishReason: FinishReason.fromJson(
            json['finish_reason'] as Map<String, dynamic>),
        usage: Usage.fromJson(json['usage'] as Map<String, dynamic>),
        providerMetadata:
            json['provider_metadata'] as Map<String, dynamic>?,
      );

  @override
  Map<String, dynamic> toJson() => {
        'Finish': {
          'finish_reason': finishReason.toJson(),
          'usage': usage.toJson(),
          if (providerMetadata != null) 'provider_metadata': providerMetadata,
        },
      };
}

/// An error occurred mid-stream.
final class StreamPartError extends StreamPart {
  final dynamic error;

  StreamPartError({required this.error});

  factory StreamPartError.fromJson(Map<String, dynamic> json) =>
      StreamPartError(error: json['error']);

  @override
  Map<String, dynamic> toJson() => {
        'Error': {'error': error},
      };
}

// ── Tool calls ─────────────────────────────────────────────────────────────────

/// Start of a tool call's input streaming.
final class StreamPartToolInputStart extends StreamPart {
  final String id;
  final String toolName;
  final bool? providerExecuted;
  final bool? isDynamic;
  final String? title;
  final Map<String, dynamic>? providerMetadata;

  StreamPartToolInputStart({
    required this.id,
    required this.toolName,
    this.providerExecuted,
    this.isDynamic,
    this.title,
    this.providerMetadata,
  });

  factory StreamPartToolInputStart.fromJson(Map<String, dynamic> json) =>
      StreamPartToolInputStart(
        id: json['id'] as String,
        toolName: json['tool_name'] as String,
        providerExecuted: json['provider_executed'] as bool?,
        isDynamic: json['dynamic'] as bool?,
        title: json['title'] as String?,
        providerMetadata:
            json['provider_metadata'] as Map<String, dynamic>?,
      );

  @override
  Map<String, dynamic> toJson() => {
        'ToolInputStart': {
          'id': id,
          'tool_name': toolName,
          if (providerExecuted != null) 'provider_executed': providerExecuted,
          if (isDynamic != null) 'dynamic': isDynamic,
          if (title != null) 'title': title,
          if (providerMetadata != null) 'provider_metadata': providerMetadata,
        },
      };
}

/// A delta of tool call input (partial JSON).
final class StreamPartToolInputDelta extends StreamPart {
  final String id;
  final String delta;
  final Map<String, dynamic>? providerMetadata;

  StreamPartToolInputDelta({
    required this.id,
    required this.delta,
    this.providerMetadata,
  });

  factory StreamPartToolInputDelta.fromJson(Map<String, dynamic> json) =>
      StreamPartToolInputDelta(
        id: json['id'] as String,
        delta: json['delta'] as String,
        providerMetadata:
            json['provider_metadata'] as Map<String, dynamic>?,
      );

  @override
  Map<String, dynamic> toJson() => {
        'ToolInputDelta': {
          'id': id,
          'delta': delta,
          if (providerMetadata != null) 'provider_metadata': providerMetadata,
        },
      };
}

/// End of a tool call's input streaming.
final class StreamPartToolInputEnd extends StreamPart {
  final String id;
  final Map<String, dynamic>? providerMetadata;

  StreamPartToolInputEnd({required this.id, this.providerMetadata});

  factory StreamPartToolInputEnd.fromJson(Map<String, dynamic> json) =>
      StreamPartToolInputEnd(
        id: json['id'] as String,
        providerMetadata:
            json['provider_metadata'] as Map<String, dynamic>?,
      );

  @override
  Map<String, dynamic> toJson() => {
        'ToolInputEnd': {
          'id': id,
          if (providerMetadata != null) 'provider_metadata': providerMetadata,
        },
      };
}

/// A complete tool call (alternative to the start/delta/end flow).
final class StreamPartToolCall extends StreamPart {
  final String toolCallId;
  final String toolName;
  final dynamic input;
  final bool? providerExecuted;
  final bool? isDynamic;
  final String? thoughtSignature;
  final Map<String, dynamic>? providerMetadata;

  StreamPartToolCall({
    required this.toolCallId,
    required this.toolName,
    required this.input,
    this.providerExecuted,
    this.isDynamic,
    this.thoughtSignature,
    this.providerMetadata,
  });

  factory StreamPartToolCall.fromJson(Map<String, dynamic> json) =>
      StreamPartToolCall(
        toolCallId: json['tool_call_id'] as String,
        toolName: json['tool_name'] as String,
        input: json['input'],
        providerExecuted: json['provider_executed'] as bool?,
        isDynamic: json['dynamic'] as bool?,
        thoughtSignature: json['thought_signature'] as String?,
        providerMetadata:
            json['provider_metadata'] as Map<String, dynamic>?,
      );

  @override
  Map<String, dynamic> toJson() => {
        'ToolCall': {
          'tool_call_id': toolCallId,
          'tool_name': toolName,
          'input': input,
          if (providerExecuted != null) 'provider_executed': providerExecuted,
          if (isDynamic != null) 'dynamic': isDynamic,
          if (thoughtSignature != null) 'thought_signature': thoughtSignature,
          if (providerMetadata != null) 'provider_metadata': providerMetadata,
        },
      };
}

/// A tool result (provider-executed tools).
final class StreamPartToolResult extends StreamPart {
  final String toolCallId;
  final String toolName;
  final dynamic result;
  final bool? isError;
  final bool? preliminary;
  final bool? isDynamic;
  final Map<String, dynamic>? providerMetadata;

  StreamPartToolResult({
    required this.toolCallId,
    required this.toolName,
    required this.result,
    this.isError,
    this.preliminary,
    this.isDynamic,
    this.providerMetadata,
  });

  factory StreamPartToolResult.fromJson(Map<String, dynamic> json) =>
      StreamPartToolResult(
        toolCallId: json['tool_call_id'] as String,
        toolName: json['tool_name'] as String,
        result: json['result'],
        isError: json['is_error'] as bool?,
        preliminary: json['preliminary'] as bool?,
        isDynamic: json['dynamic'] as bool?,
        providerMetadata:
            json['provider_metadata'] as Map<String, dynamic>?,
      );

  @override
  Map<String, dynamic> toJson() => {
        'ToolResult': {
          'tool_call_id': toolCallId,
          'tool_name': toolName,
          'result': result,
          if (isError != null) 'is_error': isError,
          if (preliminary != null) 'preliminary': preliminary,
          if (isDynamic != null) 'dynamic': isDynamic,
          if (providerMetadata != null) 'provider_metadata': providerMetadata,
        },
      };
}

// ── File ──────────────────────────────────────────────────────────────────────

/// A file generated by the model (e.g. an image or document). The `data`
/// field is a `FileData` tagged union; there is **no** `filename` field.
final class StreamPartFile extends StreamPart {
  final FileData data;
  final String mediaType;
  final Map<String, dynamic>? providerMetadata;

  StreamPartFile({
    required this.data,
    required this.mediaType,
    this.providerMetadata,
  });

  factory StreamPartFile.fromJson(Map<String, dynamic> json) => StreamPartFile(
        data: FileData.fromJson(json['data'] as Map<String, dynamic>),
        mediaType: json['media_type'] as String,
        providerMetadata:
            json['provider_metadata'] as Map<String, dynamic>?,
      );

  @override
  Map<String, dynamic> toJson() => {
        'File': {
          'data': data.toJson(),
          'media_type': mediaType,
          if (providerMetadata != null) 'provider_metadata': providerMetadata,
        },
      };
}

// ── Reasoning ──────────────────────────────────────────────────────────────────

/// Start of a reasoning / thinking segment.
final class StreamPartReasoningStart extends StreamPart {
  final String id;
  final Map<String, dynamic>? providerMetadata;

  StreamPartReasoningStart({required this.id, this.providerMetadata});

  factory StreamPartReasoningStart.fromJson(Map<String, dynamic> json) =>
      StreamPartReasoningStart(
        id: json['id'] as String,
        providerMetadata:
            json['provider_metadata'] as Map<String, dynamic>?,
      );

  @override
  Map<String, dynamic> toJson() => {
        'ReasoningStart': {
          'id': id,
          if (providerMetadata != null) 'provider_metadata': providerMetadata,
        },
      };
}

/// A delta of a reasoning / thinking segment.
final class StreamPartReasoningDelta extends StreamPart {
  final String id;
  final String delta;
  final Map<String, dynamic>? providerMetadata;

  StreamPartReasoningDelta({
    required this.id,
    required this.delta,
    this.providerMetadata,
  });

  factory StreamPartReasoningDelta.fromJson(Map<String, dynamic> json) =>
      StreamPartReasoningDelta(
        id: json['id'] as String,
        delta: json['delta'] as String,
        providerMetadata:
            json['provider_metadata'] as Map<String, dynamic>?,
      );

  @override
  Map<String, dynamic> toJson() => {
        'ReasoningDelta': {
          'id': id,
          'delta': delta,
          if (providerMetadata != null) 'provider_metadata': providerMetadata,
        },
      };
}

/// End of a reasoning / thinking segment.
final class StreamPartReasoningEnd extends StreamPart {
  final String id;
  final Map<String, dynamic>? providerMetadata;

  StreamPartReasoningEnd({required this.id, this.providerMetadata});

  factory StreamPartReasoningEnd.fromJson(Map<String, dynamic> json) =>
      StreamPartReasoningEnd(
        id: json['id'] as String,
        providerMetadata:
            json['provider_metadata'] as Map<String, dynamic>?,
      );

  @override
  Map<String, dynamic> toJson() => {
        'ReasoningEnd': {
          'id': id,
          if (providerMetadata != null) 'provider_metadata': providerMetadata,
        },
      };
}

// ── Metadata / sources / raw ──────────────────────────────────────────────────

/// Response metadata (id, timestamp, model_id).
final class StreamPartResponseMetadata extends StreamPart {
  final String? id;
  final String? timestamp;
  final String? modelId;

  StreamPartResponseMetadata({this.id, this.timestamp, this.modelId});

  factory StreamPartResponseMetadata.fromJson(Map<String, dynamic> json) =>
      StreamPartResponseMetadata(
        id: json['id'] as String?,
        timestamp: json['timestamp'] as String?,
        modelId: json['model_id'] as String?,
      );

  @override
  Map<String, dynamic> toJson() => {
        'ResponseMetadata': {
          if (id != null) 'id': id,
          if (timestamp != null) 'timestamp': timestamp,
          if (modelId != null) 'model_id': modelId,
        },
      };
}

/// A source / citation (e.g. URL citation from search-preview models).
final class StreamPartSource extends StreamPart {
  final String id;
  final String sourceType;
  final String? url;
  final String? title;
  final Map<String, dynamic>? providerMetadata;

  StreamPartSource({
    required this.id,
    required this.sourceType,
    this.url,
    this.title,
    this.providerMetadata,
  });

  factory StreamPartSource.fromJson(Map<String, dynamic> json) =>
      StreamPartSource(
        id: json['id'] as String,
        sourceType: json['source_type'] as String,
        url: json['url'] as String?,
        title: json['title'] as String?,
        providerMetadata:
            json['provider_metadata'] as Map<String, dynamic>?,
      );

  @override
  Map<String, dynamic> toJson() => {
        'Source': {
          'id': id,
          'source_type': sourceType,
          if (url != null) 'url': url,
          if (title != null) 'title': title,
          if (providerMetadata != null) 'provider_metadata': providerMetadata,
        },
      };
}

/// A raw chunk from the provider (for debugging, when `include_raw_chunks`
/// is set).
final class StreamPartRaw extends StreamPart {
  final dynamic rawValue;

  StreamPartRaw({required this.rawValue});

  factory StreamPartRaw.fromJson(Map<String, dynamic> json) =>
      StreamPartRaw(rawValue: json['raw_value']);

  @override
  Map<String, dynamic> toJson() => {
        'Raw': {'raw_value': rawValue},
      };
}

/// Fallback for unknown/forward-compatible `StreamPart` variants.
///
/// Newer core versions may emit tags this binding does not model yet; this
/// class passes them through verbatim (`{tag: data}`) instead of discarding
/// or crashing.
final class StreamPartUnknown extends StreamPart {
  final String tag;
  final Map<String, dynamic> data;

  StreamPartUnknown({required this.tag, required this.data});

  @override
  Map<String, dynamic> toJson() => {tag: data};
}

// ─────────────────────────────────────────────────────────────────────────────
// FileBytes (shared.rs) — externally-tagged: {"Binary": [...]} / {"Base64": "..."}
// ─────────────────────────────────────────────────────────────────────────────

sealed class FileBytes {
  const FileBytes();
  factory FileBytes.fromJson(Map<String, dynamic> json) {
    final e = json.entries.first;
    return switch (e.key) {
      'Binary' => FileBytesBinary(data: (e.value as List).cast<int>()),
      'Base64' => FileBytesBase64(data: e.value as String),
      _ => throw FormatException('unknown FileBytes tag: $e'),
    };
  }
  Map<String, dynamic> toJson();
}

final class FileBytesBinary extends FileBytes {
  final List<int> data;
  const FileBytesBinary({required this.data});
  @override
  Map<String, dynamic> toJson() => {'Binary': data};
}

final class FileBytesBase64 extends FileBytes {
  final String data;
  const FileBytesBase64({required this.data});
  @override
  Map<String, dynamic> toJson() => {'Base64': data};
}

// ─────────────────────────────────────────────────────────────────────────────
// FileData (shared.rs) — externally-tagged: {"Data": {...}} / {"Url": {...}} / ...
// ─────────────────────────────────────────────────────────────────────────────

sealed class FileData {
  const FileData();
  factory FileData.fromJson(Map<String, dynamic> json) {
    final e = json.entries.first;
    return switch (e.key) {
      'Data' => FileDataData(
          data: FileBytes.fromJson((e.value as Map)['data'] as Map<String, dynamic>)),
      'Url' => FileDataUrl(url: (e.value as Map)['url'] as String),
      'Reference' => FileDataReference(
          reference: (e.value as Map)['reference'] as Map<String, dynamic>),
      'Text' => FileDataText(text: (e.value as Map)['text'] as String),
      _ => throw FormatException('unknown FileData tag: $e'),
    };
  }
  Map<String, dynamic> toJson();
}

final class FileDataData extends FileData {
  final FileBytes data;
  const FileDataData({required this.data});
  @override
  Map<String, dynamic> toJson() => {'Data': {'data': data.toJson()}};
}

final class FileDataUrl extends FileData {
  final String url;
  const FileDataUrl({required this.url});
  @override
  Map<String, dynamic> toJson() => {'Url': {'url': url}};
}

final class FileDataReference extends FileData {
  final Map<String, dynamic> reference;
  const FileDataReference({required this.reference});
  @override
  Map<String, dynamic> toJson() => {'Reference': {'reference': reference}};
}

final class FileDataText extends FileData {
  final String text;
  const FileDataText({required this.text});
  @override
  Map<String, dynamic> toJson() => {'Text': {'text': text}};
}

// ─────────────────────────────────────────────────────────────────────────────
// ContentPart (content.rs) — internally-tagged: {"type": "text", "text": "..."}
// ─────────────────────────────────────────────────────────────────────────────

sealed class ContentPart {
  const ContentPart();
  factory ContentPart.fromJson(Map<String, dynamic> json) {
    final tag = json['type'] as String;
    return switch (tag) {
      'text' => ContentPartText.fromJson(json),
      'image' => ContentPartImage.fromJson(json),
      'file' => ContentPartFile.fromJson(json),
      'file_base64' => ContentPartFileBase64.fromJson(json),
      'file_url' => ContentPartFileUrl.fromJson(json),
      'file_reference' => ContentPartFileReference.fromJson(json),
      'reasoning' => ContentPartReasoning.fromJson(json),
      'tool_call' => ContentPartToolCall.fromJson(json),
      'tool_result' => ContentPartToolResult.fromJson(json),
      _ => ContentPartUnknown(tag: tag, data: json),
    };
  }
  Map<String, dynamic> toJson();
}

final class ContentPartText extends ContentPart {
  final String text;
  final Map<String, dynamic>? providerOptions;
  const ContentPartText({required this.text, this.providerOptions});
  static ContentPartText fromJson(Map<String, dynamic> json) => ContentPartText(
        text: json['text'] as String,
        providerOptions: json['provider_options'] as Map<String, dynamic>?,
      );
  @override
  Map<String, dynamic> toJson() => {
        'type': 'text',
        'text': text,
        if (providerOptions != null) 'provider_options': providerOptions,
      };
}

final class ContentPartImage extends ContentPart {
  final List<int> image;
  final String mediaType;
  final Map<String, dynamic>? providerOptions;
  const ContentPartImage(
      {required this.image, required this.mediaType, this.providerOptions});
  static ContentPartImage fromJson(Map<String, dynamic> json) =>
      ContentPartImage(
        image: (json['image'] as List).cast<int>(),
        mediaType: json['media_type'] as String,
        providerOptions: json['provider_options'] as Map<String, dynamic>?,
      );
  @override
  Map<String, dynamic> toJson() => {
        'type': 'image',
        'image': image,
        'media_type': mediaType,
        if (providerOptions != null) 'provider_options': providerOptions,
      };
}

final class ContentPartFile extends ContentPart {
  final List<int> data;
  final String mediaType;
  final String? filename;
  final Map<String, dynamic>? providerOptions;
  const ContentPartFile(
      {required this.data, required this.mediaType, this.filename, this.providerOptions});
  static ContentPartFile fromJson(Map<String, dynamic> json) => ContentPartFile(
        data: (json['data'] as List).cast<int>(),
        mediaType: json['media_type'] as String,
        filename: json['filename'] as String?,
        providerOptions: json['provider_options'] as Map<String, dynamic>?,
      );
  @override
  Map<String, dynamic> toJson() => {
        'type': 'file',
        'data': data,
        'media_type': mediaType,
        if (filename != null) 'filename': filename,
        if (providerOptions != null) 'provider_options': providerOptions,
      };
}

final class ContentPartFileBase64 extends ContentPart {
  final String data;
  final String mediaType;
  final String? filename;
  final Map<String, dynamic>? providerOptions;
  const ContentPartFileBase64(
      {required this.data, required this.mediaType, this.filename, this.providerOptions});
  static ContentPartFileBase64 fromJson(Map<String, dynamic> json) =>
      ContentPartFileBase64(
        data: json['data'] as String,
        mediaType: json['media_type'] as String,
        filename: json['filename'] as String?,
        providerOptions: json['provider_options'] as Map<String, dynamic>?,
      );
  @override
  Map<String, dynamic> toJson() => {
        'type': 'file_base64',
        'data': data,
        'media_type': mediaType,
        if (filename != null) 'filename': filename,
        if (providerOptions != null) 'provider_options': providerOptions,
      };
}

final class ContentPartFileUrl extends ContentPart {
  final String url;
  final String mediaType;
  final Map<String, dynamic>? providerOptions;
  const ContentPartFileUrl(
      {required this.url, required this.mediaType, this.providerOptions});
  static ContentPartFileUrl fromJson(Map<String, dynamic> json) =>
      ContentPartFileUrl(
        url: json['url'] as String,
        mediaType: json['media_type'] as String,
        providerOptions: json['provider_options'] as Map<String, dynamic>?,
      );
  @override
  Map<String, dynamic> toJson() => {
        'type': 'file_url',
        'url': url,
        'media_type': mediaType,
        if (providerOptions != null) 'provider_options': providerOptions,
      };
}

final class ContentPartFileReference extends ContentPart {
  final String mediaType;
  final Map<String, dynamic> reference;
  final String? filename;
  final Map<String, dynamic>? providerOptions;
  const ContentPartFileReference(
      {required this.mediaType, required this.reference, this.filename, this.providerOptions});
  static ContentPartFileReference fromJson(Map<String, dynamic> json) =>
      ContentPartFileReference(
        mediaType: json['media_type'] as String,
        reference: json['reference'] as Map<String, dynamic>,
        filename: json['filename'] as String?,
        providerOptions: json['provider_options'] as Map<String, dynamic>?,
      );
  @override
  Map<String, dynamic> toJson() => {
        'type': 'file_reference',
        'media_type': mediaType,
        'reference': reference,
        if (filename != null) 'filename': filename,
        if (providerOptions != null) 'provider_options': providerOptions,
      };
}

final class ContentPartReasoning extends ContentPart {
  final String text;
  final String? signature;
  final Map<String, dynamic>? providerOptions;
  const ContentPartReasoning(
      {required this.text, this.signature, this.providerOptions});
  static ContentPartReasoning fromJson(Map<String, dynamic> json) =>
      ContentPartReasoning(
        text: json['text'] as String,
        signature: json['signature'] as String?,
        providerOptions: json['provider_options'] as Map<String, dynamic>?,
      );
  @override
  Map<String, dynamic> toJson() => {
        'type': 'reasoning',
        'text': text,
        if (signature != null) 'signature': signature,
        if (providerOptions != null) 'provider_options': providerOptions,
      };
}

final class ContentPartToolCall extends ContentPart {
  final String toolCallId;
  final String toolName;
  final dynamic input;
  final String? thoughtSignature;
  final Map<String, dynamic>? providerOptions;
  const ContentPartToolCall(
      {required this.toolCallId, required this.toolName, required this.input, this.thoughtSignature, this.providerOptions});
  static ContentPartToolCall fromJson(Map<String, dynamic> json) =>
      ContentPartToolCall(
        toolCallId: json['tool_call_id'] as String,
        toolName: json['tool_name'] as String,
        input: json['input'],
        thoughtSignature: json['thought_signature'] as String?,
        providerOptions: json['provider_options'] as Map<String, dynamic>?,
      );
  @override
  Map<String, dynamic> toJson() => {
        'type': 'tool_call',
        'tool_call_id': toolCallId,
        'tool_name': toolName,
        'input': input,
        if (thoughtSignature != null) 'thought_signature': thoughtSignature,
        if (providerOptions != null) 'provider_options': providerOptions,
      };
}

final class ContentPartToolResult extends ContentPart {
  final String toolCallId;
  final dynamic result;
  final String? toolName;
  final bool? isError;
  final bool? preliminary;
  final bool? isDynamic;
  final Map<String, dynamic>? providerOptions;
  const ContentPartToolResult(
      {required this.toolCallId, required this.result, this.toolName, this.isError, this.preliminary, this.isDynamic, this.providerOptions});
  static ContentPartToolResult fromJson(Map<String, dynamic> json) =>
      ContentPartToolResult(
        toolCallId: json['tool_call_id'] as String,
        result: json['result'],
        toolName: json['tool_name'] as String?,
        isError: json['is_error'] as bool?,
        preliminary: json['preliminary'] as bool?,
        isDynamic: json['dynamic'] as bool?,
        providerOptions: json['provider_options'] as Map<String, dynamic>?,
      );
  @override
  Map<String, dynamic> toJson() => {
        'type': 'tool_result',
        'tool_call_id': toolCallId,
        'result': result,
        if (toolName != null) 'tool_name': toolName,
        if (isError != null) 'is_error': isError,
        if (preliminary != null) 'preliminary': preliminary,
        if (isDynamic != null) 'dynamic': isDynamic,
        if (providerOptions != null) 'provider_options': providerOptions,
      };
}

final class ContentPartUnknown extends ContentPart {
  final String tag;
  final Map<String, dynamic> data;
  const ContentPartUnknown({required this.tag, required this.data});
  @override
  Map<String, dynamic> toJson() => data;
}
