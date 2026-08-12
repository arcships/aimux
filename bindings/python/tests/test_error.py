"""Error contract tests for the Python binding.

Every failure is an ``AimuxError`` subclass. ``status`` is on the base — the
observed status on ``APICallError``, 401 on ``TokenExpiredError``, ``None``
elsewhere. Every other payload attribute belongs to the one exception class the
core fills it for, and is *absent* (not ``None``) on the others.

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
    "retryable",
    "provider_code",
    "provider_message",
    "response_body",
    "request_id",
)

_ERROR_BODY = json.dumps(
    {"error": {"message": "slow down", "type": "rate_limit_exceeded"}}
)


def _free_port():
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def _rate_limited_server(port):
    """Always answers 429 with a provider error body plus retry/request headers."""

    class Handler(BaseHTTPRequestHandler):
        def do_POST(self):
            self.rfile.read(int(self.headers.get("Content-Length", 0)))
            resp = _ERROR_BODY.encode()
            self.send_response(429)
            self.send_header("content-type", "application/json")
            self.send_header("content-length", str(len(resp)))
            self.send_header("x-request-id", "req_abc123")
            self.send_header("retry-after-ms", "1500")
            self.end_headers()
            self.wfile.write(resp)

        def log_message(self, *args):
            pass

    HTTPServer(("127.0.0.1", port), Handler).serve_forever()


def test_no_such_provider_carries_provider_id():
    from aimux import NoSuchProviderError, provider

    with pytest.raises(NoSuchProviderError) as excinfo:
        provider("definitely-not-a-provider", "sk-test-fake-key", "some-model")
    err = excinfo.value

    # The registry payload is an attribute, not something to dig out of JSON.
    assert err.provider_id == "definitely-not-a-provider"
    assert err.status is None
    for attr in _API_CALL_ONLY:
        assert not hasattr(err, attr), f"{attr} must not exist on {type(err).__name__}"


def test_error_value_still_carries_the_raw_core_json():
    from aimux import NoSuchProviderError, provider

    with pytest.raises(NoSuchProviderError) as excinfo:
        provider("definitely-not-a-provider", "sk-test-fake-key", "some-model")

    parsed = json.loads(excinfo.value.error_value)
    assert list(parsed) == ["NoSuchProvider"]
    assert parsed["NoSuchProvider"]["provider_id"] == "definitely-not-a-provider"


def test_api_call_error_carries_every_field_the_response_produced():
    from aimux import APICallError, generate_text, openai

    port = _free_port()
    proc = Process(target=_rate_limited_server, args=(port,))
    proc.start()
    try:
        for _ in range(50):
            try:
                with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
                    s.connect(("127.0.0.1", port))
                break
            except ConnectionRefusedError:
                time.sleep(0.05)

        model = openai("test-key", "gpt-4o", f"http://127.0.0.1:{port}")
        with pytest.raises(APICallError) as excinfo:
            generate_text(model, "hi")
        err = excinfo.value

        assert err.status == 429
        assert err.retry_ms == 1500
        assert err.retryable is True
        assert err.provider_code == "rate_limit_exceeded"
        assert err.request_id == "req_abc123"
        assert err.response_body == _ERROR_BODY
        # The provider's text on its own; str(e) is the composed form.
        assert err.provider_message == "slow down"
        assert "slow down" in str(err)
        assert str(err) != err.provider_message
    finally:
        proc.terminate()
        proc.join(timeout=2)
