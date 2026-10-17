package ai.arcships.aimux;

import ai.arcships.aimux.generated.AimuxOps;

import com.fasterxml.jackson.databind.JsonNode;
import java.util.concurrent.Executor;
import java.util.concurrent.Future;

/** Typed resource facade; request JSON uses the existing core wire DTO. */
public final class Model extends NativeHandle {
    Model(Aimux aimux, long id) { super(aimux, id); }
    public JsonNode info() { return call(AimuxOps.OP_MODEL_INFO, "{}"); }
    public JsonNode generate(String paramsJson) { return call(AimuxOps.OP_TEXT_GENERATE, paramsJson); }
    public Future<JsonNode> generateAsync(String paramsJson, Executor executor) {
        return callAsync(AimuxOps.OP_TEXT_GENERATE, paramsJson, executor);
    }
    public JsonNode generateObject(String paramsJson) { return call(AimuxOps.OP_TEXT_GENERATE_OBJECT, paramsJson); }
    public Future<JsonNode> generateObjectAsync(String paramsJson, Executor executor) {
        return callAsync(AimuxOps.OP_TEXT_GENERATE_OBJECT, paramsJson, executor);
    }
    public JsonNode consume(String paramsJson) { return call(AimuxOps.OP_TEXT_CONSUME, paramsJson); }
    public Future<JsonNode> consumeAsync(String paramsJson, Executor executor) {
        return callAsync(AimuxOps.OP_TEXT_CONSUME, paramsJson, executor);
    }
    public JsonNode generateOpenAI(String paramsJson) { return call(AimuxOps.OP_TEXT_GENERATE_OPEN_AI, paramsJson); }
    public Future<JsonNode> generateOpenAIAsync(String paramsJson, Executor executor) {
        return callAsync(AimuxOps.OP_TEXT_GENERATE_OPEN_AI, paramsJson, executor);
    }
    public void stream(String paramsJson, java.util.function.Consumer<StreamFrame> consumer) {
        stream(AimuxOps.OP_TEXT_STREAM, paramsJson, consumer);
    }
    public Future<JsonNode> streamAsync(String paramsJson, java.util.function.Consumer<StreamFrame> consumer,
                                         Executor executor) {
        return streamAsync(AimuxOps.OP_TEXT_STREAM, paramsJson, consumer, executor);
    }
    public void streamOpenAI(String paramsJson, java.util.function.Consumer<StreamFrame> consumer) {
        stream(AimuxOps.OP_TEXT_STREAM_OPEN_AI, paramsJson, consumer);
    }
}
