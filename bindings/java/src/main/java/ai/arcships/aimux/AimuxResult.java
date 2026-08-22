package ai.arcships.aimux;

import com.sun.jna.Pointer;
import com.sun.jna.ptr.LongByReference;
import com.sun.jna.ptr.PointerByReference;

/**
 * Package-private helpers for FFI result extraction and error decoding.
 *
 * <p>Every fallible C call returns an {@code aimux_error_t *} ({@code null}
 * = success, result in the out-parameter). Its code identifies an AiMuxError
 * (1–13), RecordingError (100–105), or a failure detected by the C ABI
 * (200–206). The last range collapses to {@link IllegalStateException}
 * ({@code "aimux ffi: "} + message); Java does not expose seven additional
 * exception types. Each helper frees the pointer exactly once. User-triggerable
 * C ABI failures are caught before the C call: {@link #requireJson} for
 * raw JSON ({@link IllegalArgumentException}), local closed-guards for
 * use-after-close ({@link IllegalStateException}).
 */
final class AimuxResult {
    private static final int FFI_FIRST = 200;
    private static final int FFI_LAST = 206;

    private AimuxResult() {}

    /**
     * Validate an optional caller-supplied raw JSON string before it crosses
     * the C ABI, so malformed input fails as
     * {@link IllegalArgumentException} naming the parameter. {@code null} and
     * {@code ""} pass (the C layer treats both as defaults for opts/config).
     */
    static void requireJson(String value, String name) {
        if (value == null || value.isEmpty()) {
            return;
        }
        checkJson(value, name);
    }

    /**
     * Validate a required raw JSON string: {@code null} →
     * {@link NullPointerException}, blank → {@link IllegalArgumentException},
     * otherwise as {@link #requireJson}.
     */
    static void requireJsonNonNull(String value, String name) {
        java.util.Objects.requireNonNull(value, name);
        if (value.trim().isEmpty()) {
            throw new IllegalArgumentException(name + ": invalid JSON: empty");
        }
        checkJson(value, name);
    }

    private static void checkJson(String value, String name) {
        try {
            Types.AimuxJson.MAPPER.reader()
                .with(com.fasterxml.jackson.databind.DeserializationFeature.FAIL_ON_TRAILING_TOKENS)
                .readTree(value);
        } catch (com.fasterxml.jackson.core.JsonProcessingException e) {
            throw new IllegalArgumentException(name + ": invalid JSON: " + e.getOriginalMessage(), e);
        }
    }

    private static String prefix(String context) {
        return (context == null || context.isEmpty()) ? "" : context + ": ";
    }

    /**
     * Decode an error from a call that may return {@code AiMuxError}: 1–13 →
     * {@link AimuxException}; 200–206 → {@link IllegalStateException}.
     * Frees {@code e}.
     */
    static RuntimeException expectAimuxError(Pointer e, String context) {
        String prefix = prefix(context);
        if (e == null) {
            throw new IllegalStateException(prefix + "missing aimux error");
        }
        AimuxFFI ffi = AimuxFFI.INSTANCE;
        try {
            int code = ffi.aimux_error_code(e);
            if (isFfiCode(code)) {
                return ffiError(e, prefix);
            }
            if (code < AimuxException.AIMUX_E_OTHER || code > AimuxException.AIMUX_E_ABORTED) {
                return codeMismatch(code, prefix, "AiMuxError");
            }
            return AimuxException.fromC(e, prefix);
        } finally {
            ffi.aimux_error_free(e);
        }
    }

    /**
     * Decode an error from a recording call: 100–105 →
     * {@link RecordingException}; 200–206 → {@link IllegalStateException}.
     * Frees {@code e}.
     */
    static RuntimeException expectRecordingError(Pointer e, String context) {
        String prefix = prefix(context);
        if (e == null) {
            throw new IllegalStateException(prefix + "missing recording error");
        }
        AimuxFFI ffi = AimuxFFI.INSTANCE;
        try {
            int code = ffi.aimux_error_code(e);
            if (isFfiCode(code)) {
                return ffiError(e, prefix);
            }
            if (!RecordingErrorCode.isCCode(code)) {
                return codeMismatch(code, prefix, "RecordingError");
            }
            return RecordingException.fromC(e, prefix);
        } finally {
            ffi.aimux_error_free(e);
        }
    }

    /**
     * Decode an error from a call that only exposes C ABI failures:
     * message → {@link IllegalStateException}. Deletes {@code e}.
     */
    static RuntimeException expectFfiError(Pointer e, String context) {
        String prefix = prefix(context);
        if (e == null) {
            throw new IllegalStateException(prefix + "missing aimux error");
        }
        try {
            int code = AimuxFFI.INSTANCE.aimux_error_code(e);
            if (!isFfiCode(code)) {
                return codeMismatch(code, prefix, "C ABI failure");
            }
            return ffiError(e, prefix);
        } finally {
            AimuxFFI.INSTANCE.aimux_error_free(e);
        }
    }

    private static boolean isFfiCode(int code) {
        return code >= FFI_FIRST && code <= FFI_LAST;
    }

    private static IllegalStateException codeMismatch(int code, String prefix, String expected) {
        return new IllegalStateException(prefix + "aimux ffi: expected " + expected + " code, got " + code);
    }

    /** Read the returned error's message; does not free. */
    private static IllegalStateException ffiError(Pointer e, String prefix) {
        String msg = takeString(AimuxFFI.INSTANCE.aimux_error_message(e));
        if (msg == null || msg.isEmpty()) {
            msg = "C ABI failure";
        }
        return new IllegalStateException(prefix + "aimux ffi: " + msg);
    }

    /** Copy an owned C string and free it; {@code null} stays {@code null}. */
    static String takeString(Pointer p) {
        if (p == null) {
            return null;
        }
        try {
            return p.getString(0, "UTF-8");
        } finally {
            AimuxFFI.INSTANCE.aimux_free_string(p);
        }
    }

    /**
     * Accept a constructor result: {@code e == null} → the handle written to
     * {@code out}; otherwise throw {@link #expectAimuxError}.
     */
    static long extractHandle(Pointer e, LongByReference out, String context) {
        if (e != null) {
            throw expectAimuxError(e, context);
        }
        return out.getValue();
    }

    /**
     * Accept a JSON-result call: {@code e == null} → the caller-owned UTF-8
     * string written to {@code out} (freed here); otherwise throw
     * {@link #expectAimuxError}.
     */
    static String extractString(Pointer e, PointerByReference out, String context) {
        if (e != null) {
            throw expectAimuxError(e, context);
        }
        String s = takeString(out.getValue());
        if (s == null) {
            throw new IllegalStateException(prefix(context) + "aimux ffi: success with NULL result");
        }
        return s;
    }
}
