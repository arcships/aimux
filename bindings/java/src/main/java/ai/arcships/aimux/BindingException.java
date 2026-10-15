package ai.arcships.aimux;

import com.fasterxml.jackson.databind.JsonNode;

/** Binding domain errors from the RFC-0039 envelope. */
public abstract class BindingException extends NativeException {
    BindingException(JsonNode envelope) { super(envelope); }
    static BindingException decode(JsonNode envelope) {
        switch (envelope.path("error").path("name").asText()) {
            case "NullPointer": return new NullPointer(envelope);
            case "InvalidUtf8": return new InvalidUtf8(envelope);
            case "InvalidWireJson": return new InvalidWireJson(envelope);
            case "InvalidHandle": return new InvalidHandle(envelope);
            case "ReentrantCall": return new ReentrantCall(envelope);
            case "ResultSerialization": return new ResultSerialization(envelope);
            case "CallbackFailure": return new CallbackFailure(envelope);
            default: return new Unknown(envelope);
        }
    }

    public static final class NullPointer extends BindingException {
        NullPointer(JsonNode envelope) { super(envelope); }
    }
    public static final class InvalidUtf8 extends BindingException {
        InvalidUtf8(JsonNode envelope) { super(envelope); }
    }
    public static final class InvalidWireJson extends BindingException {
        InvalidWireJson(JsonNode envelope) { super(envelope); }
    }
    public static final class InvalidHandle extends BindingException {
        InvalidHandle(JsonNode envelope) { super(envelope); }
    }
    public static final class ReentrantCall extends BindingException {
        ReentrantCall(JsonNode envelope) { super(envelope); }
    }
    public static final class ResultSerialization extends BindingException {
        ResultSerialization(JsonNode envelope) { super(envelope); }
    }
    public static final class CallbackFailure extends BindingException {
        CallbackFailure(JsonNode envelope) { super(envelope); }
    }
    /** An unrecognized variant, retaining its complete envelope. */
    public static final class Unknown extends BindingException {
        Unknown(JsonNode envelope) { super(envelope); }
    }
}
