"""Tests for the typed Pydantic wrapper (aimux.wrapper).

Verifies the FULL chain through the typed layer: typed options / prompt models
→ JSON boundary → PyO3 → aimux-core → mock HTTP → typed result objects.

Reuses the mock-server fixtures (MockServer / RecordingMockServer /
SequencedMockServer) and canned provider responses from test_e2e.py.
"""

import json

import pytest
from pydantic import TypeAdapter

from aimux import deepseek, anthropic
from aimux.wrapper import (
    ContentPart,
    GenerateTextOptions,
    GenerateTextResult,
    ModelMessage,
    TextContentPart,
    FunctionTool,
    ToolChoiceTool,
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
            model = deepseek("test-key", "gpt-4o", mock.url)
            result = generate_text(model, "What is Rust?")

        assert isinstance(result, GenerateTextResult)
        assert result.text == "Rust is a systems programming language."
        # Usage is parsed into a model, not left as a dict.
        assert result.usage.input_tokens.total == 10
        assert result.usage.output_tokens.total == 8

    def test_parses_tool_calls(self):
        with RecordingMockServer(OPENAI_TOOL_CALL) as mock:
            model = deepseek("test-key", "gpt-4o", mock.url)
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
        tool_call_parts = [c for c in result.raw.content if c.type == "tool-call"]
        assert tool_call_parts, "raw.content must contain a tool-call variant"
        assert tool_call_parts[0].tool_name == "get_weather"
        assert tool_call_parts[0].tool_call_id == "call_abc"
        # raw content keeps the provider's argument text (see test_e2e).
        assert tool_call_parts[0].input == '{"location":"Tokyo"}'

    def test_raw_content_is_text_variant(self):
        with MockServer(OPENAI_CHAT) as mock:
            model = deepseek("test-key", "gpt-4o", mock.url)
            result = generate_text(model, "What is Rust?")

        # raw.content[0] is a typed Text content part.
        first = result.raw.content[0]
        assert first.type == "text"
        assert first.text == "Rust is a systems programming language."


# ── stream_text: typed-ish dicts ─────────────────────────────────────────────

class TestStreamText:

    def test_yields_text_delta_dicts(self):
        with MockServer(OPENAI_STREAM, content_type="text/event-stream") as mock:
            model = deepseek("test-key", "gpt-4o", mock.url)
            parts = list(stream_text(model, "Say hello"))

        assert len(parts) > 0
        text = "".join(p["delta"] for p in parts if p["type"] == "text-delta")
        assert text == "Hello world"

    def test_stream_part_can_be_typed(self):
        """A yielded dict round-trips through parse_stream_part into a model."""
        with MockServer(OPENAI_STREAM, content_type="text/event-stream") as mock:
            model = deepseek("test-key", "gpt-4o", mock.url)
            first_delta = next(
                p for p in stream_text(model, "Say hello") if p["type"] == "text-delta"
            )

        sp = parse_stream_part(first_delta)
        assert sp.type == "text-delta"
        assert sp.delta == "Hello"
        # Round-trips back to the wire dict form.
        assert sp.model_dump(exclude_none=True, by_alias=True) == first_delta

    # ── RFC-0016 M2: include_raw_chunks ─────────────────────────────────────

    def test_include_raw_chunks_emits_raw_parts(self):
        """include_raw_chunks=True surfaces one Raw part per JSON SSE event,
        before the parsed parts ([DONE] excluded)."""
        with MockServer(OPENAI_STREAM, content_type="text/event-stream") as mock:
            model = deepseek("test-key", "gpt-4o", mock.url)
            parts = list(
                stream_text(model, "Say hello",
                            GenerateTextOptions(include_raw_chunks=True))
            )

        raw_parts = [p for p in parts if p["type"] == "raw"]
        # OPENAI_STREAM has 3 JSON events (2 content chunks + 1 usage chunk);
        # the [DONE] sentinel emits no Raw.
        assert len(raw_parts) == 3
        # Each Raw carries the parsed JSON payload of the SSE event.
        assert (
            raw_parts[0]["rawValue"]["choices"][0]["delta"]["content"]
            == "Hello"
        )
        # Raw for the first event precedes its TextDelta.
        first_raw_idx = parts.index(raw_parts[0])
        first_text_idx = next(
            i for i, p in enumerate(parts) if p["type"] == "text-delta"
        )
        assert first_raw_idx < first_text_idx

    def test_raw_chunks_off_by_default(self):
        """Default options emit no Raw parts."""
        with MockServer(OPENAI_STREAM, content_type="text/event-stream") as mock:
            model = deepseek("test-key", "gpt-4o", mock.url)
            parts = list(stream_text(model, "Say hello"))

        assert all(p["type"] != "raw" for p in parts)

    def test_include_raw_chunks_option_roundtrip(self):
        """Pure model-layer round-trip of the include_raw_chunks option."""
        opts = GenerateTextOptions(include_raw_chunks=True)
        dumped = _opts_to_json(opts)
        assert '"includeRawChunks":true' in dumped
        back = GenerateTextOptions.model_validate_json(dumped)
        assert back.include_raw_chunks is True


# ── options: tools / tool_choice reach the provider ──────────────────────────

class TestOptionsReachProvider:

    def test_tools_and_tool_choice_reach_provider(self):
        with RecordingMockServer(OPENAI_TOOL_CALL) as mock:
            model = deepseek("test-key", "gpt-4o", mock.url)
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
            model = deepseek("test-key", "gpt-4o", mock.url)
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
        AI SDK shape Rust expects. This is a deterministic
        unit test of the boundary, independent of provider conversion."""
        opts = GenerateTextOptions(
            tools=[FunctionTool(name="get_weather", input_schema={"type": "object"})],
            tool_choice=ToolChoiceTool(tool_name="get_weather"),
        )
        wire = json.loads(_opts_to_json(opts))
        assert wire["toolChoice"] == {"type": "tool", "toolName": "get_weather"}

        # And it round-trips back into the typed model.
        from aimux.wrapper import ToolChoiceTool as _TCT
        back = _TCT.model_validate(wire["toolChoice"])
        assert back.tool_name == "get_weather"


# ── prompt: multi-role + multipart messages ──────────────────────────────────

class TestPromptMessages:

    def test_multi_role_messages_reach_provider(self):
        with RecordingMockServer(OPENAI_CHAT) as mock:
            model = deepseek("test-key", "gpt-4o", mock.url)
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

    def test_image_bytes_are_base64_on_the_wire(self):
        """ImagePart.image is DataContent upstream (provider-utils
        types/content-part.ts:42): bytes are a base64 string in the JSON."""
        part = TypeAdapter(ContentPart).validate_python(
            {"type": "image", "image": b"\x89PNG", "mediaType": "image/png"}
        )
        assert part.model_dump(by_alias=True, exclude_none=True) == {
            "type": "image",
            "image": "iVBORw==",
            "mediaType": "image/png",
        }
        with RecordingMockServer(ANTHROPIC_MSG) as mock:
            model = anthropic("test-key", "claude-3-5-sonnet-20241022", mock.url)
            generate_text(model, [ModelMessage(role="user", content=[part])])
            body = mock.received_body()
        source = body["messages"][0]["content"][0]["source"]
        assert source == {"type": "base64", "media_type": "image/png", "data": "iVBORw=="}


    def test_file_parts_are_the_ai_sdk_file_part(self):
        """FilePart.data is FileData | DataContent | URL | ProviderReference
        upstream (provider-utils types/content-part.ts:75, file-data.ts:44-48);
        there is no file-base64 / file-url / file-reference part."""
        adapter = TypeAdapter(ContentPart)
        for wire in [
            {"type": "file", "data": {"type": "data", "data": "aGk="}, "mediaType": "text/plain"},
            {"type": "file", "data": {"type": "url", "url": "https://example.com/a.pdf"},
             "mediaType": "application/pdf"},
            {"type": "file", "data": {"type": "reference", "reference": {"anthropic": "file-abc"}},
             "mediaType": "application/pdf", "filename": "a.pdf"},
            {"type": "file", "data": {"type": "text", "text": "hello"}, "mediaType": "text/plain"},
            {"type": "file", "data": "aGk=", "mediaType": "text/plain"},
            {"type": "file", "data": {"anthropic": "file-abc"}, "mediaType": "application/pdf"},
            {"type": "reasoning-file", "data": "aGk=", "mediaType": "image/png"},
            {"type": "reasoning-file", "data": {"type": "url", "url": "https://example.com/a.png"},
             "mediaType": "image/png"},
        ]:
            assert adapter.dump_python(adapter.validate_python(wire), by_alias=True) == wire
        assert adapter.dump_python(
            adapter.validate_python({"type": "file", "data": b"hi", "mediaType": "text/plain"}),
            by_alias=True,
        ) == {"type": "file", "data": "aGk=", "mediaType": "text/plain"}
        for removed in ("file-base64", "file-url", "file-reference"):
            with pytest.raises(ValueError):
                adapter.validate_python(
                    {"type": removed, "data": "aGk=", "url": "https://example.com/a.pdf",
                     "mediaType": "application/pdf"}
                )

        # A data: URL's media type wins (ai prompt/file-part-data.ts:31-41);
        # a bare reference is a reference (:133-138).
        parts = [
            adapter.validate_python(
                {"type": "file", "data": "data:image/png;base64,iVBORw==", "mediaType": "image"}
            ),
            adapter.validate_python(
                {"type": "file", "data": {"anthropic": "file-abc"}, "mediaType": "application/pdf"}
            ),
        ]
        with RecordingMockServer(ANTHROPIC_MSG) as mock:
            model = anthropic("test-key", "claude-3-5-sonnet-20241022", mock.url)
            generate_text(model, [ModelMessage(role="user", content=parts)])
            body = mock.received_body()
        content = body["messages"][0]["content"]
        assert content[0]["source"] == {"type": "base64", "media_type": "image/png", "data": "iVBORw=="}
        assert content[1] == {"type": "document", "source": {"type": "file", "file_id": "file-abc"}}


# ── JSON null and undefined (#232 C34) ───────────────────────────────────────

class TestNullIsKeptApartFromMissing:

    def test_unknown_fields_keep_null(self):
        """``body?: unknown`` (provider language-model-v4-generate-result.ts:41,
        :56) and ``inputSchemaInput?: unknown`` (provider-utils
        types/tool-approval-request.ts:41): the AI SDK's JSON keeps ``null``
        and omits ``undefined``; so does the dump of a parsed model."""
        from aimux.wrapper import GenerateResponseMetadata, RequestInfo

        for wire in ({}, {"body": None}, {"body": {"a": None}}):
            assert GenerateResponseMetadata.model_validate(wire).model_dump(
                mode="json", by_alias=True) == wire
            assert RequestInfo.model_validate(wire).model_dump(
                mode="json", by_alias=True) == wire
        approval = {"type": "tool-approval-request", "approvalId": "a1",
                    "toolCallId": "c1", "inputSchemaInput": None}
        part = TypeAdapter(ContentPart).validate_python(approval)
        assert part.model_dump(mode="json", by_alias=True) == approval

    def test_null_tool_input_is_sent_as_null(self):
        """A tool call's ``input`` is a JSON value; ``null`` is sent back as
        ``null`` instead of being dropped (which Rust rejects as missing)."""
        part = TypeAdapter(ContentPart).validate_python(
            {"type": "tool-call", "toolCallId": "c1", "toolName": "t", "input": None})
        assert part.model_dump(mode="json", by_alias=True)["input"] is None
        assert TextContentPart(text="x").model_dump(mode="json", by_alias=True) == {
            "type": "text", "text": "x"}


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
            model = deepseek("test-key", "gpt-4o", mock.url)

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
                            "type": "tool-call",
                            "toolCallId": "call_abc",
                            "toolName": "get_weather",
                            "input": {"location": "Tokyo"},
                        }
                    ],
                ),
                ModelMessage(
                    role="tool",
                    content=[
                        {
                            "type": "tool-result",
                            "toolCallId": "call_abc",
                            "toolName": "get_weather",
                            "output": {
                                "type": "json",
                                "value": {"temperature": 22, "condition": "sunny"},
                            },
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
