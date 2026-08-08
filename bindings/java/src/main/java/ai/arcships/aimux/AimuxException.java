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
 * } catch (RateLimitedError e) {
 *     // e.getRetryMs(), e.getStatusCode() == 429
 * } catch (AuthenticationError e) {
 *     // e.getStatusCode() == 401
 * } catch (AimuxException e) {
 *     // any engine / binding failure
 * }
 * }</pre>
 *
 * <p>Every instance carries {@link #getCode()} (C {@code AimuxErrorCode} 0–19),
 * {@link #getStatusCode()} (HTTP or {@code -1}), and {@link #getRetryMs()} (hint
 * or {@code -1}; {@code 0} = retry now). Message text comes from the C layer.
 *
 * <p>Message-only constructors default {@code status=-1}, {@code retryMs=-1} for
 * backward compatibility with typed-layer decode failures.
 */
public class AimuxException extends RuntimeException {

    private static final long serialVersionUID = 1L;

    // ── AimuxErrorCode (aimux-error.h) — append-only, never renumber ─────────

    public static final int AIMUX_OK = 0;
    public static final int AIMUX_E_UNKNOWN = 1;
    public static final int AIMUX_E_PROVIDER = 2;
    public static final int AIMUX_E_HTTP = 3;
    public static final int AIMUX_E_JSON = 4;
    public static final int AIMUX_E_STREAM = 5;
    public static final int AIMUX_E_TOOL = 6;
    public static final int AIMUX_E_INVALID_ARGUMENT = 7;
    public static final int AIMUX_E_INVALID_PROMPT = 8;
    public static final int AIMUX_E_RATE_LIMITED = 9;
    public static final int AIMUX_E_AUTH = 10;
    public static final int AIMUX_E_TOKEN_EXPIRED = 11;
    public static final int AIMUX_E_MODEL_NOT_FOUND = 12;
    public static final int AIMUX_E_UNSUPPORTED = 13;
    public static final int AIMUX_E_NO_SUCH_MODEL = 14;
    public static final int AIMUX_E_UNKNOWN_PROVIDER = 15;
    public static final int AIMUX_E_API_CALL = 16;
    public static final int AIMUX_E_TIMEOUT = 17;
    public static final int AIMUX_E_ABORTED = 18;
    public static final int AIMUX_E_OTHER = 19;

    private final int code;
    private final int status;
    private final long retryMs;

    // Set once by the fromC construction path; null for local / synthesized
    // failures. Not a constructor param so the 18 subclass constructors keep
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

    /** C {@code AimuxErrorCode} value (0–19). */
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
     * {@code {"RateLimited":{"retry_after_ms":1500,"message":"..."}}}), or
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
            case AIMUX_E_PROVIDER:
                return new ProviderError(message, status, retryMs);
            case AIMUX_E_HTTP:
                return new HttpError(message, status, retryMs);
            case AIMUX_E_JSON:
                return new JsonError(message, status, retryMs);
            case AIMUX_E_STREAM:
                return new StreamError(message, status, retryMs);
            case AIMUX_E_TOOL:
                return new ToolError(message, status, retryMs);
            case AIMUX_E_INVALID_ARGUMENT:
                return new InvalidArgumentError(message, status, retryMs);
            case AIMUX_E_INVALID_PROMPT:
                return new InvalidPromptError(message, status, retryMs);
            case AIMUX_E_RATE_LIMITED:
                return new RateLimitedError(message, status, retryMs);
            case AIMUX_E_AUTH:
                return new AuthenticationError(message, status, retryMs);
            case AIMUX_E_TOKEN_EXPIRED:
                return new TokenExpiredError(message, status, retryMs);
            case AIMUX_E_MODEL_NOT_FOUND:
                return new ModelNotFoundError(message, status, retryMs);
            case AIMUX_E_UNSUPPORTED:
                return new UnsupportedError(message, status, retryMs);
            case AIMUX_E_NO_SUCH_MODEL:
                return new NoSuchModelError(message, status, retryMs);
            case AIMUX_E_UNKNOWN_PROVIDER:
                return new UnknownProviderError(message, status, retryMs);
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

    /** Core {@code error_type()} names, indexed by the dense code values 0–19. */
    private static final String[] NAMES = {
        "OK", "Unknown", "Provider", "Http", "Json", "Stream", "Tool",
        "InvalidArgument", "InvalidPrompt", "RateLimited", "Auth", "TokenExpired",
        "ModelNotFound", "Unsupported", "NoSuchModel", "UnknownProvider",
        "ApiCall", "Timeout", "Aborted", "Other",
    };

    /** Core {@code error_type()} name for a code (diagnostics / empty messages). */
    public static String codeName(int code) {
        return (code >= 0 && code < NAMES.length) ? NAMES[code] : "Code(" + code + ")";
    }

    // ── Subclasses (mirror Node error.ts / Python hierarchy) ────────────────

    public static class ProviderError extends AimuxException {
        public ProviderError(String message, int status, long retryMs) {
            super(message, AIMUX_E_PROVIDER, status, retryMs);
        }
    }

    public static class HttpError extends AimuxException {
        public HttpError(String message, int status, long retryMs) {
            super(message, AIMUX_E_HTTP, status, retryMs);
        }
    }

    public static class JsonError extends AimuxException {
        public JsonError(String message, int status, long retryMs) {
            super(message, AIMUX_E_JSON, status, retryMs);
        }
    }

    public static class StreamError extends AimuxException {
        public StreamError(String message, int status, long retryMs) {
            super(message, AIMUX_E_STREAM, status, retryMs);
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

    public static class RateLimitedError extends AimuxException {
        public RateLimitedError(String message, int status, long retryMs) {
            super(message, AIMUX_E_RATE_LIMITED, status == -1 ? 429 : status, retryMs);
        }
    }

    /** Auth / bad API key (HTTP 401). Named after OpenAI {@code AuthenticationError}. */
    public static class AuthenticationError extends AimuxException {
        public AuthenticationError(String message, int status, long retryMs) {
            super(message, AIMUX_E_AUTH, status == -1 ? 401 : status, retryMs);
        }
    }

    public static class TokenExpiredError extends AimuxException {
        public TokenExpiredError(String message, int status, long retryMs) {
            super(message, AIMUX_E_TOKEN_EXPIRED, status == -1 ? 401 : status, retryMs);
        }
    }

    public static class ModelNotFoundError extends AimuxException {
        public ModelNotFoundError(String message, int status, long retryMs) {
            super(message, AIMUX_E_MODEL_NOT_FOUND, status == -1 ? 404 : status, retryMs);
        }
    }

    public static class UnsupportedError extends AimuxException {
        public UnsupportedError(String message, int status, long retryMs) {
            super(message, AIMUX_E_UNSUPPORTED, status, retryMs);
        }
    }

    public static class NoSuchModelError extends AimuxException {
        public NoSuchModelError(String message, int status, long retryMs) {
            super(message, AIMUX_E_NO_SUCH_MODEL, status, retryMs);
        }
    }

    public static class UnknownProviderError extends AimuxException {
        public UnknownProviderError(String message, int status, long retryMs) {
            super(message, AIMUX_E_UNKNOWN_PROVIDER, status, retryMs);
        }
    }

    /** Provider HTTP / API call failure (AI SDK {@code APICallError} analogue). */
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
