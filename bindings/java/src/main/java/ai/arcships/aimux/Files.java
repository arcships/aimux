package ai.arcships.aimux;

import ai.arcships.aimux.generated.AimuxOps;

import com.fasterxml.jackson.databind.JsonNode;
import java.util.concurrent.Executor;
import java.util.concurrent.Future;

/** Typed resource facade; request JSON uses the existing core wire DTO. */
public final class Files extends NativeHandle {
    Files(Aimux aimux, long id) { super(aimux, id); }
    public JsonNode upload(String paramsJson) { return call(AimuxOps.OP_FILES_UPLOAD, paramsJson); }
    public Future<JsonNode> uploadAsync(String paramsJson, Executor executor) {
        return callAsync(AimuxOps.OP_FILES_UPLOAD, paramsJson, executor);
    }
}
