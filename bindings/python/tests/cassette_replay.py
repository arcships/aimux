"""Cassette replay server for Python tests.

Reads real cassette JSON files from aimux-providers/tests/cassettes/
and serves them via a local HTTP server with the same matching logic
as the Rust `common::replay` module.

The server runs in a separate process (multiprocessing) because PyO3's
block_on blocks the calling thread.
"""

import json
import os
import socket
import time
from http.server import HTTPServer, BaseHTTPRequestHandler
from multiprocessing import Process
from pathlib import Path

CASSETTE_BASE = Path(__file__).resolve().parents[3] / "aimux-providers" / "tests" / "cassettes"


def _find_free_port():
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def _load_cassettes(provider):
    """Load all cassette JSON files for a provider."""
    cdir = CASSETTE_BASE / provider
    cassettes = []
    for f in sorted(cdir.glob("*.json")):
        try:
            raw = json.loads(f.read_text())
            body = raw.get("request", {}).get("body", {})
            if isinstance(body, str):
                try:
                    body = json.loads(body)
                except Exception:
                    body = {}
            cassettes.append({
                "scenario": raw.get("scenario", f.name),
                "req_path": raw.get("request", {}).get("path", "/"),
                "req_method": raw.get("request", {}).get("method", "POST"),
                "req_body": body if isinstance(body, dict) else {},
                "resp_status": raw.get("response", {}).get("status", 200),
                "resp_headers": raw.get("response", {}).get("headers", {}),
                "resp_body": raw.get("response", {}).get("body", ""),
            })
        except Exception:
            pass
    return cassettes


def _score(cassette_body, req_body):
    """Score a cassette against the incoming request body."""
    if not isinstance(cassette_body, dict) or not isinstance(req_body, dict):
        return 0
    s = 0
    for key, val in cassette_body.items():
        if not isinstance(val, (str, bool, int, float)):
            continue
        if req_body.get(key) == val:
            if key == "model":
                s += 100
            elif key == "stream":
                s += 10
            else:
                s += 1
    return s


def _find_best_match(cassettes, method, path, req_body):
    """Find the best matching cassette for a request."""
    group = [c for c in cassettes if c["req_method"] == method and c["req_path"] == path]
    if not group:
        return None
    best = group[0]
    best_score = _score(best["req_body"], req_body)
    for c in group[1:]:
        s = _score(c["req_body"], req_body)
        if s > best_score:
            best_score = s
            best = c
    return best


def _server_proc(port, cassettes):
    """Mock HTTP server process."""

    class Handler(BaseHTTPRequestHandler):
        def do_POST(self):
            body = self.rfile.read(int(self.headers.get("Content-Length", 0)))
            try:
                req_body = json.loads(body)
            except Exception:
                req_body = {}

            match = _find_best_match(cassettes, "POST", self.path, req_body)
            if not match:
                self.send_response(404)
                self.send_header("content-type", "application/json")
                self.end_headers()
                self.wfile.write(json.dumps({"error": f"no cassette for POST {self.path}"}).encode())
                return

            resp = match["resp_body"].encode() if isinstance(match["resp_body"], str) else match["resp_body"]
            self.send_response(match["resp_status"])
            for k, v in match["resp_headers"].items():
                self.send_header(k, v)
            self.send_header("content-length", str(len(resp)))
            self.end_headers()
            self.wfile.write(resp)

        def do_GET(self):
            match = _find_best_match(cassettes, "GET", self.path, {})
            if not match:
                self.send_response(404)
                self.send_header("content-type", "application/json")
                self.end_headers()
                self.wfile.write(json.dumps({"error": f"no cassette for GET {self.path}"}).encode())
                return
            resp = match["resp_body"].encode() if isinstance(match["resp_body"], str) else match["resp_body"]
            self.send_response(match["resp_status"])
            for k, v in match["resp_headers"].items():
                self.send_header(k, v)
            self.send_header("content-length", str(len(resp)))
            self.end_headers()
            self.wfile.write(resp)

        def log_message(self, *args):
            pass

    server = HTTPServer(("127.0.0.1", port), Handler)
    server.serve_forever()


class CassetteServer:
    """Context manager that starts a cassette replay server in a subprocess."""

    def __init__(self, provider):
        self.cassettes = _load_cassettes(provider)
        if not self.cassettes:
            raise RuntimeError(f"No cassettes found for provider: {provider}")
        self.port = _find_free_port()
        self.url = f"http://127.0.0.1:{self.port}"
        self.proc = Process(target=_server_proc, args=(self.port, self.cassettes))

    @property
    def count(self):
        return len(self.cassettes)

    def __enter__(self):
        self.proc.start()
        # Wait for server to be ready
        for _ in range(50):
            try:
                with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
                    s.connect(("127.0.0.1", self.port))
                    break
            except ConnectionRefusedError:
                time.sleep(0.05)
        return self

    def __exit__(self, *args):
        self.proc.terminate()
        self.proc.join(timeout=2)
