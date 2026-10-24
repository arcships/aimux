package ai.arcships.aimux;

import com.fasterxml.jackson.databind.JsonNode;

/**
 * One link of a native error's {@code cause} chain: a nested AI SDK error
 * ({@link #getErrorName()} is its name) or an underlying OS, TLS or parser
 * error (no name). Its own cause continues the chain.
 */
public final class ErrorCause extends RuntimeException {
    private final JsonNode error;

    private ErrorCause(JsonNode error) {
        super(error.path("message").asText());
        this.error = error.deepCopy();
        ErrorCause cause = of(error);
        if (cause != null) initCause(cause);
    }

    /**
     * The cause of {@code payload}, or null when it has none. A retry error's
     * cause is its last attempt, as Rust's {@code source()} and the AI SDK
     * {@code lastError}.
     */
    static ErrorCause of(JsonNode payload) {
        JsonNode cause = payload.get("cause");
        JsonNode errors = payload.path("errors");
        if (cause == null && "AI_RetryError".equals(payload.path("name").asText()) && errors.isArray() && !errors.isEmpty()) {
            cause = errors.get(errors.size() - 1);
        }
        return cause != null && cause.isObject() ? new ErrorCause(cause) : null;
    }

    public String getErrorName() { return error.path("name").asText(); }
    public JsonNode getError() { return error.deepCopy(); }
}
