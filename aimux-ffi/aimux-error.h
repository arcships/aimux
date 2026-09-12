/**
 * aimux-error.h — C error model for aimux-ffi.
 *
 * Every fallible function returns `aimux_error_t *`: NULL on success and an
 * owned error on failure. The normal result is written to the function's
 * trailing out-parameter, which remains at its documented sentinel on failure.
 *
 * Every non-NULL error has one non-zero `aimux_error_code_t` and one message.
 * Codes 1..14 come from `AiMuxError`, 100..105 from
 * `RecordingError`, and 200..206 identify failures detected while crossing
 * the C ABI. Higher-level bindings reconstruct their native error types from
 * that code; they map all 200..206 codes to the language's existing
 * argument/state/invariant error.
 *
 * Strings returned by getters are owned by the caller and must be released
 * with `aimux_free_string()`. Release the error itself exactly once with
 * `aimux_error_free()`; passing NULL to either release function is safe.
 */

#ifndef AIMUX_ERROR_H
#define AIMUX_ERROR_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/** An error returned by a failed Aimux C ABI invocation. Opaque and owned. */
typedef struct aimux_error aimux_error_t;

/**
 * Stable machine-readable code returned by `aimux_error_code()`.
 *
 * Existing values are never renumbered or reused. New values are appended
 * within a reserved range or added in a new range. A non-NULL error must never
 * report AIMUX_OK; an unknown non-zero value means the header and loaded
 * library are incompatible.
 */
typedef enum aimux_error_code {
    AIMUX_OK = 0,

    /* AiMuxError: 1..14. */
    AIMUX_E_OTHER = 1,
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
    /* Reclaims the slot the pre-unification `Other` vacated: the opaque-pointer
     * ABI break means no pre-unification caller can link, so nothing can
     * misread it. */
    AIMUX_E_RETRY = 14,

    /* RecordingError: 100..105. */
    AIMUX_E_RECORDING_INIT = 100,
    AIMUX_E_RECORDING_OPEN_FILE = 101,
    AIMUX_E_RECORDING_SPAWN = 102,
    AIMUX_E_RECORDING_WRITER_GONE = 103,
    AIMUX_E_RECORDING_FLUSH_TIMEOUT = 104,
    AIMUX_E_RECORDING_WRITE = 105,

    /* Failures detected while crossing the C ABI: 200..206. */
    AIMUX_E_FFI_NULL_POINTER = 200,
    AIMUX_E_FFI_INVALID_UTF8 = 201,
    AIMUX_E_FFI_INVALID_WIRE_JSON = 202,
    AIMUX_E_FFI_INVALID_HANDLE = 203,
    AIMUX_E_FFI_REENTRANT_CALL = 204,
    AIMUX_E_FFI_RESULT_SERIALIZATION = 205,
    AIMUX_E_FFI_CALLBACK_FAILURE = 206
} aimux_error_code_t;

/** Release an error. NULL-safe; call exactly once for a non-NULL error. */
void aimux_error_free(aimux_error_t *error);

/** AIMUX_OK for NULL; otherwise the error's single non-zero code. */
int32_t aimux_error_code(const aimux_error_t *error);

/** Human-readable description for every code; caller frees the result. */
char *aimux_error_message(const aimux_error_t *error);

/*
 * AiMuxError facts. These getters answer only for the documented AiMuxError
 * code and return NULL / -1 / 0 for every RecordingError, C ABI failure,
 * unrelated AiMuxError code, or NULL.
 */

/** 1 when retrying may help; only AIMUX_E_API_CALL can answer 1. */
int32_t aimux_error_retryable(const aimux_error_t *error);

/**
 * Observed HTTP status for AIMUX_E_API_CALL; 401 for
 * AIMUX_E_TOKEN_EXPIRED; -1 otherwise or when no response was observed.
 */
int32_t aimux_error_status(const aimux_error_t *error);

/* AIMUX_E_API_CALL — returned strings are caller-owned. */

/** Retry hint in milliseconds (0 = retry now), or -1 when absent. */
int64_t aimux_error_retry_ms(const aimux_error_t *error);
/** Provider's own error code, e.g. "insufficient_quota". */
char *aimux_error_provider_code(const aimux_error_t *error);
/** Failure text without Aimux's composed prefix. */
char *aimux_error_provider_message(const aimux_error_t *error);
/** Raw provider response body. */
char *aimux_error_response_body(const aimux_error_t *error);
/** Sanitized request URL of the failed call. */
char *aimux_error_url(const aimux_error_t *error);
/** Sanitized request body values, as a JSON string. */
char *aimux_error_request_body_values(const aimux_error_t *error);
/** Sanitized response headers, as one JSON object string of
 *  name → value pairs, e.g. {"retry-after-ms":"1500"}. */
char *aimux_error_response_headers(const aimux_error_t *error);
/** Parsed provider error data, as a JSON string. */
char *aimux_error_provider_data(const aimux_error_t *error);

/* AIMUX_E_NO_SUCH_MODEL — returned strings are caller-owned. */

char *aimux_error_model_id(const aimux_error_t *error);
char *aimux_error_model_type(const aimux_error_t *error);

/* AIMUX_E_NO_SUCH_PROVIDER — returned string is caller-owned. */

char *aimux_error_provider_id(const aimux_error_t *error);

/*
 * AIMUX_E_RETRY — retrying stopped; the error keeps the per-attempt history.
 * `aimux_error_message()` composes the summary ("Failed after N attempts…").
 */

/** Why retrying stopped: "maxRetriesExceeded" (every permitted attempt
 *  failed with a retryable error) or "errorNotRetryable" (a later attempt
 *  failed with a non-retryable error). Caller-owned string. */
char *aimux_error_retry_reason(const aimux_error_t *error);
/** Number of recorded attempt errors; 0 under any other code or for NULL. */
int32_t aimux_error_retry_count(const aimux_error_t *error);
/** The attempt error at `index` (0-based, oldest first; the last entry is
 *  the final attempt) as a NEW owned error — read it with these same getters
 *  and release it with aimux_error_free(), independently of the parent.
 *  NULL when `index` is out of range or under any other code. */
aimux_error_t *aimux_error_retry_error_at(const aimux_error_t *error,
                                          int32_t index);

#ifdef __cplusplus
}
#endif

#endif /* AIMUX_ERROR_H */
