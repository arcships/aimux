"""Error contract tests for the Python binding.

Every AiMuxError failure is an ``AimuxError`` subclass; the binding's own failures
are Python builtins (``ValueError`` / ``RuntimeError``, never an ``AimuxError``).
Payload attributes belong to the exception class that carries them and are
absent on unrelated classes. Optional ``APICallError`` values use ``None``.

No outbound network: registry lookups fail before any request, and the API-call
case talks to a mock server on 127.0.0.1.
"""

import json
import socket
import time
from http.server import BaseHTTPRequestHandler, HTTPServer
from multiprocessing import Process

import pytest

# Attributes the core only ever fills on ApiCall.
_API_CALL_ONLY = (
    "retry_ms",
    "provider_code",
    "provider_message",
    "response_body",
    "url",
    "request_body_values",
    "response_headers",
    "data",
)

_ERROR_BODY = json.dumps(
    {"error": {"message": "slow down", "type": "rate_limit_exceeded"}}
)


def _free_port():
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def _rate_limited_server(port, retry_after_ms="1500"):
    """Always answers 429 with a provider error body plus retry/request headers."""

    class Handler(BaseHTTPRequestHandler):
        def do_POST(self):
            self.rfile.read(int(self.headers.get("Content-Length", 0)))
            resp = _ERROR_BODY.encode()
            self.send_response(429)
            self.send_header("content-type", "application/json")
            self.send_header("content-length", str(len(resp)))
            self.send_header("x-request-id", "req_abc123")
            self.send_header("retry-after-ms", retry_after_ms)
            self.end_headers()
            self.wfile.write(resp)

        def log_message(self, *args):
            pass

    HTTPServer(("127.0.0.1", port), Handler).serve_forever()


def _wait_for_port(port):
    for _ in range(50):
        try:
            with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
                s.connect(("127.0.0.1", port))
            break
        except ConnectionRefusedError:
            time.sleep(0.05)


def test_no_such_provider_carries_only_its_payload_and_no_json():
    from aimux import AimuxError, NoSuchProviderError, provider

    with pytest.raises(NoSuchProviderError) as excinfo:
        provider("definitely-not-a-provider", "sk-test-fake-key", "some-model")
    err = excinfo.value

    assert type(err) is NoSuchProviderError
    assert isinstance(err, AimuxError)
    # The registry payload is an attribute, not something to dig out of JSON.
    assert err.provider_id == "definitely-not-a-provider"
    for attr in ("code", "status", "retryable", *_API_CALL_ONLY):
        assert not hasattr(err, attr), f"{attr} must not exist on {type(err).__name__}"
    # Every payload field is a typed attribute; there is no JSON companion.
    assert not hasattr(err, "error_value")


def test_api_call_error_carries_every_field_the_response_produced():
    from aimux import APICallError, generate_text, openai

    port = _free_port()
    proc = Process(target=_rate_limited_server, args=(port,))
    proc.start()
    try:
        _wait_for_port(port)

        model = openai("test-key", "gpt-4o", f"http://127.0.0.1:{port}")
        # 429 is retryable; disable retries to observe the bare call failure.
        with pytest.raises(APICallError) as excinfo:
            generate_text(model, "hi", {"max_retries": 0})
        err = excinfo.value

        assert type(err) is APICallError
        assert err.status == 429
        assert err.retry_ms == 1500
        assert err.retryable is True
        assert err.provider_code == "rate_limit_exceeded"
        # No distinct request-id field: the raw header evidence carries it.
        assert err.response_headers["x-request-id"] == "req_abc123"
        assert err.response_headers["retry-after-ms"] == "1500"
        assert f"127.0.0.1:{port}" in err.url
        assert err.request_body_values["model"] == "gpt-4o"
        assert err.response_body == _ERROR_BODY
        # The provider's text on its own; str(e) is the composed form.
        assert err.provider_message == "slow down"
        assert "slow down" in str(err)
        assert str(err) != err.provider_message
    finally:
        proc.terminate()
        proc.join(timeout=2)


def test_retry_error_carries_reason_and_typed_attempt_history():
    from aimux import APICallError, RetryError, generate_text, openai

    port = _free_port()
    # retry-after-ms: 1 keeps the honored retry delay negligible.
    proc = Process(target=_rate_limited_server, args=(port, "1"))
    proc.start()
    try:
        _wait_for_port(port)

        model = openai("test-key", "gpt-4o", f"http://127.0.0.1:{port}")
        with pytest.raises(RetryError) as excinfo:
            generate_text(model, "hi", {"max_retries": 1})
        err = excinfo.value

        assert type(err) is RetryError
        assert err.reason == "maxRetriesExceeded"
        # Attempt history, oldest first; each entry is a full typed exception.
        assert len(err.errors) == 2
        assert err.last_error is err.errors[-1]
        for attempt in err.errors:
            assert type(attempt) is APICallError
            assert attempt.status == 429
            assert attempt.provider_code == "rate_limit_exceeded"
        assert "Failed after 2 attempts" in str(err)
    finally:
        proc.terminate()
        proc.join(timeout=2)


def test_malformed_wire_json_is_a_value_error_not_a_core_error():
    from aimux import AimuxError, openai

    # The native method: the wrapper's generate_text() JSON-encodes a str
    # prompt, so malformed wire text can only reach the binding this way.
    model = openai("test-key", "gpt-4o", "http://127.0.0.1:9")
    with pytest.raises(ValueError, match="prompt_json") as excinfo:
        model.generate_text("{not json")

    assert not isinstance(excinfo.value, AimuxError)
