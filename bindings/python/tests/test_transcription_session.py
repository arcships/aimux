"""Behavioral tests for the RFC-0028 transcription session API (Python binding).

Covers the session surface (`start_transcription_session` / `push_audio` /
`next_part` / `close`) against a fake realtime WebSocket server:

- `next_part(timeout_ms=...)` raises `APITimeoutError` that is *determinable*
  for retries (`retryable` is False), and the session stays live afterwards.
- A server-sent realtime `error` event propagates verbatim as `APICallError`.
- A connect failure is delivered through the session channel as the first
  `next_part` result (the session API never silently hangs).

The fake WS server runs in a separate process (multiprocessing), like the
HTTP mocks in test_e2e — PyO3 blocks the calling thread, so a threading-based
server would deadlock. Handshake + frame codec are hand-rolled (no deps).
"""

import base64
import hashlib
import json
import socket
from multiprocessing import Process

import pytest

from aimux import (
    APICallError,
    APITimeoutError,
    openai_transcription,
    start_transcription_session,
)

WS_GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"


# ── Minimal server-side WebSocket codec (frames < 126 bytes) ────────────────

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
    for line in buf.decode("latin-1").split("\r\n"):
        if line.lower().startswith("sec-websocket-key:"):
            key = line.split(":", 1)[1].strip()
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
            "\r\n"
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


def _find_free_port():
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


class FakeRealtimeServer:
    """Context manager: fake realtime WS server playing `script`."""

    def __init__(self, script):
        self.port = _find_free_port()
        self.url = f"http://127.0.0.1:{self.port}"
        self.proc = Process(target=_ws_server_proc, args=(self.port, script))

    def __enter__(self):
        self.proc.start()
        # Wait for the listener to be ready (probe connects are skipped by
        # the server), same gate as test_e2e.MockServer.
        import time as _time

        for _ in range(100):
            try:
                with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
                    s.connect(("127.0.0.1", self.port))
                break
            except ConnectionRefusedError:
                _time.sleep(0.05)
        return self

    def __exit__(self, *args):
        self.proc.terminate()
        self.proc.join(timeout=2)
        if self.proc.is_alive():
            # SIGTERM missed the deadline — SIGKILL so tests never leak the
            # fake-server process.
            self.proc.kill()


SESSION_CREATED = '{"type":"session.created"}'
ERROR_EVENT = (
    '{"type":"error","error":'
    '{"message":"insufficient quota for realtime transcription"}}'
)
DELTA_EVENT = (
    '{"type":"conversation.item.input_audio_transcription.delta",'
    '"item_id":"i","delta":"he"}'
)


class TestTranscriptionSessionBehavior:
    """Session API semantics through the PyO3 channel."""

    def test_next_part_timeout_is_determinable_and_session_survives(self):
        # session.created at 50ms (no part mapping), delta at 900ms: the
        # 250ms next_part window must time out in between and the session
        # must still deliver the later delta.
        with FakeRealtimeServer([(50, ("event", SESSION_CREATED)),
                                 (900, ("event", DELTA_EVENT))]) as mock:
            model = openai_transcription("k", "gpt-realtime-whisper", mock.url)
            session = start_transcription_session(model, None)
            try:
                first = session.next_part(timeout_ms=3000)
                assert first is not None
                assert "StreamStart" in json.loads(first)["Ok"]

                with pytest.raises(APITimeoutError) as ei:
                    session.next_part(timeout_ms=250)
                # Timeout is a spent budget, not a transport failure — the
                # retry verdict must be decidable (and be "don't retry").
                assert "no transcription part within timeout" in str(ei.value)
                assert ei.value.retryable is False

                # The session stayed live across the timeout.
                late = session.next_part(timeout_ms=5000)
                assert late is not None
                assert "TranscriptDelta" in json.loads(late)["Ok"]
            finally:
                session.close()

    def test_server_error_event_propagates_verbatim(self):
        # NOTE on contract: in-stream errors (a server `error` event) are
        # delivered by the Python driver as a serialized
        # `{"Err": {...}}` *part* — only connect failures and next_part
        # timeouts raise (see the FFI session, which returns Err items).
        with FakeRealtimeServer([(50, ("event", ERROR_EVENT))]) as mock:
            model = openai_transcription("k", "gpt-realtime-whisper", mock.url)
            session = start_transcription_session(model, None)
            try:
                first = session.next_part(timeout_ms=5000)
                assert first is not None
                assert "StreamStart" in json.loads(first)["Ok"]

                err_part = session.next_part(timeout_ms=5000)
                assert err_part is not None
                api = json.loads(err_part)["Err"]["ApiCall"]
                # The provider's own message, verbatim, not transport noise.
                assert (
                    api["message"] == "insufficient quota for realtime transcription"
                )
                # The retry verdict is decidable from the part.
                assert api["is_retryable"] is False
                assert api["status_code"] is None

                # The error terminated the stream: the channel ends normally.
                assert session.next_part(timeout_ms=3000) is None
            finally:
                session.close()

    def test_connect_failure_surfaces_through_the_channel(self):
        # Port 1 on loopback: connection refused immediately.
        model = openai_transcription("k", "gpt-realtime-whisper",
                                     "http://127.0.0.1:1")
        session = start_transcription_session(model, None)
        with pytest.raises(APICallError) as ei:
            session.next_part(timeout_ms=5000)
        assert "websocket connect failed" in str(ei.value)
        # The session terminated with the connect error (channel closed).
        assert session.next_part(timeout_ms=3000) is None
