"""Synthetic provider replies and independent HTTP/WS servers for binding tests."""
import base64
import hashlib
import json
import multiprocessing
import socket
import time
from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, HTTPServer

# The OpenAI provider's default languageModel uses the Responses API.
OPENAI_RESPONSES = json.dumps({
    "id": "resp-test",
    "object": "response",
    "model": "gpt-4o",
    "status": "completed",
    "output": [{
        "id": "msg-test", "type": "message", "role": "assistant", "status": "completed",
        "content": [{"type": "output_text", "text": "Rust is a systems programming language.",
                     "annotations": []}],
    }],
    "usage": {"input_tokens": 10, "output_tokens": 8, "total_tokens": 18},
})

OPENAI_RESPONSES_STREAM = "".join(
    f"event: {event['type']}\ndata: {json.dumps(event)}\n\n" for event in (
        {"type": "response.created", "response": {
            "id": "resp-test", "model": "gpt-4o", "created_at": 1234,
        }},
        {"type": "response.output_item.added", "output_index": 0, "item": {
            "id": "msg-test", "type": "message", "role": "assistant", "content": [],
        }},
        {"type": "response.output_text.delta", "output_index": 0,
         "item_id": "msg-test", "content_index": 0, "delta": "Hello"},
        {"type": "response.output_text.delta", "output_index": 0,
         "item_id": "msg-test", "content_index": 0, "delta": " world"},
        {"type": "response.output_item.done", "output_index": 0, "item": {
            "id": "msg-test", "type": "message", "role": "assistant", "status": "completed",
            "content": [{"type": "output_text", "text": "Hello world", "annotations": []}],
        }},
        {"type": "response.completed", "response": {
            "id": "resp-test", "status": "completed",
            "usage": {"input_tokens": 5, "output_tokens": 2, "total_tokens": 7},
        }},
    )
)

EMBEDDING_RESP = json.dumps({
    "data": [{"embedding": [0.1, 0.2, 0.3], "index": 0}],
    "model": "text-embedding-3-small",
    "usage": {"prompt_tokens": 3, "total_tokens": 3},
})

SPEECH_RESP = b"ID3\x04\x00fake-mp3-audio-payload"

IMAGE_RESP = json.dumps({"data": [{"b64_json": "aW1hZ2Ux"}]})

TRANSCRIPTION_RESP = json.dumps({"text": "Hello world"})

RERANKING_RESP = json.dumps({
    "results": [
        {"index": 1, "relevance_score": 0.95},
        {"index": 0, "relevance_score": 0.3},
    ],
})

SEARCH_RESP = json.dumps({
    "results": [
        {"title": "Rust", "url": "https://rust-lang.org", "content": "Rust is..."},
    ],
    "answer": "Rust is a systems language.",
})

FILES_RESP = json.dumps({
    "id": "file-abc",
    "object": "file",
    "bytes": 1024,
    "created_at": 1234,
    "filename": "test.pdf",
    "purpose": "assistants",
})

WS_GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"

def _text_frame(payload: str) -> bytes:
    data = payload.encode()
    assert len(data) < 126, "extend the codec for long payloads"
    return bytes([0x81, len(data)]) + data

def _close_frame(code: int, reason: str) -> bytes:
    data = code.to_bytes(2, "big") + reason.encode()
    return bytes([0x88, len(data)]) + data

def _handshake(sock: socket.socket) -> None:
    buf = b""
    while b"\r\n\r\n" not in buf:
        chunk = sock.recv(4096)
        if not chunk:
            raise EOFError("client vanished mid-handshake")
        buf += chunk
    key = None
    protocols = []
    for line in buf.decode("latin-1").split("\r\n"):
        if line.lower().startswith("sec-websocket-key:"):
            key = line.split(":", 1)[1].strip()
        elif line.lower().startswith("sec-websocket-protocol:"):
            protocols = [p.strip() for p in line.split(":", 1)[1].split(",")]
    if key is None:
        raise ValueError("no Sec-WebSocket-Key in request")
    accept = base64.b64encode(
        hashlib.sha1((key + WS_GUID).encode()).digest()
    ).decode()
    sock.sendall(
        (
            "HTTP/1.1 101 Switching Protocols\r\n"
            "Upgrade: websocket\r\n"
            "Connection: Upgrade\r\n"
            f"Sec-WebSocket-Accept: {accept}\r\n"
            # The realtime API answers the `realtime` subprotocol the client
            # offers; a WebSocket client fails when an offer gets no answer.
            + ("Sec-WebSocket-Protocol: realtime\r\n" if "realtime" in protocols else "")
            + "\r\n"
        ).encode()
    )

