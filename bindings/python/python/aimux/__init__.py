"""Python ownership and JSON adapters for RFC-0039 operations.

Operation names and DTOs are defined by the shared runtime manifest. All calls
are synchronous; use the caller's executor when integrating with asyncio.
"""
import json
import weakref

from . import ops
from .aimux import (
    ABI_VERSION, OPS_VERSION, EXPECTED_ABI_VERSION, EXPECTED_OPS_VERSION,
    EXPECTED_MANIFEST_DIGEST, Abort, Client as _NativeClient, StreamIterator,
    manifest_digest, manifest_json,
)
from ._errors import *
from ._errors import __all__ as _error_names, from_envelope as _from_envelope


def verify_contract(expected_digest=EXPECTED_MANIFEST_DIGEST):
    """Check runtime version and, when supplied, the consumer's pinned digest."""
    description = json.loads(manifest_json())
    if ABI_VERSION != EXPECTED_ABI_VERSION or OPS_VERSION != EXPECTED_OPS_VERSION:
        raise ImportError("aimux ABI/ops version differs from the Python binding")
    if description["abiVersion"] != ABI_VERSION or description["opsVersion"] != OPS_VERSION:
        raise ImportError("aimux manifest version differs from the native module")
    if expected_digest is not None and manifest_digest() != expected_digest:
        raise ImportError("aimux manifest digest differs from the consumer's pinned contract")
    return description


manifest = verify_contract()


def _wire(value):
    if isinstance(value, Handle):
        return value.id
    if isinstance(value, dict):
        return {key: _wire(item) for key, item in value.items()}
    if isinstance(value, (list, tuple)):
        return [_wire(item) for item in value]
    return value


def _encode(params):
    try:
        return json.dumps(_wire({} if params is None else params), allow_nan=False)
    except (TypeError, ValueError, RecursionError) as error:
        raise _from_envelope({
            "kind": "binding", "code": 202, "message": str(error),
            "error": {"name": "InvalidWireJson", "argument": "params", "message": str(error)},
        }) from error


def _text(value, argument):
    if not isinstance(value, str):
        raise _from_envelope({
            "kind": "binding", "code": 202, "message": argument + " must be a string",
            "error": {"name": "InvalidWireJson", "argument": argument, "message": "expected string"},
        })
    try:
        value.encode("utf-8")
    except UnicodeError as error:
        raise _from_envelope({
            "kind": "binding", "code": 201, "message": str(error),
            "error": {"name": "InvalidUtf8", "argument": argument},
        }) from error
    return value


def _abort(value):
    if value is not None and not isinstance(value, Abort):
        raise _from_envelope({
            "kind": "binding", "code": 202, "message": "abort must be an Abort object",
            "error": {"name": "InvalidWireJson", "argument": "abort", "message": "expected Abort"},
        })
    return value


class Client:
    """An interpreter-scoped client; no host callbacks or retry policy."""

    def __init__(self):
        self._native = _NativeClient()

    def call(self, op, params=None, *, handle="0", abort=None):
        return json.loads(self._native.call(
            _text(_wire(handle), "handle"), _text(op, "op"), _encode(params), _abort(abort),
        ))

    def stream(self, op, params=None, *, handle, abort=None):
        return EventStream(self._native.stream(
            _text(_wire(handle), "handle"), _text(op, "op"), _encode(params), _abort(abort),
        ))

    def operation_push(self, handle, data):
        """Admit one raw audio chunk; a full queue raises ResourceLimit."""
        if not isinstance(data, (bytes, bytearray, memoryview)):
            raise _from_envelope({
                "kind": "binding", "code": 202, "message": "data must be bytes-like",
                "error": {"name": "InvalidWireJson", "argument": "data", "message": "expected bytes"},
            })
        size = data.nbytes if isinstance(data, memoryview) else len(data)
        if size > 1024 * 1024:
            raise _from_envelope({
                "kind": "protocol", "code": 305, "message": "audio chunk exceeds 1 MiB",
                "error": {"name": "ResourceLimit", "message": "audio chunk exceeds 1 MiB"},
            })
        self._native.operation_push(_text(_wire(handle), "handle"), bytes(data))

    def adopt(self, handle):
        """Take responsibility for a returned decimal-string handle."""
        return Handle(self, handle)


def _release(client, handle):
    try:
        client._native.drop_later(handle)
    except Exception:
        # GC is best effort. Explicit close propagates teardown errors.
        pass


class Handle:
    """Own one runtime handle. Use close or a context manager for teardown."""

    def __init__(self, client, handle):
        if not isinstance(handle, str) or not handle.isascii() or not handle.isdecimal() or not 0 < int(handle) < (1 << 64):
            raise ValueError("owned handles must be nonzero decimal strings")
        self.client = client
        self.id = handle
        self._release = weakref.finalize(self, _release, client, handle)

    def call(self, op, params=None, *, abort=None):
        return self.client.call(op, params, handle=self.id, abort=abort)

    def stream(self, op, params=None, *, abort=None):
        return self.client.stream(op, params, handle=self.id, abort=abort)

    def push(self, data):
        self.client.operation_push(self.id, data)

    def close(self):
        if self._release.alive:
            self.client.call(ops.OP_HANDLE_DROP, handle=self.id)
            self._release.detach()

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.close()


class EventStream:
    """Iterator over event frames. Terminal failures raise typed exceptions."""

    def __init__(self, native):
        self._native = native

    def __iter__(self):
        return self

    def __next__(self):
        return json.loads(next(self._native))

    def close(self):
        self._native.close()

    def cancel(self):
        self._native.cancel()

    def join(self):
        self._native.join()

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.close()


__all__ = _error_names + [
    "ABI_VERSION", "OPS_VERSION", "EXPECTED_MANIFEST_DIGEST", "Abort", "Client", "Handle", "EventStream",
    "StreamIterator", "manifest", "manifest_json", "manifest_digest", "verify_contract",
]
