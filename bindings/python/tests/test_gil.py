"""Blocking native calls release the GIL.

The mock server here runs in a thread of the test process, so it can only
answer while the calling thread waits inside a native call if that call
released the GIL. Each call carries a total timeout: if the GIL were held,
the server could not answer and the call would fail with a timeout instead
of hanging the suite.
"""

import json
import threading
import time
from http.server import BaseHTTPRequestHandler, HTTPServer

import pytest

from aimux import deepseek, generate_text, openai_embedding

CHAT = json.dumps({
    "id": "chatcmpl-gil",
    "model": "deepseek-chat",
    "choices": [{
        "message": {"role": "assistant", "content": "served by a Python thread"},
        "finish_reason": "stop",
    }],
    "usage": {"prompt_tokens": 1, "completion_tokens": 5, "total_tokens": 6},
})

EMBEDDING = json.dumps({
    "data": [{"embedding": [0.5, 0.25], "index": 0}],
    "model": "text-embedding-3-small",
    "usage": {"prompt_tokens": 1, "total_tokens": 1},
})

LIMITS = {"maxRetries": 0, "timeout": {"totalMs": 10000}}


class _ThreadServer:
    """HTTP server on a daemon thread of this process, answering slowly."""

    def __init__(self, body):
        progress = self.progress = []

        class Handler(BaseHTTPRequestHandler):
            def do_POST(self):
                self.rfile.read(int(self.headers.get("Content-Length", 0)))
                # Keep the native call waiting while this thread keeps running.
                for _ in range(10):
                    progress.append(time.monotonic())
                    time.sleep(0.02)
                payload = body.encode()
                self.send_response(200)
                self.send_header("content-type", "application/json")
                self.send_header("content-length", str(len(payload)))
                self.end_headers()
                self.wfile.write(payload)

            def log_message(self, *args):
                pass

        self.server = HTTPServer(("127.0.0.1", 0), Handler)
        self.url = "http://127.0.0.1:%d" % self.server.server_address[1]
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, *args):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=5)


def test_generate_text_lets_other_python_threads_run():
    with _ThreadServer(CHAT) as server:
        # A chat-completions provider: openai() uses the Responses API.
        model = deepseek("test-key", "deepseek-chat", server.url)
        started = time.monotonic()
        result = generate_text(model, "hi", LIMITS)
        assert result["text"] == "served by a Python thread"
        # The server thread ran while generate_text was blocked.
        assert len(server.progress) == 10
        assert all(t >= started for t in server.progress)


def test_embed_lets_other_python_threads_run():
    with _ThreadServer(EMBEDDING) as server:
        embedder = openai_embedding("test-key", "text-embedding-3-small", base_url=server.url)
        opts = dict(LIMITS, values=[])
        result = json.loads(embedder.embed(json.dumps(["hello"]), json.dumps(opts)))
        assert result["embeddings"][0] == pytest.approx([0.5, 0.25])
        assert len(server.progress) == 10
