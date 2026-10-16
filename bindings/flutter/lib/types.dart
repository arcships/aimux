// types.dart — typed models for the JSON the Rust core exchanges across the
// C ABI. Mirrors the ts-rs generated types in `bindings/node/src/types/*.ts`:
// camelCase field names, unions tagged with a `type` key (kebab-case values;
// `sourceType` for sources), optional fields **absent** rather than `null`.
//
// The classes are hand-written: every `fromJson` reads exactly that JSON and
// every `toJson` writes it back (nested values are plain maps, ready for
// `jsonEncode`). Only the OpenAI Chat Completions family at the bottom keeps
// OpenAI's own snake_case and uses `json_serializable`.
//
// Weak-typed fields (`Map<String, dynamic>`) are the ones the TS types leave
// open (`JsonValue`, provider metadata, warnings, errors).

import 'dart:async';
import 'dart:convert' show base64;

import 'package:json_annotation/json_annotation.dart';

part 'types.g.dart';

typedef Json = Map<String, dynamic>;

List<T> _list<T>(Object? v, T Function(Json) f) =>
    [for (final e in (v as List? ?? const <Object?>[])) f(e as Json)];
List<Json> _maps(Object? v) => _list(v, (m) => m);
Json? _opt(Object? v) => v as Json?;
List<T>? _optList<T>(Object? v, T Function(Json) f) =>
    v == null ? null : _list(v, f);

/// Bytes in the AI SDK's JSON: a `Uint8Array` is a base64 string
/// (`@ai-sdk/provider-utils` `uint8-utils.ts:12-23`), never an array of ints.
String encodeBase64Bytes(List<int> bytes) => base64.encode(bytes);

/// Decodes base64 as `convertBase64ToUint8Array` does (`uint8-utils.ts:6-10`):
/// `-` and `_` read as `+` and `/`, padding optional, ASCII whitespace ignored.
List<int> decodeBase64Bytes(Object? json) {
  if (json is! String) {
    throw FormatException('bytes must be a base64 string, got: $json');
  }
  var text = json
      .replaceAll(RegExp(r'[\t\n\f\r =]'), '')
      .replaceAll('-', '+')
      .replaceAll('_', '/');
  text = text.padRight(text.length + (4 - text.length % 4) % 4, '=');
  return base64.decode(text);
}

// Shared enums

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

// Token usage, finish reason

/// Input token usage detail with cache breakdown. Mirrors `InputTokenUsage.ts`.
class InputTokenUsage {
  final int? total;
  final int? noCache;
  final int? cacheRead;
  final int? cacheWrite;
  const InputTokenUsage({this.total, this.noCache, this.cacheRead, this.cacheWrite});
  factory InputTokenUsage.fromJson(Json j) => InputTokenUsage(
      total: j['total'] as int?,
      noCache: j['noCache'] as int?,
      cacheRead: j['cacheRead'] as int?,
      cacheWrite: j['cacheWrite'] as int?);
  Json toJson() => {
        if (total != null) 'total': total,
        if (noCache != null) 'noCache': noCache,
        if (cacheRead != null) 'cacheRead': cacheRead,
        if (cacheWrite != null) 'cacheWrite': cacheWrite,
      };
}

/// Mirrors `OutputTokenUsage.ts`.
class OutputTokenUsage {
  final int? total;
  final int? text;
  final int? reasoning;
  const OutputTokenUsage({this.total, this.text, this.reasoning});
  factory OutputTokenUsage.fromJson(Json j) => OutputTokenUsage(
      total: j['total'] as int?,
      text: j['text'] as int?,
      reasoning: j['reasoning'] as int?);
  Json toJson() => {
        if (total != null) 'total': total,
        if (text != null) 'text': text,
        if (reasoning != null) 'reasoning': reasoning,
      };
}

/// Token usage statistics. Mirrors `Usage.ts`.
class Usage {
  final InputTokenUsage inputTokens;
  final OutputTokenUsage outputTokens;

  /// Raw usage information from the provider (opaque JSON).
  final Json? raw;

  const Usage({required this.inputTokens, required this.outputTokens, this.raw});
  factory Usage.fromJson(Json j) => Usage(
      inputTokens: InputTokenUsage.fromJson(j['inputTokens'] as Json),
      outputTokens: OutputTokenUsage.fromJson(j['outputTokens'] as Json),
      raw: _opt(j['raw']));
  Json toJson() => {
        'inputTokens': inputTokens.toJson(),
        'outputTokens': outputTokens.toJson(),
        if (raw != null) 'raw': raw,
      };
}

/// Input-token breakdown of a [LanguageModelUsage]. Mirrors `InputTokenDetails.ts`.
class InputTokenDetails {
  final int? noCacheTokens;
  final int? cacheReadTokens;
  final int? cacheWriteTokens;
  const InputTokenDetails({this.noCacheTokens, this.cacheReadTokens, this.cacheWriteTokens});
  factory InputTokenDetails.fromJson(Json j) => InputTokenDetails(
      noCacheTokens: j['noCacheTokens'] as int?,
      cacheReadTokens: j['cacheReadTokens'] as int?,
      cacheWriteTokens: j['cacheWriteTokens'] as int?);
  Json toJson() => {
        if (noCacheTokens != null) 'noCacheTokens': noCacheTokens,
        if (cacheReadTokens != null) 'cacheReadTokens': cacheReadTokens,
        if (cacheWriteTokens != null) 'cacheWriteTokens': cacheWriteTokens,
      };
}

/// Output-token breakdown of a [LanguageModelUsage]. Mirrors `OutputTokenDetails.ts`.
class OutputTokenDetails {
  final int? textTokens;
  final int? reasoningTokens;
  const OutputTokenDetails({this.textTokens, this.reasoningTokens});
  factory OutputTokenDetails.fromJson(Json j) => OutputTokenDetails(
      textTokens: j['textTokens'] as int?,
      reasoningTokens: j['reasoningTokens'] as int?);
  Json toJson() => {
        if (textTokens != null) 'textTokens': textTokens,
        if (reasoningTokens != null) 'reasoningTokens': reasoningTokens,
      };
}

/// Call-level token usage, the AI SDK's `LanguageModelUsage`: flat camelCase
/// counts, an unknown count `null` here and absent in the JSON. Provider
/// results keep [Usage]. Mirrors `LanguageModelUsage.ts`.
class LanguageModelUsage {
  final int? inputTokens;
  final InputTokenDetails inputTokenDetails;
  final int? outputTokens;
  final OutputTokenDetails outputTokenDetails;
  final int? totalTokens;

  /// Raw usage information from the provider (opaque JSON).
  final Json? raw;

  const LanguageModelUsage({
    this.inputTokens,
    this.inputTokenDetails = const InputTokenDetails(),
    this.outputTokens,
    this.outputTokenDetails = const OutputTokenDetails(),
    this.totalTokens,
    this.raw,
  });
  factory LanguageModelUsage.fromJson(Json j) => LanguageModelUsage(
      inputTokens: j['inputTokens'] as int?,
      inputTokenDetails: j['inputTokenDetails'] == null
          ? const InputTokenDetails()
          : InputTokenDetails.fromJson(j['inputTokenDetails'] as Json),
      outputTokens: j['outputTokens'] as int?,
      outputTokenDetails: j['outputTokenDetails'] == null
          ? const OutputTokenDetails()
          : OutputTokenDetails.fromJson(j['outputTokenDetails'] as Json),
      totalTokens: j['totalTokens'] as int?,
      raw: _opt(j['raw']));
  Json toJson() => {
        if (inputTokens != null) 'inputTokens': inputTokens,
        'inputTokenDetails': inputTokenDetails.toJson(),
        if (outputTokens != null) 'outputTokens': outputTokens,
        'outputTokenDetails': outputTokenDetails.toJson(),
        if (totalTokens != null) 'totalTokens': totalTokens,
        if (raw != null) 'raw': raw,
      };
}

/// Why generation stopped. Mirrors `FinishReason.ts`.
///
/// [unified] is the kebab-case unified reason ([FinishReasonUnified]);
/// [raw] is the provider-specific reason string.
class FinishReason {
  final String unified;
  final String? raw;
  const FinishReason({required this.unified, this.raw});
  factory FinishReason.fromJson(Json j) =>
      FinishReason(unified: j['unified'] as String, raw: j['raw'] as String?);
  Json toJson() => {'unified': unified, if (raw != null) 'raw': raw};
}

// Tool calls

/// A parsed tool call requested by the model. Mirrors `ToolCall.ts`.
///
/// [error] is the typed lookup, parse, schema or repair failure of an invalid
/// call: an `AiMuxError` object keyed by `name` (`{"name":"AI_NoSuchToolError",
/// "toolName":...}`).
class ToolCall {
  final String toolCallId;
  final String toolName;
  final dynamic input;
  final bool? providerExecuted;

  /// The wire key is `dynamic`, a Dart built-in identifier, hence the name.
  final bool? isDynamic;
  final Json? providerMetadata;

  /// Set by the core when the call stays invalid after optional repair.
  final bool? invalid;
  final Json? error;

  const ToolCall({
    required this.toolCallId,
    required this.toolName,
    required this.input,
    this.providerExecuted,
    this.isDynamic,
    this.providerMetadata,
    this.invalid,
    this.error,
  });

  factory ToolCall.fromJson(Json j) => ToolCall(
        toolCallId: _checkTypeTag(j, 'tool-call')['toolCallId'] as String,
        toolName: j['toolName'] as String,
        input: j['input'],
        providerExecuted: j['providerExecuted'] as bool?,
        isDynamic: j['dynamic'] as bool?,
        providerMetadata: _opt(j['providerMetadata']),
        invalid: j['invalid'] as bool?,
        error: _opt(j['error']),
      );

  /// Writes the call with its `"type": "tool-call"` tag: a call outside
  /// `content` is the AI SDK's tagged `TypedToolCall`
  /// (ai generate-text/tool-call.ts).
  Json toJson() => {
        'type': 'tool-call',
        'toolCallId': toolCallId,
        'toolName': toolName,
        'input': input,
        if (providerExecuted != null) 'providerExecuted': providerExecuted,
        if (isDynamic != null) 'dynamic': isDynamic,
        if (providerMetadata != null) 'providerMetadata': providerMetadata,
        if (invalid != null) 'invalid': invalid,
        if (error != null) 'error': error,
      };
}

/// Checks the `type` tag of a value that always carries one (a standalone
/// tool call or source): JSON written before the tag has none.
Json _checkTypeTag(Json j, String expected) {
  final tag = j['type'];
  if (tag != null && tag != expected) {
    throw FormatException('expected type "$expected", got "$tag"');
  }
  return j;
}

// Tool-call repair (RFC-0035)

/// A tool call as the model emitted it: [input] is the provider's raw argument
/// **text**, not a decoded object. Mirrors `RawToolCall.ts`.
///
/// Both the call handed to a [RepairToolCall] hook
/// ([ToolCallRepairContext.toolCall]) and the replacement it returns. The
/// replacement is re-parsed and re-validated against the tool's schema, so
/// [input] must be the argument text the tool expects (usually a JSON object
/// literal); returning text that still fails validation leaves the original
/// call invalid, now carrying that parse or validation error (not a
/// `ToolCallRepairError`).
class RawToolCall {
  final String toolCallId;
  final String toolName;

