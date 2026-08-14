"""Response-side reasoning signature visibility for the Python binding (#135).

The Python binding constructs prompts as plain JSON dicts, so input-side
transparency is guaranteed by serde (asserted at the Rust level). What this
test proves is the *visibility* half of the round-trip: a thinking-block
response surfaces `signature` on the reasoning content of the result, so
extended-thinking multi-turn can echo it back.
"""

import json

from aimux import anthropic, generate_text

from test_e2e import MockServer

ANTHROPIC_THINKING = json.dumps({
    "id": "msg_py_rt",
    "type": "message",
    "role": "assistant",
    "model": "claude-sonnet-4-20250514",
    "content": [
        {"type": "thinking", "thinking": "pondering", "signature": "sig-py-resp-1"},
        {"type": "text", "text": "The answer."},
    ],
    "stop_reason": "end_turn",
    "usage": {"input_tokens": 10, "output_tokens": 5},
})


class TestReasoningSignatureVisibility:

    def test_result_carries_response_signature(self):
        with MockServer(ANTHROPIC_THINKING) as mock:
            model = anthropic("test-key", "claude-sonnet-4-20250514", mock.url)
            result = generate_text(model, "Hello")
            assert result["text"] == "The answer."
            dumped = json.dumps(result)
            assert "sig-py-resp-1" in dumped, (
                "expected the result to carry the thinking-block signature, "
                f"got {dumped}"
            )
