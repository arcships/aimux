package ai.arcships.aimux;

import ai.arcships.aimux.types.ErrorEnvelope;
import ai.arcships.aimux.types.DtoJson;

import com.fasterxml.jackson.databind.JsonNode;

/** Structured error envelope, retaining the complete AI SDK or domain payload. */
public abstract class NativeException extends RuntimeException {
    private final ErrorEnvelope envelope;

    NativeException(ErrorEnvelope envelope) {
        super(envelope.getMessage());
        this.envelope = envelope;
    }

    public final String getKind() { return DtoJson.MAPPER.convertValue(envelope.getKind(), String.class); }
    public final int getCode() { return envelope.getCode(); }
    public final String getErrorName() { return envelope.getError().path("name").asText(); }
    public final JsonNode getError() { return envelope.getError(); }
    public final JsonNode getEnvelope() { return DtoJson.MAPPER.valueToTree(envelope); }
    public final Long getRetryAfterMs() {
        return envelope.getRetryAfterMs();
    }
}