  /// The provider's raw argument text (e.g. `'{"city":"Singapore"}'`).
  final String input;

  /// Whether the provider executes the call itself. The core adopts the
  /// replacement as returned, so a hook must carry this over (as
  /// [copyWith] does) or the call turns into a client-executed one.
  final bool? providerExecuted;

  /// Whether the call targets a dynamic tool (the wire key is `dynamic`).
  final bool? isDynamic;
  final Json? providerMetadata;

  const RawToolCall({
    required this.toolCallId,
    required this.toolName,
    required this.input,
    this.providerExecuted,
    this.isDynamic,
    this.providerMetadata,
  });

  factory RawToolCall.fromJson(Json j) => RawToolCall(
        toolCallId: _checkTypeTag(j, 'tool-call')['toolCallId'] as String,
        toolName: j['toolName'] as String,
        input: j['input'] as String,
        providerExecuted: j['providerExecuted'] as bool?,
        isDynamic: j['dynamic'] as bool?,
        providerMetadata: _opt(j['providerMetadata']),
      );

  /// A copy with [input] (and optionally the id or name) replaced and every
  /// provider field kept — the usual shape of a repair.
  RawToolCall copyWith({String? toolCallId, String? toolName, String? input}) =>
      RawToolCall(
        toolCallId: toolCallId ?? this.toolCallId,
        toolName: toolName ?? this.toolName,
        input: input ?? this.input,
        providerExecuted: providerExecuted,
        isDynamic: isDynamic,
        providerMetadata: providerMetadata,
      );

  /// Writes the call with its `"type": "tool-call"` tag, as the AI SDK's
  /// `LanguageModelV4ToolCall`.
  Json toJson() => {
        'type': 'tool-call',
        'toolCallId': toolCallId,
        'toolName': toolName,
        'input': input,
        if (providerExecuted != null) 'providerExecuted': providerExecuted,
        if (isDynamic != null) 'dynamic': isDynamic,
        if (providerMetadata != null) 'providerMetadata': providerMetadata,
      };
}

/// The argument a [RepairToolCall] hook receives: one invalid tool call plus
/// everything needed to ask a model to fix it.
///
/// Built by the core from the same prompt and options the call was generated
/// with, so [tools], [messages] and [instructions] are exactly what the model
/// saw — the host never re-derives them.
class ToolCallRepairContext {
  /// The invalid call, with the provider's raw argument text.
  final RawToolCall toolCall;

  /// The lookup, parse or validation failure as an `AiMuxError` object keyed by
  /// `name` — the same shape as [ToolCall.error].
  final Json? error;

  /// The schema [toolCall] failed to satisfy.
  final Json? inputSchema;

  /// The full tool set the call was generated with.
  final List<Tool> tools;

  /// The messages the model saw.
  final List<ModelMessage> messages;

  /// The system instructions the call was generated with, if any.
  final String? instructions;

  const ToolCallRepairContext({
    required this.toolCall,
    this.error,
    this.inputSchema,
    this.tools = const [],
    this.messages = const [],
    this.instructions,
  });

  factory ToolCallRepairContext.fromJson(Json j) => ToolCallRepairContext(
        toolCall: RawToolCall.fromJson(j['toolCall'] as Json),
        error: _opt(j['error']),
        inputSchema: _opt(j['inputSchema']),
        tools: _list(j['tools'], Tool.fromJson),
        messages: _list(j['messages'], ModelMessage.fromJson),
        instructions: j['instructions'] as String?,
      );
}

/// A host-side hook that repairs one invalid tool call (RFC-0035), mirroring
/// the AI SDK's `repairToolCall`. See [GenerateTextOptions.repairToolCall].
///
/// Return a [RawToolCall] to replace the call, or null to leave it as it is.
/// Throwing means the repair failed: the call stays invalid and carries a
/// `ToolCallRepairError` whose cause is the thrown object's `toString()`.
///
/// The hook runs on the caller's isolate, after the native call has returned,
/// so it may itself call back into aimux (including another model call).
/// A `Future` return is only awaited on the streaming path, which is the only
/// asynchronous entry point; the synchronous entry points
/// ([TypedModel.generateText] and friends) reject one with a [StateError].
typedef RepairToolCall = FutureOr<RawToolCall?> Function(
    ToolCallRepairContext context);

// Tools

class FunctionToolInputExample {
  final Json input;
  const FunctionToolInputExample({required this.input});
  factory FunctionToolInputExample.fromJson(Json j) =>
      FunctionToolInputExample(input: j['input'] as Json);
  Json toJson() => {'input': input};
}

/// A function tool definition. Mirrors `FunctionTool.ts`.
class FunctionTool {
  final String name;
  final String? description;
  final Json inputSchema;
  final bool? strict;
  final Json? providerOptions;
  final List<FunctionToolInputExample>? inputExamples;

  const FunctionTool({
    required this.name,
    this.description,
    required this.inputSchema,
    this.strict,
    this.providerOptions,
    this.inputExamples,
  });

  factory FunctionTool.fromJson(Json j) => FunctionTool(
        name: j['name'] as String,
        description: j['description'] as String?,
        inputSchema: j['inputSchema'] as Json,
        strict: j['strict'] as bool?,
        providerOptions: _opt(j['providerOptions']),
        inputExamples:
            _optList(j['inputExamples'], FunctionToolInputExample.fromJson),
      );

  Json toJson() => {
        'name': name,
        if (description != null) 'description': description,
        'inputSchema': inputSchema,
        if (strict != null) 'strict': strict,
        if (providerOptions != null) 'providerOptions': providerOptions,
        if (inputExamples != null)
          'inputExamples': [for (final e in inputExamples!) e.toJson()],
      };
}

/// A tool definition: function or provider tool. Mirrors `Tool.ts`
/// (`{"type":"function", ...FunctionTool}` / `{"type":"provider", id, name,
/// args}`).
class Tool {
  final String type;
  final FunctionTool? function;
  final String? id;
  final String? name;
  final Json? args;

  /// Whether a result may arrive in a later response than its call
  /// (provider-utils@5.0.55 src/types/tool.ts:316-326).
  final bool? supportsDeferredResults;

  const Tool._(
      {required this.type,
      this.function,
      this.id,
      this.name,
      this.args,
      this.supportsDeferredResults});

  factory Tool.function(FunctionTool fn) => Tool._(type: 'function', function: fn);
  factory Tool.provider(
          {required String id,
          required String name,
          required Json args,
          bool? supportsDeferredResults}) =>
      Tool._(
          type: 'provider',
          id: id,
          name: name,
          args: args,
          supportsDeferredResults: supportsDeferredResults);

  Json toJson() => function != null
      ? {'type': 'function', ...function!.toJson()}
      : {
          'type': 'provider',
          'id': id!,
          'name': name!,
          'args': args ?? {},
          if (supportsDeferredResults != null)
            'supportsDeferredResults': supportsDeferredResults,
        };

  factory Tool.fromJson(Json j) => j['type'] == 'function'
      ? Tool.function(FunctionTool.fromJson(j))
      : Tool.provider(
          id: j['id'] as String,
          name: j['name'] as String,
          args: j['args'] as Json,
          supportsDeferredResults: j['supportsDeferredResults'] as bool?);
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

  dynamic toJson() =>
      _kind == 'tool' ? {'type': 'tool', 'toolName': toolName} : _kind;
  factory ToolChoice.fromJson(dynamic json) => json is String
      ? ToolChoice._(json, null)
      : ToolChoice._('tool', (json as Json)['toolName'] as String);
}

// Request / response metadata

/// Mirrors `RequestInfo.ts`.
class RequestInfo {
  final dynamic body;
  final bool? _hasBody;
  /// `hasBody: true` with a null `body` writes `"body": null`; the AI SDK's
  /// JSON keeps `null` apart from a missing value.
  const RequestInfo({this.body, bool? hasBody}) : _hasBody = hasBody;
  /// Whether `body` is present, even when it is JSON `null`.
  bool get hasBody => _hasBody ?? body != null;
  factory RequestInfo.fromJson(Json j) =>
      RequestInfo(body: j['body'], hasBody: j.containsKey('body'));
  Json toJson() => {if (hasBody) 'body': body};
}

/// Mirrors `ResponseInfo.ts`.
class ResponseInfo {
  final String? id;
  final String? timestamp;
  final String? modelId;
  final Map<String, String>? headers;
  final dynamic body;
  final bool? _hasBody;
  /// `hasBody: true` with a null `body` writes `"body": null`; the AI SDK's
  /// JSON keeps `null` apart from a missing value.
  const ResponseInfo(
      {this.id,
      this.timestamp,
      this.modelId,
      this.headers,
      this.body,
      bool? hasBody})
      : _hasBody = hasBody;
  /// Whether `body` is present, even when it is JSON `null`.
  bool get hasBody => _hasBody ?? body != null;
  factory ResponseInfo.fromJson(Json j) => ResponseInfo(
      id: j['id'] as String?,
      timestamp: j['timestamp'] as String?,
      modelId: j['modelId'] as String?,
      headers: (j['headers'] as Json?)?.cast<String, String>(),
      body: j['body'],
      hasBody: j.containsKey('body'));
  Json toJson() => {
        if (id != null) 'id': id,
        if (timestamp != null) 'timestamp': timestamp,
        if (modelId != null) 'modelId': modelId,
        if (headers != null) 'headers': headers,
        if (hasBody) 'body': body,
      };
}

/// Mirrors `ResponseMetadata.ts`.
class ResponseMetadata {
  final String? id;
  final String? timestamp;
  final String? modelId;
  const ResponseMetadata({this.id, this.timestamp, this.modelId});
  factory ResponseMetadata.fromJson(Json j) => ResponseMetadata(
      id: j['id'] as String?,
      timestamp: j['timestamp'] as String?,
      modelId: j['modelId'] as String?);
  Json toJson() => {
        if (id != null) 'id': id,
        if (timestamp != null) 'timestamp': timestamp,
        if (modelId != null) 'modelId': modelId,
      };
}

// File data, generated files, sources

/// Raw bytes or a base64 string — `FileBytes.ts` is `string`, the AI SDK's
/// `Uint8Array | string` in JSON (`shared-v4-file-data.ts:7-10`). Binary bytes
/// are written as base64; parsing yields [FileBytesBase64].
sealed class FileBytes {
  const FileBytes();
  factory FileBytes.fromJson(Object json) => json is String
      ? FileBytesBase64(data: json)
      : throw FormatException('FileBytes must be a base64 string, got: $json');
  Object toJson();
}

final class FileBytesBinary extends FileBytes {
  final List<int> data;
  const FileBytesBinary({required this.data});
  @override
  Object toJson() => encodeBase64Bytes(data);
}

final class FileBytesBase64 extends FileBytes {
  final String data;
  const FileBytesBase64({required this.data});
  @override
  Object toJson() => data;
}

