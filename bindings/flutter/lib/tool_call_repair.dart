import 'generated/ops.dart' as ops;
import 'aimux.dart';

/// Pure repair DTO operations. Host repair functions remain on the caller's
/// isolate; they are never installed as native callbacks.
final class ToolCallRepair {
  final Aimux client;
  ToolCallRepair(this.client);

  Future<ToolRepairContextResult> context(
          ToolRepairContextRequest params) async =>
      ToolRepairContextResult.fromJson(
          await client.call(ops.OP_TOOL_REPAIR_CONTEXT, params: params));
  Future<ToolCall> apply(ToolRepairApplyRequest params) async =>
      ToolCall.fromJson((await client.call(ops.OP_TOOL_REPAIR_APPLY,
          params: params)) as Map<String, dynamic>);
  Future<GenerateTextResult> applyResult(
          ToolRepairApplyResultRequest params) async =>
      GenerateTextResult.fromJson(
          (await client.call(ops.OP_TOOL_REPAIR_APPLY_RESULT, params: params))
              as Map<String, dynamic>);
}
