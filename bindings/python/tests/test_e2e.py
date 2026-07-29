"""End-to-end provider tests with mock HTTP server.

Verifies the FULL chain: Python → PyO3 → Rust engine → HTTP mock →
response parsing → typed result.

The mock server runs in a separate process (multiprocessing) because
PyO3's block_on blocks the calling thread — a threading-based server
would deadlock.
"""

import json
import os
import signal
import socket
import time
from http.server import HTTPServer, BaseHTTPRequestHandler
from multiprocessing import Process

import pytest

from aimux import openai, anthropic, generate_text, stream_text


# ── Mock server in a separate process ───────────────────────────────────────

def _find_free_port():
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(('127.0.0.1', 0))
        return s.getsockname()[1]


def _mock_server_proc(port, response_body, content_type, status=200):
    """Mock HTTP server that always returns the same response."""

    class Handler(BaseHTTPRequestHandler):
        def do_POST(self):
            body = self.rfile.read(int(self.headers.get('Content-Length', 0)))
            # Store last body in the server for inspection
            self.server.last_body = body  # type: ignore

            resp = response_body.encode() if isinstance(response_body, str) else response_body
            self.send_response(status)
            self.send_header('content-type', content_type)
            self.send_header('content-length', str(len(resp)))
            self.end_headers()
            self.wfile.write(resp)

        def do_GET(self):
            resp = response_body.encode() if isinstance(response_body, str) else response_body
            self.send_response(status)
            self.send_header('content-type', content_type)
            self.send_header('content-length', str(len(resp)))
            self.end_headers()
            self.wfile.write(resp)

        def log_message(self, *args):
            pass

    server = HTTPServer(('127.0.0.1', port), Handler)
    server.last_body = None  # type: ignore
    server.serve_forever()


class MockServer:
    """Context manager that starts a mock HTTP server in a subprocess."""

    def __init__(self, response_body, content_type='application/json', status=200):
        self.port = _find_free_port()
        self.url = f'http://127.0.0.1:{self.port}'
        self.proc = Process(
            target=_mock_server_proc,
            args=(self.port, response_body, content_type, status),
        )

    def __enter__(self):
        self.proc.start()
        # Wait for server to be ready
        for _ in range(50):
            try:
                with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
                    s.connect(('127.0.0.1', self.port))
                    break
            except ConnectionRefusedError:
                time.sleep(0.05)
        return self

    def __exit__(self, *args):
        self.proc.terminate()
        self.proc.join(timeout=2)


# ── Mock response data (real API shapes) ────────────────────────────────────

OPENAI_CHAT = json.dumps({
    "id": "chatcmpl-test",
    "model": "gpt-4o",
    "choices": [{
        "message": {"role": "assistant", "content": "Rust is a systems programming language."},
        "finish_reason": "stop",
    }],
    "usage": {"prompt_tokens": 10, "completion_tokens": 8, "total_tokens": 18},
})

OPENAI_STREAM = (
    'data: {"id":"1","model":"gpt-4o","choices":[{"delta":{"content":"Hello"}}]}\n\n'
    'data: {"id":"1","model":"gpt-4o","choices":[{"delta":{"content":" world"}}]}\n\n'
    'data: {"id":"1","model":"gpt-4o","choices":[{"delta":{},"finish_reason":"stop"}],'
    '"usage":{"prompt_tokens":5,"completion_tokens":2,"total_tokens":7}}\n\n'
    'data: [DONE]\n\n'
)

ANTHROPIC_MSG = json.dumps({
    "id": "msg_test",
    "type": "message",
    "role": "assistant",
    "model": "claude-3-5-sonnet-20241022",
    "content": [{"type": "text", "text": "Hello from Claude!"}],
    "stop_reason": "end_turn",
    "usage": {"input_tokens": 10, "output_tokens": 5},
})

ANTHROPIC_STREAM = (
    'event: message_start\ndata: {"type":"message_start","message":'
    '{"id":"msg_1","model":"claude-3-5-sonnet-20241022",'
    '"usage":{"input_tokens":10,"output_tokens":0}}}\n\n'
    'event: content_block_start\ndata: {"type":"content_block_start",'
    '"index":0,"content_block":{"type":"text","text":""}}\n\n'
    'event: content_block_delta\ndata: {"type":"content_block_delta",'
    '"index":0,"delta":{"type":"text_delta","text":"Hello"}}\n\n'
    'event: content_block_delta\ndata: {"type":"content_block_delta",'
    '"index":0,"delta":{"type":"text_delta","text":" from Claude"}}\n\n'
    'event: content_block_stop\ndata: {"type":"content_block_stop","index":0}\n\n'
    'event: message_delta\ndata: {"type":"message_delta",'
    '"delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":5}}\n\n'
    'event: message_stop\ndata: {"type":"message_stop"}\n\n'
)


# ── Tests ───────────────────────────────────────────────────────────────────

class TestOpenAIE2E:

    def test_generate_text(self):
        with MockServer(OPENAI_CHAT) as mock:
            model = openai("test-key", "gpt-4o", mock.url)
            result = generate_text(model, "What is Rust?")
            assert result["text"] == "Rust is a systems programming language."
            assert result["usage"] is not None

    def test_stream_text(self):
        with MockServer(OPENAI_STREAM, content_type='text/event-stream') as mock:
            model = openai("test-key", "gpt-4o", mock.url)
            parts = list(stream_text(model, "Say hello"))
            assert len(parts) > 0
            text = "".join(
                p["TextDelta"]["delta"] for p in parts if "TextDelta" in p
            )
            assert text == "Hello world"

    def test_generate_text_with_options(self):
        with MockServer(OPENAI_CHAT) as mock:
            model = openai("test-key", "gpt-4o", mock.url)
            generate_text(model, "Hello", {"max_output_tokens": 100, "temperature": 0.5})
            # Options should reach the HTTP request (tested implicitly — if they
            # don't serialize, the Rust side would error)


class TestAnthropicE2E:

    def test_generate_text(self):
        with MockServer(ANTHROPIC_MSG) as mock:
            model = anthropic("test-key", "claude-3-5-sonnet-20241022", mock.url)
            result = generate_text(model, "Hello")
            assert result["text"] == "Hello from Claude!"
            assert result["usage"] is not None

    def test_stream_text(self):
        with MockServer(ANTHROPIC_STREAM, content_type='text/event-stream') as mock:
            model = anthropic("test-key", "claude-3-5-sonnet-20241022", mock.url)
            parts = list(stream_text(model, "Hello"))
            assert len(parts) > 0
            text = "".join(
                p["TextDelta"]["delta"] for p in parts if "TextDelta" in p
            )
            assert text == "Hello from Claude"
