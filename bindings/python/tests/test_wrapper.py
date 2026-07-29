"""Tests for the typed Pydantic wrapper (aimux.wrapper).

Verifies the FULL chain through the typed layer: typed options / prompt models
→ JSON boundary → PyO3 → Rust engine → mock HTTP → typed result objects.

Reuses the mock-server fixtures (MockServer / RecordingMockServer /
SequencedMockServer) and canned provider responses from test_e2e.py.
"""

import json

import pytest

from aimux import openai, anthropic
from aimux.wrapper import (
    GenerateTextOptions,
    GenerateTextResult,
    ModelMessage,
    TextContentPart,
    FunctionTool,
    ToolChoiceTool,
    StreamPart,
    generate_text,
    stream_text,
    parse_stream_part,
    _opts_to_json,
)

# Reuse the mock servers + canned responses from the e2e suite.
from test_e2e import (
    MockServer,
    RecordingMockServer,
    SequencedMockServer,
    OPENAI_CHAT,
    OPENAI_STREAM,
    OPENAI_TOOL_CALL,
    ANTHROPIC_MSG,
)


# ── generate_text: typed result ──────────────────────────────────────────────

class TestGenerateText:

    def test_returns_typed_result_with_text(self):
        with MockServer(OPENAI_CHAT) as mock:
            model = openai("test-key", "gpt-4o", mock.url)
            result = generate_text(model, "What is Rust?")

        assert isinstance(result, GenerateTextResult)
        assert result.text == "Rust is a systems programming language."
        # Usage is parsed into a model, not left as a dict.
        assert result.usage.input_tokens.total == 10
        assert result.usage.output_tokens.total == 8

    def test_parses_tool_calls(self):
        with RecordingMockServer(OPENAI_TOOL_CALL) as mock:
            model = openai("test-key", "gpt-4o", mock.url)
            opts = GenerateTextOptions(
                tools=[
                    FunctionTool(
                        name="get_weather",
                        description="Get weather for a location",
                        input_schema={
                            "type": "object",
                            "properties": {"location": {"type": "string"}},
                            "required": ["location"],
                        },
                    ),
                ],
            )
            result = generate_text(model, "What's the weather in Tokyo?", opts)

        # Convenience field: tool_calls extracted as typed ToolCall objects.
        assert isinstance(result.tool_calls, list)
        assert len(result.tool_calls) == 1
        tc = result.tool_calls[0]
        assert tc.tool_name == "get_weather"
        assert tc.tool_call_id == "call_abc"
        assert tc.input == {"location": "Tokyo"}

        # raw.content carries the ToolCall variant as a typed GenerateContent.
        assert isinstance(result.raw.content, list)
        tool_call_parts = [c for c in result.raw.content if c.root.type == "ToolCall"]
        assert tool_call_parts, "raw.content must contain a ToolCall variant"
        assert tool_call_parts[0].root.tool_name == "get_weather"
        assert tool_call_parts[0].root.tool_call_id == "call_abc"
        assert tool_call_parts[0].root.input == {"location": "Tokyo"}

    def test_raw_content_is_text_variant(self):
        with MockServer(OPENAI_CHAT) as mock:
            model = openai("test-key", "gpt-4o", mock.url)
            result = generate_text(model, "What is Rust?")

        # raw.content[0] is a typed Text content part.
        first = result.raw.content[0]
        assert first.root.type == "Text"
        assert first.root.text == "Rust is a systems programming language."


# ── stream_text: typed-ish dicts ─────────────────────────────────────────────

class TestStreamText:

    def test_yields_text_delta_dicts(self):
        with MockServer(OPENAI_STREAM, content_type="text/event-stream") as mock:
            model = openai("test-key", "gpt-4o", mock.url)
            parts = list(stream_text(model, "Say hello"))

        assert len(parts) > 0
        text = "".join(p["TextDelta"]["delta"] for p in parts if "TextDelta" in p)
        assert text == "Hello world"

    def test_stream_part_can_be_typed(self):
        """A yielded dict round-trips through parse_stream_part into a model."""
        with MockServer(OPENAI_STREAM, content_type="text/event-stream") as mock:
            model = openai("test-key", "gpt-4o", mock.url)
            first_delta = next(
                p for p in stream_text(model, "Say hello") if "TextDelta" in p
            )

        sp = parse_stream_part(first_delta)
        assert isinstance(sp, StreamPart)
        assert sp.root.type == "TextDelta"
        assert sp.root.delta == "Hello"
        # Round-trips back to the external-tag dict form.
        assert sp.model_dump() == first_delta


# ── options: tools / tool_choice reach the provider ──────────────────────────

