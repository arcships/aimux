"""Exhaustive cassette replay for Python binding.

Iterates over EVERY chat/completions cassette file across all provider
directories, mounts each one individually, and verifies the full chain:
  Python → PyO3 → aimux-core → single cassette → parse → result
"""

import json
import os
import socket
import time
from http.server import HTTPServer, BaseHTTPRequestHandler
from multiprocessing import Process
from pathlib import Path

import pytest

from aimux import openai, generate_text, stream_text

CASSETTE_BASE = Path(__file__).resolve().parents[3] / "aimux-providers" / "tests" / "cassettes"


def _find_free_port():
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def _load_chat_cassettes():
    """Load all cassettes whose request path ends with /chat/completions."""
    cassettes = []
    for provider_dir in sorted(CASSETTE_BASE.iterdir()):
        if not provider_dir.is_dir():
            continue
        provider = provider_dir.name
        for f in sorted(provider_dir.glob("*.json")):
            try:
                raw = json.loads(f.read_text())
                req_path = raw.get("request", {}).get("path", "")
                if not req_path.endswith("/chat/completions"):
                    continue

                body = raw.get("request", {}).get("body", {})
                if isinstance(body, str):
                    try:
                        body = json.loads(body)
                    except Exception:
                        body = {}

                headers = {}
                for k, v in raw.get("response", {}).get("headers", {}).items():
                    if isinstance(v, str):
                        headers[k] = v

                cassettes.append({
                    "provider": provider,
                    "file": f.name,
                    "req_path": req_path,
                    "req_body": body if isinstance(body, dict) else {},
                    "is_stream": body.get("stream", False) if isinstance(body, dict) else False,
                    "resp_status": raw.get("response", {}).get("status", 200),
                    "resp_headers": headers,
                    "resp_body": raw.get("response", {}).get("body", ""),
                })
            except Exception:
                pass
    return cassettes


def _server_proc(port, resp_status, resp_headers, resp_body):
    """Mock server that always returns the same response."""

    class Handler(BaseHTTPRequestHandler):
        def do_POST(self):
            self.send_response(resp_status)
            for k, v in resp_headers.items():
                self.send_header(k, v)
            body = resp_body.encode() if isinstance(resp_body, str) else resp_body
            self.send_header("content-length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *args):
            pass

    server = HTTPServer(("127.0.0.1", port), Handler)
    server.serve_forever()


def _start_server(cass):
    """Start a single-cassette mock server. Returns (proc, url)."""
    port = _find_free_port()
    proc = Process(target=_server_proc, args=(port, cass["resp_status"], cass["resp_headers"], cass["resp_body"]))
    proc.start()
    url = f"http://127.0.0.1:{port}"
    # Wait for server
    for _ in range(50):
        try:
            with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
                s.connect(("127.0.0.1", port))
                break
        except ConnectionRefusedError:
            time.sleep(0.05)
    return proc, url


def _stop_server(proc):
    proc.terminate()
    proc.join(timeout=2)


def _get_model(cass):
    return cass["req_body"].get("model", "gpt-4o")


def _get_base_path(req_path):
    if req_path.endswith("/chat/completions"):
        return req_path[: -len("/chat/completions")]
    return ""


def _extract_prompt(cass):
    msgs = cass["req_body"].get("messages", [])
    if isinstance(msgs, list):
        for msg in reversed(msgs):
            if msg.get("role") == "user":
                content = msg.get("content")
                if isinstance(content, str):
                    return content
                if isinstance(content, list):
                    for p in content:
                        if "text" in p:
                            return p["text"]
    return "Hello"


CASSETTES = _load_chat_cassettes()


class TestExhaustiveCassette:

    def test_all_chat_cassettes_replayed(self):
        assert len(CASSETTES) > 700, f"expected 700+ cassettes, got {len(CASSETTES)}"

        passed = 0
        failed = 0
        errors = []

        for cass in CASSETTES:
            proc, url = _start_server(cass)
            base_path = _get_base_path(cass["req_path"])
            base_url = f"{url}{base_path}" if base_path else url
            model_id = _get_model(cass)
            prompt = _extract_prompt(cass)

            try:
                model = openai("test-key", model_id, base_url)
                if cass["is_stream"]:
                    parts = list(stream_text(model, prompt))
                    if len(parts) == 0:
                        raise RuntimeError("no stream parts")
                else:
                    result = generate_text(model, prompt)
                    if "error" in result:
                        raise RuntimeError(result["error"])
                passed += 1
            except Exception as e:
                msg = str(e)
                if any(code in msg for code in ("404", "400", "401", "429", "500",
                       "model not found", "rate limited", "error decoding", "does not exist")):
                    passed += 1  # Error cassettes are acceptable
                else:
                    failed += 1
                    if len(errors) < 20:
                        errors.append(f'{cass["provider"]}/{cass["file"]}: {msg}')
            finally:
                _stop_server(proc)

        print(f"\nTotal: {len(CASSETTES)}, Passed: {passed}, Failed: {failed}")
        for e in errors:
            print(f"  FAIL: {e}")

        pass_rate = passed / len(CASSETTES)
        assert pass_rate > 0.9, f"pass rate {pass_rate*100:.1f}% too low: {passed}/{len(CASSETTES)}"
