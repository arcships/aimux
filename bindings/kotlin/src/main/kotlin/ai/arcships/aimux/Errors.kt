package ai.arcships.aimux

/**
 * Machine-readable codes matching aimux-ffi [AimuxErrorCode] (aimux-error.h).
 * Append-only: never renumber existing values. 2..19 mirror core AiMuxError
 * variants in declaration order.
 */
const val AIMUX_OK: Int = 0
const val AIMUX_E_UNKNOWN: Int = 1
const val AIMUX_E_PROVIDER: Int = 2
const val AIMUX_E_HTTP: Int = 3
const val AIMUX_E_JSON: Int = 4
const val AIMUX_E_STREAM: Int = 5
const val AIMUX_E_TOOL: Int = 6
const val AIMUX_E_INVALID_ARGUMENT: Int = 7
const val AIMUX_E_INVALID_PROMPT: Int = 8
const val AIMUX_E_RATE_LIMITED: Int = 9
const val AIMUX_E_AUTH: Int = 10
const val AIMUX_E_TOKEN_EXPIRED: Int = 11
const val AIMUX_E_MODEL_NOT_FOUND: Int = 12
const val AIMUX_E_UNSUPPORTED: Int = 13
const val AIMUX_E_NO_SUCH_MODEL: Int = 14
const val AIMUX_E_UNKNOWN_PROVIDER: Int = 15
const val AIMUX_E_API_CALL: Int = 16
const val AIMUX_E_TIMEOUT: Int = 17
const val AIMUX_E_ABORTED: Int = 18
const val AIMUX_E_OTHER: Int = 19

/**
 * Base exception for all aimux engine / binding failures.
 *
 * Sealed: every subclass lives in this file, so a Kotlin `when` over the
 * hierarchy is exhaustive. Java callers can still `catch (AimuxException e)`.
 * Subclasses mirror Node's hierarchy and C `AimuxErrorCode` 2..19.
 *
 * ```kotlin
 * try {
 *     model.generateText("\"hi\"")
 * } catch (e: RateLimitedError) {
 *     // e.retryMs, e.status == 429
 * } catch (e: AuthenticationError) {
 *     // e.status often 401
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
         * Status defaults for well-known HTTP kinds match Node when C reports -1.
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
            AIMUX_E_PROVIDER -> ProviderError(message, status, retryMs, cause, errorValue)
            AIMUX_E_HTTP -> HttpError(message, status, retryMs, cause, errorValue)
            AIMUX_E_JSON -> JsonError(message, status, retryMs, cause, errorValue)
            AIMUX_E_STREAM -> StreamError(message, status, retryMs, cause, errorValue)
            AIMUX_E_TOOL -> ToolError(message, status, retryMs, cause, errorValue)
            AIMUX_E_INVALID_ARGUMENT -> InvalidArgumentError(message, status, retryMs, cause, errorValue)
            AIMUX_E_INVALID_PROMPT -> InvalidPromptError(message, status, retryMs, cause, errorValue)
            AIMUX_E_RATE_LIMITED -> RateLimitedError(
                message,
                status = if (status == -1) 429 else status,
                retryMs = retryMs,
                cause = cause,
                errorValue = errorValue,
            )
            AIMUX_E_AUTH -> AuthenticationError(
                message,
                status = if (status == -1) 401 else status,
                retryMs = retryMs,
                cause = cause,
                errorValue = errorValue,
            )
            AIMUX_E_TOKEN_EXPIRED -> TokenExpiredError(
                message,
                status = if (status == -1) 401 else status,
                retryMs = retryMs,
                cause = cause,
                errorValue = errorValue,
            )
            AIMUX_E_MODEL_NOT_FOUND -> ModelNotFoundError(
                message,
                status = if (status == -1) 404 else status,
                retryMs = retryMs,
                cause = cause,
                errorValue = errorValue,
            )
            AIMUX_E_UNSUPPORTED -> UnsupportedError(message, status, retryMs, cause, errorValue)
            AIMUX_E_NO_SUCH_MODEL -> NoSuchModelError(message, status, retryMs, cause, errorValue)
            AIMUX_E_UNKNOWN_PROVIDER -> UnknownProviderError(message, status, retryMs, cause, errorValue)
            AIMUX_E_API_CALL -> APICallError(message, status, retryMs, cause, errorValue)
            AIMUX_E_TIMEOUT -> TimeoutError(message, status, retryMs, cause, errorValue)
            AIMUX_E_ABORTED -> RequestAbortedError(message, status, retryMs, cause, errorValue)
            AIMUX_E_OTHER -> OtherError(message, status, retryMs, cause, errorValue)
            AIMUX_OK -> OtherError(message.ifEmpty { "aimux: operation failed" }, status, retryMs, cause, errorValue)
            else -> UnknownAimuxError(message, code, status, retryMs, cause, errorValue)
        }

        /** Core / C code → short name (e.g. `"Auth"`, `"RateLimited"`). */
        @JvmStatic
        fun codeName(code: Int): String = when (code) {
            AIMUX_OK -> "OK"
            AIMUX_E_UNKNOWN -> "Unknown"
            AIMUX_E_PROVIDER -> "Provider"
            AIMUX_E_HTTP -> "Http"
            AIMUX_E_JSON -> "Json"
            AIMUX_E_STREAM -> "Stream"
            AIMUX_E_TOOL -> "Tool"
            AIMUX_E_INVALID_ARGUMENT -> "InvalidArgument"
            AIMUX_E_INVALID_PROMPT -> "InvalidPrompt"
            AIMUX_E_RATE_LIMITED -> "RateLimited"
            AIMUX_E_AUTH -> "Auth"
            AIMUX_E_TOKEN_EXPIRED -> "TokenExpired"
            AIMUX_E_MODEL_NOT_FOUND -> "ModelNotFound"
            AIMUX_E_UNSUPPORTED -> "Unsupported"
            AIMUX_E_NO_SUCH_MODEL -> "NoSuchModel"
            AIMUX_E_UNKNOWN_PROVIDER -> "UnknownProvider"
            AIMUX_E_API_CALL -> "ApiCall"
            AIMUX_E_TIMEOUT -> "Timeout"
            AIMUX_E_ABORTED -> "Aborted"
            AIMUX_E_OTHER -> "Other"
            else -> "Code($code)"
        }
    }
}

