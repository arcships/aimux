"""End-to-end provider tests with mock HTTP server.

Verifies the FULL chain: Python → PyO3 → aimux-core → HTTP mock →
response parsing → typed result.

The mock server runs in a separate process (multiprocessing) because
PyO3's block_on blocks the calling thread — a threading-based server
would deadlock.
"""

import json
import os
import signal
import socket
import tempfile
import time
from http.server import HTTPServer, BaseHTTPRequestHandler
from multiprocessing import Process, Queue

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


# ── Mock server that records the request body ───────────────────────────────


def _recording_server_proc(port, response_body, content_type, q):
    """Mock server that records each request body into the queue."""

    class Handler(BaseHTTPRequestHandler):
        def do_POST(self):
            body = self.rfile.read(int(self.headers.get('Content-Length', 0)))
            try:
                q.put(json.loads(body))
            except Exception:
                q.put(None)
            resp = response_body.encode() if isinstance(response_body, str) else response_body
            self.send_response(200)
            self.send_header('content-type', content_type)
            self.send_header('content-length', str(len(resp)))
            self.end_headers()
            self.wfile.write(resp)

        def log_message(self, *args):
            pass

    server = HTTPServer(('127.0.0.1', port), Handler)
    server.serve_forever()


class RecordingMockServer:
    """Mock server that returns a fixed response and records the request body."""

    def __init__(self, response_body, content_type='application/json'):
        self.port = _find_free_port()
        self.url = f'http://127.0.0.1:{self.port}'
        self.q = Queue()
        self.proc = Process(
            target=_recording_server_proc,
            args=(self.port, response_body, content_type, self.q),
        )

    def __enter__(self):
        self.proc.start()
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

    def received_body(self, timeout=2.0):
        return self.q.get(timeout=timeout)


# ── Mock server that returns a different response per request (FIFO) ────────


def _seq_server_proc(port, responses, content_type, q):
    """Mock server that returns different responses for each request (FIFO).

    Like the recording server, every request body is pushed to the queue so the
    caller can inspect what reached the provider. The response for the Nth
    request is ``responses[min(N, len(responses)-1)]`` (the last response is
    reused for any extra requests).
    """
    call_idx = [0]

    class Handler(BaseHTTPRequestHandler):
        def do_POST(self):
            body = self.rfile.read(int(self.headers.get('Content-Length', 0)))
            try:
                q.put(json.loads(body))
            except Exception:
                q.put(None)
            idx = call_idx[0]
            call_idx[0] += 1
            resp = responses[min(idx, len(responses) - 1)]
            resp_bytes = resp.encode() if isinstance(resp, str) else resp
            self.send_response(200)
            self.send_header('content-type', content_type)
            self.send_header('content-length', str(len(resp_bytes)))
            self.end_headers()
            self.wfile.write(resp_bytes)

        def log_message(self, *args):
            pass

    server = HTTPServer(('127.0.0.1', port), Handler)
    server.serve_forever()


class SequencedMockServer:
    """Mock server that returns a different response for each request (FIFO).

    The response list is consumed in order (the last entry is reused for any
    extra requests beyond the list length). Each request body is recorded into
    the queue, so callers can assert on what reached the provider — useful for
    full tool-call round-trips where the server must return a tool-call
    response first and a final text response second.
    """

    def __init__(self, responses, content_type='application/json'):
        self.port = _find_free_port()
        self.url = f'http://127.0.0.1:{self.port}'
        self.q = Queue()
        self.proc = Process(
            target=_seq_server_proc,
            args=(self.port, responses, content_type, self.q),
        )

    def __enter__(self):
        self.proc.start()
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

    def received_body(self, timeout=2.0):
        return self.q.get(timeout=timeout)


# ── Tool-call parsing & structured content ──────────────────────────────────

OPENAI_TOOL_CALL = json.dumps({
    "id": "chatcmpl-tc",
    "model": "gpt-4o",
    "choices": [{
        "message": {
            "role": "assistant",
            "content": None,
            "tool_calls": [{
                "id": "call_abc",
                "type": "function",
                "function": {"name": "get_weather", "arguments": '{"location":"Tokyo"}'},
            }],
        },
        "finish_reason": "tool_calls",
    }],
    "usage": {"prompt_tokens": 20, "completion_tokens": 10, "total_tokens": 30},
})


