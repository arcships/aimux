/**
 * aimux-error.h — C error model for aimux-ffi.
 *
 * Transport: fallible calls use return-value sentinels (0 / NULL) for success
 * or failure. Optional details go through AimuxError *err (NULL = discard).
 *
 * Modeling: flat error codes (13 AiMuxError variants + OK + UNKNOWN) and one
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
 * Machine-readable codes. Values 2..14 map to aimux-core's 13 current
 * AiMuxError variants, numbered consecutively; 1 (UNKNOWN) is the
 * FFI-only fallback for errors with no core variant.
 */
typedef enum AimuxErrorCode {
    AIMUX_OK = 0,
    AIMUX_E_UNKNOWN = 1,
    AIMUX_E_JSON_PARSE = 2,
    AIMUX_E_INVALID_RESPONSE_DATA = 3,
    AIMUX_E_TOOL = 4,
    AIMUX_E_INVALID_ARGUMENT = 5,
    AIMUX_E_INVALID_PROMPT = 6,
    AIMUX_E_TOKEN_EXPIRED = 7,
    AIMUX_E_UNSUPPORTED_FUNCTIONALITY = 8,
    AIMUX_E_NO_SUCH_MODEL = 9,
    AIMUX_E_NO_SUCH_PROVIDER = 10,
    AIMUX_E_API_CALL = 11,
    AIMUX_E_TIMEOUT = 12,
    AIMUX_E_ABORTED = 13,
    AIMUX_E_OTHER = 14
} AimuxErrorCode;

/**
 * Error report filled by aimux-ffi on failure when the caller passed a
 * non-NULL AimuxError *.
 *
 * On failure: code != AIMUX_OK; message is a non-empty NUL-terminated UTF-8
 * string allocated by aimux — release it with aimux_free_string(). status is
 * the HTTP status or -1; retry_ms is the retry hint or -1 (0 = retry
 * now). error_value is the lossless machine-readable form of the source
 * error — the externally-tagged JSON of aimux-core's AiMuxError, e.g.
 * {"ApiCall":{"status_code":401,"message":"..."}} — or NULL when the
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
     * 1 when retrying may help, 0 when it will not — the core's retry
     * verdict, stored at construction. Do NOT infer it from `status`: a
     * statusless AIMUX_E_API_CALL is a transport failure (retryable) when the
     * request went out, and a missing API key (not retryable) when it never
     * did. The callee always writes this field.
     */
    int retryable;
    /**
     * Reserved for future ABI extension. Must be zero; the callee zeroes it
     * on failure. The struct size is part of the caller-allocated ABI and
     * can never change — this is the room left to grow.
     */
    int reserved;
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
    e->retryable = 0;
    e->reserved = 0;
}

#ifdef __cplusplus
}
#endif

#endif /* AIMUX_ERROR_H */
