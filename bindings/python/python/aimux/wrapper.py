"""Typed AI SDK wire schemas and text helpers over interpreter-owned handles.

The shared ops dispatcher validates requests and executes all provider logic.
Pydantic is optional and is only required by this module.
"""

from __future__ import annotations

import base64
import json
from typing import Annotated, Any, Dict, Iterator, List, Literal, Optional, Union

from pydantic import (
    BaseModel,
    BeforeValidator,
    ConfigDict,
    Field,
    SerializationInfo,
    TypeAdapter,
    model_serializer,
)
from pydantic.alias_generators import to_camel

from . import Handle

__all__ = [
    "Role",
    "FinishReasonUnified",
    "ReasoningEffort",
    "MessageContent",
    "ContentPart",
    "Tool",
    "ToolChoice",
    "ResponseFormat",
    "StreamPart",
    "GenerateContent",
    "Source",
    "TextContent",
    "FileData",
    "FilePartData",
    "ReasoningFilePartData",
    "FileBytes",
    "Warning",
    "AiMuxErrorValue",
    "InputTokenUsage",
    "OutputTokenUsage",
    "GeneratedFileData",
    "GeneratedFile",
    "LanguageModelFile",
    "Usage",
    "InputTokenDetails",
    "OutputTokenDetails",
    "LanguageModelUsage",
    "FinishReason",
    "ResponseMetadata",
    "StepResponseMetadata",
    "StepResponse",
    "StepModel",
    "StepResult",
    "ToolCall",
    "RawToolCall",
    "ToolCallRepairContext",
    "ModelMessage",
    "FunctionTool",
    "FunctionToolInputExample",
    "ProviderTool",
    "TextContentPart",
    "GenerateTextOptions",
    "GenerateTextResult",
    "GenerateObjectResult",
    "GenerateObjectResponse",
    "GenerateResult",
    "GenerateResponseMetadata",
    "RequestInfo",
    "generate_text",
    "stream_text",
    "parse_stream_part",
    "ChatCompletion",
    "ChatCompletionChoice",
    "ChatCompletionMessage",
    "ChatCompletionToolCall",
    "ChatCompletionFunction",
    "ChatCompletionUsage",
    "ChatCompletionChunk",
    "ChatCompletionChunkChoice",
    "ChatCompletionDelta",
    "ChatCompletionChunkToolCall",
    "ChatCompletionChunkFunction",
    "PromptTokensDetails",
    "CompletionTokensDetails",
    "generate_text_as_openai",
    "stream_text_as_openai",
]


_JSON_VALUE_TYPES = (Any, Optional[Any])


class _Wire(BaseModel):
    """Base of every aimux model: ``snake_case`` attributes, camelCase JSON keys.

    ``None`` is omitted when dumping, as Rust omits ``None`` (``undefined`` in
    the AI SDK's JSON), except in a JSON-value field (``Any``) that was given
    a value: there ``None`` is a JSON ``null``, which the AI SDK keeps apart
    from a missing value (Rust ``wire::present``).
    """

    model_config = ConfigDict(alias_generator=to_camel, populate_by_name=True)

    @model_serializer(mode="wrap")
    def _omit_undefined(self, handler: Any, info: SerializationInfo) -> Any:
        data = handler(self)
        if not isinstance(data, dict):
            return data
        for name, field in type(self).model_fields.items():
            if getattr(self, name, None) is not None:
                continue
            if name in self.model_fields_set and field.annotation in _JSON_VALUE_TYPES:
                continue
            data.pop(field.alias if info.by_alias and field.alias else name, None)
        return data


class _Meta(_Wire):
    provider_metadata: Optional[Dict[str, Any]] = None


Role = Literal["system", "user", "assistant", "tool"]

FinishReasonUnified = Literal[
    "stop", "length", "content-filter", "tool-calls", "error", "other"
]

ReasoningEffort = Literal[
    "provider-default", "none", "minimal", "low", "medium", "high", "xhigh"
]

#: Opaque provider error value (Rust ``AiMuxError``): an object discriminated
#: by ``name`` (``"AI_APICallError"`` …), left as an arbitrary JSON value.
#: Named ``AiMuxErrorValue`` to avoid confusion with the raised ``AimuxError``
#: exception.
AiMuxErrorValue = Any


class InputTokenUsage(_Wire):
    total: Optional[int] = None
    no_cache: Optional[int] = None
    cache_read: Optional[int] = None
    cache_write: Optional[int] = None


