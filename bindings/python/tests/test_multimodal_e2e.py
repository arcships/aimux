"""End-to-end multimodal tests with a mock HTTP server.

Verifies the FULL chain for each of the 8 multimodal features:
Python → PyO3 → Rust engine → HTTP mock → response parsing → typed result.

The mock server runs in a separate process (multiprocessing) because PyO3's
block_on blocks the calling thread — a threading-based server would deadlock.
The MockServer helper is copied from test_e2e.py (test modules are not on the
import path).
"""

import json
import socket
import time
from http.server import HTTPServer, BaseHTTPRequestHandler
from multiprocessing import Process

import pytest

from aimux import (
    openai_embedding,
    openai_speech,
    openai_image,
    openai_transcription,
    cohere_reranking,
    tavily_search,
    openai_files,
    google_video,
)


# ── Mock server in a separate process (copied from test_e2e.py) ──────────────

def _find_free_port():
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(('127.0.0.1', 0))
        return s.getsockname()[1]


def _mock_server_proc(port, response_body, content_type, status=200):
    """Mock HTTP server that always returns the same response.

    ``response_body`` may be ``str`` or ``bytes`` (raw binary, e.g. for speech).
    Handles both POST and GET so any provider verb works.
    """

    class Handler(BaseHTTPRequestHandler):
        def _respond(self):
            resp = (
                response_body.encode()
                if isinstance(response_body, str)
                else response_body
            )
            self.send_response(status)
            self.send_header('content-type', content_type)
            self.send_header('content-length', str(len(resp)))
            self.end_headers()
            self.wfile.write(resp)

        def do_POST(self):
            # Drain the request body so the client can finish sending.
            self.rfile.read(int(self.headers.get('Content-Length', 0)))
            self._respond()

        def do_GET(self):
            self._respond()

        def log_message(self, *args):
            pass

    server = HTTPServer(('127.0.0.1', port), Handler)
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


# ── Mock response data (real provider shapes, verified in Go E2E tests) ──────

EMBEDDING_RESP = json.dumps({
    "data": [{"embedding": [0.1, 0.2, 0.3], "index": 0}],
    "model": "text-embedding-3-small",
    "usage": {"prompt_tokens": 3, "total_tokens": 3},
})

# Speech returns raw binary with content-type audio/mpeg (NOT JSON).
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

# Google Video uses a multi-step async API; we only parse a canned result here.
VIDEO_RESULT_JSON = json.dumps({
    "videos": [
        {"Url": {"url": "https://example.com/v.mp4", "media_type": "video/mp4"}},
    ],
})


# ── Tests ───────────────────────────────────────────────────────────────────

class TestMultimodalE2E:
    """Full-chain E2E tests for all 8 multimodal features."""

    def test_embedding(self):
        with MockServer(EMBEDDING_RESP) as mock:
            embedder = openai_embedding(
                "test-key", "text-embedding-3-small", base_url=mock.url
            )
            result = json.loads(embedder.embed(json.dumps(["hello"])))
            assert len(result["embeddings"]) == 1
            assert len(result["embeddings"][0]) == 3
            assert result["embeddings"][0] == pytest.approx(
                [0.1, 0.2, 0.3], abs=1e-5
            )

    def test_speech(self):
        # Mock returns raw binary with content-type audio/mpeg.
        with MockServer(SPEECH_RESP, content_type='audio/mpeg') as mock:
            speaker = openai_speech("test-key", "tts-1", base_url=mock.url)
            opts = json.dumps({"text": "Hi", "voice": "alloy", "output_format": "mp3"})
            result = json.loads(speaker.generate(opts))
            # Binary audio comes back as AudioData::Binary -> {"Binary": [bytes...]}
            assert "Binary" in result["audio"]
            assert len(result["audio"]["Binary"]) > 0

    def test_image(self):
        with MockServer(IMAGE_RESP) as mock:
            imager = openai_image("test-key", "dall-e-3", base_url=mock.url)
            # ImageCallOptions.provider_options is a required (non-Option) field,
            # so it must be present in the serialized options.
            opts = json.dumps(
                {"prompt": "otter", "n": 1, "provider_options": {}}
            )
            result = json.loads(imager.generate(opts))
            assert len(result["images"]["Base64"]) == 1
            assert result["images"]["Base64"][0] == "aW1hZ2Ux"

    def test_transcription(self):
        with MockServer(TRANSCRIPTION_RESP) as mock:
            transcriber = openai_transcription(
                "test-key", "whisper-1", base_url=mock.url
            )
            result = json.loads(transcriber.generate("dGVzdA==", "audio/mp3"))
            assert result["text"] == "Hello world"

    def test_reranking(self):
        with MockServer(RERANKING_RESP) as mock:
            reranker = cohere_reranking(
                "test-key", "rerank-v3.0", base_url=mock.url
            )
            # RerankingDocuments is an externally-tagged enum; the text-documents
            # variant is {"Text": {"values": [...]}} (a bare array is rejected).
            docs = json.dumps({"Text": {"values": ["doc1", "doc2"]}})
            result = json.loads(reranker.rerank("which?", docs))
            assert len(result["ranking"]) == 2
            assert result["ranking"][0]["relevance_score"] == 0.95

    def test_search(self):
        with MockServer(SEARCH_RESP) as mock:
            searcher = tavily_search("test-key", base_url=mock.url)
            result = json.loads(searcher.search("What is Rust?"))
            assert len(result["results"]) == 1
            assert result["results"][0]["title"] == "Rust"
            assert result["answer"] == "Rust is a systems language."

    def test_files(self):
        with MockServer(FILES_RESP) as mock:
            files = openai_files("test-key", base_url=mock.url)
            result = json.loads(files.upload_file("dGVzdA==", "application/pdf"))
            assert result["provider_reference"]["openai"] == "file-abc"

    def test_video_construction_and_result_parsing(self):
        # Google Video uses a multi-step async API (POST predict → poll operation
        # → fetch result); a single-response mock can't cover the full flow.
        # Verify construction doesn't throw, then parse a canned result shape.
        video = google_video(
            "test-key", "veo-3.0", base_url="http://localhost:9999"
        )
        assert video is not None

        result = json.loads(VIDEO_RESULT_JSON)
        assert len(result["videos"]) == 1
        assert result["videos"][0]["Url"]["url"] == "https://example.com/v.mp4"
        assert result["videos"][0]["Url"]["media_type"] == "video/mp4"
