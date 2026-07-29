// types.dart — typed models mirroring aimux-core/bindings/*.ts (ts-rs).
//
// Hand-written Dart classes with json_serializable. Field names are
// camelCase; `@JsonKey(name: ...)` maps to the snake_case wire format
// produced by the Rust core (serde). These wrap the raw
// `Map<String, dynamic>` boundary exposed by the dart:ffi binding in
// `aimux.dart`, so callers get compile-time types instead of dynamic maps.
//
// Wire shapes are derived from the Rust structs in aimux-core
// (`types.rs`, `tool.rs`, `generate.rs`):
//   - GenerateTextResult  (generate.rs:90)
//   - ToolCall            (tool.rs:102)
//   - Usage / TokenUsage  (types.rs:33, types.rs:44)
//   - FinishReason        (types.rs:10)

import 'package:json_annotation/json_annotation.dart';

part 'types.g.dart';

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

  ToolCall({
    required this.toolCallId,
    required this.toolName,
    this.input,
    this.providerExecuted,
    this.isDynamic,
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

class GenerateContent {
  final String tag;
  final Map<String, dynamic> data;
  GenerateContent({required this.tag, required this.data});
  String? get text => data['text'] as String?;
  String? get toolCallId => data['tool_call_id'] as String?;
  String? get toolName => data['tool_name'] as String?;
  factory GenerateContent.fromJson(Map<String, dynamic> json) {
    final e = json.entries.first;
    return GenerateContent(tag: e.key, data: e.value as Map<String, dynamic>);
  }
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
  final GenerateResult raw;

  GenerateTextResult({
    required this.text,
    required this.toolCalls,
    required this.finishReason,
    required this.usage,
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
  final List<Tool>? tools;
  @JsonKey(name: 'tool_choice')
  final ToolChoice? toolChoice;

  GenerateTextOptions({
    this.maxOutputTokens,
    this.temperature,
    this.tools,
    this.toolChoice,
  });

  factory GenerateTextOptions.fromJson(Map<String, dynamic> json) {
    return GenerateTextOptions(
      maxOutputTokens: json['max_output_tokens'] as int?,
      temperature: (json['temperature'] as num?)?.toDouble(),
      tools: (json['tools'] as List<dynamic>?)
          ?.map((e) => Tool.fromJson(e as Map<String, dynamic>))
          .toList(),
      toolChoice: json['tool_choice'] != null
          ? ToolChoice.fromJson(json['tool_choice'])
          : null,
    );
  }
  Map<String, dynamic> toJson() => {
        if (maxOutputTokens != null) 'max_output_tokens': maxOutputTokens,
        if (temperature != null) 'temperature': temperature,
        if (tools != null) 'tools': tools!.map((t) => t.toJson()).toList(),
        if (toolChoice != null) 'tool_choice': toolChoice!.toJson(),
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
}

// ─────────────────────────────────────────────────────────────────────────────
// Streaming
// ─────────────────────────────────────────────────────────────────────────────

/// A single chunk in the stream returned by `stream_text`. Mirrors
/// `StreamPart.ts`.
///
/// `StreamPart` is an externally-tagged union: each part is a single-key map
/// like `{"TextDelta": {"id": "...", "delta": "..."}}`. Rather than model
/// every variant with json_serializable, we keep the raw map and expose typed
/// accessors + a `fromJson`/`toJson` factory for the common variants.
class StreamPart {
  final Map<String, dynamic> _raw;

  StreamPart(this._raw);

  factory StreamPart.fromJson(Map<String, dynamic> json) => StreamPart(json);

  Map<String, dynamic> toJson() => Map<String, dynamic>.from(_raw);

  /// The union tag — the single top-level key (e.g. `"TextDelta"`,
  /// `"ToolCall"`, `"Finish"`, `"Error"`), or `''` if the part is empty.
  String get type => _raw.isNotEmpty ? _raw.keys.first : '';

  /// The inner payload map, or `null` if the part has no payload.
  Map<String, dynamic>? get data =>
      _raw.isNotEmpty ? (_raw.values.first as Map<String, dynamic>?) : null;

  // ── Text variants ────────────────────────────────────────────────────────
  bool get isTextStart => type == 'TextStart';
  bool get isTextDelta => type == 'TextDelta';
  bool get isTextEnd => type == 'TextEnd';

  /// The text delta carried by a `TextDelta` part, else `null`.
  String? get textDelta => isTextDelta ? (data?['delta'] as String?) : null;

  /// The stream part id for text parts, else `null`.
  String? get textId =>
      (isTextStart || isTextDelta || isTextEnd) ? (data?['id'] as String?) : null;

  // ── Tool variants ─────────────────────────────────────────────────────────
  bool get isToolInputStart => type == 'ToolInputStart';
  bool get isToolInputDelta => type == 'ToolInputDelta';
  bool get isToolInputEnd => type == 'ToolInputEnd';
  bool get isToolCall => type == 'ToolCall';

  /// Tool name for `ToolCall` / `ToolInputStart` parts, else `null`.
  String? get toolName =>
      (isToolCall || isToolInputStart) ? (data?['tool_name'] as String?) : null;

  /// Tool call id for `ToolCall` parts, else `null`.
  String? get toolCallId =>
      isToolCall ? (data?['tool_call_id'] as String?) : null;

  /// Parsed tool input for `ToolCall` parts, else `null`.
  Map<String, dynamic>? get toolInput =>
      isToolCall ? (data?['input'] as Map<String, dynamic>?) : null;

  // ── Finish / error ───────────────────────────────────────────────────────
  bool get isFinish => type == 'Finish';
  bool get isError => type == 'Error';

  /// Usage reported by a `Finish` part, else `null`.
  Usage? get finishUsage {
    if (!isFinish) return null;
    final usage = data?['usage'];
    return usage is Map<String, dynamic> ? Usage.fromJson(usage) : null;
  }

  @override
  String toString() => 'StreamPart($type: $data)';
}
