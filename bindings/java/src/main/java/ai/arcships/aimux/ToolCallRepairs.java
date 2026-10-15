package ai.arcships.aimux;

import ai.arcships.aimux.generated.AimuxOps;

import com.fasterxml.jackson.databind.JsonNode;

/** Pure DTO repair operations. Host code owns any subsequent repair loop. */
public final class ToolCallRepairs {
    private ToolCallRepairs() {}
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
