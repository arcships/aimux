package ai.arcships.aimux;

import ai.arcships.aimux.types.ErrorEnvelope;

import com.fasterxml.jackson.databind.JsonNode;

/** Binding domain errors from the RFC-0039 envelope. */
public abstract class BindingException extends NativeException {
    BindingException(ErrorEnvelope envelope) { super(envelope); }
    static BindingException decode(ErrorEnvelope envelope) {
        switch (envelope.getError().path("name").asText()) {
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
        NullPointer(ErrorEnvelope envelope) { super(envelope); }
    }
    public static final class InvalidUtf8 extends BindingException {
        InvalidUtf8(ErrorEnvelope envelope) { super(envelope); }
    }
    public static final class InvalidWireJson extends BindingException {
        InvalidWireJson(ErrorEnvelope envelope) { super(envelope); }
    }
    public static final class InvalidHandle extends BindingException {
        InvalidHandle(ErrorEnvelope envelope) { super(envelope); }
    }
    public static final class ReentrantCall extends BindingException {
        ReentrantCall(ErrorEnvelope envelope) { super(envelope); }
    }
    public static final class ResultSerialization extends BindingException {
        ResultSerialization(ErrorEnvelope envelope) { super(envelope); }
    }
    public static final class CallbackFailure extends BindingException {
        CallbackFailure(ErrorEnvelope envelope) { super(envelope); }
    }
    /** An unrecognized variant, retaining its complete envelope. */
    public static final class Unknown extends BindingException {
        Unknown(ErrorEnvelope envelope) { super(envelope); }
    }
}
