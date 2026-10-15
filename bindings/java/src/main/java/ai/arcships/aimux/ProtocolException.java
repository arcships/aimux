package ai.arcships.aimux;

import com.fasterxml.jackson.databind.JsonNode;

/** Protocol domain errors from the RFC-0039 envelope. */
public abstract class ProtocolException extends NativeException {
    ProtocolException(JsonNode envelope) { super(envelope); }
    static ProtocolException decode(JsonNode envelope) {
        switch (envelope.path("error").path("name").asText()) {
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
        InvalidOp(JsonNode envelope) { super(envelope); }
    }
    public static final class InvalidParams extends ProtocolException {
        InvalidParams(JsonNode envelope) { super(envelope); }
    }
    public static final class UnsupportedOpsVersion extends ProtocolException {
        UnsupportedOpsVersion(JsonNode envelope) { super(envelope); }
    }
    public static final class ConcurrentConsumer extends ProtocolException {
        ConcurrentConsumer(JsonNode envelope) { super(envelope); }
    }
    public static final class InvalidState extends ProtocolException {
        InvalidState(JsonNode envelope) { super(envelope); }
    }
    public static final class ResourceLimit extends ProtocolException {
        ResourceLimit(JsonNode envelope) { super(envelope); }
    }
    public static final class HandleExhausted extends ProtocolException {
        HandleExhausted(JsonNode envelope) { super(envelope); }
    }
    /** An unrecognized variant, retaining its complete envelope. */
    public static final class Unknown extends ProtocolException {
        Unknown(JsonNode envelope) { super(envelope); }
    }
}