/// File data of a file part. Mirrors `FileData.ts`:
/// `{"type":"data"|"url"|"reference"|"text", ...}`.
sealed class FileData {
  const FileData();
  factory FileData.fromJson(Json j) => switch (j['type']) {
        'data' => FileDataData(data: FileBytes.fromJson(j['data'] as Object)),
        'url' => FileDataUrl(
            url: j['url'] as String, originalUrl: j['originalUrl'] as String?),
        'reference' => FileDataReference(reference: j['reference'] as Json),
        'text' => FileDataText(text: j['text'] as String),
        _ => throw FormatException('unknown FileData type: ${j['type']}'),
      };
  Json toJson();
}

final class FileDataData extends FileData {
  final FileBytes data;
  const FileDataData({required this.data});
  @override
  Json toJson() => {'type': 'data', 'data': data.toJson()};
}

final class FileDataUrl extends FileData {
  final String url;
  final String? originalUrl;
  const FileDataUrl({required this.url, this.originalUrl});
  @override
  Json toJson() => {
        'type': 'url',
        'url': url,
        if (originalUrl != null) 'originalUrl': originalUrl,
      };
}

final class FileDataReference extends FileData {
  final Json reference;
  const FileDataReference({required this.reference});
  @override
  Json toJson() => {'type': 'reference', 'reference': reference};
}

final class FileDataText extends FileData {
  final String text;
  const FileDataText({required this.text});
  @override
  Json toJson() => {'type': 'text', 'text': text};
}

/// `FilePart.data` (Rust `FilePartData`, provider-utils
/// `types/content-part.ts:75`): `FileData | DataContent | URL |
/// ProviderReference`, in JSON a tagged [FileData], a string (base64 or a URL)
/// or a bare `{provider: id}` reference.
sealed class FilePartData {
  const FilePartData();
  factory FilePartData.fromJson(Object j) {
    if (j is String) return FilePartDataBare(j);
    if (j is Map) {
      final m = j.cast<String, dynamic>();
      if (const {'data', 'url', 'reference', 'text'}.contains(m['type'])) {
        return FilePartDataTagged(FileData.fromJson(m));
      }
      return FilePartDataReference(Map<String, String>.from(m));
    }
    throw FormatException(
        'file data must be a tagged object, a string or a provider reference: $j');
  }
  Object toJson();
}

/// A tagged [FileData].
final class FilePartDataTagged extends FilePartData {
  final FileData data;
  const FilePartDataTagged(this.data);
  @override
  Object toJson() => data.toJson();
}

/// A bare string: base64 data or a URL.
final class FilePartDataBare extends FilePartData {
  final String value;
  const FilePartDataBare(this.value);
  @override
  Object toJson() => value;
}

/// A bare provider reference, `{provider: id}`.
final class FilePartDataReference extends FilePartData {
  final Map<String, String> reference;
  const FilePartDataReference(this.reference);
  @override
  Object toJson() => reference;
}

/// `ReasoningFilePart.data` (Rust `ReasoningFilePartData`,
/// `types/content-part.ts:164`): tagged data or a URL ([GeneratedFileData]),
/// or a bare string (base64 or a URL).
sealed class ReasoningFilePartData {
  const ReasoningFilePartData();
  factory ReasoningFilePartData.fromJson(Object j) {
    if (j is String) return ReasoningFilePartDataBare(j);
    if (j is Map) {
      return ReasoningFilePartDataTagged(
          GeneratedFileData.fromJson(j.cast<String, dynamic>()));
    }
    throw FormatException(
        'reasoning file data must be tagged data or a URL, or a string: $j');
  }
  Object toJson();
}

/// Tagged data or a URL.
final class ReasoningFilePartDataTagged extends ReasoningFilePartData {
  final GeneratedFileData data;
  const ReasoningFilePartDataTagged(this.data);
  @override
  Object toJson() => data.toJson();
}

/// A bare string: base64 data or a URL.
final class ReasoningFilePartDataBare extends ReasoningFilePartData {
  final String value;
  const ReasoningFilePartDataBare(this.value);
  @override
  Object toJson() => value;
}

/// Data or a URL returned for a generated file. Mirrors `GeneratedFileData.ts`.
sealed class GeneratedFileData {
  const GeneratedFileData();
  factory GeneratedFileData.fromJson(Json j) => switch (j['type']) {
        'data' =>
          GeneratedFileDataData(data: FileBytes.fromJson(j['data'] as Object)),
        'url' => GeneratedFileDataUrl(
            url: j['url'] as String, originalUrl: j['originalUrl'] as String?),
        _ => throw FormatException('unknown GeneratedFileData type: ${j['type']}'),
      };
  Json toJson();
}

final class GeneratedFileDataData extends GeneratedFileData {
  final FileBytes data;
  const GeneratedFileDataData({required this.data});
  @override
  Json toJson() => {'type': 'data', 'data': data.toJson()};
}

final class GeneratedFileDataUrl extends GeneratedFileData {
  final String url;
  final String? originalUrl;
  const GeneratedFileDataUrl({required this.url, this.originalUrl});
  @override
  Json toJson() => {
        'type': 'url',
        'url': url,
        if (originalUrl != null) 'originalUrl': originalUrl,
      };
}

/// A file in a provider's output (Rust `LanguageModelFile`,
/// `LanguageModelV4File`): tagged base64 data or a URL. Mirrors
/// `LanguageModelFile.ts`; the payload of the raw [GenerateContent] file parts.
class LanguageModelFile {
  final GeneratedFileData data;
  final String mediaType;
  final Json? providerMetadata;
  const LanguageModelFile(
      {required this.data, required this.mediaType, this.providerMetadata});
  factory LanguageModelFile.fromJson(Json j) => LanguageModelFile(
      data: GeneratedFileData.fromJson(j['data'] as Json),
      mediaType: j['mediaType'] as String,
      providerMetadata: _opt(j['providerMetadata']));
  Json toJson() => {
        'data': data.toJson(),
        'mediaType': mediaType,
        if (providerMetadata != null) 'providerMetadata': providerMetadata,
      };
}

/// A generated file (the AI SDK's `GeneratedFile`,
/// `generate-text/generated-file.ts:10-32`). Mirrors `GeneratedFile.ts`:
/// `{"base64", "mediaType", "providerMetadata"?}`.
class GeneratedFile {
  /// The file as a base64 string.
  final String base64;
  final String mediaType;
  final Json? providerMetadata;
  const GeneratedFile(
      {required this.base64, required this.mediaType, this.providerMetadata});
  factory GeneratedFile.fromJson(Json j) => GeneratedFile(
      base64: j['base64'] as String,
      mediaType: j['mediaType'] as String,
      providerMetadata: _opt(j['providerMetadata']));

  /// The file's bytes.
  List<int> get bytes => decodeBase64Bytes(base64);
  Json toJson() => {
        'base64': base64,
        'mediaType': mediaType,
        if (providerMetadata != null) 'providerMetadata': providerMetadata,
      };
}

/// A source / citation. Mirrors `Source.ts`, tagged by `sourceType`
/// (`"url"` / `"document"`). Its JSON also carries `"type": "source"`, as the
/// AI SDK's `Source` (provider language-model-v4-source.ts) does in a
/// result's `sources`.
sealed class Source {
  const Source();

  factory Source.fromJson(Json j) =>
      switch (_checkTypeTag(j, 'source')['sourceType']) {
        'url' => UrlSource(
            id: j['id'] as String,
            url: j['url'] as String,
            title: j['title'] as String?,
            providerMetadata: _opt(j['providerMetadata'])),
        'document' => DocumentSource(
            id: j['id'] as String,
            mediaType: j['mediaType'] as String,
            title: j['title'] as String,
            filename: j['filename'] as String?,
            providerMetadata: _opt(j['providerMetadata'])),
        _ => throw FormatException('invalid source type: ${j['sourceType']}'),
      };

  Json toJson();
}

final class UrlSource extends Source {
  final String id;
  final String url;
  final String? title;
  final Json? providerMetadata;

  const UrlSource(
      {required this.id, required this.url, this.title, this.providerMetadata});

  @override
  Json toJson() => {
        'type': 'source',
        'sourceType': 'url',
        'id': id,
        'url': url,
        if (title != null) 'title': title,
        if (providerMetadata != null) 'providerMetadata': providerMetadata,
      };
}

final class DocumentSource extends Source {
  final String id;
  final String mediaType;
  final String title;
  final String? filename;
  final Json? providerMetadata;

  const DocumentSource(
      {required this.id,
      required this.mediaType,
      required this.title,
      this.filename,
      this.providerMetadata});

  @override
  Json toJson() => {
        'type': 'source',
        'sourceType': 'document',
        'id': id,
        'mediaType': mediaType,
        'title': title,
        if (filename != null) 'filename': filename,
        if (providerMetadata != null) 'providerMetadata': providerMetadata,
      };
}

// ContentPart (message content) — `{"type": "text", ...}`

/// The output of a tool call in a `tool-result` part. Mirrors
/// `ToolResultOutput.ts`: `{"type":"text"|"error-text","value":"…"}`,
/// `{"type":"json"|"error-json","value":<json>}`,
/// `{"type":"execution-denied","reason"?}` or `{"type":"content","value":[…]}`
/// (`value` is then a list of text / file / custom content maps, kept as
/// weak-typed maps).
class ToolResultOutput {
  final String type;
  final dynamic value;
  final String? reason;
  final Json? providerOptions;

  const ToolResultOutput._(this.type, {this.value, this.reason, this.providerOptions});

  factory ToolResultOutput.text(String value, {Json? providerOptions}) =>
      ToolResultOutput._('text', value: value, providerOptions: providerOptions);
  factory ToolResultOutput.json(Object? value, {Json? providerOptions}) =>
      ToolResultOutput._('json', value: value, providerOptions: providerOptions);
  factory ToolResultOutput.errorText(String value, {Json? providerOptions}) =>
      ToolResultOutput._('error-text', value: value, providerOptions: providerOptions);
  factory ToolResultOutput.errorJson(Object? value, {Json? providerOptions}) =>
      ToolResultOutput._('error-json', value: value, providerOptions: providerOptions);
  factory ToolResultOutput.executionDenied({String? reason, Json? providerOptions}) =>
      ToolResultOutput._('execution-denied',
          reason: reason, providerOptions: providerOptions);
  factory ToolResultOutput.content(List<Json> value) =>
      ToolResultOutput._('content', value: value);

  factory ToolResultOutput.fromJson(Json j) => ToolResultOutput._(
        j['type'] as String,
        value: j['value'],
        reason: j['reason'] as String?,
        providerOptions: _opt(j['providerOptions']),
      );

  Json toJson() => {
        'type': type,
        if (type != 'execution-denied') 'value': value,
        if (reason != null) 'reason': reason,
        if (providerOptions != null) 'providerOptions': providerOptions,
      };
}