class TestOptionsReachProvider:

    def test_tools_and_tool_choice_reach_provider(self):
        with RecordingMockServer(OPENAI_TOOL_CALL) as mock:
            model = openai("test-key", "gpt-4o", mock.url)
            opts = GenerateTextOptions(
                tools=[
                    FunctionTool(
                        name="get_weather",
                        input_schema={
                            "type": "object",
                            "properties": {"location": {"type": "string"}},
                        },
                    ),
                ],
                tool_choice="required",
            )
            generate_text(model, "Hello", opts)

            body = mock.received_body()

        # OpenAI nests the function tool under a `function` key and renames
        # `input_schema` → `parameters`. The typed model serialized correctly
        # across the JSON boundary (proven separately) and the converter shaped
        # it for the provider.
        assert isinstance(body["tools"], list)
        assert body["tools"][0]["type"] == "function"
        assert body["tools"][0]["function"]["name"] == "get_weather"
        assert body["tools"][0]["function"]["parameters"] == {
            "type": "object",
            "properties": {"location": {"type": "string"}},
        }
        # tool_choice string variant passes through to the provider unchanged.
        assert body["tool_choice"] == "required"

    def test_tool_choice_tool_variant_forces_tool_at_provider(self):
        """The ``tool`` tool-choice variant forces the specific tool at the
        provider (OpenAI renders it as ``{"type":"function","function":{...}}``)."""
        with RecordingMockServer(OPENAI_TOOL_CALL) as mock:
            model = openai("test-key", "gpt-4o", mock.url)
            opts = GenerateTextOptions(
                tools=[FunctionTool(name="get_weather", input_schema={"type": "object"})],
                tool_choice=ToolChoiceTool(tool_name="get_weather"),
            )
            generate_text(model, "Hello", opts)

            body = mock.received_body()

        assert body["tool_choice"] == {
            "type": "function",
            "function": {"name": "get_weather"},
        }

    def test_tool_choice_serializes_camel_case_tool_name(self):
        """At the wrapper→Rust JSON boundary, the ``tool`` tool-choice variant
        must serialize as ``{"type":"tool","toolName":"..."}`` (camelCase), the
        hand-rolled serde wire format Rust expects. This is a deterministic
        unit test of the boundary, independent of provider conversion."""
        opts = GenerateTextOptions(
            tools=[FunctionTool(name="get_weather", input_schema={"type": "object"})],
            tool_choice=ToolChoiceTool(tool_name="get_weather"),
        )
        wire = json.loads(_opts_to_json(opts))
        assert wire["tool_choice"] == {"type": "tool", "toolName": "get_weather"}

        # And it round-trips back into the typed model.
        from aimux.wrapper import ToolChoiceTool as _TCT
        back = _TCT.model_validate(wire["tool_choice"])
        assert back.tool_name == "get_weather"


# ── prompt: multi-role + multipart messages ──────────────────────────────────

class TestPromptMessages:

    def test_multi_role_messages_reach_provider(self):
        with RecordingMockServer(OPENAI_CHAT) as mock:
            model = openai("test-key", "gpt-4o", mock.url)
            prompt = [
                ModelMessage(role="system", content="You are a helpful assistant."),
                ModelMessage(role="user", content="What is Rust?"),
            ]
            result = generate_text(model, prompt)

            body = mock.received_body()

        msgs = body["messages"]
        assert isinstance(msgs, list)
        assert len(msgs) == 2
        assert msgs[0] == {"role": "system", "content": "You are a helpful assistant."}
        assert msgs[1] == {"role": "user", "content": "What is Rust?"}
        assert result.text  # typed result still returned

    def test_multipart_content_reaches_provider(self):
        """Typed multi-part content reaches the provider as a content-part
        array. (Anthropic preserves the array form; OpenAI flattens text parts
        to a string, so Anthropic is the cleaner provider to assert against.)"""
        with RecordingMockServer(ANTHROPIC_MSG) as mock:
            model = anthropic("test-key", "claude-3-5-sonnet-20241022", mock.url)
            prompt = [
                ModelMessage(
                    role="user",
                    content=[TextContentPart(text="Hello, multimodal world.")],
                ),
            ]
            generate_text(model, prompt)

            body = mock.received_body()

        msg = body["messages"][0]
        assert msg["role"] == "user"
        # Content is the multi-part array form (not flattened to a string).
        assert isinstance(msg["content"], list)
        assert msg["content"][0] == {"type": "text", "text": "Hello, multimodal world."}


# ── tool-call round trip with typed content parts ────────────────────────────

class TestToolCallRoundTrip:

    def test_round_trip_with_typed_content_parts(self):
        """Full tool-call round trip using typed ModelMessage content parts.

        First call → model requests a tool call (OPENAI_TOOL_CALL).
        The assistant/tool messages are built from typed content parts
        (tool_call / tool_result), and a second call returns final text.
        """
        opts = GenerateTextOptions(
            tools=[
                FunctionTool(
                    name="get_weather",
                    description="Get weather for a location",
                    input_schema={
                        "type": "object",
                        "properties": {"location": {"type": "string"}},
                        "required": ["location"],
                    },
                ),
            ],
        )

        with SequencedMockServer([OPENAI_TOOL_CALL, OPENAI_CHAT]) as mock:
            model = openai("test-key", "gpt-4o", mock.url)

            # Step 1: model requests a tool call.
            result = generate_text(model, "What's the weather in Tokyo?", opts)
            assert len(result.tool_calls) == 1
            assert result.tool_calls[0].tool_name == "get_weather"
            assert result.tool_calls[0].tool_call_id == "call_abc"

            # Step 2: back-fill the ToolResult via typed content parts.
            messages = [
                ModelMessage(role="user", content="What's the weather in Tokyo?"),
                ModelMessage(
                    role="assistant",
                    content=[
                        {
                            "type": "tool_call",
                            "tool_call_id": "call_abc",
                            "tool_name": "get_weather",
                            "input": {"location": "Tokyo"},
                        }
                    ],
                ),
                ModelMessage(
                    role="tool",
                    content=[
                        {
                            "type": "tool_result",
                            "tool_call_id": "call_abc",
                            "output": {"temperature": 22, "condition": "sunny"},
                        }
                    ],
                ),
            ]

            # Step 3: second call returns final text.
            result2 = generate_text(model, messages, opts)
            assert result2.text == "Rust is a systems programming language."

            # Step 4: the second request carries the full round trip.
            mock.received_body()  # drain first request
            body2 = mock.received_body()

        msgs = body2["messages"]
        assert len(msgs) == 3
        assert msgs[-1]["role"] == "tool"
        assert msgs[-1]["tool_call_id"] == "call_abc"
