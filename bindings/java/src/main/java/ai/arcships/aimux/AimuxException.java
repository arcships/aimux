package ai.arcships.aimux;

import com.fasterxml.jackson.databind.JsonNode;
import com.sun.jna.Pointer;
import java.util.List;

/**
 * AiMuxError hierarchy (OpenAI Java / Vercel AI SDK style).
 *
 * <p>Raised when a fallible C ABI call returns an AiMuxError code (1–13, 15–17).
 * Recording failures use the
 * independent {@link RecordingException} type; C ABI failures (bad raw
 * wire JSON, use-after-close, re-entrant call) surface as plain
 * {@link IllegalArgumentException} / {@link IllegalStateException}. Prefer
 * {@code instanceof} on subclasses over stringly code checks:
 *
 * <pre>{@code
 * try {
 *     model.generateText("\"hi\"");
 * } catch (APICallError e) {
 *     // Classification is the status field (AI SDK APICallError.statusCode):
 *     // 429 → rate limited (e.getRetryMs()), 401 → auth, 404 → model
 * } catch (AimuxException e) {
 *     // any AiMuxError
 * }
 * }</pre>
 *
 * <p>Every instance carries {@link #getCode()} (C {@code aimux_error_code_t} 1–13, 15–17),
 * {@link #getStatusCode()} (HTTP or {@code -1}), {@link #getRetryMs()} (hint
 * or {@code -1}; {@code 0} = retry now) and {@link #isRetryable()}. Message
 * text comes from the C layer.
 *
 * <p>Message-only constructors default {@code status=-1}, {@code retryMs=-1} for
 * backward compatibility with typed-layer decode failures.
 */
public class AimuxException extends RuntimeException {

    private static final long serialVersionUID = 1L;

    // ── aimux_error_code_t (aimux-error.h) ──────────────────────────────────
    // 15 variant codes (1–13 and 15–17; 1 is the catch-all OTHER; 4 is retired —
    // the legacy Tool variant — and 14 is reserved); every HTTP-shaped failure
    // arrives as AIMUX_E_API_CALL. A code outside that set is a header/library
    // mismatch and fails with IllegalStateException, never an AimuxException.
    // Recording failures are a different type: see RecordingException.

    public static final int AIMUX_OK = 0;
    public static final int AIMUX_E_JSON_PARSE = 2;
    public static final int AIMUX_E_INVALID_RESPONSE_DATA = 3;
    public static final int AIMUX_E_INVALID_ARGUMENT = 5;
    public static final int AIMUX_E_INVALID_PROMPT = 6;
    public static final int AIMUX_E_TOKEN_EXPIRED = 7;
    public static final int AIMUX_E_UNSUPPORTED_FUNCTIONALITY = 8;
    public static final int AIMUX_E_NO_SUCH_MODEL = 9;
    public static final int AIMUX_E_NO_SUCH_PROVIDER = 10;
    public static final int AIMUX_E_API_CALL = 11;
    public static final int AIMUX_E_TIMEOUT = 12;
    public static final int AIMUX_E_ABORTED = 13;
    public static final int AIMUX_E_NO_SUCH_TOOL = 15;
    public static final int AIMUX_E_INVALID_TOOL_INPUT = 16;
    public static final int AIMUX_E_TOOL_CALL_REPAIR = 17;
    public static final int AIMUX_E_OTHER = 1;

    private final int code;
    private final int status;
    private final long retryMs;

    // Set once by the fromC construction path; false for local / synthesized
    // failures. Not a constructor param so the 15 subclass constructors keep
    // their public signatures.
    private boolean retryable;

    // ── Constructors ────────────────────────────────────────────────────────

    /** Message-only; code = {@link #AIMUX_E_OTHER}, status/retryMs = -1. */
    public AimuxException(String message) {
        this(message, AIMUX_E_OTHER, -1, -1L);
    }

    /** Message + cause; code = {@link #AIMUX_E_OTHER}, status/retryMs = -1. */
    public AimuxException(String message, Throwable cause) {
        this(message, AIMUX_E_OTHER, -1, -1L, cause);
    }

