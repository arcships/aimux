import 'generated/ops.dart' as ops;
import 'aimux.dart';

/// Pure repair DTO operations. Host repair functions remain on the caller's
/// isolate; they are never installed as native callbacks.
final class ToolCallRepair {
  final Aimux client;
  ToolCallRepair(this.client);

  Future<Object?> context(Map<String, dynamic> params) =>
      client.call(ops.OP_TOOL_REPAIR_CONTEXT, params: params);
  Future<Object?> apply(Map<String, dynamic> params) =>
      client.call(ops.OP_TOOL_REPAIR_APPLY, params: params);
  Future<Object?> applyResult(Map<String, dynamic> params) =>
      client.call(ops.OP_TOOL_REPAIR_APPLY_RESULT, params: params);
}
