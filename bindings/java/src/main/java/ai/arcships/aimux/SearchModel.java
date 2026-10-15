package ai.arcships.aimux;

import ai.arcships.aimux.generated.AimuxOps;

import com.fasterxml.jackson.databind.JsonNode;
import java.util.concurrent.Executor;
import java.util.concurrent.Future;

/** Typed resource facade; request JSON uses the existing core wire DTO. */
public final class SearchModel extends NativeHandle {
    SearchModel(Aimux aimux, long id) { super(aimux, id); }
    public JsonNode info() { return call(AimuxOps.OP_MODEL_INFO, "{}"); }
    public JsonNode search(String paramsJson) { return call(AimuxOps.OP_SEARCH_SEARCH, paramsJson); }
    public Future<JsonNode> searchAsync(String paramsJson, Executor executor) {
        return callAsync(AimuxOps.OP_SEARCH_SEARCH, paramsJson, executor);
    }
}
