// Host-side tool-call repair (RFC-0035).
//
// An unparseable tool call never fails generation: it comes back with
// `invalid: true` and an `error`. Repair is therefore post-processing done
// here, in Go — the binding calls the normal entry point, then feeds every
// invalid call through three pure native functions (no handle, no callback,
// no session; JSON in, JSON out) with the caller's repair function in between.

package aimux

/*
#include <stdlib.h>
#include "aimux-ffi.h"
*/
import "C"

import (
	"encoding/json"
	"fmt"
	"unsafe"
)

// RawToolCall is a tool call as the provider emitted it: Input is the raw
// argument *text*, not a parsed object. It is what a repair function reads
// and what it returns.
type RawToolCall struct {
	ToolCallID       string          `json:"tool_call_id"`
	ToolName         string          `json:"tool_name"`
	Input            string          `json:"input"`
	ProviderExecuted *bool           `json:"provider_executed,omitempty"`
	Dynamic          *bool           `json:"dynamic,omitempty"`
	ThoughtSignature *string         `json:"thought_signature,omitempty"`
	ProviderMetadata json.RawMessage `json:"provider_metadata,omitempty"`
}

// ToolCallRepairContext is the argument handed to a RepairToolCallFunc — the
// same shape as the AI SDK's repairToolCall argument.
//
// Messages and Instructions are the transcript the model saw, derived by the
// engine from the prompt and options of the call being repaired.
type ToolCallRepairContext struct {
	// ToolCall is the failed call, with its raw argument text.
	ToolCall RawToolCall `json:"tool_call"`
	// Error is the serialized AiMuxError the call carries (e.g.
	// {"InvalidToolInput": {...}} or {"NoSuchTool": {...}}).
	Error json.RawMessage `json:"error"`
	// InputSchema is the JSON schema of the named tool, or an empty-object
	// schema when the name does not resolve to a tool.
	InputSchema  json.RawMessage `json:"input_schema"`
	Tools        []Tool          `json:"tools"`
	Messages     []ModelMessage  `json:"messages"`
	Instructions *string         `json:"instructions"`
}

// RepairToolCallFunc repairs one invalid tool call.
//
// Return a replacement call to have it re-parsed and re-validated, nil to
// leave the call invalid as it is, or an error to record the repair attempt
// as failed (the call stays invalid, carrying a ToolCallRepair error whose
// cause is the error's message). It runs outside any native call, so it may
// itself call back into aimux — e.g. ask a model to fix the arguments.
//
// At most one attempt is made per call.
type RepairToolCallFunc func(ctx *ToolCallRepairContext) (*RawToolCall, error)

// ── Native wrappers ─────────────────────────────────────────────────────────

// cOptsString turns an options JSON string into a C string ("" → NULL, which
// the C ABI reads as "defaults"). The returned func frees it.
func cOptsString(optsJSON string) (*C.char, func()) {
	if optsJSON == "" {
		return nil, func() {}
	}
	p := C.CString(optsJSON)
	return p, func() { C.free(unsafe.Pointer(p)) }
}

// toolCallRepairContextJSON builds the repair argument for one invalid call.
// It returns the JSON literal "null" when the call was made without a tool
// set: such a call is never repaired (the AI SDK rule), so the caller skips it.
func toolCallRepairContextJSON(toolCallJSON, promptJSON, optsJSON string) (string, error) {
	cCall := C.CString(toolCallJSON)
	defer C.free(unsafe.Pointer(cCall))
	cPrompt := C.CString(promptJSON)
	defer C.free(unsafe.Pointer(cPrompt))
	cOpts, freeOpts := cOptsString(optsJSON)
	defer freeOpts()

	return ffiString(func(out **C.char) *C.aimux_error_t {
		return C.aimux_tool_call_repair_context(cCall, cPrompt, cOpts, out)
	})
}

// applyToolCallRepairJSON resolves one tool call against a reply, returning
// the resulting ToolCall JSON.
func applyToolCallRepairJSON(toolCallJSON, optsJSON, replyJSON string) (string, error) {
	cCall := C.CString(toolCallJSON)
	defer C.free(unsafe.Pointer(cCall))
	cReply := C.CString(replyJSON)
	defer C.free(unsafe.Pointer(cReply))
	cOpts, freeOpts := cOptsString(optsJSON)
	defer freeOpts()

	return ffiString(func(out **C.char) *C.aimux_error_t {
		return C.aimux_apply_tool_call_repair(cCall, cOpts, cReply, out)
	})
}

// applyToolCallRepairToResultJSON patches a whole result document: both
// tool_calls and the matching response_messages tool-call part.
func applyToolCallRepairToResultJSON(resultJSON, optsJSON, toolCallID, replyJSON string) (string, error) {
	cResult := C.CString(resultJSON)
	defer C.free(unsafe.Pointer(cResult))
	cID := C.CString(toolCallID)
	defer C.free(unsafe.Pointer(cID))
	cReply := C.CString(replyJSON)
	defer C.free(unsafe.Pointer(cReply))
	cOpts, freeOpts := cOptsString(optsJSON)
	defer freeOpts()

	return ffiString(func(out **C.char) *C.aimux_error_t {
		return C.aimux_apply_tool_call_repair_to_result(cResult, cOpts, cID, cReply, out)
	})
}