class OutputTokenUsage(_Wire):
    total: Optional[int] = None
    text: Optional[int] = None
    reasoning: Optional[int] = None


class Usage(_Wire):
    """Provider-level token usage (Rust ``Usage``), as on :class:`GenerateResult`."""

    input_tokens: InputTokenUsage = Field(default_factory=InputTokenUsage)
    output_tokens: OutputTokenUsage = Field(default_factory=OutputTokenUsage)
    raw: Optional[Dict[str, Any]] = None


class InputTokenDetails(_Wire):
    """Breakdown of the input tokens (Rust ``InputTokenDetails``)."""

    no_cache_tokens: Optional[int] = None
    cache_read_tokens: Optional[int] = None
    cache_write_tokens: Optional[int] = None


class OutputTokenDetails(_Wire):
    """Breakdown of the output tokens (Rust ``OutputTokenDetails``)."""

    text_tokens: Optional[int] = None
    reasoning_tokens: Optional[int] = None


class LanguageModelUsage(_Wire):
    """Token usage of a step or a whole call (Rust ``LanguageModelUsage``,
    the AI SDK's ``LanguageModelUsage``): flat totals plus detail objects.
    Unknown counts are ``None``, never zero.
    """

    input_tokens: Optional[int] = None
    input_token_details: InputTokenDetails = Field(default_factory=InputTokenDetails)
    output_tokens: Optional[int] = None
    output_token_details: OutputTokenDetails = Field(default_factory=OutputTokenDetails)
    total_tokens: Optional[int] = None
    raw: Optional[Dict[str, Any]] = None


class FinishReason(_Wire):
    """Why generation stopped (Rust ``FinishReason``)."""

    unified: FinishReasonUnified
    raw: Optional[str] = None


class ResponseMetadata(_Wire):
    """Metadata about the API response (Rust ``ResponseMetadata``)."""

    id: Optional[str] = None
    timestamp: Optional[str] = None
    model_id: Optional[str] = None


class GenerateResponseMetadata(ResponseMetadata):
    """Response metadata with headers and body (Rust ``ResponseInfo``)."""

    headers: Optional[Dict[str, str]] = None
    body: Optional[Any] = None


class RequestInfo(_Wire):
    body: Optional[Any] = None


class StepResponseMetadata(_Wire):
    """The response of a step without its messages and body (Rust
    ``StepResponseMetadata``), carried by the ``finish-step`` stream part.
    """

    id: str = ""
    timestamp: str = ""
    model_id: str = ""
    headers: Optional[Dict[str, str]] = None


class ToolCall(_Meta):
    """A tool call requested by the model (Rust ``tool::ToolCall``).

    Its JSON carries ``"type": "tool-call"``: a call outside ``content`` is the
    AI SDK's tagged ``TypedToolCall`` (ai ``generate-text/tool-call.ts``).
    """

    type: Literal["tool-call"] = "tool-call"
    tool_call_id: str
    tool_name: str
    input: Any
    provider_executed: Optional[bool] = None
    dynamic: Optional[bool] = None
    invalid: Optional[bool] = None
    error: Optional[AiMuxErrorValue] = None


class RawToolCall(_Meta):
    """A tool call as the provider emitted it (Rust ``RawToolCall``).

    Unlike :class:`ToolCall`, ``input`` is the **raw argument text** — the
    string the model produced, parsed by nobody yet. This is both what a
    repair function is shown and what it returns. Its JSON carries
    ``"type": "tool-call"``, as the AI SDK's ``LanguageModelV4ToolCall``.
    """

    type: Literal["tool-call"] = "tool-call"
    tool_call_id: str
    tool_name: str
    input: str
    provider_executed: Optional[bool] = None
    dynamic: Optional[bool] = None


class _WarningUnsupported(_Wire):
    type: Literal["unsupported"]
    feature: str
    details: Optional[str] = None


class _WarningCompatibility(_Wire):
    type: Literal["compatibility"]
    feature: str
    details: Optional[str] = None


class _WarningDeprecated(_Wire):
    type: Literal["deprecated"]
    setting: str
    message: str


class _WarningOther(_Wire):
    type: Literal["other"]
    message: str


Warning = Annotated[
    Union[_WarningUnsupported, _WarningCompatibility, _WarningDeprecated, _WarningOther],
    Field(discriminator="type"),
]


def _bytes_to_base64(value: Any) -> Any:
    if isinstance(value, (bytes, bytearray, memoryview)):
        return base64.b64encode(bytes(value)).decode("ascii")
    return value


