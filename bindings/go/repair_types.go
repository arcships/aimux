package aimux

import "encoding/json"

// RawToolCall is a tool call as the provider emitted it: Input is the raw
// argument *text*, not a parsed object. It is what a repair function reads
// and what it returns. Its JSON is the AI SDK's LanguageModelV4ToolCall,
// tagged {"type":"tool-call",...}.
type RawToolCall struct {
	ToolCallID       string          `json:"toolCallId"`
	ToolName         string          `json:"toolName"`
	Input            string          `json:"input"`
	ProviderExecuted *bool           `json:"providerExecuted,omitempty"`
	Dynamic          *bool           `json:"dynamic,omitempty"`
	ProviderMetadata json.RawMessage `json:"providerMetadata,omitempty"`
}

// MarshalJSON writes the call with its "type":"tool-call" tag first.
func (c RawToolCall) MarshalJSON() ([]byte, error) {
	type plain RawToolCall
	return json.Marshal(struct {
		Type string `json:"type"`
		plain
	}{"tool-call", plain(c)})
}

// UnmarshalJSON reads a call with the canonical "type":"tool-call" tag.
func (c *RawToolCall) UnmarshalJSON(data []byte) error {
	type plain RawToolCall
	var tagged struct {
		Type json.RawMessage `json:"type"`
		plain
	}
	if err := json.Unmarshal(data, &tagged); err != nil {
		return err
	}
	if err := checkTypeTag(tagged.Type, "tool-call"); err != nil {
		return err
	}
	*c = RawToolCall(tagged.plain)
	return nil
}

// ToolCallRepairContext is the argument handed to a RepairToolCallFunc — the
// same shape as the AI SDK's repairToolCall argument.
//
// Messages and Instructions are the transcript the model saw, derived by the
// engine from the prompt and options of the call being repaired.
type ToolCallRepairContext struct {
	// ToolCall is the failed call, with its raw argument text.
	ToolCall RawToolCall `json:"toolCall"`
	// Error is the serialized AiMuxError (AI SDK error JSON, tagged by "name" the call carries (e.g.
	// {"name":"AI_InvalidToolInputError",...} or {"name":"AI_NoSuchToolError",...}).
	Error json.RawMessage `json:"error"`
	// InputSchema is the JSON schema of the named tool, or an empty-object
	// schema when the name does not resolve to a tool.
	InputSchema  json.RawMessage `json:"inputSchema"`
	Tools        []Tool          `json:"tools"`
	Messages     []ModelMessage  `json:"messages"`
	Instructions *string         `json:"instructions"`
}
