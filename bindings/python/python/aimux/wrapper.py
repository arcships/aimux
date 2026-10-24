"""Typed operation helpers; requires the optional Pydantic dependency."""
from __future__ import annotations

from typing import Iterator

from pydantic import BaseModel, TypeAdapter

from . import Handle, ops
from ._types import *
from ._types import __all__ as _type_names

__all__ = _type_names + [
    "generate_text", "stream_text", "parse_stream_part",
    "generate_text_as_openai", "stream_text_as_openai",
]


def generate_text(model: Handle, request: TextRequest) -> GenerateTextResult:
    """Execute text.generate with a generated request DTO."""
    return GenerateTextResult.model_validate(
        model.call(ops.OP_TEXT_GENERATE, request.model_dump(mode="json", by_alias=True))
    )


_FRAME = TypeAdapter(StreamFrame)
_STREAM_PART = TypeAdapter(TextStreamPart)


def stream_text(model: Handle, request: TextRequest) -> Iterator[BaseModel]:
    """Yield generated event DTOs; closing the iterator cancels the stream."""
    with model.stream(ops.OP_TEXT_STREAM, request.model_dump(mode="json", by_alias=True)) as stream:
        for frame in stream:
            yield _FRAME.validate_python(frame).event


def parse_stream_part(part: dict) -> BaseModel:
    return _STREAM_PART.validate_python(part)


def generate_text_as_openai(model: Handle, request: TextRequest) -> ChatCompletion:
    return ChatCompletion.model_validate(
        model.call(ops.OP_TEXT_GENERATE_OPEN_AI, request.model_dump(mode="json", by_alias=True))
    )


def stream_text_as_openai(model: Handle, request: TextRequest) -> Iterator[ChatCompletionChunk]:
    with model.stream(ops.OP_TEXT_STREAM_OPEN_AI, request.model_dump(mode="json", by_alias=True)) as stream:
        for frame in stream:
            yield _FRAME.validate_python(frame).event
