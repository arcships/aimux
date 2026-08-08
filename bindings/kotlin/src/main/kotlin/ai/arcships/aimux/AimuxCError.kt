package ai.arcships.aimux

import com.sun.jna.Pointer
import com.sun.jna.Structure

/**
 * JNA mirror of C `AimuxError` (aimux-error.h): 40 bytes
 * (int32 code, int32 status, int64 retry_ms, char *message, char *error_value,
 * and one reserved pointer slot for future ABI extension — always zero).
 *
 * Pass as the trailing `err` argument to fallible FFI calls. On failure the
 * callee fills every field and allocates [message] (NUL-terminated UTF-8) and,
 * for errors originating in aimux-core, [error_value] — the lossless
 * externally-tagged AiMuxError JSON (NULL for failures synthesized at the FFI
 * boundary, e.g. bad arguments or invalid handles). The caller owns both and
 * must release each with `aimux_free_string` exactly once. On success `*err`
 * is left untouched. Check the function return first (0 / NULL), then read
 * this. Field defaults mirror `aimux_error_clear`.
 */
@Structure.FieldOrder("code", "status", "retry_ms", "message", "error_value", "reserved0")
open class AimuxCError : Structure() {
    @JvmField var code: Int = 0
    @JvmField var status: Int = -1
    @JvmField var retry_ms: Long = -1
    @JvmField var message: Pointer? = null
    @JvmField var error_value: Pointer? = null
    @JvmField var reserved0: Pointer? = null
}
