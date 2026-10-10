// Typed data structures mirroring the aimux-core wire format: the JSON the
// Rust engine reads and writes, which is the AI SDK JSON (camelCase field
// names, unions tagged by a "type" key, absent rather than null for optional
// fields). The generated TypeScript types in bindings/node/src/types are the
// exact shape.
//
// The raw JSON boundary is handled by Model.GenerateText / Model.StreamText —
// this layer only provides typed parsing so callers don't manually dig through
// JSON. Decoding is lenient (unknown keys ignored) so future engine additions
// don't break existing clients.

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

// Source is a URL or document source. Exactly one variant is populated.
type Source struct {
	URL      *URLSource
	Document *DocumentSource
}

type URLSource struct {
	ID               string          `json:"id"`
	URL              string          `json:"url"`
	Title            *string         `json:"title,omitempty"`
	ProviderMetadata json.RawMessage `json:"providerMetadata,omitempty"`
}

type DocumentSource struct {
	ID               string          `json:"id"`
	MediaType        string          `json:"mediaType"`
	Title            string          `json:"title"`
	Filename         *string         `json:"filename,omitempty"`
	ProviderMetadata json.RawMessage `json:"providerMetadata,omitempty"`
}

func (s Source) MarshalJSON() ([]byte, error) {
	if s.URL != nil && s.Document == nil {
		return json.Marshal(struct {
			SourceType string `json:"sourceType"`
			*URLSource
		}{"url", s.URL})
	}
	if s.Document != nil && s.URL == nil {
		return json.Marshal(struct {
			SourceType string `json:"sourceType"`
			*DocumentSource
		}{"document", s.Document})
	}
	return nil, fmt.Errorf("aimux: Source must have exactly one variant")
}

func (s *Source) UnmarshalJSON(data []byte) error {
	var fields map[string]json.RawMessage
	if err := json.Unmarshal(data, &fields); err != nil {
		return err
	}
	var sourceType string
	if err := json.Unmarshal(fields["sourceType"], &sourceType); err != nil {
		return fmt.Errorf("aimux: invalid sourceType: %w", err)
	}
	required := []string{"id"}
	switch sourceType {
	case "url":
		required = append(required, "url")
	case "document":
		required = append(required, "mediaType", "title")
	default:
		return fmt.Errorf("aimux: unknown sourceType %q", sourceType)
	}
	for _, name := range required {
		var value *string
		if err := json.Unmarshal(fields[name], &value); err != nil || value == nil {
			return fmt.Errorf("aimux: Source requires string %s", name)
		}
	}
	if sourceType == "url" {
		var value URLSource
		if err := json.Unmarshal(data, &value); err != nil {
			return err
		}
		*s = Source{URL: &value}
	} else {
		var value DocumentSource
		if err := json.Unmarshal(data, &value); err != nil {
			return err
		}
		*s = Source{Document: &value}
	}
	return nil
}

// InputTokenUsage is input token usage detail with cache breakdown.
type InputTokenUsage struct {
	Total      *uint32 `json:"total,omitempty"`
	NoCache    *uint32 `json:"noCache,omitempty"`
	CacheRead  *uint32 `json:"cacheRead,omitempty"`
	CacheWrite *uint32 `json:"cacheWrite,omitempty"`
}

// OutputTokenUsage is output token usage detail.
type OutputTokenUsage struct {
	Total     *uint32 `json:"total,omitempty"`
	Text      *uint32 `json:"text,omitempty"`
	Reasoning *uint32 `json:"reasoning,omitempty"`
}

// Usage is token usage statistics.
type Usage struct {
	InputTokens  InputTokenUsage            `json:"inputTokens,omitempty"`
	OutputTokens OutputTokenUsage           `json:"outputTokens,omitempty"`
	Raw          map[string]json.RawMessage `json:"raw,omitempty"`
}

// FinishReason is the finish reason.
type FinishReason struct {
	Unified FinishReasonUnified `json:"unified,omitempty"`
	Raw     *string             `json:"raw,omitempty"`
}

