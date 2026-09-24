import json
from pathlib import Path

import pytest

from aimux import (
    InvalidArgumentError,
    apply_tool_call_repair,
    apply_tool_call_repair_to_result,
    generate_text,
    generate_text_as_openai,
    openai,
    stream_text,
    tool_call_repair_context,
)
from aimux import wrapper as typed
from test_e2e import MockServer, OPENAI_TOOL_CALL

WEATHER_SCHEMA = {
    "type": "object",
    "properties": {"city": {"type": "string"}},
    "required": ["city"],
    "additionalProperties": False,
}
TOOLS = [{"type": "function", "name": "get_weather", "input_schema": WEATHER_SCHEMA}]
TYPED_TOOLS = [typed.FunctionTool(name="get_weather", input_schema=WEATHER_SCHEMA)]
# The same canned response, with arguments the schema accepts.
VALID_TOOL_CALL = OPENAI_TOOL_CALL.replace("location", "city")
TOOL_CALL_STREAM = (
    'data: {"id":"1","model":"gpt-4o","choices":[{"delta":{"role":"assistant",'
    '"tool_calls":[{"index":0,"id":"call_s1","type":"function",'
    '"function":{"name":"get_weather","arguments":""}}]}}]}\n\n'
    'data: {"id":"1","model":"gpt-4o","choices":[{"delta":{"tool_calls":[{"index":0,'
    '"function":{"arguments":"{\\"location\\":\\"Tokyo\\"}"}}]}}]}\n\n'
    'data: {"id":"1","model":"gpt-4o","choices":[{"delta":{},"finish_reason":"tool_calls"}],'
    '"usage":{"prompt_tokens":5,"completion_tokens":2,"total_tokens":7}}\n\n'
    'data: [DONE]\n\n'
)


def _repair(context):
    return dict(context["tool_call"], input='{"city":"Tokyo"}')


def _cases():
    path = Path(__file__).resolve().parents[3] / "contract-tests/fixtures/tool-call-repair.json"
    return json.loads(path.read_text(encoding="utf-8"))["cases"]


def _call(case):
    args = case["input"]
    opts = None if args["opts"] is None else json.dumps(args["opts"])
    if case["function"] == "tool_call_repair_context":
        return tool_call_repair_context(json.dumps(args["tool_call"]), json.dumps(args["prompt"]), opts)
    if case["function"] == "apply_tool_call_repair":
        return apply_tool_call_repair(json.dumps(args["tool_call"]), opts, json.dumps(args["reply"]))
    if case["function"] == "apply_tool_call_repair_to_result":
        return apply_tool_call_repair_to_result(
            json.dumps(args["result"]), opts, args["tool_call_id"], json.dumps(args["reply"])
        )
    raise AssertionError(f"unknown fixture function {case['function']!r}")


@pytest.mark.parametrize("case", _cases(), ids=lambda case: case["name"])
def test_shared_fixture(case):
    if case.get("expected_error"):
        with pytest.raises(InvalidArgumentError):
            _call(case)
    else:
        assert json.loads(_call(case)) == case["expected"]


def test_generate_repairs_call_and_transcript():
    captured = {}

    def repair(context):
        captured.update(context)
        return _repair(context)

    with MockServer(OPENAI_TOOL_CALL) as mock:
        model = openai("test-key", "gpt-4o", mock.url)
        result = generate_text(model, "What's the weather in Tokyo?", {
            "tools": TOOLS,
            "repair_tool_call": repair,
        })

    assert captured["tool_call"]["input"] == '{"location":"Tokyo"}'
    assert result["tool_calls"][0]["input"] == {"city": "Tokyo"}
    transcript_calls = [
        part
        for message in result["response_messages"]
        for part in message["content"]
        if isinstance(part, dict) and part.get("type") == "tool_call"
    ]
    assert [part["input"] for part in transcript_calls] == [result["tool_calls"][0]["input"]]


