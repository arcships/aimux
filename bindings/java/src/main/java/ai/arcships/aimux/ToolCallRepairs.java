package ai.arcships.aimux;

import ai.arcships.aimux.generated.AimuxOps;
import ai.arcships.aimux.types.*;

import com.fasterxml.jackson.databind.JsonNode;

/** Pure DTO repair operations. Host code owns any subsequent repair loop. */
public final class ToolCallRepairs {
    private ToolCallRepairs() {}
    public static ToolRepairContextResult context(Aimux aimux, ToolRepairContextRequest request) {
        // A null result denotes a call that does not need repair.
        return ToolRepairContextResult.fromJson(aimux.call(AimuxOps.OP_TOOL_REPAIR_CONTEXT, request.toString()));
    }
    public static ToolCall apply(Aimux aimux, ToolRepairApplyRequest request) {
        return aimux.call(AimuxOps.OP_TOOL_REPAIR_APPLY, request, ToolCall.class);
    }
    public static GenerateTextResult applyResult(Aimux aimux, ToolRepairApplyResultRequest request) {
        return aimux.call(AimuxOps.OP_TOOL_REPAIR_APPLY_RESULT, request, GenerateTextResult.class);
    }
    public static JsonNode context(Aimux aimux, String paramsJson) {
        return aimux.call(AimuxOps.OP_TOOL_REPAIR_CONTEXT, paramsJson);
    }
    public static JsonNode apply(Aimux aimux, String paramsJson) {
        return aimux.call(AimuxOps.OP_TOOL_REPAIR_APPLY, paramsJson);
    }
    public static JsonNode applyResult(Aimux aimux, String paramsJson) {
        return aimux.call(AimuxOps.OP_TOOL_REPAIR_APPLY_RESULT, paramsJson);
    }
}