/// One part of a message's content. Mirrors `ContentPart.ts`; unknown `type`
/// values fall back to [ContentPartUnknown] and pass through verbatim.
sealed class ContentPart {
  const ContentPart();
  factory ContentPart.fromJson(Json j) => switch (j['type']) {
        'text' => ContentPartText(
            text: j['text'] as String, providerOptions: _opt(j['providerOptions'])),
        'custom' => ContentPartCustom(
            kind: j['kind'] as String, providerOptions: _opt(j['providerOptions'])),
        'reasoning-file' => ContentPartReasoningFile(
            data: ReasoningFilePartData.fromJson(j['data'] as Object),
            mediaType: j['mediaType'] as String,
            providerOptions: _opt(j['providerOptions'])),
        'tool-approval-request' => ContentPartToolApprovalRequest(
            approvalId: j['approvalId'] as String,
            toolCallId: j['toolCallId'] as String,
            reason: j['reason'] as String?,
            isAutomatic: j['isAutomatic'] as bool?,
            signature: j['signature'] as String?,
            inputSchemaInput: j['inputSchemaInput'],
            hasInputSchemaInput: j.containsKey('inputSchemaInput')),
        'image' => ContentPartImage(
            image: decodeBase64Bytes(j['image']),
            mediaType: j['mediaType'] as String,
            providerOptions: _opt(j['providerOptions'])),
        'file' => ContentPartFile(
            data: FilePartData.fromJson(j['data'] as Object),
            mediaType: j['mediaType'] as String,
            filename: j['filename'] as String?,
            providerOptions: _opt(j['providerOptions'])),
        'reasoning' => ContentPartReasoning(
            text: j['text'] as String,
            signature: j['signature'] as String?,
            providerOptions: _opt(j['providerOptions'])),
        'tool-call' => ContentPartToolCall(
            toolCallId: j['toolCallId'] as String,
            toolName: j['toolName'] as String,
            input: j['input'],
            providerExecuted: j['providerExecuted'] as bool?,
            providerOptions: _opt(j['providerOptions'])),
        'tool-result' => ContentPartToolResult(
            toolCallId: j['toolCallId'] as String,
            toolName: j['toolName'] as String,
            output: ToolResultOutput.fromJson(j['output'] as Json),
            providerOptions: _opt(j['providerOptions'])),
        _ => ContentPartUnknown(j),
      };
  Json toJson();
}

final class ContentPartText extends ContentPart {
  final String text;
  final Json? providerOptions;
  const ContentPartText({required this.text, this.providerOptions});
  @override
  Json toJson() => {
        'type': 'text',
        'text': text,
        if (providerOptions != null) 'providerOptions': providerOptions,
      };
}

final class ContentPartCustom extends ContentPart {
  final String kind;
  final Json? providerOptions;
  const ContentPartCustom({required this.kind, this.providerOptions});
  @override
  Json toJson() => {
        'type': 'custom',
        'kind': kind,
        if (providerOptions != null) 'providerOptions': providerOptions,
      };
}

/// The AI SDK `ReasoningFilePart` (`types/content-part.ts:146-180`).
final class ContentPartReasoningFile extends ContentPart {
  final ReasoningFilePartData data;
  final String mediaType;
  final Json? providerOptions;
  const ContentPartReasoningFile(
      {required this.data, required this.mediaType, this.providerOptions});
  @override
  Json toJson() => {
        'type': 'reasoning-file',
        'data': data.toJson(),
        'mediaType': mediaType,
        if (providerOptions != null) 'providerOptions': providerOptions,
      };
}

final class ContentPartToolApprovalRequest extends ContentPart {
  final String approvalId;
  final String toolCallId;
  final String? reason;
  final bool? isAutomatic;
  final String? signature;
  final dynamic inputSchemaInput;
  final bool? _hasInputSchemaInput;
  /// `hasInputSchemaInput: true` with a null `inputSchemaInput` writes
  /// `"inputSchemaInput": null`; the AI SDK's JSON keeps `null` apart from a
  /// missing value.
  const ContentPartToolApprovalRequest(
      {required this.approvalId,
      required this.toolCallId,
      this.reason,
      this.isAutomatic,
      this.signature,
      this.inputSchemaInput,
      bool? hasInputSchemaInput})
      : _hasInputSchemaInput = hasInputSchemaInput;
  /// Whether `inputSchemaInput` is present, even when it is JSON `null`.
  bool get hasInputSchemaInput =>
      _hasInputSchemaInput ?? inputSchemaInput != null;
  @override
  Json toJson() => {
        'type': 'tool-approval-request',
        'approvalId': approvalId,
        'toolCallId': toolCallId,
        if (reason != null) 'reason': reason,
        if (isAutomatic != null) 'isAutomatic': isAutomatic,
        if (signature != null) 'signature': signature,
        if (hasInputSchemaInput) 'inputSchemaInput': inputSchemaInput,
      };
}

final class ContentPartImage extends ContentPart {
  final List<int> image;
  final String mediaType;
  final Json? providerOptions;
  const ContentPartImage(
      {required this.image, required this.mediaType, this.providerOptions});
  @override
  Json toJson() => {
        'type': 'image',
        'image': encodeBase64Bytes(image),
        'mediaType': mediaType,
        if (providerOptions != null) 'providerOptions': providerOptions,
      };
}

/// The AI SDK `FilePart` (provider-utils `types/content-part.ts:62-102`):
/// `data` is a [FilePartData]: tagged [FileData], a bare string (base64 or a
/// URL) or a bare provider reference.
final class ContentPartFile extends ContentPart {
  final FilePartData data;
  final String mediaType;
  final String? filename;
  final Json? providerOptions;
  const ContentPartFile(
      {required this.data,
      required this.mediaType,
      this.filename,
      this.providerOptions});

  /// Inline bytes, written as `{"type":"data","data":"<base64>"}`.
  ContentPartFile.bytes(List<int> bytes,
      {required String mediaType, String? filename, Json? providerOptions})
      : this(
            data: FilePartDataTagged(
                FileDataData(data: FileBytesBinary(data: bytes))),
            mediaType: mediaType,
            filename: filename,
            providerOptions: providerOptions);
  @override
  Json toJson() => {
        'type': 'file',
        'data': data.toJson(),
        'mediaType': mediaType,
        if (filename != null) 'filename': filename,
        if (providerOptions != null) 'providerOptions': providerOptions,
      };
}

final class ContentPartReasoning extends ContentPart {
  final String text;
  final String? signature;
  final Json? providerOptions;
  const ContentPartReasoning(
      {required this.text, this.signature, this.providerOptions});
  @override
  Json toJson() => {
        'type': 'reasoning',
        'text': text,
        if (signature != null) 'signature': signature,
        if (providerOptions != null) 'providerOptions': providerOptions,
      };
}

final class ContentPartToolCall extends ContentPart {
  final String toolCallId;
  final String toolName;
  final dynamic input;
  final bool? providerExecuted;
  final Json? providerOptions;
  const ContentPartToolCall(
      {required this.toolCallId,
      required this.toolName,
      required this.input,
      this.providerExecuted,
      this.providerOptions});
  @override
  Json toJson() => {
        'type': 'tool-call',
        'toolCallId': toolCallId,
        'toolName': toolName,
        'input': input,
        if (providerExecuted != null) 'providerExecuted': providerExecuted,
        if (providerOptions != null) 'providerOptions': providerOptions,
      };
}

/// The call-layer tool result a host appends to the transcript: the call's
/// [toolCallId] and [toolName] plus a typed [output].
final class ContentPartToolResult extends ContentPart {
  final String toolCallId;
  final String toolName;
  final ToolResultOutput output;
  final Json? providerOptions;
  const ContentPartToolResult(
      {required this.toolCallId,
      required this.toolName,
      required this.output,
      this.providerOptions});
  @override
  Json toJson() => {
        'type': 'tool-result',
        'toolCallId': toolCallId,
        'toolName': toolName,
        'output': output.toJson(),
        if (providerOptions != null) 'providerOptions': providerOptions,
      };
}

/// Forward-compatibility: a `type` this binding does not model, verbatim.
final class ContentPartUnknown extends ContentPart {
  final Json data;
  const ContentPartUnknown(this.data);
  String get tag => data['type'] as String;
  @override
  Json toJson() => data;
}

// Messages

/// A single chat message. Mirrors `ModelMessage.ts`.
///
/// `content` is either a plain `String` or a `List` of content-part maps
/// (see [ContentPart]); it is kept as `Object` so both shapes pass through
/// verbatim.
class ModelMessage {
  final String role;
  final Object content;

  const ModelMessage({required this.role, required this.content});

  factory ModelMessage.fromJson(Json j) =>
      ModelMessage(role: j['role'] as String, content: j['content'] as Object);
  Json toJson() => {'role': role, 'content': content};

  /// [content] as a list of content-part maps: a `String` becomes a single
  /// `{'type': 'text', 'text': content}` part.
  List<Json> get contentParts {
    final c = content;
    if (c is String) {
      return [
        {'type': 'text', 'text': c},
      ];
    }
    if (c is List) return c.whereType<Json>().toList();
    return const [];
  }
}

// GenerateResult (provider result) and its content

/// A content item in a [GenerateResult]. Mirrors `GenerateContent.ts` with the
/// provider-level parameters (`RawToolCall`, `RawToolApprovalRequest`,
/// `LanguageModelFile`): `{"type": "text" | "tool-call" | "source" | "reasoning" |
/// "file" | "reasoning-file" | "custom" | "tool-approval-request" |
/// "tool-result", ...}`. Unknown types fall back to [GenerateContentUnknown].
sealed class GenerateContent {
  const GenerateContent();

  /// The wire `type` value, e.g. `'text'`, `'tool-call'`.
  String get tag => toJson()['type'] as String;

  Json toJson();

  factory GenerateContent.fromJson(Json j) => switch (j['type']) {
        'text' => GenerateContentText(
            text: j['text'] as String,
            providerMetadata: _opt(j['providerMetadata'])),
        'tool-call' => GenerateContentToolCall(RawToolCall.fromJson(j)),
        'source' => GenerateContentSource(source: Source.fromJson(j)),
        'reasoning' => GenerateContentReasoning(
            text: j['text'] as String,
            providerMetadata: _opt(j['providerMetadata'])),
        'file' => GenerateContentFile(LanguageModelFile.fromJson(j)),
        'reasoning-file' =>
          GenerateContentReasoningFile(LanguageModelFile.fromJson(j)),
        'custom' => GenerateContentCustom(
            kind: j['kind'] as String,
            providerMetadata: _opt(j['providerMetadata'])),
        'tool-approval-request' => GenerateContentToolApprovalRequest(
            approvalId: j['approvalId'] as String,
            toolCallId: j['toolCallId'] as String,
            providerMetadata: _opt(j['providerMetadata'])),
        'tool-result' => GenerateContentToolResult(ToolResult.fromJson(j)),
        _ => GenerateContentUnknown(j),
      };
}

final class GenerateContentText extends GenerateContent {
  final String text;
  final Json? providerMetadata;
  const GenerateContentText({required this.text, this.providerMetadata});
  @override
  Json toJson() => {
        'type': 'text',
        'text': text,
        if (providerMetadata != null) 'providerMetadata': providerMetadata,
      };
}

/// A tool call as the provider emitted it (raw argument text).
final class GenerateContentToolCall extends GenerateContent {
  final RawToolCall call;
  const GenerateContentToolCall(this.call);
  @override
  Json toJson() => {'type': 'tool-call', ...call.toJson()};
}

final class GenerateContentSource extends GenerateContent {
  final Source source;
  const GenerateContentSource({required this.source});
  @override
  Json toJson() => {'type': 'source', ...source.toJson()};
}