// ToolCall represents a tool call requested by the model.
type ToolCall struct {
	ToolCallID       string          `json:"toolCallId"`
	ToolName         string          `json:"toolName"`
	Input            json.RawMessage `json:"input,omitempty"`
	ProviderExecuted *bool           `json:"providerExecuted,omitempty"`
	Dynamic          *bool           `json:"dynamic,omitempty"`
	// ProviderMetadata carries provider-specific data associated with this call.
	ProviderMetadata json.RawMessage `json:"providerMetadata,omitempty"`
	// Invalid is set by Core when the tool call stays invalid after optional repair.
	Invalid *bool `json:"invalid,omitempty"`
	// Error is the typed lookup, parse, schema, or repair failure for an invalid call.
	Error json.RawMessage `json:"error,omitempty"`
}

// ContentPart is a single content part, in a prompt message or a result: an
// object tagged by "type" ("text", "tool-call", "tool-result", "source", ...).
// It is kept as raw JSON for forward compatibility. A tool-result part is
// {"type":"tool-result","toolCallId":...,"toolName":...,"output":{...}} where
// output is {"type":"text"|"json"|"error-text"|"error-json","value":...},
// {"type":"execution-denied","reason":...} or {"type":"content","value":[...]}.
type ContentPart = json.RawMessage

// ResponseMetadata describes the provider response.
type ResponseMetadata struct {
	ID        *string `json:"id,omitempty"`
	Timestamp *string `json:"timestamp,omitempty"`
	ModelID   *string `json:"modelId,omitempty"`
}

// ResponseInfo is response metadata with optional HTTP headers and body.
type ResponseInfo struct {
	ResponseMetadata
	Headers map[string]string `json:"headers,omitempty"`
	Body    json.RawMessage   `json:"body,omitempty"`
}

// RequestInfo is provider request information with an optional HTTP body.
type RequestInfo struct {
	Body json.RawMessage `json:"body,omitempty"`
}

// GenerateResult is the raw provider result.
type GenerateResult struct {
	Content          []ContentPart     `json:"content,omitempty"`
	FinishReason     FinishReason      `json:"finishReason,omitempty"`
	Usage            Usage             `json:"usage,omitempty"`
	Warnings         []json.RawMessage `json:"warnings,omitempty"`
	ProviderMetadata json.RawMessage   `json:"providerMetadata,omitempty"`
	Response         *ResponseInfo     `json:"response,omitempty"`
	Request          *RequestInfo      `json:"request,omitempty"`
}

// GenerateTextResult is the typed result of a GenerateText call.
type GenerateTextResult struct {
	Content          []ContentPart     `json:"content,omitempty"`
	Text             string            `json:"text"`
	ToolCalls        []ToolCall        `json:"toolCalls,omitempty"`
	FinishReason     FinishReason      `json:"finishReason,omitempty"`
	Usage            Usage             `json:"usage,omitempty"`
	Warnings         []json.RawMessage `json:"warnings,omitempty"`
	Raw              GenerateResult    `json:"raw"`
	Reasoning        []json.RawMessage `json:"reasoning,omitempty"`
	ReasoningText    string            `json:"reasoningText,omitempty"`
	Sources          []Source          `json:"sources,omitempty"`
	Files            []json.RawMessage `json:"files,omitempty"`
	ResponseMessages []ModelMessage    `json:"responseMessages,omitempty"`
	// RawFinishReason is the raw provider-specific finish reason string.
	RawFinishReason *string `json:"rawFinishReason,omitempty"`
	// ProviderMetadata is provider-specific metadata (e.g. Anthropic cache info).
	// Mirrored from raw.providerMetadata for top-level convenience.
	ProviderMetadata json.RawMessage `json:"providerMetadata,omitempty"`
	// Request is the request information (body) from the provider.
	Request RequestInfo `json:"request"`
	// Response is the response metadata, headers and body from the provider.
	Response ResponseInfo `json:"response"`
	// TotalUsage is total token usage across all steps. In single-step mode
	// (aimux's default), equals Usage. Provided for AI SDK parity.
	TotalUsage Usage `json:"totalUsage,omitempty"`
}