def test_stream_repairs_tool_call_and_preserves_deltas():
    with MockServer(TOOL_CALL_STREAM, content_type="text/event-stream") as mock:
        model = openai("test-key", "gpt-4o", mock.url)
        parts = list(stream_text(model, "What's the weather in Tokyo?", {
            "tools": TOOLS,
            "repair_tool_call": _repair,
        }))

    calls = [part["ToolCall"] for part in parts if "ToolCall" in part]
    assert len(calls) == 1
    assert calls[0]["input"] == {"city": "Tokyo"}
    deltas = "".join(part["ToolInputDelta"]["delta"] for part in parts if "ToolInputDelta" in part)
    assert deltas == '{"location":"Tokyo"}'


def test_generate_as_openai_carries_the_repaired_arguments():
    # A ChatCompletion has no `invalid` marker: the binding repairs the native
    # result and converts it, on both API layers.
    with MockServer(OPENAI_TOOL_CALL) as mock:
        model = openai("test-key", "gpt-4o", mock.url)
        plain = generate_text_as_openai(model, "What's the weather in Tokyo?", {
            "tools": TOOLS,
            "repair_tool_call": _repair,
        })
        completion = typed.generate_text_as_openai(
            model,
            "What's the weather in Tokyo?",
            typed.GenerateTextOptions(
                tools=TYPED_TOOLS,
                repair_tool_call=_typed_repair,
            ),
        )

    arguments = plain["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"]
    assert json.loads(arguments) == {"city": "Tokyo"}
    typed_call = completion.choices[0].message.tool_calls[0]
    assert json.loads(typed_call.function.arguments) == {"city": "Tokyo"}


def test_a_wrong_return_type_is_an_error_not_a_failed_repair():
    # Only the hook's own exception means "failed"; the typed layer's encoding
    # of what it returned is not the hook's failure and must surface as itself.
    options = typed.GenerateTextOptions(
        tools=TYPED_TOOLS,
        repair_tool_call=lambda ctx: {"tool_call_id": "x", "tool_name": "y", "input": "{}"},
    )
    with MockServer(OPENAI_TOOL_CALL) as mock:
        model = openai("test-key", "gpt-4o", mock.url)
        with pytest.raises(AttributeError):
            typed.generate_text(model, "What's the weather in Tokyo?", options)


def _raise(_context):
    raise RuntimeError("repair model unavailable")


def _typed_repair(context):
    return context.tool_call.model_copy(update={"input": '{"city":"Tokyo"}'})


def test_repair_loop_branches():
    """Host-loop branches the shared fixture cannot reach, on the typed layer."""
    cases = [
        # name, response, with_tools, hook, expected hook invocations, error check
        ("raises", OPENAI_TOOL_CALL, True, _raise, 1, lambda e: (
            e["ToolCallRepair"]["cause"] == {"Other": "repair model unavailable"}
            and "InvalidToolInput" in e["ToolCallRepair"]["original_error"]
        )),
        ("returns None", OPENAI_TOOL_CALL, True, lambda ctx: None, 1,
         lambda e: set(e) == {"InvalidToolInput"}),
        ("valid call", VALID_TOOL_CALL, True, _typed_repair, 0, None),
        ("no tools", OPENAI_TOOL_CALL, False, _typed_repair, 0,
         lambda e: set(e) == {"NoSuchTool"}),
    ]

    for name, response, with_tools, hook, invocations, check_error in cases:
        seen = []

        def spy(context, hook=hook, seen=seen):
            seen.append(context)
            return hook(context)

        options = typed.GenerateTextOptions(
            tools=TYPED_TOOLS if with_tools else None,
            repair_tool_call=spy,
        )
        with MockServer(response) as mock:
            model = openai("test-key", "gpt-4o", mock.url)
            result = typed.generate_text(model, "What's the weather in Tokyo?", options)

        assert len(seen) == invocations, name
        call = result.tool_calls[0]
        if check_error is None:
            assert not call.invalid and call.input == {"city": "Tokyo"}, name
        else:
            assert call.invalid is True, name
            assert check_error(call.error), (name, call.error)