#: Bytes as the AI SDK's JSON carries a ``Uint8Array``: a base64 string.
#: ``bytes`` given here are encoded (Rust ``wire::base64_bytes``).
Base64Bytes = Annotated[str, BeforeValidator(_bytes_to_base64)]

FileBytes = Base64Bytes


class _FileDataData(_Wire):
    type: Literal["data"]
    data: FileBytes


class _FileDataUrl(_Wire):
    type: Literal["url"]
    url: str
    original_url: Optional[str] = None


class _FileDataReference(_Wire):
    type: Literal["reference"]
    reference: Dict[str, str]


class _FileDataText(_Wire):
    type: Literal["text"]
    text: str


FileData = Annotated[
    Union[_FileDataData, _FileDataUrl, _FileDataReference, _FileDataText],
    Field(discriminator="type"),
]

#: Data or a URL returned for a generated file (Rust ``GeneratedFileData``).
GeneratedFileData = Annotated[
    Union[_FileDataData, _FileDataUrl], Field(discriminator="type")
]


class LanguageModelFile(_Meta):
    """A file in a provider's output (Rust ``LanguageModelFile``,
    ``LanguageModelV4File``): tagged base64 data or a URL."""

    data: GeneratedFileData
    media_type: str


class GeneratedFile(_Meta):
    """A generated file (Rust ``GeneratedFile``, the AI SDK's
    ``GeneratedFile``): ``{"base64": ..., "mediaType": ...}`` on the wire."""

    base64: FileBytes
    media_type: str

    def to_bytes(self) -> bytes:
        """The file's bytes, decoded from ``base64``."""
        return base64.b64decode(self.base64)


class _UrlSource(_Meta):
    # A source is the AI SDK's tagged ``Source`` (provider
    # ``language-model-v4-source.ts``) in a result's ``sources`` too.
    type: Literal["source"] = "source"
    id: str
    source_type: Literal["url"]
    url: str
    title: Optional[str] = None


class _DocumentSource(_Meta):
    type: Literal["source"] = "source"
    id: str
    source_type: Literal["document"]
    media_type: str
    title: str
    filename: Optional[str] = None


Source = Annotated[Union[_UrlSource, _DocumentSource], Field(discriminator="source_type")]


class _ContentUrlSource(_UrlSource):
    type: Literal["source"]


class _ContentDocumentSource(_DocumentSource):
    type: Literal["source"]


_ContentSource = Annotated[
    Union[_ContentUrlSource, _ContentDocumentSource], Field(discriminator="source_type")
]


class _Text(_Meta):
    type: Literal["text"]
    text: str


class _Reasoning(_Meta):
    type: Literal["reasoning"]
    text: str


class _Custom(_Meta):
    type: Literal["custom"]
    kind: str


class _File(LanguageModelFile):
    type: Literal["file"]


class _ReasoningFile(LanguageModelFile):
    type: Literal["reasoning-file"]


class _FileOutput(_Meta):
    """``{"type": "file", "file": GeneratedFile, "providerMetadata"?}``."""

    type: Literal["file"]
    file: GeneratedFile


class _ReasoningFileOutput(_Meta):
    type: Literal["reasoning-file"]
    file: GeneratedFile


class _RawToolCallPart(RawToolCall):
    type: Literal["tool-call"]


class _ToolCallPart(ToolCall):
    type: Literal["tool-call"]


class _ToolResultPart(_Meta):
    type: Literal["tool-result"]
    tool_call_id: str
    tool_name: str
    result: Any
    is_error: Optional[bool] = None
    preliminary: Optional[bool] = None
    dynamic: Optional[bool] = None


class _TypedToolResult(_Meta):
    """Rust ``TypedToolResult``: a tool result with the input of the call it
    answers (absent for a deferred provider result)."""

    type: Literal["tool-result"]
    tool_call_id: str
    tool_name: str
    input: Optional[Any] = None
    output: Any
    provider_executed: Optional[bool] = None
    dynamic: Optional[bool] = None
    preliminary: Optional[bool] = None


class _TypedToolError(_Meta):
    """Rust ``TypedToolError``: a provider-executed tool's error result, or
    the error that follows an invalid tool call."""

    type: Literal["tool-error"]
    tool_call_id: str
    tool_name: str
    input: Optional[Any] = None
    error: Any
    provider_executed: Optional[bool] = None
    dynamic: Optional[bool] = None


class _ToolOutputDenied(_Wire):
    type: Literal["tool-output-denied"]
    tool_call_id: str
    tool_name: str
    provider_executed: Optional[bool] = None
    dynamic: Optional[bool] = None