class ProviderError(
    message: String,
    status: Int = -1,
    retryMs: Long = -1,
    cause: Throwable? = null,
    errorValue: String? = null,
) : AimuxException(message, AIMUX_E_PROVIDER, status, retryMs, cause, errorValue)

class HttpError(
    message: String,
    status: Int = -1,
    retryMs: Long = -1,
    cause: Throwable? = null,
    errorValue: String? = null,
) : AimuxException(message, AIMUX_E_HTTP, status, retryMs, cause, errorValue)

class JsonError(
    message: String,
    status: Int = -1,
    retryMs: Long = -1,
    cause: Throwable? = null,
    errorValue: String? = null,
) : AimuxException(message, AIMUX_E_JSON, status, retryMs, cause, errorValue)

class StreamError(
    message: String,
    status: Int = -1,
    retryMs: Long = -1,
    cause: Throwable? = null,
    errorValue: String? = null,
) : AimuxException(message, AIMUX_E_STREAM, status, retryMs, cause, errorValue)

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

class RateLimitedError(
    message: String,
    status: Int = 429,
    retryMs: Long = -1,
    cause: Throwable? = null,
    errorValue: String? = null,
) : AimuxException(message, AIMUX_E_RATE_LIMITED, status, retryMs, cause, errorValue)

/** Auth / bad API key (HTTP 401). Named after OpenAI/Anthropic `AuthenticationError`. */
class AuthenticationError(
    message: String,
    status: Int = 401,
    retryMs: Long = -1,
    cause: Throwable? = null,
    errorValue: String? = null,
) : AimuxException(message, AIMUX_E_AUTH, status, retryMs, cause, errorValue)

class TokenExpiredError(
    message: String,
    status: Int = 401,
    retryMs: Long = -1,
    cause: Throwable? = null,
    errorValue: String? = null,
) : AimuxException(message, AIMUX_E_TOKEN_EXPIRED, status, retryMs, cause, errorValue)

class ModelNotFoundError(
    message: String,
    status: Int = 404,
    retryMs: Long = -1,
    cause: Throwable? = null,
    errorValue: String? = null,
) : AimuxException(message, AIMUX_E_MODEL_NOT_FOUND, status, retryMs, cause, errorValue)

class UnsupportedError(
    message: String,
    status: Int = -1,
    retryMs: Long = -1,
    cause: Throwable? = null,
    errorValue: String? = null,
) : AimuxException(message, AIMUX_E_UNSUPPORTED, status, retryMs, cause, errorValue)

class NoSuchModelError(
    message: String,
    status: Int = -1,
    retryMs: Long = -1,
    cause: Throwable? = null,
    errorValue: String? = null,
) : AimuxException(message, AIMUX_E_NO_SUCH_MODEL, status, retryMs, cause, errorValue)

class UnknownProviderError(
    message: String,
    status: Int = -1,
    retryMs: Long = -1,
    cause: Throwable? = null,
    errorValue: String? = null,
) : AimuxException(message, AIMUX_E_UNKNOWN_PROVIDER, status, retryMs, cause, errorValue)

/** Provider HTTP / API call failure (AI SDK `APICallError` analogue). */
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
