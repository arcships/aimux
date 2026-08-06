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

// ReasoningEffort controls how much reasoning effort the model spends.
// Mirrors Kotlin ReasoningEffort (Types.kt:72-81).
type ReasoningEffort string

const (
	ReasoningProviderDefault ReasoningEffort = "provider-default"
	ReasoningNone            ReasoningEffort = "none"
	ReasoningMinimal         ReasoningEffort = "minimal"
	ReasoningLow             ReasoningEffort = "low"
	ReasoningMedium          ReasoningEffort = "medium"
	ReasoningHigh            ReasoningEffort = "high"
	ReasoningXHigh           ReasoningEffort = "xhigh"
)

// ToolChoice is polymorphic on the wire: bare string ("auto"/"none"/"required")
// or tagged object ({"type":"tool","toolName":"..."}). Modeled as raw JSON to
// preserve both shapes; use the constructors below.
type ToolChoice = json.RawMessage

// ToolChoiceAuto/None/Required are helper constructors for the string variants.
func ToolChoiceAuto() ToolChoice     { return json.RawMessage(`"auto"`) }
func ToolChoiceNone() ToolChoice     { return json.RawMessage(`"none"`) }
func ToolChoiceRequired() ToolChoice { return json.RawMessage(`"required"`) }

// ToolChoiceTool builds a ToolChoice selecting a specific tool.
// Uses json.Marshal with a struct to ensure correct field order and escaping.
func ToolChoiceTool(name string) ToolChoice {
	b, err := json.Marshal(struct {
		Type     string `json:"type"`
		ToolName string `json:"toolName"`
	}{Type: "tool", ToolName: name})
	if err != nil {
		// Should not happen for a string — fall back to a safe literal.
		return json.RawMessage(`{"type":"tool","toolName":""}`)
	}
	return json.RawMessage(b)
}

// ── Core types ───────────────────────────────────────────────────────────────

// TokenUsage is token usage detail with cache breakdown.
type TokenUsage struct {
	Total      *uint32 `json:"total,omitempty"`
	NoCache    *uint32 `json:"no_cache,omitempty"`
	CacheRead  *uint32 `json:"cache_read,omitempty"`
	CacheWrite *uint32 `json:"cache_write,omitempty"`
	Text       *uint32 `json:"text,omitempty"`
	Reasoning  *uint32 `json:"reasoning,omitempty"`
}

// Usage is token usage statistics.
type Usage struct {
	InputTokens  TokenUsage      `json:"input_tokens,omitempty"`
	OutputTokens TokenUsage      `json:"output_tokens,omitempty"`
	Raw          json.RawMessage `json:"raw,omitempty"`
}

// FinishReason is the finish reason.
type FinishReason struct {
	Unified FinishReasonUnified `json:"unified,omitempty"`
	Raw     *string             `json:"raw,omitempty"`
}

// ToolCall represents a tool call requested by the model.
// Mirrors Kotlin ToolCall (Types.kt:157-164).
type ToolCall struct {
	ToolCallID       string          `json:"tool_call_id"`
	ToolName         string          `json:"tool_name"`
	Input            json.RawMessage `json:"input,omitempty"`
	ProviderExecuted *bool           `json:"provider_executed,omitempty"`
	Dynamic          *bool           `json:"dynamic,omitempty"`
	ThoughtSignature *string         `json:"thought_signature,omitempty"`
}

// ContentPart is a single content part in the raw response.
// The wire format uses internally-tagged unions; we keep it as raw JSON
// for forward compatibility.
type ContentPart = json.RawMessage

// ResponseMetadata describes the provider HTTP response.
// Mirrors Kotlin ResponseMetadata (Types.kt:141-146).
type ResponseMetadata struct {
	ID        *string `json:"id,omitempty"`
	Timestamp *string `json:"timestamp,omitempty"`
	ModelID   *string `json:"model_id,omitempty"`
}

// GenerateResult is the raw provider result.
// Mirrors Kotlin GenerateResult (Types.kt:853-871).
type GenerateResult struct {
	Content          []ContentPart     `json:"content,omitempty"`
	FinishReason     FinishReason      `json:"finish_reason,omitempty"`
	Usage            Usage             `json:"usage,omitempty"`
	Warnings         []json.RawMessage `json:"warnings,omitempty"`
	ProviderMetadata json.RawMessage   `json:"provider_metadata,omitempty"`
	Response         ResponseMetadata  `json:"response,omitempty"`
	RequestBody      json.RawMessage   `json:"request_body,omitempty"`
	ResponseHeaders  map[string]string `json:"response_headers,omitempty"`
}

