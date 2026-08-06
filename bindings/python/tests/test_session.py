"""Tests for RFC-0024 session grouping through the Python binding.

Verifies: `session_id` in typed options reaches the core, calls are grouped in
the registered SessionStore, and the query API returns parsed results. Uses
the shared mock HTTP server — no real API calls.
"""

import pytest

from aimux import (
    openai,
    init_session_store,
    init_session_infer,
    session_calls,
    list_sessions,
)
from aimux.wrapper import GenerateTextOptions, generate_text

from test_e2e import MockServer, OPENAI_CHAT


class TestSessionGrouping:
    """Explicit session_id groups calls; query API returns them."""

    def test_explicit_session_groups_calls(self):
        init_session_store()
        init_session_infer(False)

        with MockServer(OPENAI_CHAT) as mock:
            model = openai("test-key", "gpt-4o", mock.url)

            generate_text(model, "first", GenerateTextOptions(session_id="sess-1"))
            generate_text(model, "second", GenerateTextOptions(session_id="sess-1"))

            calls = session_calls("sess-1")
            assert len(calls) == 2
            assert [c["step"] for c in calls] == [0, 1]
            assert calls[0]["call_id"] != calls[1]["call_id"]
            assert calls[0]["recorded_at"].endswith("Z")

            # A call without session_id (inference off) is not grouped.
            generate_text(model, "third")
            sessions = list_sessions()
            assert len(sessions) == 1
            assert sessions[0]["session_id"] == "sess-1"
            assert sessions[0]["source"] == "Explicit"

            # Unknown session → empty.
            assert session_calls("nope") == []

            # Separate explicit session.
            generate_text(model, "other", GenerateTextOptions(session_id="sess-2"))
            assert len(list_sessions()) == 2
            assert len(session_calls("sess-2")) == 1

    def test_opt_in_inferer_groups_prefix_continuations(self):
        init_session_store()
        init_session_infer(True)

        with MockServer(OPENAI_CHAT) as mock:
            model = openai("test-key", "gpt-4o", mock.url)

            generate_text(model, "u1")
            generate_text(
                model,
                [
                    {"role": "user", "content": "u1"},
                    {"role": "assistant", "content": "a1"},
                    {"role": "user", "content": "u2"},
                ],
            )

            sessions = list_sessions()
            autos = [s for s in sessions if s["session_id"].startswith("auto-")]
            assert len(autos) == 1, "prefix continuation stays in one auto session"
            assert autos[0]["source"] == "Inferred"
            assert len(autos[0]["calls"]) == 2
