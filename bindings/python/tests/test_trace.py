"""Tests for RFC-0015 cache probing through the Python binding.

Verifies: `trace()` wraps a model, probed calls record fingerprints/verdicts,
and the query API returns parsed results. Uses the shared mock HTTP server —
no real API calls.
"""

import json

from aimux import openai
from aimux.wrapper import GenerateTextOptions, generate_text

from test_e2e import MockServer, OPENAI_CHAT


class TestTraceProbe:
    def test_trace_records_calls_and_query_api(self):
        with MockServer(OPENAI_CHAT) as mock:
            raw = openai("test-key", "gpt-4o", mock.url)
            traced = raw.trace_audited(True)

            # > 4 KiB user message so block-aligned prefixes actually match.
            big = "x" * 5000
            generate_text(
                traced,
                [{"role": "user", "content": big}],
                GenerateTextOptions(session_id="sess-1"),
            )
            generate_text(
                traced,
                [
                    {"role": "user", "content": big},
                    {"role": "assistant", "content": "a1"},
                    {"role": "user", "content": "u2"},
                ],
                GenerateTextOptions(session_id="sess-1"),
            )

            stats = json.loads(traced.trace_aggregate())
            assert len(stats) == 1
            assert stats[0]["requests"] == 2
            assert stats[0]["provider"] == "openai"
            assert "verdict_counts" in stats[0]

            chain = json.loads(traced.trace_session_chain("sess-1"))
            assert len(chain["record_ids"]) == 2
            assert chain["prefix_stability"] > 0.5

            jsonl = traced.trace_export_jsonl()
            lines = [l for l in jsonl.strip().split("\n") if l]
            assert len(lines) == 2, "one TraceRecord per line"
            first = json.loads(lines[0])
            assert first["fingerprint"]["body_hash"]
            assert first["session_id"] == "sess-1"

            traced.trace_clear()
            assert traced.trace_export_jsonl().strip() == ""

    def test_untraced_model_rejects_query_api(self):
        with MockServer(OPENAI_CHAT) as mock:
            raw = openai("test-key", "gpt-4o", mock.url)
            try:
                raw.trace_aggregate()
                assert False, "untraced model must reject trace_aggregate"
            except Exception as e:
                assert "not traced" in str(e)

    def test_non_traced_model_generates_normally(self):
        with MockServer(OPENAI_CHAT) as mock:
            model = openai("test-key", "gpt-4o", mock.url)
            result = generate_text(model, "hello")
            assert result.text