final class GenerateContentReasoning extends GenerateContent {
  final String text;
  final Json? providerMetadata;
  const GenerateContentReasoning({required this.text, this.providerMetadata});
  @override
  Json toJson() => {
        'type': 'reasoning',
        'text': text,
        if (providerMetadata != null) 'providerMetadata': providerMetadata,
      };
}

final class GenerateContentFile extends GenerateContent {
  final LanguageModelFile file;
  const GenerateContentFile(this.file);
  @override
  Json toJson() => {'type': 'file', ...file.toJson()};
}

final class GenerateContentReasoningFile extends GenerateContent {
  final LanguageModelFile file;
  const GenerateContentReasoningFile(this.file);
  @override
  Json toJson() => {'type': 'reasoning-file', ...file.toJson()};
}

final class GenerateContentCustom extends GenerateContent {
  final String kind;
  final Json? providerMetadata;
  const GenerateContentCustom({required this.kind, this.providerMetadata});
  @override
  Json toJson() => {
        'type': 'custom',
        'kind': kind,
        if (providerMetadata != null) 'providerMetadata': providerMetadata,
      };
}

final class GenerateContentToolApprovalRequest extends GenerateContent {
  final String approvalId;
  final String toolCallId;
  final Json? providerMetadata;
  const GenerateContentToolApprovalRequest(
      {required this.approvalId, required this.toolCallId, this.providerMetadata});
  @override
  Json toJson() => {
        'type': 'tool-approval-request',
        'approvalId': approvalId,
        'toolCallId': toolCallId,
        if (providerMetadata != null) 'providerMetadata': providerMetadata,
      };
}

/// The result of a provider-executed tool. Mirrors `ToolResult.ts`: [result] is
/// the tool's output, [isError] / [preliminary] / [isDynamic] (wire key
/// `dynamic`) its flags.
class ToolResult {
  final String toolCallId;
  final String toolName;
  final dynamic result;
  final bool? isError;
  final bool? preliminary;
  final bool? isDynamic;
  final Json? providerMetadata;
  const ToolResult({
    required this.toolCallId,
    required this.toolName,
    required this.result,
    this.isError,
    this.preliminary,
    this.isDynamic,
    this.providerMetadata,
  });
  factory ToolResult.fromJson(Json j) => ToolResult(
        toolCallId: j['toolCallId'] as String,
        toolName: j['toolName'] as String,
        result: j['result'],
        isError: j['isError'] as bool?,
        preliminary: j['preliminary'] as bool?,
        isDynamic: j['dynamic'] as bool?,
        providerMetadata: _opt(j['providerMetadata']),
      );
  Json toJson() => {
        'toolCallId': toolCallId,
        'toolName': toolName,
        'result': result,
        if (isError != null) 'isError': isError,
        if (preliminary != null) 'preliminary': preliminary,
        if (isDynamic != null) 'dynamic': isDynamic,
        if (providerMetadata != null) 'providerMetadata': providerMetadata,
      };
}

final class GenerateContentToolResult extends GenerateContent {
  final ToolResult result;
  const GenerateContentToolResult(this.result);
  @override
  Json toJson() => {'type': 'tool-result', ...result.toJson()};
}

/// Forward-compatibility: a `type` this binding does not model, verbatim.
final class GenerateContentUnknown extends GenerateContent {
  final Json data;
  const GenerateContentUnknown(this.data);
  @override
  Json toJson() => data;
}

/// The raw provider result. Mirrors `GenerateResult.ts`.
class GenerateResult {
  final List<GenerateContent> content;
  final FinishReason finishReason;
  final Usage usage;
  final List<Json> warnings;
  final Json? providerMetadata;
  final RequestInfo? request;
  final ResponseInfo? response;
  const GenerateResult({
    required this.content,
    required this.finishReason,
    required this.usage,
    required this.warnings,
    this.providerMetadata,
    this.request,
    this.response,
  });
  factory GenerateResult.fromJson(Json j) => GenerateResult(
        content: _list(j['content'], GenerateContent.fromJson),
        finishReason: FinishReason.fromJson(j['finishReason'] as Json),
        usage: Usage.fromJson(j['usage'] as Json),
        warnings: _maps(j['warnings']),
        providerMetadata: _opt(j['providerMetadata']),
        request: j['request'] == null ? null : RequestInfo.fromJson(j['request'] as Json),
        response:
            j['response'] == null ? null : ResponseInfo.fromJson(j['response'] as Json),
      );
  Json toJson() => {
        'content': [for (final c in content) c.toJson()],
        'finishReason': finishReason.toJson(),
        'usage': usage.toJson(),
        'warnings': warnings,
        if (providerMetadata != null) 'providerMetadata': providerMetadata,
        if (request != null) 'request': request!.toJson(),
        if (response != null) 'response': response!.toJson(),
      };
}

// Results / options

/// Result of `generate_text` and of a consumed stream (user-facing), the AI
/// SDK's `GenerateTextResult`. Mirrors `GenerateTextResult.ts`.
///
/// [steps] holds every step (weak type); the other fields are derived from
/// them: the last step's text, finish reason, reasoning, request, response
/// and provider metadata, every step's content, files, sources, tool calls,
/// tool results and warnings, and the usage summed over the steps.
/// [reasoning], [reasoningText] (`null` when there is none), [totalUsage]
/// (equal to [usage]), [request], [response] and [providerMetadata] are
/// deprecated upstream. The typed-union fields (`content`, `reasoning`,
/// `files`, `toolResults`, `warnings`, `steps`) stay weak-typed maps.
class GenerateTextResult {
  final List<Json> content;
  final String text;
  final List<Json> reasoning;
  final String? reasoningText;
  final List<Json> files;
  final List<Source> sources;
  final List<ToolCall> toolCalls;
  final List<Json> toolResults;

  /// The unified finish reason of the last step (`"stop"`, `"length"`,
  /// `"content-filter"`, `"tool-calls"`, `"error"`, `"other"`).
  final String finishReason;

  /// Raw provider-specific finish reason string (e.g. "end_turn").
  final String? rawFinishReason;
  final LanguageModelUsage usage;
  final LanguageModelUsage totalUsage;
  final List<Json> warnings;
  final RequestInfo request;

  /// The last step's response id, timestamp and model id.
  final ResponseMetadata response;

  /// Assistant messages ready to append to the prompt for the next turn.
  final List<ModelMessage> responseMessages;
  final Json? providerMetadata;

  /// Every step of the call. Weak type.
  final List<Json> steps;

  /// The value parsed by the `output` option, `null` when it was not parsed
  /// or when the parsed value is JSON `null` ([hasOutput] tells them apart).
  final Object? output;
  final bool? _hasOutput;

  /// Whether the output was parsed (`output` is present in the JSON, even
  /// as `null`); `hasOutput: true` with a null [output] writes
  /// `"output": null`.
  bool get hasOutput => _hasOutput ?? output != null;

  const GenerateTextResult({
    this.content = const [],
    required this.text,
    this.reasoning = const [],
    this.reasoningText,
    this.files = const [],
    this.sources = const [],
    this.toolCalls = const [],
    this.toolResults = const [],
    required this.finishReason,
    this.rawFinishReason,
    required this.usage,
    required this.totalUsage,
    this.warnings = const [],
    this.request = const RequestInfo(),
    required this.response,
    this.responseMessages = const [],
    this.providerMetadata,
    this.steps = const [],
    this.output,
    bool? hasOutput,
  }) : _hasOutput = hasOutput;

  factory GenerateTextResult.fromJson(Json j) => GenerateTextResult(
        content: _maps(j['content']),
        text: j['text'] as String,
        reasoning: _maps(j['reasoning']),
        reasoningText: j['reasoningText'] as String?,
        files: _maps(j['files']),
        sources: _list(j['sources'], Source.fromJson),
        toolCalls: _list(j['toolCalls'], ToolCall.fromJson),
        toolResults: _maps(j['toolResults']),
        finishReason: j['finishReason'] as String,
        rawFinishReason: j['rawFinishReason'] as String?,
        usage: LanguageModelUsage.fromJson(j['usage'] as Json),
        totalUsage: LanguageModelUsage.fromJson(j['totalUsage'] as Json),
        warnings: _maps(j['warnings']),
        request: RequestInfo.fromJson(j['request'] as Json),
        response: ResponseMetadata.fromJson(j['response'] as Json),
        responseMessages: _list(j['responseMessages'], ModelMessage.fromJson),
        providerMetadata: _opt(j['providerMetadata']),
        steps: _maps(j['steps']),
        output: j['output'],
        hasOutput: j.containsKey('output'),
      );

  Json toJson() => {
        'content': content,
        'text': text,
        'reasoning': reasoning,
        if (reasoningText != null) 'reasoningText': reasoningText,
        'files': files,
        'sources': [for (final s in sources) s.toJson()],
        'toolCalls': [for (final c in toolCalls) c.toJson()],
        'toolResults': toolResults,
        'finishReason': finishReason,
        if (rawFinishReason != null) 'rawFinishReason': rawFinishReason,
        'usage': usage.toJson(),
        'totalUsage': totalUsage.toJson(),
        'warnings': warnings,
        'request': request.toJson(),
        'response': response.toJson(),
        'responseMessages': [for (final m in responseMessages) m.toJson()],
        if (providerMetadata != null) 'providerMetadata': providerMetadata,
        'steps': steps,
        if (hasOutput) 'output': output,
      };
}

/// Result of `generate_object`, the AI SDK's `GenerateObjectResult`: the
/// parsed JSON object and the call's reasoning, finish reason, usage,
/// warnings, request, response and provider metadata. The AI SDK deprecates
/// `generateObject` for `generateText` with an `output`. Mirrors
/// `GenerateObjectResult.ts`; [object] is an arbitrary JSON value.
class GenerateObjectResult {
  final Object? object;

  /// Concatenated reasoning text, if the model produced any.
  final String? reasoning;

  /// The unified finish reason (`"stop"`, `"length"`, ...).
  final String finishReason;
  final LanguageModelUsage usage;
  final List<Json> warnings;
  final RequestInfo request;

  /// Response metadata (id, timestamp, modelId).
  final ResponseMetadata response;
  final Json? providerMetadata;

  const GenerateObjectResult({
    this.object,
    this.reasoning,
    required this.finishReason,
    required this.usage,
    this.warnings = const [],
    this.request = const RequestInfo(),
    required this.response,
    this.providerMetadata,
  });

  factory GenerateObjectResult.fromJson(Json j) => GenerateObjectResult(
        object: j['object'],
        reasoning: j['reasoning'] as String?,
        finishReason: j['finishReason'] as String,
        usage: LanguageModelUsage.fromJson(j['usage'] as Json),
        warnings: _maps(j['warnings']),
        request: RequestInfo.fromJson(j['request'] as Json),
        response: ResponseMetadata.fromJson(j['response'] as Json),
        providerMetadata: _opt(j['providerMetadata']),
      );

  Json toJson() => {
        'object': object,
        if (reasoning != null) 'reasoning': reasoning,
        'finishReason': finishReason,
        'usage': usage.toJson(),
        'warnings': warnings,
        'request': request.toJson(),
        'response': response.toJson(),
        if (providerMetadata != null) 'providerMetadata': providerMetadata,
      };
}

/// Per-call timeout configuration, in milliseconds; null disables a limit.
/// Mirrors `TimeoutConfiguration.ts`.
class TimeoutConfiguration {
  final int? totalMs;
  final int? stepMs;
  final int? firstChunkMs;
  final int? chunkMs;

