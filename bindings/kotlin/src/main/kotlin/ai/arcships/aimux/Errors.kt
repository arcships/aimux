package ai.arcships.aimux

import com.sun.jna.Pointer

/**
 * Machine-readable codes matching aimux-ffi `aimux_error_code_t` (aimux-error.h).
 * 1..13 mirror the 13 core variants (1 is the catch-all `Other`). The
 * per-status codes (Provider, Http, RateLimited, Auth, ModelNotFound) are
 * gone, every HTTP-shaped failure arrives as [AIMUX_E_API_CALL].
 */
const val AIMUX_OK: Int = 0
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
const val AIMUX_E_OTHER: Int = 1

/**
 * Base exception for every core `AiMuxError`.
 * Recorder failures are [RecordingException] (no shared base). Codes 200..206
 * surface as plain
 * [IllegalArgumentException] (pre-validation) / [IllegalStateException]
 * (`"aimux ffi: …"`, see [expectAimuxError]).
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
 * Transport: Rust → C `aimux_error_t *` with code 1..13 → [fromC].
 * Primary path is not a JSON
 * error envelope.
 */
sealed class AimuxException(
    message: String,
    val code: Int = AIMUX_E_OTHER,
    val status: Int = -1,
    val retryMs: Long = -1,
    cause: Throwable? = null,
    /**
     * The `AiMuxError` retry verdict: true when retrying may help.
     *
     * Not derivable from [status] — two failures can both report -1 and
     * disagree: a transport failure (the request went out, the connection was
     * reset) is retryable, a missing API key (the request never went out) is
     * not.
     */
    val retryable: Boolean = false,
) : RuntimeException(message, cause) {

    companion object {
        /**
         * Build a typed exception from a returned `const aimux_error_t *`, prefixing
         * [prefix] to the message.
         *
         * Reads code, message and retryable for every code, the payload
         * getters only under their owning code, and frees every returned
         * string. Does not own the pointer: the caller ([expectAimuxError]) frees
         * the returned error afterwards. Code [AIMUX_OK] or a code outside 1..13
         * is a header/library mismatch and throws [IllegalStateException].
         */
        @JvmStatic
        internal fun fromC(error: Pointer, prefix: String = ""): AimuxException {
            val lib = FFI.lib
            val code = lib.aimux_error_code(error)
            check(code != AIMUX_OK) { "${prefix}non-null aimux error carries AIMUX_OK" }
            val msg = prefix + (takeString(lib.aimux_error_message(error))?.ifEmpty { null } ?: "aimux: ${codeName(code)}")
            val retryable = lib.aimux_error_retryable(error) != 0
            return when (code) {
                AIMUX_E_API_CALL -> createByCode(
                    code,
                    msg,
                    lib.aimux_error_status(error),
                    lib.aimux_error_retry_ms(error),
                    retryable = retryable,
                    providerCode = takeString(lib.aimux_error_provider_code(error)),
                    providerMessage = takeString(lib.aimux_error_provider_message(error)),
                    requestId = takeString(lib.aimux_error_request_id(error)),
                    responseBody = takeString(lib.aimux_error_response_body(error)),
                )
                AIMUX_E_NO_SUCH_MODEL -> createByCode(
                    code,
                    msg,
                    retryable = retryable,
                    modelId = takeString(lib.aimux_error_model_id(error)) ?: "",
                    modelType = takeString(lib.aimux_error_model_type(error)) ?: "",
                )
                AIMUX_E_NO_SUCH_PROVIDER -> createByCode(
                    code,
                    msg,
                    retryable = retryable,
                    providerId = takeString(lib.aimux_error_provider_id(error)) ?: "",
                )
                else -> createByCode(code, msg, retryable = retryable)
            }
        }

        /**
         * Build the subclass for a core / C error code (1..13).
         *
         * Any other code — [AIMUX_OK] on a failure path or a code this binding
         * does not know — is a header/library mismatch and throws
         * [IllegalStateException] rather than mapping to an error type.
         */
        @JvmStatic
        fun createByCode(
            code: Int,
            message: String,
            status: Int = -1,
            retryMs: Long = -1,
            cause: Throwable? = null,
            retryable: Boolean = false,
            providerCode: String? = null,
            providerMessage: String? = null,
            requestId: String? = null,
            responseBody: String? = null,
            modelId: String = "",
            modelType: String = "",
            providerId: String = "",
        ): AimuxException = when (code) {
            AIMUX_E_JSON_PARSE -> JSONParseError(message, status, retryMs, cause, retryable)
            AIMUX_E_INVALID_RESPONSE_DATA -> InvalidResponseDataError(message, status, retryMs, cause, retryable)
            AIMUX_E_TOOL -> ToolError(message, status, retryMs, cause, retryable)
            AIMUX_E_INVALID_ARGUMENT -> InvalidArgumentError(message, status, retryMs, cause, retryable)
            AIMUX_E_INVALID_PROMPT -> InvalidPromptError(message, status, retryMs, cause, retryable)
            AIMUX_E_TOKEN_EXPIRED -> TokenExpiredError(
                message,
                status = if (status == -1) 401 else status,
                retryMs = retryMs,
                cause = cause,
                retryable = retryable,
            )
            AIMUX_E_UNSUPPORTED_FUNCTIONALITY -> UnsupportedFunctionalityError(message, status, retryMs, cause, retryable)
            AIMUX_E_NO_SUCH_MODEL -> NoSuchModelError(message, status, retryMs, cause, retryable, modelId, modelType)
            AIMUX_E_NO_SUCH_PROVIDER -> NoSuchProviderError(message, status, retryMs, cause, retryable, providerId)
            AIMUX_E_API_CALL -> APICallError(message, status, retryMs, cause, retryable, providerCode, providerMessage, requestId, responseBody)
            AIMUX_E_TIMEOUT -> TimeoutError(message, status, retryMs, cause, retryable)
            AIMUX_E_ABORTED -> RequestAbortedError(message, status, retryMs, cause, retryable)
            AIMUX_E_OTHER -> OtherError(message, status, retryMs, cause, retryable)
            else -> throw IllegalStateException("Unknown aimux_error_code_t: $code")
        }

        /** Core / C code → short name. */
        @JvmStatic
        fun codeName(code: Int): String = when (code) {
            AIMUX_OK -> "OK"
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
    retryable: Boolean = false,
) : AimuxException(message, AIMUX_E_JSON_PARSE, status, retryMs, cause, retryable)

class InvalidResponseDataError(
    message: String,
    status: Int = -1,
    retryMs: Long = -1,
    cause: Throwable? = null,
    retryable: Boolean = false,
) : AimuxException(message, AIMUX_E_INVALID_RESPONSE_DATA, status, retryMs, cause, retryable)

class ToolError(
    message: String,
    status: Int = -1,
    retryMs: Long = -1,
    cause: Throwable? = null,
    retryable: Boolean = false,
) : AimuxException(message, AIMUX_E_TOOL, status, retryMs, cause, retryable)

class InvalidArgumentError(
    message: String,
    status: Int = -1,
    retryMs: Long = -1,
    cause: Throwable? = null,
    retryable: Boolean = false,
) : AimuxException(message, AIMUX_E_INVALID_ARGUMENT, status, retryMs, cause, retryable)

class InvalidPromptError(
    message: String,
    status: Int = -1,
    retryMs: Long = -1,
    cause: Throwable? = null,
    retryable: Boolean = false,
) : AimuxException(message, AIMUX_E_INVALID_PROMPT, status, retryMs, cause, retryable)

/** Access token expired and must be refreshed (HTTP 401, but retry-after-refresh). */
class TokenExpiredError(
    message: String,
    status: Int = 401,
    retryMs: Long = -1,
    cause: Throwable? = null,
    retryable: Boolean = false,
) : AimuxException(message, AIMUX_E_TOKEN_EXPIRED, status, retryMs, cause, retryable)

class UnsupportedFunctionalityError(
    message: String,
    status: Int = -1,
    retryMs: Long = -1,
    cause: Throwable? = null,
    retryable: Boolean = false,
) : AimuxException(message, AIMUX_E_UNSUPPORTED_FUNCTIONALITY, status, retryMs, cause, retryable)

class NoSuchModelError(
    message: String,
    status: Int = -1,
    retryMs: Long = -1,
    cause: Throwable? = null,
    retryable: Boolean = false,
    /** The model id that was asked for ("" when synthesized locally). */
    val modelId: String = "",
    /** The model type it was asked for as ("" when synthesized locally). */
    val modelType: String = "",
) : AimuxException(message, AIMUX_E_NO_SUCH_MODEL, status, retryMs, cause, retryable)

class NoSuchProviderError(
    message: String,
    status: Int = -1,
    retryMs: Long = -1,
    cause: Throwable? = null,
    retryable: Boolean = false,
    /** The provider id that was asked for ("" when synthesized locally). */
    val providerId: String = "",
) : AimuxException(message, AIMUX_E_NO_SUCH_PROVIDER, status, retryMs, cause, retryable)

/**
 * Every HTTP-shaped provider failure (AI SDK `APICallError` analogue).
 *
 * Classification is [status], not the class: 401 auth, 404 model not found,
 * 429 rate limited (with [retryMs]); -1 means no HTTP response was ever
 * observed — a missing API key, an error built without a request, or a
 * transport failure — and says nothing about whether a retry would help: read
 * [retryable] for that, never the status sentinel.
 * The provider's own detail, when the response carried it: [providerCode],
 * [providerMessage], [requestId], [responseBody] (null when absent or synthesized locally).
 */
class APICallError(
    message: String,
    status: Int = -1,
    retryMs: Long = -1,
    cause: Throwable? = null,
    retryable: Boolean = false,
    /** Provider's own error code, e.g. "insufficient_quota". */
    val providerCode: String? = null,
    /** The failure's own text without the composed prefix [message] carries, e.g. "slow down". */
    val providerMessage: String? = null,
    /** Provider request id, for support tickets. */
    val requestId: String? = null,
    /** Raw response body. */
    val responseBody: String? = null,
) : AimuxException(message, AIMUX_E_API_CALL, status, retryMs, cause, retryable)

class TimeoutError(
    message: String,
    status: Int = -1,
    retryMs: Long = -1,
    cause: Throwable? = null,
    retryable: Boolean = false,
) : AimuxException(message, AIMUX_E_TIMEOUT, status, retryMs, cause, retryable)

/** Request aborted (not a Java interruption). */
class RequestAbortedError(
    message: String = "request aborted",
    status: Int = -1,
    retryMs: Long = -1,
    cause: Throwable? = null,
    retryable: Boolean = false,
) : AimuxException(message, AIMUX_E_ABORTED, status, retryMs, cause, retryable)

class OtherError(
    message: String,
    status: Int = -1,
    retryMs: Long = -1,
    cause: Throwable? = null,
    retryable: Boolean = false,
) : AimuxException(message, AIMUX_E_OTHER, status, retryMs, cause, retryable)

/**
 * Code of a recording failure. Kotlin keeps the six values independent of
 * the C transport codes 100..105.
 * [INIT], [OPEN_FILE] and [SPAWN] are reported by [initRecording];
 * [WRITER_GONE], [FLUSH_TIMEOUT] and [WRITE] by [recordingTryFlush].
 */
enum class RecordingErrorCode {
    INIT, OPEN_FILE, SPAWN, WRITER_GONE, FLUSH_TIMEOUT, WRITE;

    internal companion object {
        fun isCCode(code: Int): Boolean = code in 100..105

        fun fromC(code: Int): RecordingErrorCode = entries.getOrNull(code - 100)
            ?: throw IllegalStateException("Unknown aimux_error_code_t: $code")
    }
}

/**
 * Recording failure reported by [initRecording] and [recordingTryFlush]. Independent of
 * [AimuxException] — the recording subsystem has its own error type in Rust
 * and in the C ABI's 100..105 range, so it does too here.
 */
class RecordingException(val code: RecordingErrorCode, message: String) : RuntimeException(message) {
    internal companion object {
        /**
         * Build from a returned `const aimux_error_t *`. Does not own it: the caller
         * ([expectRecordingError]) frees the returned error.
         */
        fun fromC(error: Pointer, prefix: String = ""): RecordingException {
            val lib = FFI.lib
            val code = RecordingErrorCode.fromC(lib.aimux_error_code(error))
            val msg = takeString(lib.aimux_error_message(error))?.ifEmpty { null }
                ?: "aimux: recording $code"
            return RecordingException(code, prefix + msg)
        }
    }
}
