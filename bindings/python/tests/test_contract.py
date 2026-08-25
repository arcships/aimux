"""Cross-language contract tests for the Python binding.

Consumes the shared `contract-tests/fixtures/wire-format.json` — the same
fixtures asserted by Rust `contract_test.rs`, Node `run-node.ts`, Go
`wire_format_test.go` and Java/Kotlin `ContractTest`.

Pure model-layer tests: no native library needed.
"""

import json
from pathlib import Path

from aimux.wrapper import GenerateContent, GenerateTextOptions, ModelMessage, parse_stream_part

_REPO_ROOT = Path(__file__).resolve().parents[3]


def _fixtures():
    path = _REPO_ROOT / "contract-tests" / "fixtures" / "wire-format.json"
    return json.loads(path.read_text(encoding="utf-8"))


def _fixture_json(name):
    for f in _fixtures():
        if f["name"] == name:
            return f["json"]
    raise AssertionError(f"no fixture named {name!r}")


def test_stream_part_raw_fixture_decodes_to_raw():
    """RFC-0016 M2: the shared stream_part_raw fixture parses into the typed
    Raw variant with the payload intact."""
    sp = parse_stream_part(json.loads(_fixture_json("stream_part_raw")))
    assert sp.root.type == "Raw"
    assert sp.root.raw_value["id"] == "c1"
    assert sp.root.raw_value["choices"] == []


def test_include_raw_chunks_true_fixture_roundtrips():
    """RFC-0016 M2 true-case: include_raw_chunks:true survives typed parse
    and re-serialization."""
    opts = GenerateTextOptions.model_validate_json(
        _fixture_json("generate_text_options_include_raw_chunks_true")
    )
    assert opts.include_raw_chunks is True
    assert '"include_raw_chunks":true' in opts.model_dump_json(exclude_none=True)


def test_generate_text_options_default_fixture_all_null():
    """Default options: include_raw_chunks must be null like every other
    field (RFC-0016 M2 default is off)."""
    opts = GenerateTextOptions.model_validate_json(
        _fixture_json("generate_text_options_default")
    )
    assert opts.include_raw_chunks is None


def test_numeric_options_fixture_keeps_precision():
    """The regression lock for ``top_k``.

    The value is asserted rather than merely decoded: Python would happily
    accept 40.5 into a float either way, but asserting it means a fixture
    narrowed back to an integer is caught here too.
    """
    opts = GenerateTextOptions.model_validate_json(
        _fixture_json("generate_text_options_numeric_types")
    )
    assert opts.top_k == 40.5
    assert opts.frequency_penalty == -0.5
    assert opts.temperature == 0.7
    assert opts.top_p == 0.95
    assert opts.max_output_tokens == 256
    assert opts.seed == 42
    assert opts.max_retries == 3


def test_generate_content_fixtures_decode_into_typed_variants():
    """Every GenerateContent fixture decodes into the typed union.

    Decoding alone is the check that matters here: the union is discriminated,
    so a variant whose shape drifted stops matching and validation fails. The
    count is asserted so a newly added fixture cannot slip past this test
    unnoticed.
    """
    fixtures = [f for f in _fixtures() if f["type"] == "GenerateContent"]
    assert len(fixtures) == 8, f"expected 8 GenerateContent fixtures, saw {len(fixtures)}"

    by_name = {}
    for f in fixtures:
        content = GenerateContent.model_validate(json.loads(f["json"]))
        by_name[f["name"]] = content.root

    # Spot-check the shapes the maintainer flagged as invisible to a
    # parse-only check: the tool-call input, the nested file union, and
    # Source's optionals.
    tool_call = by_name["generate_content_tool_call"]
    assert tool_call.tool_call_id == "call_1"
    assert tool_call.input == '{"city":"Paris"}'
    assert tool_call.provider_executed is True

    file_part = by_name["generate_content_file"]
    assert file_part.media_type == "image/png"

    source = by_name["generate_content_source_unset_optionals"]
    assert source.url is None
    assert source.title is None


def test_provider_executed_tool_transcript_message_fixture_roundtrips():
    message = ModelMessage.model_validate_json(
        _fixture_json("model_message_provider_executed_tool_transcript")
    )
    call, result = message.content
    assert call.provider_executed is True
    assert call.tool_name == "search"
    assert result.tool_name == "search"
    assert result.is_error is False
    assert result.preliminary is True
    assert result.dynamic is True

    reencoded = message.model_dump_json(exclude_none=True)
    decoded = ModelMessage.model_validate_json(reencoded)
    assert decoded == message