    public AimuxException(String message, int code, int status, long retryMs) {
        super(message);
        this.code = code;
        this.status = status;
        this.retryMs = retryMs;
    }

    public AimuxException(String message, int code, int status, long retryMs, Throwable cause) {
        super(message, cause);
        this.code = code;
        this.status = status;
        this.retryMs = retryMs;
    }

    // ── Accessors ───────────────────────────────────────────────────────────

    /** C {@code aimux_error_code_t} value (1–13, 15–17). */
    public int getCode() {
        return code;
    }

    /** HTTP status when known; otherwise {@code -1}. */
    public int getStatusCode() {
        return status;
    }

    /** Rate-limit hint in ms; {@code -1} if none; {@code 0} means retry immediately. */
    public long getRetryMs() {
        return retryMs;
    }

    /**
     * The {@code AiMuxError} retry verdict: {@code true} when retrying may help.
     *
     * <p>Not derivable from {@link #getStatusCode()} — two failures can both
     * report {@code -1} and disagree: a transport failure (the request went out,
     * the connection was reset) is retryable, a missing API key (the request
     * never went out) is not. {@code false} for local (non-FFI) failures and
     * for failures synthesized by the C ABI.
     */
    public boolean isRetryable() {
        return retryable;
    }

    // ── Factories ───────────────────────────────────────────────────────────

    /**
     * Build a typed exception from a returned {@code const aimux_error_t *},
     * prefixing {@code prefix} to the message. Reads the code, message,
     * retryable, status and the payload getters for that code only, freeing
     * every returned string. Does not own the pointer: the caller
     * ({@link AimuxResult#expectAimuxError}) frees the returned error afterwards.
     * A code outside 1–13 / 15–17 is a header/library mismatch →
     * {@link IllegalStateException}.
     */
    static AimuxException fromC(Pointer error, String prefix) {
        AimuxFFI ffi = AimuxFFI.INSTANCE;
        int code = ffi.aimux_error_code(error);
        if (code == AIMUX_OK) {
            throw new IllegalStateException(prefix + "non-null aimux error carries AIMUX_OK");
        }
        String msg = AimuxResult.takeString(ffi.aimux_error_message(error));
        if (msg == null || msg.isEmpty()) {
            msg = "aimux: " + codeName(code);
        }
        msg = prefix + msg;
        AimuxException ex;
        switch (code) {
            case AIMUX_E_API_CALL:
                ex = new APICallError(msg, ffi.aimux_error_status(error), ffi.aimux_error_retry_ms(error),
                    AimuxResult.takeString(ffi.aimux_error_provider_code(error)),
                    AimuxResult.takeString(ffi.aimux_error_provider_message(error)),
                    AimuxResult.takeString(ffi.aimux_error_request_id(error)),
                    AimuxResult.takeString(ffi.aimux_error_response_body(error)));
                break;
            case AIMUX_E_NO_SUCH_MODEL:
                ex = new NoSuchModelError(msg, -1, -1L,
                    AimuxResult.takeString(ffi.aimux_error_model_id(error)),
                    AimuxResult.takeString(ffi.aimux_error_model_type(error)));
                break;
            case AIMUX_E_NO_SUCH_PROVIDER:
                ex = new NoSuchProviderError(msg, -1, -1L,
                    AimuxResult.takeString(ffi.aimux_error_provider_id(error)));
                break;
            case AIMUX_E_NO_SUCH_TOOL:
                ex = new NoSuchToolError(msg, -1, -1L,
                    AimuxResult.takeString(ffi.aimux_error_tool_name(error)),
                    parseStringList(AimuxResult.takeString(ffi.aimux_error_available_tools(error))));
                break;
            case AIMUX_E_INVALID_TOOL_INPUT:
                ex = new InvalidToolInputError(msg, -1, -1L,
                    AimuxResult.takeString(ffi.aimux_error_tool_name(error)),
                    AimuxResult.takeString(ffi.aimux_error_tool_input(error)));
                break;
            case AIMUX_E_TOOL_CALL_REPAIR:
                ex = new ToolCallRepairError(msg, -1, -1L,
                    parseWireJson(AimuxResult.takeString(ffi.aimux_error_original_error(error))));
                break;
            default:
                ex = createByCode(code, msg, -1, -1L);
        }
        ex.retryable = ffi.aimux_error_retryable(error) != 0;
        return ex;
    }

