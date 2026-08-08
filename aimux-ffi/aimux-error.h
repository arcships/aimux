/**
 * aimux-error.h — C error model for aimux-ffi.
 *
 * Transport: fallible calls use return-value sentinels (0 / NULL) for success
 * or failure. Optional details go through AimuxError *err (NULL = discard).
 *
 * Modeling: flat error codes (18 AiMuxError variants + OK + UNKNOWN) and one
 * plain 40-byte struct. Check the function return value first; only then read
 * *err. On failure the callee overwrites every field and allocates `message`;
 * the caller owns it and must release it with aimux_free_string(). On success
 * *err is left untouched.
 */

#ifndef AIMUX_ERROR_H
#define AIMUX_ERROR_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/**
 * Machine-readable codes. Append-only: never renumber existing values.
 * AIMUX_OK / AIMUX_E_UNKNOWN are C-side sentinels; 2..19 mirror aimux-core's
 * 18 AiMuxError variants in declaration order.
 */
typedef enum AimuxErrorCode {
    AIMUX_OK = 0,
    AIMUX_E_UNKNOWN = 1,
    AIMUX_E_PROVIDER = 2,
    AIMUX_E_HTTP = 3,
    AIMUX_E_JSON = 4,
    AIMUX_E_STREAM = 5,
    AIMUX_E_TOOL = 6,
    AIMUX_E_INVALID_ARGUMENT = 7,
    AIMUX_E_INVALID_PROMPT = 8,
    AIMUX_E_RATE_LIMITED = 9,
    AIMUX_E_AUTH = 10,
    AIMUX_E_TOKEN_EXPIRED = 11,
    AIMUX_E_MODEL_NOT_FOUND = 12,
    AIMUX_E_UNSUPPORTED = 13,
    AIMUX_E_NO_SUCH_MODEL = 14,
    AIMUX_E_UNKNOWN_PROVIDER = 15,
    AIMUX_E_API_CALL = 16,
    AIMUX_E_TIMEOUT = 17,
    AIMUX_E_ABORTED = 18,
    AIMUX_E_OTHER = 19
} AimuxErrorCode;

/**
 * Error report filled by aimux-ffi on failure when the caller passed a
 * non-NULL AimuxError *.
 *
 * On failure: code != AIMUX_OK; message is a non-empty NUL-terminated UTF-8
 * string allocated by aimux — release it with aimux_free_string(). status is
 * the HTTP status or -1; retry_ms is the RateLimited hint or -1 (0 = retry
 * now). error_value is the lossless machine-readable form of the source
 * error — the externally-tagged JSON of aimux-core's AiMuxError, e.g.
 * {"RateLimited":{"retry_after_ms":1500,"message":"..."}} — or NULL when the
 * failure was synthesized at the FFI boundary (bad argument, invalid handle)
 * and has no core error value. Release it with aimux_free_string() too.
 *
 * Initialize with aimux_error_clear() (or `= {0}` and set status/retry_ms
 * yourself) before first use so the owned pointers are valid to free.
 */
typedef struct AimuxError {
    AimuxErrorCode code;
    int status;
    int64_t retry_ms;
    char *message;
    char *error_value;
    /**
     * Reserved for future ABI extension. Must be zero; the callee zeroes it
     * on failure. The struct size is part of the caller-allocated ABI and
     * can never change — this slot is the only room left to grow.
     */
    void *reserved[1];
} AimuxError;

/**
 * Reset to OK / no hint / no strings. Does not free previous message /
 * error_value — release those with aimux_free_string() first if they were
 * set by a failed call.
 */
static inline void aimux_error_clear(AimuxError *e) {
    if (!e) {
        return;
    }
    e->code = AIMUX_OK;
    e->status = -1;
    e->retry_ms = -1;
    e->message = 0;
    e->error_value = 0;
    e->reserved[0] = 0;
}

#ifdef __cplusplus
}
#endif

#endif /* AIMUX_ERROR_H */
