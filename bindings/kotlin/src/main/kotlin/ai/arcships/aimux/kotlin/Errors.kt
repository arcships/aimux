package ai.arcships.aimux.kotlin

import ai.arcships.aimux.AimuxException as JavaAimuxException
import ai.arcships.aimux.NativeException
import ai.arcships.aimux.BindingException
import ai.arcships.aimux.ProtocolException
import ai.arcships.aimux.RecordingException

/** Sealed view of Java's decoded envelope. No JSON decoding occurs in Kotlin. */
sealed class AimuxException protected constructor(val javaError: NativeException) :
    RuntimeException(javaError.message, javaError) {
    val code get() = javaError.code
    val kind get() = javaError.kind
    val errorName get() = javaError.errorName
    val retryAfterMs get() = javaError.retryAfterMs
    val error get() = javaError.error
    val envelope get() = javaError.envelope

    companion object {
        @JvmStatic
        fun fromJava(error: NativeException): AimuxException = when (error) {
            is JavaAimuxException.NoOutputGeneratedError -> NoOutputGeneratedError(error)
            is JavaAimuxException.JSONParseError -> JSONParseError(error)
            is JavaAimuxException.InvalidResponseDataError -> InvalidResponseDataError(error)
            is JavaAimuxException.ToolCallNotFoundForApprovalError -> ToolCallNotFoundForApprovalError(error)
            is JavaAimuxException.InvalidArgumentError -> InvalidArgumentError(error)
            is JavaAimuxException.InvalidPromptError -> InvalidPromptError(error)
            is JavaAimuxException.LoadAPIKeyError -> LoadAPIKeyError(error)
            is JavaAimuxException.LoadSettingError -> LoadSettingError(error)
            is JavaAimuxException.TokenExpiredError -> TokenExpiredError(error)
            is JavaAimuxException.UnsupportedFunctionalityError -> UnsupportedFunctionalityError(error)
            is JavaAimuxException.NoSuchModelError -> NoSuchModelError(error)
            is JavaAimuxException.NoSuchProviderError -> NoSuchProviderError(error)
            is JavaAimuxException.APICallError -> APICallError(error)
            is JavaAimuxException.RetryError -> RetryError(error)
            is JavaAimuxException.TimeoutError -> TimeoutError(error)
            is JavaAimuxException.RequestAbortedError -> RequestAbortedError(error)
            is JavaAimuxException.NoSuchToolError -> NoSuchToolError(error)
            is JavaAimuxException.InvalidToolInputError -> InvalidToolInputError(error)
            is JavaAimuxException.ToolCallRepairError -> ToolCallRepairError(error)
            is JavaAimuxException.MissingToolResultsError -> MissingToolResultsError(error)
            is JavaAimuxException.ToolChoiceViolationError -> ToolChoiceViolationError(error)
            is JavaAimuxException.OtherError -> OtherError(error)
            is BindingException -> BindingError(error)
            is ProtocolException -> ProtocolError(error)
            is RecordingException -> RecordingError(error)
            else -> UnknownError(error)
        }
    }
}

class NoOutputGeneratedError internal constructor(val details: JavaAimuxException.NoOutputGeneratedError) : AimuxException(details)
class JSONParseError internal constructor(val details: JavaAimuxException.JSONParseError) : AimuxException(details)
class InvalidResponseDataError internal constructor(val details: JavaAimuxException.InvalidResponseDataError) : AimuxException(details)
class ToolCallNotFoundForApprovalError internal constructor(val details: JavaAimuxException.ToolCallNotFoundForApprovalError) : AimuxException(details)
class InvalidArgumentError internal constructor(val details: JavaAimuxException.InvalidArgumentError) : AimuxException(details)
class InvalidPromptError internal constructor(val details: JavaAimuxException.InvalidPromptError) : AimuxException(details)
class LoadAPIKeyError internal constructor(val details: JavaAimuxException.LoadAPIKeyError) : AimuxException(details)
class LoadSettingError internal constructor(val details: JavaAimuxException.LoadSettingError) : AimuxException(details)
class TokenExpiredError internal constructor(val details: JavaAimuxException.TokenExpiredError) : AimuxException(details)
class UnsupportedFunctionalityError internal constructor(val details: JavaAimuxException.UnsupportedFunctionalityError) : AimuxException(details)
class NoSuchModelError internal constructor(val details: JavaAimuxException.NoSuchModelError) : AimuxException(details)
class NoSuchProviderError internal constructor(val details: JavaAimuxException.NoSuchProviderError) : AimuxException(details)
class APICallError internal constructor(val details: JavaAimuxException.APICallError) : AimuxException(details)
class RetryError internal constructor(val details: JavaAimuxException.RetryError) : AimuxException(details)
class TimeoutError internal constructor(val details: JavaAimuxException.TimeoutError) : AimuxException(details)
class RequestAbortedError internal constructor(val details: JavaAimuxException.RequestAbortedError) : AimuxException(details)
class NoSuchToolError internal constructor(val details: JavaAimuxException.NoSuchToolError) : AimuxException(details)
class InvalidToolInputError internal constructor(val details: JavaAimuxException.InvalidToolInputError) : AimuxException(details)
class ToolCallRepairError internal constructor(val details: JavaAimuxException.ToolCallRepairError) : AimuxException(details)
class MissingToolResultsError internal constructor(val details: JavaAimuxException.MissingToolResultsError) : AimuxException(details)
class ToolChoiceViolationError internal constructor(val details: JavaAimuxException.ToolChoiceViolationError) : AimuxException(details)
class OtherError internal constructor(val details: JavaAimuxException.OtherError) : AimuxException(details)

class BindingError internal constructor(val details: BindingException) : AimuxException(details)
class ProtocolError internal constructor(val details: ProtocolException) : AimuxException(details)
class RecordingError internal constructor(val details: RecordingException) : AimuxException(details)

/** Preserves a future Java subtype without interpreting its open payload. */
class UnknownError internal constructor(error: NativeException) : AimuxException(error)
