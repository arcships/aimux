package ai.arcships.aimux;

import com.sun.jna.Pointer;

/**
 * Package-private helpers for FFI result extraction and error mapping.
 *
 * <p>C sentinel returns (0 / NULL) + {@link AimuxCError} out-param
 * → {@link AimuxException#fromC}.
 */
final class AimuxResult {
    private AimuxResult() {}

    /** Allocate a cleared {@link AimuxCError} for one fallible FFI call. */
    static AimuxCError newError() {
        AimuxCError err = new AimuxCError();
        err.clear();
        return err;
    }

    /**
     * Accept a constructor handle, or throw {@link AimuxException} from {@code err}.
     *
     * @param handle  native handle ({@code 0} = failure)
     * @param err     filled by the C callee on failure
     * @param context optional prefix for the thrown message (e.g. factory description)
     * @return the non-zero handle
     */
    static long extractHandle(long handle, AimuxCError err, String context) {
        if (handle == 0L) {
            throw AimuxException.fromC(err, context);
        }
        return handle;
    }

    /**
     * Read a caller-owned UTF-8 string from an FFI return pointer, free it, and
     * throw {@link AimuxException} if the pointer is null (failure).
     *
     * @param ptr     the pointer returned by an {@code aimux_*} function (may be null)
     * @param err     filled by the C callee on failure
     * @param context method name for diagnostics
     * @return the result string (never null)
     */
    static String extractString(Pointer ptr, AimuxCError err, String context) {
        if (ptr == null) {
            throw AimuxException.fromC(err, context);
        }
        try {
            return ptr.getString(0, "UTF-8");
        } finally {
            AimuxFFI.INSTANCE.aimux_free_string(ptr);
        }
    }
}
