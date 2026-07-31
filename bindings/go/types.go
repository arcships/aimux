// Typed data structures mirroring the aimux-core wire format (same shapes as
// the Kotlin Types.kt and the generated TypeScript .d.ts types).
//
// Field names use JSON tags matching the wire format's snake_case. The raw JSON
// boundary is handled by Model.GenerateText / Model.StreamText — this layer
// only provides typed parsing so callers don't manually dig through JSON.
//
// These types are intentionally lenient on decode (unknown keys ignored, every
// field optional) so future engine additions don't break existing clients.

package aimux

import (
	"encoding/json"
	"fmt"
)

// ── Enums (string-backed on the wire) ──────────────────────────────────────

type Role string

const (
	RoleSystem    Role = "system"
	RoleUser      Role = "user"
	RoleAssistant Role = "assistant"
	RoleTool      Role = "tool"
)

type FinishReasonUnified string

const (
	FinishStop          FinishReasonUnified = "stop"
	FinishLength        FinishReasonUnified = "length"
	FinishContentFilter FinishReasonUnified = "content-filter"
	FinishToolCalls     FinishReasonUnified = "tool-calls"
	FinishError         FinishReasonUnified = "error"
	FinishOther         FinishReasonUnified = "other"
)

type ToolChoice = json.RawMessage

// ToolChoiceAuto/None/Required are helper constructors for the string variants.
func ToolChoiceAuto() ToolChoice     { return json.RawMessage(`"auto"`) }
func ToolChoiceNone() ToolChoice     { return json.RawMessage(`"none"`) }
func ToolChoiceRequired() ToolChoice { return json.RawMessage(`"required"`) }

// ToolChoiceTool builds a ToolChoice selecting a specific tool.
func ToolChoiceTool(name string) ToolChoice {
	return json.RawMessage(`{"type":"tool","toolName":"` + name + `"}`)
}

// ── Core types ───────────────────────────────────────────────────────────────

// TokenUsage is token usage detail with cache breakdown.
type TokenUsage struct {
	Total     *int64 `json:"total,omitempty"`
	NoCache   *int64 `json:"no_cache,omitempty"`
	CacheRead *int64 `json:"cache_read,omitempty"`
	CacheWrite *int64 `json:"cache_write,omitempty"`
	Text      *int64 `json:"text,omitempty"`
	Reasoning *int64 `json:"reasoning,omitempty"`
}

// Usage is token usage statistics.
type Usage struct {
	InputTokens  TokenUsage    `json:"input_tokens,omitempty"`
	OutputTokens TokenUsage    `json:"output_tokens,omitempty"`
	Raw          json.RawMessage `json:"raw,omitempty"`
}

// FinishReason is the finish reason.
type FinishReason struct {
	Unified FinishReasonUnified `json:"unified,omitempty"`
	Raw     json.RawMessage     `json:"raw,omitempty"`
}

// ToolCall represents a tool call requested by the model.
type ToolCall struct {
	ToolCallID string          `json:"tool_call_id"`
	ToolName   string          `json:"tool_name"`
	Input      json.RawMessage `json:"input,omitempty"`
}

// ContentPart is a single content part in the raw response.
// The wire format uses internally-tagged unions; we keep it as raw JSON
// for forward compatibility.
type ContentPart = json.RawMessage

// GenerateResult is the raw provider result.
type GenerateResult struct {
	Content     []ContentPart `json:"content"`
	FinishReason FinishReason `json:"finish_reason"`
	Usage       Usage         `json:"usage"`
}

// GenerateTextResult is the typed result of a GenerateText call.
type GenerateTextResult struct {
	Text     string           `json:"text"`
	ToolCalls []ToolCall      `json:"tool_calls,omitempty"`
	Raw      GenerateResult   `json:"raw"`
	Usage    Usage            `json:"usage"`
	FinishReason FinishReason `json:"finish_reason"`
}

