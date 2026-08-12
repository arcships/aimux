package ai.arcships.aimux

/**
 * Machine-readable codes matching aimux-ffi [AimuxErrorCode] (aimux-error.h).
 * 13 variant codes; the per-status codes (Provider, Http, RateLimited, Auth,
 * ModelNotFound) are gone, every HTTP-shaped failure arrives as
 * [AIMUX_E_API_CALL].
 */
const val AIMUX_OK: Int = 0
const val AIMUX_E_UNKNOWN: Int = 1
const val AIMUX_E_JSON_PARSE: Int = 2
const val AIMUX_E_INVALID_RESPONSE_DATA: Int = 3
const val AIMUX_E_TOOL: Int = 4
const val AIMUX_E_INVALID_ARGUMENT: Int = 5
const val AIMUX_E_INVALID_PROMPT: Int = 6
const val AIMUX_E_TOKEN_EXPIRED: Int = 7
const val AIMUX_E_UNSUPPORTED_FUNCTIONALITY: Int = 8
const val AIMUX_E_NO_SUCH_MODEL: Int = 9
const val AIMUX_E_NO_SUCH_PROVIDER: Int = 10
const val AIMUX_E_API_CALL: Int = 11
const val AIMUX_E_TIMEOUT: Int = 12
const val AIMUX_E_ABORTED: Int = 13
const val AIMUX_E_OTHER: Int = 14

/**
 * Base exception for all aimux engine / binding failures.
 *
 * Sealed: every subclass lives in this file, so a Kotlin `when` over the
 * hierarchy is exhaustive. Java callers can still `catch (AimuxException e)`.
 * Subclasses mirror Node's hierarchy — one class per core `AiMuxError` variant.
 *
 * ```kotlin
 * try {
 *     model.generateText("\"hi\"")
 * } catch (e: APICallError) {
 *     // Classification is the status field: 429 → rate limited (e.retryMs),
 *     // 401 → auth, 404 → model not found, -1 → no HTTP response observed
 * } catch (e: AimuxException) {
 *     // e.code, e.status, e.retryMs
 * }
 * ```
 *
 * Transport: Rust → C `AimuxError` → [fromC]. Primary path is not a JSON
 * error envelope.
 */