class _RawApprovalRequest(_Meta):
    type: Literal["tool-approval-request"]
    approval_id: str
    tool_call_id: str


class _ApprovalRequest(_Wire):
    type: Literal["tool-approval-request"]
    approval_id: str
    tool_call: ToolCall
    reason: Optional[str] = None
    is_automatic: Optional[bool] = None
    signature: Optional[str] = None


class _ApprovalResponse(_Wire):
    type: Literal["tool-approval-response"]
    approval_id: str
    tool_call: ToolCall
    approved: bool
    reason: Optional[str] = None
    provider_executed: Optional[bool] = None


GenerateContent = Annotated[
    Union[
        _Text,
        _RawToolCallPart,
        _ContentSource,
        _Reasoning,
        _File,
        _ReasoningFile,
        _Custom,
        _RawApprovalRequest,
        _ToolResultPart,
    ],
    Field(discriminator="type"),
]

TextContent = Annotated[
    Union[
        _Text,
        _Custom,
        _Reasoning,
        _ReasoningFileOutput,
        _ContentSource,
        _FileOutput,
        _ToolCallPart,
        _TypedToolResult,
        _TypedToolError,
        _ApprovalRequest,
        _ApprovalResponse,
    ],
    Field(discriminator="type"),
]


class _SPIdMeta(_Meta):
    id: str


class _SPDelta(_SPIdMeta):
    delta: str


class _SPTextChunk(_SPIdMeta):
    """A text or reasoning delta: ``text``, not ``delta``, in the text stream."""

    text: str


class _SPTextStart(_SPIdMeta):
    type: Literal["text-start"]


class _SPTextDelta(_SPTextChunk):
    type: Literal["text-delta"]


class _SPTextEnd(_SPIdMeta):
    type: Literal["text-end"]


class _SPStart(_Wire):
    type: Literal["start"]


class _SPStartStep(_Wire):
    type: Literal["start-step"]
    request: RequestInfo
    warnings: List[Warning]


class _SPFinishStep(_Meta):
    type: Literal["finish-step"]
    response: StepResponseMetadata
    usage: LanguageModelUsage
    # Timing of the step (Rust ``StepResultPerformance``). Weak type (Dict).
    performance: Dict[str, Any] = Field(default_factory=dict)
    finish_reason: FinishReasonUnified
    raw_finish_reason: Optional[str] = None


class _SPFinish(_Wire):
    type: Literal["finish"]
    finish_reason: FinishReasonUnified
    raw_finish_reason: Optional[str] = None
    total_usage: LanguageModelUsage


class _SPAbort(_Wire):
    type: Literal["abort"]
    reason: Optional[str] = None


class _SPError(_Wire):
    type: Literal["error"]
    error: AiMuxErrorValue


class _SPToolInputStart(_SPIdMeta):
    type: Literal["tool-input-start"]
    tool_name: str
    provider_executed: Optional[bool] = None
    dynamic: Optional[bool] = None
    title: Optional[str] = None


class _SPToolInputDelta(_SPDelta):
    type: Literal["tool-input-delta"]


class _SPToolInputEnd(_SPIdMeta):
    type: Literal["tool-input-end"]


class _SPReasoningStart(_SPIdMeta):
    type: Literal["reasoning-start"]


class _SPReasoningDelta(_SPTextChunk):
    type: Literal["reasoning-delta"]


class _SPReasoningEnd(_SPIdMeta):
    type: Literal["reasoning-end"]


class _SPRaw(_Wire):
    type: Literal["raw"]
    raw_value: Any


#: A single chunk of the stream returned by ``stream_text`` (Rust
#: ``TextStreamPart``, the AI SDK's full stream), e.g.
#: ``{"type": "text-delta", "id": ..., "text": ...}``: ``start``, then per
#: step ``start-step`` … ``finish-step``, then ``finish`` (or ``abort``).
StreamPart = Annotated[
    Union[
        _SPTextStart,
        _SPTextDelta,
        _SPTextEnd,
        _SPStart,
        _SPStartStep,
        _SPFinish,
        _SPFinishStep,
        _SPAbort,
        _SPError,
        _SPToolInputStart,
        _SPToolInputDelta,
        _SPToolInputEnd,
        _ToolCallPart,
        _TypedToolResult,
        _TypedToolError,
        _ToolOutputDenied,
        _FileOutput,
        _ReasoningFileOutput,
        _Custom,
        _ApprovalRequest,
        _ApprovalResponse,
        _SPReasoningStart,
        _SPReasoningDelta,
        _SPReasoningEnd,
        _ContentSource,
        _SPRaw,
    ],
    Field(discriminator="type"),
]


