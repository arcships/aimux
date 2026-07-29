"""Tests for aimux Python binding.

These tests do NOT make real API calls — they test the module surface
and error handling.
"""

import json
import pytest


def test_module_imports():
    """Native module loads and exports functions."""
    from aimux import openai, anthropic, deepseek, generate_text, stream_text

    assert callable(openai)
    assert callable(anthropic)
    assert callable(deepseek)
    assert callable(generate_text)
    assert callable(stream_text)


def test_openai_creates_model():
    """openai() creates a model instance."""
    from aimux import openai

    model = openai("sk-test-fake-key", "gpt-4o-mini")
    assert model is not None
    assert hasattr(model, "generate_text")
    assert hasattr(model, "stream_text")


def test_anthropic_creates_model():
    """anthropic() creates a model instance."""
    from aimux import anthropic

    model = anthropic("sk-ant-test-fake-key", "claude-3-5-sonnet-20241022")
    assert model is not None
    assert hasattr(model, "generate_text")


def test_deepseek_creates_model():
    """deepseek() creates a model instance."""
    from aimux import deepseek

    model = deepseek("sk-test-fake-key", "deepseek-chat")
    assert model is not None
    assert hasattr(model, "stream_text")


def test_generate_text_rejects_invalid_prompt():
    """generate_text raises on invalid prompt JSON."""
    from aimux import openai

    model = openai("sk-test-fake-key", "gpt-4o-mini")
    with pytest.raises(Exception, match="invalid prompt"):
        model.generate_text("{invalid json}")


def test_stream_text_returns_iterator():
    """stream_text returns a StreamIterator."""
    from aimux import openai

    model = openai("sk-test-fake-key", "gpt-4o-mini")
    it = model.stream_text('"hello"')
    # Should be iterable
    assert hasattr(it, "__iter__")
    assert hasattr(it, "__next__")
