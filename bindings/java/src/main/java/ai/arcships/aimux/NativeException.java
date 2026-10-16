package ai.arcships.aimux;

import ai.arcships.aimux.types.ErrorEnvelope;

import com.fasterxml.jackson.databind.JsonNode;
import java.math.BigInteger;

/** Structured error envelope, retaining the complete AI SDK or domain payload. */
public abstract class NativeException extends RuntimeException {
    private final ErrorEnvelope envelope;

    NativeException(ErrorEnvelope envelope) {
        super(envelope.getMessage());
        this.envelope = envelope;
    }

    public final String getKind() { return envelope.getKind().toJson(); }
    public final int getCode() { return envelope.getCode(); }
    public final String getErrorName() { return envelope.getError().path("name").asText(); }
    public final JsonNode getError() { return envelope.getError(); }
    public final JsonNode getEnvelope() { return envelope.toJson(); }
    public final BigInteger getRetryAfterMs() {
        return envelope.getRetryAfterMs();
    }
}
