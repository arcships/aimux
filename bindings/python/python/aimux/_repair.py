"""Host-side tool-call repair loop (RFC-0035), shared by both API layers.

The native layer never calls back into Python: it hands out invalid tool calls
as data, and these helpers re-enter it with the host's answer through three
pure functions. Everything here speaks JSON strings and plain dicts; the typed
wrapper adapts its pydantic models on the way in and out.
"""

from __future__ import annotations

import json
from typing import Any, Callable, Dict, Optional

from .aimux import (
    apply_tool_call_repair,
    apply_tool_call_repair_to_result,
    tool_call_repair_context,
)

#: Layer adapter: takes the repair-context dict, returns a replacement raw tool
#: call dict, or None to leave the call unchanged. Raising
#: :class:`RepairFailed` (via :func:`run_hook`) means "failed"; any other
#: exception is a bug on this side and propagates.
RepairAdapter = Callable[[Dict[str, Any]], Optional[Dict[str, Any]]]


class RepairFailed(Exception):
    """The caller's repair function raised; only its message crosses over."""


def run_hook(fn: Callable[[Any], Any], argument: Any) -> Any:
    """Call the caller's repair function.

    Only an exception raised by ``fn`` itself becomes a ``failed`` reply; the
    adapters' own decoding and encoding stay outside, so a bug there surfaces
    as itself instead of posing as a repair failure.
    """
    try:
        return fn(argument)
    except Exception as exc:
        raise RepairFailed(str(exc) or type(exc).__name__) from exc


def _reply(repair: RepairAdapter, context: Dict[str, Any]) -> str:
    try:
        replacement = repair(context)
    except RepairFailed as exc:
        # The host language's exception has no counterpart on the Rust side:
        # only its message survives, as the cause of a ToolCallRepairError.
        return json.dumps({"type": "failed", "message": str(exc)})
    if replacement is None:
        return json.dumps({"type": "unchanged"})
    return json.dumps({"type": "repaired", "tool_call": replacement})


def _context(call: Dict[str, Any], prompt_json: str, opts_json: Optional[str]):
    """The AI SDK repair argument, or None when the call is not repairable."""
    return json.loads(tool_call_repair_context(json.dumps(call), prompt_json, opts_json))


def repair_result(
    result_json: str,
    prompt_json: str,
    opts_json: Optional[str],
    repair: RepairAdapter,
) -> str:
    """Repair every invalid tool call of a serialized result, in order.

    Handles ``generate_text`` / ``generate_object`` (``raw.tool_calls``) and the
    aggregated stream result alike; one repair attempt per call, as in the
    AI SDK. Returns the patched result JSON.
    """
    result = json.loads(result_json)
    calls = result.get("tool_calls")
    if calls is None:
        calls = result.get("raw", {}).get("tool_calls") or []
    for call in calls:
        if not call.get("invalid"):
            continue
        context = _context(call, prompt_json, opts_json)
        if context is None:
            continue
        result_json = apply_tool_call_repair_to_result(
            result_json, opts_json, call["tool_call_id"], _reply(repair, context)
        )
    return result_json


def generate_openai_with_repair(
    model: Any,
    prompt_json: str,
    opts_json: Optional[str],
    repair: Optional[RepairAdapter],
) -> str:
    """``generate_text_as_openai`` with repair, for both API layers.

    A ChatCompletion has no ``invalid`` marker, so with a repair function the
    native result is generated, repaired, then converted.
    """
    if repair is None:
        return model.generate_text_as_openai(prompt_json, opts_json)
    result_json = model.generate_text(prompt_json, opts_json)
    result_json = repair_result(result_json, prompt_json, opts_json, repair)
    return model.generate_text_result_as_openai(result_json)


def repair_stream_part(
    part_json: str,
    prompt_json: str,
    opts_json: Optional[str],
    repair: RepairAdapter,
) -> str:
    """Replace an invalid ``ToolCall`` stream part with its repaired form.

    Every other part — tool-input deltas included — is forwarded untouched, so
    repair never delays the stream.
    """
    part = json.loads(part_json)
    call = part.get("ToolCall") if isinstance(part, dict) else None
    if call is None or not call.get("invalid"):
        return part_json
    context = _context(call, prompt_json, opts_json)
    if context is None:
        return part_json
    repaired = apply_tool_call_repair(
        json.dumps(call), opts_json, _reply(repair, context)
    )
    return json.dumps({"ToolCall": json.loads(repaired)})
