package ai.arcships.aimux;

import com.fasterxml.jackson.databind.JsonNode;
import java.math.BigInteger;

/** Structured error envelope, retaining the complete AI SDK or domain payload. */
public abstract class NativeException extends RuntimeException {
    private final JsonNode envelope;

    NativeException(JsonNode envelope) {
        super(envelope.path("message").asText());
        this.envelope = envelope.deepCopy();
    }

    public final String getKind() { return envelope.path("kind").asText(); }
    public final int getCode() { return envelope.path("code").intValue(); }
    public final String getErrorName() { return envelope.path("error").path("name").asText(); }
    public final JsonNode getError() { return envelope.path("error").deepCopy(); }
    public final JsonNode getEnvelope() { return envelope.deepCopy(); }
    public final BigInteger getRetryAfterMs() {
        JsonNode hint = envelope.get("retryAfterMs");
        return hint == null ? null : hint.bigIntegerValue();
    }
}