    // The FFI guarantees well-formed JSON in these payloads; a parse failure is
    // a header/library mismatch, same as an unknown code.

    private static List<String> parseStringList(String json) {
        if (json == null) {
            return null;
        }
        try {
            return Types.AimuxJson.MAPPER.readValue(json,
                new com.fasterxml.jackson.core.type.TypeReference<List<String>>() {});
        } catch (com.fasterxml.jackson.core.JsonProcessingException e) {
            throw new IllegalStateException("aimux ffi: invalid error payload JSON: " + e.getOriginalMessage(), e);
        }
    }

    private static JsonNode parseWireJson(String json) {
        if (json == null) {
            return null;
        }
        try {
            return Types.AimuxJson.MAPPER.readTree(json);
        } catch (com.fasterxml.jackson.core.JsonProcessingException e) {
            throw new IllegalStateException("aimux ffi: invalid error payload JSON: " + e.getOriginalMessage(), e);
        }
    }

    /**
     * Local (non-FFI) failure: default status/retryMs = -1.
     */
    public static AimuxException of(int code, String message) {
        return of(code, message, -1, -1L);
    }

    /**
     * Local (non-FFI) failure with explicit status / retry hint.
     */
    public static AimuxException of(int code, String message, int status, long retryMs) {
        String msg = (message == null || message.isEmpty()) ? "aimux: " + codeName(code) : message;
        return createByCode(code, msg, status, retryMs);
    }

    private static AimuxException createByCode(int code, String message, int status, long retryMs) {
        switch (code) {
            case AIMUX_E_JSON_PARSE:
                return new JSONParseError(message, status, retryMs);
            case AIMUX_E_INVALID_RESPONSE_DATA:
                return new InvalidResponseDataError(message, status, retryMs);
            case AIMUX_E_INVALID_ARGUMENT:
                return new InvalidArgumentError(message, status, retryMs);
            case AIMUX_E_INVALID_PROMPT:
                return new InvalidPromptError(message, status, retryMs);
            case AIMUX_E_TOKEN_EXPIRED:
                return new TokenExpiredError(message, status, retryMs);
            case AIMUX_E_UNSUPPORTED_FUNCTIONALITY:
                return new UnsupportedFunctionalityError(message, status, retryMs);
            case AIMUX_E_NO_SUCH_MODEL:
                return new NoSuchModelError(message, status, retryMs);
            case AIMUX_E_NO_SUCH_PROVIDER:
                return new NoSuchProviderError(message, status, retryMs);
            case AIMUX_E_API_CALL:
                return new APICallError(message, status, retryMs);
            case AIMUX_E_TIMEOUT:
                return new TimeoutError(message, status, retryMs);
            case AIMUX_E_ABORTED:
                return new RequestAbortedError(message, status, retryMs);
            case AIMUX_E_NO_SUCH_TOOL:
                return new NoSuchToolError(message, status, retryMs);
            case AIMUX_E_INVALID_TOOL_INPUT:
                return new InvalidToolInputError(message, status, retryMs);
            case AIMUX_E_TOOL_CALL_REPAIR:
                return new ToolCallRepairError(message, status, retryMs);
            case AIMUX_E_OTHER:
                return new OtherError(message, status, retryMs);
            default:
                throw new IllegalStateException("Unknown aimux_error_code_t: " + code);
        }
    }

