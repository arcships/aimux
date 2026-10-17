package ai.arcships.aimux;

import ai.arcships.aimux.generated.AimuxOps;

import com.fasterxml.jackson.databind.JsonNode;
import java.util.concurrent.Executor;
import java.util.concurrent.Future;

/** Typed resource facade; request JSON uses the existing core wire DTO. */
public final class ImageModel extends NativeHandle {
    ImageModel(Aimux aimux, long id) { super(aimux, id); }
    public JsonNode info() { return call(AimuxOps.OP_MODEL_INFO, "{}"); }
    public JsonNode generate(String paramsJson) { return call(AimuxOps.OP_IMAGE_GENERATE, paramsJson); }
    public Future<JsonNode> generateAsync(String paramsJson, Executor executor) {
        return callAsync(AimuxOps.OP_IMAGE_GENERATE, paramsJson, executor);
    }
}