  const TimeoutConfiguration({this.totalMs, this.stepMs, this.firstChunkMs, this.chunkMs});

  factory TimeoutConfiguration.fromJson(Json j) => TimeoutConfiguration(
        totalMs: j['totalMs'] as int?,
        stepMs: j['stepMs'] as int?,
        firstChunkMs: j['firstChunkMs'] as int?,
        chunkMs: j['chunkMs'] as int?,
      );
  Json toJson() => {
        if (totalMs != null) 'totalMs': totalMs,
        if (stepMs != null) 'stepMs': stepMs,
        if (firstChunkMs != null) 'firstChunkMs': firstChunkMs,
        if (chunkMs != null) 'chunkMs': chunkMs,
      };
}

/// User-facing options for `generate_text` / `stream_text`. Mirrors
/// `GenerateTextOptions.ts`; unset fields are omitted from the JSON.
class GenerateTextOptions {
  final int? maxOutputTokens;
  final double? temperature;
  final List<String>? stopSequences;
  final double? topP;
  final double? topK;
  final double? presencePenalty;
  final double? frequencyPenalty;

  /// Structured output, the AI SDK's `output`, tagged by `type`: for example
  /// `{'type': 'object', 'schema': {...}}`,
  /// `{'type': 'array', 'element': {...}, 'minItems': 1}`,
  /// `{'type': 'choice', 'options': [...]}` or `{'type': 'json'}`. The parsed
  /// value is [GenerateTextResult.output].
  final Json? output;
  final int? seed;
  final List<Tool>? tools;
  final ToolChoice? toolChoice;
  final Map<String, String>? headers;
  final Json? providerOptions;
  final ReasoningEffort? reasoning;
  final String? instructions;
  final int? maxRetries;
  final TimeoutConfiguration? timeout;
  final bool? includeRawChunks;
  final String? sessionId;

  /// Repair a tool call the model got wrong (RFC-0035), mirroring the AI SDK's
  /// `repairToolCall`.
  ///
  /// Runs host-side, after generation: every call in the result with
  /// `invalid == true` is offered to the hook once, in order, and the reply is
  /// re-validated by the core. A call made without [tools] is never offered —
  /// the hook is not invoked for it.
  ///
  /// Honoured by [TypedModel]'s `generateText` / `generateObject` /
  /// `consumeStreamText` / `streamText`, and by `generateTextAsOpenAI`, which
  /// repairs the native result before converting it. `streamTextAsOpenAI`
  /// does not reflect repair: the chunk stream forwards the provider's
  /// argument deltas verbatim, as in the AI SDK.
  ///
  /// Never serialized — it is a host closure, not part of the wire options.
  final RepairToolCall? repairToolCall;

  const GenerateTextOptions({
    this.maxOutputTokens,
    this.temperature,
    this.stopSequences,
    this.topP,
    this.topK,
    this.presencePenalty,
    this.frequencyPenalty,
    this.output,
    this.seed,
    this.tools,
    this.toolChoice,
    this.headers,
    this.providerOptions,
    this.reasoning,
    this.instructions,
    this.maxRetries,
    this.timeout,
    this.includeRawChunks,
    this.sessionId,
    this.repairToolCall,
  });

  factory GenerateTextOptions.fromJson(Json j) => GenerateTextOptions(
        maxOutputTokens: j['maxOutputTokens'] as int?,
        temperature: (j['temperature'] as num?)?.toDouble(),
        stopSequences: (j['stopSequences'] as List?)?.cast<String>(),
        topP: (j['topP'] as num?)?.toDouble(),
        topK: (j['topK'] as num?)?.toDouble(),
        presencePenalty: (j['presencePenalty'] as num?)?.toDouble(),
        frequencyPenalty: (j['frequencyPenalty'] as num?)?.toDouble(),
        output: _opt(j['output']),
        seed: j['seed'] as int?,
        tools: _optList(j['tools'], Tool.fromJson),
        toolChoice:
            j['toolChoice'] == null ? null : ToolChoice.fromJson(j['toolChoice']),
        headers: (j['headers'] as Json?)?.cast<String, String>(),
        providerOptions: _opt(j['providerOptions']),
        reasoning: j['reasoning'] == null
            ? null
            : ReasoningEffort.fromJson(j['reasoning'] as String),
        instructions: j['instructions'] as String?,
        maxRetries: j['maxRetries'] as int?,
        timeout: j['timeout'] == null
            ? null
            : TimeoutConfiguration.fromJson(j['timeout'] as Json),
        includeRawChunks: j['includeRawChunks'] as bool?,
        sessionId: j['sessionId'] as String?,
      );

  Json toJson() => {
        if (maxOutputTokens != null) 'maxOutputTokens': maxOutputTokens,
        if (temperature != null) 'temperature': temperature,
        if (stopSequences != null) 'stopSequences': stopSequences,
        if (topP != null) 'topP': topP,
        if (topK != null) 'topK': topK,
        if (presencePenalty != null) 'presencePenalty': presencePenalty,
        if (frequencyPenalty != null) 'frequencyPenalty': frequencyPenalty,
        if (output != null) 'output': output,
        if (seed != null) 'seed': seed,
        if (tools != null) 'tools': [for (final t in tools!) t.toJson()],
        if (toolChoice != null) 'toolChoice': toolChoice!.toJson(),
        if (headers != null) 'headers': headers,
        if (providerOptions != null) 'providerOptions': providerOptions,
        if (reasoning != null) 'reasoning': reasoning!.toJson(),
        if (instructions != null) 'instructions': instructions,
        if (maxRetries != null) 'maxRetries': maxRetries,
        if (timeout != null) 'timeout': timeout!.toJson(),
        if (includeRawChunks != null) 'includeRawChunks': includeRawChunks,
        if (sessionId != null) 'sessionId': sessionId,
      };
}

// Streaming

/// A single part of the stream returned by `stream_text`. Mirrors
/// `TextStreamPart.ts`, the AI SDK's full stream: `start`, then per step
/// `start-step` … `finish-step`, then `finish` (or `abort`).
/// `{"type": "text-delta", "id": ..., "text": ...}` — the tag values are
/// `text-start`, `text-delta`, `text-end`, `start`, `start-step`,
/// `finish-step`, `finish`, `abort`, `error`, `tool-input-start`,
/// `tool-input-delta`, `tool-input-end`, `tool-call`, `tool-result`,
/// `tool-error`, `tool-output-denied`, `file`, `reasoning-file`, `custom`,
/// `tool-approval-request`, `tool-approval-response`, `reasoning-start`,
/// `reasoning-delta`, `reasoning-end`, `source` and `raw`. A `type` this
/// binding does not model becomes [StreamPartUnknown] and passes through
/// verbatim. Narrow via `switch` / `is` / `whereType`.
sealed class StreamPart {
  const StreamPart();

  /// The wire `type` value, e.g. `'text-delta'`.
  String get type => toJson()['type'] as String;

  Json toJson();

  factory StreamPart.fromJson(Json j) {
    String id() => j['id'] as String;
    Json? meta() => _opt(j['providerMetadata']);
    return switch (j['type']) {
      'text-start' => StreamPartTextStart(id: id(), providerMetadata: meta()),
      'text-delta' => StreamPartTextDelta(
          id: id(), text: j['text'] as String, providerMetadata: meta()),
      'text-end' => StreamPartTextEnd(id: id(), providerMetadata: meta()),
      'start' => const StreamPartStart(),
      'start-step' => StreamPartStartStep(
          request: RequestInfo.fromJson(j['request'] as Json),
          warnings: _maps(j['warnings'])),
      'finish-step' => StreamPartFinishStep(
          response: ResponseMetadata.fromJson(j['response'] as Json),
          usage: LanguageModelUsage.fromJson(j['usage'] as Json),
          performance: _opt(j['performance']),
          finishReason: j['finishReason'] as String,
          rawFinishReason: j['rawFinishReason'] as String?,
          providerMetadata: meta()),
      'finish' => StreamPartFinish(
          finishReason: j['finishReason'] as String,
          rawFinishReason: j['rawFinishReason'] as String?,
          totalUsage: LanguageModelUsage.fromJson(j['totalUsage'] as Json)),
      'abort' => StreamPartAbort(reason: j['reason'] as String?),
      'error' => StreamPartError(error: j['error'] as Json),
      'tool-input-start' => StreamPartToolInputStart(
          id: id(),
          toolName: j['toolName'] as String,
          providerExecuted: j['providerExecuted'] as bool?,
          isDynamic: j['dynamic'] as bool?,
          title: j['title'] as String?,
          providerMetadata: meta()),
      'tool-input-delta' => StreamPartToolInputDelta(
          id: id(), delta: j['delta'] as String, providerMetadata: meta()),
      'tool-input-end' => StreamPartToolInputEnd(id: id(), providerMetadata: meta()),
      'tool-call' => StreamPartToolCall(ToolCall.fromJson(j)),
      'tool-result' => StreamPartToolResult.fromJson(j),
      'tool-error' => StreamPartToolError.fromJson(j),
      'tool-output-denied' => StreamPartToolOutputDenied.fromJson(j),
      'file' => StreamPartFile(
          file: GeneratedFile.fromJson(j['file'] as Json), providerMetadata: meta()),
      'reasoning-file' => StreamPartReasoningFile(
          file: GeneratedFile.fromJson(j['file'] as Json), providerMetadata: meta()),
      'custom' => StreamPartCustom(kind: j['kind'] as String, providerMetadata: meta()),
      'tool-approval-request' => StreamPartToolApprovalRequest(
          approvalId: j['approvalId'] as String,
          toolCall: ToolCall.fromJson(j['toolCall'] as Json),
          reason: j['reason'] as String?,
          isAutomatic: j['isAutomatic'] as bool?,
          signature: j['signature'] as String?),
      'tool-approval-response' => StreamPartToolApprovalResponse.fromJson(j),
      'reasoning-start' => StreamPartReasoningStart(id: id(), providerMetadata: meta()),
      'reasoning-delta' => StreamPartReasoningDelta(
          id: id(), text: j['text'] as String, providerMetadata: meta()),
      'reasoning-end' => StreamPartReasoningEnd(id: id(), providerMetadata: meta()),
      'source' => StreamPartSource(source: Source.fromJson(j)),
      'raw' => StreamPartRaw(rawValue: j['rawValue']),
      _ => StreamPartUnknown(j),
    };
  }

  @override
  String toString() => 'StreamPart($type)';
}

/// Start / delta / end of a text segment.
final class StreamPartTextStart extends StreamPart {
  final String id;
  final Json? providerMetadata;
  const StreamPartTextStart({required this.id, this.providerMetadata});
  @override
  Json toJson() => {
        'type': 'text-start',
        'id': id,
        if (providerMetadata != null) 'providerMetadata': providerMetadata,
      };
}

final class StreamPartTextDelta extends StreamPart {
  final String id;

  /// The delta, under `text` as in the AI SDK's `TextStreamPart`.
  final String text;
  final Json? providerMetadata;
  const StreamPartTextDelta(
      {required this.id, required this.text, this.providerMetadata});
  @override
  Json toJson() => {
        'type': 'text-delta',
        'id': id,
        'text': text,
        if (providerMetadata != null) 'providerMetadata': providerMetadata,
      };
}