class _ResponseFormatText(_Wire):
    type: Literal["text"] = "text"


class _ResponseFormatJson(_Wire):
    type: Literal["json"] = "json"
    schema_: Optional[Any] = Field(default=None, alias="schema")
    name: Optional[str] = None
    description: Optional[str] = None


ResponseFormat = Annotated[
    Union[_ResponseFormatText, _ResponseFormatJson], Field(discriminator="type")
]


class FunctionToolInputExample(_Wire):
    input: Dict[str, Any]


class FunctionTool(_Wire):
    """A user-defined function tool (Rust ``FunctionTool``)."""

    type: Literal["function"] = "function"
    name: str
    description: Optional[str] = None
    input_schema: Any
    strict: Optional[bool] = None
    provider_options: Optional[Dict[str, Any]] = None
    input_examples: Optional[List[FunctionToolInputExample]] = None


class ProviderTool(_Wire):
    """A provider-defined tool (Rust ``ProviderTool``)."""

    type: Literal["provider"] = "provider"
    id: str
    name: str
    args: Dict[str, Any]


Tool = Annotated[
    Union[FunctionTool, ProviderTool],
    Field(discriminator="type"),
]


class _PartOptions(_Wire):
    provider_options: Optional[Any] = None


class TextContentPart(_PartOptions):
    """A text content part (Rust ``ContentPart::Text``)."""

    type: Literal["text"] = "text"
    text: str


class _ImageContentPart(_PartOptions):
    type: Literal["image"] = "image"
    image: Base64Bytes
    media_type: str


#: ``FilePart.data`` (Rust ``FilePartData``): a tagged ``FileData``, a string
#: (base64 or a URL; ``bytes`` are encoded as base64) or a bare provider
#: reference ``{provider: id}``.
FilePartData = Annotated[
    Union[FileData, Base64Bytes, Dict[str, str]],
    Field(union_mode="left_to_right"),
]

#: ``ReasoningFilePart.data`` (Rust ``ReasoningFilePartData``): tagged data or
#: a URL, or a string (base64 or a URL).
ReasoningFilePartData = Annotated[
    Union[GeneratedFileData, Base64Bytes],
    Field(union_mode="left_to_right"),
]


class _FileContentPart(_PartOptions):
    """The AI SDK ``FilePart``: ``data`` is ``FilePartData``."""

    type: Literal["file"] = "file"
    data: FilePartData
    media_type: str
    filename: Optional[str] = None


class _ReasoningContentPart(_PartOptions):
    type: Literal["reasoning"] = "reasoning"
    text: str
    signature: Optional[str] = None


class _ToolCallContentPart(_PartOptions):
    type: Literal["tool-call"] = "tool-call"
    tool_call_id: str
    tool_name: str
    input: Any
    provider_executed: Optional[bool] = None


class _ToolResultContentPart(_PartOptions):
    """``output`` is a ``{"type": "text" | "json" | "execution-denied" |
    "error-text" | "error-json" | "content", ...}`` object."""

    type: Literal["tool-result"] = "tool-result"
    tool_call_id: str
    tool_name: str
    output: Any


class _CustomContentPart(_PartOptions):
    type: Literal["custom"] = "custom"
    kind: str


class _ReasoningFileContentPart(_PartOptions):
    type: Literal["reasoning-file"] = "reasoning-file"
    data: ReasoningFilePartData
    media_type: str


class _ToolApprovalRequestContentPart(_Wire):
    type: Literal["tool-approval-request"] = "tool-approval-request"
    approval_id: str
    tool_call_id: str
    reason: Optional[str] = None
    is_automatic: Optional[bool] = None
    signature: Optional[str] = None
    input_schema_input: Optional[Any] = None


ContentPart = Annotated[
    Union[
        TextContentPart,
        _ImageContentPart,
        _FileContentPart,
        _ReasoningContentPart,
        _ToolCallContentPart,
        _ToolResultContentPart,
        _CustomContentPart,
        _ReasoningFileContentPart,
        _ToolApprovalRequestContentPart,
    ],
    Field(discriminator="type"),
]


MessageContent = Union[str, List[ContentPart]]


class ToolChoiceTool(_Wire):
    """The ``tool`` tool-choice variant (forces a specific tool)."""

    type: Literal["tool"] = "tool"
    tool_name: str


ToolChoice = Union[Literal["auto", "none", "required"], ToolChoiceTool]


