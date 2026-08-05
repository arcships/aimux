"""Real cassette replay tests for Python binding.

Hard asserts only — no catch-all pass. Providers with non-standard paths
are excluded, not faked.
"""

import json
import pytest

from aimux import openai, anthropic, generate_text, stream_text
from tests.cassette_replay import CassetteServer


class TestOpenAICassette:

    def test_generate(self):
        with CassetteServer("openai") as srv:
            assert srv.count > 0
            model = openai("test-key", "gpt-4o", f"{srv.url}/v1")
            result = generate_text(model, "Hello")
            assert "error" not in result, f"unexpected error: {result.get('error')}"
            assert isinstance(result["text"], str)
            assert len(result["text"]) > 0, "text should be non-empty"
            assert "usage" in result
            assert "finish_reason" in result

    def test_stream(self):
        with CassetteServer("openai") as srv:
            model = openai("test-key", "gpt-4o", f"{srv.url}/v1")
            parts = list(stream_text(model, "Hello"))
            assert len(parts) > 0, "should receive stream parts"
            types = [list(p.keys())[0] for p in parts]
            assert "StreamStart" in types, "should have StreamStart"
            assert "Finish" in types, "should have Finish"


class TestAnthropicCassette:

    def test_generate(self):
        with CassetteServer("anthropic") as srv:
            assert srv.count > 0
            model = anthropic("test-key", "claude-sonnet-4-6", f"{srv.url}/v1")
            result = generate_text(model, "Hello")
            assert "error" not in result, f"unexpected error: {result.get('error')}"
            assert isinstance(result["text"], str)
            assert len(result["text"]) > 0, "text should be non-empty"
            assert "usage" in result

    def test_stream(self):
        with CassetteServer("anthropic") as srv:
            model = anthropic("test-key", "claude-3-haiku-20240307", f"{srv.url}/v1")
            parts = list(stream_text(model, "Hello"))
            assert len(parts) > 0, "should receive stream parts"


class TestDeepSeekCassette:

    def test_generate(self):
        with CassetteServer("deepseek") as srv:
            assert srv.count > 0
            model = openai("test-key", "deepseek-chat", srv.url)
            result = generate_text(model, "Hello")
            assert "error" not in result, f"unexpected error: {result.get('error')}"
            assert isinstance(result["text"], str)

    def test_generate_usage_raw_carries_vendor_fields(self):
        """RFC-0016 M10: DeepSeek's vendor-specific usage fields survive in
        usage.raw (they are NOT part of the typed Usage model)."""
        with CassetteServer("deepseek") as srv:
            model = openai("test-key", "deepseek-chat", srv.url)
            result = generate_text(model, "Hello")

        raw = result.get("usage", {}).get("raw")
        assert raw, "usage.raw should be populated (RFC-0016 M10)"
        assert isinstance(raw.get("prompt_cache_hit_tokens"), int)
        assert isinstance(raw.get("prompt_cache_miss_tokens"), int)
        # Typed totals still work alongside the raw object.
        assert result["usage"]["input_tokens"]["total"] is not None


class TestGroqCassette:

    def test_generate(self):
        with CassetteServer("groq") as srv:
            assert srv.count > 0
            model = openai("test-key", "llama-3.3-70b-versatile", f"{srv.url}/openai/v1")
            result = generate_text(model, "Hello")
            assert "error" not in result, f"unexpected error: {result.get('error')}"
            assert isinstance(result["text"], str)


class TestMistralCassette:

    def test_generate(self):
        with CassetteServer("mistral") as srv:
            assert srv.count > 0
            model = openai("test-key", "ministral-8b-latest", f"{srv.url}/v1")
            result = generate_text(model, "Hello")
            assert "error" not in result, f"unexpected error: {result.get('error')}"
            assert isinstance(result["text"], str)


class TestOllamaCassette:

    def test_generate(self):
        with CassetteServer("ollama") as srv:
            assert srv.count > 0
            model = openai("test-key", "qwen3:4b", f"{srv.url}/v1")
            result = generate_text(model, "Hello")
            assert "error" not in result, f"unexpected error: {result.get('error')}"
            assert isinstance(result["text"], str)


class TestPerplexityCassette:

    def test_generate(self):
        with CassetteServer("perplexity") as srv:
            assert srv.count > 0
            model = openai("test-key", "sonar", srv.url)
            result = generate_text(model, "Hello")
            assert "error" not in result, f"unexpected error: {result.get('error')}"
            assert isinstance(result["text"], str)