def _ws_server_proc(port, script):
    """Fake realtime server. `script` is a list of (delay_ms, action) tuples
    (relative to handshake completion); action is ("event", json_text) or
    ("close", code, reason). Client frames are masked and simply drained.
    Readiness probes (bare TCP connects from the test process) are skipped.
    """
    import time as _time

    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
        listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        listener.bind(("127.0.0.1", port))
        listener.listen(1)
        listener.settimeout(10)
        while True:
            conn, _ = listener.accept()
            with conn:
                try:
                    _handshake(conn)
                except (EOFError, ValueError, OSError):
                    # Readiness probe (closes without speaking WS) — skip.
                    continue
                start = _time.monotonic()
                for delay_ms, action in script:
                    target = start + delay_ms / 1000
                    # Drain client frames (masked; content irrelevant).
                    while _time.monotonic() < target:
                        conn.settimeout(max(0.01, target - _time.monotonic()))
                        try:
                            if not conn.recv(65536):
                                return
                        except socket.timeout:
                            pass
                    if action[0] == "event":
                        conn.sendall(_text_frame(action[1]))
                    elif action[0] == "close":
                        conn.sendall(_close_frame(action[1], action[2]))
                    else:
                        raise ValueError(f"unknown action {action!r}")
                # Hold the socket open after the script — the client drives
                # the teardown — so a silent gap is never an accidental
                # disconnect.
                conn.settimeout(10)
                try:
                    while conn.recv(65536):
                        pass
                except OSError:
                    pass
                return

DELTA_EVENT = (
    '{"type":"conversation.item.input_audio_transcription.delta",'
    '"item_id":"i","delta":"he"}'
)


def _http_server(connection, body, content_type, delay, status):
    class Handler(BaseHTTPRequestHandler):
        def respond(self):
            if self.command == "POST":
                self.rfile.read(int(self.headers.get("Content-Length", 0)))
            time.sleep(delay)
            # A dict body is keyed by request path suffix first, then by method.
            payload = next((value for key, value in body.items() if self.path.endswith(key)),
                           body.get(self.command)) if isinstance(body, dict) else body
            if isinstance(payload, str): payload = payload.encode()
            self.send_response(status)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            try: self.wfile.write(payload)
            except (BrokenPipeError, ConnectionResetError): pass
        do_POST = respond
        do_GET = respond
        def log_message(self, *_): pass
    with HTTPServer(("127.0.0.1", 0), Handler) as server:
        connection.send(server.server_address[1])
        server.serve_forever()


@contextmanager
def slow_http(body=OPENAI_RESPONSES, content_type="application/json", delay=0.5, status=200):
    parent, child = multiprocessing.Pipe()
    process = multiprocessing.Process(target=_http_server, args=(child, body, content_type, delay, status))
    process.start()
    try:
        if not parent.poll(5):
            raise RuntimeError("HTTP fixture could not bind its localhost socket")
        port = parent.recv()
        yield "http://127.0.0.1:%d" % port
    finally:
        process.terminate()
        process.join(timeout=2)
        if process.is_alive(): process.kill(); process.join(timeout=2)
        parent.close()
        child.close()


def _delayed_ws(port, delay):
    original = _handshake
    def delayed(sock):
        time.sleep(delay)
        original(sock)
    globals()["_handshake"] = delayed
    _ws_server_proc(port, [(0, ("event", DELTA_EVENT))])


@contextmanager
def slow_ws(delay=0.5):
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        port = probe.getsockname()[1]
    process = multiprocessing.Process(target=_delayed_ws, args=(port, delay))
    process.start()
    try:
        for _ in range(100):
            try:
                with socket.create_connection(("127.0.0.1", port), timeout=0.1): break
            except ConnectionRefusedError: time.sleep(0.01)
        else: raise RuntimeError("WS fixture did not start")
        yield "http://127.0.0.1:%d" % port
    finally:
        process.terminate()
        process.join(timeout=2)
        if process.is_alive(): process.kill(); process.join(timeout=2)
