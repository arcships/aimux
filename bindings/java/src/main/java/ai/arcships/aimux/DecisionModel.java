package ai.arcships.aimux;

import ai.arcships.aimux.generated.AimuxOps;

import com.fasterxml.jackson.databind.JsonNode;
import java.util.concurrent.Executor;
import java.util.concurrent.Future;

/** Decision model resource; requests contain the canonical options object. */
public final class DecisionModel extends NativeHandle {
    DecisionModel(Aimux aimux, long id) { super(aimux, id); }
    public JsonNode capabilities() { return call(AimuxOps.OP_DECISION_CAPABILITIES, "{}"); }
    public JsonNode decide(String paramsJson) { return call(AimuxOps.OP_DECISION_DECIDE, paramsJson); }
    public Future<JsonNode> decideAsync(String paramsJson, Executor executor) {
        return callAsync(AimuxOps.OP_DECISION_DECIDE, paramsJson, executor);
    }
}