class ModelMessage(_Wire):
    """A single user-facing chat message (Rust ``ModelMessage``).

    ``content`` is untagged on the wire: a bare string or a list of content
    parts.
    """

    role: Role
    content: MessageContent


class TimeoutConfiguration(_Wire):
    """Per-call timeout configuration (Rust ``TimeoutConfiguration``).

    All values are milliseconds; ``None`` disables the corresponding limit.
    """

    total_ms: Optional[int] = None
    step_ms: Optional[int] = None
    first_chunk_ms: Optional[int] = None
    chunk_ms: Optional[int] = None


class ToolCallRepairContext(_Wire):
    """What a ``repair_tool_call`` function is given (RFC-0035).

    Mirrors the AI SDK ``repairToolCall`` argument: the offending call with its
    raw argument text, the error it failed with, the schema it was checked
    against, and the tools, messages and instructions the call was generated
    with.
    """

    tool_call: RawToolCall
    error: AiMuxErrorValue
    input_schema: Any = None
    tools: List[Tool] = Field(default_factory=list)
    messages: List[ModelMessage] = Field(default_factory=list)
    instructions: Optional[str] = None


class GenerateTextOptions(_Wire):
    """User-facing options for ``generate_text`` / ``stream_text``.

    Mirrors Rust ``GenerateTextOptions``. All fields are optional; unset fields
    default to ``None`` (Rust treats absent / ``null`` as ``None``).

    Invalid tool calls arrive with ``invalid``/``error`` set on the tool call.
    Use the explicit toolRepair operations to repair them.
    """

    max_output_tokens: Optional[int] = None
    temperature: Optional[float] = None
    stop_sequences: Optional[List[str]] = None
    top_p: Optional[float] = None
    top_k: Optional[float] = None
    presence_penalty: Optional[float] = None
    frequency_penalty: Optional[float] = None
    output: Optional[Dict[str, Any]] = None
    """What the final text is parsed into (Rust ``Output``), in its wire
    form, tagged by ``type``: ``{"type": "text"}``,
    ``{"type": "object", "schema": ...}``,
    ``{"type": "array", "element": ..., "minItems": 1}``,
    ``{"type": "choice", "options": [...]}`` or ``{"type": "json"}``. The
    parsed value lands on ``GenerateTextResult.output``.
    """
    seed: Optional[int] = None
    tools: Optional[List[Tool]] = None
    tool_choice: Optional[ToolChoice] = None
    headers: Optional[Dict[str, str]] = None
    provider_options: Optional[Dict[str, Any]] = None
    reasoning: Optional[ReasoningEffort] = None
    instructions: Optional[str] = None
    max_retries: Optional[int] = None
    session_id: Optional[str] = None
    """Session identifier (RFC-0024): groups consecutive calls into a session.

    Explicit values take precedence; when ``None`` and the optional session
    inferer is enabled (``AIMUX_SESSION_INFER=1``),
    one may be inferred from prompt-prefix continuation.
    """
    timeout: Optional[TimeoutConfiguration] = None
    include_raw_chunks: Optional[bool] = None


class GenerateResult(_Wire):
    """Provider-level result of one model call (Rust ``GenerateResult``)."""

    content: List[GenerateContent]
    finish_reason: FinishReason
    usage: Usage
    warnings: List[Warning]
    provider_metadata: Optional[Any] = None
    request: Optional[RequestInfo] = None
    response: Optional[GenerateResponseMetadata] = None


class StepResponse(StepResponseMetadata):
    """The response of a step (Rust ``StepResponse``), with the step's
    response messages."""

    body: Optional[Any] = None
    messages: List[ModelMessage] = Field(default_factory=list)


class StepModel(_Wire):
    """The model that ran a step (Rust ``StepModel``)."""

    provider: str
    model_id: str


class StepResult(_Wire):
    """One model call of a text generation (Rust ``StepResult``, the AI SDK's
    ``StepResult``)."""

    call_id: str
    step_number: int
    model: StepModel
    content: List[TextContent] = Field(default_factory=list)
    finish_reason: FinishReasonUnified
    raw_finish_reason: Optional[str] = None
    usage: LanguageModelUsage
    # Timing of the step (Rust ``StepResultPerformance``). Weak type (Dict).
    performance: Dict[str, Any] = Field(default_factory=dict)
    warnings: List[Warning] = Field(default_factory=list)
    request: RequestInfo = Field(default_factory=RequestInfo)
    response: StepResponse = Field(default_factory=StepResponse)
    provider_metadata: Optional[Dict[str, Any]] = None