final class StreamPartTextEnd extends StreamPart {
  final String id;
  final Json? providerMetadata;
  const StreamPartTextEnd({required this.id, this.providerMetadata});
  @override
  Json toJson() => {
        'type': 'text-end',
        'id': id,
        if (providerMetadata != null) 'providerMetadata': providerMetadata,
      };
}

/// Opens the stream (the AI SDK's `start` part).
final class StreamPartStart extends StreamPart {
  const StreamPartStart();
  @override
  Json toJson() => {'type': 'start'};
}

/// Opens a step: the step's request and the provider's call warnings.
final class StreamPartStartStep extends StreamPart {
  final RequestInfo request;
  final List<Json> warnings;
  const StreamPartStartStep({this.request = const RequestInfo(), this.warnings = const []});
  @override
  Json toJson() => {'type': 'start-step', 'request': request.toJson(), 'warnings': warnings};
}

/// Closes a step: its response metadata, usage, performance and finish reason.
final class StreamPartFinishStep extends StreamPart {
  final ResponseMetadata response;
  final LanguageModelUsage usage;

  /// Timing of the step (Rust `StepResultPerformance`). Weak type.
  final Json? performance;
  final String finishReason;
  final String? rawFinishReason;
  final Json? providerMetadata;
  const StreamPartFinishStep({
    required this.response,
    required this.usage,
    this.performance,
    required this.finishReason,
    this.rawFinishReason,
    this.providerMetadata,
  });
  @override
  Json toJson() => {
        'type': 'finish-step',
        'response': response.toJson(),
        'usage': usage.toJson(),
        if (performance != null) 'performance': performance,
        'finishReason': finishReason,
        if (rawFinishReason != null) 'rawFinishReason': rawFinishReason,
        if (providerMetadata != null) 'providerMetadata': providerMetadata,
      };
}

/// Closes the stream: the last step's finish reason and the usage summed over
/// the steps.
final class StreamPartFinish extends StreamPart {
  final String finishReason;
  final String? rawFinishReason;
  final LanguageModelUsage totalUsage;
  const StreamPartFinish(
      {required this.finishReason, this.rawFinishReason, required this.totalUsage});
  @override
  Json toJson() => {
        'type': 'finish',
        'finishReason': finishReason,
        if (rawFinishReason != null) 'rawFinishReason': rawFinishReason,
        'totalUsage': totalUsage.toJson(),
      };
}

/// Ends a stream whose call was aborted.
final class StreamPartAbort extends StreamPart {
  final String? reason;
  const StreamPartAbort({this.reason});
  @override
  Json toJson() => {'type': 'abort', if (reason != null) 'reason': reason};
}

/// An error mid-stream: an `AiMuxError` object keyed by `name`
/// (`{"name": "AI_APICallError", ...}`).
final class StreamPartError extends StreamPart {
  final Json error;
  const StreamPartError({required this.error});
  @override
  Json toJson() => {'type': 'error', 'error': error};
}

/// Start of a tool call's input streaming.
final class StreamPartToolInputStart extends StreamPart {
  final String id;
  final String toolName;
  final bool? providerExecuted;
  final bool? isDynamic;
  final String? title;
  final Json? providerMetadata;
  const StreamPartToolInputStart({
    required this.id,
    required this.toolName,
    this.providerExecuted,
    this.isDynamic,
    this.title,
    this.providerMetadata,
  });
  @override
  Json toJson() => {
        'type': 'tool-input-start',
        'id': id,
        'toolName': toolName,
        if (providerExecuted != null) 'providerExecuted': providerExecuted,
        if (isDynamic != null) 'dynamic': isDynamic,
        if (title != null) 'title': title,
        if (providerMetadata != null) 'providerMetadata': providerMetadata,
      };
}

/// A delta of tool call input (partial JSON text).
final class StreamPartToolInputDelta extends StreamPart {
  final String id;
  final String delta;
  final Json? providerMetadata;
  const StreamPartToolInputDelta(
      {required this.id, required this.delta, this.providerMetadata});
  @override
  Json toJson() => {
        'type': 'tool-input-delta',
        'id': id,
        'delta': delta,
        if (providerMetadata != null) 'providerMetadata': providerMetadata,
      };
}

final class StreamPartToolInputEnd extends StreamPart {
  final String id;
  final Json? providerMetadata;
  const StreamPartToolInputEnd({required this.id, this.providerMetadata});
  @override
  Json toJson() => {
        'type': 'tool-input-end',
        'id': id,
        if (providerMetadata != null) 'providerMetadata': providerMetadata,
      };
}

/// A complete, parsed tool call ([ToolCall.invalid] marks one that stayed
/// invalid after optional repair).
final class StreamPartToolCall extends StreamPart {
  final ToolCall call;
  const StreamPartToolCall(this.call);
  @override
  Json toJson() => {'type': 'tool-call', ...call.toJson()};
}

/// A provider-executed tool's result (Rust `TypedToolResult`), with the
/// input of the call it answers (`null` for a deferred result).
final class StreamPartToolResult extends StreamPart {
  final String toolCallId;
  final String toolName;
  final dynamic input;
  final dynamic output;
  final bool? providerExecuted;
  final Json? providerMetadata;
  final bool? isDynamic;
  final bool? preliminary;
  const StreamPartToolResult({
    required this.toolCallId,
    required this.toolName,
    this.input,
    required this.output,
    this.providerExecuted,
    this.providerMetadata,
    this.isDynamic,
    this.preliminary,
  });
  factory StreamPartToolResult.fromJson(Json j) => StreamPartToolResult(
        toolCallId: j['toolCallId'] as String,
        toolName: j['toolName'] as String,
        input: j['input'],
        output: j['output'],
        providerExecuted: j['providerExecuted'] as bool?,
        providerMetadata: _opt(j['providerMetadata']),
        isDynamic: j['dynamic'] as bool?,
        preliminary: j['preliminary'] as bool?,
      );
  @override
  Json toJson() => {
        'type': 'tool-result',
        'toolCallId': toolCallId,
        'toolName': toolName,
        if (input != null) 'input': input,
        'output': output,
        if (providerExecuted != null) 'providerExecuted': providerExecuted,
        if (providerMetadata != null) 'providerMetadata': providerMetadata,
        if (isDynamic != null) 'dynamic': isDynamic,
        if (preliminary != null) 'preliminary': preliminary,
      };
}

/// A provider-executed tool's error result, or the error that follows an
/// invalid tool call (Rust `TypedToolError`).
final class StreamPartToolError extends StreamPart {
  final String toolCallId;
  final String toolName;
  final dynamic input;
  final dynamic error;
  final bool? providerExecuted;
  final Json? providerMetadata;
  final bool? isDynamic;
  const StreamPartToolError({
    required this.toolCallId,
    required this.toolName,
    this.input,
    required this.error,
    this.providerExecuted,
    this.providerMetadata,
    this.isDynamic,
  });
  factory StreamPartToolError.fromJson(Json j) => StreamPartToolError(
        toolCallId: j['toolCallId'] as String,
        toolName: j['toolName'] as String,
        input: j['input'],
        error: j['error'],
        providerExecuted: j['providerExecuted'] as bool?,
        providerMetadata: _opt(j['providerMetadata']),
        isDynamic: j['dynamic'] as bool?,
      );
  @override
  Json toJson() => {
        'type': 'tool-error',
        'toolCallId': toolCallId,
        'toolName': toolName,
        if (input != null) 'input': input,
        'error': error,
        if (providerExecuted != null) 'providerExecuted': providerExecuted,
        if (providerMetadata != null) 'providerMetadata': providerMetadata,
        if (isDynamic != null) 'dynamic': isDynamic,
      };
}

/// A tool whose execution was denied (Rust `ToolOutputDenied`).
final class StreamPartToolOutputDenied extends StreamPart {
  final String toolCallId;
  final String toolName;
  final bool? providerExecuted;
  final bool? isDynamic;
  const StreamPartToolOutputDenied({
    required this.toolCallId,
    required this.toolName,
    this.providerExecuted,
    this.isDynamic,
  });
  factory StreamPartToolOutputDenied.fromJson(Json j) => StreamPartToolOutputDenied(
        toolCallId: j['toolCallId'] as String,
        toolName: j['toolName'] as String,
        providerExecuted: j['providerExecuted'] as bool?,
        isDynamic: j['dynamic'] as bool?,
      );
  @override
  Json toJson() => {
        'type': 'tool-output-denied',
        'toolCallId': toolCallId,
        'toolName': toolName,
        if (providerExecuted != null) 'providerExecuted': providerExecuted,
        if (isDynamic != null) 'dynamic': isDynamic,
      };
}

/// A file generated by the model: `{"type": "file", "file": GeneratedFile,
/// "providerMetadata"?}` (`stream-language-model-call.ts:569-590`).
final class StreamPartFile extends StreamPart {
  final GeneratedFile file;
  final Json? providerMetadata;
  const StreamPartFile({required this.file, this.providerMetadata});
  @override
  Json toJson() => {
        'type': 'file',
        'file': file.toJson(),
        if (providerMetadata != null) 'providerMetadata': providerMetadata,
      };
}

/// A file generated as part of reasoning (`GeneratedFileOutput.ts`).
final class StreamPartReasoningFile extends StreamPart {
  final GeneratedFile file;
  final Json? providerMetadata;
  const StreamPartReasoningFile({required this.file, this.providerMetadata});
  @override
  Json toJson() => {
        'type': 'reasoning-file',
        'file': file.toJson(),
        if (providerMetadata != null) 'providerMetadata': providerMetadata,
      };
}

final class StreamPartCustom extends StreamPart {
  final String kind;
  final Json? providerMetadata;
  const StreamPartCustom({required this.kind, this.providerMetadata});
  @override
  Json toJson() => {
        'type': 'custom',
        'kind': kind,
        if (providerMetadata != null) 'providerMetadata': providerMetadata,
      };
}

/// A tool call waiting for approval (`ToolApprovalRequestOutput.ts`).
final class StreamPartToolApprovalRequest extends StreamPart {
  final String approvalId;
  final ToolCall toolCall;
  final String? reason;
  final bool? isAutomatic;
  final String? signature;
  const StreamPartToolApprovalRequest({
    required this.approvalId,
    required this.toolCall,
    this.reason,
    this.isAutomatic,
    this.signature,
  });
  @override
  Json toJson() => {
        'type': 'tool-approval-request',
        'approvalId': approvalId,
        'toolCall': toolCall.toJson(),
        if (reason != null) 'reason': reason,
        if (isAutomatic != null) 'isAutomatic': isAutomatic,
        if (signature != null) 'signature': signature,
      };
}