// ParseGenerateTextResult parses the JSON string returned by Model.GenerateText
// into a typed GenerateTextResult.
func ParseGenerateTextResult(jsonStr string) (*GenerateTextResult, error) {
	var r GenerateTextResult
	if err := json.Unmarshal([]byte(jsonStr), &r); err != nil {
		return nil, fmt.Errorf("aimux: failed to parse GenerateTextResult: %w", err)
	}
	return &r, nil
}

// ── ModelMessage (for multi-role prompts) ─────────────────────────────────────

// ModelMessage is a single message in a conversation.
type ModelMessage struct {
	Role    Role   `json:"role"`
	Content string `json:"content"`
}

// MarshalMessages serializes a slice of ModelMessage to JSON for use as a prompt.
func MarshalMessages(msgs []ModelMessage) (string, error) {
	b, err := json.Marshal(msgs)
	if err != nil {
		return "", fmt.Errorf("aimux: failed to marshal messages: %w", err)
	}
	return string(b), nil
}

// ── GenerateTextOptions (for typed options building) ──────────────────────────

// GenerateTextOptions is the typed options for text generation.
// All fields are optional (pointer types) to match the engine's schema.
type GenerateTextOptions struct {
	MaxOutputTokens   *int               `json:"max_output_tokens,omitempty"`
	Temperature        *float64           `json:"temperature,omitempty"`
	StopSequences      []string           `json:"stop_sequences,omitempty"`
	TopP               *float64           `json:"top_p,omitempty"`
	TopK               *int               `json:"top_k,omitempty"`
	PresencePenalty    *float64           `json:"presence_penalty,omitempty"`
	FrequencyPenalty   *float64           `json:"frequency_penalty,omitempty"`
	ResponseFormat    json.RawMessage    `json:"response_format,omitempty"`
	Seed               *int64             `json:"seed,omitempty"`
	Tools             []Tool              `json:"tools,omitempty"`
	ToolChoice        ToolChoice          `json:"tool_choice,omitempty"`
	Headers           map[string]string   `json:"headers,omitempty"`
	ProviderOptions   json.RawMessage    `json:"provider_options,omitempty"`
	Instructions       *string            `json:"instructions,omitempty"`
}

// Tool is a function tool definition.
type Tool struct {
	Type        string          `json:"type"`         // always "function"
	Name        string          `json:"name"`
	InputSchema json.RawMessage `json:"input_schema,omitempty"`
}

// MarshalOptions serializes GenerateTextOptions to JSON. Returns "" for nil opts.
func MarshalOptions(opts *GenerateTextOptions) (string, error) {
	if opts == nil {
		return "", nil
	}
	b, err := json.Marshal(opts)
	if err != nil {
		return "", fmt.Errorf("aimux: failed to marshal options: %w", err)
	}
	return string(b), nil
}

// ── StreamPart parsing ─────────────────────────────────────────────────────

// StreamPart is a parsed stream part. The wire format uses externally-tagged
// JSON (e.g. {"TextDelta":{"delta":"..."}}). We decode the tag and payload.
type StreamPart struct {
	Tag     string          `json:"-"`
	Payload json.RawMessage `json:"-"`
}

// ParseStreamPart parses a StreamPart JSON string.
func ParseStreamPart(jsonStr string) (*StreamPart, error) {
	var raw map[string]json.RawMessage
	if err := json.Unmarshal([]byte(jsonStr), &raw); err != nil {
		return nil, fmt.Errorf("aimux: failed to parse StreamPart: %w", err)
	}
	// Externally-tagged: the map should have exactly one key.
	for tag, payload := range raw {
		return &StreamPart{Tag: tag, Payload: payload}, nil
	}
	return &StreamPart{}, nil
}

// TextDeltaPayload is the payload of a {"TextDelta":{...}} stream part.
type TextDeltaPayload struct {
	ID    string `json:"id,omitempty"`
	Delta string `json:"delta"`
}
