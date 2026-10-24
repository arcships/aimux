"""Generated codec checks independent of a native extension or provider."""
import json

import pytest

pytest.importorskip('pydantic')
from pydantic import TypeAdapter, ValidationError
from dto_vectors import types as dto


def test_closed_and_open_objects():
    with pytest.raises(ValidationError, match='extra_forbidden'):
        dto.TextRequest.model_validate({'prompt': 'hello', 'unknown': True})
    assert dto.RequestInfo.model_validate({'unknown': True}).model_dump(by_alias=True) == {}


def test_presence_and_nested_large_integers():
    for value in ({}, {'body': None}, {'body': {'integer': (1 << 64) - 1}}):
        result = dto.RequestInfo.model_validate_json(json.dumps(value))
        assert json.loads(result.model_dump_json(by_alias=True)) == value
    absent = dto.RequestInfo()
    explicit = dto.RequestInfo(body=None)
    assert 'body' not in absent.model_fields_set
    assert 'body' in explicit.model_fields_set


def test_discriminator_and_duplicate_tag_shapes():
    adapter = TypeAdapter(dto.ContentPart)
    for source in (
        {'type': 'source', 'sourceType': 'url', 'id': 's', 'url': 'https://example.com'},
        {'type': 'source', 'sourceType': 'document', 'id': 's', 'title': 'Doc', 'mediaType': 'text/plain'},
    ):
        assert adapter.dump_python(adapter.validate_python(source), by_alias=True) == source
    with pytest.raises(ValidationError, match='future-tag'):
        adapter.validate_python({'type': 'future-tag'})
    with pytest.raises(ValidationError, match='union_tag_not_found'):
        adapter.validate_python({'text': 'missing tag'})


def test_ordered_union_uses_first_matching_branch():
    adapter = TypeAdapter(dto.ModelPrompt)
    assert adapter.validate_python('hello') == 'hello'
    value = [{'role': 'user', 'content': 'hello'}]
    assert adapter.dump_python(adapter.validate_python(value), by_alias=True) == value


def test_wire_codecs_and_integer_ranges():
    assert dto.base64_wire(b'hello') == 'aGVsbG8='
    assert dto.base64_wire('aGVsbG8=', decode=True) == b'hello'
    for value in ('!', 'é', 1):
        with pytest.raises(ValueError):
            dto.base64_wire(value, decode=True)
    adapter = TypeAdapter(dto.HandleId)
    assert adapter.validate_python(str((1 << 64) - 1)) == str((1 << 64) - 1)
    for value in (str(1 << 64), '-1', 1, '１２'):
        with pytest.raises(ValidationError):
            adapter.validate_python(value)
    with pytest.raises(ValidationError):
        dto.ErrorEnvelope(kind='binding', code=1 << 31, message='invalid', error={})
    assert dto.timestamp('2026-10-16T04:00:00Z') == '2026-10-16T04:00:00Z'
    with pytest.raises(ValueError):
        dto.timestamp('2026-10-16T04:00:00')
