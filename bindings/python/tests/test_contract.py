"""Cross-language contract tests for the Python binding.

Consumes the shared `contract-tests/fixtures/wire-format.json` — the same
fixtures asserted by Rust `contract_test.rs`, Node `run-node.ts`, Go
`wire_format_test.go` and Java/Kotlin `ContractTest`.

Pure model-layer tests: no native library needed.
"""

import json
from pathlib import Path

from aimux.wrapper import GenerateTextOptions, parse_stream_part

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