    /** Core {@code error_type()} name for a code (diagnostics / empty messages). */
    public static String codeName(int code) {
        switch (code) {
            case AIMUX_OK:
                return "OK";
            case AIMUX_E_JSON_PARSE:
                return "JsonParse";
            case AIMUX_E_INVALID_RESPONSE_DATA:
                return "InvalidResponseData";
            case AIMUX_E_INVALID_ARGUMENT:
                return "InvalidArgument";
            case AIMUX_E_INVALID_PROMPT:
                return "InvalidPrompt";
            case AIMUX_E_TOKEN_EXPIRED:
                return "TokenExpired";
            case AIMUX_E_UNSUPPORTED_FUNCTIONALITY:
                return "UnsupportedFunctionality";
            case AIMUX_E_NO_SUCH_MODEL:
                return "NoSuchModel";
            case AIMUX_E_NO_SUCH_PROVIDER:
                return "NoSuchProvider";
            case AIMUX_E_API_CALL:
                return "ApiCall";
            case AIMUX_E_TIMEOUT:
                return "Timeout";
            case AIMUX_E_ABORTED:
                return "Aborted";
            case AIMUX_E_NO_SUCH_TOOL:
                return "NoSuchTool";
            case AIMUX_E_INVALID_TOOL_INPUT:
                return "InvalidToolInput";
            case AIMUX_E_TOOL_CALL_REPAIR:
                return "ToolCallRepair";
            case AIMUX_E_OTHER:
                return "Other";
            default:
                return "Code(" + code + ")";
        }
    }

    // ── Subclasses (mirror Node error.ts / Python hierarchy) ────────────────

    public static class JSONParseError extends AimuxException {
        public JSONParseError(String message, int status, long retryMs) {
            super(message, AIMUX_E_JSON_PARSE, status, retryMs);
        }
    }

    public static class InvalidResponseDataError extends AimuxException {
        public InvalidResponseDataError(String message, int status, long retryMs) {
            super(message, AIMUX_E_INVALID_RESPONSE_DATA, status, retryMs);
        }
    }

    public static class InvalidArgumentError extends AimuxException {
        public InvalidArgumentError(String message, int status, long retryMs) {
            super(message, AIMUX_E_INVALID_ARGUMENT, status, retryMs);
        }
    }

    public static class InvalidPromptError extends AimuxException {
        public InvalidPromptError(String message, int status, long retryMs) {
            super(message, AIMUX_E_INVALID_PROMPT, status, retryMs);
        }
    }

    public static class TokenExpiredError extends AimuxException {
        public TokenExpiredError(String message, int status, long retryMs) {
            super(message, AIMUX_E_TOKEN_EXPIRED, status == -1 ? 401 : status, retryMs);
        }
    }

    public static class UnsupportedFunctionalityError extends AimuxException {
        public UnsupportedFunctionalityError(String message, int status, long retryMs) {
            super(message, AIMUX_E_UNSUPPORTED_FUNCTIONALITY, status, retryMs);
        }
    }

    public static class NoSuchModelError extends AimuxException {
        private final String modelId;
        private final String modelType;

        public NoSuchModelError(String message, int status, long retryMs) {
            this(message, status, retryMs, null, null);
        }

        public NoSuchModelError(String message, int status, long retryMs,
                                String modelId, String modelType) {
            super(message, AIMUX_E_NO_SUCH_MODEL, status, retryMs);
            this.modelId = modelId;
            this.modelType = modelType;
        }

        /** The model id that was asked for, or {@code null} for local failures. */
        public String getModelId() {
            return modelId;
        }

        /** The model type it was asked for as, or {@code null} for local failures. */
        public String getModelType() {
            return modelType;
        }
    }

    public static class NoSuchProviderError extends AimuxException {
        private final String providerId;

        public NoSuchProviderError(String message, int status, long retryMs) {
            this(message, status, retryMs, null);
        }

        public NoSuchProviderError(String message, int status, long retryMs, String providerId) {
            super(message, AIMUX_E_NO_SUCH_PROVIDER, status, retryMs);
            this.providerId = providerId;
        }

        /** The provider id that was asked for, or {@code null} for local failures. */
        public String getProviderId() {
            return providerId;
        }
    }

    /**
     * Every HTTP-shaped failure (AI SDK {@code APICallError} analogue): provider
     * errors, transport failures, 401 auth, 404 model, 429 rate limit. Classify
     * with {@link #getStatusCode()}; {@code -1} means no HTTP response was ever
     * observed — a missing API key, an error built without a request, or a
     * transport failure — and says nothing about whether a retry would help:
     * read {@link #isRetryable()} for that, never the status sentinel.
     */
    public static class APICallError extends AimuxException {
        private final String providerCode;
        private final String providerMessage;
        private final String requestId;
        private final String responseBody;

