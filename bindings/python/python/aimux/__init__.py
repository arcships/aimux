"""aimux — Unified LLM service layer for Python (Rust core, 325 providers).

This wrapper layer parses JSON strings from the native layer into Python dicts,
providing a Pythonic API surface.
"""

import json
from typing import Any, AsyncIterator, Dict, List, Optional, Union

from .aimux import (
    AimuxError,
    ProviderError,
    HttpError,
    JsonError,
    StreamError,
    ToolError,
    InvalidArgumentError,
    InvalidPromptError,
    RateLimitError,
    AuthenticationError,
    TokenExpiredError,
    ModelNotFoundError,
    UnsupportedError,
    NoSuchModelError,
    UnknownProviderError,
    APICallError,
    APITimeoutError,
    RequestAbortedError,
    OtherError,
    Model,
    StreamIterator,
    openai,
    anthropic,
    deepseek,
    google,
    cohere,
    mistral,
    xai,
    bedrock,
    vertex,
    anthropic_aws,
    azure,
    provider as _native_provider,
    create_provider as _native_create_provider,
    ProviderHandle,
    init_session_store,
    init_session_infer,
    session_calls as _session_calls_json,
    list_sessions as _list_sessions_json,
    EmbeddingModel,
    SpeechModel,
    ImageModel,
    TranscriptionModel,
    RerankingModel,
    VideoModel,
    SearchModel,
    Files,
    openai_embedding,
    openai_speech,
    openai_image,
    openai_transcription,
    openai_files,
    cohere_embedding,
    cohere_reranking,
    google_embedding,
    google_image,
    google_video,
    tavily_search,
)

__all__ = [
    "AimuxError",
    "ProviderError",
    "HttpError",
    "JsonError",
    "StreamError",
    "ToolError",
    "InvalidArgumentError",
    "InvalidPromptError",
    "RateLimitError",
    "AuthenticationError",
    "TokenExpiredError",
    "ModelNotFoundError",
    "UnsupportedError",
    "NoSuchModelError",
    "UnknownProviderError",
    "APICallError",
    "APITimeoutError",
    "RequestAbortedError",
    "OtherError",
    "Model",
    "openai",
    "anthropic",
    "deepseek",
    "google",
    "cohere",
    "mistral",
    "xai",
    "bedrock",
    "vertex",
    "anthropic_aws",
    "azure",
    "provider",
    "create_provider",
    "get_model_specs",
    "ProviderHandle",
    "init_session_store",
    "init_session_infer",
    "session_calls",
    "list_sessions",
    "EmbeddingModel",
    "SpeechModel",
    "ImageModel",
    "TranscriptionModel",
    "RerankingModel",
    "VideoModel",
    "SearchModel",
    "Files",
    "openai_embedding",
    "openai_speech",
    "openai_image",
    "openai_transcription",
    "openai_files",
    "cohere_embedding",
    "cohere_reranking",
    "google_embedding",
    "google_image",
    "google_video",
    "tavily_search",
    "generate_text",
    "stream_text",
    "generate_text_as_openai",
    "stream_text_as_openai",
]


def session_calls(session_id: str) -> List[Dict[str, Any]]:
    """All calls of a session, ordered by step (RFC-0024).

    Empty list if the session is unknown or no store is registered
    (``init_session_store()`` must be called first).
    """
    return json.loads(_session_calls_json(session_id))


def list_sessions() -> List[Dict[str, Any]]:
    """All known sessions (RFC-0024)."""
    return json.loads(_list_sessions_json())


def provider(
    name: str,
    api_key: Optional[str],
    model_id: str,
    base_url: Optional[str] = None,
    config: Optional[Dict[str, Any]] = None,
) -> Model:
    """Create a language model from the built-in registry by provider name.

    Args:
        name: Registry provider name (e.g. "deepseek", "groq").
        api_key: API key; None reads the provider's env var.
        model_id: Model ID.
        base_url: Base-URL override (wins over config["base_url"]).
        config: Full ProviderOptions dict — base_url / headers / organization /
            project / max_retries / body_overrides.
    """
    config_json = json.dumps(config) if config is not None else None
    return _native_provider(name, api_key, model_id, base_url, config_json)


