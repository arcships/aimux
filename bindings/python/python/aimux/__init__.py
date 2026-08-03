"""aimux — Unified LLM service layer for Python (Rust core, 325 providers).

This wrapper layer parses JSON strings from the native layer into Python dicts,
providing a Pythonic API surface.
"""

import json
from typing import Any, AsyncIterator, Dict, List, Optional, Union

from .aimux import (
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
    provider,
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
]


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
