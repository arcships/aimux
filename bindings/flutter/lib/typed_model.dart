import 'aimux.dart';
import 'types.dart';

/// Typed results over the shared ops DTOs. Request maps follow the manifest's
/// request schema; no provider field tables are duplicated in this facade.
final class TypedModel {
  final Aimux client;
  final OpsHandle handle;
  TypedModel(this.client, this.handle);

  Future<GenerateTextResult> generateText(Map<String, dynamic> params) async =>
      GenerateTextResult.fromJson(
        await client.call('text.generate', handle: handle, params: params)
            as Map<String, dynamic>,
      );

  Future<GenerateObjectResult> generateObject(
    Map<String, dynamic> params,
  ) async =>
      GenerateObjectResult.fromJson(
        await client.call('text.generateObject', handle: handle, params: params)
            as Map<String, dynamic>,
      );

  Future<GenerateTextResult> consumeStreamText(
    Map<String, dynamic> params,
  ) async =>
      GenerateTextResult.fromJson(
        await client.call('text.consume', handle: handle, params: params)
            as Map<String, dynamic>,
      );

  Future<ChatCompletion> generateTextAsOpenAI(
    Map<String, dynamic> params,
  ) async =>
      ChatCompletion.fromJson(
        await client.call('text.generateOpenAI', handle: handle, params: params)
            as Map<String, dynamic>,
      );

  Stream<StreamPart> streamText(Map<String, dynamic> params) =>
      _events('text.stream', params).map(StreamPart.fromJson);

  Stream<ChatCompletionChunk> streamTextAsOpenAI(Map<String, dynamic> params) =>
      _events('text.streamOpenAI', params).map(ChatCompletionChunk.fromJson);

  Stream<Map<String, dynamic>> _events(
    String op,
    Map<String, dynamic> params,
  ) async* {
    await for (final frame in client.stream(
      op,
      handle: handle,
      params: params,
    )) {
      if (frame is EventFrame) yield frame.event as Map<String, dynamic>;
      if (frame is EndFrame) {
        if (frame.error != null) throw frame.error!;
        if (frame.outcome == 'cancelled') {
          throw StateError('Operation cancelled');
        }
      }
    }
  }

  Future<void> close() => handle.close();
}
