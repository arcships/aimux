package ai.arcships.aimux;

import ai.arcships.aimux.generated.AimuxOps;

import com.fasterxml.jackson.databind.JsonNode;
import java.util.concurrent.Executor;
import java.util.concurrent.Future;

/** Typed resource facade; request JSON uses the existing core wire DTO. */
public final class TranscriptionModel extends NativeHandle {
    TranscriptionModel(Aimux aimux, long id) { super(aimux, id); }
    public JsonNode info() { return call(AimuxOps.OP_MODEL_INFO, "{}"); }
    public JsonNode generate(String paramsJson) { return call(AimuxOps.OP_TRANSCRIPTION_GENERATE, paramsJson); }
    public Future<JsonNode> generateAsync(String paramsJson, Executor executor) {
        return callAsync(AimuxOps.OP_TRANSCRIPTION_GENERATE, paramsJson, executor);
    }
    public TranscriptionSession start(String paramsJson) {
        return new TranscriptionSession(aimux, AimuxResult.handle(call(AimuxOps.OP_TRANSCRIPTION_START, paramsJson)));
    }
}
