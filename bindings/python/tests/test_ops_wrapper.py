"""Retained typed wire-schema regressions, independent of provider constructors."""
import importlib.util
import os
import unittest

HAS_TYPED = importlib.util.find_spec("pydantic") is not None and any(
    os.environ.get(flag) == "1" for flag in ("AIMUX_REAL_OPS", "AIMUX_OPS_DOUBLE")
)
if HAS_TYPED:
    from pydantic import TypeAdapter
    from aimux.wrapper import (
        ContentPart, GenerateContent, GeneratedFile, GenerateTextOptions,
        GenerateTextResult, ModelMessage, StreamPart, TextContent,
        TextContentPart, FunctionTool, ToolChoiceTool, _opts_to_json,
    )

@unittest.skipUnless(HAS_TYPED, "requires installed binding and optional Pydantic")
class TestGeneratedFiles(unittest.TestCase):

    def test_generated_file_is_base64_and_media_type(self):
        """GeneratedFile is {base64, mediaType, providerMetadata?} (ai
        generate-text/generated-file.ts:10-32); a text file part is
        {type, file, providerMetadata?} (convert-language-model-content.ts:55-70);
        the provider's LanguageModelV4File keeps {type, mediaType, data}."""
        file = GeneratedFile.model_validate({"base64": "aGk=", "mediaType": "text/plain"})
        assert file.model_dump(by_alias=True) == {"base64": "aGk=", "mediaType": "text/plain"}
        assert GeneratedFile(base64=b"hi", media_type="text/plain").base64 == "aGk="
        assert file.to_bytes() == b"hi"

        for tag in ("file", "reasoning-file"):
            wire = {
                "type": tag,
                "file": {"base64": "aGk=", "mediaType": "text/plain"},
                "providerMetadata": {"google": {"thoughtSignature": "sig"}},
            }
            for adapter in (TypeAdapter(TextContent), TypeAdapter(StreamPart)):
                assert adapter.dump_python(adapter.validate_python(wire), by_alias=True) == wire

        raw = {
            "type": "file",
            "data": {"type": "url", "url": "https://example.com/a.png"},
            "mediaType": "image/png",
        }
        adapter = TypeAdapter(GenerateContent)
        assert adapter.dump_python(adapter.validate_python(raw), by_alias=True) == raw

@unittest.skipUnless(HAS_TYPED, "requires installed binding and optional Pydantic")
class TestStandaloneTypeTags(unittest.TestCase):

    def test_standalone_tool_calls_and_sources_are_tagged(self):
        """A result's toolCalls and sources are the AI SDK's TypedToolCall and
        Source, tagged {type: 'tool-call'} (ai generate-text/tool-call.ts:8-14)
        and {type: 'source', sourceType} (provider language-model-v4-source.ts:8,
        :36); generate-text-result.ts:73, :78. A repair function's call is a
        tagged LanguageModelV4ToolCall (language-model-v4-tool-call.ts:7)."""
        from aimux.wrapper import GenerateTextResult, RawToolCall, ToolCall

        call = {"type": "tool-call", "toolCallId": "call-1", "toolName": "weather", "input": {"city": "SG"}}
        url = {"type": "source", "sourceType": "url", "id": "s1", "url": "https://example.com"}
        result = GenerateTextResult.model_validate(
            {"text": "", "finishReason": "stop", "toolCalls": [call], "sources": [url]}
        )
        dumped = result.model_dump(by_alias=True)
        assert dumped["toolCalls"] == [call]
        assert dumped["sources"] == [url]
        assert RawToolCall(tool_call_id="c", tool_name="t", input="{}").model_dump(by_alias=True) == {
            "type": "tool-call", "toolCallId": "c", "toolName": "t", "input": "{}",
        }

        # JSON written before the tag still reads; another tag is rejected.
        assert ToolCall.model_validate({"toolCallId": "c", "toolName": "t", "input": {}}).type == "tool-call"
        with self.assertRaises(ValueError):
            ToolCall.model_validate({"type": "source", "toolCallId": "c", "toolName": "t", "input": {}})

@unittest.skipUnless(HAS_TYPED, "requires installed binding and optional Pydantic")
class TestNullIsKeptApartFromMissing(unittest.TestCase):

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

@unittest.skipUnless(HAS_TYPED, "requires installed binding and optional Pydantic")
class TestStepsAndUsageAreCamelCase(unittest.TestCase):

    def test_usage_and_step_round_trip(self):
        """``LanguageModelUsage`` (ai types/usage.ts:10-67) is camelCase with
        an unknown count absent; a step is ``StepResult``'s own fields
        (generate-text/step-result.ts:138-278), camelCase."""
        from aimux.wrapper import LanguageModelUsage, StepResult

        usage = {"inputTokens": 3, "inputTokenDetails": {"cacheReadTokens": 1},
                 "outputTokens": 2, "outputTokenDetails": {}, "totalTokens": 5}
        parsed = LanguageModelUsage.model_validate(usage)
        assert parsed.input_token_details.cache_read_tokens == 1
        assert parsed.model_dump(mode="json", by_alias=True) == usage

        step = {
            "callId": "c1", "stepNumber": 0,
            "model": {"provider": "openai", "modelId": "gpt-4o"},
            "content": [], "finishReason": "stop", "usage": usage,
            "performance": {"stepTimeMs": 1.0}, "warnings": [], "request": {},
            "response": {"id": "r1", "timestamp": "2026-01-01T00:00:00.000Z",
                         "modelId": "gpt-4o", "body": None, "messages": []},
        }
        parsed_step = StepResult.model_validate(step)
        assert parsed_step.model.model_id == "gpt-4o"
        assert parsed_step.response.model_id == "gpt-4o"
        assert parsed_step.model_dump(mode="json", by_alias=True) == step