        public APICallError(String message, int status, long retryMs) {
            this(message, status, retryMs, null, null, null, null);
        }

        public APICallError(String message, int status, long retryMs,
                            String providerCode, String providerMessage, String requestId, String responseBody) {
            super(message, AIMUX_E_API_CALL, status, retryMs);
            this.providerCode = providerCode;
            this.providerMessage = providerMessage;
            this.requestId = requestId;
            this.responseBody = responseBody;
        }

        /** The provider's own error code (e.g. {@code "insufficient_quota"}), or {@code null}. */
        public String getProviderCode() {
            return providerCode;
        }

        /** The failure's own text without the composed prefix {@link #getMessage()} carries (e.g. {@code "slow down"}), or {@code null}. */
        public String getProviderMessage() {
            return providerMessage;
        }

        /** Provider request id (for support tickets), or {@code null}. */
        public String getRequestId() {
            return requestId;
        }

        /** Raw response body, or {@code null}. */
        public String getResponseBody() {
            return responseBody;
        }
    }

    public static class TimeoutError extends AimuxException {
        public TimeoutError(String message, int status, long retryMs) {
            super(message, AIMUX_E_TIMEOUT, status, retryMs);
        }
    }

    /** Request aborted (not a DOM {@code AbortError}). */
    public static class RequestAbortedError extends AimuxException {
        public RequestAbortedError(String message, int status, long retryMs) {
            super(message, AIMUX_E_ABORTED, status, retryMs);
        }

        public RequestAbortedError() {
            this("request aborted", -1, -1L);
        }
    }

    /** The model called a tool that is not in the supplied tool set. */
    public static class NoSuchToolError extends AimuxException {
        private final String toolName;
        private final List<String> availableTools;

        public NoSuchToolError(String message, int status, long retryMs) {
            this(message, status, retryMs, null, null);
        }

        public NoSuchToolError(String message, int status, long retryMs,
                               String toolName, List<String> availableTools) {
            super(message, AIMUX_E_NO_SUCH_TOOL, status, retryMs);
            this.toolName = toolName;
            this.availableTools = availableTools;
        }

        /** The tool name the model called, or {@code null} for local failures. */
        public String getToolName() {
            return toolName;
        }

        /** The available tool names, or {@code null} when no tool set was supplied. */
        public List<String> getAvailableTools() {
            return availableTools;
        }
    }

    /** The model's tool arguments failed to parse or validate against the schema. */
    public static class InvalidToolInputError extends AimuxException {
        private final String toolName;
        private final String toolInput;

        public InvalidToolInputError(String message, int status, long retryMs) {
            this(message, status, retryMs, null, null);
        }

        public InvalidToolInputError(String message, int status, long retryMs,
                                     String toolName, String toolInput) {
            super(message, AIMUX_E_INVALID_TOOL_INPUT, status, retryMs);
            this.toolName = toolName;
            this.toolInput = toolInput;
        }

        /** The tool name the model called, or {@code null} for local failures. */
        public String getToolName() {
            return toolName;
        }

        /** The raw argument text the model produced, or {@code null} for local failures. */
        public String getToolInput() {
            return toolInput;
        }
    }

    /** Tool-call repair itself failed. */
    public static class ToolCallRepairError extends AimuxException {
        private final JsonNode originalError;

        public ToolCallRepairError(String message, int status, long retryMs) {
            this(message, status, retryMs, null);
        }

        public ToolCallRepairError(String message, int status, long retryMs, JsonNode originalError) {
            super(message, AIMUX_E_TOOL_CALL_REPAIR, status, retryMs);
            this.originalError = originalError;
        }

        /**
         * The original lookup/parse/validation error as externally-tagged wire
         * JSON (the same encoding as {@code ToolCall.error}), or {@code null}
         * for local failures.
         */
        public JsonNode getOriginalError() {
            return originalError;
        }
    }

    public static class OtherError extends AimuxException {
        public OtherError(String message, int status, long retryMs) {
            super(message, AIMUX_E_OTHER, status, retryMs);
        }
    }
}
