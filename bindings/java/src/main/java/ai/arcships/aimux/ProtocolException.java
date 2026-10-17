package ai.arcships.aimux;

import ai.arcships.aimux.types.ErrorEnvelope;

import com.fasterxml.jackson.databind.JsonNode;

/** Protocol domain errors from the RFC-0039 envelope. */
public abstract class ProtocolException extends NativeException {
    ProtocolException(ErrorEnvelope envelope) { super(envelope); }
    static ProtocolException decode(ErrorEnvelope envelope) {
        switch (envelope.getError().path("name").asText()) {
            case "InvalidOp": return new InvalidOp(envelope);
            case "InvalidParams": return new InvalidParams(envelope);
            case "UnsupportedOpsVersion": return new UnsupportedOpsVersion(envelope);
            case "ConcurrentConsumer": return new ConcurrentConsumer(envelope);
            case "InvalidState": return new InvalidState(envelope);
            case "ResourceLimit": return new ResourceLimit(envelope);
            case "HandleExhausted": return new HandleExhausted(envelope);
            default: return new Unknown(envelope);
        }
    }

    public static final class InvalidOp extends ProtocolException {
        InvalidOp(ErrorEnvelope envelope) { super(envelope); }
    }
    public static final class InvalidParams extends ProtocolException {
        InvalidParams(ErrorEnvelope envelope) { super(envelope); }
    }
    public static final class UnsupportedOpsVersion extends ProtocolException {
        UnsupportedOpsVersion(ErrorEnvelope envelope) { super(envelope); }
    }
    public static final class ConcurrentConsumer extends ProtocolException {
        ConcurrentConsumer(ErrorEnvelope envelope) { super(envelope); }
    }
    public static final class InvalidState extends ProtocolException {
        InvalidState(ErrorEnvelope envelope) { super(envelope); }
    }
    public static final class ResourceLimit extends ProtocolException {
        ResourceLimit(ErrorEnvelope envelope) { super(envelope); }
    }
    public static final class HandleExhausted extends ProtocolException {
        HandleExhausted(ErrorEnvelope envelope) { super(envelope); }
    }
    /** An unrecognized variant, retaining its complete envelope. */
    public static final class Unknown extends ProtocolException {
        Unknown(ErrorEnvelope envelope) { super(envelope); }
    }
}