class GenerateTextResult(_Wire):
    """Result of ``generate_text`` and of a consumed stream (Rust
    ``GenerateTextResult``, the AI SDK's ``GenerateTextResult``).

    ``steps`` holds every model call; the other fields are read from the final
    step, except ``usage``/``total_usage`` (summed over the steps) and
    ``output`` (the parsed ``options.output``, absent when not parsed).
    """

    content: List[TextContent] = Field(default_factory=list)
    text: str
    # reasoning/files/tool_results use weak types (Dict) — use content for
    # full typing; response_messages reuses ModelMessage for next-turn prompt.
    reasoning: List[Dict[str, Any]] = Field(default_factory=list)
    reasoning_text: Optional[str] = None
    files: List[Dict[str, Any]] = Field(default_factory=list)
    sources: List[Source] = Field(default_factory=list)
    tool_calls: List[ToolCall] = Field(default_factory=list)
    tool_results: List[Dict[str, Any]] = Field(default_factory=list)
    finish_reason: FinishReasonUnified
    # The provider's own finish reason (e.g. "stop", "end_turn").
    raw_finish_reason: Optional[str] = None
    usage: LanguageModelUsage = Field(default_factory=LanguageModelUsage)
    # Deprecated upstream: equal to ``usage``.
    total_usage: LanguageModelUsage = Field(default_factory=LanguageModelUsage)
    warnings: List[Warning] = Field(default_factory=list)
    request: RequestInfo = Field(default_factory=RequestInfo)
    response: StepResponse = Field(default_factory=StepResponse)
    response_messages: List[ModelMessage] = Field(default_factory=list)
    # Provider-specific metadata (e.g. Anthropic cache info). Weak type (Dict).
    provider_metadata: Optional[Dict[str, Any]] = None
    steps: List[StepResult] = Field(default_factory=list)
    # The parsed ``options.output``; ``None`` when it was not parsed.
    output: Optional[Any] = None


class GenerateObjectResponse(StepResponseMetadata):
    """The response of a ``generate_object`` call (Rust
    ``GenerateObjectResponse``)."""

    body: Optional[Any] = None


class GenerateObjectResult(_Wire):
    """Result of the deprecated ``generate_object`` (Rust
    ``GenerateObjectResult``, the AI SDK's ``GenerateObjectResult``).
    """

    # `object` is an arbitrary JSON value — weak type (Any).
    object: Any
    # The reasoning text, ``None`` when there is none.
    reasoning: Optional[str] = None
    finish_reason: FinishReasonUnified
    usage: LanguageModelUsage = Field(default_factory=LanguageModelUsage)
    warnings: List[Warning] = Field(default_factory=list)
    request: RequestInfo = Field(default_factory=RequestInfo)
    response: GenerateObjectResponse = Field(default_factory=GenerateObjectResponse)
    # Provider-specific metadata (e.g. Anthropic cache info). Weak type (Dict).
    provider_metadata: Optional[Dict[str, Any]] = None


class ChatCompletionFunction(BaseModel):
    """Function payload of a tool call (Rust ``ChatCompletionFunction``)."""

    name: str
    arguments: str


class ChatCompletionToolCall(BaseModel):
    """A tool call in an assistant message (Rust ``ChatCompletionToolCall``)."""

    id: str
    type: str
    function: ChatCompletionFunction


class ChatCompletionMessage(BaseModel):
    """The assistant message in a ChatCompletion (Rust ``ChatCompletionMessage``)."""

    role: str
    content: Optional[str] = None
    reasoning_content: Optional[str] = None
    tool_calls: Optional[List[ChatCompletionToolCall]] = None
    annotations: Optional[List[Any]] = None


class ChatCompletionChoice(BaseModel):
    """A single choice in a ChatCompletion (Rust ``ChatCompletionChoice``)."""

    index: int
    message: ChatCompletionMessage
    finish_reason: Optional[str] = None
    logprobs: Optional[Any] = None


class PromptTokensDetails(BaseModel):
    """Prompt token breakdown (Rust ``PromptTokensDetails``)."""

    cached_tokens: int
    cache_write_tokens: Optional[int] = None


class CompletionTokensDetails(BaseModel):
    """Completion token breakdown (Rust ``CompletionTokensDetails``)."""

    reasoning_tokens: Optional[int] = None