/// The answer to a tool approval request (Rust `ToolApprovalResponseOutput`).
final class StreamPartToolApprovalResponse extends StreamPart {
  final String approvalId;
  final ToolCall toolCall;
  final bool approved;
  final String? reason;
  final bool? providerExecuted;
  const StreamPartToolApprovalResponse({
    required this.approvalId,
    required this.toolCall,
    required this.approved,
    this.reason,
    this.providerExecuted,
  });
  factory StreamPartToolApprovalResponse.fromJson(Json j) => StreamPartToolApprovalResponse(
        approvalId: j['approvalId'] as String,
        toolCall: ToolCall.fromJson(j['toolCall'] as Json),
        approved: j['approved'] as bool,
        reason: j['reason'] as String?,
        providerExecuted: j['providerExecuted'] as bool?,
      );
  @override
  Json toJson() => {
        'type': 'tool-approval-response',
        'approvalId': approvalId,
        'toolCall': toolCall.toJson(),
        'approved': approved,
        if (reason != null) 'reason': reason,
        if (providerExecuted != null) 'providerExecuted': providerExecuted,
      };
}

/// Start / delta / end of a reasoning segment.
final class StreamPartReasoningStart extends StreamPart {
  final String id;
  final Json? providerMetadata;
  const StreamPartReasoningStart({required this.id, this.providerMetadata});
  @override
  Json toJson() => {
        'type': 'reasoning-start',
        'id': id,
        if (providerMetadata != null) 'providerMetadata': providerMetadata,
      };
}

final class StreamPartReasoningDelta extends StreamPart {
  final String id;

  /// The delta, under `text` as in the AI SDK's `TextStreamPart`.
  final String text;
  final Json? providerMetadata;
  const StreamPartReasoningDelta(
      {required this.id, required this.text, this.providerMetadata});
  @override
  Json toJson() => {
        'type': 'reasoning-delta',
        'id': id,
        'text': text,
        if (providerMetadata != null) 'providerMetadata': providerMetadata,
      };
}

final class StreamPartReasoningEnd extends StreamPart {
  final String id;
  final Json? providerMetadata;
  const StreamPartReasoningEnd({required this.id, this.providerMetadata});
  @override
  Json toJson() => {
        'type': 'reasoning-end',
        'id': id,
        if (providerMetadata != null) 'providerMetadata': providerMetadata,
      };
}

/// A source / citation.
final class StreamPartSource extends StreamPart {
  final Source source;
  const StreamPartSource({required this.source});
  @override
  Json toJson() => {'type': 'source', ...source.toJson()};
}

/// A raw provider chunk (when `includeRawChunks` is set).
final class StreamPartRaw extends StreamPart {
  final dynamic rawValue;
  const StreamPartRaw({required this.rawValue});
  @override
  Json toJson() => {'type': 'raw', 'rawValue': rawValue};
}

/// Forward-compatibility: a `type` this binding does not model, verbatim.
final class StreamPartUnknown extends StreamPart {
  final Json data;
  const StreamPartUnknown(this.data);
  @override
  Json toJson() => data;
}

// OpenAI Chat Completions output (RFC-0026).
//
// Mirrors `aimux-core::openai_output`. Field names are camelCase; `@JsonKey`
// maps to the wire's snake_case. The `type` field is JSON `"type"` (Rust
// `#[serde(rename = "type")]`) → `toolType`. Arbitrary-JSON fields
// (`logprobs`, `annotations`) are `dynamic` / `List<dynamic>?`.

/// A complete Chat Completion response (non-streaming). Mirrors OpenAI
/// `chat.completion`.
@JsonSerializable()
class ChatCompletion {
  final String id;
  final String object;
  final int created;
  final String model;
  final List<ChatCompletionChoice> choices;
  final ChatCompletionUsage usage;
  @JsonKey(name: 'system_fingerprint')
  final String? systemFingerprint;

  ChatCompletion({
    required this.id,
    required this.object,
    required this.created,
    required this.model,
    required this.choices,
    required this.usage,
    this.systemFingerprint,
  });

  factory ChatCompletion.fromJson(Map<String, dynamic> json) =>
      _$ChatCompletionFromJson(json);
  Map<String, dynamic> toJson() => _$ChatCompletionToJson(this);
}

@JsonSerializable()
class ChatCompletionChoice {
  final int index;
  final ChatCompletionMessage message;
  @JsonKey(name: 'finish_reason')
  final String? finishReason;
  final dynamic logprobs;

  ChatCompletionChoice({
    required this.index,
    required this.message,
    this.finishReason,
    this.logprobs,
  });

  factory ChatCompletionChoice.fromJson(Map<String, dynamic> json) =>
      _$ChatCompletionChoiceFromJson(json);
  Map<String, dynamic> toJson() => _$ChatCompletionChoiceToJson(this);
}

@JsonSerializable()
class ChatCompletionMessage {
  final String role;
  final String? content;
  @JsonKey(name: 'reasoning_content')
  final String? reasoningContent;
  @JsonKey(name: 'tool_calls')
  final List<ChatCompletionToolCall>? toolCalls;
  final List<dynamic>? annotations;

  ChatCompletionMessage({
    required this.role,
    this.content,
    this.reasoningContent,
    this.toolCalls,
    this.annotations,
  });

  factory ChatCompletionMessage.fromJson(Map<String, dynamic> json) =>
      _$ChatCompletionMessageFromJson(json);
  Map<String, dynamic> toJson() => _$ChatCompletionMessageToJson(this);
}

/// A tool call in a [ChatCompletionMessage].
///
/// Wire: `{"id","type":"function","function":{"name","arguments"}}`. The
/// `type` field is JSON `"type"` (Rust `#[serde(rename = "type")]`).
@JsonSerializable()
class ChatCompletionToolCall {
  final String id;
  @JsonKey(name: 'type')
  final String toolType;
  final ChatCompletionFunction function;

  ChatCompletionToolCall({
    required this.id,
    required this.toolType,
    required this.function,
  });

  factory ChatCompletionToolCall.fromJson(Map<String, dynamic> json) =>
      _$ChatCompletionToolCallFromJson(json);
  Map<String, dynamic> toJson() => _$ChatCompletionToolCallToJson(this);
}

@JsonSerializable()
class ChatCompletionFunction {
  final String name;
  final String arguments;

  ChatCompletionFunction({required this.name, required this.arguments});

  factory ChatCompletionFunction.fromJson(Map<String, dynamic> json) =>
      _$ChatCompletionFunctionFromJson(json);
  Map<String, dynamic> toJson() => _$ChatCompletionFunctionToJson(this);
}

/// Token usage statistics (shared by streaming and non-streaming).
@JsonSerializable()
class ChatCompletionUsage {
  @JsonKey(name: 'prompt_tokens')
  final int promptTokens;
  @JsonKey(name: 'completion_tokens')
  final int completionTokens;
  @JsonKey(name: 'total_tokens')
  final int totalTokens;
  @JsonKey(name: 'prompt_tokens_details')
  final PromptTokensDetails? promptTokensDetails;
  @JsonKey(name: 'completion_tokens_details')
  final CompletionTokensDetails? completionTokensDetails;

  ChatCompletionUsage({
    required this.promptTokens,
    required this.completionTokens,
    required this.totalTokens,
    this.promptTokensDetails,
    this.completionTokensDetails,
  });

  factory ChatCompletionUsage.fromJson(Map<String, dynamic> json) =>
      _$ChatCompletionUsageFromJson(json);
  Map<String, dynamic> toJson() => _$ChatCompletionUsageToJson(this);
}

@JsonSerializable()
class PromptTokensDetails {
  @JsonKey(name: 'cached_tokens')
  final int cachedTokens;
  @JsonKey(name: 'cache_write_tokens')
  final int? cacheWriteTokens;

  PromptTokensDetails({required this.cachedTokens, this.cacheWriteTokens});

  factory PromptTokensDetails.fromJson(Map<String, dynamic> json) =>
      _$PromptTokensDetailsFromJson(json);
  Map<String, dynamic> toJson() => _$PromptTokensDetailsToJson(this);
}

@JsonSerializable()
class CompletionTokensDetails {
  @JsonKey(name: 'reasoning_tokens')
  final int? reasoningTokens;

  CompletionTokensDetails({this.reasoningTokens});

  factory CompletionTokensDetails.fromJson(Map<String, dynamic> json) =>
      _$CompletionTokensDetailsFromJson(json);
  Map<String, dynamic> toJson() => _$CompletionTokensDetailsToJson(this);
}

/// A single Chat Completion chunk (streaming). Mirrors OpenAI
/// `chat.completion.chunk`.
@JsonSerializable()
class ChatCompletionChunk {
  final String id;
  final String object;
  final int created;
  final String model;
  final List<ChatCompletionChunkChoice> choices;
  final ChatCompletionUsage? usage;

  ChatCompletionChunk({
    required this.id,
    required this.object,
    required this.created,
    required this.model,
    required this.choices,
    this.usage,
  });

  factory ChatCompletionChunk.fromJson(Map<String, dynamic> json) =>
      _$ChatCompletionChunkFromJson(json);
  Map<String, dynamic> toJson() => _$ChatCompletionChunkToJson(this);
}

@JsonSerializable()
class ChatCompletionChunkChoice {
  final int index;
  final ChatCompletionDelta delta;
  @JsonKey(name: 'finish_reason')
  final String? finishReason;
  final dynamic logprobs;

  ChatCompletionChunkChoice({
    required this.index,
    required this.delta,
    this.finishReason,
    this.logprobs,
  });

  factory ChatCompletionChunkChoice.fromJson(Map<String, dynamic> json) =>
      _$ChatCompletionChunkChoiceFromJson(json);
  Map<String, dynamic> toJson() => _$ChatCompletionChunkChoiceToJson(this);
}

@JsonSerializable()
class ChatCompletionDelta {
  final String? role;
  final String? content;
  @JsonKey(name: 'reasoning_content')
  final String? reasoningContent;
  @JsonKey(name: 'tool_calls')
  final List<ChatCompletionChunkToolCall>? toolCalls;

  ChatCompletionDelta({
    this.role,
    this.content,
    this.reasoningContent,
    this.toolCalls,
  });

  factory ChatCompletionDelta.fromJson(Map<String, dynamic> json) =>
      _$ChatCompletionDeltaFromJson(json);
  Map<String, dynamic> toJson() => _$ChatCompletionDeltaToJson(this);
}

/// A tool call delta in a [ChatCompletionChunk].
///
/// Wire: `{"index","id"?,"type":"function"?,"function":{"name"?,"arguments"?}}`.
/// The `type` field is JSON `"type"` (Rust `#[serde(rename = "type")]`).
@JsonSerializable()
class ChatCompletionChunkToolCall {
  final int index;
  final String? id;
  @JsonKey(name: 'type')
  final String? toolType;
  final ChatCompletionChunkFunction function;

  ChatCompletionChunkToolCall({
    required this.index,
    this.id,
    this.toolType,
    required this.function,
  });

  factory ChatCompletionChunkToolCall.fromJson(Map<String, dynamic> json) =>
      _$ChatCompletionChunkToolCallFromJson(json);
  Map<String, dynamic> toJson() => _$ChatCompletionChunkToolCallToJson(this);
}

@JsonSerializable()
class ChatCompletionChunkFunction {
  final String? name;
  final String? arguments;

  ChatCompletionChunkFunction({this.name, this.arguments});

  factory ChatCompletionChunkFunction.fromJson(Map<String, dynamic> json) =>
      _$ChatCompletionChunkFunctionFromJson(json);
  Map<String, dynamic> toJson() => _$ChatCompletionChunkFunctionToJson(this);
}
