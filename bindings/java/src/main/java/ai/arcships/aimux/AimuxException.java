package ai.arcships.aimux;

import com.fasterxml.jackson.databind.JsonNode;

/** AI SDK-aligned errors, selected by error.name in the envelope. */
public abstract class AimuxException extends NativeException {
    AimuxException(JsonNode envelope) { super(envelope); }
    public final boolean isRetryable() { return getError().path("isRetryable").asBoolean(false); }
    public final int getStatusCode() { return getError().path("statusCode").asInt(-1); }

    static AimuxException decode(JsonNode envelope) {
        switch (envelope.path("error").path("name").asText()) {
            case "AI_Error": return new OtherError(envelope);
            case "AI_JSONParseError": return new JSONParseError(envelope);
            case "AI_InvalidResponseDataError": return new InvalidResponseDataError(envelope);
            case "AI_ToolCallNotFoundForApprovalError": return new ToolCallNotFoundForApprovalError(envelope);
            case "AI_InvalidArgumentError": return new InvalidArgumentError(envelope);
            case "AI_InvalidPromptError": return new InvalidPromptError(envelope);
            case "AI_TokenExpiredError": return new TokenExpiredError(envelope);
            case "AI_UnsupportedFunctionalityError": return new UnsupportedFunctionalityError(envelope);
            case "AI_NoSuchModelError": return new NoSuchModelError(envelope);
            case "AI_NoSuchProviderError": return new NoSuchProviderError(envelope);
            case "AI_APICallError": return new APICallError(envelope);
            case "TimeoutError": return new TimeoutError(envelope);
            case "AbortError": return new RequestAbortedError(envelope);
            case "AI_RetryError": return new RetryError(envelope);
            case "AI_NoSuchToolError": return new NoSuchToolError(envelope);
            case "AI_InvalidToolInputError": return new InvalidToolInputError(envelope);
            case "AI_ToolCallRepairError": return new ToolCallRepairError(envelope);
            case "AI_NoOutputGeneratedError": return new NoOutputGeneratedError(envelope);
            case "AI_LoadAPIKeyError": return new LoadAPIKeyError(envelope);
            case "AI_LoadSettingError": return new LoadSettingError(envelope);
            case "AI_MissingToolResultsError": return new MissingToolResultsError(envelope);
            case "AI_ToolChoiceViolationError": return new ToolChoiceViolationError(envelope);
            default: return new Unknown(envelope);
        }
    }

    public static final class OtherError extends AimuxException {
        OtherError(JsonNode envelope) { super(envelope); }
    }
    public static final class JSONParseError extends AimuxException {
        JSONParseError(JsonNode envelope) { super(envelope); }
    }
    public static final class InvalidResponseDataError extends AimuxException {
        InvalidResponseDataError(JsonNode envelope) { super(envelope); }
    }
    public static final class ToolCallNotFoundForApprovalError extends AimuxException {
        ToolCallNotFoundForApprovalError(JsonNode envelope) { super(envelope); }
    }
    public static final class InvalidArgumentError extends AimuxException {
        InvalidArgumentError(JsonNode envelope) { super(envelope); }
    }
    public static final class InvalidPromptError extends AimuxException {
        InvalidPromptError(JsonNode envelope) { super(envelope); }
    }
    public static final class TokenExpiredError extends AimuxException {
        TokenExpiredError(JsonNode envelope) { super(envelope); }
    }
    public static final class UnsupportedFunctionalityError extends AimuxException {
        UnsupportedFunctionalityError(JsonNode envelope) { super(envelope); }
    }
    public static final class NoSuchModelError extends AimuxException {
        NoSuchModelError(JsonNode envelope) { super(envelope); }
    }
    public static final class NoSuchProviderError extends AimuxException {
        NoSuchProviderError(JsonNode envelope) { super(envelope); }
    }
    public static final class APICallError extends AimuxException {
        APICallError(JsonNode envelope) { super(envelope); }
    }
    public static final class TimeoutError extends AimuxException {
        TimeoutError(JsonNode envelope) { super(envelope); }
    }
    public static final class RequestAbortedError extends AimuxException {
        RequestAbortedError(JsonNode envelope) { super(envelope); }
    }
    public static final class RetryError extends AimuxException {
        RetryError(JsonNode envelope) { super(envelope); }
    }
    public static final class NoSuchToolError extends AimuxException {
        NoSuchToolError(JsonNode envelope) { super(envelope); }
    }
    public static final class InvalidToolInputError extends AimuxException {
        InvalidToolInputError(JsonNode envelope) { super(envelope); }
    }
    public static final class ToolCallRepairError extends AimuxException {
        ToolCallRepairError(JsonNode envelope) { super(envelope); }
    }
    public static final class NoOutputGeneratedError extends AimuxException {
        NoOutputGeneratedError(JsonNode envelope) { super(envelope); }
    }
    public static final class LoadAPIKeyError extends AimuxException {
        LoadAPIKeyError(JsonNode envelope) { super(envelope); }
    }
    public static final class LoadSettingError extends AimuxException {
        LoadSettingError(JsonNode envelope) { super(envelope); }
    }
    public static final class MissingToolResultsError extends AimuxException {
        MissingToolResultsError(JsonNode envelope) { super(envelope); }
    }
    public static final class ToolChoiceViolationError extends AimuxException {
        ToolChoiceViolationError(JsonNode envelope) { super(envelope); }
    }
    /** An unrecognized variant, retaining its complete envelope. */
    public static final class Unknown extends AimuxException {
        Unknown(JsonNode envelope) { super(envelope); }
    }
}