class ChatCompletionUsage(BaseModel):
    """Token usage for a Chat Completion (Rust ``ChatCompletionUsage``)."""

    prompt_tokens: int
    completion_tokens: int
    total_tokens: int
    prompt_tokens_details: Optional[PromptTokensDetails] = None
    completion_tokens_details: Optional[CompletionTokensDetails] = None


class ChatCompletion(BaseModel):
    """A complete OpenAI Chat Completion response (Rust ``ChatCompletion``)."""

    id: str
    object: str
    created: int
    model: str
    choices: List[ChatCompletionChoice]
    usage: ChatCompletionUsage
    system_fingerprint: Optional[str] = None


class ChatCompletionChunkFunction(BaseModel):
    """Function payload of a chunk tool call (Rust ``ChatCompletionChunkFunction``)."""

    name: Optional[str] = None
    arguments: Optional[str] = None


class ChatCompletionChunkToolCall(BaseModel):
    """A tool-call delta in a streaming chunk (Rust ``ChatCompletionChunkToolCall``)."""

    index: int
    id: Optional[str] = None
    type: Optional[str] = None
    function: ChatCompletionChunkFunction


class ChatCompletionDelta(BaseModel):
    """The delta payload of a streaming chunk (Rust ``ChatCompletionDelta``)."""

    role: Optional[str] = None
    content: Optional[str] = None
    reasoning_content: Optional[str] = None
    tool_calls: Optional[List[ChatCompletionChunkToolCall]] = None


class ChatCompletionChunkChoice(BaseModel):
    """A single choice in a streaming chunk (Rust ``ChatCompletionChunkChoice``)."""

    index: int
    delta: ChatCompletionDelta
    finish_reason: Optional[str] = None
    logprobs: Optional[Any] = None


class ChatCompletionChunk(BaseModel):
    """A single OpenAI Chat Completion streaming chunk (Rust ``ChatCompletionChunk``)."""

    id: str
    object: str
    created: int
    model: str
    choices: List[ChatCompletionChunkChoice]
    usage: Optional[ChatCompletionUsage] = None


Prompt = Union[str, List[ModelMessage], List[Dict[str, Any]]]


def _prompt_to_json(prompt: Prompt) -> str:
    """Build the JSON prompt string expected by the native layer.

    A bare string is emitted as a JSON string (``ModelPrompt::Text``). A list
    of messages is emitted as an array (``ModelPrompt::Messages``).
    ``ModelMessage`` instances are serialized with ``mode="json"`` so nested
    models become JSON primitives.
    """
    if isinstance(prompt, str):
        return json.dumps(prompt)
    if not isinstance(prompt, list):
        raise TypeError(
            f"prompt must be str or list of messages, got {type(prompt).__name__}"
        )
    msgs = [
        m.model_dump(mode="json", by_alias=True)
        if isinstance(m, BaseModel)
        else m
        for m in prompt
    ]
    return json.dumps(msgs)


def _opts_to_json(options: Optional[GenerateTextOptions]) -> Optional[str]:
    """Serialize options to the JSON string expected by the native layer."""
    if options is None:
        return None
    if not isinstance(options, GenerateTextOptions):
        raise TypeError(
            "options must be a GenerateTextOptions instance, got "
            f"{type(options).__name__}"
        )
    return options.model_dump_json(by_alias=True)


def _params(prompt, options):
    return {
        "prompt": json.loads(_prompt_to_json(prompt)),
        "options": json.loads(_opts_to_json(options) or "{}"),
    }


def generate_text(model: Handle, prompt: Prompt, options=None) -> GenerateTextResult:
    """Call text.generate using the shared wire DTO."""
    return GenerateTextResult.model_validate(model.call("text.generate", _params(prompt, options)))


def stream_text(model: Handle, prompt: Prompt, options=None) -> Iterator[dict]:
    """Yield text events; closing the generator cancels the native stream."""
    with model.stream("text.stream", _params(prompt, options)) as stream:
        for frame in stream:
            yield frame["event"]


_STREAM_PART = TypeAdapter(StreamPart)


def parse_stream_part(part: dict) -> BaseModel:
    return _STREAM_PART.validate_python(part)


def generate_text_as_openai(model: Handle, prompt: Prompt, options=None) -> ChatCompletion:
    return ChatCompletion.model_validate(model.call("text.generateOpenAI", _params(prompt, options)))


def stream_text_as_openai(model: Handle, prompt: Prompt, options=None) -> Iterator[ChatCompletionChunk]:
    with model.stream("text.streamOpenAI", _params(prompt, options)) as stream:
        for frame in stream:
            yield ChatCompletionChunk.model_validate(frame["event"])