def create_provider(
    name: str,
    api_key: Optional[str] = None,
    base_url: Optional[str] = None,
    config: Optional[Dict[str, Any]] = None,
) -> ProviderHandle:
    """Create a provider handle for model discovery (RFC-0027).

    Unlike ``provider()`` (which binds to a single model_id), this returns a
    ``ProviderHandle`` that supports ``list_models()`` and ``model()``.

    Args:
        name: Registry provider name (e.g. "deepseek", "groq").
        api_key: API key; None reads the provider's env var.
        base_url: Base-URL override (wins over config["base_url"]).
        config: Full ProviderOptions dict — base_url / headers / organization /
            project / max_retries / body_overrides.
    """
    config_json = json.dumps(config) if config is not None else None
    return _native_create_provider(name, api_key, base_url, config_json)


def get_model_specs(source_url: Optional[str] = None) -> Dict[str, Any]:
    """Fetch the community model catalogue (anya2a). Returns a dict representing
    the Catalogue (provider -> model_id -> ModelSpec). Thin fetch — no caching.

    Args:
        source_url: Optional URL override (default = anya2a endpoint).
    """
    return json.loads(_native_get_model_specs(source_url))


def _prompt_to_json(prompt: Union[str, List[Dict[str, Any]]]) -> str:
    """Convert a prompt to the JSON string expected by the native layer."""
    if isinstance(prompt, str):
        return json.dumps(prompt)
    return json.dumps({"prompt": prompt})


def _opts_to_json(options: Optional[Dict[str, Any]]) -> Optional[str]:
    """Convert options dict to JSON string."""
    if options is None:
        return None
    return json.dumps(options)


def generate_text(
    model: Model,
    prompt: Union[str, List[Dict[str, Any]]],
    options: Optional[Dict[str, Any]] = None,
) -> Dict[str, Any]:
    """Generate text (non-streaming). Returns a dict result.

    Args:
        model: A model instance from openai(), anthropic(), etc.
        prompt: A string or a list of message dicts.
        options: Optional generation options.

    Returns:
        Dict with keys: text, tool_calls, finish_reason, usage, warnings, raw.
    """
    prompt_json = _prompt_to_json(prompt)
    opts_json = _opts_to_json(options)
    result_json = model.generate_text(prompt_json, opts_json)
    return json.loads(result_json)


def stream_text(
    model: Model,
    prompt: Union[str, List[Dict[str, Any]]],
    options: Optional[Dict[str, Any]] = None,
):
    """Stream text from a model. Yields StreamPart dicts.

    Usage:
        for part in stream_text(model, "Write a haiku about Rust."):
            if "TextDelta" in part:
                print(part["TextDelta"]["delta"], end="")
    """
    prompt_json = _prompt_to_json(prompt)
    opts_json = _opts_to_json(options)
    iterator = model.stream_text(prompt_json, opts_json)
    for part_json in iterator:
        yield json.loads(part_json)


def generate_text_as_openai(
    model: Model,
    prompt: Union[str, List[Dict[str, Any]]],
    options: Optional[Dict[str, Any]] = None,
) -> Dict[str, Any]:
    """Generate text (non-streaming) and return an OpenAI Chat Completion dict.

    Same as generate_text, but the result is an OpenAI ``chat.completion``
    object (id, object, choices, usage, …). Works with any provider.

    Args:
        model: A model instance from openai(), anthropic(), etc.
        prompt: A string or a list of message dicts.
        options: Optional generation options.

    Returns:
        Dict with OpenAI Chat Completion keys: id, object, created, model,
        choices, usage, system_fingerprint.
    """
    prompt_json = _prompt_to_json(prompt)
    opts_json = _opts_to_json(options)
    result_json = model.generate_text_as_openai(prompt_json, opts_json)
    return json.loads(result_json)


def stream_text_as_openai(
    model: Model,
    prompt: Union[str, List[Dict[str, Any]]],
    options: Optional[Dict[str, Any]] = None,
):
    """Stream text as OpenAI Chat Completion chunk dicts.

    Same as stream_text, but each yielded item is an OpenAI
    ``chat.completion.chunk`` object. Works with any provider.

    Stream options (include_usage, include_reasoning) are read from
    options["provider_options"]["openai"]["stream_options"] (both default True).

    Usage:
        for chunk in stream_text_as_openai(model, "Write a haiku."):
            if chunk.get("choices"):
                delta = chunk["choices"][0].get("delta", {})
                if "content" in delta:
                    print(delta["content"], end="")
    """
    prompt_json = _prompt_to_json(prompt)
    opts_json = _opts_to_json(options)
    iterator = model.stream_text_as_openai(prompt_json, opts_json)
    for chunk_json in iterator:
        yield json.loads(chunk_json)
