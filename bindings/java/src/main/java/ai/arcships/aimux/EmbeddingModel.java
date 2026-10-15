package ai.arcships.aimux;

import ai.arcships.aimux.generated.AimuxOps;

import com.fasterxml.jackson.databind.JsonNode;
import java.util.concurrent.Executor;
import java.util.concurrent.Future;

/** Typed resource facade; request JSON uses the existing core wire DTO. */
public final class EmbeddingModel extends NativeHandle {
    EmbeddingModel(Aimux aimux, long id) { super(aimux, id); }
    public JsonNode info() { return call(AimuxOps.OP_MODEL_INFO, "{}"); }
    public JsonNode embed(String paramsJson) { return call(AimuxOps.OP_EMBEDDING_EMBED, paramsJson); }
    public Future<JsonNode> embedAsync(String paramsJson, Executor executor) {
        return callAsync(AimuxOps.OP_EMBEDDING_EMBED, paramsJson, executor);
    }
    public JsonNode embedMany(String paramsJson) { return call(AimuxOps.OP_EMBEDDING_EMBED_MANY, paramsJson); }
    public Future<JsonNode> embedManyAsync(String paramsJson, Executor executor) {
        return callAsync(AimuxOps.OP_EMBEDDING_EMBED_MANY, paramsJson, executor);
    }
}
