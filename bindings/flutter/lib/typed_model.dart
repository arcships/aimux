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

  /// Release the native handle. Delegates to the wrapped [Model]; safe to
  /// call multiple times.
  void close() => _raw.close();
}