// ── Host loop ───────────────────────────────────────────────────────────────

// repairReply runs the user's function over a context JSON and encodes its
// outcome as a ToolCallRepairReply. A returned error becomes a "failed"
// reply — it is the user's verdict on this call, not a failure of the call.
func repairReply(repair RepairToolCallFunc, contextJSON string) (string, error) {
	var ctx ToolCallRepairContext
	if err := json.Unmarshal([]byte(contextJSON), &ctx); err != nil {
		return "", fmt.Errorf("aimux: failed to parse tool-call repair context: %w", err)
	}
	replacement, err := repair(&ctx)
	if err != nil {
		reply, mErr := json.Marshal(struct {
			Type    string `json:"type"`
			Message string `json:"message"`
		}{"failed", err.Error()})
		if mErr != nil {
			return "", fmt.Errorf("aimux: failed to encode repair failure: %w", mErr)
		}
		return string(reply), nil
	}
	if replacement == nil {
		return `{"type":"unchanged"}`, nil
	}
	reply, err := json.Marshal(struct {
		Type     string       `json:"type"`
		ToolCall *RawToolCall `json:"tool_call"`
	}{"repaired", replacement})
	if err != nil {
		return "", fmt.Errorf("aimux: failed to encode repaired tool call: %w", err)
	}
	return string(reply), nil
}

// invalidToolCalls lists the invalid tool calls of a result document, in
// order, as (id, JSON) pairs. A GenerateObjectResult carries the text result
// under `raw`; a GenerateTextResult's own `raw` has no tool_calls, so the
// same lookup covers both — and StreamTextResultAggregated, whose tool_calls
// are top-level.
func invalidToolCalls(resultJSON string) [][2]string {
	var doc struct {
		ToolCalls []json.RawMessage `json:"tool_calls"`
		Raw       struct {
			ToolCalls []json.RawMessage `json:"tool_calls"`
		} `json:"raw"`
	}
	if json.Unmarshal([]byte(resultJSON), &doc) != nil {
		return nil
	}
	calls := doc.ToolCalls
	if calls == nil {
		calls = doc.Raw.ToolCalls
	}
	var invalid [][2]string
	for _, call := range calls {
		var head struct {
			ToolCallID string `json:"tool_call_id"`
			Invalid    *bool  `json:"invalid"`
		}
		if json.Unmarshal(call, &head) != nil || head.Invalid == nil || !*head.Invalid {
			continue
		}
		invalid = append(invalid, [2]string{head.ToolCallID, string(call)})
	}
	return invalid
}

// repairResultJSON is the non-streaming repair loop, shared by every typed
// entry point that returns a whole document: for each invalid call, build its
// context, ask the user's function, and patch the document with the answer.
// The result is the patched JSON, ready to decode. Patching one call never
// touches another, so the calls are collected once up front.
func repairResultJSON(resultJSON, promptJSON, optsJSON string, repair RepairToolCallFunc) (string, error) {
	for _, call := range invalidToolCalls(resultJSON) {
		id, callJSON := call[0], call[1]
		contextJSON, err := toolCallRepairContextJSON(callJSON, promptJSON, optsJSON)
		if err != nil {
			return "", err
		}
		if contextJSON == "null" {
			continue
		}
		reply, err := repairReply(repair, contextJSON)
		if err != nil {
			return "", err
		}
		resultJSON, err = applyToolCallRepairToResultJSON(resultJSON, optsJSON, id, reply)
		if err != nil {
			return "", err
		}
	}
	return resultJSON, nil
}

// repairStreamToolCall is the streaming counterpart: it resolves one
// {"ToolCall": …} stream part payload, returning the payload to deliver.
// A valid call, or one made without a tool set, passes through untouched.
// Tool-input delta parts never reach here — they are forwarded immediately,
// as the AI SDK does.
func repairStreamToolCall(payload []byte, promptJSON, optsJSON string, repair RepairToolCallFunc) ([]byte, error) {
	var call struct {
		Invalid *bool `json:"invalid"`
	}
	if err := json.Unmarshal(payload, &call); err != nil || call.Invalid == nil || !*call.Invalid {
		return payload, nil
	}
	contextJSON, err := toolCallRepairContextJSON(string(payload), promptJSON, optsJSON)
	if err != nil {
		return nil, err
	}
	if contextJSON == "null" {
		return payload, nil
	}
	reply, err := repairReply(repair, contextJSON)
	if err != nil {
		return nil, err
	}
	repaired, err := applyToolCallRepairJSON(string(payload), optsJSON, reply)
	if err != nil {
		return nil, err
	}
	return []byte(repaired), nil
}

// maybeRepairResult applies the non-streaming repair loop when opts carry a
// repair function, and is a pass-through otherwise.
func maybeRepairResult(resultJSON, promptJSON, optsJSON string, opts *GenerateTextOptions) (string, error) {
	if opts == nil || opts.RepairToolCall == nil {
		return resultJSON, nil
	}
	return repairResultJSON(resultJSON, promptJSON, optsJSON, opts.RepairToolCall)
}
