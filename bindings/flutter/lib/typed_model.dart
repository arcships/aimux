// typed_model.dart — typed wrapper around the raw dart:ffi `Model`.
//
// Eliminates the JSON/Map boundary at the FFI edge: inputs and outputs are
// typed classes (see `types.dart`). The raw `Model` API in `aimux.dart` is
// left unchanged — this layer only (de)serializes at the boundary, so any
// existing caller using `Model.generateText` / `Model.streamText` keeps
// working byte-for-byte.

import 'dart:async';

import 'package:aimux/aimux.dart';
import 'package:aimux/types.dart';

/// A typed facade over [Model].
///
/// Wraps the raw `Map<String, dynamic>`-based API with typed classes:
/// [GenerateTextOptions] in, [GenerateTextResult] out, [StreamPart]s out of
/// `streamText`. Construct with a [Model] (e.g. `Model.openai(...)`); call
/// [close] to release the native handle.
///
/// Example:
/// ```dart
/// final model = Model.openai(apiKey, 'gpt-4o', baseUrl: baseUrl);
/// final typed = TypedModel(model);
/// try {
///   final result = typed.generateText('Hello', GenerateTextOptions(temperature: 0.7));
///   print(result.text);
/// } finally {
///   typed.close();
/// }
/// ```
class TypedModel {
  final Model _raw;

  TypedModel(this._raw);

  /// Generate text from a string [prompt].
  ///
  /// [options] are serialized and passed to the underlying FFI call. Returns
  /// a typed [GenerateTextResult]. Throws if the provider returns an error.
  GenerateTextResult generateText(
    String prompt, [
    GenerateTextOptions? options,
  ]) {
    final result = _raw.generateText(prompt, options?.toJson());
    return GenerateTextResult.fromJson(result);
  }

  /// Generate text from a list of typed [ModelMessage]s (multi-turn).
  ///
  /// Each message is serialized with [ModelMessage.toJson] and forwarded as a
  /// multi-message prompt (the same shape `Model.generateText` accepts).
  GenerateTextResult generateTextMessages(
    List<ModelMessage> messages, [
    GenerateTextOptions? options,
  ]) {
    final prompt = messages.map((m) => m.toJson()).toList();
    final result = _raw.generateText(prompt, options?.toJson());
    return GenerateTextResult.fromJson(result);
  }

  // ── generateObject (M12, RFC-0016) ──────────────────────────────────────

  /// Generate a structured JSON object from a string [prompt] (M12).
  ///
  /// Same signature as [generateText]; returns a typed
  /// [GenerateObjectResult]. Pass `response_format: { "Json": { ... } }`
  /// via [options] for schema control; the engine applies JSON repair
  /// before parsing.
  GenerateObjectResult generateObject(
    String prompt, [
    GenerateTextOptions? options,
  ]) {
    final result = _raw.generateObject(prompt, options?.toJson());
    return GenerateObjectResult.fromJson(result);
  }

  /// Generate a structured JSON object from a list of typed [ModelMessage]s
  /// (M12).
  GenerateObjectResult generateObjectMessages(
    List<ModelMessage> messages, [
    GenerateTextOptions? options,
  ]) {
    final prompt = messages.map((m) => m.toJson()).toList();
    final result = _raw.generateObject(prompt, options?.toJson());
    return GenerateObjectResult.fromJson(result);
  }

  // ── consumeStreamText (M11, RFC-0016) ───────────────────────────────────

  /// Consume a stream to completion from a string [prompt] and return the
  /// aggregated result (M11). Synchronous (blocks until the stream finishes).
  StreamTextResultAggregated consumeStreamText(
    String prompt, [
    GenerateTextOptions? options,
  ]) {
    final result = _raw.consumeStreamText(prompt, options?.toJson());
    return StreamTextResultAggregated.fromJson(result);
  }

  /// Consume a stream to completion from a list of typed [ModelMessage]s
  /// and return the aggregated result (M11).
  StreamTextResultAggregated consumeStreamTextMessages(
    List<ModelMessage> messages, [
    GenerateTextOptions? options,
  ]) {
    final prompt = messages.map((m) => m.toJson()).toList();
    final result = _raw.consumeStreamText(prompt, options?.toJson());
    return StreamTextResultAggregated.fromJson(result);
  }

  /// Stream text, yielding typed [StreamPart]s.
  ///
  /// [prompt] may be a `String` or a `List<Map<String, dynamic>>` of messages,
  /// matching the raw `Model.streamText` contract. The stream blocks the
  /// calling isolate until completion (see `aimux.dart` for FFI semantics).
  Stream<StreamPart> streamText(
    Object prompt, [
    GenerateTextOptions? options,
  ]) {
    return _raw.streamText(prompt, options?.toJson()).map(StreamPart.fromJson);
  }

  // ── OpenAI-compatible output (RFC-0026) ─────────────────────────────────

  /// Generate text (non-streaming) with OpenAI Chat Completions output.
  ///
  /// Returns a typed [ChatCompletion]. Throws if the provider returns an error.
  ChatCompletion generateTextAsOpenAI(
    String prompt, [
    GenerateTextOptions? options,
  ]) {
    final result = _raw.generateTextAsOpenAI(prompt, options?.toJson());
    return ChatCompletion.fromJson(result);
  }

  /// Generate text from a list of typed [ModelMessage]s with OpenAI Chat
  /// Completions output.
  ChatCompletion generateTextAsOpenAIMessages(
    List<ModelMessage> messages, [
    GenerateTextOptions? options,
  ]) {
    final prompt = messages.map((m) => m.toJson()).toList();
    final result = _raw.generateTextAsOpenAI(prompt, options?.toJson());
    return ChatCompletion.fromJson(result);
  }

  /// Stream text with OpenAI Chat Completions output, yielding typed
  /// [ChatCompletionChunk]s (RFC-0026).
  ///
  /// [prompt] may be a `String` or a `List<Map<String, dynamic>>` of messages.
  /// Stream options (`include_usage`, `include_reasoning`) are passed via
  /// `options.providerOptions` → `openai.stream_options` on the wire.
  Stream<ChatCompletionChunk> streamTextAsOpenAI(
    Object prompt, [
    GenerateTextOptions? options,
  ]) {
    return _raw
        .streamTextAsOpenAI(prompt, options?.toJson())
        .map(ChatCompletionChunk.fromJson);
  }

  /// Release the native handle. Delegates to the wrapped [Model]; safe to
  /// call multiple times.
  void close() => _raw.close();
}