sealed class AimuxException(
    message: String,
    val code: Int = AIMUX_E_OTHER,
    val status: Int = -1,
    val retryMs: Long = -1,
    cause: Throwable? = null,
    /**
     * Lossless machine-readable source error: the externally-tagged JSON of
     * aimux-core's `AiMuxError` (C `error_value`), or null when the failure
     * was synthesized at the FFI boundary (bad argument, invalid handle).
     */
    val errorValue: String? = null,
) : RuntimeException(message, cause) {

    companion object {
        /**
         * Map a filled C [AimuxCError] into the typed hierarchy.
         *
         * Pure mapping: reads [AimuxCError.message] and [AimuxCError.error_value]
         * but never frees them — the FFI call site owns both allocations (see
         * `throwFromC` in Model.kt).
         * Call after a fallible FFI return of 0 / NULL. If [err] is empty
         * (code == AIMUX_OK), yields a generic unknown failure.
         */
        @JvmStatic
        fun fromC(err: AimuxCError): AimuxException {
            val code = err.code
            var msg = err.message?.getString(0, "UTF-8") ?: ""
            if (msg.isEmpty()) {
                msg = if (code == AIMUX_OK) {
                    "aimux: operation failed"
                } else {
                    "aimux: ${codeName(code)}"
                }
            }
            return createByCode(code, msg, err.status, err.retry_ms, errorValue = err.error_value?.getString(0, "UTF-8"))
        }

        /**
         * Build the subclass for a core / C error code.
         *
         * Codes are consecutive; unknown / future codes land in
         * [UnknownAimuxError] with their raw code preserved.
         */
        @JvmStatic
        fun createByCode(
            code: Int,
            message: String,
            status: Int = -1,
            retryMs: Long = -1,
            cause: Throwable? = null,
            errorValue: String? = null,
        ): AimuxException = when (code) {
            AIMUX_E_JSON_PARSE -> JSONParseError(message, status, retryMs, cause, errorValue)
            AIMUX_E_INVALID_RESPONSE_DATA -> InvalidResponseDataError(message, status, retryMs, cause, errorValue)
            AIMUX_E_TOOL -> ToolError(message, status, retryMs, cause, errorValue)
            AIMUX_E_INVALID_ARGUMENT -> InvalidArgumentError(message, status, retryMs, cause, errorValue)
            AIMUX_E_INVALID_PROMPT -> InvalidPromptError(message, status, retryMs, cause, errorValue)
            AIMUX_E_TOKEN_EXPIRED -> TokenExpiredError(
                message,
                status = if (status == -1) 401 else status,
                retryMs = retryMs,
                cause = cause,
                errorValue = errorValue,
            )
            AIMUX_E_UNSUPPORTED_FUNCTIONALITY -> UnsupportedFunctionalityError(message, status, retryMs, cause, errorValue)
            AIMUX_E_NO_SUCH_MODEL -> NoSuchModelError(message, status, retryMs, cause, errorValue)
            AIMUX_E_NO_SUCH_PROVIDER -> NoSuchProviderError(message, status, retryMs, cause, errorValue)
            AIMUX_E_API_CALL -> APICallError(message, status, retryMs, cause, errorValue)
            AIMUX_E_TIMEOUT -> TimeoutError(message, status, retryMs, cause, errorValue)
            AIMUX_E_ABORTED -> RequestAbortedError(message, status, retryMs, cause, errorValue)
            AIMUX_E_OTHER -> OtherError(message, status, retryMs, cause, errorValue)
            AIMUX_OK -> OtherError(message.ifEmpty { "aimux: operation failed" }, status, retryMs, cause, errorValue)
            else -> UnknownAimuxError(message, code, status, retryMs, cause, errorValue)
        }

        /** Core / C code → short name. */
        @JvmStatic
        fun codeName(code: Int): String = when (code) {
            AIMUX_OK -> "OK"
            AIMUX_E_UNKNOWN -> "Unknown"
            AIMUX_E_JSON_PARSE -> "JsonParse"
            AIMUX_E_INVALID_RESPONSE_DATA -> "InvalidResponseData"
            AIMUX_E_TOOL -> "Tool"
            AIMUX_E_INVALID_ARGUMENT -> "InvalidArgument"
            AIMUX_E_INVALID_PROMPT -> "InvalidPrompt"
            AIMUX_E_TOKEN_EXPIRED -> "TokenExpired"
            AIMUX_E_UNSUPPORTED_FUNCTIONALITY -> "UnsupportedFunctionality"
            AIMUX_E_NO_SUCH_MODEL -> "NoSuchModel"
            AIMUX_E_NO_SUCH_PROVIDER -> "NoSuchProvider"
            AIMUX_E_API_CALL -> "ApiCall"
            AIMUX_E_TIMEOUT -> "Timeout"
            AIMUX_E_ABORTED -> "Aborted"
            AIMUX_E_OTHER -> "Other"
            else -> "Code($code)"
        }
    }
}

class JSONParseError(
    message: String,
    status: Int = -1,
    retryMs: Long = -1,
    cause: Throwable? = null,
    errorValue: String? = null,
) : AimuxException(message, AIMUX_E_JSON_PARSE, status, retryMs, cause, errorValue)

class InvalidResponseDataError(
    message: String,
    status: Int = -1,
    retryMs: Long = -1,
    cause: Throwable? = null,
    errorValue: String? = null,
) : AimuxException(message, AIMUX_E_INVALID_RESPONSE_DATA, status, retryMs, cause, errorValue)

