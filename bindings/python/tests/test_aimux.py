"""Tests for aimux Python binding.

These tests do NOT make real API calls — they test the module surface
and error handling.
"""

import json
import pytest


def test_module_imports():
    """Native module loads and exports functions."""
    from aimux import openai, anthropic, deepseek, provider, generate_text, stream_text

    assert callable(openai)
    assert callable(anthropic)
    assert callable(deepseek)
    assert callable(provider)
    assert callable(generate_text)
    assert callable(stream_text)


def test_provider_creates_registry_model():
    """provider() creates a registry model instance (RFC-0017 phase 4)."""
    from aimux import provider

    model = provider("groq", "sk-test-fake-key", "llama-3.3-70b")
    assert model is not None
    assert hasattr(model, "generate_text")
    assert hasattr(model, "stream_text")


def test_provider_unknown_name_raises():
    """provider() rejects unknown names, naming the one that did not resolve."""
    from aimux import provider

    with pytest.raises(Exception, match="no-such-provider"):
        provider("no-such-provider", "k", "m")


def test_provider_accepts_full_config():
    """provider() accepts a full ProviderOptions config dict (RFC-0017 §3.4)."""
    from aimux import provider

    model = provider(
        "groq",
        "sk-test-fake-key",
        "llama-3.3-70b",
        config={
            "base_url": "https://example.com/v1",
            "headers": {"X-Custom": "1"},
            "organization": "org-1",
            "project": "proj-1",
        },
    )
    assert model is not None
    assert hasattr(model, "generate_text")


@pytest.mark.parametrize(
    "key,value",
    [("max_retries", 0), ("body_overrides", {"temperature": 0.1})],
)
def test_provider_rejects_removed_config_keys(key, value):
    """max_retries is call-level and body_overrides is gone: neither is silently ignored."""
    from aimux import provider

    with pytest.raises(Exception, match=key):
        provider("groq", "sk-test-fake-key", "llama-3.3-70b", config={key: value})


@pytest.mark.parametrize(
    "key,value",
    [("max_retries", 0), ("body_overrides", {"temperature": 0.1})],
)
def test_create_provider_rejects_removed_config_keys(key, value):
    """create_provider() goes through the same ProviderOptions check."""
    from aimux import InvalidArgumentError, create_provider

    with pytest.raises(InvalidArgumentError, match=key):
        create_provider("groq", "sk-test-fake-key", config={key: value})


def test_provider_base_url_param_wins_over_config():
    """Explicit base_url parameter overrides config["base_url"]."""
    from aimux import provider

    model = provider(
        "groq",
        "sk-test-fake-key",
        "llama-3.3-70b",
        base_url="https://param.example.com/v1",
        config={"base_url": "https://config.example.com/v1"},
    )
    assert model is not None


def test_provider_invalid_config_raises():
    """provider() rejects a config with wrong field types."""
    from aimux import provider

    with pytest.raises(Exception, match="invalid config"):
        provider(
            "abacus",
            "sk-test-fake-key",
            "llama-3.3-70b",
            config={"headers": "not-a-map"},
        )


def test_provider_missing_env_key_raises(monkeypatch):
    """provider() with api_key=None reads the env var at call time and fails clearly when unset."""
    from aimux import LoadAPIKeyError, generate_text, provider

    monkeypatch.delenv("ABACUS_API_KEY", raising=False)
    model = provider("abacus", None, "m")
    with pytest.raises(LoadAPIKeyError, match="(?i)api key"):
        generate_text(model, "hi", {"maxRetries": 0})


def test_provider_params_fill_a_preset_template(monkeypatch):
    """config["params"] reaches the preset's template parameters."""
    from aimux import InvalidArgumentError, generate_text, provider

    monkeypatch.delenv("CLOUDFLARE_ACCOUNT_ID", raising=False)
    # Without the parameter the preset cannot build its base URL (reported at call time).
    unfilled = provider("cloudflare_workers_ai", "sk-test-fake-key", "m")
    with pytest.raises(InvalidArgumentError, match="account_id"):
        generate_text(unfilled, "hi", {"maxRetries": 0})
    model = provider(
        "cloudflare_workers_ai",
        "sk-test-fake-key",
        "m",
        config={"params": {"account_id": "acct123"}},
    )
    assert model is not None


def test_provider_params_the_preset_does_not_declare_are_rejected():
    from aimux import InvalidArgumentError, provider

    with pytest.raises(InvalidArgumentError, match="no template parameter `account_id`"):
        provider(
            "abacus",
            "sk-test-fake-key",
            "m",
            config={"params": {"account_id": "acct123"}},
        )


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
    """A wire-JSON text that does not parse is the binding's own failure.

    pyo3 style: a ValueError naming the argument, never the engine's
    InvalidPromptError.
    """
    from aimux import openai

    model = openai("sk-test-fake-key", "gpt-4o-mini")
    with pytest.raises(ValueError, match=r"^prompt_json: invalid JSON"):
        model.generate_text("{invalid json}")


def test_stream_text_returns_iterator():
    """stream_text returns a StreamIterator."""
    from aimux import openai

    model = openai("sk-test-fake-key", "gpt-4o-mini")
    it = model.stream_text('"hello"')
    # Should be iterable
    assert hasattr(it, "__iter__")
    assert hasattr(it, "__next__")