class TestStructuredContent:

    def test_generate_text_parses_tool_calls(self):
        with RecordingMockServer(OPENAI_TOOL_CALL) as mock:
            model = openai("test-key", "gpt-4o", mock.url)
            opts = {
                "tools": [{
                    "type": "function",
                    "name": "get_weather",
                    "description": "Get weather for a location",
                    "input_schema": {
                        "type": "object",
                        "properties": {"location": {"type": "string"}},
                        "required": ["location"],
                    },
                }],
            }
            result = generate_text(model, "What's the weather in Tokyo?", opts)

            # Convenience field: tool_calls extracted
            assert len(result["tool_calls"]) == 1
            assert result["tool_calls"][0]["tool_name"] == "get_weather"
            assert result["tool_calls"][0]["tool_call_id"] == "call_abc"
            assert result["tool_calls"][0]["input"] == {"location": "Tokyo"}

            # Structured content: raw.content contains the ToolCall variant
            assert "raw" in result
            assert isinstance(result["raw"]["content"], list)
            tc = next((c for c in result["raw"]["content"] if "ToolCall" in c), None)
            assert tc is not None, "raw.content must contain a ToolCall variant"
            assert tc["ToolCall"]["tool_name"] == "get_weather"
            assert tc["ToolCall"]["tool_call_id"] == "call_abc"
            # raw content keeps the provider's argument text; parsing happens
            # at the Core boundary (top-level tool_calls carry the object).
            assert tc["ToolCall"]["input"] == '{"location":"Tokyo"}'

    def test_multi_role_messages_reach_provider(self):
        with RecordingMockServer(OPENAI_CHAT) as mock:
            model = openai("test-key", "gpt-4o", mock.url)
            prompt = [
                {"role": "system", "content": "You are a helpful assistant."},
                {"role": "user", "content": "What is Rust?"},
            ]
            result = generate_text(model, prompt)

            body = mock.received_body()
            assert isinstance(body["messages"], list)
            assert len(body["messages"]) == 2
            assert body["messages"][0]["role"] == "system"
            assert body["messages"][0]["content"] == "You are a helpful assistant."
            assert body["messages"][1]["role"] == "user"
            assert body["messages"][1]["content"] == "What is Rust?"
            assert result["text"]

    def test_tool_choice_reaches_provider(self):
        with RecordingMockServer(OPENAI_TOOL_CALL) as mock:
            model = openai("test-key", "gpt-4o", mock.url)
            opts = {
                "tools": [{
                    "type": "function",
                    "name": "get_weather",
                    "input_schema": {
                        "type": "object",
                        "properties": {"location": {"type": "string"}},
                    },
                }],
                "tool_choice": "required",
            }
            generate_text(model, "Hello", opts)

            body = mock.received_body()
            assert body["tool_choice"] == "required"

    def test_stream_text_parses_tool_call_parts(self):
        """Stream text should surface ToolCall/ToolInputDelta parts (not just TextDelta/Finish)."""
        sse_body = (
            'data: {"id":"1","model":"gpt-4o","choices":[{"delta":{"role":"assistant","tool_calls":[{"index":0,"id":"call_xyz","type":"function","function":{"name":"get_weather","arguments":""}}]}}]}\n\n'
            'data: {"id":"1","model":"gpt-4o","choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\\"location\\":\\"Tokyo\\"}"}}]}}]}\n\n'
            'data: {"id":"1","model":"gpt-4o","choices":[{"delta":{},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":5,"completion_tokens":2,"total_tokens":7}}\n\n'
            'data: [DONE]\n\n'
        )
        with MockServer(sse_body, content_type='text/event-stream') as mock:
            model = openai("test-key", "gpt-4o", mock.url)
            opts = {
                "tools": [{
                    "type": "function",
                    "name": "get_weather",
                    "input_schema": {
                        "type": "object",
                        "properties": {"location": {"type": "string"}},
                    },
                }],
            }
            parts = list(stream_text(model, "What's the weather in Tokyo?", opts))
            assert len(parts) > 0

            # Stream must contain a tool-related StreamPart (not just TextDelta/Finish)
            has_tool_part = any(
                "ToolCall" in p or "ToolInputDelta" in p or "ToolInputStart" in p
                for p in parts
            )
            assert has_tool_part, f"expected a tool stream part; got: {parts}"

            # If a complete ToolCall part is present, verify its fields
            tool_call_parts = [p for p in parts if "ToolCall" in p]
            if tool_call_parts:
                tc = tool_call_parts[0]["ToolCall"]
                assert tc["tool_name"] == "get_weather"

    def test_tool_call_round_trip(self):
        """Full tool-call round trip: two generate_text calls with a ToolResult
        back-filled between them.

        First call → model requests a tool call (OPENAI_TOOL_CALL).
        User executes the tool and sends the result back.
        Second call → model returns final text (OPENAI_CHAT).

        The messages use the framework's user-facing content-part shape
        (``tool_call`` / ``tool_result`` parts). The OpenAI converter turns
        these into the provider-facing body (assistant ``tool_calls`` array +
        ``tool`` role message with ``tool_call_id``), which is what we assert
        on for the second request.
        """
        # The mock answers request #1 with a tool call and request #2 with text.
        with SequencedMockServer([OPENAI_TOOL_CALL, OPENAI_CHAT]) as mock:
            model = openai("test-key", "gpt-4o", mock.url)
            opts = {
                "tools": [{
                    "type": "function",
                    "name": "get_weather",
                    "description": "Get weather for a location",
                    "input_schema": {
                        "type": "object",
                        "properties": {"location": {"type": "string"}},
                        "required": ["location"],
                    },
                }],
            }

            # Step 1: first call — the model requests a tool call.
            result = generate_text(model, "What's the weather in Tokyo?", opts)
            assert len(result["tool_calls"]) == 1
            assert result["tool_calls"][0]["tool_name"] == "get_weather"
            assert result["tool_calls"][0]["tool_call_id"] == "call_abc"

            # Step 2: back-fill the ToolResult and re-send the conversation.
            # Roles: user → assistant(tool_call) → tool(result).
            messages = [
                {"role": "user", "content": "What's the weather in Tokyo?"},
                {"role": "assistant", "content": [
                    {"type": "tool_call", "tool_call_id": "call_abc",
                     "tool_name": "get_weather", "input": {"location": "Tokyo"}},
                ]},
                {"role": "tool", "content": [
                    {"type": "tool_result", "tool_call_id": "call_abc",
                     "result": {"temperature": 22, "condition": "sunny"}},
                ]},
            ]

            # Step 3: second call — same tools, model returns final text.
            result2 = generate_text(model, messages, opts)
            assert result2["text"] == "Rust is a systems programming language."

            # Step 4: the second request body must carry the full round trip.
            mock.received_body()  # first request (tool-call request) — drain
            body2 = mock.received_body()
            msgs = body2["messages"]
            assert isinstance(msgs, list)
            assert len(msgs) == 3, f"expected 3 messages, got {len(msgs)}"
            last = msgs[-1]
            assert last["role"] == "tool", f"expected last role 'tool', got {last.get('role')}"
            assert last["tool_call_id"] == "call_abc"
