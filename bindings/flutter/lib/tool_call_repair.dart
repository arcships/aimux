import 'aimux.dart';

/// Pure repair DTO operations. Host repair functions remain on the caller's
/// isolate; they are never installed as native callbacks.
final class ToolCallRepair {
  final Aimux client;
  ToolCallRepair(this.client);

  Future<Object?> context(Map<String, dynamic> params) =>
      client.call('toolRepair.context', params: params);
  Future<Object?> apply(Map<String, dynamic> params) =>
      client.call('toolRepair.apply', params: params);
  Future<Object?> applyResult(Map<String, dynamic> params) =>
      client.call('toolRepair.applyResult', params: params);
}
