"""Construct Python exceptions from the canonical ops error envelope.

Headers and retry policy are projected by Rust, never recalculated here.
"""
import re

from ._error_payloads import ErrorEnvelope, payload_fields


class AimuxError(Exception):
    """Base for failures in the aimux domain."""


class RecordingError(Exception):
    """Failure in the recording domain."""


class BindingError(Exception):
    """Failure in the binding domain."""


class ProtocolError(Exception):
    """Failure in the protocol domain."""


class SourceError(Exception):
    """An underlying error that is not an aimux error (OS, TLS, JSON parser)."""


class APICallError(AimuxError): pass
class RetryError(AimuxError): pass
class JSONParseError(AimuxError): pass
class NoOutputGeneratedError(AimuxError): pass
class InvalidResponseDataError(AimuxError): pass
class TypeValidationError(AimuxError): pass
class ToolCallNotFoundForApprovalError(AimuxError): pass
class NoSuchToolError(AimuxError): pass
class InvalidToolInputError(AimuxError): pass
class ToolCallRepairError(AimuxError): pass
class InvalidArgumentError(AimuxError): pass
class InvalidPromptError(AimuxError): pass
class LoadAPIKeyError(AimuxError): pass
class LoadSettingError(AimuxError): pass
class TokenExpiredError(AimuxError): pass
class UnsupportedFunctionalityError(AimuxError): pass
class NoSuchModelError(AimuxError): pass
class NoSuchProviderError(AimuxError): pass
class MissingToolResultsError(AimuxError): pass
class ToolChoiceViolationError(AimuxError): pass
class APITimeoutError(AimuxError): pass
class RequestAbortedError(AimuxError): pass
class OtherError(AimuxError): pass


_AIMUX_TYPES = {
    "AI_APICallError": APICallError,
    "AI_RetryError": RetryError,
    "AI_JSONParseError": JSONParseError,
    "AI_NoOutputGeneratedError": NoOutputGeneratedError,
    "AI_InvalidResponseDataError": InvalidResponseDataError,
    "AI_TypeValidationError": TypeValidationError,
    "AI_ToolCallNotFoundForApprovalError": ToolCallNotFoundForApprovalError,
    "AI_NoSuchToolError": NoSuchToolError,
    "AI_InvalidToolInputError": InvalidToolInputError,
    "AI_ToolCallRepairError": ToolCallRepairError,
    "AI_InvalidArgumentError": InvalidArgumentError,
    "AI_InvalidPromptError": InvalidPromptError,
    "AI_LoadAPIKeyError": LoadAPIKeyError,
    "AI_LoadSettingError": LoadSettingError,
    "AI_TokenExpiredError": TokenExpiredError,
    "AI_UnsupportedFunctionalityError": UnsupportedFunctionalityError,
    "AI_NoSuchModelError": NoSuchModelError,
    "AI_NoSuchProviderError": NoSuchProviderError,
    "AI_MissingToolResultsError": MissingToolResultsError,
    "AI_ToolChoiceViolationError": ToolChoiceViolationError,
    "TimeoutError": APITimeoutError,
    "AbortError": RequestAbortedError,
    "AI_Error": OtherError,
}
_DOMAINS = {
    "aimux": AimuxError, "recording": RecordingError,
    "binding": BindingError, "protocol": ProtocolError,
}


def _snake(name):
    return re.sub(r"(?<!^)(?=[A-Z])", "_", name).lower()


def _source_error(payload):
    """An underlying error `{message, cause?}` and the rest of its chain."""
    error = SourceError(payload.get("message", ""))
    error.error = payload
    cause = payload.get("cause")
    error.cause = _source_error(cause) if isinstance(cause, dict) else None
    error.__cause__ = error.cause
    return error


def _payload_exception(payload, domain="aimux", message=None):
    cls = _AIMUX_TYPES.get(payload.get("name"), AimuxError) if domain == "aimux" else _DOMAINS[domain]
    error = cls(message if message is not None else payload.get("message", payload.get("name", "")))
    error.error = payload
    for field, value in payload_fields(payload, _payload_exception, from_envelope):
        setattr(error, _snake(field), value)
    if isinstance(getattr(error, "cause", None), dict) and "name" not in error.cause:
        error.cause = _source_error(error.cause)
    if isinstance(getattr(error, "cause", None), BaseException):
        error.__cause__ = error.cause
    if isinstance(error, RetryError):
        error.last_error = error.errors[-1] if error.errors else None
        # Rust's `source()` of a retry error is its last attempt.
        error.__cause__ = error.last_error
    return error


def from_envelope(envelope):
    """Preserve the envelope and every payload field, including nested errors."""
    view = ErrorEnvelope.from_json(envelope)
    error = _payload_exception(view.error, view.kind, view.message)
    error.envelope = envelope
    error.kind = view.kind
    error.code = view.code
    error.retry_after_ms = view.retry_after_ms
    return error


__all__ = [cls.__name__ for cls in _AIMUX_TYPES.values()] + [
    "AimuxError", "RecordingError", "BindingError", "ProtocolError", "SourceError",
]
