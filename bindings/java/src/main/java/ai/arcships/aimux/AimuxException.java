package ai.arcships.aimux;

/**
 * Engine / binding failure hierarchy (OpenAI Java / Vercel AI SDK style).
 *
 * <p>Raised when a fallible C ABI call fails: return sentinel (0 / {@code NULL})
 * with details in {@link AimuxCError}. Prefer {@code instanceof} on subclasses
 * over stringly code checks:
 *
 * <pre>{@code
 * try {
 *     model.generateText("\"hi\"");
 * } catch (APICallError e) {
 *     // Classification is the status field (AI SDK APICallError.statusCode):
 *     // 429 → rate limited (e.getRetryMs()), 401 → auth, 404 → model
 * } catch (AimuxException e) {
 *     // any engine / binding failure
 * }
 * }</pre>
 *
 * <p>Every instance carries {@link #getCode()} (C {@code AimuxErrorCode} 0–14),
 * {@link #getStatusCode()} (HTTP or {@code -1}), and {@link #getRetryMs()} (hint
 * or {@code -1}; {@code 0} = retry now). Message text comes from the C layer.
 *
 * <p>Message-only constructors default {@code status=-1}, {@code retryMs=-1} for
 * backward compatibility with typed-layer decode failures.
 */
public class AimuxException extends RuntimeException {

    private static final long serialVersionUID = 1L;

    // ── AimuxErrorCode (aimux-error.h) ──────────────────────────────────────
    // 14 variant codes (0–14); every HTTP-shaped failure arrives as AIMUX_E_API_CALL.

    public static final int AIMUX_OK = 0;
    public static final int AIMUX_E_UNKNOWN = 1;
    public static final int AIMUX_E_JSON_PARSE = 2;
    public static final int AIMUX_E_INVALID_RESPONSE_DATA = 3;
    public static final int AIMUX_E_TOOL = 4;
    public static final int AIMUX_E_INVALID_ARGUMENT = 5;
    public static final int AIMUX_E_INVALID_PROMPT = 6;
    public static final int AIMUX_E_TOKEN_EXPIRED = 7;
    public static final int AIMUX_E_UNSUPPORTED_FUNCTIONALITY = 8;
    public static final int AIMUX_E_NO_SUCH_MODEL = 9;
    public static final int AIMUX_E_NO_SUCH_PROVIDER = 10;
    public static final int AIMUX_E_API_CALL = 11;
    public static final int AIMUX_E_TIMEOUT = 12;
    public static final int AIMUX_E_ABORTED = 13;
    public static final int AIMUX_E_OTHER = 14;

    private final int code;
    private final int status;
    private final long retryMs;

    // Set once by the fromC construction path; null for local / synthesized
    // failures. Not a constructor param so the 13 subclass constructors keep
    // their public signatures.
    private String errorValue;

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

    /** C {@code AimuxErrorCode} value (0–14). */
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
     * Raw externally-tagged AiMuxError JSON from the engine (e.g.
     * {@code {"ApiCall":{"status_code":429,"retry_after_ms":1500,...}}}), or
     * {@code null} for FFI-synthesized failures (bad args, invalid handles)
     * and local (non-FFI) failures. The binding does no parsing.
     */
    public String getErrorValue() {
        return errorValue;
    }

    // ── Factories ───────────────────────────────────────────────────────────

    /**
     * Build a typed exception from a filled C {@link AimuxCError} out-param.
     * Null or {@link #AIMUX_OK} yields a generic unknown failure.
     */
    public static AimuxException fromC(AimuxCError e) {
        return fromC(e, null);
    }

    /**
     * Build a typed exception from a filled C {@link AimuxCError} out-param,
     * prefixing {@code context} (e.g. a factory description) when present.
     * Consumes (reads and frees) the C-allocated message.
     */
    static AimuxException fromC(AimuxCError e, String context) {
        int code;
        int status;
        long retryMs;
        String msg;
        String errorValue = null;
        if (e == null) {
            code = AIMUX_E_OTHER;
            status = -1;
            retryMs = -1L;
            msg = "aimux: operation failed";
        } else {
            // After a JNA Library call, Structure out-params are already synced
            // native → Java. Do not call e.read() here: that would clobber
            // pure-Java probes (unit tests) that only set fields in Java.
            code = (e.code == AIMUX_OK) ? AIMUX_E_OTHER : e.code;
            status = e.status;
            retryMs = e.retry_ms;
            msg = e.takeMessage();
            errorValue = e.takeErrorValue();
            if (msg.isEmpty()) {
                msg = "aimux: " + codeName(e.code);
            }
        }
        if (context != null && !context.isEmpty()) {
            msg = context + ": " + msg;
        }
        AimuxException ex = createByCode(code, msg, status, retryMs);
        ex.errorValue = errorValue;
        return ex;
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
            case AIMUX_E_TOOL:
                return new ToolError(message, status, retryMs);
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
            case AIMUX_E_OTHER:
                return new OtherError(message, status, retryMs);
            case AIMUX_E_UNKNOWN:
            default:
                // Unknown / out-of-range: base class so callers still catch AimuxException.
                return new AimuxException(message, code, status, retryMs);
        }
    }

    /** Core {@code error_type()} name for a code (diagnostics / empty messages). */
    public static String codeName(int code) {
        switch (code) {
            case AIMUX_OK:
                return "OK";
            case AIMUX_E_UNKNOWN:
                return "Unknown";
            case AIMUX_E_JSON_PARSE:
                return "JsonParse";
            case AIMUX_E_INVALID_RESPONSE_DATA:
                return "InvalidResponseData";
            case AIMUX_E_TOOL:
                return "Tool";
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

    public static class ToolError extends AimuxException {
        public ToolError(String message, int status, long retryMs) {
            super(message, AIMUX_E_TOOL, status, retryMs);
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
        public NoSuchModelError(String message, int status, long retryMs) {
            super(message, AIMUX_E_NO_SUCH_MODEL, status, retryMs);
        }
    }

    public static class NoSuchProviderError extends AimuxException {
        public NoSuchProviderError(String message, int status, long retryMs) {
            super(message, AIMUX_E_NO_SUCH_PROVIDER, status, retryMs);
        }
    }

    /**
     * Every HTTP-shaped failure (AI SDK {@code APICallError} analogue): provider
     * errors, transport failures, 401 auth, 404 model, 429 rate limit. Classify
     * with {@link #getStatusCode()}; {@code -1} means no HTTP response was ever
     * observed — a missing API key, an error built without a request, or a
     * transport failure — and says nothing about whether a retry would help.
     */
    public static class APICallError extends AimuxException {
        public APICallError(String message, int status, long retryMs) {
            super(message, AIMUX_E_API_CALL, status, retryMs);
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

    public static class OtherError extends AimuxException {
        public OtherError(String message, int status, long retryMs) {
            super(message, AIMUX_E_OTHER, status, retryMs);
        }
    }
}
