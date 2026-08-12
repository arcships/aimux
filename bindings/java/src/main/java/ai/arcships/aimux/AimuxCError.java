package ai.arcships.aimux;

import com.sun.jna.Pointer;
import com.sun.jna.Structure;

/**
 * JNA mirror of C {@code AimuxError} (see {@code aimux-ffi/aimux-error.h}).
 *
 * <p>Fallible C calls take a trailing {@code AimuxError *err} (may be null on
 * the C side). On failure the callee fills {@code *err} and allocates
 * {@code message} (a NUL-terminated UTF-8 string owned by the caller — release
 * with {@code aimux_free_string}). On success {@code *err} is left untouched.
 *
 * <p>Field layout matches the 40-byte C struct: {@code code}, {@code status},
 * {@code retry_ms}, {@code message}, {@code error_value}, {@code retryable},
 * {@code reserved} (the last two 32-bit ints split what was one reserved
 * pointer slot; {@code reserved} is always zero).
 */
@Structure.FieldOrder({"code", "status", "retry_ms", "message", "error_value", "retryable", "reserved"})
public class AimuxCError extends Structure {

    /** {@link AimuxException} code constant (0 = OK). */
    public int code;

    /** HTTP status, or {@code -1} if none. */
    public int status;

    /** Rate-limit hint in ms, or {@code -1}; {@code 0} means retry immediately. */
    public long retry_ms;

    /** Callee-allocated NUL-terminated UTF-8 message; null when no failure. */
    public Pointer message;

    /**
     * Callee-allocated NUL-terminated UTF-8 externally-tagged AiMuxError JSON
     * (e.g. {@code {"ApiCall":{"status_code":429,"retry_after_ms":1500,...}}}),
     * or null for FFI-synthesized failures (bad args, invalid handles).
     */
    public Pointer error_value;

    /**
     * The engine's retry verdict: {@code 1} when retrying may help, {@code 0}
     * when it will not. Written by the callee on every failure, and <em>not</em>
     * derivable from {@link #status}: a statusless API-call failure is a
     * retryable transport error when the request went out, and a non-retryable
     * missing API key when it never did.
     */
    public int retryable;

    /** Reserved for future ABI extension; always zero. */
    public int reserved;

    public AimuxCError() {
        super();
    }

    /**
     * Reset to OK / no hint / no message (mirrors C {@code aimux_error_clear}).
     * Call before every fallible FFI invocation when reusing an instance.
     */
    public void clear() {
        code = AimuxException.AIMUX_OK;
        status = -1;
        retry_ms = -1L;
        message = null;
        error_value = null;
        retryable = 0;
        reserved = 0;
    }

    /**
     * Read the callee-allocated message and free it exactly once via
     * {@code aimux_free_string}. Returns {@code ""} when no message was set.
     */
    String takeMessage() {
        Pointer p = message;
        message = null;
        if (p == null) {
            return "";
        }
        try {
            return p.getString(0, "UTF-8");
        } finally {
            AimuxFFI.INSTANCE.aimux_free_string(p);
        }
    }

    /**
     * Read the callee-allocated error_value JSON and free it exactly once via
     * {@code aimux_free_string}. Returns {@code null} when absent (it is
     * semantically optional, unlike {@link #takeMessage()} which returns "").
     */
    String takeErrorValue() {
        Pointer p = error_value;
        error_value = null;
        if (p == null) {
            return null;
        }
        try {
            return p.getString(0, "UTF-8");
        } finally {
            AimuxFFI.INSTANCE.aimux_free_string(p);
        }
    }
}