// GenerateObjectResult is the typed result of a GenerateObject call.
// `object` is an arbitrary JSON value and `raw` is the full GenerateTextResult.
type GenerateObjectResult struct {
	Object           json.RawMessage    `json:"object,omitempty"`
	FinishReason     FinishReason       `json:"finishReason,omitempty"`
	RawFinishReason  *string            `json:"rawFinishReason,omitempty"`
	Usage            Usage              `json:"usage,omitempty"`
	Warnings         []json.RawMessage  `json:"warnings,omitempty"`
	Reasoning        *string            `json:"reasoning,omitempty"`
	ProviderMetadata json.RawMessage    `json:"providerMetadata,omitempty"`
	Response         ResponseMetadata   `json:"response,omitempty"`
	Raw              GenerateTextResult `json:"raw"`
}

// StreamTextResultAggregated is the aggregated result of a consumed stream.
type StreamTextResultAggregated struct {
	Content          []ContentPart     `json:"content,omitempty"`
	Text             string            `json:"text"`
	Reasoning        []json.RawMessage `json:"reasoning,omitempty"`
	ReasoningText    string            `json:"reasoningText,omitempty"`
	ToolCalls        []ToolCall        `json:"toolCalls,omitempty"`
	Sources          []Source          `json:"sources,omitempty"`
	Files            []json.RawMessage `json:"files,omitempty"`
	FinishReason     FinishReason      `json:"finishReason,omitempty"`
	RawFinishReason  *string           `json:"rawFinishReason,omitempty"`
	Usage            Usage             `json:"usage,omitempty"`
	TotalUsage       Usage             `json:"totalUsage,omitempty"`
	Warnings         []json.RawMessage `json:"warnings,omitempty"`
	ProviderMetadata json.RawMessage   `json:"providerMetadata,omitempty"`
	// Request is the request information from the provider.
	Request RequestInfo `json:"request"`
	// Response is the response metadata and headers from the last step.
	Response         ResponseInfo   `json:"response"`
	ResponseMessages []ModelMessage `json:"responseMessages,omitempty"`
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

// ParseGenerateObjectResult parses the JSON string returned by
// Model.GenerateObject into a typed GenerateObjectResult.
func ParseGenerateObjectResult(jsonStr string) (*GenerateObjectResult, error) {
	var r GenerateObjectResult
	if err := json.Unmarshal([]byte(jsonStr), &r); err != nil {
		return nil, fmt.Errorf("aimux: failed to parse GenerateObjectResult: %w", err)
	}
	return &r, nil
}

// ParseStreamTextResultAggregated parses the JSON string returned by
// Model.ConsumeStreamText into a typed StreamTextResultAggregated.
func ParseStreamTextResultAggregated(jsonStr string) (*StreamTextResultAggregated, error) {
	var r StreamTextResultAggregated
	if err := json.Unmarshal([]byte(jsonStr), &r); err != nil {
		return nil, fmt.Errorf("aimux: failed to parse StreamTextResultAggregated: %w", err)
	}
	return &r, nil
}

// ── ModelMessage (for multi-role prompts) ─────────────────────────────────────

// ModelMessage is a single message in a conversation.
// Content is `any` so it can be a plain string (common case) or a slice of
// ContentParts (multi-part content, e.g. tool results).
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
// TimeoutConfiguration sets per-call timeouts.
type TimeoutConfiguration struct {
	TotalMs      *uint64 `json:"totalMs,omitempty"`
	StepMs       *uint64 `json:"stepMs,omitempty"`
	FirstChunkMs *uint64 `json:"firstChunkMs,omitempty"`
	ChunkMs      *uint64 `json:"chunkMs,omitempty"`
}

// GenerateTextOptions mirrors the shared wire options. All fields are optional.
type GenerateTextOptions struct {
	MaxOutputTokens  *uint32               `json:"maxOutputTokens,omitempty"`
	Temperature      *float64              `json:"temperature,omitempty"`
	StopSequences    []string              `json:"stopSequences,omitempty"`
	TopP             *float64              `json:"topP,omitempty"`
	TopK             *float64              `json:"topK,omitempty"`
	PresencePenalty  *float64              `json:"presencePenalty,omitempty"`
	FrequencyPenalty *float64              `json:"frequencyPenalty,omitempty"`
	ResponseFormat   json.RawMessage       `json:"responseFormat,omitempty"`
	Seed             *uint64               `json:"seed,omitempty"`
	Tools            []Tool                `json:"tools,omitempty"`
	ToolChoice       ToolChoice            `json:"toolChoice,omitempty"`
	Headers          map[string]string     `json:"headers,omitempty"`
	ProviderOptions  json.RawMessage       `json:"providerOptions,omitempty"`
	Reasoning        *ReasoningEffort      `json:"reasoning,omitempty"`
	Instructions     *string               `json:"instructions,omitempty"`
	MaxRetries       *uint32               `json:"maxRetries,omitempty"`
	Timeout          *TimeoutConfiguration `json:"timeout,omitempty"`
	IncludeRawChunks *bool                 `json:"includeRawChunks,omitempty"`
	SessionID        *string               `json:"sessionId,omitempty"`
	// RepairToolCall repairs tool calls the model got wrong (RFC-0035). It
	// runs in Go, after generation, on every call the engine marked invalid —
	// never serialized into the options sent to the engine.
	//
	// It applies to Generate, GenerateObj, ConsumeStream and Stream, and to
	// GenerateAsOpenAI, which repairs the native result before converting it.
	// StreamAsOpenAI does not reflect repair: its tool-argument deltas are the
	// provider's text, forwarded as they arrive (as in the AI SDK).
	RepairToolCall RepairToolCallFunc `json:"-"`
}

type FunctionToolInputExample struct {
	Input map[string]json.RawMessage `json:"input"`
}

// Tool is a tool definition: a function tool (Type "function": Name,
// InputSchema, ...) or a provider-defined tool (Type "provider": ID, Name, Args).
type Tool struct {
	Type            string                     `json:"type"` // "function" or "provider"
	Name            string                     `json:"name"`
	Description     *string                    `json:"description,omitempty"`
	InputSchema     json.RawMessage            `json:"inputSchema,omitempty"`
	Strict          *bool                      `json:"strict,omitempty"`
	ProviderOptions map[string]json.RawMessage `json:"providerOptions,omitempty"`
	InputExamples   []FunctionToolInputExample `json:"inputExamples,omitempty"`
	// ID and Args are the provider tool's id ("<provider>.<tool>") and arguments.
	ID   string                     `json:"id,omitempty"`
	Args map[string]json.RawMessage `json:"args,omitempty"`
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

// StreamPart is a parsed stream part. The wire format is an object tagged by
// its "type" key (e.g. {"type":"text-delta","id":"...","delta":"..."}).
type StreamPart struct {
	// Type is the part's "type" (e.g. "text-delta", "tool-call", "finish").
	Type string `json:"-"`
	// Raw is the whole part object, "type" key included. Decode it into the
	// struct of your choice, e.g. TextDeltaPayload.
	Raw json.RawMessage `json:"-"`
}

// ParseStreamPart parses a StreamPart JSON string.
func ParseStreamPart(jsonStr string) (*StreamPart, error) {
	var head struct {
		Type string `json:"type"`
	}
	if err := json.Unmarshal([]byte(jsonStr), &head); err != nil {
		return nil, fmt.Errorf("aimux: failed to parse StreamPart: %w", err)
	}
	if head.Type == "" {
		return nil, fmt.Errorf("aimux: StreamPart has no type: %s", jsonStr)
	}
	return &StreamPart{Type: head.Type, Raw: json.RawMessage(jsonStr)}, nil
}

// Source decodes a "source" stream part.
func (p *StreamPart) Source() (*Source, error) {
	if p.Type != "source" {
		return nil, fmt.Errorf("aimux: expected source stream part, got %s", p.Type)
	}
	var source Source
	if err := json.Unmarshal(p.Raw, &source); err != nil {
		return nil, err
	}
	return &source, nil
}

// TextDeltaPayload is a {"type":"text-delta",...} stream part.
type TextDeltaPayload struct {
	ID    string `json:"id,omitempty"`
	Delta string `json:"delta"`
}

// ── Cache probing (RFC-0015) wire types ─────────────────────────────────────

// TraceFilter filters aggregate queries (RFC-0015 §5.3).
type TraceFilter struct {
	Provider    *string `json:"provider,omitempty"`
	Model       *string `json:"model,omitempty"`
	SessionID   *string `json:"sessionId,omitempty"`
	SinceUnixMs *int64  `json:"sinceUnixMs,omitempty"`
}

// TraceStats is one (provider, model) aggregation group.
type TraceStats struct {
	Provider                string            `json:"provider"`
	Model                   string            `json:"model"`
	Requests                uint64            `json:"requests"`
	InputTokensTotal        uint64            `json:"inputTokensTotal"`
	ClaimedCacheReadTotal   uint64            `json:"claimedCacheReadTotal"`
	ClaimedCacheWriteTotal  uint64            `json:"claimedCacheWriteTotal"`
	ReportedHitRate         *float64          `json:"reportedHitRate,omitempty"`
	ClientUpperBoundHitRate *float64          `json:"clientUpperBoundHitRate,omitempty"`
	VerdictCounts           map[string]uint64 `json:"verdictCounts"`
	TTFTP50Ms               *uint64           `json:"ttftP50Ms,omitempty"`
	TTFTP95Ms               *uint64           `json:"ttftP95Ms,omitempty"`
	Errors                  uint64            `json:"errors"`
}

// TraceRecord is one probed call (fingerprints only — no plaintext bodies).
type TraceRecord struct {
	Provider             string             `json:"provider"`
	Model                string             `json:"model"`
	RequestID            *string            `json:"requestId,omitempty"`
	SessionID            *string            `json:"sessionId,omitempty"`
	TraceID              string             `json:"callId"`
	SentAtUnixMs         int64              `json:"sentAtUnixMs"`
	LCPTokenUpper        *uint64            `json:"lcpTokenUpper,omitempty"`
	TTFTMs               *uint64            `json:"ttftMs,omitempty"`
	Fingerprint          Fingerprint        `json:"fingerprint"`
	Usage                UsageSnapshot      `json:"usage"`
	ResponseCacheHeaders map[string]string  `json:"responseCacheHeaders,omitempty"`
	RequestCacheHints    *RequestCacheHints `json:"requestCacheHints,omitempty"`
	Verdict              json.RawMessage    `json:"verdict,omitempty"`
	Error                *string            `json:"error,omitempty"`
}

// RequestCacheHints is the best-effort request-side cache hint snapshot.
type RequestCacheHints struct {
	RequestedWrite bool `json:"requestedWrite"`
}

// Fingerprint is the block-hash chain of a denoised request body (hex).
type Fingerprint struct {
	BodyHash      string   `json:"bodyHash"`
	LenBytes      uint64   `json:"lenBytes"`
	BlockSize     uint64   `json:"blockSize"`
	BlockHashes   []string `json:"blockHashes"`
	TokenEstimate uint64   `json:"tokenEstimate"`
}

// UsageSnapshot is the 7-field flat usage snapshot + raw passthrough.
type UsageSnapshot struct {
	InputTotal      *uint64         `json:"inputTotal,omitempty"`
	InputNoCache    *uint64         `json:"inputNoCache,omitempty"`
	CacheRead       *uint64         `json:"cacheRead,omitempty"`
	CacheWrite      *uint64         `json:"cacheWrite,omitempty"`
	OutputTotal     *uint64         `json:"outputTotal,omitempty"`
	OutputText      *uint64         `json:"outputText,omitempty"`
	OutputReasoning *uint64         `json:"outputReasoning,omitempty"`
	Raw             json.RawMessage `json:"raw,omitempty"`
}

// SessionChainView is the per-session ordered chain view.
type SessionChainView struct {
	SessionID       string        `json:"sessionId"`
	RecordIDs       []string      `json:"recordIds"`
	PrefixStability float64       `json:"prefixStability"`
	Breaks          []PrefixBreak `json:"breaks"`
}

// PrefixBreak marks a prefix break between consecutive session records.
type PrefixBreak struct {
	AtRecordID    string `json:"atRecordId"`
	PrevRecordID  string `json:"prevRecordId"`
	LCPBytes      uint64 `json:"lcpBytes"`
	ExpectedBreak bool   `json:"expectedBreak"`
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
