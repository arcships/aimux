package ai.arcships.aimux

import com.sun.jna.Pointer
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive

/**
 * Machine-readable codes matching aimux-ffi `aimux_error_code_t` (aimux-error.h).
 * 1..14 mirror the 14 core variants (1 is the catch-all `Other`, 14 is `Retry`).
 * The per-status codes (Provider, Http, RateLimited, Auth,
 * ModelNotFound) are gone, every HTTP-shaped failure arrives as
 * [AIMUX_E_API_CALL].
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
const val AIMUX_E_RETRY: Int = 14

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
 * Transport: Rust → C `aimux_error_t *` with code 1..14 → [fromC].
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
         * the returned error afterwards (retry attempt errors are new owned
         * copies and are freed here). Code [AIMUX_OK] or a code outside
         * 1..14 is a header/library mismatch and throws [IllegalStateException].
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
                    responseBody = takeString(lib.aimux_error_response_body(error)),
                    url = takeString(lib.aimux_error_url(error)),
                    requestBodyValues = parseJson(takeString(lib.aimux_error_request_body_values(error))),
                    responseHeaders = headerMap(takeString(lib.aimux_error_response_headers(error))),
                    data = parseJson(takeString(lib.aimux_error_provider_data(error))),
                )
                AIMUX_E_RETRY -> createByCode(
                    code,
                    msg,
                    retryable = retryable,
                    retryReason = RetryErrorReason.fromWire(takeString(lib.aimux_error_retry_reason(error))),
                    retryErrors = retryHistory(error),
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
         * Decode the per-attempt history of an [AIMUX_E_RETRY] error. Each
         * attempt is a new owned `aimux_error_t *` (index 0 = oldest) that can
         * itself be any AiMuxError — including a nested Retry — and is freed
         * here, independently of the parent.
         */
        private fun retryHistory(error: Pointer): List<AimuxException> {
            val lib = FFI.lib
            return (0 until lib.aimux_error_retry_count(error)).mapNotNull { i ->
                lib.aimux_error_retry_error_at(error, i)?.let { attempt ->
                    try {
                        fromC(attempt)
                    } finally {
                        lib.aimux_error_free(attempt)
                    }
                }
            }
        }

        /** Parse a getter-returned JSON string; null (absent) stays null. */
        private fun parseJson(json: String?): JsonElement? = try {
            json?.let(Json::parseToJsonElement)
        } catch (_: Exception) {
            null
        }

        /** Response headers arrive as one JSON object string of string→string pairs. */
        private fun headerMap(json: String?): Map<String, String>? =
            (parseJson(json) as? JsonObject)?.mapValues { (_, value) ->
                (value as? JsonPrimitive)?.content ?: value.toString()
            }

        /**
         * Build the subclass for a core / C error code (1..14).
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
            responseBody: String? = null,
            url: String? = null,
            requestBodyValues: JsonElement? = null,
            responseHeaders: Map<String, String>? = null,
            data: JsonElement? = null,
            modelId: String = "",
            modelType: String = "",
            providerId: String = "",
            retryReason: RetryErrorReason = RetryErrorReason.MAX_RETRIES_EXCEEDED,
            retryErrors: List<AimuxException> = emptyList(),
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
            AIMUX_E_API_CALL -> APICallError(
                message, status, retryMs, cause, retryable,
                providerCode, providerMessage, responseBody,
                url, requestBodyValues, responseHeaders, data,
            )
            AIMUX_E_RETRY -> RetryError(
                message,
                retryReason,
                // A deserialized Retry may carry no attempts; keep RetryError total.
                retryErrors.ifEmpty { listOf(OtherError(message)) },
                cause,
            )
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
            AIMUX_E_RETRY -> "Retry"
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
 * [providerMessage], [responseBody], [url], [requestBodyValues],
 * [responseHeaders], [data] (null when absent or synthesized locally).
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
    /** Raw response body. */
    val responseBody: String? = null,
    /** Sanitized request URL. */
    val url: String? = null,
    /** Sanitized request body values (any JSON type). */
    val requestBodyValues: JsonElement? = null,
    /** Sanitized response headers (also carry provider request ids). */
    val responseHeaders: Map<String, String>? = null,
    /** Parsed provider error data (AI SDK `APICallError.data`). */
    val data: JsonElement? = null,
) : AimuxException(message, AIMUX_E_API_CALL, status, retryMs, cause, retryable)

/** Why the retry loop gave up ([RetryError.reason]); wire names are the core's serde camelCase. */
enum class RetryErrorReason(val wireValue: String) {
    /** Every permitted attempt failed with a retryable error. */
    MAX_RETRIES_EXCEEDED("maxRetriesExceeded"),

    /** A later attempt failed with a non-retryable error. */
    ERROR_NOT_RETRYABLE("errorNotRetryable");

    companion object {
        fun fromWire(value: String?): RetryErrorReason =
            if (value == ERROR_NOT_RETRYABLE.wireValue) ERROR_NOT_RETRYABLE else MAX_RETRIES_EXCEEDED
    }
}

/**
 * The retry loop gave up (AI SDK `RetryError` analogue): [reason] says why,
 * [errors] is the per-attempt history (oldest first), [lastError] the final
 * attempt.
 */
class RetryError(
    message: String,
    val reason: RetryErrorReason,
    val errors: List<AimuxException>,
    cause: Throwable? = null,
) : AimuxException(message, AIMUX_E_RETRY, -1, -1, cause) {
    init {
        require(errors.isNotEmpty()) { "RetryError requires at least one error" }
    }

    val lastError: AimuxException = errors.last()
}

class TimeoutError(
    message: String,
    status: Int = -1,
    retryMs: Long = -1,
    cause: Throwable? = null,
    retryable: Boolean = false,
) : AimuxException(message, AIMUX_E_TIMEOUT, status, retryMs, cause, retryable)

/** Request aborted (not a Java interruption); the message is the abort payload. */
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