class ToolError(
    message: String,
    status: Int = -1,
    retryMs: Long = -1,
    cause: Throwable? = null,
    errorValue: String? = null,
) : AimuxException(message, AIMUX_E_TOOL, status, retryMs, cause, errorValue)

class InvalidArgumentError(
    message: String,
    status: Int = -1,
    retryMs: Long = -1,
    cause: Throwable? = null,
    errorValue: String? = null,
) : AimuxException(message, AIMUX_E_INVALID_ARGUMENT, status, retryMs, cause, errorValue)

class InvalidPromptError(
    message: String,
    status: Int = -1,
    retryMs: Long = -1,
    cause: Throwable? = null,
    errorValue: String? = null,
) : AimuxException(message, AIMUX_E_INVALID_PROMPT, status, retryMs, cause, errorValue)

/** Access token expired and must be refreshed (HTTP 401, but retry-after-refresh). */
class TokenExpiredError(
    message: String,
    status: Int = 401,
    retryMs: Long = -1,
    cause: Throwable? = null,
    errorValue: String? = null,
) : AimuxException(message, AIMUX_E_TOKEN_EXPIRED, status, retryMs, cause, errorValue)

class UnsupportedFunctionalityError(
    message: String,
    status: Int = -1,
    retryMs: Long = -1,
    cause: Throwable? = null,
    errorValue: String? = null,
) : AimuxException(message, AIMUX_E_UNSUPPORTED_FUNCTIONALITY, status, retryMs, cause, errorValue)

class NoSuchModelError(
    message: String,
    status: Int = -1,
    retryMs: Long = -1,
    cause: Throwable? = null,
    errorValue: String? = null,
) : AimuxException(message, AIMUX_E_NO_SUCH_MODEL, status, retryMs, cause, errorValue)

class NoSuchProviderError(
    message: String,
    status: Int = -1,
    retryMs: Long = -1,
    cause: Throwable? = null,
    errorValue: String? = null,
) : AimuxException(message, AIMUX_E_NO_SUCH_PROVIDER, status, retryMs, cause, errorValue)

/**
 * Every HTTP-shaped provider failure (AI SDK `APICallError` analogue).
 *
 * Classification is [status], not the class: 401 auth, 404 model not found,
 * 429 rate limited (with [retryMs]); -1 means no HTTP response was ever
 * observed — a missing API key, an error built without a request, or a
 * transport failure — and says nothing about whether a retry would help.
 * The full detail — `provider_code`, `response_body`, `request_id`,
 * `is_retryable` — is in [errorValue].
 */
class APICallError(
    message: String,
    status: Int = -1,
    retryMs: Long = -1,
    cause: Throwable? = null,
    errorValue: String? = null,
) : AimuxException(message, AIMUX_E_API_CALL, status, retryMs, cause, errorValue)

class TimeoutError(
    message: String,
    status: Int = -1,
    retryMs: Long = -1,
    cause: Throwable? = null,
    errorValue: String? = null,
) : AimuxException(message, AIMUX_E_TIMEOUT, status, retryMs, cause, errorValue)

/** Request aborted (not a Java interruption). */
class RequestAbortedError(
    message: String = "request aborted",
    status: Int = -1,
    retryMs: Long = -1,
    cause: Throwable? = null,
    errorValue: String? = null,
) : AimuxException(message, AIMUX_E_ABORTED, status, retryMs, cause, errorValue)

class OtherError(
    message: String,
    status: Int = -1,
    retryMs: Long = -1,
    cause: Throwable? = null,
    errorValue: String? = null,
) : AimuxException(message, AIMUX_E_OTHER, status, retryMs, cause, errorValue)

/** `AIMUX_E_UNKNOWN` or an unrecognized / future code; preserves the raw [code]. */
class UnknownAimuxError(
    message: String,
    code: Int = AIMUX_E_UNKNOWN,
    status: Int = -1,
    retryMs: Long = -1,
    cause: Throwable? = null,
    errorValue: String? = null,
) : AimuxException(message, code, status, retryMs, cause, errorValue)
