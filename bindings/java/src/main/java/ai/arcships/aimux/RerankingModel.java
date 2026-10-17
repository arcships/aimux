package ai.arcships.aimux;

import ai.arcships.aimux.generated.AimuxOps;

import com.fasterxml.jackson.databind.JsonNode;
import java.util.concurrent.Executor;
import java.util.concurrent.Future;

/** Typed resource facade; request JSON uses the existing core wire DTO. */
public final class RerankingModel extends NativeHandle {
    RerankingModel(Aimux aimux, long id) { super(aimux, id); }
    public JsonNode info() { return call(AimuxOps.OP_MODEL_INFO, "{}"); }
    public JsonNode rerank(String paramsJson) { return call(AimuxOps.OP_RERANKING_RERANK, paramsJson); }
    public Future<JsonNode> rerankAsync(String paramsJson, Executor executor) {
        return callAsync(AimuxOps.OP_RERANKING_RERANK, paramsJson, executor);
    }
}