// GenerateTextResult is the typed result of a GenerateText call.
// Mirrors Kotlin GenerateTextResult (Types.kt:878-886).
type GenerateTextResult struct {
	Text         string            `json:"text"`
	ToolCalls    []ToolCall        `json:"tool_calls,omitempty"`
	FinishReason FinishReason      `json:"finish_reason,omitempty"`
	Usage        Usage             `json:"usage,omitempty"`
	Warnings     []json.RawMessage `json:"warnings,omitempty"`
	Raw          GenerateResult    `json:"raw"`
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
// Content is `any` so it can be a plain string (common case) or a slice of
// typed ContentParts (multi-part content, e.g. tool results).
// Mirrors Kotlin ModelMessage (Types.kt:523-548).
type ModelMessage struct {
	Role    Role `json:"role"`
	Content any  `json:"content"`
}

// NewTextMessage builds a message with plain string content (the common case).
func NewTextMessage(role Role, text string) ModelMessage {
	return ModelMessage{Role: role, Content: text}
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
// Mirrors Kotlin GenerateTextOptions (Types.kt:561-579).
// TimeoutConfiguration sets per-call timeouts (RFC-0016 H3).
type TimeoutConfiguration struct {
	TotalMs      *uint64 `json:"total_ms,omitempty"`
	FirstChunkMs *uint64 `json:"first_chunk_ms,omitempty"`
	ChunkMs      *uint64 `json:"chunk_ms,omitempty"`
}

// GenerateTextOptions mirrors the shared wire options.
type GenerateTextOptions struct {
	MaxOutputTokens  *uint32               `json:"max_output_tokens,omitempty"`
	Temperature      *float64              `json:"temperature,omitempty"`
	StopSequences    []string              `json:"stop_sequences,omitempty"`
	TopP             *float64              `json:"top_p,omitempty"`
	TopK             *float64              `json:"top_k,omitempty"`
	PresencePenalty  *float64              `json:"presence_penalty,omitempty"`
	FrequencyPenalty *float64              `json:"frequency_penalty,omitempty"`
	ResponseFormat   json.RawMessage       `json:"response_format,omitempty"`
	Seed             *uint64               `json:"seed,omitempty"`
	Tools            []Tool                `json:"tools,omitempty"`
	ToolChoice       ToolChoice            `json:"tool_choice,omitempty"`
	Headers          map[string]string     `json:"headers,omitempty"`
	ProviderOptions  json.RawMessage       `json:"provider_options,omitempty"`
	Reasoning        *ReasoningEffort      `json:"reasoning,omitempty"`
	Instructions     *string               `json:"instructions,omitempty"`
	BodyOverrides    json.RawMessage       `json:"body_overrides,omitempty"`
	MaxRetries       *uint32               `json:"max_retries,omitempty"`
	Timeout          *TimeoutConfiguration `json:"timeout,omitempty"`
	IncludeRawChunks *bool                 `json:"include_raw_chunks,omitempty"`
	SessionID        *string               `json:"session_id,omitempty"`
}

// Tool is a function tool definition (the "function" variant).
// Mirrors Kotlin Tool.Function (Types.kt:209-230).
type Tool struct {
	Type            string                     `json:"type"` // always "function"
	Name            string                     `json:"name"`
	Description     *string                    `json:"description,omitempty"`
	InputSchema     json.RawMessage            `json:"input_schema,omitempty"`
	Strict          *bool                      `json:"strict,omitempty"`
	ProviderOptions map[string]json.RawMessage `json:"provider_options,omitempty"`
	InputExamples   []json.RawMessage          `json:"input_examples,omitempty"`
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
	// Externally-tagged: the map must have exactly one key.
	if len(raw) == 0 {
		return nil, fmt.Errorf("aimux: StreamPart has no variant tag: %s", jsonStr)
	}
	if len(raw) > 1 {
		return nil, fmt.Errorf("aimux: StreamPart has multiple variant tags (%d), expected 1: %s", len(raw), jsonStr)
	}
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

// ── Cache probing (RFC-0015) wire types ─────────────────────────────────────

// TraceFilter filters aggregate queries (RFC-0015 §5.3).
type TraceFilter struct {
	Provider    *string `json:"provider,omitempty"`
	Model       *string `json:"model,omitempty"`
	SessionID   *string `json:"session_id,omitempty"`
	SinceUnixMs *int64  `json:"since_unix_ms,omitempty"`
}

// TraceStats is one (provider, model) aggregation group.
type TraceStats struct {
	Provider                string            `json:"provider"`
	Model                   string            `json:"model"`
	Requests                uint64            `json:"requests"`
	InputTokensTotal        uint64            `json:"input_tokens_total"`
	ClaimedCacheReadTotal   uint64            `json:"claimed_cache_read_total"`
	ClaimedCacheWriteTotal  uint64            `json:"claimed_cache_write_total"`
	ReportedHitRate         *float64          `json:"reported_hit_rate,omitempty"`
	ClientUpperBoundHitRate *float64          `json:"client_upper_bound_hit_rate,omitempty"`
	VerdictCounts           map[string]uint64 `json:"verdict_counts"`
	TTFTP50Ms               *uint64           `json:"ttft_p50_ms,omitempty"`
	TTFTP95Ms               *uint64           `json:"ttft_p95_ms,omitempty"`
	Errors                  uint64            `json:"errors"`
}

// TraceRecord is one probed call (fingerprints only — no plaintext bodies).
type TraceRecord struct {
	Provider             string             `json:"provider"`
	Model                string             `json:"model"`
	RequestID            *string            `json:"request_id,omitempty"`
	SessionID            *string            `json:"session_id,omitempty"`
	TraceID              string             `json:"trace_id"`
	SentAtUnixMs         int64              `json:"sent_at_unix_ms"`
	LCPTokenUpper        *uint64            `json:"lcp_token_upper,omitempty"`
	TTFTMs               *uint64            `json:"ttft_ms,omitempty"`
	Fingerprint          Fingerprint        `json:"fingerprint"`
	Usage                UsageSnapshot      `json:"usage"`
	ResponseCacheHeaders map[string]string  `json:"response_cache_headers,omitempty"`
	RequestCacheHints    *RequestCacheHints `json:"request_cache_hints,omitempty"`
	Verdict              json.RawMessage    `json:"verdict,omitempty"`
	Error                *string            `json:"error,omitempty"`
}

// RequestCacheHints is the best-effort request-side cache hint snapshot.
type RequestCacheHints struct {
	RequestedWrite bool `json:"requested_write"`
}

// Fingerprint is the block-hash chain of a denoised request body (hex).
type Fingerprint struct {
	BodyHash      string   `json:"body_hash"`
	LenBytes      uint64   `json:"len_bytes"`
	BlockSize     uint64   `json:"block_size"`
	BlockHashes   []string `json:"block_hashes"`
	TokenEstimate uint64   `json:"token_estimate"`
}

// UsageSnapshot is the 7-field flat usage snapshot + raw passthrough.
type UsageSnapshot struct {
	InputTotal      *uint64         `json:"input_total,omitempty"`
	InputNoCache    *uint64         `json:"input_no_cache,omitempty"`
	CacheRead       *uint64         `json:"cache_read,omitempty"`
	CacheWrite      *uint64         `json:"cache_write,omitempty"`
	OutputTotal     *uint64         `json:"output_total,omitempty"`
	OutputText      *uint64         `json:"output_text,omitempty"`
	OutputReasoning *uint64         `json:"output_reasoning,omitempty"`
	Raw             json.RawMessage `json:"raw,omitempty"`
}

// SessionChainView is the per-session ordered chain view.
type SessionChainView struct {
	SessionID       string        `json:"session_id"`
	RecordIDs       []string      `json:"record_ids"`
	PrefixStability float64       `json:"prefix_stability"`
	Breaks          []PrefixBreak `json:"breaks"`
}

// PrefixBreak marks a prefix break between consecutive session records.
type PrefixBreak struct {
	AtRecordID    string `json:"at_record_id"`
	PrevRecordID  string `json:"prev_record_id"`
	LCPBytes      uint64 `json:"lcp_bytes"`
	ExpectedBreak bool   `json:"expected_break"`
	Kind          string `json:"kind"`
}

// ── OpenAI Chat Completions output (RFC-0026) ──────────────────────────────
//
// Mirrors aimux_core::openai_output. JSON tags match the serde wire format:
// Rust `#[serde(skip_serializing_if = "Option::is_none")]` → Go `omitempty`;
// Rust `#[serde(rename = "type")]` → Go `json:"type"`. Fields without
// skip_serializing_if (e.g. finish_reason, content) omit `omitempty` so they
// serialize as null when empty, matching the Rust output.

// ChatCompletionFunction is the function payload of a tool call.
type ChatCompletionFunction struct {
	Name      string `json:"name"`
	Arguments string `json:"arguments"`
}

// ChatCompletionToolCall is a tool call in an assistant message.
type ChatCompletionToolCall struct {
	ID       string                 `json:"id"`
	Type     string                 `json:"type"`
	Function ChatCompletionFunction `json:"function"`
}

// ChatCompletionMessage is the assistant message in a ChatCompletion.
type ChatCompletionMessage struct {
	Role             string                   `json:"role"`
	Content          *string                  `json:"content"`
	ReasoningContent *string                  `json:"reasoning_content,omitempty"`
	ToolCalls        []ChatCompletionToolCall `json:"tool_calls,omitempty"`
	Annotations      []json.RawMessage        `json:"annotations,omitempty"`
}

// ChatCompletionChoice is a single choice in a ChatCompletion.
type ChatCompletionChoice struct {
	Index        int                   `json:"index"`
	Message      ChatCompletionMessage `json:"message"`
	FinishReason *string               `json:"finish_reason"`
	Logprobs     json.RawMessage       `json:"logprobs,omitempty"`
}

// PromptTokensDetails is the prompt token breakdown.
type PromptTokensDetails struct {
	CachedTokens     uint32  `json:"cached_tokens"`
	CacheWriteTokens *uint32 `json:"cache_write_tokens,omitempty"`
}

// CompletionTokensDetails is the completion token breakdown.
type CompletionTokensDetails struct {
	ReasoningTokens *uint32 `json:"reasoning_tokens,omitempty"`
}

// ChatCompletionUsage is token usage for a Chat Completion.
type ChatCompletionUsage struct {
	PromptTokens            int                      `json:"prompt_tokens"`
	CompletionTokens        int                      `json:"completion_tokens"`
	TotalTokens             int                      `json:"total_tokens"`
	PromptTokensDetails     *PromptTokensDetails     `json:"prompt_tokens_details,omitempty"`
	CompletionTokensDetails *CompletionTokensDetails `json:"completion_tokens_details,omitempty"`
}

// ChatCompletion is a complete OpenAI Chat Completion response (non-streaming).
type ChatCompletion struct {
	ID                string                 `json:"id"`
	Object            string                 `json:"object"`
	Created           uint64                 `json:"created"`
	Model             string                 `json:"model"`
	Choices           []ChatCompletionChoice `json:"choices"`
	Usage             ChatCompletionUsage    `json:"usage"`
	SystemFingerprint *string                `json:"system_fingerprint,omitempty"`
}

// ChatCompletionChunkFunction is the function payload of a chunk tool call.
type ChatCompletionChunkFunction struct {
	Name      *string `json:"name,omitempty"`
	Arguments *string `json:"arguments,omitempty"`
}

// ChatCompletionChunkToolCall is a tool-call delta in a streaming chunk.
type ChatCompletionChunkToolCall struct {
	Index    int                         `json:"index"`
	ID       *string                     `json:"id,omitempty"`
	Type     *string                     `json:"type,omitempty"`
	Function ChatCompletionChunkFunction `json:"function"`
}

// ChatCompletionDelta is the delta payload of a streaming chunk.
type ChatCompletionDelta struct {
	Role             *string                       `json:"role,omitempty"`
	Content          *string                       `json:"content,omitempty"`
	ReasoningContent *string                       `json:"reasoning_content,omitempty"`
	ToolCalls        []ChatCompletionChunkToolCall `json:"tool_calls,omitempty"`
}

// ChatCompletionChunkChoice is a single choice in a streaming chunk.
type ChatCompletionChunkChoice struct {
	Index        int                 `json:"index"`
	Delta        ChatCompletionDelta `json:"delta"`
	FinishReason *string             `json:"finish_reason"`
	Logprobs     json.RawMessage     `json:"logprobs,omitempty"`
}

// ChatCompletionChunk is a single OpenAI Chat Completion streaming chunk.
type ChatCompletionChunk struct {
	ID      string                      `json:"id"`
	Object  string                      `json:"object"`
	Created uint64                      `json:"created"`
	Model   string                      `json:"model"`
	Choices []ChatCompletionChunkChoice `json:"choices"`
	Usage   *ChatCompletionUsage        `json:"usage,omitempty"`
}

// ParseChatCompletion parses the JSON string returned by Model.GenerateTextAsOpenAI
// into a typed ChatCompletion.
func ParseChatCompletion(jsonStr string) (*ChatCompletion, error) {
	var r ChatCompletion
	if err := json.Unmarshal([]byte(jsonStr), &r); err != nil {
		return nil, fmt.Errorf("aimux: failed to parse ChatCompletion: %w", err)
	}
	return &r, nil
}

// ParseChatCompletionChunk parses a ChatCompletionChunk JSON string.
func ParseChatCompletionChunk(jsonStr string) (*ChatCompletionChunk, error) {
	var r ChatCompletionChunk
	if err := json.Unmarshal([]byte(jsonStr), &r); err != nil {
		return nil, fmt.Errorf("aimux: failed to parse ChatCompletionChunk: %w", err)
	}
	return &r, nil
}
